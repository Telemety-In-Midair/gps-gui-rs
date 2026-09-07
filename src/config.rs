//! Loadable TOML configuration: marker colors, overlay sizes, the distance
//! readout, track recording, and the BLE node settings.
//!
//! The Settings page edits these live and writes them back with [`AppConfig::save`],
//! which edits an existing file in place (comments, key order, and keys this app
//! does not know about all survive) and generates a documented one from
//! [`AppConfig::to_toml`] when there is nothing there yet.
//!
//! Schema (all fields optional; missing ones keep their defaults):
//!
//! ```toml
//! [colors]
//! track = "#0078ff"   # your track, heading arrow, position dot
//! fixed = "#ff5028"   # the connected node's marker, distance line and path
//! outline = "#ffffff" # ring around the position and node dots
//!
//! [phone]             # this device
//! name = "Phone"      # what the map and the pages call it
//! location = false    # use its own GNSS for the current position
//!
//! [ui]                # the pages rather than the map: their colors and text
//! theme = "light"     # "light", "dark" (both solarized) or "system"
//! ok = "#859900"      # "yes" and the green feedback lines
//! error = "#dc322f"   # "no", errors, and the red feedback lines
//! busy = "#268bd2"    # scanning and connecting
//! pulse = "#cb4b16"   # a toolbar button flagging that it has no target
//! background = ""     # pages, map bar and popups; empty follows the theme
//! button = ""         # a button at rest; empty follows the theme
//! text = ""           # body text and the toolbar glyphs; empty follows the theme
//! text_scale = 1.0    # multiplier on the page text; 1.0 is the default size
//!
//! [sizes]             # screen points; each overlay is sized independently
//! marker = 8.0        # current-position dot radius
//! beacon = 6.0        # node dot radius
//! track = 3.0         # track polyline width (your track and the node paths)
//! distance_line = 3.0 # user<->node line width
//! distance_text = 14.0 # distance label font size
//!
//! [distance]
//! show = false        # draw the distance label on the user<->node line
//! units = "metric"    # "metric" (km/m) or "imperial" (mi/ft)
//! dotted = true       # draw the user<->node line dotted rather than solid
//!
//! [map]               # the map page: its tiles and its overlays
//! tiles = "osm"       # "osm" (OpenStreetMap, OpenTopoMap) or "arcgis" (Esri
//!                     # streets, outdoor and satellite, under an API key)
//! arcgis_key = ""     # your own ArcGIS key; empty uses the built-in one
//! bar_opacity = 1.0   # the bottom bar and the status bar backgrounds, 0 - 1
//! show_key = true     # the color key in the top left corner
//!
//! [ble]
//! enabled = true      # master switch for the BLE GPS source
//! show_on_map = true  # draw the connected node on the map at all
//! show_path = false   # draw the path of the incoming BLE GPS data
//! location = true     # use the connected node's GPS as the current position
//! mac = "AA:BB:CC:DD:EE:FF"  # pin a specific node; omit to scan by service
//!
//! [ble.names]         # names for known nodes, keyed by MAC; a node reporting
//!                     # a name of its own writes it here
//! "AA:BB:CC:DD:EE:FF" = "Truck"
//!
//! [lora]
//! show_path = true    # draw the remote LoRa nodes' paths on the map
//! pulse_secs = 10.0   # a node's marker pulses while heard within this; 0 never
//!
//! [lora.names]        # nicknames for remote nodes, keyed by LoRa address
//! 3 = "Truck"
//!
//! [track]
//! min_distance = 3.0  # meters of movement before a new track point is recorded
//! show_path = true    # draw your own recorded path
//!
//! [compass]
//! marker_arrow = true # point the marker arrow with the compass outside heading-up
//! arrow_hz = 4.0      # compass rate while only that arrow needs it
//!
//! [status_bar]        # the read-out along the bottom of the map
//! show = false        # draw it at all
//! cycle_secs = 5.0    # seconds each node holds the read-out when several are heard
//! rssi_top_dbm = -20  # a reception this strong fills the graph
//! rssi_bottom_dbm = -120 # and this weak is empty; heights are linear in dBm
//!
//! [log]
//! auto_start = false  # start recording the CSV log as soon as the app launches
//! file = ""           # log path; empty means a timestamped file beside this one
//! ref_lat = 51.4779   # fixed reference coordinate; distance to it is a logged
//! ref_lon = -0.0015   # column. Omit both to leave it unset.
//! ```

use std::collections::BTreeMap;

use egui::Color32;
use serde::Deserialize;
use toml_edit::{DocumentMut, Item, Table, Value};

use crate::solarized;
use crate::tiles::TileProvider;

/// Colors used to draw the map markers.
#[derive(Clone, Copy)]
pub struct MarkerColors {
    /// Track polyline, heading arrow, and the current-position dot.
    pub track: Color32,
    /// The connected node's marker, the line drawn to it, and its path.
    pub fixed: Color32,
    /// Ring around both dots, which is what keeps them apart from the tiles
    /// under them whatever the base map looks like there.
    pub outline: Color32,
}

impl Default for MarkerColors {
    fn default() -> Self {
        Self {
            track: Color32::from_rgb(0, 120, 255),
            fixed: Color32::from_rgb(255, 80, 40),
            outline: Color32::WHITE,
        }
    }
}

/// Distinct colors for the remote LoRa nodes, cycled by address so each node
/// keeps a stable color across a session. Deliberately not in the config: these
/// exist to tell many nodes apart at a glance rather than to carry a meaning
/// worth editing, and they steer clear of the phone track's blue and the
/// connected board's orange-red so the three groups never read as the same.
pub const REMOTE_PALETTE: [Color32; 8] = [
    Color32::from_rgb(255, 165, 0),   // orange
    Color32::from_rgb(60, 180, 75),   // green
    Color32::from_rgb(145, 30, 180),  // purple
    Color32::from_rgb(70, 190, 220),  // cyan
    Color32::from_rgb(240, 50, 230),  // magenta
    Color32::from_rgb(0, 128, 128),   // teal
    Color32::from_rgb(210, 245, 60),  // lime
    Color32::from_rgb(245, 130, 200), // pink
];

/// The palette color for a remote node at LoRa address `addr`. Two addresses a
/// palette-length apart share a color; with a handful of nodes in the air that
/// is rare, and the marker's own position tells them apart regardless.
pub fn remote_color(addr: u8) -> Color32 {
    REMOTE_PALETTE[addr as usize % REMOTE_PALETTE.len()]
}

/// Which theme the pages are drawn in. Both are solarized
/// ([`crate::solarized`]); the choice is which end of its monotone run is the
/// page and which is the text.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ThemeChoice {
    /// Solarized light, and the app's default: these pages are read outdoors
    /// more often than not.
    #[default]
    Light,
    /// Solarized dark.
    Dark,
    /// Whichever of the two the desktop or the phone is set to, and it follows
    /// that setting while the app runs.
    System,
}

impl ThemeChoice {
    /// Every choice, in the order the Settings page offers them.
    pub const ALL: [Self; 3] = [Self::Light, Self::Dark, Self::System];

    /// The TOML spelling, the one [`Self::parse`] reads back.
    pub fn as_str(self) -> &'static str {
        match self {
            ThemeChoice::Light => "light",
            ThemeChoice::Dark => "dark",
            ThemeChoice::System => "system",
        }
    }

    /// The label on the Settings page's picker.
    pub fn label(self) -> &'static str {
        match self {
            ThemeChoice::Light => "Light",
            ThemeChoice::Dark => "Dark",
            ThemeChoice::System => "System",
        }
    }

    /// What egui resolves the drawn theme through. `System` is the only one
    /// that can change without the setting changing, the window manager owning
    /// the answer.
    pub fn preference(self) -> egui::ThemePreference {
        match self {
            ThemeChoice::Light => egui::ThemePreference::Light,
            ThemeChoice::Dark => egui::ThemePreference::Dark,
            ThemeChoice::System => egui::ThemePreference::System,
        }
    }

    /// Parse the TOML `ui.theme` string (case-insensitive).
    fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_lowercase().as_str() {
            "light" => Ok(ThemeChoice::Light),
            "dark" => Ok(ThemeChoice::Dark),
            "system" | "auto" => Ok(ThemeChoice::System),
            other => Err(format!(
                "invalid ui.theme {other:?}, expected \"light\", \"dark\" or \"system\""
            )),
        }
    }
}

/// The pages rather than the map: which theme they are drawn in, the feedback
/// and status lines, the pulse on a toolbar button that has nothing to act on,
/// the surfaces and text everything else is drawn with, and how big that text
/// is.
///
/// `theme` picks the ground everything else sits on. The three colors after it
/// carry meaning by color and so are always set. The three after those are
/// overrides of that theme and default to `None`, which leaves the theme's own.
/// They are independent, so setting only one of them is a way to end up with
/// text that cannot be read on its background - the theme is what keeps them in
/// step, and an override is a promise to do that by hand.
#[derive(Clone, Copy)]
pub struct UiSettings {
    /// Which of the two themes the pages are drawn in.
    pub theme: ThemeChoice,
    /// "yes" on the Status page, and the `Ok` feedback lines.
    pub ok: Color32,
    /// "no" on the Status page, error text, and the `Err` feedback lines.
    pub error: Color32,
    /// The link lines while the app is scanning or connecting: something is
    /// in progress and not yet either of the two above.
    pub busy: Color32,
    /// Background pulse on a toolbar button with no target.
    pub pulse: Color32,
    /// Fill behind the pages, the map's controls bar and the popups. `None`
    /// keeps the theme's own.
    pub background: Option<Color32>,
    /// Fill of a button at rest; the hover and press shades are derived from it.
    /// `None` keeps the theme's own.
    pub button: Option<Color32>,
    /// Body text, and with it the toolbar glyphs and the weak and strong
    /// shades derived from it. `None` keeps the theme's own.
    pub text: Option<Color32>,
    /// Multiplier on every text style, for reading the pages at arm's length or
    /// on a dense phone screen. `1.0` is egui's own sizes.
    ///
    /// It carries the page layout with it: the gaps and input widths are
    /// measured in text heights, so they grow with the text rather than leaving
    /// larger glyphs in the same cramped rows. The map's icons and overlays keep
    /// their own sizes - a touch target and a marker dot are not text.
    pub text_scale: f32,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::Light,
            // The palette's own green, red and orange: these three are the
            // only page colors the theme does not reach, so leaving them on
            // an unrelated set of primaries is what would look wrong.
            ok: solarized::GREEN,
            error: solarized::RED,
            busy: solarized::BLUE,
            pulse: solarized::ORANGE,
            background: None,
            button: None,
            text: None,
            text_scale: 1.0,
        }
    }
}

/// Sizes, in screen points, for the drawn map overlays. Each is independent so
/// the markers, lines, and label can be tuned separately.
#[derive(Clone, Copy)]
pub struct MarkerSizes {
    /// Radius of the current-position dot.
    pub marker: f32,
    /// Radius of a node dot, the connected node's and the remote ones'.
    pub beacon: f32,
    /// Width of the recorded track polylines (your track and the node paths).
    pub track: f32,
    /// Width of the line drawn from the current position to the node.
    pub distance_line: f32,
    /// Font size of the distance label drawn on that line.
    pub distance_text: f32,
}

impl Default for MarkerSizes {
    fn default() -> Self {
        Self {
            marker: 8.0,
            beacon: 6.0,
            track: 3.0,
            distance_line: 3.0,
            distance_text: 14.0,
        }
    }
}

/// This device: what it is called, and whether its own receiver is used.
#[derive(Clone)]
pub struct PhoneSettings {
    /// What the map's marker, the Points page and the log legend call this
    /// device. "Phone" by default, since that is what it is in the field.
    pub name: String,
    /// Use the device's own GNSS for the current position. Off by default:
    /// the usual setup is a node held next to the phone whose receiver is
    /// the better one, and the phone's costs battery for a second answer.
    /// See `[ble] location` for that node.
    pub location: bool,
}

impl Default for PhoneSettings {
    fn default() -> Self {
        Self {
            name: DEFAULT_PHONE_NAME.to_string(),
            location: false,
        }
    }
}

/// What this device is called when nobody has named it.
pub const DEFAULT_PHONE_NAME: &str = "Phone";

/// The map page: who serves its tiles, and its own overlays - how
/// see-through its two bars are, and whether the color key is drawn.
#[derive(Clone)]
pub struct MapSettings {
    /// Who serves the tiles. ArcGIS is the provider with satellite imagery.
    pub tiles: TileProvider,
    /// An ArcGIS API key of your own; empty means the built-in one
    /// ([`crate::tiles::DEFAULT_ARCGIS_KEY`]). Only read while `tiles` is
    /// ArcGIS.
    pub arcgis_key: String,
    /// Opacity of the controls bar and the status bar backgrounds, 0 (the
    /// map shows through) to 1 (solid).
    pub bar_opacity: f32,
    /// Draw the key under the controls bar: one row per marker color.
    pub show_key: bool,
}

impl Default for MapSettings {
    fn default() -> Self {
        Self {
            tiles: TileProvider::Osm,
            arcgis_key: String::new(),
            bar_opacity: 1.0,
            show_key: true,
        }
    }
}

/// Unit system for the distance label.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DistanceUnits {
    /// Kilometers and meters.
    Metric,
    /// Miles and feet.
    Imperial,
}

impl DistanceUnits {
    /// Format a distance given in meters: the larger unit once past one of it,
    /// otherwise the smaller whole unit.
    pub fn format(self, meters: f64) -> String {
        match self {
            DistanceUnits::Metric => {
                if meters >= 1000.0 {
                    format!("{:.2} km", meters / 1000.0)
                } else {
                    format!("{meters:.0} m")
                }
            }
            DistanceUnits::Imperial => {
                const M_PER_MILE: f64 = 1609.344;
                const FT_PER_M: f64 = 3.280_84;
                if meters >= M_PER_MILE {
                    format!("{:.2} mi", meters / M_PER_MILE)
                } else {
                    format!("{:.0} ft", meters * FT_PER_M)
                }
            }
        }
    }

    /// The TOML spelling, the one [`Self::parse`] reads back.
    pub fn as_str(self) -> &'static str {
        match self {
            DistanceUnits::Metric => "metric",
            DistanceUnits::Imperial => "imperial",
        }
    }

    /// Parse the TOML `distance.units` string (case-insensitive).
    fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_lowercase().as_str() {
            "metric" | "km" | "km/m" | "m" => Ok(DistanceUnits::Metric),
            "imperial" | "mi" | "mi/ft" | "ft" => Ok(DistanceUnits::Imperial),
            other => Err(format!(
                "invalid distance.units {other:?}, expected \"metric\" or \"imperial\""
            )),
        }
    }
}

/// Beacon-distance readout settings.
#[derive(Clone, Copy)]
pub struct DistanceSettings {
    /// Draw the distance label on the line to the beacon.
    pub show: bool,
    /// Which unit system the label uses.
    pub units: DistanceUnits,
    /// Draw the line to the beacon dotted rather than solid.
    pub dotted: bool,
}

impl Default for DistanceSettings {
    fn default() -> Self {
        Self {
            show: false,
            units: DistanceUnits::Metric,
            dotted: true,
        }
    }
}

/// BLE beacon settings.
#[derive(Clone)]
pub struct BleSettings {
    /// Connect to the beacon at all.
    pub enabled: bool,
    /// Draw the connected board on the map at all: its marker, heartbeat,
    /// path and the distance line to it. Off, the map is the phone and the
    /// remote nodes only; the link, the recording and the Status page carry
    /// on regardless, since a board held in the hand next to the phone is
    /// a marker on top of your own and nothing else.
    pub show_on_map: bool,
    /// Draw the path of the incoming BLE GPS data on the map.
    pub show_path: bool,
    /// Use the connected node's GPS as this device's current position while
    /// it has a fix. On by default, which with `[phone] location` off is
    /// what makes the node beside the phone the one receiver in use.
    pub location: bool,
    /// Pin a specific device MAC; `None` scans for the GPS service.
    pub mac: Option<String>,
    /// Nicknames for known boards, keyed by MAC. A board that has not been
    /// named advertises by its address, and a C3 beacon by a name every C3
    /// shares, so these are what tell such boards apart in the picker. A
    /// name stored on the board itself wins over the nickname wherever the
    /// board is labelled. Keys are normalized by [`normalize_mac`] so
    /// lookups ignore case and separator style.
    pub names: BTreeMap<String, String>,
}

impl Default for BleSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            show_on_map: true,
            show_path: false,
            location: true,
            mac: None,
            names: BTreeMap::new(),
        }
    }
}

impl BleSettings {
    /// The nickname for `mac`, if one is set.
    pub fn name_of(&self, mac: &str) -> Option<&str> {
        self.names.get(&normalize_mac(mac)).map(String::as_str)
    }

    /// Name `mac`, or forget it when `name` is blank - clearing the box is how
    /// a board leaves the picker.
    pub fn set_name(&mut self, mac: &str, name: &str) {
        let key = normalize_mac(mac);
        let name = name.trim();
        if name.is_empty() {
            self.names.remove(&key);
        } else {
            self.names.insert(key, name.to_string());
        }
    }

    /// How this device should be labelled: its nickname, or the MAC itself
    /// when it has none.
    pub fn label_of(&self, mac: &str) -> String {
        self.name_of(mac).unwrap_or(mac).to_string()
    }

    /// True when `mac` is the pinned device, comparing normalized.
    pub fn is_selected(&self, mac: &str) -> bool {
        self.mac.as_deref().map(normalize_mac) == Some(normalize_mac(mac))
    }
}

/// A MAC as a table key: uppercase, colon-separated. Boards report addresses in
/// whatever case their stack prefers and a hand-typed one may use dashes, so
/// keying on the raw string would file the same board twice.
pub fn normalize_mac(mac: &str) -> String {
    mac.trim()
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect::<String>()
        .to_ascii_uppercase()
        .as_bytes()
        .chunks(2)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect::<Vec<_>>()
        .join(":")
}

/// Remote LoRa node settings: whether to draw their paths, and their
/// nicknames.
///
/// A node's identity is its LoRa address (1-255), a namespace of its own,
/// separate from the BLE MACs the [`BleSettings`] nicknames key on: the same
/// physical board could be reached over BLE by MAC and heard over LoRa by
/// address, and the two never meet in one table.
#[derive(Clone)]
pub struct LoraSettings {
    /// Draw the remote nodes' paths on the map. The per-node color is
    /// separate; this only hides the lines.
    pub show_path: bool,
    /// How long after a node was last heard its marker keeps pulsing, in
    /// seconds; 0 never pulses. The pulse is what says "on the air now"
    /// rather than "last seen here".
    pub pulse_secs: f32,
    /// Nicknames for known nodes, keyed by LoRa address.
    pub names: BTreeMap<u8, String>,
}

impl Default for LoraSettings {
    fn default() -> Self {
        // Shown by default: relaying these onto the map is the whole reason the
        // nodes are received, unlike the connected board's own path.
        Self {
            show_path: true,
            pulse_secs: DEFAULT_PULSE_SECS,
            names: BTreeMap::new(),
        }
    }
}

/// How long a remote node's marker pulses after it was last heard.
pub const DEFAULT_PULSE_SECS: f32 = 10.0;

impl LoraSettings {
    /// The nickname for `addr`, if one is set.
    pub fn name_of(&self, addr: u8) -> Option<&str> {
        self.names.get(&addr).map(String::as_str)
    }

    /// Name `addr`, or forget it when `name` is blank.
    pub fn set_name(&mut self, addr: u8, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            self.names.remove(&addr);
        } else {
            self.names.insert(addr, name.to_string());
        }
    }

    /// How this node should be labelled: its nickname, or "Node N" when it has
    /// none. This is the one place a remote's nickname is resolved, so every
    /// page names the same node the same way.
    pub fn label_of(&self, addr: u8) -> String {
        self.name_of(addr)
            .map(str::to_string)
            .unwrap_or_else(|| format!("Node {addr}"))
    }
}

/// Track recording settings.
#[derive(Clone, Copy)]
pub struct TrackSettings {
    /// Minimum distance in meters from the last recorded point before another
    /// is appended to a track. Decimates GPS jitter; 0 records every fix.
    pub min_distance: f64,
    /// Draw the phone's own recorded path on the map. Recording is unaffected:
    /// the points are kept either way, this only hides the line.
    pub show_path: bool,
}

impl Default for TrackSettings {
    fn default() -> Self {
        Self {
            min_distance: 3.0,
            show_path: true,
        }
    }
}

/// Compass (rotation-vector sensor) settings.
///
/// Heading-up turns the whole map and always runs the sensor at its full rate.
/// These control the other modes - north-up and tracking - where the sensor is
/// only pointing the marker's heading arrow and can run far slower. The sensor
/// is fused from the accelerometer, gyroscope and magnetometer, so the rate is
/// what the arrow costs in battery.
#[derive(Clone, Copy)]
pub struct CompassSettings {
    /// Point the marker's heading arrow with the compass in north-up and
    /// tracking modes, rather than falling back to GPS course over ground.
    pub marker_arrow: bool,
    /// Sensor rate, in Hz, while only that arrow needs it.
    pub arrow_hz: f32,
}

impl Default for CompassSettings {
    fn default() -> Self {
        Self {
            marker_arrow: true,
            arrow_hz: 4.0,
        }
    }
}

/// The map's bottom status bar: the recent-signal graph and the per-node
/// read-out beside it.
///
/// Off by default. It covers a strip of the map with a read-out that only
/// means anything once a node is being heard, so it is something to turn on
/// for a run rather than the map's resting state.
#[derive(Clone, Copy)]
pub struct StatusBarSettings {
    /// Draw the bar along the bottom of the map.
    pub show: bool,
    /// Seconds each node holds the read-out before the next one takes it.
    /// Only used when more than one node has been heard; a single node keeps
    /// the read-out to itself and never cycles.
    pub cycle_secs: f32,
    /// The signal that fills a bar of the graph, in dBm.
    pub rssi_top_dbm: i16,
    /// The signal that leaves a bar empty, in dBm. Heights are linear in
    /// dBm between the two, which is the log scale of the received power:
    /// each equal step up a bar is the same ratio of power.
    pub rssi_bottom_dbm: i16,
}

impl Default for StatusBarSettings {
    fn default() -> Self {
        Self {
            show: false,
            cycle_secs: 5.0,
            rssi_top_dbm: RSSI_TOP_DEFAULT_DBM,
            rssi_bottom_dbm: RSSI_BOTTOM_DEFAULT_DBM,
        }
    }
}

/// The graph's default span: the ceiling is a node in the same room, the
/// floor is under any sensitivity worth plotting.
pub const RSSI_TOP_DEFAULT_DBM: i16 = -20;
pub const RSSI_BOTTOM_DEFAULT_DBM: i16 = -120;

/// The dBm values the graph's span is accepted in, and the least it may
/// cover: a span narrower than this turns a few dB of noise into a
/// full-height swing, which is what the fixed scale exists to avoid.
pub const RSSI_DBM_MIN: i16 = -200;
pub const RSSI_DBM_MAX: i16 = 0;
pub const RSSI_SPAN_MIN: i16 = 10;

/// CSV logging settings.
///
/// The log is a file rather than a view, so its two settings are a path and
/// whether to start writing to it unasked. The reference coordinate is here
/// too because it is what a logged distance is measured against, and a run
/// measured against a moving control device answers a different question from
/// one measured against a surveyed point.
#[derive(Clone, Default)]
pub struct LogSettings {
    /// Start recording as soon as the app launches, so a run can be logged
    /// without remembering to arm it first.
    pub auto_start: bool,
    /// Where the CSV is written. `None` generates a timestamped name beside
    /// the config file, which is the one directory known to be writable on
    /// both platforms.
    pub file: Option<String>,
    /// A fixed coordinate every logged position is also measured against.
    /// Both halves or neither; a lone latitude is not a point.
    pub ref_lat: Option<f64>,
    pub ref_lon: Option<f64>,
}

impl LogSettings {
    /// The reference coordinate, when both halves are set.
    pub fn reference(&self) -> Option<(f64, f64)> {
        Some((self.ref_lat?, self.ref_lon?))
    }

    /// Set the reference to a coordinate, or clear it with `None` - both
    /// halves move together so a half-set reference is never reachable.
    pub fn set_reference(&mut self, point: Option<(f64, f64)>) {
        let (lat, lon) = match point {
            Some(p) => (Some(p.0), Some(p.1)),
            None => (None, None),
        };
        self.ref_lat = lat;
        self.ref_lon = lon;
    }
}

/// Everything a config file can carry.
#[derive(Clone, Default)]
pub struct AppConfig {
    pub colors: MarkerColors,
    pub phone: PhoneSettings,
    pub ui: UiSettings,
    pub sizes: MarkerSizes,
    pub distance: DistanceSettings,
    pub map: MapSettings,
    pub ble: BleSettings,
    pub lora: LoraSettings,
    pub track: TrackSettings,
    pub compass: CompassSettings,
    pub status_bar: StatusBarSettings,
    pub log: LogSettings,
}

/// Mirrors the TOML shape; every field optional so a partial file keeps the
/// defaults for whatever it leaves out.
#[derive(Deserialize, Default)]
struct RawConfig {
    #[serde(default)]
    colors: RawColors,
    #[serde(default)]
    phone: RawPhone,
    #[serde(default)]
    ui: RawUi,
    #[serde(default)]
    sizes: RawSizes,
    #[serde(default)]
    distance: RawDistance,
    #[serde(default)]
    map: RawMap,
    #[serde(default)]
    ble: RawBle,
    #[serde(default)]
    lora: RawLora,
    #[serde(default)]
    track: RawTrack,
    #[serde(default)]
    compass: RawCompass,
    #[serde(default)]
    status_bar: RawStatusBar,
    #[serde(default)]
    log: RawLog,
}

#[derive(Deserialize, Default)]
struct RawColors {
    track: Option<String>,
    fixed: Option<String>,
    outline: Option<String>,
}

#[derive(Deserialize, Default)]
struct RawPhone {
    name: Option<String>,
    location: Option<bool>,
}

#[derive(Deserialize, Default)]
struct RawMap {
    tiles: Option<String>,
    arcgis_key: Option<String>,
    bar_opacity: Option<f32>,
    show_key: Option<bool>,
}

#[derive(Deserialize, Default)]
struct RawUi {
    theme: Option<String>,
    ok: Option<String>,
    error: Option<String>,
    busy: Option<String>,
    pulse: Option<String>,
    background: Option<String>,
    button: Option<String>,
    text: Option<String>,
    text_scale: Option<f32>,
}

#[derive(Deserialize, Default)]
struct RawSizes {
    marker: Option<f32>,
    beacon: Option<f32>,
    track: Option<f32>,
    distance_line: Option<f32>,
    distance_text: Option<f32>,
}

#[derive(Deserialize, Default)]
struct RawDistance {
    show: Option<bool>,
    units: Option<String>,
    dotted: Option<bool>,
}

#[derive(Deserialize, Default)]
struct RawBle {
    enabled: Option<bool>,
    show_on_map: Option<bool>,
    show_path: Option<bool>,
    location: Option<bool>,
    mac: Option<String>,
    #[serde(default)]
    names: BTreeMap<String, String>,
}

#[derive(Deserialize, Default)]
struct RawLora {
    show_path: Option<bool>,
    pulse_secs: Option<f32>,
    // TOML table keys are strings; parsed to a u8 address on the way in.
    #[serde(default)]
    names: BTreeMap<String, String>,
}

#[derive(Deserialize, Default)]
struct RawTrack {
    min_distance: Option<f64>,
    show_path: Option<bool>,
}

#[derive(Deserialize, Default)]
struct RawCompass {
    marker_arrow: Option<bool>,
    arrow_hz: Option<f32>,
}

#[derive(Deserialize, Default)]
struct RawStatusBar {
    show: Option<bool>,
    cycle_secs: Option<f32>,
    rssi_top_dbm: Option<i16>,
    rssi_bottom_dbm: Option<i16>,
}

#[derive(Deserialize, Default)]
struct RawLog {
    auto_start: Option<bool>,
    file: Option<String>,
    ref_lat: Option<f64>,
    ref_lon: Option<f64>,
}

/// Parse a `#rrggbb` (or bare `rrggbb`) hex string into a color.
fn parse_hex(s: &str) -> Result<Color32, String> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("invalid hex color {s:?}, expected #rrggbb"));
    }
    let n = u32::from_str_radix(h, 16).map_err(|_| format!("invalid hex color {s:?}"))?;
    Ok(Color32::from_rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

/// Parse a color that may be left to the theme: an empty string is "unset", so
/// the key can stay in the file with nothing in it rather than being deleted to
/// turn the override off.
fn parse_hex_opt(s: &str) -> Result<Option<Color32>, String> {
    if s.trim().is_empty() {
        return Ok(None);
    }
    parse_hex(s).map(Some)
}

/// `#rrggbb` for a color: the form [`parse_hex`] reads back.
fn hex(c: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}

/// `#rrggbb` for a color left to the theme when unset - an empty string, which
/// [`parse_hex_opt`] reads back as "unset".
fn hex_opt(c: Option<Color32>) -> String {
    c.map(hex).unwrap_or_default()
}

/// A TOML float for an `f32` setting. Widening straight to `f64` exposes the
/// binary representation (14.4f32 becomes 14.399999618530273 in the file), so go
/// through the shortest decimal that reads back as the same `f32`.
fn f32_value(v: f32) -> Value {
    format!("{v:?}").parse::<f64>().unwrap_or(v as f64).into()
}

/// Set `[section] key = value`, adding the table or the key when either is
/// missing. Replacing an existing value keeps its decor, so only the value
/// itself changes on disk - the surrounding spacing and any trailing comment
/// stay put.
fn set(doc: &mut DocumentMut, section: &str, key: &str, value: Value) {
    let table = doc
        .entry(section)
        .or_insert_with(|| Item::Table(Table::new()));
    let Some(table) = table.as_table_mut() else {
        return;
    };
    let mut value = value;
    if let Some(old) = table.get(key).and_then(Item::as_value) {
        *value.decor_mut() = old.decor().clone();
    } else {
        *value.decor_mut() = toml_edit::Decor::new(" ", "");
    }
    table.insert(key, Item::Value(value));
}

/// Set `[section] key = value` for a setting that may be unset, removing the
/// key when it is.
///
/// Unlike the colors and the MAC, an unset number has no empty form to leave
/// in the file: `ref_lat = ""` is not a float and would fail to load. So the
/// key goes away entirely, and its absence is what "unset" reads as.
fn set_opt(doc: &mut DocumentMut, section: &str, key: &str, value: Option<Value>) {
    match value {
        Some(value) => set(doc, section, key, value),
        None => {
            if let Some(table) = doc.get_mut(section).and_then(Item::as_table_mut) {
                table.remove(key);
            }
        }
    }
}

/// Rewrite `[ble.names]` to match `names`, adding the sub-table when it is
/// missing. Entries that are still present keep their decor (so a comment on a
/// nickname line survives), entries no longer in the map are dropped - that is
/// what makes "forget this board" stick across a save.
///
/// MAC keys need quoting, which `toml_edit` applies by itself: a bare key
/// cannot contain colons, so it falls back to a quoted one.
fn set_names(doc: &mut DocumentMut, names: &BTreeMap<String, String>) {
    let table = doc
        .entry("ble")
        .or_insert_with(|| Item::Table(Table::new()));
    let Some(table) = table.as_table_mut() else {
        return;
    };
    let names_table = table
        .entry("names")
        .or_insert_with(|| Item::Table(Table::new()));
    let Some(names_table) = names_table.as_table_mut() else {
        return;
    };
    // Drop keys we no longer know, and any spelled differently from the
    // canonical form - the loop below re-adds those under their normalized
    // key, and keeping the old spelling would file the same board twice.
    names_table.retain(|key, _| names.contains_key(key) && normalize_mac(key) == key);
    for (mac, name) in names {
        if let Some(old) = names_table.get(mac).and_then(Item::as_value) {
            let mut value: Value = name.as_str().into();
            *value.decor_mut() = old.decor().clone();
            names_table.insert(mac, Item::Value(value));
        } else {
            names_table.insert(mac, Item::Value(name.as_str().into()));
        }
    }
}

/// Rewrite `[lora.names]` to match `names`, mirroring [`set_names`]: entries
/// still present keep their decor, ones no longer known are dropped. Keys are
/// the numeric address written as a string; a bare TOML key of ASCII digits is
/// legal, so `toml_edit` writes `3 = "Truck"` without quoting.
fn set_lora_names(doc: &mut DocumentMut, names: &BTreeMap<u8, String>) {
    let table = doc
        .entry("lora")
        .or_insert_with(|| Item::Table(Table::new()));
    let Some(table) = table.as_table_mut() else {
        return;
    };
    let names_table = table
        .entry("names")
        .or_insert_with(|| Item::Table(Table::new()));
    let Some(names_table) = names_table.as_table_mut() else {
        return;
    };
    // Drop keys no longer known, and any that do not parse to an address we
    // hold (a stale or malformed key would otherwise linger).
    names_table.retain(|key, _| {
        key.trim()
            .parse::<u8>()
            .is_ok_and(|addr| names.contains_key(&addr))
    });
    for (addr, name) in names {
        let key = addr.to_string();
        if let Some(old) = names_table.get(&key).and_then(Item::as_value) {
            let mut value: Value = name.as_str().into();
            *value.decor_mut() = old.decor().clone();
            names_table.insert(&key, Item::Value(value));
        } else {
            names_table.insert(&key, Item::Value(name.as_str().into()));
        }
    }
}

/// Range the marker-arrow compass rate is accepted (and edited) in. The floor
/// keeps the arrow from lagging behind a turn; the ceiling is about where the
/// sensor stops being the cheap background reader this rate is meant to be.
pub const COMPASS_HZ_MIN: f32 = 0.5;
pub const COMPASS_HZ_MAX: f32 = 30.0;

/// Range the status bar's per-node dwell is accepted (and edited) in. The
/// floor is about the shortest a reading can be taken in at a glance; the
/// ceiling is where a node further down the cycle waits long enough that the
/// bar reads as stuck on one of them.
pub const STATUS_CYCLE_MIN: f32 = 1.0;
pub const STATUS_CYCLE_MAX: f32 = 60.0;

/// Range the page text scale is accepted (and edited) in. Below 1.0 is there to
/// walk an overshoot back rather than to shrink the text far; the ceiling is
/// about where a phone page holds a few words to the line and the buttons stop
/// fitting their labels.
pub const TEXT_SCALE_MIN: f32 = 0.8;
pub const TEXT_SCALE_MAX: f32 = 2.5;

/// Validate an overlay size: finite and strictly positive.
fn parse_size(name: &str, v: f32) -> Result<f32, String> {
    if !v.is_finite() || v <= 0.0 {
        return Err(format!("sizes.{name} must be > 0, got {v}"));
    }
    Ok(v)
}

impl AppConfig {
    /// Read the config from a TOML file at `path`. Missing fields fall back
    /// to the defaults; a returned `Err` is a human-readable message for the
    /// UI.
    pub fn load(path: &str) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        Self::from_toml(&text)
    }

    /// Write these settings to the TOML file at `path`, returning `true` when
    /// the file had to be created.
    ///
    /// An existing file is edited in place rather than rewritten: comments, key
    /// order, and any keys this app knows nothing about all survive, and only
    /// the values it owns are replaced. With no file there (or an unreadable
    /// one) it generates a fresh documented one from [`Self::to_toml`].
    pub fn save(&self, path: &str) -> Result<bool, String> {
        let Ok(text) = std::fs::read_to_string(path) else {
            std::fs::write(path, self.to_toml()).map_err(|e| format!("{path}: {e}"))?;
            return Ok(true);
        };
        let mut doc: DocumentMut = text.parse().map_err(|e| format!("{path}: {e}"))?;
        set(&mut doc, "colors", "track", hex(self.colors.track).into());
        set(&mut doc, "colors", "fixed", hex(self.colors.fixed).into());
        set(
            &mut doc,
            "colors",
            "outline",
            hex(self.colors.outline).into(),
        );
        set(&mut doc, "phone", "name", self.phone.name.as_str().into());
        set(&mut doc, "phone", "location", self.phone.location.into());
        set(&mut doc, "ui", "theme", self.ui.theme.as_str().into());
        set(&mut doc, "ui", "ok", hex(self.ui.ok).into());
        set(&mut doc, "ui", "error", hex(self.ui.error).into());
        set(&mut doc, "ui", "busy", hex(self.ui.busy).into());
        set(&mut doc, "ui", "pulse", hex(self.ui.pulse).into());
        // Left in the file as an empty string rather than dropped, so the key is
        // there to fill in by hand.
        set(
            &mut doc,
            "ui",
            "background",
            hex_opt(self.ui.background).into(),
        );
        set(&mut doc, "ui", "button", hex_opt(self.ui.button).into());
        set(&mut doc, "ui", "text", hex_opt(self.ui.text).into());
        set(&mut doc, "ui", "text_scale", f32_value(self.ui.text_scale));
        set(&mut doc, "sizes", "marker", f32_value(self.sizes.marker));
        set(&mut doc, "sizes", "beacon", f32_value(self.sizes.beacon));
        set(&mut doc, "sizes", "track", f32_value(self.sizes.track));
        set(
            &mut doc,
            "sizes",
            "distance_line",
            f32_value(self.sizes.distance_line),
        );
        set(
            &mut doc,
            "sizes",
            "distance_text",
            f32_value(self.sizes.distance_text),
        );
        set(&mut doc, "distance", "show", self.distance.show.into());
        set(
            &mut doc,
            "distance",
            "units",
            self.distance.units.as_str().into(),
        );
        set(&mut doc, "distance", "dotted", self.distance.dotted.into());
        set(&mut doc, "map", "tiles", self.map.tiles.as_str().into());
        // Kept as an empty string, like the theme overrides, so the key is
        // there to paste into.
        set(
            &mut doc,
            "map",
            "arcgis_key",
            self.map.arcgis_key.trim().into(),
        );
        set(
            &mut doc,
            "map",
            "bar_opacity",
            f32_value(self.map.bar_opacity),
        );
        set(&mut doc, "map", "show_key", self.map.show_key.into());
        set(&mut doc, "ble", "enabled", self.ble.enabled.into());
        set(&mut doc, "ble", "show_on_map", self.ble.show_on_map.into());
        set(&mut doc, "ble", "show_path", self.ble.show_path.into());
        set(&mut doc, "ble", "location", self.ble.location.into());
        // An empty string reads back as "unset", so an unpinned MAC keeps the
        // key in the file rather than dropping the line.
        set(
            &mut doc,
            "ble",
            "mac",
            self.ble.mac.clone().unwrap_or_default().into(),
        );
        set(&mut doc, "lora", "show_path", self.lora.show_path.into());
        set(
            &mut doc,
            "lora",
            "pulse_secs",
            f32_value(self.lora.pulse_secs),
        );
        set(
            &mut doc,
            "track",
            "min_distance",
            self.track.min_distance.into(),
        );
        set(&mut doc, "track", "show_path", self.track.show_path.into());
        set(
            &mut doc,
            "compass",
            "marker_arrow",
            self.compass.marker_arrow.into(),
        );
        set(
            &mut doc,
            "compass",
            "arrow_hz",
            f32_value(self.compass.arrow_hz),
        );
        set(&mut doc, "status_bar", "show", self.status_bar.show.into());
        set(
            &mut doc,
            "status_bar",
            "cycle_secs",
            f32_value(self.status_bar.cycle_secs),
        );
        set(
            &mut doc,
            "status_bar",
            "rssi_top_dbm",
            i64::from(self.status_bar.rssi_top_dbm).into(),
        );
        set(
            &mut doc,
            "status_bar",
            "rssi_bottom_dbm",
            i64::from(self.status_bar.rssi_bottom_dbm).into(),
        );
        set(&mut doc, "log", "auto_start", self.log.auto_start.into());
        set(
            &mut doc,
            "log",
            "file",
            self.log.file.clone().unwrap_or_default().into(),
        );
        set_opt(&mut doc, "log", "ref_lat", self.log.ref_lat.map(Into::into));
        set_opt(&mut doc, "log", "ref_lon", self.log.ref_lon.map(Into::into));
        set_names(&mut doc, &self.ble.names);
        set_lora_names(&mut doc, &self.lora.names);
        std::fs::write(path, doc.to_string()).map_err(|e| format!("{path}: {e}"))?;
        Ok(false)
    }

    /// These settings as a complete, commented TOML file - what the Settings
    /// page generates when there is no config file yet.
    pub fn to_toml(&self) -> String {
        let s = &self.sizes;
        // With no boards named yet, show the shape as a comment: the section is
        // hand-editable, and an empty header explains nothing about what goes
        // in it.
        let names = if self.ble.names.is_empty() {
            "# \"AA:BB:CC:DD:EE:FF\" = \"Truck\"\n".to_string()
        } else {
            self.ble
                .names
                .iter()
                .map(|(mac, name)| format!("{mac:?} = {name:?}\n"))
                .collect()
        };
        // Same treatment for the LoRa node names, keyed by numeric address.
        let lora_names = if self.lora.names.is_empty() {
            "# 3 = \"Truck\"\n".to_string()
        } else {
            self.lora
                .names
                .iter()
                .map(|(addr, name)| format!("{addr} = {name:?}\n"))
                .collect()
        };
        // An unset reference has no value to write, so the pair is shown as a
        // comment: the keys are what a hand-edit needs to see, and a zeroed
        // pair would be a real coordinate off the coast of Africa.
        let reference = match self.log.reference() {
            Some((lat, lon)) => format!(
                "ref_lat = {lat}\n\
                 ref_lon = {lon}          # fixed point every logged position is measured against\n"
            ),
            None => {
                "# ref_lat = 51.4779   # a fixed point every logged position is measured against\n\
                     # ref_lon = -0.0015\n"
                    .to_string()
            }
        };
        format!(
            "# gps-gui-rs settings. Every key is optional; a missing one keeps its default.\n\
             \n\
             [colors]\n\
             track = \"{track}\"   # your track, heading arrow, position dot\n\
             fixed = \"{fixed}\"   # the connected node's marker, distance line and path\n\
             outline = \"{outline}\" # ring around the position and node dots\n\
             \n\
             [phone]              # this device\n\
             name = \"{phone_name}\"       # what the map and the pages call it\n\
             location = {phone_location}     # use its own GNSS for the current position\n\
             \n\
             [ui]                 # the pages, not the map: their colors and text\n\
             theme = \"{theme}\"      # \"light\", \"dark\" (both solarized) or \"system\"\n\
             ok = \"{ok}\"      # \"yes\" and the green feedback lines\n\
             error = \"{error}\"   # \"no\", errors, and the red feedback lines\n\
             busy = \"{busy}\"    # scanning and connecting\n\
             pulse = \"{pulse}\"   # a toolbar button flagging that it has no target\n\
             # Empty follows the light/dark theme, which is what keeps these three\n\
             # readable against each other. Setting one is taking that on yourself.\n\
             background = \"{background}\"      # behind the pages, the map bar and the popups\n\
             button = \"{button}\"          # a button at rest; hover and press are shaded from it\n\
             text = \"{text}\"            # body text; weak and strong are shaded from it\n\
             text_scale = {text_scale:?}      # page text size ({scale_min} - {scale_max}); the page spacing follows it\n\
             \n\
             [sizes]              # screen points; each overlay is sized independently\n\
             marker = {marker:?}          # current-position dot radius\n\
             beacon = {beacon:?}          # node dot radius\n\
             track = {track_w:?}           # track polyline width (your track and the node paths)\n\
             distance_line = {dline:?}    # user<->node line width\n\
             distance_text = {dtext:?}   # distance label font size\n\
             \n\
             [distance]\n\
             show = {show}        # draw the distance label on the line to the node\n\
             units = \"{units}\"    # \"metric\" (km/m) or \"imperial\" (mi/ft)\n\
             dotted = {dotted}       # draw distance line dotted rather than solid\n\
             \n\
             [map]                # the map page: its tiles and its overlays\n\
             tiles = \"{tiles}\"        # \"osm\" (OpenStreetMap, OpenTopoMap) or \"arcgis\" (Esri streets, outdoor, satellite)\n\
             arcgis_key = \"{arcgis_key}\"      # your own ArcGIS key; empty uses the built-in one\n\
             bar_opacity = {bar_opacity:?}    # the bottom bar and the status bar backgrounds, 0 - 1\n\
             show_key = {show_key}      # the color key in the top left corner\n\
             \n\
             [ble]\n\
             enabled = {enabled}       # master switch for the BLE GPS source\n\
             show_on_map = {show_on_map}   # draw the connected node on the map at all\n\
             show_path = {show_path}    # draw the path of the incoming BLE GPS data\n\
             location = {ble_location}      # use the connected node's GPS as the current position\n\
             mac = \"{mac}\"            # pin a specific node; empty scans by service\n\
             \n\
             [ble.names]          # names for known nodes, keyed by MAC; a node reporting a name writes it here\n\
             {names}\
             \n\
             [lora]               # remote nodes heard over LoRa and relayed by the connected node\n\
             show_path = {lora_show_path}    # draw the remote nodes' paths on the map\n\
             pulse_secs = {pulse_secs:?}    # a node's marker pulses while heard within this; 0 never\n\
             \n\
             [lora.names]         # nicknames for remote nodes, keyed by LoRa address (1-255)\n\
             {lora_names}\
             \n\
             [track]\n\
             min_distance = {min_distance:?}   # meters of movement before a new track point\n\
             show_path = {show_track}    # draw your own recorded path\n\
             \n\
             [compass]            # heading-up always runs the sensor at full rate\n\
             marker_arrow = {marker_arrow}  # point the marker arrow with the compass in north-up and tracking\n\
             arrow_hz = {arrow_hz:?}       # sensor rate while only that arrow needs it\n\
             \n\
             [status_bar]         # the read-out along the bottom of the map\n\
             show = {status_show}         # draw it at all\n\
             cycle_secs = {cycle_secs:?}     # seconds each node holds the read-out ({cycle_min} - {cycle_max})\n\
             rssi_top_dbm = {rssi_top}    # a reception this strong fills the graph\n\
             rssi_bottom_dbm = {rssi_bottom} # and this weak is empty; heights are linear in dBm\n\
             \n\
             [log]                # the CSV log on the Logging page\n\
             auto_start = {auto_start}  # start recording as soon as the app launches\n\
             file = \"{log_file}\"           # log path; empty is a timestamped file beside this one\n\
             {reference}",
            track = hex(self.colors.track),
            fixed = hex(self.colors.fixed),
            outline = hex(self.colors.outline),
            phone_name = self.phone.name,
            phone_location = self.phone.location,
            theme = self.ui.theme.as_str(),
            ok = hex(self.ui.ok),
            error = hex(self.ui.error),
            busy = hex(self.ui.busy),
            pulse = hex(self.ui.pulse),
            background = hex_opt(self.ui.background),
            button = hex_opt(self.ui.button),
            text = hex_opt(self.ui.text),
            text_scale = self.ui.text_scale,
            scale_min = TEXT_SCALE_MIN,
            scale_max = TEXT_SCALE_MAX,
            marker = s.marker,
            beacon = s.beacon,
            track_w = s.track,
            dline = s.distance_line,
            dtext = s.distance_text,
            show = self.distance.show,
            units = self.distance.units.as_str(),
            dotted = self.distance.dotted,
            tiles = self.map.tiles.as_str(),
            arcgis_key = self.map.arcgis_key.trim(),
            bar_opacity = self.map.bar_opacity,
            show_key = self.map.show_key,
            enabled = self.ble.enabled,
            show_on_map = self.ble.show_on_map,
            show_path = self.ble.show_path,
            ble_location = self.ble.location,
            mac = self.ble.mac.clone().unwrap_or_default(),
            names = names,
            lora_show_path = self.lora.show_path,
            pulse_secs = self.lora.pulse_secs,
            lora_names = lora_names,
            min_distance = self.track.min_distance,
            show_track = self.track.show_path,
            marker_arrow = self.compass.marker_arrow,
            arrow_hz = self.compass.arrow_hz,
            status_show = self.status_bar.show,
            cycle_secs = self.status_bar.cycle_secs,
            cycle_min = STATUS_CYCLE_MIN,
            cycle_max = STATUS_CYCLE_MAX,
            rssi_top = self.status_bar.rssi_top_dbm,
            rssi_bottom = self.status_bar.rssi_bottom_dbm,
            auto_start = self.log.auto_start,
            log_file = self.log.file.clone().unwrap_or_default(),
            reference = reference,
        )
    }

    fn from_toml(text: &str) -> Result<Self, String> {
        let raw: RawConfig = toml::from_str(text).map_err(|e| e.to_string())?;
        let mut config = Self::default();
        if let Some(s) = raw.colors.track {
            config.colors.track = parse_hex(&s)?;
        }
        if let Some(s) = raw.colors.fixed {
            config.colors.fixed = parse_hex(&s)?;
        }
        if let Some(s) = raw.colors.outline {
            config.colors.outline = parse_hex(&s)?;
        }
        // A blank name is no name: the default stands rather than a marker
        // labelled with nothing.
        if let Some(name) = raw.phone.name {
            let name = name.trim();
            if !name.is_empty() {
                config.phone.name = name.to_string();
            }
        }
        if let Some(v) = raw.phone.location {
            config.phone.location = v;
        }
        if let Some(s) = raw.ui.theme {
            config.ui.theme = ThemeChoice::parse(&s)?;
        }
        if let Some(s) = raw.ui.ok {
            config.ui.ok = parse_hex(&s)?;
        }
        if let Some(s) = raw.ui.error {
            config.ui.error = parse_hex(&s)?;
        }
        if let Some(s) = raw.ui.busy {
            config.ui.busy = parse_hex(&s)?;
        }
        if let Some(s) = raw.ui.pulse {
            config.ui.pulse = parse_hex(&s)?;
        }
        if let Some(s) = raw.ui.background {
            config.ui.background = parse_hex_opt(&s)?;
        }
        if let Some(s) = raw.ui.button {
            config.ui.button = parse_hex_opt(&s)?;
        }
        if let Some(s) = raw.ui.text {
            config.ui.text = parse_hex_opt(&s)?;
        }
        if let Some(v) = raw.ui.text_scale {
            if !v.is_finite() || !(TEXT_SCALE_MIN..=TEXT_SCALE_MAX).contains(&v) {
                return Err(format!(
                    "ui.text_scale must be between {TEXT_SCALE_MIN} and {TEXT_SCALE_MAX}, got {v}"
                ));
            }
            config.ui.text_scale = v;
        }
        if let Some(v) = raw.sizes.marker {
            config.sizes.marker = parse_size("marker", v)?;
        }
        if let Some(v) = raw.sizes.beacon {
            config.sizes.beacon = parse_size("beacon", v)?;
        }
        if let Some(v) = raw.sizes.track {
            config.sizes.track = parse_size("track", v)?;
        }
        if let Some(v) = raw.sizes.distance_line {
            config.sizes.distance_line = parse_size("distance_line", v)?;
        }
        if let Some(v) = raw.sizes.distance_text {
            config.sizes.distance_text = parse_size("distance_text", v)?;
        }
        if let Some(v) = raw.distance.show {
            config.distance.show = v;
        }
        if let Some(s) = raw.distance.units {
            config.distance.units = DistanceUnits::parse(&s)?;
        }
        if let Some(v) = raw.distance.dotted {
            config.distance.dotted = v;
        }
        if let Some(s) = raw.map.tiles {
            config.map.tiles = TileProvider::parse(&s)?;
        }
        if let Some(s) = raw.map.arcgis_key {
            config.map.arcgis_key = s.trim().to_string();
        }
        if let Some(v) = raw.map.bar_opacity {
            if !v.is_finite() || !(0.0..=1.0).contains(&v) {
                return Err(format!("map.bar_opacity must be between 0 and 1, got {v}"));
            }
            config.map.bar_opacity = v;
        }
        if let Some(v) = raw.map.show_key {
            config.map.show_key = v;
        }
        if let Some(v) = raw.ble.enabled {
            config.ble.enabled = v;
        }
        if let Some(v) = raw.ble.location {
            config.ble.location = v;
        }
        if let Some(v) = raw.ble.show_on_map {
            config.ble.show_on_map = v;
        }
        if let Some(v) = raw.ble.show_path {
            config.ble.show_path = v;
        }
        // Treat an empty string as unset so a template line can stay in the
        // file.
        config.ble.mac = raw.ble.mac.filter(|m| !m.trim().is_empty());
        // Normalize on the way in: a hand-edited file may spell a MAC any way,
        // and the picker looks these up by normalized key.
        config.ble.names = raw
            .ble
            .names
            .into_iter()
            .filter(|(_, name)| !name.trim().is_empty())
            .map(|(mac, name)| (normalize_mac(&mac), name.trim().to_string()))
            .collect();
        if let Some(v) = raw.lora.show_path {
            config.lora.show_path = v;
        }
        if let Some(v) = raw.lora.pulse_secs {
            if !v.is_finite() || v < 0.0 {
                return Err(format!("lora.pulse_secs must be >= 0, got {v}"));
            }
            config.lora.pulse_secs = v;
        }
        // LoRa node keys are addresses (1-255); 0 is the local GPS, not a
        // remote. A key that is not one is a typo worth surfacing rather than
        // silently dropping, since the file is hand-editable.
        for (key, name) in raw.lora.names {
            if name.trim().is_empty() {
                continue;
            }
            let addr = key.trim().parse::<u8>().ok().filter(|&a| a != 0);
            let Some(addr) = addr else {
                return Err(format!(
                    "invalid lora.names address {key:?}, expected a number 1-255"
                ));
            };
            config.lora.names.insert(addr, name.trim().to_string());
        }
        if let Some(v) = raw.track.min_distance {
            if !v.is_finite() || v < 0.0 {
                return Err(format!("track.min_distance must be >= 0, got {v}"));
            }
            config.track.min_distance = v;
        }
        if let Some(v) = raw.track.show_path {
            config.track.show_path = v;
        }
        if let Some(v) = raw.compass.marker_arrow {
            config.compass.marker_arrow = v;
        }
        if let Some(v) = raw.compass.arrow_hz {
            if !v.is_finite() || !(COMPASS_HZ_MIN..=COMPASS_HZ_MAX).contains(&v) {
                return Err(format!(
                    "compass.arrow_hz must be between {COMPASS_HZ_MIN} and {COMPASS_HZ_MAX}, got {v}"
                ));
            }
            config.compass.arrow_hz = v;
        }
        if let Some(v) = raw.status_bar.show {
            config.status_bar.show = v;
        }
        if let Some(v) = raw.status_bar.cycle_secs {
            if !v.is_finite() || !(STATUS_CYCLE_MIN..=STATUS_CYCLE_MAX).contains(&v) {
                return Err(format!(
                    "status_bar.cycle_secs must be between {STATUS_CYCLE_MIN} and \
                     {STATUS_CYCLE_MAX}, got {v}"
                ));
            }
            config.status_bar.cycle_secs = v;
        }
        if let Some(v) = raw.status_bar.rssi_top_dbm {
            config.status_bar.rssi_top_dbm = v;
        }
        if let Some(v) = raw.status_bar.rssi_bottom_dbm {
            config.status_bar.rssi_bottom_dbm = v;
        }
        // Checked as a pair whichever half the file set: the graph needs a
        // span, and a top under the bottom is a graph drawn upside down.
        let (top, bottom) = (config.status_bar.rssi_top_dbm, config.status_bar.rssi_bottom_dbm);
        if !(RSSI_DBM_MIN..=RSSI_DBM_MAX).contains(&top)
            || !(RSSI_DBM_MIN..=RSSI_DBM_MAX).contains(&bottom)
            || top - bottom < RSSI_SPAN_MIN
        {
            return Err(format!(
                "status_bar.rssi_top_dbm ({top}) and rssi_bottom_dbm ({bottom}) must be dBm \
                 values between {RSSI_DBM_MIN} and {RSSI_DBM_MAX}, at least {RSSI_SPAN_MIN} apart"
            ));
        }
        if let Some(v) = raw.log.auto_start {
            config.log.auto_start = v;
        }
        // Empty is unset, as for the pinned MAC: the key stays in the file as
        // a template rather than having to be deleted to fall back.
        config.log.file = raw.log.file.filter(|p| !p.trim().is_empty());
        // Both halves or neither. A file with only one of them is a typo, and
        // silently ignoring it would leave a reference-distance column that is
        // always empty with nothing saying why.
        match (raw.log.ref_lat, raw.log.ref_lon) {
            (Some(lat), Some(lon)) => {
                if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
                    return Err(format!(
                        "log reference {lat}, {lon} is not a coordinate \
                         (lat -90..90, lon -180..180)"
                    ));
                }
                config.log.set_reference(Some((lat, lon)));
            }
            (None, None) => {}
            _ => {
                return Err("log.ref_lat and log.ref_lon must be set together".to_string());
            }
        }
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_switches_km_at_a_kilometer() {
        assert_eq!(DistanceUnits::Metric.format(0.0), "0 m");
        assert_eq!(DistanceUnits::Metric.format(940.0), "940 m");
        assert_eq!(DistanceUnits::Metric.format(1000.0), "1.00 km");
        assert_eq!(DistanceUnits::Metric.format(2500.0), "2.50 km");
    }

    #[test]
    fn imperial_switches_miles_at_a_mile() {
        assert_eq!(DistanceUnits::Imperial.format(0.0), "0 ft");
        // Just under a mile stays in feet.
        assert_eq!(DistanceUnits::Imperial.format(1609.0), "5279 ft");
        assert_eq!(DistanceUnits::Imperial.format(1609.344), "1.00 mi");
        assert_eq!(DistanceUnits::Imperial.format(3218.688), "2.00 mi");
    }

    #[test]
    fn units_parse_is_case_insensitive_and_aliased() {
        assert_eq!(DistanceUnits::parse("Metric").unwrap(), DistanceUnits::Metric);
        assert_eq!(DistanceUnits::parse("km/m").unwrap(), DistanceUnits::Metric);
        assert_eq!(DistanceUnits::parse("IMPERIAL").unwrap(), DistanceUnits::Imperial);
        assert_eq!(DistanceUnits::parse("mi/ft").unwrap(), DistanceUnits::Imperial);
        assert!(DistanceUnits::parse("furlongs").is_err());
    }

    #[test]
    fn generated_file_reads_back_as_the_same_settings() {
        let cfg = AppConfig::default();
        let back = AppConfig::from_toml(&cfg.to_toml()).unwrap();
        assert_eq!(back.colors.fixed, cfg.colors.fixed);
        assert_eq!(back.colors.outline, cfg.colors.outline);
        assert_eq!(back.ui.theme, cfg.ui.theme);
        assert_eq!(back.ui.ok, cfg.ui.ok);
        assert_eq!(back.ui.error, cfg.ui.error);
        assert_eq!(back.ui.pulse, cfg.ui.pulse);
        assert_eq!(back.ui.text_scale, cfg.ui.text_scale);
        assert_eq!(back.sizes.distance_text, cfg.sizes.distance_text);
        assert_eq!(back.distance.units, cfg.distance.units);
        assert_eq!(back.track.min_distance, cfg.track.min_distance);
        assert_eq!(back.track.show_path, cfg.track.show_path);
        assert_eq!(back.compass.marker_arrow, cfg.compass.marker_arrow);
        assert_eq!(back.compass.arrow_hz, cfg.compass.arrow_hz);
        assert_eq!(back.status_bar.show, cfg.status_bar.show);
        assert_eq!(back.status_bar.cycle_secs, cfg.status_bar.cycle_secs);
        // The generated `mac = ""` means "scan by service", not a pinned MAC.
        assert_eq!(back.ble.mac, None);
        assert_eq!(back.ble.show_on_map, cfg.ble.show_on_map);
        assert_eq!(back.ble.location, cfg.ble.location);
        assert_eq!(back.phone.name, cfg.phone.name);
        assert_eq!(back.phone.location, cfg.phone.location);
        assert_eq!(back.map.bar_opacity, cfg.map.bar_opacity);
        assert_eq!(back.map.show_key, cfg.map.show_key);
        assert_eq!(back.map.tiles, cfg.map.tiles);
        assert_eq!(back.map.arcgis_key, cfg.map.arcgis_key);
        assert_eq!(back.lora.pulse_secs, cfg.lora.pulse_secs);
        assert_eq!(back.status_bar.rssi_top_dbm, cfg.status_bar.rssi_top_dbm);
        assert_eq!(back.status_bar.rssi_bottom_dbm, cfg.status_bar.rssi_bottom_dbm);
        assert_eq!(back.ui.busy, cfg.ui.busy);
    }

    /// The phone's own receiver is off by default and the node's is on:
    /// the usual setup is a node held next to the phone, and its receiver
    /// is the one worth the battery.
    #[test]
    fn the_location_source_defaults_to_the_node() {
        let cfg = AppConfig::default();
        assert!(!cfg.phone.location);
        assert!(cfg.ble.location);
        assert_eq!(cfg.phone.name, "Phone");

        let cfg = AppConfig::from_toml(
            "[phone]\nname = \"  Sam  \"\nlocation = true\n\n[ble]\nlocation = false\n",
        )
        .unwrap();
        assert_eq!(cfg.phone.name, "Sam");
        assert!(cfg.phone.location);
        assert!(!cfg.ble.location);
        // A blank name is no name.
        assert_eq!(
            AppConfig::from_toml("[phone]\nname = \"  \"").unwrap().phone.name,
            "Phone"
        );
    }

    /// The graph's span is a pair: both ends are dBm values and the top is
    /// above the bottom by enough to be a graph.
    #[test]
    fn the_status_bar_span_is_checked_as_a_pair() {
        let cfg = AppConfig::from_toml("[status_bar]\nrssi_top_dbm = -30\nrssi_bottom_dbm = -110")
            .unwrap();
        assert_eq!(cfg.status_bar.rssi_top_dbm, -30);
        assert_eq!(cfg.status_bar.rssi_bottom_dbm, -110);
        // One end moved past the other, or a span too narrow to read.
        assert!(AppConfig::from_toml("[status_bar]\nrssi_top_dbm = -130").is_err());
        assert!(AppConfig::from_toml("[status_bar]\nrssi_bottom_dbm = -25").is_err());
        assert!(AppConfig::from_toml("[status_bar]\nrssi_top_dbm = 40").is_err());
        assert!(
            AppConfig::from_toml("[status_bar]\nrssi_top_dbm = -50\nrssi_bottom_dbm = -55")
                .is_err()
        );
        assert_eq!(AppConfig::default().status_bar.rssi_top_dbm, -20);
        assert_eq!(AppConfig::default().status_bar.rssi_bottom_dbm, -120);
    }

    #[test]
    fn map_opacity_and_pulse_are_range_checked() {
        assert!(AppConfig::from_toml("[map]\nbar_opacity = 1.5").is_err());
        assert!(AppConfig::from_toml("[map]\nbar_opacity = -0.1").is_err());
        assert!(AppConfig::from_toml("[lora]\npulse_secs = -1.0").is_err());
        let cfg = AppConfig::from_toml("[map]\nbar_opacity = 0.5\nshow_key = false\n\n[lora]\npulse_secs = 0.0").unwrap();
        assert_eq!(cfg.map.bar_opacity, 0.5);
        assert!(!cfg.map.show_key);
        assert_eq!(cfg.lora.pulse_secs, 0.0);
        assert_eq!(AppConfig::default().lora.pulse_secs, 10.0);
    }

    /// The tile provider and its key read back from a file, a file that
    /// predates them stays on OpenStreetMap, and a key is stored trimmed so
    /// a pasted newline never reaches a URL.
    #[test]
    fn map_tiles_and_key_are_read_and_default_to_osm() {
        let cfg = AppConfig::from_toml("[map]\ntiles = \"ArcGIS\"\narcgis_key = \" abc \"\n").unwrap();
        assert_eq!(cfg.map.tiles, TileProvider::ArcGis);
        assert_eq!(cfg.map.arcgis_key, "abc");
        let old = AppConfig::from_toml("[map]\nshow_key = false\n").unwrap();
        assert_eq!(old.map.tiles, TileProvider::Osm);
        assert_eq!(old.map.arcgis_key, "");
        assert!(AppConfig::from_toml("[map]\ntiles = \"google\"").is_err());

        let path = std::env::temp_dir().join(format!("gps-gui-tiles-{}.toml", std::process::id()));
        let path = path.to_str().unwrap().to_string();
        let _ = std::fs::remove_file(&path);
        let mut cfg = AppConfig::default();
        cfg.map.tiles = TileProvider::ArcGis;
        cfg.map.arcgis_key = "mine".to_string();
        cfg.save(&path).unwrap();
        cfg.save(&path).unwrap();
        let back = AppConfig::load(&path).unwrap();
        assert_eq!(back.map.tiles, TileProvider::ArcGis);
        assert_eq!(back.map.arcgis_key, "mine");
        let _ = std::fs::remove_file(path);
    }

    /// The map switch for the connected board reads back from a file, and a
    /// file that predates it leaves the board drawn.
    #[test]
    fn board_on_map_is_read_and_defaults_to_shown() {
        let cfg = AppConfig::from_toml("[ble]\nshow_on_map = false\n").unwrap();
        assert!(!cfg.ble.show_on_map);
        let cfg = AppConfig::from_toml("[ble]\nenabled = true\n").unwrap();
        assert!(cfg.ble.show_on_map);
    }

    #[test]
    fn save_edits_in_place_and_round_trips() {
        let path = std::env::temp_dir().join("gps-gui-rs-config-save-test.toml");
        let path = path.to_str().unwrap();
        std::fs::write(
            path,
            "# keep me\n[sizes]\nmarker = 8.0 # and me\n\n[extra]\nunknown = 1\n",
        )
        .unwrap();

        let mut cfg = AppConfig::default();
        cfg.sizes.marker = 12.5;
        cfg.colors.track = Color32::from_rgb(1, 2, 3);
        // False: the file was already there, so it was edited, not generated.
        assert!(!cfg.save(path).unwrap());

        let text = std::fs::read_to_string(path).unwrap();
        assert!(text.contains("# keep me"), "{text}");
        assert!(text.contains("# and me"), "{text}");
        assert!(text.contains("unknown = 1"), "{text}");

        let back = AppConfig::load(path).unwrap();
        assert_eq!(back.sizes.marker, 12.5);
        assert_eq!(back.colors.track, Color32::from_rgb(1, 2, 3));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_log_reference_needs_both_halves() {
        let cfg = AppConfig::from_toml("[log]\nref_lat = 51.4779\nref_lon = -0.0015\n").unwrap();
        assert_eq!(cfg.log.reference(), Some((51.4779, -0.0015)));
        assert_eq!(AppConfig::default().log.reference(), None);
        // A lone half is a typo, and reads as an error rather than as unset.
        assert!(AppConfig::from_toml("[log]\nref_lat = 51.4779\n").is_err());
        assert!(AppConfig::from_toml("[log]\nref_lon = -0.0015\n").is_err());
        assert!(AppConfig::from_toml("[log]\nref_lat = 91.0\nref_lon = 0.0\n").is_err());
    }

    #[test]
    fn clearing_the_reference_removes_the_keys() {
        let path = std::env::temp_dir().join("gps-gui-rs-log-ref-test.toml");
        let path = path.to_str().unwrap();
        std::fs::write(path, "[log]\nref_lat = 51.4779\nref_lon = -0.0015\n").unwrap();

        let mut cfg = AppConfig::load(path).unwrap();
        assert_eq!(cfg.log.reference(), Some((51.4779, -0.0015)));
        // An unset coordinate has no empty form, so the keys have to go - left
        // behind as `""` the file would no longer load.
        cfg.log.set_reference(None);
        assert!(!cfg.save(path).unwrap());
        let text = std::fs::read_to_string(path).unwrap();
        assert!(!text.contains("ref_lat"), "{text}");
        assert_eq!(AppConfig::load(path).unwrap().log.reference(), None);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_generated_config_reads_back_with_the_log_table() {
        let mut cfg = AppConfig::default();
        cfg.log.auto_start = true;
        cfg.log.file = Some("/tmp/run.csv".to_string());
        cfg.log.set_reference(Some((51.4779, -0.0015)));
        let back = AppConfig::from_toml(&cfg.to_toml()).unwrap();
        assert!(back.log.auto_start);
        assert_eq!(back.log.file.as_deref(), Some("/tmp/run.csv"));
        assert_eq!(back.log.reference(), Some((51.4779, -0.0015)));
        // With nothing set the reference is shown as a comment, so a generated
        // file still loads and still says what the keys are.
        let empty = AppConfig::from_toml(&AppConfig::default().to_toml()).unwrap();
        assert_eq!(empty.log.reference(), None);
        assert_eq!(empty.log.file, None);
    }

    #[test]
    fn mac_normalization_folds_case_and_separators() {
        assert_eq!(normalize_mac("aa:bb:cc:dd:ee:ff"), "AA:BB:CC:DD:EE:FF");
        assert_eq!(normalize_mac("AA-BB-CC-DD-EE-FF"), "AA:BB:CC:DD:EE:FF");
        assert_eq!(normalize_mac(" aabbccddeeff "), "AA:BB:CC:DD:EE:FF");
    }

    #[test]
    fn names_are_found_however_the_mac_is_spelled() {
        let cfg = AppConfig::from_toml(
            "[ble]\nmac = \"aa:bb:cc:dd:ee:ff\"\n\n[ble.names]\n\"AA-BB-CC-DD-EE-FF\" = \"Truck\"\n",
        )
        .unwrap();
        // Stored canonically regardless of how the file spelled it.
        assert_eq!(cfg.ble.names.get("AA:BB:CC:DD:EE:FF").unwrap(), "Truck");
        assert_eq!(cfg.ble.name_of("aa:bb:cc:dd:ee:ff"), Some("Truck"));
        // The pinned MAC matches the same board despite the different spelling.
        assert!(cfg.ble.is_selected("AA-BB-CC-DD-EE-FF"));
        assert!(!cfg.ble.is_selected("11:22:33:44:55:66"));
    }

    #[test]
    fn unnamed_board_falls_back_to_its_mac() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.ble.label_of("AA:BB:CC:DD:EE:FF"), "AA:BB:CC:DD:EE:FF");
    }

    #[test]
    fn set_name_adds_renames_and_forgets() {
        let mut ble = BleSettings::default();
        ble.set_name("aa:bb:cc:dd:ee:ff", "Truck");
        assert_eq!(ble.name_of("AA:BB:CC:DD:EE:FF"), Some("Truck"));
        // Renaming through a different spelling hits the same entry.
        ble.set_name("AA-BB-CC-DD-EE-FF", "Van");
        assert_eq!(ble.names.len(), 1);
        assert_eq!(ble.name_of("AA:BB:CC:DD:EE:FF"), Some("Van"));
        // Blanking the name forgets the board.
        ble.set_name("AA:BB:CC:DD:EE:FF", "  ");
        assert!(ble.names.is_empty());
    }

    #[test]
    fn saved_names_round_trip_and_forgetting_sticks() {
        let path = std::env::temp_dir().join("gps-gui-rs-config-names-test.toml");
        let path = path.to_str().unwrap();
        let _ = std::fs::remove_file(path);

        let mut cfg = AppConfig::default();
        cfg.ble.set_name("AA:BB:CC:DD:EE:FF", "Truck");
        cfg.ble.set_name("11:22:33:44:55:66", "Backpack");
        // No file yet, so this generates one from the template.
        assert!(cfg.save(path).unwrap());
        let back = AppConfig::load(path).unwrap();
        assert_eq!(back.ble.name_of("AA:BB:CC:DD:EE:FF"), Some("Truck"));
        assert_eq!(back.ble.name_of("11:22:33:44:55:66"), Some("Backpack"));

        // Forget one and save over the existing file: the line has to go, not
        // linger because the edit-in-place path only ever adds.
        let mut cfg = back;
        cfg.ble.set_name("11:22:33:44:55:66", "");
        assert!(!cfg.save(path).unwrap());
        let back = AppConfig::load(path).unwrap();
        assert_eq!(back.ble.name_of("AA:BB:CC:DD:EE:FF"), Some("Truck"));
        assert_eq!(back.ble.name_of("11:22:33:44:55:66"), None);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn lora_names_parse_by_address_and_round_trip() {
        let cfg = AppConfig::from_toml(
            "[lora]\nshow_path = false\n\n[lora.names]\n3 = \"Truck\"\n7 = \"Drone\"\n",
        )
        .unwrap();
        assert!(!cfg.lora.show_path);
        assert_eq!(cfg.lora.name_of(3), Some("Truck"));
        assert_eq!(cfg.lora.label_of(7), "Drone");
        // No entry falls back to the address.
        assert_eq!(cfg.lora.label_of(12), "Node 12");

        // Address 0 is the local GPS, never a remote, so it is a bad key.
        assert!(AppConfig::from_toml("[lora.names]\n0 = \"nope\"").is_err());
        assert!(AppConfig::from_toml("[lora.names]\nfoo = \"nope\"").is_err());

        // The generated default file reads back unchanged.
        let back = AppConfig::from_toml(&AppConfig::default().to_toml()).unwrap();
        assert!(back.lora.show_path);
        assert!(back.lora.names.is_empty());
    }

    #[test]
    fn saved_lora_names_round_trip_and_forgetting_sticks() {
        let path = std::env::temp_dir().join("gps-gui-rs-config-lora-test.toml");
        let path = path.to_str().unwrap();
        let _ = std::fs::remove_file(path);

        let mut cfg = AppConfig::default();
        cfg.lora.set_name(3, "Truck");
        cfg.lora.set_name(7, "Drone");
        assert!(cfg.save(path).unwrap());
        let back = AppConfig::load(path).unwrap();
        assert_eq!(back.lora.name_of(3), Some("Truck"));
        assert_eq!(back.lora.name_of(7), Some("Drone"));

        // Forget one and save over the file: the line must go, not linger.
        let mut cfg = back;
        cfg.lora.set_name(7, "");
        assert!(!cfg.save(path).unwrap());
        let back = AppConfig::load(path).unwrap();
        assert_eq!(back.lora.name_of(3), Some("Truck"));
        assert_eq!(back.lora.name_of(7), None);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn compass_rate_is_range_checked() {
        assert!(AppConfig::from_toml("[compass]\narrow_hz = 0.0").is_err());
        assert!(AppConfig::from_toml("[compass]\narrow_hz = 1000.0").is_err());
        let cfg = AppConfig::from_toml("[compass]\nmarker_arrow = false\narrow_hz = 2.0").unwrap();
        assert!(!cfg.compass.marker_arrow);
        assert_eq!(cfg.compass.arrow_hz, 2.0);
    }

    /// An empty string is how a surface color says "leave it to the theme", so
    /// the key can sit in the file unset. It has to survive a save, which is
    /// what would otherwise quietly turn the override back on.
    #[test]
    fn empty_surface_colors_mean_the_theme() {
        let cfg =
            AppConfig::from_toml("[ui]\nbackground = \"\"\nbutton = \"#202020\"\ntext = \"\"")
                .unwrap();
        assert_eq!(cfg.ui.background, None);
        assert_eq!(cfg.ui.text, None);
        assert_eq!(cfg.ui.button, Some(Color32::from_rgb(32, 32, 32)));

        let path = std::env::temp_dir().join("gps-gui-rs-config-surface-test.toml");
        let path = path.to_str().unwrap();
        let _ = std::fs::remove_file(path);
        assert!(cfg.save(path).unwrap());
        let back = AppConfig::load(path).unwrap();
        assert_eq!(back.ui.background, None);
        assert_eq!(back.ui.text, None);
        assert_eq!(back.ui.button, Some(Color32::from_rgb(32, 32, 32)));
        let _ = std::fs::remove_file(path);
    }

    /// The default is the light theme, not the system's: this is an app that
    /// gets read outdoors, and following a phone that is on dark at noon is
    /// the wrong default for it.
    #[test]
    fn theme_defaults_to_light_and_round_trips() {
        assert_eq!(AppConfig::default().ui.theme, ThemeChoice::Light);
        assert!(AppConfig::from_toml("[ui]\ntheme = \"midnight\"").is_err());
        assert_eq!(
            AppConfig::from_toml("[ui]\ntheme = \"DARK\"").unwrap().ui.theme,
            ThemeChoice::Dark
        );
        assert_eq!(
            AppConfig::from_toml("[ui]\ntheme = \"auto\"").unwrap().ui.theme,
            ThemeChoice::System
        );

        let cfg = AppConfig::from_toml("[ui]\ntheme = \"dark\"").unwrap();
        let path = std::env::temp_dir().join("gps-gui-rs-config-theme-test.toml");
        let path = path.to_str().unwrap();
        let _ = std::fs::remove_file(path);
        assert!(cfg.save(path).unwrap());
        assert_eq!(AppConfig::load(path).unwrap().ui.theme, ThemeChoice::Dark);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn text_scale_is_range_checked_and_saved() {
        assert!(AppConfig::from_toml("[ui]\ntext_scale = 0.1").is_err());
        assert!(AppConfig::from_toml("[ui]\ntext_scale = 10.0").is_err());
        let cfg = AppConfig::from_toml("[ui]\ntext_scale = 1.5").unwrap();
        assert_eq!(cfg.ui.text_scale, 1.5);

        let path = std::env::temp_dir().join("gps-gui-rs-config-text-scale-test.toml");
        let path = path.to_str().unwrap();
        let _ = std::fs::remove_file(path);
        assert!(cfg.save(path).unwrap());
        assert_eq!(AppConfig::load(path).unwrap().ui.text_scale, 1.5);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn status_bar_dwell_is_range_checked_and_saved() {
        assert!(AppConfig::from_toml("[status_bar]\ncycle_secs = 0.0").is_err());
        assert!(AppConfig::from_toml("[status_bar]\ncycle_secs = 600.0").is_err());
        let cfg = AppConfig::from_toml("[status_bar]\nshow = true\ncycle_secs = 8.0").unwrap();
        assert!(cfg.status_bar.show);
        assert_eq!(cfg.status_bar.cycle_secs, 8.0);

        let path = std::env::temp_dir().join("gps-gui-rs-config-status-bar-test.toml");
        let path = path.to_str().unwrap();
        let _ = std::fs::remove_file(path);
        assert!(cfg.save(path).unwrap());
        let back = AppConfig::load(path).unwrap();
        assert!(back.status_bar.show);
        assert_eq!(back.status_bar.cycle_secs, 8.0);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    #[ignore]
    fn regenerate_the_shipped_config() {
        std::fs::write("app-settings.toml", AppConfig::default().to_toml()).unwrap();
    }

    /// The settings file the repo ships is the generated default, so it
    /// documents every key as the code writes it rather than as it once
    /// did. `cargo test regenerate_the_shipped_config -- --ignored` writes
    /// it again.
    #[test]
    fn the_shipped_config_is_the_generated_default() {
        let text = include_str!("../app-settings.toml");
        assert_eq!(
            text,
            AppConfig::default().to_toml(),
            "app-settings.toml is out of date - regenerate it"
        );
        let back = AppConfig::from_toml(text).expect("the shipped file loads");
        assert_eq!(back.phone.name, AppConfig::default().phone.name);
        assert_eq!(back.ble.location, AppConfig::default().ble.location);
    }

    #[test]
    fn sizes_reject_non_positive() {
        assert!(super::AppConfig::from_toml("[sizes]\nmarker = 0.0").is_err());
        assert!(super::AppConfig::from_toml("[sizes]\nbeacon = -1.0").is_err());
        let cfg = super::AppConfig::from_toml("[sizes]\nmarker = 12.0").unwrap();
        assert_eq!(cfg.sizes.marker, 12.0);
    }
}
