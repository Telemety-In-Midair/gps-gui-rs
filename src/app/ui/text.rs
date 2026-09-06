//! The app's prose, one module per page.
//!
//! Only the long form lives here: the lines explaining a group of settings,
//! and the hover texts saying what a control actually does. Short labels
//! stay next to the widget they name, where reading the page tells you what
//! the page says.
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
    pub(crate) const BAD_COORD: &str = "Enter latitude and longitude, example `51.4779, -0.0015`";
}

/// The Status page.
pub(crate) mod status {
    use crate::app::LocationSource;

    pub(crate) const WAITING_FIX: &str = "No position yet.";
    pub(crate) const NO_SOURCE: &str = "No position source is on. Turn one on in Settings.";
    pub(crate) const WARMING: &str = "Warming up: just connected.";
    pub(crate) const NO_TELEMETRY: &str = "No telemetry yet.";

    /// Where the position came from, for the line under it.
    pub(crate) fn source(source: LocationSource) -> &'static str {
        match source {
            LocationSource::Phone => "From this phone's receiver.",
            LocationSource::Node => "From the connected node's receiver.",
            LocationSource::Manual => "Typed in.",
        }
    }
}

/// The Settings page: the app's own settings.
pub(crate) mod settings {
    pub(crate) const INTRO: &str = "The app's settings, kept in a TOML file.";

    pub(crate) const SAVE_HOVER: &str = "Write these settings to the file above";
    pub(crate) const RESET_HOVER: &str = "Only in the app until you save";

    pub(crate) const TEXT_SCALE: &str = "Scales the text on every page.";
    pub(crate) const TEXT_SCALE_RESET_HOVER: &str = "Back to the default size";

    pub(crate) const LOOK: &str = "Every size and spacing on the pages. The adjuster edits it live.";
    pub(crate) const LOOK_SAVE_HOVER: &str = "Write the sheet, creating it if needed";
    pub(crate) const LOOK_RESET_HOVER: &str = "Every measure back to default. Only in the app until you save";
    pub(crate) const ADJUST_HOVER: &str = "Pick a thing on any page and drag its measures";
    pub(crate) const ADJUST_OPEN: &str = "Already open. Its Done button closes it";

    pub(crate) const THEME: &str = "Solarized, either way up. System follows the device.";
    pub(crate) const PAGE_COLORS: &str = "Status colors.";
    pub(crate) const THEME_COLORS: &str = "Unticked follows the theme.";

    pub(crate) const PHONE: &str = "This device.";
    pub(crate) const PHONE_LOCATION_HOVER: &str =
        "Run the phone's own receiver. Off, it is never powered";
    pub(crate) const NODE_LOCATION_HOVER: &str =
        "While the node has a fix it is where you are, and its marker folds into yours";

    pub(crate) const CENTRAL_PATH_HOVER: &str = "Your own track. Hiding a path never stops recording";
    pub(crate) const BOARD_ON_MAP_HOVER: &str = "Its marker, pulse, path and the distance line";
    pub(crate) const REMOTE_PATHS_HOVER: &str = "The LoRa nodes the connected node relays, one color each";
    pub(crate) const PULSE_HOVER: &str = "A node's marker pulses while it was heard within this";

    pub(crate) const MAP_TILES: &str = "Who serves the map.";
    pub(crate) const ARCGIS_LAYERS: &str =
        "Streets, outdoor and satellite; the map's layer button cycles them.";
    pub(crate) const ARCGIS_KEY_HOVER: &str = "Empty uses the key the app ships with";

    pub(crate) const MAP_BARS: &str = "The bars over the map.";
    pub(crate) const BAR_OPACITY_HOVER: &str = "0 shows the map through both bars";
    pub(crate) const KEY_HOVER: &str = "One row per marker color, under the top bar";

    pub(crate) const COMPASS: &str = "Phone compass for the marker arrow.";
    pub(crate) const ARROW_HZ_HOVER: &str = "Lower is cheaper: the sensor keeps three others awake";

    pub(crate) const STATUS_BAR: &str = "A signal read-out along the bottom of the map.";
    pub(crate) const STATUS_BAR_SHOW_HOVER: &str = "Covers a strip of the map";
    pub(crate) const STATUS_CYCLE_HOVER: &str = "Only once a second node is heard";
    pub(crate) const RSSI_RANGE: &str =
        "A bar is full at the top value and empty at the bottom. Linear in dBm, so each equal \
         step is the same ratio of power.";

    pub(crate) const DISCARD_HOVER: &str = "Drops every track. Not undoable";

    pub(crate) const DOWNLOAD_HOVER: &str = "Pick a box on the map to cache for offline use";
    pub(crate) const DOWNLOAD_BUSY: &str = "A download is already in progress.";
}

/// The map's bottom status bar.
pub(crate) mod statusbar {
    pub(crate) const NO_NODES: &str = "No nodes heard";
}

/// The Bluetooth page: the BLE link, and the node's own settings.
pub(crate) mod bluetooth {
    use crate::app::secs_text;

    pub(crate) const INTRO: &str = "The BLE link to a node.";

    // Node picker.
    pub(crate) const SCAN_START_HOVER: &str = "Look for nodes nearby. Drops the current link";
    pub(crate) const SCAN_STOP_HOVER: &str = "Stop looking";
    pub(crate) const SCANNING: &str = "Scanning";
    pub(crate) const NO_BOARDS_SCANNING: &str =
        "No nodes yet. A sleeping node only answers in its advertising window.";
    pub(crate) const NO_BOARDS_IDLE: &str = "No nodes known yet.";
    pub(crate) const ANY_BOARD_HOVER: &str = "Connect to the first node that answers";
    pub(crate) const NAMES_NOTE: &str =
        "A node's own name overwrites the name here. Clearing one forgets the node.";

    // Link controls.
    pub(crate) const RECONNECT_HOVER: &str = "Drop this link and start over";
    pub(crate) const CONNECT_HOVER: &str = "Connect to the selected node";
    pub(crate) const CONNECT_SLEEPING_HOVER: &str =
        "Scan without stopping, for a node that only advertises in windows";
    pub(crate) const DISCONNECT_HOVER: &str = "Drop the link and stop trying, so the node can sleep";
    pub(crate) const SLEEP_DISABLED: &str = "Sleep is off on the node: it stays awake after you disconnect.";
    pub(crate) const ONLY_ON_WINDOW: &str = "A sleeping node is only reachable in its advertising window.";

    /// What disconnecting will start, for a node with sleep switched on.
    pub(crate) fn will_sleep(interval_s: u32, window_s: u32) -> String {
        format!(
            "On disconnect it sleeps, waking every {} to advertise for {}.",
            secs_text(interval_s),
            secs_text(window_s)
        )
    }

    // Connection settings.
    pub(crate) const AUTO_CONNECT_HOVER: &str = "At launch only";
    pub(crate) const SAVE_HOVER: &str = "Write the node list and these settings to the settings file";
    pub(crate) const AWAITING_ACK: &str = "Waiting for the node";

    // Node name.
    pub(crate) const BOARD_NAME_INTRO: &str = "Stored on the node, in flash.";
    pub(crate) const BOARD_NO_NAME: &str = "No name reported yet.";
    pub(crate) const NAME_CLEAR_HOVER: &str = "Back to the address name";

    /// What a name may be, for the box it is typed in.
    pub(crate) fn board_name_hover(max: usize) -> String {
        format!("Up to {max} letters, digits, - or _")
    }

    // Node power and sleep.
    pub(crate) const BOARD_INTRO: &str = "Kept on the node, in flash.";
    pub(crate) const BOARD_NEED_LINK: &str = "Connect to see these.";
    pub(crate) const BOARD_TOO_NEW: &str = "The node's firmware is newer than the app.";
    pub(crate) const BOARD_TOO_NEW_MORE: &str = "Update the app to change its settings.";
    pub(crate) const BOARD_READING: &str = "Reading the node's settings";

    pub(crate) const MODE_INTRO: &str = "Each setting below belongs to one mode.";
    pub(crate) const MODE_STORED_HOVER: &str =
        "Everything down; wakes on the cadence to advertise. Disconnects now";
    pub(crate) const MODE_IDLE_HOVER: &str = "Reachable, GPS in backup, radio down";
    pub(crate) const MODE_TRACKING_HOVER: &str = "GPS, beacon and logging up. Survives a power cycle";
    pub(crate) const MODE_LISTENING_HOVER: &str =
        "GPS and receiver up, nothing sent, BLE always up. For the node beside the phone";

    /// What each mode is doing, for the line under the buttons. This is the
    /// node's own reported mode, not the button that was last pressed.
    pub(crate) fn mode_state(mode: midair_proto::ble::Mode) -> &'static str {
        match mode {
            midair_proto::ble::Mode::Stored => "Node: stored. Awake for this wake check only.",
            midair_proto::ble::Mode::Idle => "Node: idle. Reachable, GPS in backup, radio asleep.",
            midair_proto::ble::Mode::Tracking => "Node: tracking. GPS, beacon and logging up.",
            midair_proto::ble::Mode::Listening => {
                "Node: listening. GPS and receiver up, nothing sent."
            }
        }
    }

    /// What the idle timeout does, and the range it is clamped to.
    pub(crate) fn idle_timeout(min_s: u32, max_s: u32) -> String {
        format!(
            "How long idle lasts before the node stores itself. Off keeps it idle. {} - {}, \
             and needs a wake check set.",
            secs_text(min_s),
            secs_text(max_s)
        )
    }
    pub(crate) const IDLE_DISABLE_HOVER: &str = "Stay idle until told otherwise";

    pub(crate) const RADIO_STANDBY_HOVER: &str = "Parks the LoRa radio";
    pub(crate) const GPS_SLEEP_HOVER: &str = "GPS in backup; the next fix is a cold one";
    pub(crate) const SLEEP_DISABLE_HOVER: &str = "Never sleep";

    /// What the wake check does, and the range the node will clamp it to.
    pub(crate) fn wake_check(min_s: u32, max_s: u32) -> String {
        format!(
            "How often a stored node wakes. 0 never. {} - {}.",
            secs_text(min_s),
            secs_text(max_s)
        )
    }

    /// What the advertising window does, and the range it is clamped to.
    pub(crate) fn adv_window(min_s: u32, max_s: u32) -> String {
        format!(
            "How long each wake check advertises. {} - {}.",
            secs_text(min_s),
            secs_text(max_s)
        )
    }

    /// What the BLE on period does, and the range it is clamped to.
    pub(crate) fn ble_on(min_s: u32, max_s: u32) -> String {
        format!(
            "How long BLE stays up between off periods while tracking. {} - {}.",
            secs_text(min_s),
            secs_text(max_s)
        )
    }

    pub(crate) const BLE_OFF_DISABLE_HOVER: &str = "Keep BLE up";

    /// What the BLE off period does, and the range it is clamped to.
    pub(crate) fn ble_off(min_s: u32, max_s: u32) -> String {
        format!(
            "How long BLE is down between on periods while tracking. Still beaconing. {} - {}.",
            secs_text(min_s),
            secs_text(max_s)
        )
    }

    pub(crate) const SLEEP_NOW_HOVER: &str = "Sleep now; the link drops";
    pub(crate) const SLEEP_NOW_MODE_NOTE: &str = "A nap: the node comes back in its current mode.";
    pub(crate) const SLEEP_NOW_BLANK_HOVER: &str = "Blank uses the wake check";

    /// What "Sleep now" does, and the range it is clamped to.
    pub(crate) fn sleep_now(min_s: u32, max_s: u32) -> String {
        format!("A one-off. {} - {}.", secs_text(min_s), secs_text(max_s))
    }

    /// Two firmware behaviors that otherwise read as the node ignoring the
    /// window.
    pub(crate) fn adv_window_note(linger_s: u32) -> String {
        format!(
            "Takes effect at the next wake; after a disconnect it advertises {} regardless.",
            secs_text(linger_s)
        )
    }
}

/// The Radio page: the node's own RADIO.TOML.
pub(crate) mod radio {
    pub(crate) const INTRO: &str = "RADIO.TOML for the node.";

    pub(crate) const SEND_HOVER: &str =
        "Send to the connected node. Applied at once and written to its card";
    pub(crate) const SEND_NEEDS_CONFIG: &str = "Load or generate a config first";
    pub(crate) const SEND_NEEDS_LINK: &str = "Connect first";
    pub(crate) const SEND_WAITING: &str = "Waiting for the node";

    pub(crate) const FETCH_HOVER: &str = "Fill the editor from the connected node";
    pub(crate) const FETCH_TOO_NEW: &str = "The node's config format is newer than this app";
    pub(crate) const FETCH_NEEDS_LINK: &str = "Connect first; the node reports its config once its radio is up";

    pub(crate) const EMPTY: &str = "Load a RADIO.TOML, or generate the default.";
    pub(crate) const GENERATE_HOVER: &str = "The firmware defaults, in the editor. Save writes the file";

    pub(crate) const PUSH_CONFIRM: &str = "Send this config to the node?";
    pub(crate) const PUSH_CONFIRM_MORE: &str =
        "Replaces the node's whole config at once. Without a card it is lost on reboot.";

    pub(crate) const NO_BACKUPS: &str = "No backups yet. Saving keeps the previous version here.";

    pub(crate) const BEACON_OFF: &str = "Beacon off (interval 0): the node is silent.";
    pub(crate) const PING_OFF: &str = "No-fix ping off (ping_interval_s 0).";

    pub(crate) const HOP_OVERRUN: &str =
        "A frame longer than the window holds one channel past the hop. Lengthen hop_dwell_ms, \
         widen the bandwidth or send fewer fields.";
    pub(crate) const HOP_INTERVAL: &str =
        "Hopping, a node transmits at most once per channel visit.";
    pub(crate) const HOP_LIMIT: &str = "Limit: 400 ms per channel visit (902-928 MHz hopping rule).";
    pub(crate) const NEEDS_HOPPING: &str =
        "Under 500 kHz on one channel is not allowed in 902-928 MHz: set hop_channels, or \
         bandwidth_khz = 500.";
    pub(crate) const WIDE_SINGLE: &str =
        "500 kHz on one channel is digital modulation in 902-928 MHz: no dwell or duty limit.";
}

/// The Logging page: the CSV recorder and its graph.
pub(crate) mod logging {
    pub(crate) const INTRO: &str =
        "Every report, one CSV row each: where it was, how strongly it was heard, how far off.";

    pub(crate) const START_HOVER: &str = "Append to this file, creating it if needed";
    pub(crate) const STOP_HOVER: &str = "Close the file; the graph keeps what it has";
    pub(crate) const APPEND_NOTE: &str = "Starting appends, so a stop is a pause.";
    pub(crate) const SAVE_HOVER: &str = "Write the log file, reference and auto-start to the settings";

    pub(crate) const EXPORT_HOVER_PHONE: &str = "Copy the CSV into the phone's Downloads folder";
    pub(crate) const EXPORT_HOVER_DESKTOP: &str = "Write a timestamped copy beside the log file";
    pub(crate) const CLEAR_HOVER: &str = "Empty the graph; the file is untouched";

    pub(crate) const REFERENCE: &str =
        "A fixed coordinate every logged position is also measured against.";
    pub(crate) const BAD_COORD: &str = "Enter a coordinate as \"lat, lon\".";

    pub(crate) const LEGEND_HOVER: &str = "Show or hide this source";
    pub(crate) const PLOT_EMPTY: &str = "Nothing recorded yet.";
    pub(crate) const PLOT_NO_PAIRS: &str = "No rows carry both of these.";

    /// Rows that have scrolled off the graph but are still in the file.
    pub(crate) fn dropped(rows: usize) -> String {
        format!("The oldest {rows} rows are off the graph; they are still in the file.")
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
                "Recording for {} - {written} rows written",
                crate::points::age_text(std::time::SystemTime::now(), t)
            ),
            None => format!("Recording - {written} rows written"),
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
    pub(crate) const MENU_OPEN: &str = "Pages";
    pub(crate) const MENU_CLOSE: &str = "Close menu";
    pub(crate) const FOLD_BAR: &str = "Fold the bar away";
    pub(crate) const UNFOLD_BAR: &str = "Show the bar";

    pub(crate) const SELECT_HINT: &str = "Drag a box over the region to download";
    pub(crate) const DOWNLOAD_TITLE: &str = "Download region for offline use";
    pub(crate) const TOO_MANY_TILES: &str = "Too many tiles: shrink the box or lower the max zoom.";
    pub(crate) const NO_UPDATE: &str = "No update yet";
}

/// The adjuster.
pub(crate) mod adjust {
    pub(crate) const PICK_HOVER: &str =
        "The next tap picks the smallest thing under it. Hold, or right-click, for everything";
    pub(crate) const PICKING: &str = "Tap a thing to pick it. Hold for a list.";
    pub(crate) const NOTHING_PICKED: &str = "Nothing picked yet.";
    pub(crate) const UNDER_FINGER: &str = "Under the finger, smallest first";
    pub(crate) const RESET_HOVER: &str = "Back to its default";
    pub(crate) const INHERIT_HOVER: &str = "Drop this level's own measure and take the one above";
    pub(crate) const SAVE_HOVER: &str = "Save the changed measures";
    pub(crate) const RELOAD_HOVER: &str = "Read the sheet back, dropping unsaved changes";
    pub(crate) const DEFAULTS_HOVER: &str = "Every measure back to default. Only in the app until you save";
    pub(crate) const DONE_HOVER: &str = "Close. Unsaved changes stay until restart";
}
