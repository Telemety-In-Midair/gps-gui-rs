//! The app's prose, one module per page.
//!
//! Only what is not a label lives here: the hover texts saying what a
//! control does, and the few lines a page says of its own - a state, an
//! error, a range. Short labels stay next to the widget they name, where
//! reading the page tells you what the page says. Nothing here explains a
//! page or a group of controls: a title that needs a line under it is the
//! wrong title, and the reasoning belongs in the docs.
//!
//! Copy that takes a value is a function rather than a constant, `format!`
//! needing a literal pattern. A line describing something still in progress
//! ends without a stop, so the page can put the moving dots after it.

/// The Points page.
pub(crate) mod points {
    pub(crate) const SEARCH_HINT: &str = "search (51.47, phone, sky-1)";
}

/// The desktop manual-position bar.
pub(crate) mod manual {
    pub(crate) const BAD_COORD: &str = "Use lat, lon: 51.4779, -0.0015";
}

/// The Status page.
pub(crate) mod status {
    use crate::app::LocationSource;

    pub(crate) const WAITING_FIX: &str = "No position yet.";
    pub(crate) const NO_SOURCE: &str = "No position source on. See Settings.";
    pub(crate) const WARMING: &str = "Warming up.";
    pub(crate) const NO_TELEMETRY: &str = "No telemetry yet.";

    /// Where the position came from, for the line under it.
    pub(crate) fn source(source: LocationSource) -> &'static str {
        match source {
            LocationSource::Phone => "Source: phone",
            LocationSource::Node => "Source: node",
            LocationSource::Manual => "Source: typed in",
        }
    }
}

/// The Settings page: the app's own settings.
pub(crate) mod settings {
    pub(crate) const SAVE_HOVER: &str = "Write to the file";
    pub(crate) const RESET_HOVER: &str = "Only in the app until you save";

    pub(crate) const TEXT_SCALE_RESET_HOVER: &str = "Back to 1x";

    pub(crate) const ADJUST_HOVER: &str = "Pick a thing on any page and drag its sizes";
    pub(crate) const ADJUST_OPEN: &str = "Already open";

    pub(crate) const THEME_COLOR_HOVER: &str = "Unticked follows the theme";

    pub(crate) const PHONE_LOCATION_HOVER: &str = "Off, the phone's receiver stays off";
    pub(crate) const NODE_LOCATION_HOVER: &str = "The node's fix is your position";

    pub(crate) const CENTRAL_PATH_HOVER: &str = "Hiding never stops recording";
    pub(crate) const BOARD_ON_MAP_HOVER: &str = "Marker, path and distance line";
    pub(crate) const REMOTE_PATHS_HOVER: &str = "Nodes heard over LoRa";
    pub(crate) const PULSE_HOVER: &str = "Pulses while heard within this";

    pub(crate) const ARROW_HZ_HOVER: &str = "Lower saves power";
    pub(crate) const STATUS_CYCLE_HOVER: &str = "With two or more nodes";

    pub(crate) const DISCARD_HOVER: &str = "Not undoable";

    pub(crate) const DOWNLOAD_HOVER: &str = "Drag a box on the map";
    pub(crate) const DOWNLOAD_BUSY: &str = "Downloading";
}

/// The map's bottom status bar.
pub(crate) mod statusbar {
    pub(crate) const NO_NODES: &str = "No nodes heard";
}

/// The Bluetooth page: the BLE link, and the node's own settings.
pub(crate) mod bluetooth {
    use crate::app::secs_text;

    // Node picker.
    pub(crate) const SCAN_START_HOVER: &str = "Drops the current link";
    pub(crate) const SCAN_STOP_HOVER: &str = "Stop looking";
    pub(crate) const SCANNING: &str = "Scanning";
    pub(crate) const NO_BOARDS: &str = "No nodes yet.";
    pub(crate) const ANY_BOARD_HOVER: &str = "The first node that answers";
    pub(crate) const NAME_HOVER: &str = "Cleared, the node is forgotten";

    // Link controls.
    pub(crate) const RECONNECT_HOVER: &str = "Start over";
    pub(crate) const CONNECT_HOVER: &str = "To the selected node";
    pub(crate) const CONNECT_SLEEPING_HOVER: &str = "Keeps scanning for an advertising window";
    pub(crate) const DISCONNECT_HOVER: &str = "Lets the node sleep";

    // Connection settings.
    pub(crate) const AUTO_CONNECT_HOVER: &str = "At launch only";
    pub(crate) const SAVE_HOVER: &str = "Node list and auto-connect, to the settings file";
    pub(crate) const AWAITING_ACK: &str = "Waiting for the node";

    // Node name.
    pub(crate) const BOARD_NO_NAME: &str = "No name yet.";
    pub(crate) const NAME_CLEAR_HOVER: &str = "Back to the address name";

    /// What a name may be, for the box it is typed in.
    pub(crate) fn board_name_hover(max: usize) -> String {
        format!("Up to {max} letters, digits, - or _")
    }

    // Node power and sleep.
    pub(crate) const BOARD_NEED_LINK: &str = "Connect first.";
    pub(crate) const BOARD_TOO_NEW: &str = "Firmware newer than the app. Update the app.";
    pub(crate) const BOARD_READING: &str = "Reading";

    pub(crate) const MODE_STORED_HOVER: &str =
        "Everything down; wakes on the cadence to advertise. Disconnects now";
    pub(crate) const MODE_IDLE_HOVER: &str = "Reachable, GPS in backup, radio down";
    pub(crate) const MODE_TRACKING_HOVER: &str = "GPS, beacon and logging up. Survives a power cycle";
    pub(crate) const MODE_LISTENING_HOVER: &str =
        "GPS and receiver up, nothing sent, BLE always up. For the node beside the phone";

    pub(crate) const RADIO_STANDBY_HOVER: &str = "Parks the LoRa radio";
    pub(crate) const GPS_SLEEP_HOVER: &str = "The next fix is a cold one";

    pub(crate) const SLEEP_DISABLE_HOVER: &str = "Never sleep";
    pub(crate) const IDLE_DISABLE_HOVER: &str = "Stay idle";
    pub(crate) const IDLE_NEEDS_WAKE: &str = "Needs a wake check to store itself";
    pub(crate) const BLE_OFF_DISABLE_HOVER: &str = "Keep BLE up";

    /// The range the node clamps a setting to, for the hover on its row.
    pub(crate) fn range(min_s: u32, max_s: u32) -> String {
        format!("{} - {}", secs_text(min_s), secs_text(max_s))
    }

    pub(crate) const SLEEP_NOW_HOVER: &str = "The link drops";

    /// The blank entry of the Sleep now presets, labelled with what a blank
    /// box actually does.
    pub(crate) fn sleep_now_blank(secs: u32) -> String {
        format!("default ({})", secs_text(secs))
    }
}

/// The Radio page: the node's own RADIO.TOML.
pub(crate) mod radio {
    pub(crate) const SEND_HOVER: &str = "Applied at once and written to the node's card";
    pub(crate) const SEND_NEEDS_CONFIG: &str = "Load a config first";
    pub(crate) const SEND_NEEDS_LINK: &str = "Connect first";
    pub(crate) const SEND_WAITING: &str = "Waiting for the node";

    pub(crate) const FETCH_HOVER: &str = "The node's config, into the editor";
    pub(crate) const FETCH_TOO_NEW: &str = "The node's config is newer than the app";
    pub(crate) const FETCH_NEEDS_LINK: &str = "Connect first";

    pub(crate) const EMPTY: &str = "No config loaded.";
    pub(crate) const GENERATE_HOVER: &str = "The firmware defaults, unsaved";

    pub(crate) const PUSH_CONFIRM: &str = "Send this config to the node?";
    pub(crate) const PUSH_CONFIRM_MORE: &str = "Replaces the node's whole config.";

    pub(crate) const NO_BACKUPS: &str = "None yet.";

    pub(crate) const BEACON_OFF: &str = "Beacon off.";
    pub(crate) const PING_OFF: &str = "No-fix ping off.";

    pub(crate) const HOP_OVERRUN: &str =
        "Lengthen hop_dwell_ms, widen the bandwidth or send fewer fields.";
    pub(crate) const NEEDS_HOPPING: &str =
        "902-928 MHz needs hopping under 500 kHz: set hop_channels, or bandwidth_khz = 500.";
}

/// The Logging page: the CSV recorder and its graph.
pub(crate) mod logging {
    pub(crate) const START_HOVER: &str = "Appends to the file";
    pub(crate) const STOP_HOVER: &str = "Closes the file";
    pub(crate) const SAVE_HOVER: &str = "File, reference and auto-start, to the settings file";

    pub(crate) const EXPORT_HOVER_PHONE: &str = "To the Downloads folder";
    pub(crate) const EXPORT_HOVER_DESKTOP: &str = "A timestamped copy beside the file";
    pub(crate) const CLEAR_HOVER: &str = "The file is untouched";

    pub(crate) const BAD_COORD: &str = "Use lat, lon";

    pub(crate) const LEGEND_HOVER: &str = "Show or hide";
    pub(crate) const PLOT_EMPTY: &str = "Nothing recorded yet.";
    pub(crate) const PLOT_NO_PAIRS: &str = "No rows with both.";

    /// Rows that have scrolled off the graph but are still in the file.
    pub(crate) fn dropped(rows: usize) -> String {
        format!("{rows} older rows only in the file.")
    }

    /// Where the recorder is up to: running and for how long, paused after a
    /// run, or never started.
    pub(crate) fn state(
        recording: bool,
        started: Option<std::time::SystemTime>,
        written: usize,
    ) -> String {
        if !recording {
            return match written {
                0 => "Not recording".to_string(),
                n => format!("Stopped after {n} rows"),
            };
        }
        match started {
            Some(t) => format!(
                "Recording for {}, {written} rows",
                crate::points::age_text(std::time::SystemTime::now(), t)
            ),
            None => format!("Recording, {written} rows"),
        }
    }
}

/// The map page: the controls bar, the marker popups, and the offline
/// region download.
pub(crate) mod map {
    pub(crate) const CENTER_HOVER: &str = "Center on position (hold for markers)";
    pub(crate) const NORTH_UP: &str = "North up";
    pub(crate) const HEADING_UP: &str = "Heading up";
    pub(crate) const TOPO_MAP: &str = "Topographic map";
    pub(crate) const STANDARD_MAP: &str = "Standard map";
    pub(crate) const SATELLITE_MAP: &str = "Satellite";
    pub(crate) const ZOOM_IN: &str = "Zoom in";
    pub(crate) const ZOOM_OUT: &str = "Zoom out";
    pub(crate) const HIDE_PATHS: &str = "Hide paths";
    pub(crate) const SHOW_PATHS: &str = "Show paths";
    pub(crate) const MENU_OPEN: &str = "Menu";
    pub(crate) const MENU_CLOSE: &str = "Close menu";
    pub(crate) const FOLD_BAR: &str = "Fold the bar away";
    pub(crate) const UNFOLD_BAR: &str = "Show the bar";

    pub(crate) const SELECT_HINT: &str = "Drag a box to download";
    pub(crate) const DOWNLOAD_TITLE: &str = "Download region";
    pub(crate) const TOO_MANY_TILES: &str = "Too many tiles: shrink the box or lower the max zoom.";
    pub(crate) const NO_UPDATE: &str = "No update yet";
}

/// The adjuster.
pub(crate) mod adjust {
    pub(crate) const PICK_HOVER: &str = "Tap to pick. Hold, or right-click, for a list";
    pub(crate) const PICKING: &str = "Tap to pick. Hold for a list.";
    pub(crate) const NOTHING_PICKED: &str = "Nothing picked yet.";
    pub(crate) const UNDER_FINGER: &str = "Under the finger";
    pub(crate) const RESET_HOVER: &str = "Back to default";
    pub(crate) const INHERIT_HOVER: &str = "Take the level above";
    pub(crate) const SAVE_HOVER: &str = "Write the sheet";
    pub(crate) const RELOAD_HOVER: &str = "Drops unsaved changes";
    pub(crate) const DEFAULTS_HOVER: &str = "Only in the app until you save";
    pub(crate) const DONE_HOVER: &str = "Unsaved changes stay until restart";
}
