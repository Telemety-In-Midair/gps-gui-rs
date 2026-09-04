//! The look sheet: every size and spacing in the app, each a fraction of
//! something that scales, kept in a small text file of its own.
//!
//! The pages say *what* is on them and the code says what it does; this file
//! says how big. It is a sheet of measures and nothing else - no conditions,
//! no bindings, no logic - so the in-app adjuster can read and write it and a
//! hand edit can too, without either side having to understand the other.
//!
//! A measure is a number with a unit, and every unit is relative:
//!
//! ```text
//! page
//!     margin  2.5%        # of the smaller screen side
//!     gap
//!         item   0.5em    # of the body text height
//! bar
//!     gap     0.15icon    # of the toolbar icon side
//! settings
//!     path    50%w min 8em max 22em
//! ```
//!
//! `%w` and `%h` are of the screen width and height, and `x` is a plain
//! ratio of whatever the key's note names. There is no unit for points: a
//! fixed size cannot be written here. The two measures that are absolute -
//! a fingertip under every control, and a cap on the icon - are physical
//! rather than a look, and stay in code as [`TOUCH_MIN`] and [`ICON_MAX`].
//!
//! Indentation nests: a name alone on a line opens a block, and a dotted name
//! is the same as nesting. Comments run from `#` to the end of the line. A
//! missing key keeps its default, an unknown one is reported and skipped, and
//! a key set twice is an error.
//!
//! [`Look::save`] edits an existing file in place - comments, alignment and
//! unknown keys survive, only the changed values are rewritten - and
//! generates a documented sheet from [`Look::to_sheet`] where there is none.

use std::fmt;
use std::ops::Range;
use std::str::FromStr;

/// The smallest a control may be, in points: the floor under an icon's side
/// and under the height of every button, field, checkbox and dropdown.
///
/// The one measure in the UI that stays absolute. Everything in the sheet is
/// a fraction of the screen or of the text, but this is a touch target, and a
/// fingertip is the same size whatever the screen is.
pub const TOUCH_MIN: f32 = 40.0;

/// The largest an icon is drawn, in points: about two fingertips. Keeps the
/// toolbar from becoming a row of slabs on a large desktop window.
pub const ICON_MAX: f32 = 70.0;

/// What a measure is a fraction of.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Unit {
    /// Of the smaller screen dimension, in percent: `5%`.
    Screen,
    /// Of the screen width, in percent: `50%w`.
    Width,
    /// Of the screen height, in percent: `34%h`.
    Height,
    /// Of the body text height: `0.5em`.
    Em,
    /// Of the toolbar icon side: `0.7icon`.
    Icon,
    /// A plain ratio of whatever the key's note names: `0.25x`.
    Times,
}

impl Unit {
    /// Every unit, in the order the adjuster offers them.
    pub const ALL: [Unit; 6] = [
        Unit::Screen,
        Unit::Width,
        Unit::Height,
        Unit::Em,
        Unit::Icon,
        Unit::Times,
    ];

    /// How the unit is written after its number.
    pub fn suffix(self) -> &'static str {
        match self {
            Unit::Screen => "%",
            Unit::Width => "%w",
            Unit::Height => "%h",
            Unit::Em => "em",
            Unit::Icon => "icon",
            Unit::Times => "x",
        }
    }

    /// What a number in this unit is a fraction of, for a hover.
    pub fn describe(self) -> &'static str {
        match self {
            Unit::Screen => "percent of the smaller screen side",
            Unit::Width => "percent of the screen width",
            Unit::Height => "percent of the screen height",
            Unit::Em => "body text heights",
            Unit::Icon => "toolbar icon sides",
            Unit::Times => "a plain ratio",
        }
    }

    fn from_suffix(s: &str) -> Option<Unit> {
        Unit::ALL.into_iter().find(|u| u.suffix() == s)
    }

    /// Points per whole unit under `scale`.
    fn base(self, scale: &Scale) -> f32 {
        match self {
            Unit::Screen => scale.screen.size().min_elem() / 100.0,
            Unit::Width => scale.screen.width() / 100.0,
            Unit::Height => scale.screen.height() / 100.0,
            Unit::Em => scale.em,
            Unit::Icon => scale.icon,
            Unit::Times => 1.0,
        }
    }

    /// The span the adjuster's slider covers for values in this unit. Wide
    /// enough for every default with room past it; a typed value may exceed
    /// it.
    pub fn range(self) -> Range<f32> {
        match self {
            Unit::Screen => 0.0..25.0,
            Unit::Width | Unit::Height => 0.0..100.0,
            Unit::Em => 0.0..25.0,
            Unit::Icon => 0.0..6.0,
            Unit::Times => 0.0..3.0,
        }
    }

    /// The step the slider moves in: coarse for the percent units, fine for
    /// the rest.
    pub fn step(self) -> f64 {
        match self {
            Unit::Screen | Unit::Width | Unit::Height => 0.5,
            Unit::Em | Unit::Icon | Unit::Times => 0.05,
        }
    }
}

/// The screen, the text and the icon a frame is measured against.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Scale {
    /// The viewport, in points.
    pub screen: egui::Rect,
    /// The body text height, in points.
    pub em: f32,
    /// The toolbar icon side, in points.
    pub icon: f32,
}

/// A number with a unit.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Quantity {
    pub value: f32,
    pub unit: Unit,
}

impl Quantity {
    fn eval(&self, scale: &Scale) -> f32 {
        self.value * self.unit.base(scale)
    }
}

/// A number without trailing zeros: `2.5`, `5`, `0.25`.
fn fmt_num(v: f32) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-0" {
        "0".to_string()
    } else {
        s.to_string()
    }
}

impl fmt::Display for Quantity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", fmt_num(self.value), self.unit.suffix())
    }
}

impl FromStr for Quantity {
    type Err = String;

    fn from_str(tok: &str) -> Result<Self, String> {
        let split = tok
            .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+'))
            .unwrap_or(tok.len());
        let (num, suffix) = tok.split_at(split);
        let value: f32 = num
            .parse()
            .map_err(|_| format!("`{tok}` is not a number with a unit"))?;
        if !value.is_finite() || value < 0.0 {
            return Err(format!("`{tok}` must be zero or more"));
        }
        let unit = Unit::from_suffix(suffix).ok_or_else(|| {
            if suffix.is_empty() {
                format!("`{tok}` has no unit (%, %w, %h, em, icon, or x for a ratio)")
            } else {
                format!("`{suffix}` is not a unit (%, %w, %h, em, icon, or x for a ratio)")
            }
        })?;
        Ok(Quantity { value, unit })
    }
}

/// A measure: a quantity, optionally held between a floor and a ceiling that
/// may be in other units (`50%w min 8em max 22em`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Measure {
    pub q: Quantity,
    pub min: Option<Quantity>,
    pub max: Option<Quantity>,
}

impl Measure {
    /// A bare quantity.
    pub fn new(value: f32, unit: Unit) -> Self {
        Self {
            q: Quantity { value, unit },
            min: None,
            max: None,
        }
    }

    /// The measure in points under `scale`, the floor applied before the
    /// ceiling.
    pub fn eval(&self, scale: &Scale) -> f32 {
        let mut v = self.q.eval(scale);
        if let Some(min) = self.min {
            v = v.max(min.eval(scale));
        }
        if let Some(max) = self.max {
            v = v.min(max.eval(scale));
        }
        v
    }

    /// Rewrite the quantity in `unit` so it comes to the same points under
    /// `scale`: the adjuster's unit switch changes what a size is measured
    /// against, not the size. A unit with nothing behind it yet (an icon of
    /// zero) keeps the number instead.
    pub fn convert(&mut self, unit: Unit, scale: &Scale) {
        let base = unit.base(scale);
        if base > 0.0 {
            self.q.value = self.q.eval(scale) / base;
        }
        self.q.unit = unit;
    }

    /// Whether any part of the measure is written in `unit`.
    pub fn uses(&self, unit: Unit) -> bool {
        [Some(self.q), self.min, self.max]
            .into_iter()
            .flatten()
            .any(|q| q.unit == unit)
    }
}

impl fmt::Display for Measure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.q)?;
        if let Some(min) = self.min {
            write!(f, " min {min}")?;
        }
        if let Some(max) = self.max {
            write!(f, " max {max}")?;
        }
        Ok(())
    }
}

impl FromStr for Measure {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let tokens: Vec<&str> = s.split_whitespace().collect();
        parse_measure(&tokens)
    }
}

fn parse_measure(tokens: &[&str]) -> Result<Measure, String> {
    let (first, rest) = tokens
        .split_first()
        .ok_or_else(|| "a measure is missing".to_string())?;
    let q: Quantity = first.parse()?;
    let mut m = Measure {
        q,
        min: None,
        max: None,
    };
    let mut rest = rest.iter();
    while let Some(word) = rest.next() {
        let bound = rest
            .next()
            .ok_or_else(|| format!("`{word}` needs a measure after it"))?;
        let bound: Quantity = bound.parse()?;
        match *word {
            "min" if m.min.is_none() => m.min = Some(bound),
            "max" if m.max.is_none() => m.max = Some(bound),
            "min" | "max" => return Err(format!("`{word}` is given twice")),
            other => return Err(format!("`{other}` is not `min` or `max`")),
        }
    }
    Ok(m)
}

macro_rules! keys {
    ($( $variant:ident : $path:literal = $default:literal , $doc:literal ; )*) => {
        /// Every measure in the sheet, by name. The order here is the order
        /// of the generated sheet.
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
        pub enum Key {
            $( #[doc = $doc] $variant, )*
        }

        impl Key {
            /// Every key, in sheet order.
            pub const ALL: &'static [Key] = &[ $( Key::$variant, )* ];

            /// The dotted name the sheet uses.
            pub fn path(self) -> &'static str {
                match self { $( Key::$variant => $path, )* }
            }

            /// What the measure is for, as the sheet's comment and the
            /// adjuster's hover.
            pub fn doc(self) -> &'static str {
                match self { $( Key::$variant => $doc, )* }
            }

            fn default_text(self) -> &'static str {
                match self { $( Key::$variant => $default, )* }
            }
        }
    };
}

keys! {
    PageMargin: "page.margin" = "2.5%",
        "Between a page's body and the screen edge.";
    GapHair: "page.gap.hair" = "0.25em",
        "The vertical rhythm of a page, in text heights. A hair: between a control and the note under it.";
    GapTight: "page.gap.tight" = "0.4em",
        "Between a title and the line explaining it.";
    GapItem: "page.gap.item" = "0.5em",
        "Between two controls.";
    GapBlock: "page.gap.block" = "0.75em",
        "Between two blocks of controls.";
    GapSection: "page.gap.section" = "1em",
        "Before a section title.";
    IconSize: "icon.size" = "5%",
        "Side of a toolbar icon. A fingertip is the floor and about two fingertips the ceiling whatever this says: those are physical, not a look.";
    BarMarginX: "bar.margin.x" = "2%",
        "Inner side margin of a bar spanning the screen: the map's controls at the top and its status read-out at the bottom.";
    BarMarginY: "bar.margin.y" = "1%",
        "Inner top and bottom margin of those bars.";
    BarButtonPadX: "bar.button.pad.x" = "0.7icon",
        "Side padding around the glyph in a toolbar button. Doubles as the space that keeps the buttons apart.";
    BarButtonPadY: "bar.button.pad.y" = "0.45icon",
        "Top and bottom padding around the glyph in a toolbar button.";
    BarGap: "bar.gap" = "0.15icon",
        "Between two buttons in the controls bar.";
    CornerMargin: "corner.margin" = "3%",
        "Inset of the floating corner toggle from the screen edge.";
    CornerPad: "corner.pad" = "0.2icon",
        "Padding around the corner toggle's glyph. Tighter than the toolbar's: alone over the page, wide padding reads as a slab.";
    FieldPadX: "field.pad.x" = "0.3em",
        "Side padding inside a text input. The vertical padding is whatever brings the field up to the height of the button beside it.";
    ControlPadX: "control.pad.x" = "0.6em",
        "Side padding inside a text button. Everything under control is measured off the body font size.";
    ControlPadY: "control.pad.y" = "0.3em",
        "Top and bottom padding inside a text button.";
    ControlSpacingX: "control.spacing.x" = "0.55em",
        "Between two controls on a row.";
    ControlSpacingY: "control.spacing.y" = "0.35em",
        "Between two rows of controls.";
    ControlHeight: "control.height" = "2.6em",
        "The height every button, checkbox, dropdown and number box is laid out at. A fingertip is the floor.";
    ControlWidth: "control.width" = "3.2em",
        "The narrowest a number box or a color swatch is drawn.";
    ControlCheckSize: "control.check.size" = "1.2em",
        "The checkbox and radio glyph.";
    ControlCheckMark: "control.check.mark" = "0.7em",
        "The mark inside it.";
    ControlIndent: "control.indent" = "1.4em",
        "Indentation of a nested group.";
    ControlCombo: "control.combo" = "8em",
        "Width of a dropdown.";
    ControlScrollbar: "control.scrollbar" = "0.55em",
        "Width of a scroll bar. Floored in code at a width a finger can catch.";
    MenuRowHeight: "menu.row.height" = "1.3icon",
        "Height of a button on the menu page. Off the icon rather than the text: these are touch targets first.";
    MenuRowWidth: "menu.row.width" = "5icon",
        "Width of a menu button.";
    MenuRowGap: "menu.row.gap" = "0.3icon",
        "Between two menu buttons.";
    MenuText: "menu.text" = "0.45icon",
        "Text size on a menu button, which its glyph is sized to as well.";
    MapPopupPadX: "map.popup.pad.x" = "0.35icon",
        "Side padding of a button in the map's popups.";
    MapPopupPadY: "map.popup.pad.y" = "0.25icon",
        "Top and bottom padding of a button in the map's popups.";
    MapCenterGap: "map.center.gap" = "0.12icon",
        "Between two entries of the center button's marker list.";
    MapCenterWidth: "map.center.width" = "3.5icon",
        "The narrowest that list is drawn.";
    MapUnderBar: "map.under_bar" = "1.8icon",
        "How far below the controls bar the center menu hangs.";
    MapHintUnderBar: "map.hint_under_bar" = "1.6icon",
        "How far below the controls bar the region-select hint hangs.";
    MapMarkerLift: "map.marker_lift" = "0.35icon",
        "How far above a marker its info bubble floats.";
    MapDragMin: "map.drag_min" = "2.5%",
        "The smallest drag that counts as a region box rather than a tap. Also how far a held finger may wander and still count as a hold.";
    StatusGraphWidth: "status.graph.width" = "28%w",
        "Width of the status bar's signal graph.";
    StatusGraphHeight: "status.graph.height" = "1.7em",
        "Height of that graph, in text heights so it stays in proportion with the read-out beside it.";
    StatusBarGap: "status.bar.gap" = "0.25x",
        "Space between two bars of the graph, as a ratio of the slot each bar gets.";
    StatusBarMin: "status.bar.min" = "0.05x",
        "The shortest a bar is drawn, as a ratio of the graph height, so a barely heard node still shows.";
    PlotHeight: "plot.height" = "34%h",
        "Height of the Logging page's graph. Off the screen rather than the text: it is a picture, and keeps its shape when the text is scaled.";
    PlotPadLeft: "plot.pad.left" = "3.4em",
        "Room inside the graph frame for the value axis labels, in text heights: that side of it is text.";
    PlotPadBottom: "plot.pad.bottom" = "1.6em",
        "Room under the graph for the time axis.";
    PlotPadTop: "plot.pad.top" = "0.6em",
        "Room above the graph.";
    PlotPadRight: "plot.pad.right" = "1.2em",
        "Room to the right of the graph.";
    PlotDot: "plot.dot" = "0.16em",
        "Radius of a scatter dot.";
    PlotLine: "plot.line" = "0.12em",
        "Width of a plotted line.";
    PointsSearch: "points.search" = "60%w min 8em max 22em",
        "The Points page's search box, leaving room for the Clear button beside it.";
    PointsListMin: "points.list.min" = "4em",
        "The shortest the points list may be squeezed to. Below this the page would show filters over nothing.";
    SettingsPath: "settings.path" = "50%w min 8em max 22em",
        "The config path field on the Settings page.";
    SettingsSlider: "settings.slider" = "45%w",
        "The text-size slider. Off the screen rather than the text: its own label grows while it is dragged, and a width in text heights would walk out from under the finger.";
    LoggingPath: "logging.path" = "50%w min 8em max 22em",
        "The log path field on the Logging page.";
    LoggingReference: "logging.reference" = "45%w min 8em max 22em",
        "The reference coordinate field on the Logging page.";
    RadioPath: "radio.path" = "50%w min 8em max 22em",
        "The file path field on the Radio page.";
    RadioGlyph: "radio.glyph" = "0.55x",
        "The set and cancel glyphs beside a radio setting being edited, as a ratio of the control height.";
    RadioEnumField: "radio.enum_field" = "12em",
        "A radio setting's text input while it is being edited.";
    RadioConfirm: "radio.confirm" = "18em",
        "The widest the send-to-board confirmation grows.";
    BeaconName: "beacon.name" = "7em",
        "A board name input on the Beacon page.";
    BeaconNumber: "beacon.number" = "5em",
        "A number input on the Beacon page: an interval, a window, a timeout.";
    ManualField: "manual.field" = "50%w min 8em max 22em",
        "The desktop position entry field.";
}

impl Key {
    /// The key a sheet line names, if any.
    pub fn from_path(path: &str) -> Option<Key> {
        Key::ALL.iter().copied().find(|k| k.path() == path)
    }

    /// The measure the app ships with.
    pub fn default_measure(self) -> Measure {
        self.default_text()
            .parse()
            .unwrap_or_else(|e| panic!("default for {}: {e}", self.path()))
    }
}

const KEY_COUNT: usize = Key::ALL.len();

/// Every measure, as the app is drawing with them right now.
#[derive(Clone, PartialEq, Debug)]
pub struct Look {
    values: [Measure; KEY_COUNT],
}

impl Default for Look {
    fn default() -> Self {
        let mut values = [Measure::new(0.0, Unit::Times); KEY_COUNT];
        for &key in Key::ALL {
            values[key as usize] = key.default_measure();
        }
        Self { values }
    }
}

/// Where a parse of a sheet stopped, by line.
#[derive(Clone, PartialEq, Debug)]
pub struct SheetError {
    /// One-based, as an editor counts.
    pub line: usize,
    pub message: String,
}

impl fmt::Display for SheetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for SheetError {}

/// One assignment in a sheet, with where its value sits so it can be
/// rewritten in place.
struct Entry {
    path: String,
    measure: Measure,
    /// Zero-based line index.
    line: usize,
    /// Byte span of the measure text within that line.
    span: Range<usize>,
}

/// One block in a sheet: a name alone on a line, and the lines indented
/// under it.
struct Block {
    /// The dotted path with a trailing dot, so `name` under it is
    /// `prefix + name`.
    prefix: String,
    indent: usize,
    /// The indent of the block's first child, once there is one.
    child_indent: Option<usize>,
    /// The last line inside the block, where a new key is appended.
    last_line: usize,
}

struct Scan {
    entries: Vec<Entry>,
    blocks: Vec<Block>,
}

/// Leading whitespace in columns, a tab counting as four.
fn indent_of(line: &str) -> usize {
    line.chars()
        .take_while(|c| c.is_whitespace())
        .map(|c| if c == '\t' { 4 } else { 1 })
        .sum()
}

fn valid_name(name: &str) -> bool {
    name.split('.').all(|seg| {
        !seg.is_empty()
            && seg
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// Read a sheet's structure without judging its keys.
fn scan(text: &str) -> Result<Scan, SheetError> {
    let mut entries: Vec<Entry> = Vec::new();
    let mut blocks: Vec<Block> = Vec::new();
    // Indices into `blocks`, innermost last.
    let mut stack: Vec<usize> = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let fail = |message: String| SheetError {
            line: i + 1,
            message,
        };
        let content_end = raw.find('#').unwrap_or(raw.len());
        let content = &raw[..content_end];
        if content.trim().is_empty() {
            continue;
        }
        let indent = indent_of(content);
        while stack.last().is_some_and(|&b| blocks[b].indent >= indent) {
            stack.pop();
        }
        for &b in &stack {
            blocks[b].last_line = i;
        }
        if let Some(&b) = stack.last() {
            blocks[b].child_indent.get_or_insert(indent);
        }
        let prefix = stack
            .last()
            .map_or("", |&b| blocks[b].prefix.as_str())
            .to_string();

        let trimmed = content.trim_start();
        let name_start = content.len() - trimmed.len();
        let name = trimmed.split_whitespace().next().unwrap_or("");
        if !valid_name(name) {
            return Err(fail(format!(
                "`{name}` is not a name (letters, digits, `_`, joined by `.`)"
            )));
        }
        let after_name = name_start + name.len();
        let value_text = &content[after_name..];
        let tokens: Vec<&str> = value_text.split_whitespace().collect();
        if tokens.is_empty() {
            blocks.push(Block {
                prefix: format!("{prefix}{name}."),
                indent,
                child_indent: None,
                last_line: i,
            });
            stack.push(blocks.len() - 1);
            continue;
        }
        let path = format!("{prefix}{name}");
        if let Some(prev) = entries.iter().find(|e| e.path == path) {
            return Err(fail(format!(
                "`{path}` was already set on line {}",
                prev.line + 1
            )));
        }
        let measure = parse_measure(&tokens).map_err(|e| fail(format!("{path}: {e}")))?;
        let value_start = after_name + (value_text.len() - value_text.trim_start().len());
        let value_end = content.trim_end().len();
        entries.push(Entry {
            path,
            measure,
            line: i,
            span: value_start..value_end,
        });
    }
    Ok(Scan { entries, blocks })
}

/// The comment a generated sheet opens with.
const HEADER: &str = "\
# The look of gps-gui-rs: every size and spacing, each a fraction of something
# that scales with the screen or the text. Nothing here is a point count.
#
#   5%      of the smaller screen side      1.5em    of the body text height
#   50%w    of the screen width             0.7icon  of the toolbar icon side
#   34%h    of the screen height            0.25x    a plain ratio (see the note)
#
# A measure may be held in a range: `50%w min 8em max 22em`. A name alone on
# a line opens a block; `page.gap.item` and `item` under `gap` under `page`
# are the same key. A missing key keeps its default.
";

/// Break `text` into lines no wider than `width`, on spaces.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.len() + 1 + word.len() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

impl Look {
    /// The measure behind a key.
    pub fn get(&self, key: Key) -> Measure {
        self.values[key as usize]
    }

    pub fn set(&mut self, key: Key, measure: Measure) {
        self.values[key as usize] = measure;
    }

    /// The scale for a frame: the screen, the text height, and the icon side
    /// the sheet's own `icon.size` gives once the physical bounds are applied.
    pub fn scale(&self, screen: egui::Rect, em: f32) -> Scale {
        let base = Scale {
            screen,
            em,
            icon: 0.0,
        };
        let icon = self
            .get(Key::IconSize)
            .eval(&base)
            .clamp(TOUCH_MIN, ICON_MAX);
        Scale { screen, em, icon }
    }

    /// A key in points under `scale`.
    pub fn px(&self, key: Key, scale: &Scale) -> f32 {
        self.get(key).eval(scale)
    }

    /// Read a sheet over the defaults. Unknown keys come back as warnings,
    /// one per line; a malformed line is the error.
    pub fn from_sheet(text: &str) -> Result<(Look, Vec<String>), SheetError> {
        let scan = scan(text)?;
        let mut look = Look::default();
        let mut warnings = Vec::new();
        for e in &scan.entries {
            match Key::from_path(&e.path) {
                Some(Key::IconSize) if e.measure.uses(Unit::Icon) => {
                    return Err(SheetError {
                        line: e.line + 1,
                        message: "icon.size cannot be measured in icons".to_string(),
                    });
                }
                Some(key) => look.set(key, e.measure),
                None => warnings.push(format!("line {}: unknown key `{}`", e.line + 1, e.path)),
            }
        }
        Ok((look, warnings))
    }

    /// Read the sheet at `path`.
    pub fn load(path: &str) -> Result<(Look, Vec<String>), String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        Look::from_sheet(&text).map_err(|e| format!("{path}: {e}"))
    }

    /// The whole sheet, documented, as the app would write it fresh.
    pub fn to_sheet(&self) -> String {
        let mut out = String::from(HEADER);
        let mut open: Vec<&str> = Vec::new();
        for &key in Key::ALL {
            let segs: Vec<&str> = key.path().split('.').collect();
            let (leaf, parents) = segs.split_last().expect("a path has a leaf");
            let common = open
                .iter()
                .zip(parents)
                .take_while(|(a, b)| a == b)
                .count();
            open.truncate(common);
            for seg in &parents[common..] {
                if open.is_empty() {
                    out.push('\n');
                }
                out.push_str(&"    ".repeat(open.len()));
                out.push_str(seg);
                out.push('\n');
                open.push(seg);
            }
            let indent = "    ".repeat(open.len());
            for line in wrap(key.doc(), 78usize.saturating_sub(indent.len() + 2)) {
                out.push_str(&format!("{indent}# {line}\n"));
            }
            // Values line up within a block: pad each name to the longest
            // leaf that shares its parents.
            let width = Key::ALL
                .iter()
                .filter(|k| k.path().rsplit_once('.').map(|(p, _)| p) == key.path().rsplit_once('.').map(|(p, _)| p))
                .map(|k| k.path().rsplit('.').next().unwrap_or("").len())
                .max()
                .unwrap_or(0);
            out.push_str(&format!("{indent}{leaf:<width$}  {}\n", self.get(key)));
        }
        out
    }

    /// Write `keys` into an existing sheet's text: a key already there has
    /// its value rewritten in place, and one that is not is added at the end
    /// of the deepest block it belongs under, or at the end of the file.
    pub fn edit_sheet(&self, text: &str, keys: &[Key]) -> Result<String, SheetError> {
        let scan = scan(text)?;
        let mut lines: Vec<String> = text.lines().map(String::from).collect();
        let mut inserts: Vec<(usize, String)> = Vec::new();
        let mut tail: Vec<String> = Vec::new();
        for &key in keys {
            let value = self.get(key).to_string();
            if let Some(e) = scan.entries.iter().find(|e| e.path == key.path()) {
                let raw = &lines[e.line];
                lines[e.line] = format!("{}{}{}", &raw[..e.span.start], value, &raw[e.span.end..]);
                continue;
            }
            let block = scan
                .blocks
                .iter()
                .filter(|b| key.path().starts_with(&b.prefix))
                .max_by_key(|b| b.prefix.len());
            match block {
                Some(b) => {
                    let indent = b.child_indent.unwrap_or(b.indent + 4);
                    let name = &key.path()[b.prefix.len()..];
                    inserts.push((b.last_line + 1, format!("{}{name}  {value}", " ".repeat(indent))));
                }
                None => tail.push(format!("{}  {value}", key.path())),
            }
        }
        // From the bottom up, so earlier indices stay right; a group landing on
        // one spot keeps its own order.
        inserts.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
        let mut i = 0;
        while i < inserts.len() {
            let at = inserts[i].0;
            let mut j = i;
            while j < inserts.len() && inserts[j].0 == at {
                j += 1;
            }
            for (k, (_, line)) in inserts[i..j].iter().enumerate() {
                lines.insert(at + k, line.clone());
            }
            i = j;
        }
        lines.extend(tail);
        let mut out = lines.join("\n");
        if !out.is_empty() {
            out.push('\n');
        }
        Ok(out)
    }

    /// Write the look to `path`. An existing sheet is edited in place - only
    /// the keys whose value differs from what it holds are touched, and a key
    /// it does not hold is only added when off its default - and a missing
    /// one is generated whole. `Ok(true)` when the file was created.
    pub fn save(&self, path: &str) -> Result<bool, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let scan = scan(&text).map_err(|e| format!("{path}: {e}"))?;
                let defaults = Look::default();
                let changed: Vec<Key> = Key::ALL
                    .iter()
                    .copied()
                    .filter(|&k| match scan.entries.iter().find(|e| e.path == k.path()) {
                        Some(e) => e.measure != self.get(k),
                        None => self.get(k) != defaults.get(k),
                    })
                    .collect();
                let out = self
                    .edit_sheet(&text, &changed)
                    .map_err(|e| format!("{path}: {e}"))?;
                std::fs::write(path, out).map_err(|e| format!("{path}: {e}"))?;
                Ok(false)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                std::fs::write(path, self.to_sheet()).map_err(|e| format!("{path}: {e}"))?;
                Ok(true)
            }
            Err(e) => Err(format!("{path}: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scale() -> Scale {
        Scale {
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 800.0)),
            em: 10.0,
            icon: 40.0,
        }
    }

    fn m(s: &str) -> Measure {
        s.parse().unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    #[test]
    fn every_default_parses_and_icon_size_is_not_circular() {
        for &key in Key::ALL {
            let d = key.default_measure();
            assert!(!key.doc().is_empty(), "{} has no note", key.path());
            assert!(!d.uses(Unit::Icon) || key != Key::IconSize);
        }
        assert_eq!(Key::from_path("page.gap.item"), Some(Key::GapItem));
        assert_eq!(Key::from_path("page.gap"), None);
    }

    #[test]
    fn paths_are_unique_and_never_both_a_block_and_a_key() {
        for &a in Key::ALL {
            for &b in Key::ALL {
                if a != b {
                    assert_ne!(a.path(), b.path());
                    assert!(
                        !b.path().starts_with(&format!("{}.", a.path())),
                        "{} is both a key and a block over {}",
                        a.path(),
                        b.path()
                    );
                }
            }
        }
    }

    #[test]
    fn measure_text_round_trips() {
        for text in ["2.5%", "0.25em", "5icon", "0.25x", "50%w min 8em max 22em", "34%h max 3icon"] {
            assert_eq!(m(text).to_string(), text);
        }
        assert_eq!(m("1.000em").to_string(), "1em");
        assert_eq!(m("0.50x").to_string(), "0.5x");
    }

    #[test]
    fn measure_rejects_the_unitless_and_the_fixed() {
        assert!("5".parse::<Measure>().is_err());
        assert!("5pt".parse::<Measure>().is_err());
        assert!("5px".parse::<Measure>().is_err());
        assert!("-1em".parse::<Measure>().is_err());
        assert!("5em min".parse::<Measure>().is_err());
        assert!("5em foo 1em".parse::<Measure>().is_err());
        assert!("5em min 1em min 2em".parse::<Measure>().is_err());
        assert!("".parse::<Measure>().is_err());
    }

    #[test]
    fn eval_uses_the_unit_and_the_range() {
        let s = scale();
        assert_eq!(m("5%").eval(&s), 20.0);
        assert_eq!(m("50%w").eval(&s), 200.0);
        assert_eq!(m("34%h").eval(&s), 272.0);
        assert_eq!(m("1.5em").eval(&s), 15.0);
        assert_eq!(m("0.7icon").eval(&s), 28.0);
        assert_eq!(m("0.25x").eval(&s), 0.25);
        assert_eq!(m("50%w min 8em max 22em").eval(&s), 200.0);
        let narrow = Scale {
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(100.0, 800.0)),
            ..s
        };
        assert_eq!(m("50%w min 8em max 22em").eval(&narrow), 80.0);
        let wide = Scale {
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 800.0)),
            ..s
        };
        assert_eq!(m("50%w min 8em max 22em").eval(&wide), 220.0);
    }

    #[test]
    fn convert_keeps_the_points_and_changes_the_unit() {
        let s = scale();
        let mut q = m("2em");
        q.convert(Unit::Screen, &s);
        assert_eq!(q.to_string(), "5%");
        q.convert(Unit::Icon, &s);
        assert_eq!(q.to_string(), "0.5icon");
        q.convert(Unit::Times, &s);
        assert_eq!(q.to_string(), "20x");
        // The range is left as it was: it is a bound, not the measure.
        let mut ranged = m("50%w min 8em max 22em");
        ranged.convert(Unit::Em, &s);
        assert_eq!(ranged.to_string(), "20em min 8em max 22em");
        // Nothing behind the unit: the number is kept rather than zeroed.
        let no_icon = Scale { icon: 0.0, ..s };
        let mut q = m("2em");
        q.convert(Unit::Icon, &no_icon);
        assert_eq!(q.to_string(), "2icon");
    }

    #[test]
    fn the_icon_is_held_between_a_fingertip_and_the_cap() {
        let look = Look::default();
        let small = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300.0, 500.0));
        assert_eq!(look.scale(small, 10.0).icon, TOUCH_MIN);
        let big = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(3000.0, 2000.0));
        assert_eq!(look.scale(big, 10.0).icon, ICON_MAX);
        let mid = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 1200.0));
        assert_eq!(look.scale(mid, 10.0).icon, 50.0);
    }

    #[test]
    fn the_shipped_sheet_reads_clean() {
        let text = include_str!("../gps-gui.look");
        let (_, warnings) = Look::from_sheet(text).unwrap_or_else(|e| panic!("gps-gui.look: {e}"));
        assert!(warnings.is_empty(), "gps-gui.look: {warnings:?}");
    }

    #[test]
    fn generated_sheet_round_trips() {
        let look = Look::default();
        let text = look.to_sheet();
        let (back, warnings) = Look::from_sheet(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(back, look);
        // Every key is on the sheet by its leaf name under its block.
        assert!(text.contains("\npage\n"));
        assert!(text.contains("    margin  2.5%\n"));
        assert!(text.contains("        item     0.5em\n"));
    }

    #[test]
    fn blocks_nest_and_dots_do_the_same() {
        let text = "page\n    gap\n        item 1em\n    margin 3%\nbar.gap 0.2icon\n\ncontrol\n\tpad.x 1em\n";
        let (look, warnings) = Look::from_sheet(text).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(look.get(Key::GapItem), m("1em"));
        assert_eq!(look.get(Key::PageMargin), m("3%"));
        assert_eq!(look.get(Key::BarGap), m("0.2icon"));
        assert_eq!(look.get(Key::ControlPadX), m("1em"));
        // Untouched keys keep their defaults.
        assert_eq!(look.get(Key::GapHair), Key::GapHair.default_measure());
    }

    #[test]
    fn unknown_keys_warn_and_repeats_fail() {
        let (look, warnings) = Look::from_sheet("page\n    margin 3%\n    wobble 1em\n").unwrap();
        assert_eq!(look.get(Key::PageMargin), m("3%"));
        assert_eq!(warnings, vec!["line 3: unknown key `page.wobble`"]);

        let err = Look::from_sheet("page.margin 3%\npage\n    margin 4%\n").unwrap_err();
        assert_eq!(err.line, 3);
        assert!(err.message.contains("line 1"), "{err}");
    }

    #[test]
    fn malformed_lines_name_their_line() {
        let err = Look::from_sheet("page\n    margin 3\n").unwrap_err();
        assert_eq!(err.line, 2);
        assert!(err.message.contains("no unit"), "{err}");
        let err = Look::from_sheet("pa ge margin 3%\n").unwrap_err();
        assert_eq!(err.line, 1);
        let err = Look::from_sheet("icon\n    size 1icon\n").unwrap_err();
        assert_eq!(err.line, 2);
    }

    #[test]
    fn comments_and_blank_lines_are_skipped() {
        let text = "# a sheet\n\npage   # the pages\n    margin 3%  # roomy\n\n";
        let (look, _) = Look::from_sheet(text).unwrap();
        assert_eq!(look.get(Key::PageMargin), m("3%"));
    }

    #[test]
    fn edit_rewrites_only_the_value_and_keeps_the_rest() {
        let text = "# my sheet\npage\n    margin    2.5%   # roomy\n    gap\n        item 0.5em\n\nbar\n    gap 0.15icon\n";
        let mut look = Look::default();
        look.set(Key::PageMargin, m("4%"));
        look.set(Key::GapHair, m("0.3em"));
        look.set(Key::BarMarginX, m("3%"));
        look.set(Key::BeaconName, m("9em"));
        let out = look
            .edit_sheet(text, &[Key::PageMargin, Key::GapHair, Key::BarMarginX, Key::BeaconName])
            .unwrap();
        let expected = "# my sheet\npage\n    margin    4%   # roomy\n    gap\n        item 0.5em\n        hair  0.3em\n\nbar\n    gap 0.15icon\n    margin.x  3%\nbeacon.name  9em\n";
        assert_eq!(out, expected);
        // And what was written reads back as what was meant.
        let (back, warnings) = Look::from_sheet(&out).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(back.get(Key::PageMargin), m("4%"));
        assert_eq!(back.get(Key::GapHair), m("0.3em"));
        assert_eq!(back.get(Key::BarMarginX), m("3%"));
        assert_eq!(back.get(Key::BeaconName), m("9em"));
    }

    #[test]
    fn edit_keeps_a_range_the_file_had() {
        let text = "settings\n    path 50%w min 8em max 22em\n";
        let (mut look, _) = Look::from_sheet(text).unwrap();
        let mut path = look.get(Key::SettingsPath);
        path.q.value = 60.0;
        look.set(Key::SettingsPath, path);
        let out = look.edit_sheet(text, &[Key::SettingsPath]).unwrap();
        assert_eq!(out, "settings\n    path 60%w min 8em max 22em\n");
    }

    #[test]
    fn save_creates_then_edits_in_place() {
        let dir = std::env::temp_dir().join(format!("gps-gui-look-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.look");
        let path = path.to_str().unwrap();
        let _ = std::fs::remove_file(path);

        let mut look = Look::default();
        assert_eq!(look.save(path), Ok(true));
        let first = std::fs::read_to_string(path).unwrap();
        assert!(first.starts_with("# The look of gps-gui-rs"));

        look.set(Key::PageMargin, m("4%"));
        assert_eq!(look.save(path), Ok(false));
        let second = std::fs::read_to_string(path).unwrap();
        assert!(second.starts_with("# The look of gps-gui-rs"));
        assert!(second.contains("    margin  4%\n"));
        let (back, _) = Look::load(path).unwrap();
        assert_eq!(back, look);

        // A file that only names a few keys stays short: saving a look that
        // is otherwise at its defaults adds nothing to it.
        std::fs::write(path, "page\n    margin 3%\n").unwrap();
        let mut look = Look::default();
        look.set(Key::PageMargin, m("3%"));
        assert_eq!(look.save(path), Ok(false));
        assert_eq!(std::fs::read_to_string(path).unwrap(), "page\n    margin 3%\n");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
