use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use egui::Pos2;
use gps_proto::packet::{self, Ack, PositionPacket};
use midair_proto::ble;
use walkers::{
    lat_lon, sources::OpenStreetMap, HeaderValue, HttpOptions, HttpTiles, MapMemory, Position,
    Projector,
};

use crate::ble::{
    BleCommand, BleEvent, BleHandle, BleRequest, ConfigWrite, Epoch, NodePing, RadioConfig,
    Settings, Telemetry,
};
use crate::compass::{self, CompassHandle};
use crate::config::{normalize_mac, AppConfig};
use crate::export::Saver;
use crate::gps::GpsFix;
use crate::logging::{self, LogAxis, LogRow, LogSource, LogStat, Logger};
use crate::offline::{self, DownloadProgress};
use crate::points::{PointSource, TrackPoint};
use crate::radio::{self, EditVal, RadioDoc};
use crate::tiles::{MapLayer, OpenTopoMap};

/// The view layer (page rendering + shared egui scaffolding). Kept in a
/// submodule so this file holds only state and the core update logic; the
/// `impl MyApp` blocks there render each page.
mod ui;

/// The name of a config setting, for the ack line.
fn setting_name(id: u8) -> &'static str {
    match id {
        packet::CFG_UPDATE_INTERVAL_MS => "notify interval",
        ble::CFG_PWR_EN => "GPS/LoRa power",
        ble::CFG_WIO_SLEEP => "WIO-E5 sleep",
        ble::CFG_GPS_SLEEP => "GPS backup mode",
        ble::CFG_ESP_SLEEP_S => "wake-check interval",
        ble::CFG_ESP_ADV_WINDOW_S => "advertising window",
        ble::CFG_SLEEP_NOW => "sleep now",
        _ => "setting",
    }
}

/// What the board said about the last config write. On success this reports
/// the value it actually applied, which for the intervals may be a clamped
/// version of what was asked for. The on/off settings ack without a value;
/// their new state arrives in the settings blob instead.
///
/// Every setting that carries a number says which number the board took.
/// That is the only place the clamping is visible: ask for a one-second
/// window on a board whose floor is higher and the ack is what tells you it
/// stored something else, rather than the setting appearing to be ignored.
fn ack_message(ack: &Ack) -> Result<String, String> {
    let name = setting_name(ack.id);
    let applied = ack.value_u32.unwrap_or(0);
    match ack.status {
        packet::ACK_OK => Ok(match ack.id {
            packet::CFG_UPDATE_INTERVAL_MS => {
                format!("Board applied: notify interval {applied} ms")
            }
            ble::CFG_ESP_SLEEP_S if applied == 0 => "Board applied: sleep disabled".to_string(),
            ble::CFG_ESP_SLEEP_S => {
                format!("Board applied: wake check every {}", secs_text(applied))
            }
            ble::CFG_ESP_ADV_WINDOW_S => {
                format!(
                    "Board applied: advertising {} per wake",
                    secs_text(applied)
                )
            }
            // The one ack that describes what is about to happen rather
            // than what was stored - and the one where the disconnect that
            // follows is the command working. Saying so here is what keeps
            // the app from reporting its own success as a fault a moment
            // later.
            ble::CFG_SLEEP_NOW => {
                format!(
                    "Board sleeping for {} - it will disconnect now",
                    secs_text(applied)
                )
            }
            _ => format!("Board applied: {name}"),
        }),
        packet::ACK_UNKNOWN_ID => Err(format!(
            "Board rejected: it does not know the {name} setting"
        )),
        packet::ACK_BAD_VALUE => Err(format!("Board rejected: bad value for {name}")),
        // Not a rejected value: the ESP could not reach the WIO-E5 over the
        // UART link between them, so the setting never got there.
        ble::ACK_WIO_ERROR => Err(format!(
            "The board could not reach the WIO-E5 to set {name} (link error)."
        )),
        ble::ACK_WIO_TIMEOUT => Err(format!(
            "The board could not reach the WIO-E5 to set {name} (no reply)."
        )),
        ble::ACK_BAD_STATE => Err(format!(
            "Board rejected {name}: not valid in its current state"
        )),
        s => Err(format!("Board rejected {name}: status {s:#04x}")),
    }
}

/// What the user last told the BLE worker to do. Explicit rather than derived
/// from the config, because "leave the board alone" is a real thing to want:
/// the board only deep-sleeps while nothing is connected, so an app that
/// always reconnects keeps it awake and its sleep interval never does
/// anything. Disconnect is how you let it sleep.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum BleIntent {
    /// Stay off the air until asked. Nothing reconnects on its own.
    Idle,
    /// Connect, going straight to a pinned MAC when there is one.
    Connect,
    /// Connect expecting the board to be asleep: scan continuously so a wake
    /// window cannot be missed. Costs more radio time, so it is not the
    /// default.
    ConnectSleeping,
    /// Look for boards without connecting, to fill the device picker. Only one
    /// board is ever connected at a time, so this drops any live link.
    Scanning,
}

/// How long a board stays in the picker after its last advertisement. Boards
/// advertise every few hundred ms, so a few seconds of silence means it is out
/// of range or asleep rather than just between packets.
const SEEN_TIMEOUT: Duration = Duration::from_secs(10);

/// The least silence from a connected board before the status line stops
/// claiming a healthy link. A floor rather than the whole story: the board
/// notifies its position every notify interval, so the effective window also
/// scales with that (see [`MyApp::board_silence`]).
const LINK_SILENT: Duration = Duration::from_secs(10);

/// How long after connecting the board counts as warming up. The GPS/LoRa
/// rail is off through sleep and through each wake window, and comes up only
/// once a central connects, so the WIO has to boot and the GPS has to make a
/// cold fix before there is anything to report.
const BOARD_WARMUP: Duration = Duration::from_secs(45);

/// A board seen by the current scan. The address is the map key and the
/// advertised name is the same on every board, so the sighting itself carries
/// only what changes: how strong it was, and when.
struct Seen {
    rssi: Option<i16>,
    /// When it last advertised, for ageing it out of the picker.
    at: Instant,
}

/// One row of the device picker: a board the app knows about, whether from the
/// nicknames in the config or from the running scan.
pub(crate) struct DeviceRow {
    /// Normalized MAC; the identity, and the key into the nickname table.
    pub mac: String,
    /// Signal strength from the running scan, absent for a board that is only
    /// known from the config or has not been heard from recently.
    pub rssi: Option<i16>,
    /// This board is the pinned one, so it is what Connect will go to.
    pub selected: bool,
}

/// A duration in seconds as a short human phrase ("45 s", "5 min", "12 h").
/// Used wherever a sleep interval is shown or confirmed.
pub(crate) fn secs_text(s: u32) -> String {
    match s {
        0 => "off".to_string(),
        s if s < 60 => format!("{s} s"),
        s if s < 3600 => format!("{:.0} min", s as f32 / 60.0),
        s if s % 3600 == 0 => format!("{} h", s / 3600),
        s => format!("{:.1} h", s as f32 / 3600.0),
    }
}

/// Great-circle distance between two positions in meters (haversine formula).
fn haversine_m(a: Position, b: Position) -> f64 {
    const EARTH_RADIUS_M: f64 = 6_371_000.0;
    let lat1 = a.y().to_radians();
    let lat2 = b.y().to_radians();
    let dlat = (b.y() - a.y()).to_radians();
    let dlon = (b.x() - a.x()).to_radians();
    let h = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon / 2.0).sin().powi(2);
    // `h` is a squared sine and so cannot exceed 1 in exact arithmetic, but for
    // near-antipodal points rounding carries it a few ulps past, where `asin`
    // is undefined. Clamping there costs nothing and keeps a NaN - which would
    // spread through every distance comparison it reached - out of the app.
    2.0 * EARTH_RADIUS_M * h.clamp(0.0, 1.0).sqrt().asin()
}

/// Initial great-circle bearing from `a` to `b`, in degrees clockwise from
/// north. Tracking mode turns the map by this so the beacon points up.
fn bearing_deg(a: Position, b: Position) -> f32 {
    let lat1 = a.y().to_radians();
    let lat2 = b.y().to_radians();
    let dlon = (b.x() - a.x()).to_radians();
    let y = dlon.sin() * lat2.cos();
    let x = lat1.cos() * lat2.sin() - lat1.sin() * lat2.cos() * dlon.cos();
    y.atan2(x).to_degrees().rem_euclid(360.0) as f32
}

/// Ease `current` toward `target` (both degrees clockwise from north) the
/// shortest way round the circle, over the time constant `tau`. Returns the new
/// angle and how far it still had to travel, so the caller can keep asking for
/// frames until it settles.
///
/// The easing is what makes a heading readable between sensor updates: the
/// marker arrow and the heading-up map both glide instead of stepping, which
/// matters most at the low compass rate the arrow alone runs at.
pub(crate) fn ease_heading(current: f32, target: f32, dt: f32, tau: f32) -> (f32, f32) {
    // Signed shortest angular distance to the target, in (-180, 180].
    let delta = (target - current + 540.0).rem_euclid(360.0) - 180.0;
    // Time-constant easing so the feel is frame-rate independent.
    let alpha = 1.0 - (-dt / tau).exp();
    ((current + delta * alpha).rem_euclid(360.0), delta.abs())
}

/// Time constant for easing the map's heading-up rotation.
pub(crate) const ROTATE_TAU: f32 = 0.12;
/// Time constant for easing the marker's heading arrow. Slower than the map's:
/// the arrow is driven at `compass.arrow_hz`, so it has further to coast
/// between readings and a tighter constant would just step visibly.
pub(crate) const ARROW_TAU: f32 = 0.25;

/// Config file loaded at startup and written back by the Settings page, unless
/// another path is typed there.
const DEFAULT_CONFIG_NAME: &str = "gps-config.toml";

/// Where the config file lives unless the Settings page is pointed elsewhere:
/// beside the tile cache, which on Android is the app's private data directory
/// (the working directory there is not writable, so a bare filename could be
/// read but never saved). On desktop the cache is a relative directory, which
/// leaves the plain filename in the working directory.
fn default_config_path(cache_dir: Option<&std::path::Path>) -> String {
    match cache_dir.and_then(std::path::Path::parent) {
        Some(dir) if !dir.as_os_str().is_empty() => {
            dir.join(DEFAULT_CONFIG_NAME).display().to_string()
        }
        _ => DEFAULT_CONFIG_NAME.to_string(),
    }
}

/// Compass rate while heading-up is turning the map. Fast enough that the whole
/// view follows the phone rather than stepping after it; the other modes only
/// move a small arrow and use the far slower `compass.arrow_hz` instead.
const HEADING_UP_HZ: f32 = 60.0;

/// Tracking mode: fraction of the screen height kept clear above the beacon and
/// below the user, so neither marker sits hard against an edge.
const TRACK_MARGIN_FRAC: f32 = 0.18;
/// Zoom range the tracking auto-fit is clamped to.
const TRACK_ZOOM_MIN: f64 = 2.0;
const TRACK_ZOOM_MAX: f64 = 19.0;

/// Whether `pos` is far enough from the last recorded track point to append it.
/// Always true for the first point; otherwise the move must be at least
/// `min_distance_m`, so a track is decimated to points that far apart.
/// When a report the board aged by `age_s` was actually heard.
///
/// A live report is aged 0 and lands on now; one replayed on connect lands
/// where it belongs in the past, so a node heard ten minutes ago does not
/// enter the track as if it had just reported.
fn heard_at(age_s: u16) -> SystemTime {
    SystemTime::now()
        .checked_sub(Duration::from_secs(age_s.into()))
        .unwrap_or_else(SystemTime::now)
}

/// The GPS columns of a position packet as a log row, shared by the connected
/// board's own fixes and the remote nodes' relayed ones: the same packet type
/// arrives by both routes and has to log identically. The caller adds what the
/// route itself carried (a node's signal reading).
///
/// A packet without a fix still logs a row - `fix = 0` and a satellite count.
/// A receiver that is up and searching is a different state from one that has
/// stopped reporting, and only a row can tell them apart afterwards.
fn packet_row(source: LogSource, packet: PositionPacket, at: SystemTime) -> LogRow {
    let mut row = LogRow::new(source, at);
    row.fix = Some(packet.has_fix());
    row.sats = Some(packet.sats);
    if packet.has_fix() {
        row.lat = Some(packet.lat_deg());
        row.lon = Some(packet.lon_deg());
        row.alt_m = Some(packet.alt_m());
        row.speed_mps = Some(packet.speed_mps());
        row.course_deg = Some(packet.course_deg());
    }
    row
}

fn far_enough(last: Option<&Position>, pos: Position, min_distance_m: f64) -> bool {
    match last {
        None => true,
        Some(&last) => haversine_m(last, pos) >= min_distance_m,
    }
}

/// Which screen is shown. The menu page switches between them.
#[derive(Clone, Copy, PartialEq)]
pub enum Page {
    /// The page menu itself: one large button per page, and the only way to
    /// reach the others. Not a destination in its own right, so it is left out
    /// of the list it draws.
    Menu,
    /// The interactive map with the position marker and track.
    Map,
    /// Searchable list of all recorded GPS points.
    Points,
    /// The current position and beacon distance, plus board health for the
    /// esp32c6-gps board (ESP/WIO/GPS/LoRa).
    Status,
    /// The BLE beacon: the link, and the board's own power and sleep settings.
    Beacon,
    /// The app's own settings, from the TOML config file (marker colors,
    /// overlay sizes, distance read-out, track recording, offline maps).
    Settings,
    /// Viewing and editing the WIO-E5 RADIO.TOML (radio, mesh, beacon, GPS).
    Radio,
    /// CSV recording of every report, its graph, and the export off the device.
    Logging,
}

/// Per-field edit flow on the Radio page. Only one field is in flight at a time:
/// the pencil opens the confirm popup, confirming unlocks the typed input, and
/// the check/x commit or discard.
#[derive(Clone, Default)]
pub enum RadioEdit {
    /// No field is being edited.
    #[default]
    None,
    /// The confirm popup is open for this field (Edit / Cancel).
    Confirm { section: String, key: String },
    /// The field is unlocked with a typed input plus a check and an x.
    Active {
        section: String,
        key: String,
        val: EditVal,
    },
}

/// Box-selection state for the offline region download on the map page.
#[derive(Clone, Copy)]
pub enum RegionSelect {
    /// Not selecting; the map behaves normally.
    Inactive,
    /// Waiting for / tracking the box drag. Panning is disabled so the drag
    /// draws a box instead of moving the map.
    Picking {
        start: Option<Pos2>,
        current: Option<Pos2>,
    },
    /// Box chosen; the confirm panel is shown over the map.
    Confirm {
        a: Position,
        b: Position,
        max_zoom: u8,
    },
}

/// Source filter on the points page.
#[derive(Clone, Copy, PartialEq)]
pub enum PointFilter {
    All,
    Phone,
    Esp,
    /// Any remote LoRa node, whatever its address.
    Remote,
}

impl PointFilter {
    fn admits(self, source: PointSource) -> bool {
        match self {
            PointFilter::All => true,
            PointFilter::Phone => source == PointSource::Phone,
            PointFilter::Esp => source == PointSource::Esp,
            PointFilter::Remote => matches!(source, PointSource::Remote(_)),
        }
    }
}

/// A map marker the user can double-click/tap to inspect.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MarkerKind {
    /// The phone / manual position dot.
    You,
    /// The connected board's own GPS.
    Beacon,
    /// A remote LoRa node relayed by the connected board, keyed by its address.
    Remote(u8),
}

impl MarkerKind {
    /// The generic label, before any nickname is applied. A remote's nickname
    /// lives in the config, so [`MyApp::marker_label`] is what resolves it;
    /// this is the fallback that names a node by its address.
    fn label(self) -> String {
        match self {
            MarkerKind::You => "You".to_string(),
            MarkerKind::Beacon => "Beacon".to_string(),
            MarkerKind::Remote(addr) => format!("Node {addr}"),
        }
    }
}

/// HTTP tile options caching to `cache_dir` (when writable). Tiles fetched once
/// are reused from disk, so previously viewed areas keep working without a
/// network. `None` disables the cache. The user agent matches the offline
/// downloader's so both read and write the same cache entries.
fn http_options(cache_dir: Option<PathBuf>) -> HttpOptions {
    HttpOptions {
        cache: cache_dir,
        user_agent: Some(HeaderValue::from_static(offline::USER_AGENT)),
        ..Default::default()
    }
}

/// What [`MyApp::apply_ui_style`] last pushed into the egui style: the theme the
/// visuals were built from, the `[ui]` overrides laid over it, and the text
/// scale. Compared whole, so the style is rewritten when any one of them moves
/// and at no other time.
#[derive(Clone, Copy, PartialEq)]
struct AppliedStyle {
    theme: egui::Theme,
    background: Option<egui::Color32>,
    button: Option<egui::Color32>,
    text: Option<egui::Color32>,
    text_scale: f32,
}

/// A remote LoRa node relayed over BLE by the connected board, keyed in
/// [`MyApp::remotes`] by its address. Each keeps its own live position and
/// recorded path, so different boards draw as different tracks and list as
/// different sources.
#[derive(Default)]
struct RemoteNode {
    /// Live position, `None` until a fix is heard (and cleared when the board
    /// is switched, since it described the old relay's view of the mesh).
    pos: Option<Position>,
    /// When `pos` was last updated, for the marker info popup.
    time: Option<SystemTime>,
    /// The last packet decoded for this node (speed, sats, ...).
    packet: PositionPacket,
    /// LoRa signal the relay last heard this node at, in dBm.
    rssi: i16,
    /// Every recorded position, for the path drawing and the points list.
    track: Vec<TrackPoint>,
    /// When the board last heard from this node at all, position or ping.
    /// Separate from `time`, which only moves when a position arrives: a node
    /// that has lost its fix is still on the air, and the two answer
    /// different questions.
    heard: Option<SystemTime>,
    /// The node's last ping, set while its newest report says it has no fix
    /// and cleared by its next position. `Some` on a node that also has a
    /// position means the position is the last one it managed before losing
    /// the fix.
    no_fix: Option<NodePing>,
}

impl RemoteNode {
    /// The position to show for this node: the live one, or the newest
    /// recorded track point once a board switch has dropped the live view.
    /// A node with any history keeps a visible marker, not just its path.
    fn last_pos(&self) -> Option<Position> {
        self.pos.or_else(|| self.track.last().map(|t| t.pos))
    }

    /// When [`Self::last_pos`] was recorded, for the marker popup's age line.
    fn last_time(&self) -> Option<SystemTime> {
        self.time.or_else(|| self.track.last().map(|t| t.time))
    }

    /// One line describing where this node is, or why it cannot say.
    ///
    /// A node without a position is still worth a line: what a ping reports
    /// is whether to look at the sky or at the board, since a silent module
    /// is usually an unpowered rail rather than a receiver that cannot see
    /// satellites.
    fn state_text(&self) -> String {
        match (self.last_pos(), self.no_fix) {
            (Some(p), None) => format!("{:.5}, {:.5}", p.y(), p.x()),
            (Some(p), Some(ping)) => {
                format!("{:.5}, {:.5} (fix lost, {})", p.y(), p.x(), ping_reason(ping))
            }
            (None, Some(ping)) => format!("no fix ({})", ping_reason(ping)),
            (None, None) => "heard, no position yet".to_string(),
        }
    }
}

/// What a ping says about why the node has no position.
fn ping_reason(ping: NodePing) -> String {
    if !ping.gps_present {
        // Not a receiver that cannot find the sky: the module is not talking
        // at all, which is a power or wiring answer.
        "gps silent".to_string()
    } else if ping.had_fix {
        format!("searching, up {}", uptime_text(ping.uptime_s))
    } else {
        format!("no fix since boot, up {}", uptime_text(ping.uptime_s))
    }
}

/// A node's uptime in the units it reads best in.
///
/// Not [`secs_text`], which renders 0 as "off" - right for a sleep interval,
/// nonsense for a node that has just booted, which is exactly when an uptime
/// is worth reading.
fn uptime_text(secs: u16) -> String {
    match secs {
        0..=59 => format!("{secs} s"),
        60..=3599 => format!("{} min", secs / 60),
        _ => format!("{} h", secs / 3600),
    }
}

pub struct MyApp {
    /// Standard OpenStreetMap tiles.
    tiles: HttpTiles,
    /// OpenTopoMap topographic tiles, shown when `layer` is `Topo`. Both share
    /// the same on-disk cache (keyed by URL) and the same `map_memory`.
    topo_tiles: HttpTiles,
    /// Which tile layer is currently drawn.
    layer: MapLayer,
    map_memory: MapMemory,
    /// Live GPS fixes, when a source is wired up (Android GNSS). `None` on
    /// desktop, where the manual position bar is shown instead.
    gps_rx: Option<Receiver<GpsFix>>,
    /// Device-facing compass, when the platform has one (Android only). The
    /// sensor behind it is powered only while heading-up needs it.
    compass: Option<CompassHandle>,
    /// The BLE worker streaming the ESP32-C3 beacon's GPS data.
    ble: BleHandle,
    /// Returns the current safe-area insets `[top, right, bottom, left]` in
    /// physical pixels. `None` on desktop (no system bars to avoid).
    insets: Option<Box<dyn Fn() -> [f32; 4]>>,
    current: Option<Position>,
    /// When the current position was last updated, for the marker info popup.
    current_time: Option<SystemTime>,
    /// Course over ground from the GPS fix.
    heading: Option<f32>,
    /// Device-facing heading from the compass sensor.
    compass_heading: Option<f32>,
    /// When set, the map is rotated so the current heading points up.
    heading_up: bool,
    /// Master switch for the recorded paths on the map, from the bar's toggle.
    /// It only hides: which of the two paths a shown map draws is
    /// `config.track.show_path` / `config.ble.show_path`. The line to the
    /// beacon and its distance label are not paths and stay either way.
    ///
    /// Session state, like `heading_up` and the base layer: it clears the map
    /// for a moment, and the settings it overrides are the saved ones.
    show_paths: bool,
    /// Tracking mode: which beacon is being kept in frame (user near the
    /// bottom, beacon near the top). `None` is off. The track button cycles it;
    /// the heading button exits.
    ///
    /// The board itself rather than its place in [`MyApp::beacon_targets`]:
    /// that list is ordered (the connected board, then the nodes by address)
    /// and grows in the middle, so an index would quietly move to another
    /// board the moment the connected board got a fix or a lower-numbered node
    /// was heard.
    tracking_beacon: Option<MarkerKind>,
    /// Rotation angle actually drawn, eased toward the live heading each frame so
    /// the map turns smoothly instead of snapping between sensor readings.
    smoothed_heading: Option<f32>,
    /// The same for the marker's heading arrow, which is eased separately: it is
    /// drawn in every mode, while the map only turns in heading-up and tracking.
    smoothed_arrow: Option<f32>,
    track: Vec<TrackPoint>,
    /// Live position of the BLE beacon; replaces the old fixed reference
    /// point, so the distance line tracks the real device.
    beacon: Option<Position>,
    /// When the beacon position was last updated, for the marker info popup.
    beacon_time: Option<SystemTime>,
    /// The last full packet from the beacon (satellites, speed, ...).
    beacon_packet: Option<PositionPacket>,
    /// Every beacon position recorded, for the path drawing and points list.
    beacon_track: Vec<TrackPoint>,
    /// Remote LoRa nodes relayed by the connected board, keyed by address. Each
    /// draws as its own colored path and marker, and lists as its own source.
    remotes: BTreeMap<u8, RemoteNode>,
    /// Last BLE status line, for the Beacon page.
    ble_status: String,
    ble_connected: bool,
    /// Notify-interval input on the Beacon page.
    ble_interval_text: String,
    /// Result of the last config write: device ack (green) or error (red).
    ble_ack: Option<Result<String, String>>,
    /// A config write is in flight and the ack has not arrived yet.
    ble_ack_pending: bool,
    /// Latest board telemetry (esp32c6-gps), for the Status page.
    telemetry: Option<Telemetry>,
    /// Latest WIO status/log line relayed by the board.
    board_log: Option<String>,
    /// The board's own power and sleep settings, as it last reported them.
    /// The controls read this rather than any local copy: the board is the
    /// authority and changes these by itself (clamping an interval).
    board_settings: Option<Settings>,
    /// The board's settings layout is newer than this build can decode, so
    /// its settings are unknown rather than defaulted.
    settings_unsupported: bool,
    /// The WIO radio config the connected board last reported, if any. Lets
    /// the Radio page fill the editor from the board itself; `None` until the
    /// board reports one (or if the board predates the read-back).
    board_radio_config: Option<RadioConfig>,
    /// The board's radio-config layout is newer than this build can decode.
    radio_config_unsupported: bool,
    /// What the app is currently asking the BLE worker to do. Session state:
    /// `config.ble.enabled` seeds it at startup and nothing writes it back, so
    /// a Disconnect lasts until the next launch rather than becoming a setting.
    ble_intent: BleIntent,
    /// Which request the worker is being held to; bumped by every press. It
    /// rides out on the command and back on every event, so what the previous
    /// request was still saying can be told apart from what this one says -
    /// see [`Epoch`].
    ble_epoch: Epoch,
    /// The base of the "trying for ..." read-out: when the worker was last
    /// asked for the current intent, or when the link last dropped - whichever
    /// came later, so the count is this attempt's, not the whole session's.
    intent_since: Instant,
    /// When the current connection came up. The GPS/LoRa rail only powers on
    /// once a central connects, so telemetry is legitimately empty for the
    /// first seconds and the Status page says warming up, not broken.
    connected_at: Option<Instant>,
    /// When the board itself last said anything (any decoded characteristic),
    /// seeded at connect. The platform can take a long time to notice a dead
    /// link, so a quiet stretch here is the earliest sign that "connected" may
    /// no longer be true; see [`MyApp::board_silence`].
    board_heard: Option<Instant>,
    /// Wake-check interval input (seconds) on the Beacon page.
    sleep_interval_text: String,
    /// Advertising-window input (seconds) on the Beacon page.
    adv_window_text: String,
    /// "Sleep now" duration input (seconds) on the Beacon page. Blank means
    /// "use the board's wake-check interval", which is what the firmware
    /// reads a zero as.
    sleep_now_text: String,
    /// Set when a sleep was commanded, so the disconnect that follows can be
    /// reported as the command working rather than as a link that dropped.
    /// Cleared on the next connect.
    sleep_commanded: Option<u32>,
    /// Which screen is currently shown.
    page: Page,
    /// The page the menu was opened from, so closing it without picking
    /// anything goes back where it was rather than to a fixed page.
    menu_from: Page,
    /// Loaded configuration (marker colors, BLE settings).
    config: AppConfig,
    /// What was last pushed into the egui style, so [`MyApp::apply_ui_style`]
    /// only writes it when something moved.
    style_applied: Option<AppliedStyle>,
    /// The config-file path typed on the Settings page.
    config_path: String,
    /// Result of the last load/save: `Ok` message (green) or error (red).
    config_feedback: Option<Result<String, String>>,
    /// Boards seen since the current scan started, keyed by normalized MAC.
    /// Cleared when a scan begins, so the picker shows what is on the air now
    /// rather than everything ever seen.
    discovered: BTreeMap<String, Seen>,
    /// Text buffers behind the nickname inputs, keyed by normalized MAC. Held
    /// apart from `config.ble.names` so a half-typed name (or one being cleared)
    /// is not immediately written back over the config.
    name_edits: BTreeMap<String, String>,
    /// The WIO-E5 RADIO.TOML being edited on the Radio page, once loaded.
    radio: Option<RadioDoc>,
    /// The RADIO.TOML path typed on the Radio page.
    radio_path: String,
    /// Result of the last radio load/save: `Ok` message (green) or error (red).
    radio_feedback: Option<Result<String, String>>,
    /// Per-field edit flow on the Radio page.
    radio_edit: RadioEdit,
    /// The send-to-board confirm popup on the Radio page is open.
    radio_push_confirm: bool,
    /// A config push is in flight and its outcome has not arrived yet.
    radio_push_pending: bool,
    /// Tile cache directory; also the target of offline region downloads.
    cache_dir: Option<PathBuf>,
    /// Box-selection state for the offline region download.
    select: RegionSelect,
    /// Progress of the running (or just-finished) offline tile download.
    download: Option<Arc<DownloadProgress>>,
    /// Search query on the points page.
    points_search: String,
    /// Source filter on the points page.
    points_filter: PointFilter,
    /// Text in the manual position bar (shown only when `gps_rx` is `None`).
    manual_gps_text: String,
    /// The last manual position entry failed to parse.
    manual_gps_bad: bool,
    /// Marker whose info popup (name + time since last update) is shown, set by
    /// double-clicking/tapping a marker on the map.
    selected_marker: Option<MarkerKind>,
    /// The center button's marker list is open (held/right-clicked the button).
    /// A plain tap centers on you instead, without ever opening this.
    center_menu: bool,
    /// Offline center-button zoom fallback: the background probe sends the zoom
    /// level to snap to (nearest cached tile) here when it finds we are offline.
    zoom_tx: Sender<f64>,
    zoom_rx: Receiver<f64>,
    /// Width the map controls row took last frame, used to center it (egui
    /// can't center a horizontal row in a single layout pass). `0.0` until the
    /// first frame has measured it.
    controls_width: f32,
    /// The CSV recorder behind the Logging page.
    logger: Logger,
    /// The log path typed on the Logging page. Seeded from `[log] file`, or a
    /// timestamped name beside the config when that is unset.
    log_path: String,
    /// Result of the last log start/stop/export: `Ok` (green) or error (red).
    log_feedback: Option<Result<String, String>>,
    /// The graph's axes: what is plotted, and against what.
    log_x: LogAxis,
    log_y: LogStat,
    /// Text buffer behind the reference-coordinate input ("lat, lon"), held
    /// apart from the config so a half-typed coordinate is not committed.
    log_ref_text: String,
    /// The last reference entry failed to parse.
    log_ref_bad: bool,
    /// Sources hidden from the graph, toggled from its legend. Session state:
    /// it is a way to read a busy plot, not a setting worth saving.
    log_hidden: BTreeSet<LogSource>,
    /// Writes a file somewhere the user can reach it (Android's Downloads).
    /// `None` on desktop, where the log path is already reachable and the
    /// export writes the copy itself.
    export: Option<Saver>,
}

impl MyApp {
    /// `gps_rx` is the live GPS fix stream, or `None` when no source is wired
    /// up (desktop) - the UI then shows a manual position entry bar instead.
    /// `cache_dir` is where tiles are cached to disk (`None` to disable). Desktop
    /// passes a local `.cache`; Android passes its writable data directory.
    /// `compass` is the device-facing heading source (`None` on desktop).
    /// `insets` reports the safe-area insets in physical pixels (`None` on desktop).
    /// `ble` is the worker connected to the ESP32-C3 GPS beacon.
    /// `export` puts a file where the user can reach it (`None` on desktop,
    /// where the log is written to a reachable path to begin with).
    pub fn new(
        ctx: egui::Context,
        gps_rx: Option<Receiver<GpsFix>>,
        cache_dir: Option<PathBuf>,
        compass: Option<CompassHandle>,
        insets: Option<Box<dyn Fn() -> [f32; 4]>>,
        ble: BleHandle,
        export: Option<Saver>,
    ) -> Self {
        // SVG loader for the button icons.
        egui_extras::install_image_loaders(&ctx);

        let (zoom_tx, zoom_rx) = std::sync::mpsc::channel();

        let mut app = Self {
            tiles: HttpTiles::with_options(
                OpenStreetMap,
                http_options(cache_dir.clone()),
                ctx.clone(),
            ),
            topo_tiles: HttpTiles::with_options(
                OpenTopoMap,
                http_options(cache_dir.clone()),
                ctx,
            ),
            layer: MapLayer::Standard,
            map_memory: MapMemory::default(),
            gps_rx,
            compass,
            ble,
            insets,
            current: None,
            current_time: None,
            heading: None,
            compass_heading: None,
            heading_up: false,
            show_paths: true,
            tracking_beacon: None,
            smoothed_heading: None,
            smoothed_arrow: None,
            track: Vec::new(),
            beacon: None,
            beacon_time: None,
            beacon_packet: None,
            beacon_track: Vec::new(),
            remotes: BTreeMap::new(),
            ble_status: "idle".to_string(),
            ble_connected: false,
            ble_interval_text: packet::UPDATE_INTERVAL_DEFAULT_MS.to_string(),
            ble_ack: None,
            ble_ack_pending: false,
            telemetry: None,
            board_log: None,
            board_settings: None,
            settings_unsupported: false,
            board_radio_config: None,
            radio_config_unsupported: false,
            // Overwritten by `apply_config`/`sync_ble_to_config` below, which
            // is what actually decides whether to connect at startup.
            ble_intent: BleIntent::Idle,
            // 0 is "nothing asked for yet"; the first request is 1.
            ble_epoch: 0,
            intent_since: Instant::now(),
            connected_at: None,
            board_heard: None,
            // The low end of the clamp range, so a stray press arms the
            // shortest sleep rather than the longest.
            sleep_interval_text: ble::ESP_SLEEP_MIN_S.to_string(),
            // The firmware's own default, not the low end: a short window is
            // the hazardous direction here (it is what makes a sleeping board
            // hard to catch), so a stray press should ask for what an
            // unconfigured board already does.
            adv_window_text: ble::ESP_ADV_DEFAULT_S.to_string(),
            // Blank rather than a number: the common press is "sleep for
            // the cadence I already configured", and a prefilled box would
            // make the uncommon one look like the default.
            sleep_now_text: String::new(),
            sleep_commanded: None,
            page: Page::Map,
            menu_from: Page::Map,
            config: AppConfig::default(),
            style_applied: None,
            // The path the auto-load below tries, so Save writes back to the
            // same file without the user having to type it.
            config_path: default_config_path(cache_dir.as_deref()),
            config_feedback: None,
            discovered: BTreeMap::new(),
            name_edits: BTreeMap::new(),
            radio: None,
            radio_path: "RADIO.toml".to_string(),
            radio_feedback: None,
            radio_edit: RadioEdit::None,
            radio_push_confirm: false,
            radio_push_pending: false,
            cache_dir,
            select: RegionSelect::Inactive,
            download: None,
            points_search: String::new(),
            points_filter: PointFilter::All,
            manual_gps_text: String::new(),
            manual_gps_bad: false,
            selected_marker: None,
            center_menu: false,
            zoom_tx,
            zoom_rx,
            controls_width: 0.0,
            logger: Logger::default(),
            // Replaced below once the config has been loaded, which is what
            // may name a log file of its own.
            log_path: String::new(),
            log_feedback: None,
            log_x: LogAxis::Time,
            log_y: LogStat::Distance,
            log_ref_text: String::new(),
            log_ref_bad: false,
            log_hidden: BTreeSet::new(),
            export,
        };

        // Auto-load the default config when present; the Settings page can load
        // any path later, and saves back to whichever one is in the box. With no
        // file the defaults apply, which include connecting to the beacon.
        let startup_path = app.config_path.clone();
        match AppConfig::load(&startup_path) {
            Ok(cfg) => app.apply_config(cfg),
            Err(_) => app.sync_ble_to_config(),
        }
        // After the config, which is where both the path and the auto-start
        // come from.
        app.sync_log_to_config();
        if app.config.log.auto_start {
            app.start_log();
        }
        app
    }

    /// Adopt a loaded config: colors, the board nicknames, and the BLE
    /// connection.
    fn apply_config(&mut self, cfg: AppConfig) {
        // The nickname inputs mirror the old config; drop them so each reseeds
        // from the config just loaded.
        self.name_edits.clear();
        self.config = cfg;
        self.sync_ble_to_config();
        self.sync_log_to_config();
    }

    // --- CSV logging -------------------------------------------------------

    /// Where a log with no configured path is written: a timestamped file
    /// beside the config, which is the one directory known to be writable on
    /// both platforms (on Android the working directory is not).
    fn default_log_path(&self) -> String {
        let name = logging::default_log_name(SystemTime::now());
        match std::path::Path::new(&self.config_path).parent() {
            Some(dir) if !dir.as_os_str().is_empty() => dir.join(name).display().to_string(),
            _ => name,
        }
    }

    /// Seed the log inputs from the config. The path is left alone while a
    /// recording is running: the file being written to is not something a
    /// config load should move out from under it.
    fn sync_log_to_config(&mut self) {
        if !self.logger.is_recording() {
            self.log_path = match &self.config.log.file {
                Some(path) => path.clone(),
                None => self.default_log_path(),
            };
        }
        self.log_ref_text = match self.config.log.reference() {
            Some((lat, lon)) => format!("{lat:.5}, {lon:.5}"),
            None => String::new(),
        };
        self.log_ref_bad = false;
    }

    /// The fixed reference coordinate, when one is configured.
    fn log_reference(&self) -> Option<Position> {
        self.config
            .log
            .reference()
            .map(|(lat, lon)| lat_lon(lat, lon))
    }

    fn start_log(&mut self) {
        let path = self.log_path.clone();
        self.log_feedback = match self.logger.start(&path) {
            Ok(()) => Some(Ok(format!("Recording to {path}"))),
            Err(e) => Some(Err(e)),
        };
    }

    fn stop_log(&mut self) {
        if !self.logger.is_recording() {
            return;
        }
        self.logger.stop();
        self.log_feedback = Some(Ok(format!("Stopped after {} rows", self.logger.written())));
    }

    /// Put a copy of the log where the user can get at it: through the
    /// platform's export (Android's Downloads) when there is one, otherwise
    /// straight to a file beside the log, since the desktop path is already
    /// somewhere reachable.
    fn export_log(&mut self) {
        let text = match self.logger.export_text() {
            Ok(text) => text,
            Err(e) => {
                self.log_feedback = Some(Err(e));
                return;
            }
        };
        let name = logging::default_log_name(SystemTime::now());
        self.log_feedback = Some(match &self.export {
            Some(save) => save(&name, &text),
            None => {
                let path = match std::path::Path::new(&self.log_path).parent() {
                    Some(dir) if !dir.as_os_str().is_empty() => dir.join(&name),
                    _ => PathBuf::from(&name),
                };
                std::fs::write(&path, &text)
                    .map(|()| format!("Copied to {}", path.display()))
                    .map_err(|e| format!("{}: {e}", path.display()))
            }
        });
    }

    /// Record one report, filling in the columns derived from where we are:
    /// the control device's own position, and the distances from it and from
    /// the fixed reference to whatever reported.
    ///
    /// Deriving them here rather than at each call site is what keeps a row's
    /// distance and its signal reading the same instant - the pairing the log
    /// exists to capture.
    fn record(&mut self, mut row: LogRow) {
        if !self.logger.is_recording() {
            return;
        }
        if let Some(user) = self.current {
            row.user_lat = Some(user.y());
            row.user_lon = Some(user.x());
        }
        if let (Some(lat), Some(lon)) = (row.lat, row.lon) {
            let pos = lat_lon(lat, lon);
            // Not for our own fixes: the distance from the control device to
            // itself is zero, and a zero in that column would read as a source
            // that had arrived rather than as one the column does not apply to.
            if row.source != LogSource::Phone {
                row.dist_user_m = self.current.map(|user| haversine_m(user, pos));
            }
            row.dist_ref_m = self.log_reference().map(|r| haversine_m(pos, r));
        }
        if let Err(e) = self.logger.push(row) {
            self.log_feedback = Some(Err(format!("Logging stopped: {e}")));
        }
    }

    /// Push the `[ui]` table into the egui style: the surface and text colors
    /// (`background`, `button` and `text`) into the visuals for the active
    /// theme, and `text_scale` into the text styles.
    ///
    /// Each application starts from `Theme::default_visuals` and
    /// `default_text_styles` rather than editing what is already there, so
    /// clearing an override restores the theme without the app keeping a copy of
    /// it, and a scale is applied to the base sizes rather than compounding on
    /// the last scaled ones. Switching theme restores the visuals the same way,
    /// which is why the theme is part of what is compared below.
    ///
    /// Only run when something changed: writing the style clones it, and a map
    /// that repaints continuously would pay for that every frame.
    fn apply_ui_style(&mut self, ctx: &egui::Context) {
        let colors = self.config.ui;
        let theme = ctx.theme();
        let wanted = AppliedStyle {
            theme,
            background: colors.background,
            button: colors.button,
            text: colors.text,
            text_scale: colors.text_scale,
        };
        if self.style_applied == Some(wanted) {
            return;
        }
        self.style_applied = Some(wanted);

        // Text size first, and for both themes: the sizes are the same either
        // way, so switching theme has nothing to redo here.
        let scale = colors.text_scale;
        ctx.all_styles_mut(|style| {
            style.text_styles = egui::style::default_text_styles()
                .into_iter()
                .map(|(name, font)| (name, egui::FontId::new(font.size * scale, font.family)))
                .collect();
        });

        let mut visuals = theme.default_visuals();
        if let Some(c) = colors.background {
            visuals.panel_fill = c;
            visuals.window_fill = c;
        }
        // Text before buttons: the button shades below are blended toward
        // whatever the text color ends up being, configured or not.
        if let Some(c) = colors.text {
            // Every widget state's foreground, which is where egui reads text,
            // the toolbar glyphs (tinted with `text_color`) and the checkmarks
            // from - rather than `override_text_color`, which reaches the plain
            // labels only and would leave the rest on the theme.
            for widget in [
                &mut visuals.widgets.noninteractive,
                &mut visuals.widgets.inactive,
                &mut visuals.widgets.hovered,
                &mut visuals.widgets.open,
            ] {
                widget.fg_stroke.color = c;
            }
            // `strong` text (the section headings) reads the active state, which
            // the theme keeps a step past the body text - brighter in the dark
            // theme, darker in the light one. Shading the configured color the
            // same way keeps the emphasis without a second setting for it.
            let past = if visuals.dark_mode {
                egui::Color32::WHITE
            } else {
                egui::Color32::BLACK
            };
            visuals.widgets.active.fg_stroke.color = c.lerp_to_gamma(past, 0.35);
            // Weak text is derived from the body color by alpha, so it follows.
        }
        if let Some(c) = colors.button {
            // The hover and press shades are blended toward the text color, not
            // toward black or white: that is lighter than the button on a dark
            // theme and darker on a light one, so one configured color reads as
            // three states in either.
            let text = visuals.text_color();
            for (widget, shade) in [
                (&mut visuals.widgets.inactive, 0.0),
                (&mut visuals.widgets.hovered, 0.15),
                (&mut visuals.widgets.active, 0.3),
                (&mut visuals.widgets.open, 0.15),
            ] {
                let fill = c.lerp_to_gamma(text, shade);
                widget.bg_fill = fill;
                widget.weak_bg_fill = fill;
            }
        }
        ctx.set_visuals_of(theme, visuals);
    }

    /// Seed the intent from the config at startup: auto-connect unless the
    /// master switch is off.
    fn sync_ble_to_config(&mut self) {
        let intent = if self.config.ble.enabled {
            BleIntent::Connect
        } else {
            BleIntent::Idle
        };
        self.set_ble_intent(intent);
    }

    /// Send one request to the worker under the current epoch.
    fn send_ble(&self, command: BleCommand) {
        let _ = self.ble.commands.send(BleRequest {
            epoch: self.ble_epoch,
            command,
        });
    }

    /// Ask the worker for a new connection state, and say so even when the
    /// intent has not changed - that is what makes the buttons re-send a
    /// request with an edited MAC, or restart a scan that has given up.
    ///
    /// Every press is forceful. Bumping the epoch first is what makes it so:
    /// the worker abandons whatever session it was running at the next step it
    /// reaches rather than finishing a connect to a board nobody is asking for
    /// any more, and everything the old session goes on to say is fenced out
    /// in [`Self::drain_sources`]. So the link state is dropped here too - a
    /// disconnect ends it, and a connect (even to the same board) re-reads all
    /// of it from scratch.
    ///
    /// Each button still sends exactly one command. They must not be composed
    /// (a Disconnect followed by a Connect, say): the worker drains its whole
    /// queue in one pass, so the later command simply overwrites the earlier
    /// one and the disconnect never happens.
    pub(crate) fn set_ble_intent(&mut self, intent: BleIntent) {
        self.ble_epoch += 1;
        // Every call sends a fresh request, so the "for ..." clock restarts
        // even when the intent itself is unchanged (a re-sent connect or a
        // restarted scan is a new attempt, not the old one continuing).
        self.intent_since = Instant::now();
        self.ble_intent = intent;
        // Not "we will be disconnected shortly": as of this press there is no
        // link, and nothing the last board said still describes anything.
        // Waiting for the worker to confirm would leave the pages showing a
        // board the user has already let go of.
        self.ble_connected = false;
        self.connected_at = None;
        self.board_heard = None;
        self.forget_board_state();
        // A radio push was riding the link this press just ended. Its outcome
        // is either never coming or belongs to the request being replaced, so
        // the page is answered here rather than left waiting for an event that
        // will be fenced out.
        if self.radio_push_pending {
            self.radio_push_pending = false;
            self.radio_feedback = Some(Err(
                "Send cancelled: the link was dropped before it finished.".to_string(),
            ));
        }
        // A new scan starts from an empty list: leaving the last one's boards
        // there would show devices that may since have gone.
        if intent == BleIntent::Scanning {
            self.discovered.clear();
        }
        // The worker's own commentary is a moment behind; until it catches up
        // its last line describes the session just abandoned.
        self.ble_status = match intent {
            BleIntent::Idle => "idle".to_string(),
            BleIntent::Scanning => "starting a scan...".to_string(),
            BleIntent::Connect | BleIntent::ConnectSleeping => "starting a connect...".to_string(),
        };
        self.send_ble(match intent {
            BleIntent::Idle => BleCommand::Disconnect,
            BleIntent::Scanning => BleCommand::Scan,
            BleIntent::Connect => BleCommand::Connect {
                mac: self.config.ble.mac.clone(),
                chase: false,
            },
            BleIntent::ConnectSleeping => BleCommand::Connect {
                mac: self.config.ble.mac.clone(),
                chase: true,
            },
        });
    }

    /// Pin `mac` (or `None` for "any board") as the device to connect to. When
    /// something is already connected this switches to the new board, since only
    /// one is ever connected at a time; while idle it just records the choice.
    pub(crate) fn select_device(&mut self, mac: Option<&str>) {
        let mac = mac.map(normalize_mac);
        if mac == self.config.ble.mac {
            return;
        }
        self.config.ble.mac = mac;
        self.forget_board_state();
        // Re-send so the worker switches now rather than at the next Connect.
        // Scanning is left alone: choosing from the list should not stop the
        // scan that is filling it.
        match self.ble_intent {
            BleIntent::Connect | BleIntent::ConnectSleeping => {
                self.set_ble_intent(self.ble_intent);
            }
            BleIntent::Idle | BleIntent::Scanning => {}
        }
    }

    /// Drop everything the last board told us, on switching to a different one.
    /// None of it describes the new board, and a stale position is the worst of
    /// it: the map would go on drawing the old board's last fix as if it were
    /// the one now selected.
    ///
    /// `beacon_track` and the remote nodes' tracks are deliberately kept. They
    /// are recorded history that also backs the Points page, so discarding them
    /// would delete data the user may want; the points carry their own
    /// timestamps and source. Only the live view (positions the old relay was
    /// showing) is dropped, so the map stops drawing them as the new board's.
    /// A remote node is still its LoRa address whichever board relays it, so
    /// its marker stays on the map at its last recorded point (see
    /// [`RemoteNode::last_pos`]); only the freshness bookkeeping resets.
    fn forget_board_state(&mut self) {
        self.beacon = None;
        self.beacon_time = None;
        self.beacon_packet = None;
        for node in self.remotes.values_mut() {
            node.pos = None;
            node.time = None;
            node.packet = PositionPacket::default();
            // Both describe what the old relay could hear, not the node.
            node.heard = None;
            node.no_fix = None;
        }
        self.board_settings = None;
        self.settings_unsupported = false;
        // The next board may run a different config; drop this one so the
        // Radio page's "load from board" cannot offer a stale board's values.
        // Any editor document the user loaded is theirs and stays put.
        self.board_radio_config = None;
        self.radio_config_unsupported = false;
        self.telemetry = None;
        self.board_log = None;
        self.ble_ack = None;
        self.ble_ack_pending = false;
    }

    /// The device picker's rows: every board with a nickname, plus every board
    /// the running scan has heard from. Named boards come first so the list has
    /// a stable shape between scans, with unnamed ones (seen but never named)
    /// after them.
    ///
    /// The pinned board always gets a row even when it is neither named nor on
    /// the air, so what the app is set to connect to is never invisible.
    pub(crate) fn device_rows(&self) -> Vec<DeviceRow> {
        let mut macs: Vec<String> = self.config.ble.names.keys().cloned().collect();
        for mac in self.discovered.keys().chain(self.config.ble.mac.iter()) {
            if !macs.contains(mac) {
                macs.push(mac.clone());
            }
        }
        macs.into_iter()
            .map(|mac| DeviceRow {
                rssi: self
                    .discovered
                    .get(&mac)
                    .filter(|seen| seen.at.elapsed() < SEEN_TIMEOUT)
                    .and_then(|seen| seen.rssi),
                selected: self.config.ble.is_selected(&mac),
                mac,
            })
            .collect()
    }

    /// The nickname input's buffer for `mac`, seeded from the config the first
    /// time the row is drawn.
    pub(crate) fn name_edit(&mut self, mac: &str) -> &mut String {
        if !self.name_edits.contains_key(mac) {
            let seed = self.config.ble.name_of(mac).unwrap_or_default().to_string();
            self.name_edits.insert(mac.to_string(), seed);
        }
        self.name_edits
            .get_mut(mac)
            .expect("inserted above when missing")
    }

    /// Adopt the typed nickname for `mac`. Blanking it forgets the board, which
    /// is how a device leaves the picker for good.
    pub(crate) fn commit_name(&mut self, mac: &str) {
        let typed = self.name_edits.get(mac).cloned().unwrap_or_default();
        self.config.ble.set_name(mac, &typed);
    }

    /// What the app is doing about the link, for the Settings and Status
    /// pages. Separate from `ble_status`, which is the worker's own running
    /// commentary on the attempt.
    pub(crate) fn ble_intent_text(&self) -> String {
        let waiting = secs_text((self.intent_since.elapsed().as_secs() as u32).max(1));
        match (self.ble_intent, self.ble_connected) {
            (BleIntent::Idle, _) => "Not connecting. The board is free to sleep.".to_string(),
            (BleIntent::Scanning, _) => {
                format!("Looking for boards for {waiting}. Not connected to any.")
            }
            // "Connected" is only claimed while the board is actually talking:
            // the platform can hold a dead link open for a long time, and this
            // line saying all is well then is worse than saying nothing.
            (_, true) => match self.board_silence() {
                Some(quiet) => format!(
                    "Connected, but nothing from the board for {}.",
                    secs_text(quiet.as_secs() as u32)
                ),
                None => "Connected. The board stays awake until you disconnect.".to_string(),
            },
            (BleIntent::Connect, false) => format!("Connecting for {waiting}."),
            (BleIntent::ConnectSleeping, false) => {
                format!("Scanning for a sleeping board for {waiting}.")
            }
        }
    }

    /// How long the connected board has been silent beyond what its notify
    /// cadence allows; `None` while traffic is arriving, before the grace runs
    /// out, or with no link at all. The grace is three notify intervals (a
    /// missed packet or two is radio weather) with [`LINK_SILENT`] as the
    /// floor for boards whose settings are unknown or whose interval is short.
    pub(crate) fn board_silence(&self) -> Option<Duration> {
        if !self.ble_connected {
            return None;
        }
        let quiet = self.board_heard?.elapsed();
        let cadence = self
            .board_settings
            .map(|s| Duration::from_millis(u64::from(s.notify_interval_ms) * 3))
            .unwrap_or(Duration::ZERO);
        (quiet > cadence.max(LINK_SILENT)).then_some(quiet)
    }

    /// The board the app is pinned to, named for a heading or a status line.
    /// "Any board" when nothing is pinned, which is what an empty MAC means.
    pub(crate) fn selected_device_label(&self) -> String {
        match &self.config.ble.mac {
            Some(mac) => self.config.ble.label_of(mac),
            None => "Any board".to_string(),
        }
    }

    /// Queue one config write to the board and wait for its ack. The controls
    /// stay disabled until the ack lands, so only one write is ever in flight
    /// and the state shown is always one the board has confirmed.
    pub(crate) fn send_config(&mut self, write: ConfigWrite) {
        self.send_ble(BleCommand::Config(write));
        self.ble_ack = None;
        self.ble_ack_pending = true;
    }

    /// Load the config file at `config_path`, recording a human-readable
    /// result for the Settings page to show.
    fn load_config(&mut self) {
        let path = self.config_path.trim().to_string();
        if path.is_empty() {
            self.config_feedback = Some(Err("Enter a file path.".to_string()));
            return;
        }
        self.config_feedback = Some(match AppConfig::load(&path) {
            Ok(cfg) => {
                self.apply_config(cfg);
                Ok(format!("Loaded {path}"))
            }
            Err(e) => Err(e),
        });
    }

    /// Write the settings as they stand to `config_path`. An existing file is
    /// edited in place (comments and unknown keys survive); with no file there
    /// yet, a documented one is generated.
    fn save_config(&mut self) {
        let path = self.config_path.trim().to_string();
        if path.is_empty() {
            self.config_feedback = Some(Err("Enter a file path.".to_string()));
            return;
        }
        self.config_feedback = Some(match self.config.save(&path) {
            Ok(true) => Ok(format!("Created {path}")),
            Ok(false) => Ok(format!("Saved {path}")),
            Err(e) => Err(e),
        });
    }

    /// Drop every setting back to its built-in default. The file is untouched
    /// until the next save, so this is undoable by reloading.
    fn reset_config(&mut self) {
        self.apply_config(AppConfig::default());
        self.config_feedback = Some(Ok("Reset to defaults. Not saved yet.".to_string()));
    }

    /// Load the RADIO.TOML at `radio_path`, recording a human-readable result
    /// for the Radio page to show and clearing any in-flight edit.
    fn load_radio(&mut self) {
        let path = self.radio_path.trim().to_string();
        if path.is_empty() {
            self.radio_feedback = Some(Err("Enter a file path.".to_string()));
            return;
        }
        self.radio_edit = RadioEdit::None;
        self.radio_feedback = Some(match RadioDoc::load(&path) {
            Ok(doc) => {
                self.radio = Some(doc);
                Ok(format!("Loaded {path}"))
            }
            Err(e) => Err(e),
        });
    }

    /// Start a RADIO.TOML at the firmware defaults, aimed at `radio_path`.
    /// Nothing is written until Save, which backs up any existing file first,
    /// so this cannot lose a config by itself.
    fn default_radio(&mut self) {
        let path = self.radio_path.trim().to_string();
        if path.is_empty() {
            self.radio_feedback = Some(Err("Enter a file path.".to_string()));
            return;
        }
        self.radio_edit = RadioEdit::None;
        self.radio_feedback = Some(match RadioDoc::default_at(&path) {
            Ok(doc) => {
                self.radio = Some(doc);
                Ok(format!("Default config ready. Press Save to write {path}"))
            }
            Err(e) => Err(e),
        });
    }

    /// Fill the editor with the settings the connected board reported over
    /// BLE. Overlays them onto the current document (or a fresh default one,
    /// so the help text and dropdowns are there) rather than a file, and
    /// leaves it dirty: Save writes it, Send to board pushes it back.
    fn load_radio_from_board(&mut self) {
        let Some(cfg) = self.board_radio_config else {
            self.radio_feedback = Some(Err(if self.radio_config_unsupported {
                "The board's config format is newer than this app can read.".to_string()
            } else {
                "No config from the board yet. Connect on the Beacon page and \
                 wait for it to report (the GPS/LoRa rail must be on).".to_string()
            }));
            return;
        };
        // With no document open, start from the firmware defaults so the
        // overlaid values keep their help strings and enum dropdowns.
        if self.radio.is_none() {
            let path = self.radio_path.trim();
            let path = if path.is_empty() { "RADIO.toml" } else { path };
            match RadioDoc::default_at(path) {
                Ok(doc) => self.radio = Some(doc),
                Err(e) => {
                    self.radio_feedback = Some(Err(e));
                    return;
                }
            }
        }
        self.radio_edit = RadioEdit::None;
        if let Some(doc) = self.radio.as_mut() {
            doc.apply_config(&cfg);
        }
        self.radio_feedback = Some(Ok(
            "Loaded the board's current settings. Edit and Save, or Send to board.".to_string(),
        ));
    }

    /// Write the edited RADIO.TOML back, backing up the previous file first.
    fn save_radio(&mut self) {
        let Some(doc) = self.radio.as_mut() else {
            return;
        };
        self.radio_feedback = Some(match doc.save() {
            Ok(Some(backup)) => {
                let name = backup
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default()
                    .to_string();
                Ok(format!("Saved. Backed up previous version as {name}"))
            }
            Ok(None) => Ok("Saved.".to_string()),
            Err(e) => Err(e),
        });
    }

    /// Send the editor's current config (unsaved edits included) to the
    /// connected board over BLE. The WIO applies it live and stores it on the
    /// SD card and in its flash backup; the outcome lands in
    /// `radio_feedback` when the board's final ack arrives.
    fn push_radio(&mut self) {
        let Some(doc) = self.radio.as_ref() else {
            return;
        };
        let data = doc.wire_bytes();
        if data.len() > radio::CONFIG_MAX {
            self.radio_feedback = Some(Err(format!(
                "Config is {} bytes stripped, over the board's {}-byte limit. \
                 Remove keys that are at their default - an absent key keeps it.",
                data.len(),
                radio::CONFIG_MAX
            )));
            return;
        }
        self.send_ble(BleCommand::PushConfig(data));
        self.radio_push_pending = true;
        self.radio_feedback = Some(Ok("Sending config to the board...".to_string()));
    }

    /// Safe-area inset at the top (status bar) in egui points.
    fn top_inset(&self, ctx: &egui::Context) -> f32 {
        match &self.insets {
            Some(f) => f()[0] / ctx.pixels_per_point(),
            None => 0.0,
        }
    }

    /// Safe-area inset at the bottom (gesture bar) in egui points.
    fn bottom_inset(&self, ctx: &egui::Context) -> f32 {
        match &self.insets {
            Some(f) => f()[2] / ctx.pixels_per_point(),
            None => 0.0,
        }
    }

    /// Device-facing compass heading if available, otherwise course over ground.
    fn effective_heading(&self) -> Option<f32> {
        self.compass_heading.or(self.heading)
    }

    /// Whether the map can be turned to a heading at all: either a heading is
    /// already known, or a compass exists that would supply one once powered.
    /// The heading-up button is shown on this rather than on a live reading,
    /// since the sensor is off until that button turns it on.
    fn has_direction(&self) -> bool {
        self.effective_heading().is_some() || self.compass.is_some()
    }

    /// Power the compass sensor, and at what rate, for what is being drawn.
    ///
    /// The rotation-vector sensor is fused from the accelerometer, gyroscope
    /// and magnetometer, so it keeps all three awake and the rate is what a
    /// device heading costs in battery. Heading-up turns the whole map and gets
    /// the full rate. North-up and tracking (which turns the map by the bearing
    /// to the beacon) only point the marker's heading arrow, so they run the
    /// sensor at `compass.arrow_hz` - or leave it off, and the arrow on course
    /// over ground, when `compass.marker_arrow` is unset.
    fn sync_compass_power(&mut self) {
        let Some(compass) = &self.compass else { return };
        // The arrow is only drawn on the map, so it only asks for the sensor
        // there; nothing is measured for an arrow behind another page.
        let arrow = self.config.compass.marker_arrow && self.page == Page::Map;
        let wanted = self.heading_up || arrow;
        let hz = if self.heading_up {
            HEADING_UP_HZ
        } else {
            self.config.compass.arrow_hz
        };
        compass
            .interval_us
            .store(compass::interval_us(hz), Ordering::Relaxed);
        // Switching off drops the last reading: it stops being updated, so
        // holding on to it would draw a heading that quietly goes stale.
        if compass.wanted.swap(wanted, Ordering::Relaxed) && !wanted {
            self.compass_heading = None;
        }
    }

    /// Center the map on `target`, leaving tracking mode (which recomputes the
    /// center every frame and would override this at once). `follow` re-follows
    /// the live position rather than pinning the map to one point, which is what
    /// centering on yourself should do.
    ///
    /// When tiles are cached to disk this also kicks off the offline check: if
    /// we turn out to be offline and the current zoom has no tile for `target`,
    /// the map snaps to the nearest zoom that does.
    fn center_on(&mut self, ctx: &egui::Context, target: Position, follow: bool) {
        self.tracking_beacon = None;
        if follow {
            self.map_memory.follow_my_position();
        } else {
            self.map_memory.center_at(target);
        }
        if let Some(dir) = self.cache_dir.clone() {
            let current_zoom = self.map_memory.zoom().round().clamp(0.0, 19.0) as u8;
            offline::spawn_offline_zoom(
                dir,
                self.layer,
                target,
                current_zoom,
                self.zoom_tx.clone(),
                ctx.clone(),
            );
        }
    }

    /// The markers the center button can center on, in menu order: you first
    /// (the plain tap's target), then each beacon board (the connected board,
    /// then each remote node). Only markers with a known position are listed,
    /// so an entry always has somewhere to go.
    fn center_targets(&self) -> Vec<(MarkerKind, Position)> {
        let mut targets: Vec<(MarkerKind, Position)> =
            self.current.map(|p| (MarkerKind::You, p)).into_iter().collect();
        targets.extend(self.beacon_targets());
        targets
    }

    /// The beacon boards the map can point to: the connected board first, then
    /// each remote node in address order, each with a known position. Tracking
    /// mode cycles through this and the center menu jumps to it, so the two
    /// keep the same order and the tracking index stays meaningful.
    fn beacon_targets(&self) -> Vec<(MarkerKind, Position)> {
        let mut targets: Vec<(MarkerKind, Position)> = self
            .beacon
            .map(|p| (MarkerKind::Beacon, p))
            .into_iter()
            .collect();
        targets.extend(
            self.remotes
                .iter()
                .filter_map(|(&addr, node)| node.last_pos().map(|p| (MarkerKind::Remote(addr), p))),
        );
        targets
    }

    /// The board the distance read-out and its line refer to: the one tracking
    /// mode currently has selected, or the connected board when not tracking.
    /// `None` until that board has a position.
    fn distance_target(&self) -> Option<(MarkerKind, Position)> {
        match self.tracking_beacon {
            Some(kind) => self.beacon_target(kind),
            None => self.beacon.map(|p| (MarkerKind::Beacon, p)),
        }
    }

    /// One board out of [`Self::beacon_targets`], or `None` when it has no
    /// position to point at any more.
    fn beacon_target(&self, kind: MarkerKind) -> Option<(MarkerKind, Position)> {
        self.beacon_targets().into_iter().find(|&(k, _)| k == kind)
    }

    /// Whether tracking mode can be entered: it frames the user and a board
    /// together, so it needs both.
    fn can_track(&self) -> bool {
        self.current.is_some() && !self.beacon_targets().is_empty()
    }

    /// Where the tracking button would go next: the board after the one being
    /// followed, or the first board when the mode is off. `None` means the next
    /// press leaves the mode.
    ///
    /// A followed board that has dropped off the list entirely (its node
    /// forgotten on a board switch) starts the walk over rather than ending it,
    /// so the button never reads as "exit" for a board that is no longer there.
    fn next_tracking(&self) -> Option<MarkerKind> {
        let targets = self.beacon_targets();
        let after = self.tracking_beacon.and_then(|current| {
            targets
                .iter()
                .position(|&(kind, _)| kind == current)
                .map(|i| targets.get(i + 1).map(|&(kind, _)| kind))
        });
        match after {
            // The followed board is listed; the answer is whatever follows it.
            Some(next) => next,
            // Not tracking, or tracking something no longer listed.
            None => targets.first().map(|&(kind, _)| kind),
        }
    }

    /// Advance the tracking selection, as the map bar's track button does: into
    /// the mode on the first board, along the list, and out of it after the
    /// last one. A single board is therefore a plain on/off toggle.
    fn cycle_tracking(&mut self) {
        let next = self.next_tracking();
        // Tracking and heading-up are mutually exclusive.
        if next.is_some() && self.tracking_beacon.is_none() {
            self.heading_up = false;
        }
        self.tracking_beacon = next;
    }

    /// What the track button's press would do, for its hover text.
    fn tracking_hint(&self) -> &'static str {
        match (self.tracking_beacon, self.next_tracking()) {
            (None, _) => "Track beacon",
            (Some(_), Some(_)) => "Next beacon",
            (Some(_), None) => "Exit tracking",
        }
    }

    /// Great-circle distance from the current position to the distance target
    /// (see [`Self::distance_target`]), in meters. `None` until both a fix and
    /// that board's position are known.
    fn distance_to_target(&self) -> Option<f64> {
        match (self.current, self.distance_target()) {
            (Some(cur), Some((_, target))) => Some(haversine_m(cur, target)),
            _ => None,
        }
    }

    /// A marker's display name, resolving a remote node's nickname from the
    /// config. The one place a node's name is decided, so every page agrees.
    fn marker_label(&self, kind: MarkerKind) -> String {
        match kind {
            MarkerKind::Remote(addr) => self.config.lora.label_of(addr),
            other => other.label(),
        }
    }

    /// Every recorded point the Points page should list, newest first: the
    /// three sources interleaved, narrowed by the page's source filter and its
    /// search box.
    ///
    /// The filtering lives here rather than on the page so the page is a list
    /// of widgets and this is a query over state. `query` arrives already
    /// trimmed and lowercased, which is the form [`TrackPoint::matches`] takes.
    pub(crate) fn visible_points(&self, filter: PointFilter, query: &str) -> Vec<TrackPoint> {
        let remote_points = self.remotes.values().flat_map(|n| n.track.iter());
        let mut rows: Vec<TrackPoint> = self
            .track
            .iter()
            .chain(self.beacon_track.iter())
            .chain(remote_points)
            .filter(|p| filter.admits(p.source))
            .filter(|p| query.is_empty() || p.matches(query))
            .copied()
            .collect();
        // Newest first; every track interleaves by record time.
        rows.sort_by_key(|p| std::cmp::Reverse(p.time));
        rows
    }

    /// How many points are recorded across every track: yours, the beacon's
    /// and the nodes'.
    pub(crate) fn recorded_points(&self) -> usize {
        let remote: usize = self.remotes.values().map(|n| n.track.len()).sum();
        self.track.len() + self.beacon_track.len() + remote
    }

    /// Drop every recorded track. Not undoable, which is why the button behind
    /// it lives on the Settings page rather than in the map bar.
    pub(crate) fn discard_tracks(&mut self) {
        self.track.clear();
        self.beacon_track.clear();
        for node in self.remotes.values_mut() {
            node.track.clear();
        }
    }

    /// Every remote node worth listing on the Status page: those with a
    /// position or a time last heard, each with how it is doing and the signal
    /// it came in at.
    ///
    /// A node with no position is listed too, with why it has none: a node
    /// searching for the sky is a node that is up, and leaving it out makes it
    /// indistinguishable from one that is out of range or dead.
    pub(crate) fn remote_states(&self) -> Vec<(u8, String, i16)> {
        self.remotes
            .iter()
            .filter(|(_, n)| n.pos.is_some() || n.heard.is_some())
            .map(|(&addr, n)| (addr, n.state_text(), n.rssi))
            .collect()
    }

    /// Whether the board is still within its post-connect warm-up. The
    /// GPS/LoRa rail is off through sleep and through each wake window and
    /// comes up only once a central connects, so the WIO has to boot and the
    /// GPS has to make a cold fix before there is anything to report - an
    /// empty read-out just after connecting is that, not a fault.
    pub(crate) fn board_warming(&self) -> bool {
        self.connected_at
            .is_some_and(|t| t.elapsed() < BOARD_WARMUP)
    }

    /// Whether the board says its GPS/LoRa power rail is switched off, which
    /// leaves the WIO-E5 and the GPS unpowered and reporting nothing.
    pub(crate) fn rail_off(&self) -> bool {
        self.board_settings.is_some_and(|s| !s.pwr_en)
    }

    /// Parse a whole number out of one of the board-setting text boxes, or
    /// answer the ack line with what it should have been.
    ///
    /// The refusal goes to the same line the board's own answer would, so a
    /// typo and a rejection read the same way and in the same place.
    fn parse_setting(text: &str, unit: &str) -> Result<u32, String> {
        text.trim()
            .parse::<u32>()
            .map_err(|_| format!("Enter a whole number of {unit}."))
    }

    /// Send the typed BLE notify interval to the board.
    pub(crate) fn apply_notify_interval(&mut self) {
        match Self::parse_setting(&self.ble_interval_text, "milliseconds") {
            Ok(ms) => self.send_config(ConfigWrite::Interval(ms)),
            Err(msg) => self.ble_ack = Some(Err(msg)),
        }
    }

    /// Send the typed deep-sleep wake interval to the board. `secs` of 0
    /// disables sleeping, which is what the Disable button sends.
    pub(crate) fn apply_sleep_interval(&mut self, secs: Option<u32>) {
        let secs = match secs {
            Some(secs) => Ok(secs),
            None => Self::parse_setting(&self.sleep_interval_text, "seconds"),
        };
        match secs {
            Ok(secs) => self.send_config(ConfigWrite::Seconds {
                id: ble::CFG_ESP_SLEEP_S,
                secs,
            }),
            Err(msg) => self.ble_ack = Some(Err(msg)),
        }
    }

    /// Send the typed advertising window to the board.
    ///
    /// No zero here, unlike the wake interval: a zero-length window would
    /// leave a sleeping board unreachable by anything short of a physical
    /// reset, so the board clamps 0 up to its floor rather than storing it.
    pub(crate) fn apply_adv_window(&mut self) {
        match Self::parse_setting(&self.adv_window_text, "seconds") {
            Ok(secs) => self.send_config(ConfigWrite::Seconds {
                id: ble::CFG_ESP_ADV_WINDOW_S,
                secs,
            }),
            Err(msg) => self.ble_ack = Some(Err(msg)),
        }
    }

    /// Tell the board to deep sleep now.
    ///
    /// A blank box is a zero, which the firmware reads as "for the
    /// configured wake-check interval" and resolves itself. The predicted
    /// duration is recorded here through the same `resolve_sleep_now` the
    /// firmware uses, so the message shown while the ack is still in flight
    /// cannot disagree with the one shown after it lands.
    pub(crate) fn apply_sleep_now(&mut self) {
        let typed = self.sleep_now_text.trim();
        let secs = if typed.is_empty() {
            Ok(0)
        } else {
            Self::parse_setting(typed, "seconds")
        };
        match secs {
            Ok(secs) => {
                let cadence = self.board_settings.map_or(0, |s| s.sleep_interval_s);
                self.sleep_commanded = Some(ble::resolve_sleep_now(secs, cadence));
                self.send_config(ConfigWrite::Seconds {
                    id: ble::CFG_SLEEP_NOW,
                    secs,
                });
            }
            Err(msg) => self.ble_ack = Some(Err(msg)),
        }
    }

    /// While tracking a beacon, recenter the map between the user and that
    /// beacon and pick a zoom that keeps both on screen with a margin. Returns
    /// the bearing (degrees) the map should be turned to so the beacon rides
    /// near the top and the user near the bottom, or `None` when not tracking
    /// or a position is missing.
    fn tracking_orientation(&mut self, ctx: &egui::Context, screen: egui::Rect) -> Option<f32> {
        let kind = self.tracking_beacon?;
        // Tracking needs both the user position and the chosen beacon. If either
        // is gone (no fix yet, beacon disconnected, the node forgotten on a
        // board switch), leave tracking mode and return `None` so the map
        // unlocks instead of freezing on a view it can no longer manage.
        let (Some(user), Some((_, beacon))) = (self.current, self.beacon_target(kind)) else {
            self.tracking_beacon = None;
            return None;
        };

        // Center between the two so, once the map is turned to put the beacon
        // straight up, they sit symmetrically about the middle of the screen.
        let mid = lat_lon(
            (user.y() + beacon.y()) / 2.0,
            (user.x() + beacon.x()) / 2.0,
        );
        self.map_memory.center_at(mid);

        // Fit: scale the zoom so the on-screen separation fills the vertical
        // span left after the top/bottom margins. Mercator pixels double per
        // zoom step, so the needed change is log2(want / have). Eased toward the
        // target so entering the mode glides rather than snaps.
        let projector = Projector::new(screen, &self.map_memory, mid);
        let user_px = projector.project(user).to_pos2();
        let beacon_px = projector.project(beacon).to_pos2();
        let have = (beacon_px - user_px).length() as f64;
        let want = (screen.height() * (1.0 - 2.0 * TRACK_MARGIN_FRAC)) as f64;
        if have > 1.0 && want > 1.0 {
            let current = self.map_memory.zoom();
            let target = (current + (want / have).log2()).clamp(TRACK_ZOOM_MIN, TRACK_ZOOM_MAX);
            let dt = ctx.input(|i| i.stable_dt).clamp(0.0, 0.1) as f64;
            let alpha = 1.0 - (-dt / 0.12).exp();
            let _ = self.map_memory.set_zoom(current + (target - current) * alpha);
            if (target - current).abs() > 0.01 {
                ctx.request_repaint();
            }
        }

        Some(bearing_deg(user, beacon))
    }

    /// Apply one phone/manual GPS fix: move the marker, update the heading, and
    /// append to the recorded track (decimated by the min-distance setting).
    fn apply_gps_fix(&mut self, fix: GpsFix) {
        let pos = lat_lon(fix.lat, fix.lon);
        self.current = Some(pos);
        self.current_time = Some(SystemTime::now());
        self.heading = fix.bearing;
        if far_enough(
            self.track.last().map(|t| &t.pos),
            pos,
            self.config.track.min_distance,
        ) {
            self.track.push(TrackPoint {
                pos,
                source: PointSource::Phone,
                time: SystemTime::now(),
            });
        }
        // Logged on every fix, not only the ones far enough apart to become
        // track points: the track is a drawn path and wants decimating, the
        // log is a record and wants the samples.
        let mut row = LogRow::new(LogSource::Phone, SystemTime::now());
        row.lat = Some(fix.lat);
        row.lon = Some(fix.lon);
        row.course_deg = fix.bearing.map(f64::from);
        row.fix = Some(true);
        self.record(row);
    }

    /// Pull every pending fix out of the channels, updating the current
    /// position, the beacon, and their tracks.
    fn drain_sources(&mut self) {
        while let Some(fix) = self.gps_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.apply_gps_fix(fix);
        }

        // A compass thread that could not start (no rotation-vector sensor on
        // this device) drops its sender. Forgetting the handle then hides the
        // heading-up button, which is keyed off the handle existing rather than
        // off a live reading - the sensor being off is the normal state.
        let mut compass_gone = false;
        if let Some(compass) = &self.compass {
            loop {
                match compass.headings.try_recv() {
                    Ok(heading) => self.compass_heading = Some(heading),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        compass_gone = true;
                        break;
                    }
                }
            }
        }
        if compass_gone {
            self.compass = None;
        }

        while let Ok(update) = self.ble.events.try_recv() {
            // The tail of a session the user has already moved on from: a fix
            // from the board just disconnected from, or a settings blob from
            // the board just switched away from. Both would otherwise land as
            // if they described what is selected now. Epoch 0 is the worker
            // speaking for itself before any request reached it (no adapter,
            // no Bluetooth), which is always worth hearing.
            if update.epoch != 0 && update.epoch < self.ble_epoch {
                continue;
            }
            let event = update.event;
            // Anything decoded off a characteristic proves the board itself is
            // still talking. Status, scan sightings and link-state changes are
            // the worker's own and say nothing about the connected board.
            match &event {
                BleEvent::Status(_) | BleEvent::Discovered(_) | BleEvent::Connected(_) => {}
                _ => self.board_heard = Some(Instant::now()),
            }
            match event {
                BleEvent::Status(s) => self.ble_status = s,
                BleEvent::Discovered(device) => {
                    self.discovered.insert(
                        normalize_mac(&device.address),
                        Seen {
                            rssi: device.rssi,
                            at: Instant::now(),
                        },
                    );
                }
                BleEvent::Connected(c) => {
                    // A drop restarts the "connecting for ..." clock: the
                    // retry begins now, not back when connecting was first
                    // asked for. Only the transition counts - the worker
                    // repeats Connected(false) on every retry cycle, and
                    // resetting on each would pin the count near zero.
                    if self.ble_connected && !c {
                        self.intent_since = Instant::now();
                    }
                    self.ble_connected = c;
                    self.connected_at = c.then(Instant::now);
                    self.board_heard = c.then(Instant::now);
                    // A commanded sleep is answered by the link going away,
                    // so the disconnect is the confirmation rather than a
                    // fault. Replacing the ack here matters because the ack
                    // line is the only thing on the page still describing
                    // the press, and "Board sleeping, it will disconnect
                    // now" left standing next to a dropped link reads as the
                    // sleep having failed to happen.
                    if let Some(secs) = self.sleep_commanded.take().filter(|_| !c) {
                        self.ble_ack = Some(Ok(format!(
                            "Board asleep for {} - it is unreachable until it wakes",
                            secs_text(secs)
                        )));
                        self.ble_ack_pending = false;
                    }
                    if c {
                        // A fresh link re-reads everything below; nothing from
                        // the last session still describes the board.
                        //
                        // The intent is left alone: it says what to do when
                        // there is no link, so a board that sleeps again should
                        // still be chased if that is what was asked for.
                        self.board_settings = None;
                        self.settings_unsupported = false;
                        self.telemetry = None;
                        self.sleep_commanded = None;
                    }
                }
                BleEvent::Fix(p) => {
                    self.beacon_packet = Some(p);
                    if p.has_fix() {
                        let pos = lat_lon(p.lat_deg(), p.lon_deg());
                        self.beacon = Some(pos);
                        self.beacon_time = Some(SystemTime::now());
                        if far_enough(
                            self.beacon_track.last().map(|t| &t.pos),
                            pos,
                            self.config.track.min_distance,
                        ) {
                            self.beacon_track.push(TrackPoint {
                                pos,
                                source: PointSource::Esp,
                                time: SystemTime::now(),
                            });
                        }
                    }
                    self.record(packet_row(LogSource::Board, p, SystemTime::now()));
                }
                BleEvent::Remote { src, rssi, packet, age_s } => {
                    // Bucket by address so each node keeps its own path.
                    let min_distance = self.config.track.min_distance;
                    // The board notifies once per report and says how long ago
                    // it heard it, so every event is a report that happened -
                    // a repeated position means a stationary node, not the
                    // relay resending its cache. Distance is what decides
                    // whether it becomes a track point, as for our own fixes.
                    let at = heard_at(age_s);
                    let node = self.remotes.entry(src).or_default();
                    node.rssi = rssi;
                    node.heard = Some(at);
                    node.no_fix = None;
                    if packet.has_fix() {
                        node.packet = packet;
                        let pos = lat_lon(packet.lat_deg(), packet.lon_deg());
                        node.pos = Some(pos);
                        node.time = Some(at);
                        if far_enough(node.track.last().map(|t| &t.pos), pos, min_distance) {
                            node.track.push(TrackPoint {
                                pos,
                                source: PointSource::Remote(src),
                                time: at,
                            });
                        }
                    }
                    // The row the distance-against-signal plot is made of: a
                    // node's report carries where it was and how strongly the
                    // relay heard it, both as of the same moment.
                    let mut row = packet_row(LogSource::Node(src), packet, at);
                    row.rssi_dbm = Some(rssi);
                    self.record(row);
                }
                BleEvent::NodePing(ping) => {
                    // A node on the air with no position to give. Its last
                    // known position stays on the map - it is still the last
                    // place the node was - but the node is now flagged as
                    // having no fix, so the marker is not read as current.
                    let node = self.remotes.entry(ping.src).or_default();
                    node.rssi = ping.rssi;
                    node.heard = Some(heard_at(ping.age_s));
                    node.no_fix = Some(ping);
                    // A ping is a range check with no range: the signal is as
                    // real as a position report's, so the row is worth having
                    // even with the distance columns empty.
                    let mut row = LogRow::new(LogSource::Node(ping.src), heard_at(ping.age_s));
                    row.rssi_dbm = Some(ping.rssi);
                    row.fix = Some(false);
                    self.record(row);
                }
                BleEvent::Ack(ack) => {
                    self.ble_ack_pending = false;
                    self.ble_ack = Some(ack_message(&ack));
                }
                BleEvent::Telemetry(t) => {
                    self.telemetry = Some(t);
                    let mut row = LogRow::new(LogSource::Telemetry, SystemTime::now());
                    row.rssi_dbm = Some(t.last_rssi);
                    // Centibels in the wire format, decibels in the log: the
                    // column is what a plot reads, and every other unit there
                    // is the one it is named in.
                    row.snr_db = Some(f64::from(t.last_snr_cb) / 100.0);
                    row.sats = Some(t.sats);
                    row.rx_count = Some(t.rx_count);
                    row.tx_count = Some(t.tx_count);
                    row.secs_since_rx = Some(t.secs_since_rx);
                    self.record(row);
                }
                BleEvent::Log(s) => self.board_log = Some(s),
                BleEvent::Settings(s) => {
                    // Seed the inputs from the board's own values the first
                    // time it reports them, so the boxes open on what it is
                    // actually set to. Later reports only move the controls
                    // that mirror the board (the checkboxes and the "Board:"
                    // lines), leaving anything half-typed alone.
                    if self.board_settings.is_none() {
                        self.ble_interval_text = s.notify_interval_ms.to_string();
                        if s.sleep_interval_s > 0 {
                            self.sleep_interval_text = s.sleep_interval_s.to_string();
                        }
                        // The board reports the window it resolved, so a 0 here
                        // would be a board that does not know its own effective
                        // value; keep the default rather than show it.
                        if s.adv_window_s > 0 {
                            self.adv_window_text = s.adv_window_s.to_string();
                        }
                    }
                    self.board_settings = Some(s);
                    self.settings_unsupported = false;
                }
                BleEvent::SettingsUnsupported => {
                    self.board_settings = None;
                    self.settings_unsupported = true;
                }
                BleEvent::RadioConfig(c) => {
                    self.board_radio_config = Some(c);
                    self.radio_config_unsupported = false;
                }
                BleEvent::RadioConfigUnsupported => {
                    self.board_radio_config = None;
                    self.radio_config_unsupported = true;
                }
                BleEvent::ConfigPushed(res) => {
                    self.radio_push_pending = false;
                    self.radio_feedback = Some(res.map_err(|e| format!("Send failed: {e}")));
                }
            }
        }

        // Offline center-button fallback: apply the zoom the probe picked (the
        // nearest level with a cached tile). Latest wins if several arrived.
        while let Ok(zoom) = self.zoom_rx.try_recv() {
            let _ = self.map_memory.set_zoom(zoom);
        }
    }
}

impl eframe::App for MyApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.drain_sources();
        // Heading-up may have been toggled (or dropped for want of a heading)
        // last frame; the sensor follows it here, once, for every page.
        self.sync_compass_power();

        let ctx = ui.ctx().clone();
        // Before anything is drawn: the pages read these colors and text sizes
        // out of the style rather than being handed them.
        self.apply_ui_style(&ctx);
        let screen = ctx.input(|i| i.viewport_rect());

        match self.page {
            Page::Menu => self.menu_page(&ctx, screen),
            Page::Map => self.map_page(&ctx, screen),
            Page::Points => self.points_page(&ctx, screen),
            Page::Status => self.status_page(&ctx, screen),
            Page::Beacon => self.beacon_page(&ctx, screen),
            Page::Settings => self.settings_page(&ctx, screen),
            Page::Radio => self.radio_page(&ctx, screen),
            Page::Logging => self.logging_page(&ctx, screen),
        }

        // Every page but the map gets the floating corner toggle; on the map
        // page the toggle lives at the right end of the controls bar instead.
        // On the menu page it is the X that closes the menu again.
        if !matches!(self.page, Page::Map) {
            self.page_toggle(&ctx, screen);
        }
        // Offline download progress floats above every page too.
        self.download_ui(&ctx, screen);

        // With no live GPS source (desktop), let a position be typed in. Only
        // on the map, where the bar can float at the bottom without landing on
        // top of a scrolling page.
        if self.gps_rx.is_none() && matches!(self.page, Page::Map) {
            self.manual_gps_bar(&ctx, screen);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ble::{BleUpdate, Inbox, Interrupt, Reporter, Wanted};
    use std::sync::mpsc::channel;

    /// An app wired to a worker that is not there: the test reads the command
    /// channel and pushes events back by hand. That is the whole of the BLE
    /// contract from the UI's side, so the buttons can be driven without a
    /// radio.
    fn test_app() -> (MyApp, Receiver<BleRequest>, Sender<BleUpdate>) {
        let (event_tx, event_rx) = channel();
        let (cmd_tx, cmd_rx) = channel();
        // A cache directory under the system temp dir is what keeps the
        // startup config auto-load away from any real gps-config.toml: the
        // app starts on its defaults rather than on the working directory's.
        let cache = std::env::temp_dir().join("gps-gui-rs-tests").join("tiles");
        let app = MyApp::new(
            egui::Context::default(),
            None,
            Some(cache),
            None,
            None,
            BleHandle {
                events: event_rx,
                commands: cmd_tx,
            },
            None,
        );
        (app, cmd_rx, event_tx)
    }

    /// A board's position report.
    fn fix() -> PositionPacket {
        PositionPacket {
            lat_e7: 481_173_000,
            lon_e7: -1_226_760_000,
            flags: packet::FLAG_FIX,
            sats: 7,
            ..PositionPacket::default()
        }
    }

    /// Everything the connected board has told us, as the pages read it.
    fn board_state(app: &MyApp) -> (bool, bool, bool, bool) {
        (
            app.ble_connected,
            app.beacon.is_some(),
            app.board_settings.is_some(),
            app.board_log.is_some(),
        )
    }

    /// Feed one event as the worker would, under the request the app is
    /// currently making.
    fn report(events: &Sender<BleUpdate>, app: &MyApp, event: BleEvent) {
        events
            .send(BleUpdate {
                epoch: app.ble_epoch,
                event,
            })
            .expect("the app holds the receiver");
    }

    /// Disconnect is not a request to stop eventually: as of the press there
    /// is no link, and nothing the board said is still on the pages.
    #[test]
    fn disconnect_drops_the_link_and_the_board_state() {
        let (mut app, cmds, events) = test_app();
        let session = app.ble_epoch;

        report(&events, &app, BleEvent::Connected(true));
        report(&events, &app, BleEvent::Fix(fix()));
        report(&events, &app, BleEvent::Settings(Settings::default()));
        report(&events, &app, BleEvent::Log("wio: ok".to_string()));
        app.drain_sources();
        assert_eq!(board_state(&app), (true, true, true, true));

        app.set_ble_intent(BleIntent::Idle);
        assert_eq!(
            board_state(&app),
            (false, false, false, false),
            "the press itself ends the link and drops what the board said"
        );
        assert!(app.connected_at.is_none() && app.board_heard.is_none());

        // The tail of that session - already on its way when the button was
        // pressed - must not put any of it back.
        let stale = |event| {
            events
                .send(BleUpdate {
                    epoch: session,
                    event,
                })
                .unwrap()
        };
        stale(BleEvent::Connected(true));
        stale(BleEvent::Fix(fix()));
        stale(BleEvent::Settings(Settings::default()));
        app.drain_sources();
        assert_eq!(board_state(&app), (false, false, false, false));

        // And the worker was told, as one command.
        let sent: Vec<_> = cmds.try_iter().collect();
        assert!(matches!(
            sent.last(),
            Some(BleRequest {
                command: BleCommand::Disconnect,
                ..
            })
        ));
    }

    /// The reported bug, from the press through to what the worker does with
    /// it: Disconnect while connecting to one board, then connect to another.
    /// The session already running for the first board has to end, or the app
    /// connects to it and only then notices it wanted the other one.
    #[test]
    fn disconnect_then_another_board_repoints_the_worker() {
        let (mut app, cmds, _events) = test_app();
        app.select_device(Some("AA:BB:CC:DD:EE:01"));
        app.set_ble_intent(BleIntent::Connect);

        // The worker takes the request and starts a session for it.
        let (event_tx, _worker_events) = channel();
        let reporter = Reporter::new(egui::Context::default(), event_tx);
        let mut wanted = Wanted::idle();
        let mut writes = Vec::new();
        let mut push = None;
        let mut inbox = Inbox {
            rx: &cmds,
            wanted: &mut wanted,
            writes: &mut writes,
            push: &mut push,
        };
        assert!(inbox.drain(&reporter).is_ok());
        let first = inbox.wanted.target();
        assert_eq!(first.mac.as_deref(), Some("AA:BB:CC:DD:EE:01"));

        // Disconnect part-way through, then pick a different board and
        // connect to that.
        app.set_ble_intent(BleIntent::Idle);
        app.select_device(Some("AA:BB:CC:DD:EE:02"));
        app.set_ble_intent(BleIntent::Connect);

        assert!(inbox.drain(&reporter).is_ok());
        assert_eq!(inbox.wanted.interrupt(&first), Some(Interrupt::Superseded));
        assert_eq!(
            inbox.wanted.target().mac.as_deref(),
            Some("AA:BB:CC:DD:EE:02"),
            "the next session goes to the board that was asked for"
        );
    }

    /// Connect pressed while connected is a real request - start over - and
    /// the way out of a link that is up but has stopped working.
    #[test]
    fn connect_while_connected_starts_over() {
        let (mut app, cmds, events) = test_app();
        report(&events, &app, BleEvent::Connected(true));
        report(&events, &app, BleEvent::Settings(Settings::default()));
        app.drain_sources();
        let connected_epoch = app.ble_epoch;
        let _ = cmds.try_iter().count();

        app.set_ble_intent(BleIntent::Connect);
        assert!(app.ble_epoch > connected_epoch, "a new request");
        assert_eq!(
            board_state(&app),
            (false, false, false, false),
            "the link is dropped and re-read rather than kept"
        );
        let sent: Vec<_> = cmds.try_iter().collect();
        assert_eq!(sent.len(), 1, "one command per press, never composed");
        assert!(matches!(
            sent[0].command,
            BleCommand::Connect { chase: false, .. }
        ));
        assert_eq!(sent[0].epoch, app.ble_epoch);
    }

    /// Picking another board while connected switches to it there and then:
    /// only one board is ever connected, so the choice is the switch.
    #[test]
    fn picking_another_board_while_connected_switches_now() {
        let (mut app, cmds, events) = test_app();
        report(&events, &app, BleEvent::Connected(true));
        report(&events, &app, BleEvent::Fix(fix()));
        app.drain_sources();
        let _ = cmds.try_iter().count();

        app.select_device(Some("AA:BB:CC:DD:EE:02"));
        assert!(!app.ble_connected);
        assert!(app.beacon.is_none(), "the old board's fix is not the new board's");
        let sent: Vec<_> = cmds.try_iter().collect();
        assert!(matches!(
            &sent.last().expect("a switch is sent at once").command,
            BleCommand::Connect { mac, .. } if mac.as_deref() == Some("AA:BB:CC:DD:EE:02")
        ));
    }

    /// A page waiting on the link is answered by the press that drops it,
    /// rather than by an event from the session it was riding - which is no
    /// longer coming, or would be fenced out if it did.
    #[test]
    fn disconnecting_mid_push_answers_the_radio_page() {
        let (mut app, _cmds, _events) = test_app();
        app.radio_push_pending = true;

        app.set_ble_intent(BleIntent::Idle);
        assert!(!app.radio_push_pending);
        assert!(matches!(app.radio_feedback, Some(Err(_))));
    }

    /// A scan is not a link, so starting one clears the connection state as
    /// firmly as a disconnect does - and empties the picker, which is about
    /// what is on the air now.
    #[test]
    fn scanning_drops_the_link() {
        let (mut app, _cmds, events) = test_app();
        report(&events, &app, BleEvent::Connected(true));
        report(&events, &app, BleEvent::Fix(fix()));
        report(
            &events,
            &app,
            BleEvent::Discovered(crate::ble::DiscoveredDevice {
                address: "AA:BB:CC:DD:EE:01".to_string(),
                name: None,
                rssi: Some(-60),
            }),
        );
        app.drain_sources();
        assert!(!app.discovered.is_empty());

        app.set_ble_intent(BleIntent::Scanning);
        assert!(!app.ble_connected);
        assert!(app.beacon.is_none());
        assert!(app.discovered.is_empty());
    }

    /// Reading 0 as "off" is what the interval read-outs want, and is exactly
    /// wrong for an elapsed time - the wake-mode line clamps off zero for it.
    #[test]
    fn secs_text_scales_and_calls_zero_off() {
        assert_eq!(secs_text(0), "off");
        assert_eq!(secs_text(1), "1 s");
        assert_eq!(secs_text(45), "45 s");
        assert_eq!(secs_text(60), "1 min");
        assert_eq!(secs_text(900), "15 min");
        assert_eq!(secs_text(3600), "1 h");
        assert_eq!(secs_text(43200), "12 h");
        // Not a whole number of hours, so it keeps a decimal.
        assert_eq!(secs_text(45000), "12.5 h");
    }

    /// The disconnect after a commanded sleep is the command working. The app
    /// has to say so, because the ack line is the only thing on the page
    /// still describing the press and "it will disconnect now" left standing
    /// beside a dead link reads as the sleep having failed.
    #[test]
    fn a_commanded_sleep_makes_the_disconnect_the_answer() {
        let (mut app, _cmds, events) = test_app();
        report(&events, &app, BleEvent::Connected(true));
        report(
            &events,
            &app,
            BleEvent::Settings(Settings {
                sleep_interval_s: 120,
                ..Settings::default()
            }),
        );
        app.drain_sources();

        // Blank box: the duration comes from the board's own cadence.
        app.sleep_now_text.clear();
        app.apply_sleep_now();
        assert_eq!(app.sleep_commanded, Some(120));
        assert!(app.ble_ack_pending);

        report(&events, &app, BleEvent::Connected(false));
        app.drain_sources();
        let msg = app.ble_ack.clone().expect("a sleep is answered").unwrap();
        assert!(msg.contains("2 min"), "{msg}");
        assert!(!app.ble_ack_pending, "the press is finished, not pending");
        assert_eq!(app.sleep_commanded, None, "and does not answer twice");
    }

    /// A link that drops on its own is still a fault. Only a sleep this app
    /// asked for gets to reinterpret a disconnect.
    #[test]
    fn an_uncommanded_disconnect_is_not_reported_as_a_sleep() {
        let (mut app, _cmds, events) = test_app();
        report(&events, &app, BleEvent::Connected(true));
        app.drain_sources();
        report(&events, &app, BleEvent::Connected(false));
        app.drain_sources();
        assert!(app.ble_ack.is_none());
    }

    /// The duration the app shows before the ack and the one the board
    /// applies are the same function, so a typed value and a blank box both
    /// predict what actually happens.
    #[test]
    fn the_sleep_duration_shown_is_the_one_the_board_will_use() {
        let (mut app, _cmds, events) = test_app();
        report(&events, &app, BleEvent::Connected(true));
        report(&events, &app, BleEvent::Settings(Settings::default()));
        app.drain_sources();

        // Sleep mode off and a blank box: the shared fallback, not zero.
        app.sleep_now_text.clear();
        app.apply_sleep_now();
        assert_eq!(app.sleep_commanded, Some(ble::SLEEP_NOW_DEFAULT_S));

        // A typed value below the floor comes up to it, as the board does.
        app.sleep_now_text = "1".to_string();
        app.apply_sleep_now();
        assert_eq!(app.sleep_commanded, Some(ble::ESP_SLEEP_MIN_S));
    }

    /// The WIO statuses are a link failure between the ESP and the WIO-E5, not
    /// a rejected value, and have to read as something the user can act on.
    #[test]
    fn ack_message_separates_wio_faults_from_rejections() {
        let ack = |id, status| Ack {
            id,
            status,
            value_u32: None,
        };
        assert!(ack_message(&Ack {
            id: ble::CFG_ESP_SLEEP_S,
            status: packet::ACK_OK,
            value_u32: Some(300),
        })
        .unwrap()
        .contains("5 min"));

        let wio = ack_message(&ack(ble::CFG_WIO_SLEEP, ble::ACK_WIO_TIMEOUT)).unwrap_err();
        assert!(wio.contains("WIO-E5"), "{wio}");
        let bad = ack_message(&ack(ble::CFG_PWR_EN, packet::ACK_BAD_VALUE)).unwrap_err();
        assert!(bad.contains("GPS/LoRa power"), "{bad}");
        // An interval of 0 turns sleep off, and must not read as "every off".
        assert_eq!(
            ack_message(&Ack {
                id: ble::CFG_ESP_SLEEP_S,
                status: packet::ACK_OK,
                value_u32: Some(0),
            }),
            Ok("Board applied: sleep disabled".to_string())
        );
    }

    /// A setting that carries a number has to say which number the board
    /// took. The advertising window had neither a name nor a value arm, so
    /// every window write acked as the literal "Board applied: setting" - and
    /// a clamped window was indistinguishable from one applied as asked.
    #[test]
    fn ack_message_quotes_the_window_the_board_stored() {
        let acked = |secs| {
            ack_message(&Ack {
                id: ble::CFG_ESP_ADV_WINDOW_S,
                status: packet::ACK_OK,
                value_u32: Some(secs),
            })
        };
        assert_eq!(
            acked(ble::ESP_ADV_MIN_S),
            Ok(format!(
                "Board applied: advertising {} per wake",
                secs_text(ble::ESP_ADV_MIN_S)
            ))
        );
        // The clamped case is the one that matters: asking for less than the
        // floor has to read as the board's number, not as agreement.
        assert!(acked(30).unwrap().contains("30 s"));
        assert!(ack_message(&Ack {
            id: ble::CFG_ESP_ADV_WINDOW_S,
            status: packet::ACK_BAD_VALUE,
            value_u32: None,
        })
        .unwrap_err()
        .contains("advertising window"));
    }

    /// The distance every range reading in the log and every label on the map
    /// is measured with. Checked against published figures rather than against
    /// itself, so an error in the formula cannot agree with the test.
    #[test]
    fn haversine_matches_known_distances() {
        let close = |got: f64, want: f64| {
            assert!(
                (got - want).abs() / want < 0.005,
                "got {got} m, wanted about {want} m"
            );
        };
        // A degree of latitude is about 111.2 km anywhere.
        close(haversine_m(lat_lon(0.0, 0.0), lat_lon(1.0, 0.0)), 111_195.0);
        // A degree of longitude shortens with the cosine of the latitude.
        close(haversine_m(lat_lon(60.0, 0.0), lat_lon(60.0, 1.0)), 55_597.0);
        // London to Paris.
        close(
            haversine_m(lat_lon(51.5074, -0.1278), lat_lon(48.8566, 2.3522)),
            343_600.0,
        );
        // A point is no distance from itself, and the measure is symmetric.
        let (a, b) = (lat_lon(51.4779, -0.0015), lat_lon(48.8566, 2.3522));
        assert_eq!(haversine_m(a, a), 0.0);
        assert_eq!(haversine_m(a, b), haversine_m(b, a));
    }

    /// Two points on opposite sides of the earth. Nothing in the app puts a
    /// node there on purpose, but a garbled packet can decode to any
    /// coordinate at all, and a NaN escaping into the log or the tracking
    /// zoom is far worse than a wrong number: it poisons every comparison it
    /// reaches.
    ///
    /// The last pair is the case that made this necessary: near-antipodal
    /// coordinates drive the haversine's `h` a few ulps past 1, where `asin`
    /// is undefined.
    #[test]
    fn antipodal_points_have_a_finite_distance() {
        for (a, b) in [
            (lat_lon(0.0, 0.0), lat_lon(0.0, 180.0)),
            (lat_lon(90.0, 0.0), lat_lon(-90.0, 0.0)),
            (lat_lon(51.4779, -0.0015), lat_lon(-51.4779, 179.9985)),
            (
                lat_lon(-66.541_632_612_071_34, -140.469_191_196_212_96),
                lat_lon(66.541_632_611_688_09, 39.530_808_804_097_774),
            ),
        ] {
            let d = haversine_m(a, b);
            assert!(d.is_finite(), "{a:?} -> {b:?} gave {d}");
            // Half the circumference, give or take the earth's shape.
            assert!((d - 20_015_000.0).abs() < 2_000.0, "{d} m");
        }
    }

    /// Tracking mode turns the map by this, so a wrong quadrant points the
    /// user backwards.
    #[test]
    fn bearing_points_the_right_way() {
        let origin = lat_lon(51.0, 0.0);
        let close = |got: f32, want: f32| {
            let delta = (got - want + 540.0).rem_euclid(360.0) - 180.0;
            assert!(delta.abs() < 0.5, "got {got} deg, wanted {want}");
        };
        close(bearing_deg(origin, lat_lon(52.0, 0.0)), 0.0);
        close(bearing_deg(origin, lat_lon(51.0, 1.0)), 90.0);
        close(bearing_deg(origin, lat_lon(50.0, 0.0)), 180.0);
        close(bearing_deg(origin, lat_lon(51.0, -1.0)), 270.0);
        // Never negative: the callers feed this straight into a rotation.
        assert!((0.0..360.0).contains(&bearing_deg(origin, lat_lon(51.0, -0.5))));
    }

    /// What decimates a track: the first point always lands, and later ones
    /// only once they have moved far enough to be worth a segment.
    #[test]
    fn far_enough_decimates_a_track() {
        let a = lat_lon(51.4779, -0.0015);
        // About 11 m north.
        let b = lat_lon(51.477_9 + 0.000_1, -0.0015);
        assert!(far_enough(None, a, 10.0));
        assert!(!far_enough(Some(&a), a, 10.0));
        assert!(far_enough(Some(&a), b, 10.0));
        assert!(!far_enough(Some(&a), b, 20.0));
        // A zero minimum records everything, which is what turning decimation
        // off has to mean.
        assert!(far_enough(Some(&a), a, 0.0));
    }

    /// A node's uptime, which unlike an interval has to render 0 as a time.
    #[test]
    fn uptime_text_scales_without_calling_zero_off() {
        assert_eq!(uptime_text(0), "0 s");
        assert_eq!(uptime_text(59), "59 s");
        assert_eq!(uptime_text(60), "1 min");
        assert_eq!(uptime_text(3599), "59 min");
        assert_eq!(uptime_text(3600), "1 h");
        // The field saturates at 18 h, so this is the largest it ever reads.
        assert_eq!(uptime_text(u16::MAX), "18 h");
    }

    /// A silent GPS module is a wiring answer and a searching one is a sky
    /// answer; the whole point of the ping is telling them apart.
    #[test]
    fn ping_reason_separates_a_silent_module_from_a_search() {
        let ping = |gps_present, had_fix, uptime_s| NodePing {
            src: 3,
            rssi: -90,
            uptime_s,
            gps_present,
            had_fix,
            age_s: 0,
        };
        assert_eq!(ping_reason(ping(false, false, 120)), "gps silent");
        // A module that is talking is never called silent, even if it has
        // never had a fix.
        assert_eq!(ping_reason(ping(true, false, 120)), "no fix since boot, up 2 min");
        assert_eq!(ping_reason(ping(true, true, 120)), "searching, up 2 min");
    }

    /// A node the board replayed on connect belongs where it was heard, not at
    /// the moment it reached us: a ten-minute-old report entering the track as
    /// current would draw a path the node never took.
    #[test]
    fn heard_at_places_a_replayed_report_in_the_past() {
        let now = SystemTime::now();
        let live = heard_at(0);
        assert!(live.duration_since(now).unwrap_or_default() < Duration::from_secs(1));
        let replayed = heard_at(600);
        let age = now.duration_since(replayed).expect("in the past");
        assert!(
            age >= Duration::from_secs(599) && age <= Duration::from_secs(601),
            "{age:?}"
        );
        // The largest age the field can carry must still be a real time.
        assert!(heard_at(u16::MAX) < now);
    }

    /// A receiver that is up and searching is a different state from one that
    /// has stopped reporting, so a fixless packet still logs - with the
    /// coordinate columns empty rather than at the null island.
    #[test]
    fn packet_row_leaves_a_fixless_packet_without_coordinates() {
        let with_fix = packet_row(LogSource::Board, fix(), SystemTime::UNIX_EPOCH);
        assert_eq!(with_fix.fix, Some(true));
        assert_eq!(with_fix.sats, Some(7));
        assert!(with_fix.lat.is_some() && with_fix.lon.is_some());

        let searching = PositionPacket {
            sats: 2,
            ..PositionPacket::default()
        };
        let row = packet_row(LogSource::Node(4), searching, SystemTime::UNIX_EPOCH);
        assert_eq!(row.fix, Some(false));
        assert_eq!(row.sats, Some(2));
        assert_eq!(row.lat, None);
        assert_eq!(row.lon, None);
        assert_eq!(row.alt_m, None);
        assert_eq!(row.speed_mps, None);
    }

    /// The points page filter. Remote admits every address, since the filter is
    /// "any node" rather than a particular one.
    #[test]
    fn point_filter_admits_only_its_own_sources() {
        use PointSource::{Esp, Phone, Remote};
        for source in [Phone, Esp, Remote(1), Remote(255)] {
            assert!(PointFilter::All.admits(source));
        }
        assert!(PointFilter::Phone.admits(Phone));
        assert!(!PointFilter::Phone.admits(Esp));
        assert!(!PointFilter::Esp.admits(Remote(1)));
        assert!(PointFilter::Remote.admits(Remote(1)));
        assert!(PointFilter::Remote.admits(Remote(255)));
        assert!(!PointFilter::Remote.admits(Phone));
    }

    /// Give a node a position, as a relayed report would.
    fn place_node(app: &mut MyApp, addr: u8, lat: f64) {
        app.remotes.entry(addr).or_default().pos = Some(lat_lon(lat, 0.0));
    }

    /// The board being tracked must not change under the user.
    ///
    /// [`MyApp::beacon_targets`] is ordered - the connected board, then the
    /// nodes by address - and grows in the middle. Tracking used to hold an
    /// index into it, so the connected board getting a fix, or a
    /// lower-numbered node being heard, shifted every later entry along and
    /// silently handed tracking (and the distance read-out, and the line drawn
    /// on the map) to a different board.
    #[test]
    fn tracking_stays_on_the_board_it_was_pointed_at() {
        let (mut app, _cmds, _events) = test_app();
        let tracked = |app: &MyApp| app.distance_target().map(|(kind, _)| kind);

        app.current = Some(lat_lon(51.0, 0.0));
        place_node(&mut app, 5, 51.1);
        app.cycle_tracking();
        assert_eq!(tracked(&app), Some(MarkerKind::Remote(5)));

        // The connected board gets a fix of its own, which lists ahead of
        // every node.
        app.beacon = Some(lat_lon(52.0, 1.0));
        assert_eq!(tracked(&app), Some(MarkerKind::Remote(5)));

        // A lower-numbered node is heard, which lists ahead of node 5.
        place_node(&mut app, 3, 51.05);
        assert_eq!(tracked(&app), Some(MarkerKind::Remote(5)));

        // And the node the user is on going quiet does not promote a
        // neighbour into its place.
        app.remotes.remove(&5);
        assert_eq!(tracked(&app), None);
    }

    /// The track button walks the boards in list order and leaves the mode
    /// after the last one, so a single board is a plain on/off toggle.
    #[test]
    fn the_track_button_walks_every_board_and_then_exits() {
        let (mut app, _cmds, _events) = test_app();
        app.current = Some(lat_lon(51.0, 0.0));

        // Nothing to track: the mode cannot be entered at all.
        assert!(!app.can_track());
        app.cycle_tracking();
        assert_eq!(app.tracking_beacon, None);

        app.beacon = Some(lat_lon(51.2, 0.0));
        place_node(&mut app, 3, 51.3);
        place_node(&mut app, 9, 51.4);
        assert!(app.can_track());

        assert_eq!(app.tracking_hint(), "Track beacon");
        app.cycle_tracking();
        assert_eq!(app.tracking_beacon, Some(MarkerKind::Beacon));
        assert_eq!(app.tracking_hint(), "Next beacon");
        app.cycle_tracking();
        assert_eq!(app.tracking_beacon, Some(MarkerKind::Remote(3)));
        app.cycle_tracking();
        assert_eq!(app.tracking_beacon, Some(MarkerKind::Remote(9)));
        // The last one, so the next press is the way out.
        assert_eq!(app.tracking_hint(), "Exit tracking");
        app.cycle_tracking();
        assert_eq!(app.tracking_beacon, None);
    }

    /// Entering tracking turns heading-up off - the two both rotate the map -
    /// but a press that cannot enter must not turn it off for nothing.
    #[test]
    fn entering_tracking_leaves_heading_up() {
        let (mut app, _cmds, _events) = test_app();
        app.heading_up = true;
        // No board to track: nothing happens, heading-up included.
        app.cycle_tracking();
        assert!(app.heading_up);

        app.current = Some(lat_lon(51.0, 0.0));
        app.beacon = Some(lat_lon(51.2, 0.0));
        app.cycle_tracking();
        assert_eq!(app.tracking_beacon, Some(MarkerKind::Beacon));
        assert!(!app.heading_up);
    }

    /// A board that goes away while it is being tracked: the walk starts over
    /// rather than reading as "the last board", which would have the button
    /// say "exit" for something that is no longer on the list.
    #[test]
    fn tracking_a_board_that_disappears_starts_the_walk_over() {
        let (mut app, _cmds, _events) = test_app();
        app.current = Some(lat_lon(51.0, 0.0));
        place_node(&mut app, 7, 51.5);
        app.cycle_tracking();
        assert_eq!(app.tracking_beacon, Some(MarkerKind::Remote(7)));

        // Node 7 is forgotten and another board takes the air.
        app.remotes.remove(&7);
        app.beacon = Some(lat_lon(51.2, 0.0));
        assert_eq!(app.tracking_hint(), "Next beacon");
        app.cycle_tracking();
        assert_eq!(app.tracking_beacon, Some(MarkerKind::Beacon));
    }

    /// The config has to be saved somewhere writable. On Android the working
    /// directory is not, so the path is derived from the cache directory the
    /// platform handed over.
    #[test]
    fn default_config_path_lands_beside_the_cache() {
        assert_eq!(
            default_config_path(Some(std::path::Path::new("/data/app/files/tiles"))),
            "/data/app/files/gps-config.toml"
        );
        // No cache directory, or one with no parent to speak of: the bare
        // filename, which on desktop is the working directory.
        assert_eq!(default_config_path(None), DEFAULT_CONFIG_NAME);
        assert_eq!(
            default_config_path(Some(std::path::Path::new("tiles"))),
            DEFAULT_CONFIG_NAME
        );
    }
}

