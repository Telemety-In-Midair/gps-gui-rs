//! The app's prose, one module per page.
//!
//! Only the long form lives here: the paragraphs explaining a group of
//! settings, and the hover texts saying what a control actually does. Short
//! labels stay next to the widget they name, where reading the page tells you
//! what the page says.
//!
//! The split is by length rather than by principle. A button reading "Save" is
//! part of the layout; the three lines under it explaining what saving writes
//! and where are a piece of writing, and having them inline is what turns a
//! page of controls into a page of string literals with controls between them.
//!
//! Copy that takes a value is a function rather than a constant, `format!`
//! needing a literal pattern.

/// The Points page.
pub(crate) mod points {
    pub(crate) const SEARCH_HINT: &str = "search (example 51.47 or central)";
}

/// The desktop manual-position bar.
pub(crate) mod manual {
    pub(crate) const BAD_COORD: &str = "Enter latitude and longitude, example 51.4779, -0.0015";
}

/// The Status page.
pub(crate) mod status {
    pub(crate) const WAITING_FIX: &str = "Waiting for a GPS fix...";

    pub(crate) const WARMING: &str =
        "Warming up: the board has only just connected, and a GPS that was cold when the board \
         booted is still working on its first fix.";

    pub(crate) const NO_TELEMETRY: &str = "No board telemetry yet.\n\
         Waiting for the Wio-S3 board (an esp32c3 beacon does not report it).";
}

/// The Settings page: the app's own TOML settings.
pub(crate) mod settings {
    pub(crate) const INTRO: &str = "What the app draws and records, kept in its own TOML file.";

    pub(crate) const SAVE_HOVER: &str =
        "Write these settings to the file above, generating it if it is not there";
    pub(crate) const RESET_HOVER: &str = "Only in the app until you save";

    pub(crate) const TEXT_SCALE: &str =
        "Scales the text on every page. The gaps and the input widths are measured in text \
         heights, so they grow with it; the map's icons and overlays keep their own sizes.";
    pub(crate) const TEXT_SCALE_RESET_HOVER: &str = "Back to the default text size";

    pub(crate) const LOOK: &str =
        "Every size and spacing on the pages, as fractions of the screen and the text, in a \
         sheet of its own beside the config. The adjuster edits it live: pick a thing on any \
         page and move the measures behind it.";
    pub(crate) const LOOK_SAVE_HOVER: &str =
        "Write the changed measures into the sheet above, generating it if it is not there";
    pub(crate) const LOOK_RESET_HOVER: &str =
        "Every measure back to what the app ships with. Only in the app until you save";
    pub(crate) const ADJUST_HOVER: &str =
        "Float the adjuster over the pages: pick a thing, drag its measures, save";
    pub(crate) const ADJUST_OPEN: &str = "Already open. Its Done button closes it";

    pub(crate) const PAGE_COLORS: &str =
        "The few places off the map that carry meaning by color; the rest follows the theme.";
    pub(crate) const THEME_COLORS: &str =
        "Unticked follows the light/dark theme, which is what keeps these three readable \
         against each other. Setting one is taking that on yourself.";

    pub(crate) const CENTRAL_PATH_HOVER: &str =
        "This device's own track. Hiding a path never stops it being recorded";
    pub(crate) const BOARD_ON_MAP_HOVER: &str =
        "Its marker, heartbeat, path and the distance line to it. The link, the recording and \
         the Status page carry on either way; this is for a board held next to the phone, \
         where its marker only sits on top of yours";
    pub(crate) const REMOTE_PATHS_HOVER: &str =
        "The LoRa nodes relayed by the connected board, one color each";
    pub(crate) const PATHS_NOTE: &str =
        "The map's path button hides them all at once without changing these; the line to the \
         beacon and its distance stay drawn either way.";

    pub(crate) const COMPASS: &str =
        "Heading-up always runs the compass at full rate. These are for the other modes, where \
         it only points the arrow on your marker.";
    pub(crate) const ARROW_HZ_HOVER: &str =
        "Lower is cheaper: the sensor is fused from the accelerometer, gyroscope and \
         magnetometer, so it keeps all three awake";

    pub(crate) const STATUS_BAR: &str =
        "A strip along the bottom of the map: the last few receptions as a bar graph, one bar \
         per node in that node's map color, and beside it one node's signal, age, satellites \
         and speed.";
    pub(crate) const STATUS_BAR_SHOW_HOVER: &str =
        "Covers a strip of the map, so it is worth having only while nodes are being heard";
    pub(crate) const STATUS_CYCLE_HOVER: &str =
        "Only used once a second node has been heard; one node keeps the read-out to itself";

    pub(crate) const DISCARD_HOVER: &str =
        "Drops every track: yours, the beacon's and the nodes'. Not undoable";

    pub(crate) const DOWNLOAD_HOVER: &str = "Pick a box on the map to cache for offline use";
    pub(crate) const DOWNLOAD_BUSY: &str = "A download is already in progress.";
}

/// The map's bottom status bar.
pub(crate) mod statusbar {
    pub(crate) const NO_NODES: &str = "No nodes heard";
}

/// The Beacon page: the BLE link, and the board's own power settings.
pub(crate) mod beacon {
    use crate::app::secs_text;

    pub(crate) const INTRO: &str = "The BLE link to a GPS beacon. One board at a time.";

    // Device picker.
    pub(crate) const SCAN_START_HOVER: &str =
        "Look for boards nearby. This drops the current link, since only one board is connected \
         at a time.";
    pub(crate) const SCAN_STOP_HOVER: &str = "Stop looking and leave the list as it stands";
    pub(crate) const NO_BOARDS_SCANNING: &str =
        "No boards yet. A sleeping board only answers during its advertising window.";
    pub(crate) const NO_BOARDS_IDLE: &str =
        "No boards known yet. Scan to find one, then name it so you can tell it apart later.";
    pub(crate) const ANY_BOARD_HOVER: &str =
        "Connect to the first board that answers, whichever it is";
    pub(crate) const NAMES_NOTE: &str =
        "Names typed here are the app's own and are saved with the rest of its settings; \
         clearing one forgets the board. A name stored on the board itself (under Board name, \
         once connected) travels with the board and is what every page calls it, over the \
         name typed here. A board without one shows its address name in grey.";

    // Link controls.
    pub(crate) const RECONNECT_HOVER: &str =
        "Drop this link and start over from a scan, on the selected board";
    pub(crate) const CONNECT_HOVER: &str =
        "Go straight to the selected board, or scan when it is set to any";
    pub(crate) const CONNECT_SLEEPING_HOVER: &str =
        "Scan without stopping. A sleeping board advertises for only a window per wake, a plain \
         connect can miss.";
    pub(crate) const DISCONNECT_HOVER: &str =
        "Drop the link now, forget what the board reported, and stop trying so it can sleep";
    pub(crate) const SLEEP_DISABLED: &str =
        "Sleep is disabled on the board, so it stays awake after you disconnect too.";
    pub(crate) const ONLY_ON_WINDOW: &str =
        "During sleep intervals the board is only reachable on its advertising window.";

    /// What disconnecting will start, for a board with sleep switched on.
    pub(crate) fn will_sleep(interval_s: u32, window_s: u32) -> String {
        format!(
            "On disconnect it will sleep, waking every {} to advertise for {}.",
            secs_text(interval_s),
            secs_text(window_s)
        )
    }

    // Connection settings.
    pub(crate) const AUTO_CONNECT_HOVER: &str =
        "Only read when the app launches; use the buttons above now";
    pub(crate) const SAVE_HOVER: &str =
        "Write the board names and these settings to the app's config file, set on the Settings \
         page";
    pub(crate) const AWAITING_ACK: &str = "waiting for device ack...";

    // Board name.
    pub(crate) const BOARD_NAME_INTRO: &str =
        "What the board calls itself. Stored in its flash, so it survives a power cycle and \
         travels with the board to any phone; while it is set, it is the board's name on \
         every page here, over the name typed in the list above.";
    pub(crate) const BOARD_NO_NAME: &str =
        "Board: no name reported yet. A board on firmware older than names never reports one.";
    pub(crate) const NAME_CLEAR_HOVER: &str =
        "Forget the stored name; the board goes back to advertising by its address";

    /// What a name may be, for the box it is typed in.
    pub(crate) fn board_name_hover(max: usize) -> String {
        format!(
            "Up to {max} letters, digits, - or _. The board advertises it from its next \
             window on and reports it at once."
        )
    }

    // Board power and sleep.
    pub(crate) const BOARD_INTRO: &str =
        "Wio-S3 settings. The board keeps these in flash, so they outlast a power cycle.";
    pub(crate) const BOARD_NEED_LINK: &str = "Connect to the board to see and change these.";
    pub(crate) const BOARD_TOO_NEW: &str = "This board's firmware is newer than the app.";
    pub(crate) const BOARD_TOO_NEW_MORE: &str =
        "Its settings use a layout this build cannot decode. Update the app to change them.";
    pub(crate) const BOARD_READING: &str = "Reading the board's settings...";

    pub(crate) const MODE_INTRO: &str =
        "What the board is for right now. Each setting below belongs to one mode, and only \
         that mode reads it.";
    pub(crate) const MODE_STORED_HOVER: &str =
        "Everything down. The board acks, then deep-sleeps on its wake cadence and \
         disconnects - connect during one of its wake checks to bring it back";
    pub(crate) const MODE_IDLE_HOVER: &str =
        "Reachable but not tracking: the GPS stays in backup and the radio stays down, so \
         reading the board's settings costs no acquisition. Ends by itself";
    pub(crate) const MODE_TRACKING_HOVER: &str =
        "GPS acquiring, beacons going out, card logging. The only mode that survives a \
         power cycle, so a board put down tracking comes back tracking";

    /// What each mode is doing, for the line under the buttons. This is the
    /// board's own reported mode, not the button that was last pressed.
    pub(crate) fn mode_state(mode: midair_proto::ble::Mode) -> &'static str {
        match mode {
            midair_proto::ble::Mode::Stored => {
                "Board: stored. Awake for this wake check only - it goes back down when the \
                 window ends."
            }
            midair_proto::ble::Mode::Idle => "Board: idle. Reachable, GPS in backup, radio asleep.",
            midair_proto::ble::Mode::Tracking => "Board: tracking. GPS, beacon and logging up.",
        }
    }

    /// What the idle timeout does, and the range it is clamped to.
    ///
    /// Said in terms of why it exists rather than what it sets: idle is the
    /// expensive state, and this is the only thing that bounds it.
    pub(crate) fn idle_timeout(min_s: u32, max_s: u32) -> String {
        format!(
            "How long the board stays idle before it stores itself. Idle costs nearly as much \
             as tracking - it is BLE that dominates, not the GPS - so this is meant to be \
             minutes. Every cold boot lands in idle, so it is also the window you get to catch \
             a board that has just come back from a flat cell. Clamped to {} - {}.",
            secs_text(min_s),
            secs_text(max_s)
        )
    }

    pub(crate) const RADIO_STANDBY_HOVER: &str =
        "Parks the LoRa radio: it stops listening, so nothing is heard or relayed";
    pub(crate) const GPS_SLEEP_HOVER: &str = "The next fix after waking is a cold one";
    pub(crate) const SLEEP_DISABLE_HOVER: &str = "Stop the board sleeping at all";

    /// What the wake check does, and the range the board will clamp it to.
    ///
    /// Which modes read it is half the meaning now: a tracker that
    /// deep-sleeps is not tracking, so this is the stored board's cadence
    /// and the sleep an idle board takes when its timeout runs out.
    pub(crate) fn wake_check(min_s: u32, max_s: u32) -> String {
        format!(
            "How often a stored board wakes to ask whether anyone wants it, and the sleep an \
             idle board takes when its timeout runs out. A tracking board ignores it. 0 means \
             the board never stores itself on its own - though being told to store itself \
             still works. Clamped to {} - {}.",
            secs_text(min_s),
            secs_text(max_s)
        )
    }

    /// What the advertising window does, and the range it is clamped to.
    pub(crate) fn adv_window(min_s: u32, max_s: u32) -> String {
        format!(
            "How long each wake check advertises before going back to sleep - the whole of \
             the time a stored board is reachable. While tracking it is the on-half of the \
             BLE off period instead. Clamped to {} - {}.",
            secs_text(min_s),
            secs_text(max_s)
        )
    }

    pub(crate) const BLE_OFF_DISABLE_HOVER: &str =
        "Keep BLE up continuously, so the board is always reachable";

    /// What the BLE off period does, and the range it is clamped to.
    ///
    /// Said in terms of what keeps running, because the name reads like the
    /// board going away: only the BLE controller stops. This is the largest
    /// power saving the board has - roughly 70 mA of the 126 it draws - and
    /// it is the one setting where the cost is purely reachability rather
    /// than any loss of function.
    pub(crate) fn ble_off(min_s: u32, max_s: u32) -> String {
        format!(
            "How long BLE is powered down between advertising windows while tracking. Saves \
             about 70 mA of the board's 126 while down. It keeps beaconing, tracking and \
             logging throughout - it just cannot be connected to until the next window. Only \
             tracking reads this: idle exists to be reachable, and a stored board has no \
             controller to take down. Clamped to {} - {}.",
            secs_text(min_s),
            secs_text(max_s)
        )
    }

    pub(crate) const SLEEP_NOW_HOVER: &str =
        "Send the board to sleep right now. It disconnects and is unreachable until it wakes";
    pub(crate) const SLEEP_NOW_MODE_NOTE: &str =
        "Unlike Stored above, this is a nap: the board comes back to the mode it is in now.";
    pub(crate) const SLEEP_NOW_BLANK_HOVER: &str =
        "Leave blank to use the wake-check interval above";

    /// What "Sleep now" does, said in terms of what it is *not*: it is the
    /// control most likely to be read as a settings change, because every
    /// other control on this page is one.
    pub(crate) fn sleep_now(min_s: u32, max_s: u32) -> String {
        format!(
            "A one-off, not a setting. Nothing is stored and the wake check above is not \
             touched - the board comes back to exactly what it is doing now. Clamped to {} - {}.",
            secs_text(min_s),
            secs_text(max_s)
        )
    }

    /// Two firmware behaviors that otherwise read as the board ignoring the
    /// window, and the shorter the window the more they stand out: the budget
    /// for a wake is taken when that wake starts, and a disconnect replaces
    /// what is left of it with a fixed linger so the app can come straight
    /// back.
    pub(crate) fn adv_window_note(linger_s: u32) -> String {
        format!(
            "A new window takes effect at the next wake, not the current one, and the stretch \
             right after you disconnect is always {} however short the window is.",
            secs_text(linger_s)
        )
    }
}

/// The Radio page: the board's own RADIO.TOML.
pub(crate) mod radio {
    pub(crate) const INTRO: &str = "RADIO.TOML for the Wio-S3 board.";

    pub(crate) const SEND_HOVER: &str =
        "Send this config to the connected board. It applies immediately and is written to the \
         SD card, which is the only place it survives a reboot.";
    pub(crate) const SEND_NEEDS_CONFIG: &str = "Load or generate a config first";
    pub(crate) const SEND_NEEDS_LINK: &str = "Connect to the board first (Beacon page)";
    pub(crate) const SEND_WAITING: &str = "Waiting for the board to answer";

    pub(crate) const FETCH_HOVER: &str =
        "Fill the editor with the settings the connected board is currently running, ready to \
         edit, save or send back.";
    pub(crate) const FETCH_TOO_NEW: &str =
        "The board's config format is newer than this app can read";
    pub(crate) const FETCH_NEEDS_LINK: &str =
        "Connect on the Beacon page; the board's settings load once its radio has come up";

    pub(crate) const EMPTY: &str =
        "Load a RADIO.TOML to view and edit the radio, mesh, beacon and GPS settings.";
    pub(crate) const GENERATE_HOVER: &str =
        "Fill the editor with the firmware defaults, ready to edit and save to the file above";

    pub(crate) const PUSH_CONFIRM: &str = "Send this config to the board?";
    pub(crate) const PUSH_CONFIRM_MORE: &str =
        "It replaces the board's whole config, takes effect immediately and is written to the \
         board's SD card. Without a card it is lost on the next reboot.";

    pub(crate) const NO_BACKUPS: &str = "No backups yet. Saving keeps the previous version here.";

    /// The beacon is off, so there is no periodic airtime to report.
    pub(crate) const BEACON_OFF: &str =
        "Beacon disabled (interval 0): the node is silent, pings included.";
    pub(crate) const PING_OFF: &str =
        "No-fix ping disabled (ping_interval_s 0): a node without a fix is silent.";

    pub(crate) const HOP_OVERRUN: &str =
        "A frame longer than the window still goes out, but it holds one channel past the \
         hop. Lengthen hop_dwell_ms, widen the bandwidth or send fewer fields.";
    pub(crate) const HOP_INTERVAL: &str =
        "Hopping, a node transmits at most once per channel visit, so the beacon interval \
         can go down to one slot.";
    pub(crate) const HOP_LIMIT: &str =
        "Limit: 400 ms on one channel per visit (902-928 MHz hopping rule). The interval is \
         not limited.";
    pub(crate) const NEEDS_HOPPING: &str =
        "Narrower than 500 kHz on a single channel is not allowed in 902-928 MHz: set \
         hop_channels, or use bandwidth_khz = 500.";
    pub(crate) const WIDE_SINGLE: &str =
        "500 kHz on one channel counts as digital modulation in 902-928 MHz: no dwell or \
         duty limit, but the band's only single-channel option.";
}

/// The Logging page: the CSV recorder and its graph.
pub(crate) mod logging {
    pub(crate) const INTRO: &str =
        "Every report from every source, written to a CSV as it arrives: where each one was, \
         how strongly it was heard, and how far off it is. One row per report, so a node's \
         distance and its signal are always the same instant.";

    pub(crate) const START_HOVER: &str = "Append to this file, creating it if it is not there";
    pub(crate) const STOP_HOVER: &str = "Close the file; what is recorded stays on the graph";
    pub(crate) const APPEND_NOTE: &str =
        "Starting appends to the file, so stopping and starting again continues the same log \
         rather than replacing it.";
    pub(crate) const SAVE_HOVER: &str =
        "Write the log file, reference and auto-start to the app config";

    pub(crate) const EXPORT_HOVER_PHONE: &str = "Copy the CSV into the phone's Downloads folder";
    pub(crate) const EXPORT_HOVER_DESKTOP: &str =
        "Write a timestamped copy of the CSV beside the log file";
    pub(crate) const CLEAR_HOVER: &str = "Empty the graph; the file on disk is untouched";

    pub(crate) const REFERENCE: &str =
        "A fixed coordinate every logged position is also measured against, so a run can be \
         read against a surveyed point rather than against a control device that is moving \
         too. Leave it empty to log only the distance to yourself.";
    pub(crate) const BAD_COORD: &str = "Enter a coordinate as \"lat, lon\".";

    pub(crate) const LEGEND_HOVER: &str = "Show or hide this source";
    pub(crate) const PLOT_EMPTY: &str = "Nothing recorded yet.";
    pub(crate) const PLOT_NO_PAIRS: &str = "No rows carry both of these.";

    /// Rows that have scrolled off the graph but are still in the file.
    pub(crate) fn dropped(rows: usize) -> String {
        format!("The oldest {rows} rows have scrolled off the graph; they are still in the file.")
    }

    /// Where the recorder is up to: running and for how long, paused after a
    /// run, or never started.
    ///
    /// "Stopped" and "Not recording" are deliberately different: a file that
    /// has rows in it is one a later Start appends to, and saying so is the
    /// only warning before it does.
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
    pub(crate) const ZOOM_IN: &str = "Zoom in";
    pub(crate) const ZOOM_OUT: &str = "Zoom out";
    pub(crate) const HIDE_PATHS: &str = "Hide paths";
    pub(crate) const SHOW_PATHS: &str = "Show paths";
    pub(crate) const MENU_OPEN: &str = "Pages";
    pub(crate) const MENU_CLOSE: &str = "Close menu";

    pub(crate) const SELECT_HINT: &str = "Drag a box over the region to download";
    pub(crate) const DOWNLOAD_TITLE: &str = "Download region for offline use";
    pub(crate) const TOO_MANY_TILES: &str = "Too many tiles: shrink the box or lower the max zoom.";
    pub(crate) const NO_UPDATE: &str = "No update yet";
}

/// The adjuster.
pub(crate) mod adjust {
    pub(crate) const PICK_HOVER: &str =
        "Arm the picker: the next tap picks the smallest thing under it. Hold, or right-click, \
         for everything under the finger, the page included";
    pub(crate) const PICKING: &str = "Tap a thing to pick it. Hold for a list.";
    pub(crate) const NOTHING_PICKED: &str =
        "Nothing picked yet. Pick something to see the measures that shape it.";
    pub(crate) const UNDER_FINGER: &str = "Under the finger, smallest first";
    pub(crate) const RESET_HOVER: &str = "This measure back to its default";
    pub(crate) const SAVE_HOVER: &str =
        "Write the changed measures into the look sheet, generating it if it is not there";
    pub(crate) const RELOAD_HOVER: &str = "Read the look sheet back, dropping unsaved changes";
    pub(crate) const DEFAULTS_HOVER: &str =
        "Every measure back to what the app ships with. Only in the app until you save";
    pub(crate) const DONE_HOVER: &str =
        "Close the adjuster. Unsaved changes stay until the app is restarted";
}
