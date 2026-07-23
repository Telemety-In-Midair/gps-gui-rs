//! Loadable, editable RADIO.TOML for the WIO-E5 board.
//!
//! Unlike [`crate::config`] (the app's own settings), this file belongs to the
//! firmware: the Radio page reads it, offers a per-field editor, and writes it
//! back keeping the comments and the `<key>_description` help strings intact.
//! `toml_edit` is used precisely so a round-trip preserves everything the file
//! carries; only the edited value changes.
//!
//! Each editable key is rendered with an input matched to its data type. The
//! type is inferred from the TOML value, but a sibling `<key>_type` string can
//! force it, which also lets a string be presented as a dropdown:
//!
//! ```toml
//! power_mode = "full"
//! power_mode_type = "enum:full,psmoo,psmct"   # dropdown of the three choices
//! meas_rate_ms_type = "int"                    # force an integer input
//! ```
//!
//! Valid `<key>_type` values are `int`, `float`, `bool`, `string`, or
//! `enum:a,b,c`. The firmware ignores unknown keys, so both `<key>_description`
//! and `<key>_type` are inert to the board.
//!
//! Saving copies the previous on-disk file into a `radio-backups` directory
//! (next to the file) under a timestamped name before overwriting, so old
//! versions are kept and can be restored.
//!
//! With no file to load, [`RadioDoc::default_at`] starts one from the
//! firmware's own reference config, so a board can be set up from the app
//! alone.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use midair_proto::radiocfg::RadioConfig;
use toml_edit::{DocumentMut, Item, Table, Value};

/// The input a field is rendered with, inferred from its TOML value or forced
/// by a sibling `<key>_type` string.
#[derive(Clone, PartialEq, Debug)]
pub enum FieldType {
    /// Whole number: a draggable integer input.
    Int,
    /// Real number: a draggable float input.
    Float,
    /// Boolean: a checkbox.
    Bool,
    /// Free text: a single-line text field.
    Str,
    /// A string constrained to a fixed set of choices: a dropdown. Options come
    /// from `<key>_type = "enum:a,b,c"`.
    Enum(Vec<String>),
}

/// One editable setting: where it lives and how to render it. The live value is
/// read from (and written to) the document, so this holds only fixed metadata.
pub struct RadioField {
    /// The `[section]` the key sits under; empty for a top-level key.
    pub section: String,
    pub key: String,
    pub ty: FieldType,
    /// Help text from the sibling `<key>_description` string, if present.
    pub description: Option<String>,
}

/// A value being edited, typed so the editor can bind a type-specific widget to
/// it and write it back without string parsing.
#[derive(Clone)]
pub enum EditVal {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
}

/// A loaded RADIO.TOML: the parsed document (the source of truth for values),
/// the ordered list of editable fields, and where it came from.
pub struct RadioDoc {
    doc: DocumentMut,
    /// Editable fields in file order, grouped by section.
    pub fields: Vec<RadioField>,
    /// The file this was loaded from and is saved back to.
    pub path: PathBuf,
    /// A value has been edited since the last load/save.
    pub dirty: bool,
}

/// Largest config the firmware accepts, enforced on the board in the ESP's
/// OP_BEGIN check, the WIO's transfer buffer and the buffer RADIO.CFG is read
/// into at boot. A push over that size is refused before any byte moves.
pub const CONFIG_MAX: usize = 1024;

/// A complete RADIO.TOML at the firmware defaults, comments and help strings
/// included: the reference file the firmware itself ships, baked in at build
/// time so the app can lay down a config with no file to start from and no
/// second copy of the schema to keep in step. The board is a sibling checkout
/// this crate already builds against.
const DEFAULT_TOML: &str = include_str!("../../esp32c6-gps/RADIO.example.toml");

/// Whether `key` is one of the metadata keys (`<name>_description` /
/// `<name>_type`) rather than an editable setting.
fn is_meta_key(key: &str) -> bool {
    key.ends_with("_description") || key.ends_with("_type")
}

/// Parse a `<key>_type` string into a [`FieldType`], or `None` if unrecognized
/// (in which case the value's own type is used instead).
fn parse_type_spec(spec: &str) -> Option<FieldType> {
    let spec = spec.trim();
    if let Some(rest) = spec.strip_prefix("enum:") {
        let opts: Vec<String> = rest
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        return (!opts.is_empty()).then_some(FieldType::Enum(opts));
    }
    match spec.to_lowercase().as_str() {
        "int" | "integer" => Some(FieldType::Int),
        "float" | "double" => Some(FieldType::Float),
        "bool" | "boolean" => Some(FieldType::Bool),
        "string" | "str" => Some(FieldType::Str),
        _ => None,
    }
}

/// The field type for `key`: the sibling `<key>_type` override when valid,
/// otherwise inferred from the value itself.
fn field_type(table: &Table, key: &str, val: &Value) -> FieldType {
    if let Some(spec) = table
        .get(&format!("{key}_type"))
        .and_then(Item::as_str)
        .and_then(parse_type_spec)
    {
        return spec;
    }
    match val {
        Value::Integer(_) => FieldType::Int,
        Value::Float(_) => FieldType::Float,
        Value::Boolean(_) => FieldType::Bool,
        _ => FieldType::Str,
    }
}

/// Append the scalar settings of one table to `out`. Nested tables and the
/// metadata keys are skipped.
fn collect_table(section: &str, table: &Table, out: &mut Vec<RadioField>) {
    for (key, item) in table.iter() {
        let Some(val) = item.as_value() else { continue };
        if is_meta_key(key) {
            continue;
        }
        out.push(RadioField {
            section: section.to_string(),
            key: key.to_string(),
            ty: field_type(table, key, val),
            description: table
                .get(&format!("{key}_description"))
                .and_then(Item::as_str)
                .map(str::to_string),
        });
    }
}

/// Build the editable-field list from a document: top-level scalars first, then
/// each `[section]` in file order.
fn collect_fields(doc: &DocumentMut) -> Vec<RadioField> {
    let root = doc.as_table();
    let mut out = Vec::new();
    collect_table("", root, &mut out);
    for (name, item) in root.iter() {
        if let Some(table) = item.as_table() {
            collect_table(name, table, &mut out);
        }
    }
    out
}

impl RadioDoc {
    /// Read and parse the RADIO.TOML at `path`. A returned `Err` is a
    /// human-readable message for the UI.
    pub fn load(path: &str) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        let doc: DocumentMut = text.parse().map_err(|e| format!("{path}: {e}"))?;
        Ok(Self {
            fields: collect_fields(&doc),
            doc,
            path: PathBuf::from(path),
            dirty: false,
        })
    }

    /// A fresh document at the firmware defaults, aimed at `path` but not
    /// written there: it starts dirty, so the file only appears when the Radio
    /// page's Save is pressed (which backs up anything already at `path`
    /// first). This is how a board gets a RADIO.TOML without one being copied
    /// onto the SD card by hand.
    pub fn default_at(path: &str) -> Result<Self, String> {
        let doc: DocumentMut = DEFAULT_TOML
            .parse()
            .map_err(|e| format!("built-in default config: {e}"))?;
        Ok(Self {
            fields: collect_fields(&doc),
            doc,
            path: PathBuf::from(path),
            dirty: true,
        })
    }

    /// The value item at `section`/`key`, if present.
    fn value(&self, section: &str, key: &str) -> Option<&Value> {
        let table = if section.is_empty() {
            self.doc.as_table()
        } else {
            self.doc.get(section)?.as_table()?
        };
        table.get(key)?.as_value()
    }

    /// The current value formatted for read-only display (no surrounding
    /// whitespace or quotes).
    pub fn display_at(&self, section: &str, key: &str) -> String {
        match self.value(section, key) {
            Some(Value::String(s)) => s.value().clone(),
            Some(Value::Integer(i)) => i.value().to_string(),
            Some(Value::Float(f)) => f.value().to_string(),
            Some(Value::Boolean(b)) => b.value().to_string(),
            Some(other) => other.to_string().trim().to_string(),
            None => String::new(),
        }
    }

    /// The field metadata for `section`/`key`, if it is an editable field.
    fn field_at(&self, section: &str, key: &str) -> Option<&RadioField> {
        self.fields
            .iter()
            .find(|f| f.section == section && f.key == key)
    }

    /// Seed an [`EditVal`] from the current value, typed per the field so the
    /// editor binds the matching widget.
    pub fn edit_val_at(&self, section: &str, key: &str) -> EditVal {
        let v = self.value(section, key);
        match self.field_at(section, key).map(|f| &f.ty) {
            Some(FieldType::Int) => EditVal::Int(v.and_then(Value::as_integer).unwrap_or(0)),
            Some(FieldType::Float) => EditVal::Float(v.and_then(Value::as_float).unwrap_or(0.0)),
            Some(FieldType::Bool) => EditVal::Bool(v.and_then(Value::as_bool).unwrap_or(false)),
            _ => EditVal::Str(v.and_then(Value::as_str).unwrap_or_default().to_string()),
        }
    }

    /// Write an edited value back into the document, keeping the value's
    /// original formatting (leading/trailing whitespace). Marks the doc dirty.
    pub fn apply(&mut self, section: &str, key: &str, val: &EditVal) {
        let table = if section.is_empty() {
            Some(self.doc.as_table_mut())
        } else {
            self.doc.get_mut(section).and_then(Item::as_table_mut)
        };
        let Some(item) = table.and_then(|t| t.get_mut(key)) else {
            return;
        };
        let Some(existing) = item.as_value() else {
            return;
        };
        // Preserve the value's decor (surrounding whitespace) so only the number
        // or string itself changes on disk.
        let decor = existing.decor().clone();
        let mut new: Value = match val {
            EditVal::Int(i) => (*i).into(),
            EditVal::Float(f) => (*f).into(),
            EditVal::Bool(b) => (*b).into(),
            EditVal::Str(s) => s.as_str().into(),
        };
        *new.decor_mut() = decor;
        *item = Item::Value(new);
        self.dirty = true;
    }

    /// Overlay the config a board reported over BLE onto this document, so the
    /// editor shows what the board is actually running while keeping the
    /// file's comments, help strings and dropdown hints. Only editable fields
    /// the board blob carries are touched; anything else (a commented-out
    /// hardware key like `tcxo_volts`, or an app-only field) keeps the
    /// document's own value, and re-pushing therefore leaves it untouched too.
    /// Marks the document dirty, since its values now differ from the file on
    /// disk.
    pub fn apply_config(&mut self, cfg: &RadioConfig) {
        let targets: Vec<(String, String)> = self
            .fields
            .iter()
            .map(|f| (f.section.clone(), f.key.clone()))
            .collect();
        for (section, key) in targets {
            if let Some(val) = config_value(cfg, &key) {
                self.apply(&section, &key, &val);
            }
        }
    }

    /// The document as it goes to the board: comments, blank lines and the
    /// `<key>_description` / `<key>_type` metadata keys dropped, each kept
    /// line trimmed. The firmware ignores every stripped byte, and the
    /// documented file is several times [`CONFIG_MAX`], so stripping is what
    /// makes a push fit at all - not a size optimization.
    pub fn wire_bytes(&self) -> Vec<u8> {
        let mut out = String::new();
        for line in self.doc.to_string().lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let key = trimmed.split('=').next().unwrap_or("").trim();
            if is_meta_key(key) {
                continue;
            }
            out.push_str(trimmed);
            out.push('\n');
        }
        out.into_bytes()
    }

    /// The directory backups are written to and read from: `radio-backups` next
    /// to the file.
    fn backup_dir(&self) -> PathBuf {
        let dir = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        dir.join("radio-backups")
    }

    /// Copy the current on-disk file into the backup directory under a
    /// timestamped name. `Ok(None)` when there is no existing file to back up.
    fn backup_existing(&self) -> Result<Option<PathBuf>, String> {
        if !self.path.exists() {
            return Ok(None);
        }
        let dir = self.backup_dir();
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let stem = self
            .path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("RADIO.toml");
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let dest = dir.join(format!("{stem}.{stamp}.bak"));
        std::fs::copy(&self.path, &dest).map_err(|e| format!("{}: {e}", dest.display()))?;
        Ok(Some(dest))
    }

    /// Back up the previous file, then write the current document. Returns the
    /// backup path when one was made.
    pub fn save(&mut self) -> Result<Option<PathBuf>, String> {
        let backup = self.backup_existing()?;
        std::fs::write(&self.path, self.doc.to_string())
            .map_err(|e| format!("{}: {e}", self.path.display()))?;
        self.dirty = false;
        Ok(backup)
    }

    /// The kept backups, newest first. Filenames carry a unix-seconds stamp so a
    /// reverse sort orders them by age.
    pub fn backups(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(self.backup_dir())
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "bak"))
            .collect();
        v.sort();
        v.reverse();
        v
    }

    /// Load a backup's contents into this document (keeping `path` pointed at
    /// the live file), marking it dirty so a Save writes it back as current.
    pub fn restore(&mut self, backup: &Path) -> Result<(), String> {
        let text =
            std::fs::read_to_string(backup).map_err(|e| format!("{}: {e}", backup.display()))?;
        let doc: DocumentMut = text.parse().map_err(|e| format!("{}: {e}", backup.display()))?;
        self.fields = collect_fields(&doc);
        self.doc = doc;
        self.dirty = true;
        Ok(())
    }
}

/// Format a beacon field mask as the `fields` string, in the wire order the
/// payload uses. Names match those the firmware parser and the reference
/// file's help text use, so the result parses back unchanged.
fn fields_to_string(mask: u8) -> String {
    use midair_proto::lora;
    [
        (lora::FIELD_LAT, "lat"),
        (lora::FIELD_LON, "lon"),
        (lora::FIELD_ALT, "altitude"),
        (lora::FIELD_SPEED, "speed"),
        (lora::FIELD_COURSE, "course"),
        (lora::FIELD_SATS, "sats"),
        (lora::FIELD_TIME, "time"),
    ]
    .iter()
    .filter(|(bit, _)| mask & bit != 0)
    .map(|(_, name)| *name)
    .collect::<Vec<_>>()
    .join(",")
}

/// A board's value for a config key, typed to match that field's editor
/// widget. `None` for a key the read-back blob does not carry (a future or
/// app-only key), so [`RadioDoc::apply_config`] leaves it at the document's
/// value. Enum and mask fields render through the shared crate's `as_str`
/// helpers, which the firmware parser accepts back unchanged.
fn config_value(cfg: &RadioConfig, key: &str) -> Option<EditVal> {
    Some(match key {
        "frequency_hz" => EditVal::Int(cfg.frequency_hz as i64),
        "spreading_factor" => EditVal::Int(cfg.spreading_factor as i64),
        "bandwidth_khz" => EditVal::Int(cfg.bandwidth_khz as i64),
        "coding_rate" => EditVal::Int(cfg.coding_rate as i64),
        "power_dbm" => EditVal::Int(cfg.power_dbm as i64),
        "rx_boost" => EditVal::Bool(cfg.rx_boost),
        "dcdc_enabled" => EditVal::Bool(cfg.dcdc_enabled),
        "tcxo_volts" => EditVal::Str(cfg.tcxo_volts.as_str().to_string()),
        "tcxo_startup_ms" => EditVal::Int(cfg.tcxo_startup_ms as i64),
        "address" => EditVal::Int(cfg.address as i64),
        "role" => EditVal::Str(cfg.role.as_str().to_string()),
        "max_hops" => EditVal::Int(cfg.max_hops as i64),
        "dedup_ttl_s" => EditVal::Int(cfg.dedup_ttl_s as i64),
        "interval_s" | "beacon_interval_s" => EditVal::Int(cfg.beacon_interval_s as i64),
        "fields" | "beacon_fields" => EditVal::Str(fields_to_string(cfg.beacon_fields)),
        "sd_enabled" => EditVal::Bool(cfg.sd_enabled),
        "verbose" => EditVal::Bool(cfg.verbose),
        "gps_enabled" => EditVal::Bool(cfg.gps.gps_enabled),
        "glonass_enabled" => EditVal::Bool(cfg.gps.glonass_enabled),
        "galileo_enabled" => EditVal::Bool(cfg.gps.galileo_enabled),
        "beidou_enabled" => EditVal::Bool(cfg.gps.beidou_enabled),
        "qzss_enabled" => EditVal::Bool(cfg.gps.qzss_enabled),
        "sbas_enabled" => EditVal::Bool(cfg.gps.sbas_enabled),
        "power_mode" => EditVal::Str(cfg.gps.power_mode.as_str().to_string()),
        "meas_rate_ms" => EditVal::Int(cfg.gps.meas_rate_ms as i64),
        "dynamic_model" | "dyn_model" => EditVal::Str(cfg.gps.dyn_model.as_str().to_string()),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
# a comment that must survive a round-trip
[radio]
frequency_hz = 915000000
frequency_hz_description = \"RF center frequency.\"

spreading_factor = 7

[gps]
gps_enabled = true
power_mode = \"full\"
power_mode_type = \"enum:full,psmoo,psmct\"
";

    fn doc() -> RadioDoc {
        RadioDoc {
            doc: SAMPLE.parse().unwrap(),
            fields: collect_fields(&SAMPLE.parse().unwrap()),
            path: PathBuf::from("RADIO.toml"),
            dirty: false,
        }
    }

    #[test]
    fn collects_fields_and_skips_metadata() {
        let d = doc();
        let keys: Vec<_> = d.fields.iter().map(|f| f.key.as_str()).collect();
        assert_eq!(keys, ["frequency_hz", "spreading_factor", "gps_enabled", "power_mode"]);
        // No `_description` / `_type` key leaks in as an editable field.
        assert!(!keys.iter().any(|k| k.contains("_description") || k.contains("_type")));
    }

    #[test]
    fn infers_and_overrides_types() {
        let d = doc();
        let ty = |k: &str| &d.fields.iter().find(|f| f.key == k).unwrap().ty;
        assert_eq!(ty("frequency_hz"), &FieldType::Int);
        assert_eq!(ty("gps_enabled"), &FieldType::Bool);
        assert_eq!(
            ty("power_mode"),
            &FieldType::Enum(vec!["full".into(), "psmoo".into(), "psmct".into()])
        );
    }

    #[test]
    fn description_is_read() {
        let d = doc();
        let f = d.fields.iter().find(|f| f.key == "frequency_hz").unwrap();
        assert_eq!(f.description.as_deref(), Some("RF center frequency."));
    }

    #[test]
    fn built_in_default_parses_with_editable_fields() {
        let d = RadioDoc::default_at("RADIO.toml").unwrap();
        // The firmware's reference config carries every section the page edits.
        for key in ["frequency_hz", "address", "interval_s", "meas_rate_ms"] {
            assert!(d.fields.iter().any(|f| f.key == key), "missing {key}");
        }
        // Nothing is on disk yet, so it must read as unsaved.
        assert!(d.dirty);
        // The help strings and the enum hints came along with it.
        let power = d.fields.iter().find(|f| f.key == "power_mode").unwrap();
        assert!(power.description.is_some());
        assert!(matches!(power.ty, FieldType::Enum(_)));
        // A plain bool key needs no page code to become a checkbox - it is
        // picked up from the file. Asserting one keeps that path covered, so a
        // firmware key added to the reference config cannot silently stop
        // reaching the editor.
        let rx_boost = d.fields.iter().find(|f| f.key == "rx_boost").unwrap();
        assert_eq!(rx_boost.ty, FieldType::Bool);
        assert!(rx_boost.description.is_some());
    }

    #[test]
    fn wire_bytes_strips_docs_and_keeps_settings() {
        let d = doc();
        let wire = String::from_utf8(d.wire_bytes()).unwrap();
        // Sections and settings survive; comments and metadata do not.
        assert!(wire.contains("[radio]"));
        assert!(wire.contains("frequency_hz = 915000000"));
        assert!(wire.contains("power_mode = \"full\""));
        assert!(!wire.contains('#'));
        assert!(!wire.contains("_description"));
        assert!(!wire.contains("_type"));
        assert!(!wire.contains("\n\n"));
    }

    /// The whole point of stripping: the reference config, several KB with its
    /// documentation, must come out under the firmware's config ceiling.
    #[test]
    fn default_config_fits_on_the_wire() {
        let d = RadioDoc::default_at("RADIO.toml").unwrap();
        let n = d.wire_bytes().len();
        assert!(n > 0 && n <= CONFIG_MAX, "{n} bytes");
    }

    #[test]
    fn apply_changes_only_the_value_and_keeps_comments() {
        let mut d = doc();
        d.apply("radio", "frequency_hz", &EditVal::Int(868000000));
        d.apply("gps", "power_mode", &EditVal::Str("psmct".into()));
        let out = d.doc.to_string();
        assert!(out.contains("frequency_hz = 868000000"));
        assert!(out.contains("power_mode = \"psmct\""));
        // Untouched context is preserved verbatim.
        assert!(out.contains("# a comment that must survive a round-trip"));
        assert!(out.contains("frequency_hz_description = \"RF center frequency.\""));
        assert!(d.dirty);
    }

    /// Overlaying a board's reported config fills the editor with its values
    /// and, crucially, survives a round-trip back through the firmware parser
    /// - the enum and mask fields have to render into strings the parser
    /// accepts, or a fetched config could not be sent back.
    #[test]
    fn apply_config_overlays_and_round_trips() {
        use midair_proto::lora;
        use midair_proto::radiocfg::{parse, DynModel, GpsConfig, PowerMode, Role};

        let mut d = RadioDoc::default_at("RADIO.toml").unwrap();
        let cfg = RadioConfig {
            address: 7,
            role: Role::Repeater,
            spreading_factor: 12,
            beacon_interval_s: 45,
            beacon_fields: lora::FIELD_LAT | lora::FIELD_LON | lora::FIELD_ALT,
            gps: GpsConfig {
                power_mode: PowerMode::PsmCyclic,
                dyn_model: DynModel::Automotive,
                ..GpsConfig::default()
            },
            ..RadioConfig::default()
        };
        d.apply_config(&cfg);

        // The editor now shows the board's values, enums as their spellings.
        assert_eq!(d.display_at("network", "address"), "7");
        assert_eq!(d.display_at("network", "role"), "repeater");
        assert_eq!(d.display_at("beacon", "fields"), "lat,lon,altitude");
        assert_eq!(d.display_at("gps", "power_mode"), "psmct");
        assert_eq!(d.display_at("gps", "dynamic_model"), "automotive");
        assert!(d.dirty);

        // Re-pushing sends exactly those values back: the parser reads them.
        let text = String::from_utf8(d.wire_bytes()).unwrap();
        let back = parse(&text).unwrap();
        assert_eq!(back.address, 7);
        assert_eq!(back.role, Role::Repeater);
        assert_eq!(back.spreading_factor, 12);
        assert_eq!(back.beacon_interval_s, 45);
        assert_eq!(back.beacon_fields, cfg.beacon_fields);
        assert_eq!(back.gps.power_mode, PowerMode::PsmCyclic);
        assert_eq!(back.gps.dyn_model, DynModel::Automotive);
    }
}
