//! BLE central for the GPS boards: the Wio-S3 (telemetry-in-midair-rs firmware) and the
//! older ESP32-C3 beacon it grew out of.
//!
//! Mirrors the gps source design: a background worker owns the platform BLE
//! stack and talks to the UI over channels. Events flow UI-ward (fixes, acks,
//! status), commands flow worker-ward (connect, config writes). Desktop uses
//! btleplug; Android drives the platform Bluetooth API through a small dex
//! shim loaded at runtime (see android.rs).
//!
//! The wire protocol lives in the shared gps-proto crate.

use std::cell::Cell;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::Duration;

use gps_proto::packet::{self, Ack};
use midair_proto::{ble, link, lora};

use crate::config::normalize_mac;
pub use gps_proto::packet::PositionPacket;
pub use midair_proto::ble::{Mode, Settings};
pub use midair_proto::link::Telemetry;
pub use midair_proto::radiocfg::RadioConfig;

#[cfg(not(target_os = "android"))]
mod desktop;

#[cfg(target_os = "android")]
mod android;

/// One board seen while scanning, for the Beacon page's device picker.
///
/// The address is still the identity - a name can be changed and a nickname in
/// the app config still wins over both - but `name` is no longer the same
/// string on every board: Wio-S3 firmware advertises `ws3gps-<label>`, and
/// falls back to the tail of its own address when nobody has named it. So a
/// board is legible in the picker before it has been nicknamed, and often
/// before it has been connected to at all. `rssi` is what distinguishes them
/// in the field: the board in your hand is the loud one.
#[derive(Clone, Debug, PartialEq)]
pub struct DiscoveredDevice {
    pub address: String,
    pub name: Option<String>,
    pub rssi: Option<i16>,
}

/// Which UI request a command or an event belongs to.
///
/// The UI bumps this on every press, so the worker can tell the request it is
/// already serving from a fresh press for the same thing - the difference
/// between ignoring a button and starting over. It rides back out on every
/// event, which is what lets the UI drop the tail of a session it has moved
/// on from: a fix from the board just disconnected from must not land on the
/// map as the newly selected board's.
///
/// Epoch 0 is "no request yet": whatever the worker says before it has drained
/// a single command (no adapter, no Bluetooth) carries it, and the UI never
/// fences that out.
pub type Epoch = u64;

/// UI -> worker: one request, tagged with the press it came from.
pub struct BleRequest {
    pub epoch: Epoch,
    pub command: BleCommand,
}

/// Worker -> UI: one event, tagged with the request the worker was serving
/// when it sent it.
pub struct BleUpdate {
    pub epoch: Epoch,
    pub event: BleEvent,
}

/// Worker -> UI.
pub enum BleEvent {
    /// Human-readable connection state for the Beacon page.
    Status(String),
    /// A board seen during a discovery scan. Sent repeatedly for the same
    /// board as the scan runs, so the signal strength stays current; the UI
    /// keys them by address.
    Discovered(DiscoveredDevice),
    /// Connection state changed; gates the config controls.
    Connected(bool),
    /// A decoded position packet from the connected board's own GPS.
    Fix(PositionPacket),
    /// A remote node's position, relayed over LoRa by the connected board and
    /// read off the remote characteristic. `src` is the originating LoRa
    /// address (1-255); the UI keeps a separate track per address. `rssi` is
    /// the LoRa signal the relay heard it at.
    ///
    /// One event per report the board heard, so a repeated position means the
    /// node is stationary rather than that the board resent its cache.
    /// `age_s` is how long before the notification the board heard it: 0 for a
    /// live report, higher for one replayed on connect.
    Remote {
        src: u8,
        rssi: i16,
        packet: PositionPacket,
        age_s: u16,
    },
    /// A remote node reporting that it is on the air without a fix. Delivered
    /// like [`BleEvent::Remote`] and carrying the same `age_s`, but there is
    /// no position in it - which is the point: a node searching for the sky is
    /// otherwise indistinguishable from one out of range or dead.
    NodePing(NodePing),
    /// A config ack: the device confirmed (or rejected) a setting.
    Ack(Ack),
    /// Board telemetry (LoRa link, GPS, SD) from the Wio-S3 board.
    Telemetry(Telemetry),
    /// The latest status/log line (ASCII) the board sent.
    Log(String),
    /// The board's own view of its power and sleep settings, read on connect
    /// and notified on every change - including changes the board makes by
    /// itself, such as clamping an interval. This, not the UI, is the
    /// authority on what the board is set to.
    Settings(Settings),
    /// The settings blob did not decode: the board's layout version is newer
    /// than this build knows. Its settings are unreadable, not defaulted.
    SettingsUnsupported,
    /// What the board calls itself, read on connect and notified whenever it
    /// is renamed. The name it advertises under, prefix included.
    ///
    /// Worth reading rather than taking from the scan: a board connected to by
    /// its pinned address may never have been scanned this session, and one
    /// renamed during the session keeps advertising the old name until its
    /// next advertising window.
    Name(String),
    /// The board's current radio configuration, read on connect and notified
    /// on every change. Lets the Radio page open on what the board is running
    /// rather than a local file the user has to hope matches.
    RadioConfig(RadioConfig),
    /// The radio-config blob did not decode: the board's layout is newer than
    /// this build knows. Distinct from "not reported yet" (an all-zero blob),
    /// which is not surfaced at all.
    RadioConfigUnsupported,
    /// A [`BleCommand::PushConfig`] finished: the board applied and stored the
    /// config, or why it did not.
    ConfigPushed(Result<String, String>),
}

/// A node heard over LoRa with no position to report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodePing {
    /// Originating LoRa address (1-255).
    pub src: u8,
    /// LoRa signal the relay heard it at, in dBm. A ping carries this as
    /// usefully as a position does, which is what makes one a range check.
    pub rssi: i16,
    /// Seconds since the node booted, saturating at 18 h. Against an earlier
    /// ping it also shows a node that rebooted.
    pub uptime_s: u16,
    /// The node's GPS module is producing NMEA. Clear means a silent module -
    /// usually an unpowered rail or wiring, not a receiver that cannot see
    /// the sky.
    pub gps_present: bool,
    /// The node held a fix at some point since it booted, so this is a fix
    /// lost rather than one never acquired.
    pub had_fix: bool,
    /// How long before the notification the board heard it; see
    /// [`BleEvent::Remote`].
    pub age_s: u16,
}

/// UI -> worker. Every command arrives inside a [`BleRequest`], and a request
/// newer than the one a session started for ends that session at the next step
/// it can - including a repeat of the command already running, which is what
/// makes a second press mean "start over" rather than nothing.
pub enum BleCommand {
    /// Start (or restart) connecting. `mac` pins a specific device; `None`
    /// scans for the first device advertising the GPS service.
    ///
    /// `chase` says the board may be asleep, advertising for only a short
    /// window per wake (configurable on the board, seconds rather than
    /// minutes). That rules out bounded connect attempts, which can keep
    /// missing a window they are out of phase with, so a chasing transport
    /// scans continuously instead - always listening, whenever the window
    /// happens to open.
    Connect { mac: Option<String>, chase: bool },
    /// Scan without connecting, reporting every board that answers as a
    /// [`BleEvent::Discovered`], until a `Connect` or `Disconnect` arrives.
    ///
    /// Separate from the scan a `Connect` does because the two want opposite
    /// things: connecting takes the first board that matches and stops, while
    /// the picker needs to keep looking so a board that advertises late still
    /// turns up in the list. Only one board is ever connected at a time, so
    /// this drops any live link first.
    Scan,
    /// Write one setting to the config characteristic. The device answers on
    /// the ack characteristic with the value it actually applied.
    Config(ConfigWrite),
    /// Push a whole radio TOML config (already stripped of comments and
    /// metadata, at most [`crate::radio::CONFIG_MAX`] bytes) through the bulk
    /// characteristic. The board applies it live and writes it to its SD
    /// card. The outcome comes back as [`BleEvent::ConfigPushed`].
    PushConfig(Vec<u8>),
    /// Drop the connection and stay idle until the next `Connect`.
    ///
    /// Forceful: it does not wait for the step in progress, and it takes the
    /// queued config writes with it. Whatever was half-done was aimed at a
    /// board the user has stopped asking for.
    Disconnect,
}

/// One config-characteristic write, `[id, len, value...]`. The gps-proto
/// notify interval and the board's own ids share the characteristic and
/// differ only in the id and the width of the value.
#[derive(Clone, Copy, Debug)]
pub enum ConfigWrite {
    /// Position notify interval in ms (gps-proto `CFG_UPDATE_INTERVAL_MS`).
    Interval(u32),
    /// A board on/off setting: radio standby or GPS backup mode.
    Flag { id: u8, on: bool },
    /// A board interval in seconds: the wake-check cadence.
    Seconds { id: u8, secs: u32 },
    /// The board's mode.
    ///
    /// Not a [`ConfigWrite::Flag`] even though it is one byte: a flag is a
    /// subsystem switched on or off, and a mode is a whole posture - which
    /// of the settings above the board even reads, and what it comes back
    /// as after a flat cell.
    Mode(ble::Mode),
    /// The board's name: the label it stores in flash and advertises behind
    /// the firmware's prefix (`ble::CFG_NAME`). A length of 0 clears it,
    /// returning the board to its address-derived name.
    ///
    /// Built through [`ConfigWrite::name`], which is where the label is
    /// checked, so the worker never sends what the board would refuse.
    /// Bytes rather than a `String` so a write stays `Copy` like the others.
    Name {
        label: [u8; ble::NAME_LABEL_MAX],
        len: u8,
    },
}

impl ConfigWrite {
    /// A name write for `label`, or `None` for a label the board would
    /// refuse: longer than [`ble::NAME_LABEL_MAX`] bytes, or anything but
    /// ASCII letters, digits, `-` and `_`. A blank label clears the name.
    pub fn name(label: &str) -> Option<Self> {
        let label = label.trim();
        if !label.is_empty() && !ble::valid_label(label.as_bytes()) {
            return None;
        }
        let mut bytes = [0u8; ble::NAME_LABEL_MAX];
        bytes[..label.len()].copy_from_slice(label.as_bytes());
        Some(ConfigWrite::Name {
            label: bytes,
            len: label.len() as u8,
        })
    }

    /// The encoded write and its length. The buffer is the longest write the
    /// config characteristic takes, which is a full-length name; every other
    /// write uses the first few bytes of it.
    pub fn encode(&self) -> ([u8; ble::CONFIG_WRITE_MAX], usize) {
        let mut b = [0u8; ble::CONFIG_WRITE_MAX];
        match *self {
            ConfigWrite::Interval(ms) => {
                let (small, n) =
                    packet::encode_config(packet::ConfigCommand::UpdateIntervalMs(ms));
                b[..n].copy_from_slice(&small[..n]);
                (b, n)
            }
            ConfigWrite::Flag { id, on } => {
                b[0] = id;
                b[1] = 1;
                b[2] = on as u8;
                (b, 3)
            }
            ConfigWrite::Seconds { id, secs } => {
                b[0] = id;
                b[1] = 4;
                b[2..6].copy_from_slice(&secs.to_le_bytes());
                (b, 6)
            }
            ConfigWrite::Mode(mode) => {
                b[0] = ble::CFG_MODE;
                b[1] = 1;
                b[2] = mode.as_wire();
                (b, 3)
            }
            ConfigWrite::Name { label, len } => {
                let len = usize::from(len);
                b[0] = ble::CFG_NAME;
                b[1] = len as u8;
                b[2..2 + len].copy_from_slice(&label[..len]);
                (b, 2 + len)
            }
        }
    }
}

/// The label a board's name carries, when somebody gave it one.
///
/// A board advertises `<prefix>-<label>`, and one that has never been named
/// puts the tail of its own address where the label goes. That fallback is a
/// name only in the sense that it is unique: nobody chose it, so it is not
/// what the board should be called anywhere a person reads, and a nickname
/// in the app config outranks it. With the board's address to hand the
/// fallback is reproduced exactly, by the rule the firmware builds it with.
/// Without one (connected to "any board"), a label of four lowercase hex
/// digits is taken for the fallback - which a chosen name could be, at the
/// cost of nothing worse than the whole advertised string being shown.
///
/// `None` for the fallback, for a name without the prefix (a C3 beacon, or
/// something that is not one of these boards), and for an empty label.
pub fn board_label<'a>(name: &'a str, mac: Option<&str>) -> Option<&'a str> {
    let label = name
        .strip_prefix(ble::NAME_PREFIX)?
        .strip_prefix(char::from(ble::NAME_SEP))?;
    if label.is_empty() {
        return None;
    }
    let unnamed = match mac.and_then(mac_bytes) {
        Some(addr) => {
            let mut buf = [0u8; ble::NAME_MAX];
            ble::advertised_name("", &addr, &mut buf) == name
        }
        None => {
            label.len() == 4
                && label
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        }
    };
    (!unnamed).then_some(label)
}

/// A printed address as the bytes the controller takes, LSB first:
/// `AA:BB:CC:DD:EE:FF` becomes `[FF, EE, DD, CC, BB, AA]`. `None` for
/// anything that is not six hex bytes.
fn mac_bytes(mac: &str) -> Option<[u8; 6]> {
    let mut out = [0u8; 6];
    let mut n = 0;
    for part in normalize_mac(mac).split(':') {
        if n == 6 {
            return None;
        }
        out[5 - n] = u8::from_str_radix(part, 16).ok()?;
        n += 1;
    }
    (n == 6).then_some(out)
}

/// How long a push waits for a bulk ack before giving up. Every op is answered
/// on the board itself - no second chip to wait on - so a link this quiet is
/// dead, not slow.
const PUSH_ACK_TIMEOUT: Duration = Duration::from_secs(10);

/// How often a step that would otherwise block comes up for air to service
/// commands. Short enough that a press feels like it landed on the press.
const CMD_POLL: Duration = Duration::from_millis(100);

/// Sends events to the UI and wakes it so it drains the channel promptly.
///
/// Also the worker's record of which request it is serving: every event is
/// tagged with that epoch on the way out (see [`Epoch`]), and the UI throws
/// away anything older than what it last asked for.
pub(crate) struct Reporter {
    ctx: egui::Context,
    tx: Sender<BleUpdate>,
    epoch: Cell<Epoch>,
}

impl Reporter {
    pub(crate) fn new(ctx: egui::Context, tx: Sender<BleUpdate>) -> Self {
        Self {
            ctx,
            tx,
            epoch: Cell::new(0),
        }
    }

    pub(crate) fn send(&self, event: BleEvent) {
        let _ = self.tx.send(BleUpdate {
            epoch: self.epoch.get(),
            event,
        });
        self.ctx.request_repaint();
    }

    pub(crate) fn status(&self, s: impl Into<String>) {
        self.send(BleEvent::Status(s.into()));
    }
}

/// What the UI currently wants from the worker. `connect` and `scan` are
/// mutually exclusive: a discovery scan has no link, and a connected session
/// does not scan.
pub(crate) struct Wanted {
    /// The request the rest of this came from; see [`Epoch`].
    epoch: Epoch,
    pub(crate) connect: bool,
    /// Run a discovery scan for the device picker; see [`BleCommand::Scan`].
    pub(crate) scan: bool,
    pub(crate) mac: Option<String>,
    /// The board may be asleep; see [`BleCommand::Connect`].
    pub(crate) chase: bool,
}

/// The request one connect session is serving. Taken when the session starts
/// and compared against [`Wanted`] at every step it could stop at, so a
/// session survives exactly the presses that did not change what was asked
/// for - and no others.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Target {
    epoch: Epoch,
    pub(crate) mac: Option<String>,
    pub(crate) chase: bool,
}

/// Why a running session has to end before it otherwise would.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Interrupt {
    /// Disconnect, or a discovery scan: stop, and do not retry.
    Stopped,
    /// A newer request replaces this one - a different board, a different
    /// chase mode, or the same board asked for again, which means start over
    /// rather than carry on.
    Superseded,
}

impl Wanted {
    /// Nothing wanted, which is how a worker starts: the UI has to ask.
    pub(crate) fn idle() -> Self {
        Self {
            epoch: 0,
            connect: false,
            scan: false,
            mac: None,
            chase: false,
        }
    }

    /// What a session started now would be serving.
    pub(crate) fn target(&self) -> Target {
        Target {
            epoch: self.epoch,
            mac: self.mac.clone(),
            chase: self.chase,
        }
    }

    /// Whether the session started for `target` may still run.
    pub(crate) fn interrupt(&self, target: &Target) -> Option<Interrupt> {
        if !self.connect {
            return Some(Interrupt::Stopped);
        }
        (self.target() != *target).then_some(Interrupt::Superseded)
    }
}

/// The UI has gone away, so there is nothing left to serve.
pub(crate) struct Gone;

/// A step gave up because the UI asked for something else (or went away).
/// Sessions unwind on it without an error: nothing failed and there is nothing
/// to retry - the worker's next pass serves whatever is wanted now.
pub(crate) struct Aborted;

/// How a connect session ended.
pub(crate) enum Ended {
    /// It stopped because the UI said so. The worker just carries on.
    Quietly,
    /// Something broke; the worker reports it and retries after a pause.
    Failed(String),
}

impl From<Aborted> for Ended {
    fn from(_: Aborted) -> Self {
        Ended::Quietly
    }
}

impl From<String> for Ended {
    fn from(e: String) -> Self {
        Ended::Failed(e)
    }
}

impl From<&str> for Ended {
    fn from(e: &str) -> Self {
        Ended::Failed(e.to_string())
    }
}

/// The UI's side of the worker: the command channel and everything a command
/// changes. Bundled because every step that can block has to keep servicing it
/// - a Disconnect that only lands once a 20 s connect attempt has finished is
/// not a disconnect, it is a delay - and because both transports then share
/// one implementation of what each command means.
pub(crate) struct Inbox<'a> {
    pub(crate) rx: &'a Receiver<BleRequest>,
    pub(crate) wanted: &'a mut Wanted,
    /// Config writes waiting for a link.
    pub(crate) writes: &'a mut Vec<ConfigWrite>,
    /// The radio-config push riding the current session, if any.
    pub(crate) push: &'a mut Option<ConfigPush>,
}

impl Inbox<'_> {
    /// Apply every pending command. Config writes are queued into `writes` so
    /// a request made while connected is applied by the pump loop; a config
    /// push lands in `push` and is started there the same way.
    ///
    /// Anything the link was carrying for a board we are no longer talking to
    /// is dropped here rather than replayed at the next board: a queued write
    /// was aimed at the board that was selected when it was made.
    pub(crate) fn drain(&mut self, report: &Reporter) -> Result<(), Gone> {
        loop {
            let request = match self.rx.try_recv() {
                Ok(request) => request,
                Err(TryRecvError::Empty) => return Ok(()),
                Err(TryRecvError::Disconnected) => return Err(Gone),
            };
            // A command from a request already replaced by a later one. The UI
            // sends in order, so this only catches a straggler queued behind a
            // press that overtook it.
            if request.epoch < self.wanted.epoch {
                continue;
            }
            match request.command {
                BleCommand::Connect { mac, chase } => {
                    if mac != self.wanted.mac {
                        self.writes.clear();
                    }
                    self.wanted.connect = true;
                    self.wanted.scan = false;
                    self.wanted.mac = mac;
                    self.wanted.chase = chase;
                }
                BleCommand::Scan => {
                    self.wanted.connect = false;
                    self.wanted.scan = true;
                    self.writes.clear();
                }
                BleCommand::Disconnect => {
                    self.wanted.connect = false;
                    self.wanted.scan = false;
                    self.writes.clear();
                }
                BleCommand::Config(w) => self.writes.push(w),
                BleCommand::PushConfig(data) => *self.push = Some(ConfigPush::new(data)),
            }
            self.wanted.epoch = request.epoch;
            report.epoch.set(request.epoch);
        }
    }

    /// Drain, then say whether the session serving `target` may continue.
    pub(crate) fn check(&mut self, report: &Reporter, target: &Target) -> Result<(), Aborted> {
        self.drain(report).map_err(|Gone| Aborted)?;
        match self.wanted.interrupt(target) {
            Some(_) => Err(Aborted),
            None => Ok(()),
        }
    }

    /// Fail any queued or in-flight config push. Called whenever the session
    /// it rode on ends (or cannot start), so the Radio page is never left
    /// waiting on a transfer that no longer exists.
    pub(crate) fn fail_push(&mut self, report: &Reporter) {
        if self.push.take().is_some() {
            report.send(BleEvent::ConfigPushed(Err(
                "disconnected before the transfer finished".to_string(),
            )));
        }
    }
}

/// One radio-config push through the board's bulk characteristic, advanced one
/// ack at a time: OP_BEGIN opens the transfer, each OP_DATA carries one chunk,
/// and OP_END has the board check the CRC, parse the file and apply it. Every
/// op is acked (id [`ble::ACK_ID_BULK`] on the ack characteristic) and the
/// next is only sent once that ack is in, which is what keeps a write burst
/// from outrunning the transfer buffer on the far side.
///
/// Visible to the crate only because [`Inbox`] holds one; nothing outside
/// this module builds or steps a push.
pub(crate) struct ConfigPush {
    data: Vec<u8>,
    /// Byte offset of the next OP_DATA chunk.
    off: usize,
    /// Sequence number of the next OP_DATA chunk.
    seq: u16,
    started: bool,
    /// OP_END is on the wire; the next bulk ack is the verdict.
    ending: bool,
}

/// What a push wants done next.
enum PushStep {
    /// Write this to the bulk characteristic and wait for the next bulk ack.
    Write(Vec<u8>),
    /// The board verified, applied and stored the config.
    Done,
    /// The transfer failed; the message is for the Radio page.
    Fail(String),
}

impl ConfigPush {
    fn new(data: Vec<u8>) -> Self {
        Self {
            data,
            off: 0,
            seq: 0,
            started: false,
            ending: false,
        }
    }

    /// The OP_BEGIN frame that opens the transfer, once; `None` after it is
    /// underway.
    fn start(&mut self) -> Option<Vec<u8>> {
        if self.started {
            return None;
        }
        self.started = true;
        let mut f = vec![ble::OP_BEGIN, ble::KIND_TOML];
        f.extend_from_slice(&(self.data.len() as u32).to_le_bytes());
        f.extend_from_slice(&link::crc32(&self.data).to_le_bytes());
        // The version field is only meaningful for firmware images; 0 for TOML.
        f.extend_from_slice(&0u16.to_le_bytes());
        Some(f)
    }

    /// Feed the bulk ack for the last op sent; says what to do next.
    fn on_ack(&mut self, ack: &Ack) -> PushStep {
        if ack.status != packet::ACK_OK {
            return PushStep::Fail(push_error(ack.status, self.ending));
        }
        if self.ending {
            return PushStep::Done;
        }
        if self.off >= self.data.len() {
            self.ending = true;
            return PushStep::Write(vec![ble::OP_END]);
        }
        let end = usize::min(self.off + ble::BULK_DATA_MAX, self.data.len());
        let mut f = vec![ble::OP_DATA];
        f.extend_from_slice(&self.seq.to_le_bytes());
        f.extend_from_slice(&self.data[self.off..end]);
        self.off = end;
        self.seq = self.seq.wrapping_add(1);
        PushStep::Write(f)
    }
}

/// Why a bulk op was refused, in words the Radio page can show. `at_end` marks
/// the OP_END ack: the board only parses the file there, so a refusal at that
/// point is the file's content or bytes that did not survive the trip, while
/// the same status earlier is the op itself.
fn push_error(status: u8, at_end: bool) -> String {
    match status {
        packet::ACK_BAD_VALUE if at_end => "the board rejected the config: check for a value \
                                            out of range, a string that is not one of the \
                                            choices, or a transfer that did not arrive intact"
            .to_string(),
        packet::ACK_BAD_VALUE => "the board rejected it (bad value or size)".to_string(),
        // Either the USB console owns a transfer, or this op found none to
        // act on. Nothing here reads as "rejected": a config the board
        // refused keeps answering ACK_BAD_VALUE however often it is retried.
        ble::ACK_BAD_STATE => "another transfer is already running".to_string(),
        s => format!("the board refused (status {s:#04x})"),
    }
}

/// Decode a settings blob into the event that describes it. A blob that fails
/// to decode is a version mismatch, which is its own event rather than a
/// silent fall back to defaults the board never reported.
fn settings_event(bytes: &[u8]) -> BleEvent {
    match Settings::decode(bytes) {
        Some(s) => BleEvent::Settings(s),
        None => BleEvent::SettingsUnsupported,
    }
}

/// Decode a name characteristic value, or `None` for one that is not a name:
/// empty, or not the ASCII the firmware builds it from. Firmware that predates
/// board names has no such characteristic at all, which is the same "nothing to
/// show" as a value that fails here.
fn name_event(bytes: &[u8]) -> Option<BleEvent> {
    let name = core::str::from_utf8(bytes).ok()?.trim();
    if name.is_empty() {
        return None;
    }
    Some(BleEvent::Name(name.to_string()))
}

/// Decode a radio-config blob into the event it describes, or `None` when the
/// board has not reported one yet. The board seeds the characteristic with
/// zeros (layout version 0) until its radio has come up, so an all-zero blob
/// is "unknown", not a version we cannot read. A non-zero blob that still
/// fails to decode is a genuine version mismatch.
fn radio_config_event(bytes: &[u8]) -> Option<BleEvent> {
    if bytes.first().copied().unwrap_or(0) == 0 {
        return None;
    }
    Some(match RadioConfig::decode(bytes) {
        Some(c) => BleEvent::RadioConfig(c),
        None => BleEvent::RadioConfigUnsupported,
    })
}

/// Decode a remote-position blob (`[src u8, rssi i16le, PositionPacket,
/// age_s u16le]`) from the remote characteristic into a [`BleEvent::Remote`].
/// `None` for a short blob, an undecodable packet, or `src` 0 - the board
/// seeds the characteristic with a zero source until it has heard a node, and
/// 0 is the local GPS in any case, delivered on the position characteristic
/// instead.
///
/// The age field is only read when the blob is long enough to hold it, so a
/// board running firmware from before it existed still reports positions -
/// as ages of zero, which is what that firmware effectively claimed by
/// notifying its cache on a timer.
fn remote_event(bytes: &[u8]) -> Option<BleEvent> {
    if bytes.len() < ble::REMOTE_LEN {
        return None;
    }
    let src = bytes[0];
    if src == 0 {
        return None;
    }
    let rssi = i16::from_le_bytes([bytes[1], bytes[2]]);
    let packet = PositionPacket::decode(&bytes[3..])?;
    Some(BleEvent::Remote { src, rssi, packet, age_s: age_of(bytes, ble::REMOTE_AGE_OFF) })
}

/// Decode a node-ping blob (`[src u8, rssi i16le, flags u8, uptime_s u16le,
/// age_s u16le]`) into a [`BleEvent::NodePing`]. `None` for a short blob or
/// `src` 0, as with a remote position.
///
/// Flag bits this build does not know are ignored rather than rejected, so a
/// node running newer firmware is still reported as alive.
fn node_ping_event(bytes: &[u8]) -> Option<BleEvent> {
    if bytes.len() < ble::NODE_PING_LEN {
        return None;
    }
    let src = bytes[0];
    if src == 0 {
        return None;
    }
    Some(BleEvent::NodePing(NodePing {
        src,
        rssi: i16::from_le_bytes([bytes[1], bytes[2]]),
        uptime_s: u16::from_le_bytes([bytes[4], bytes[5]]),
        gps_present: bytes[3] & lora::PING_FLAG_GPS_PRESENT != 0,
        had_fix: bytes[3] & lora::PING_FLAG_HAD_FIX != 0,
        age_s: age_of(bytes, ble::NODE_PING_AGE_OFF),
    }))
}

/// The age field at `off`, or 0 when the board's blob is too short to carry
/// one.
fn age_of(bytes: &[u8], off: usize) -> u16 {
    match bytes.get(off..off + 2) {
        Some(b) => u16::from_le_bytes([b[0], b[1]]),
        None => 0,
    }
}

/// The UI's handle to the BLE worker.
pub struct BleHandle {
    pub events: Receiver<BleUpdate>,
    pub commands: Sender<BleRequest>,
}

/// Spawn the BLE worker thread. It starts idle; send [`BleCommand::Connect`]
/// to begin. Desktop signature; Android needs the JVM/Activity pointers.
#[cfg(not(target_os = "android"))]
pub fn spawn(ctx: egui::Context) -> BleHandle {
    desktop::spawn(ctx)
}

/// Spawn the BLE worker thread (Android). `vm`/`activity` are the raw
/// pointers from `AndroidApp`, as with the GPS source.
#[cfg(target_os = "android")]
pub fn spawn(ctx: egui::Context, vm: usize, activity: usize) -> BleHandle {
    android::spawn(ctx, vm, activity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use midair_proto::ble;
    use std::sync::mpsc::channel;

    /// A worker's command side with no BLE behind it. What the transports
    /// share is what a command *means* - which session it ends, what it
    /// throws away - so that is what these drive, once, for both platforms.
    struct Worker {
        commands: Sender<BleRequest>,
        rx: Receiver<BleRequest>,
        report: Reporter,
        events: Receiver<BleUpdate>,
        wanted: Wanted,
        writes: Vec<ConfigWrite>,
        push: Option<ConfigPush>,
        /// The next epoch a press will use, as the UI would number them.
        epoch: Epoch,
    }

    impl Worker {
        fn new() -> Self {
            let (commands, rx) = channel();
            let (event_tx, events) = channel();
            Self {
                commands,
                rx,
                report: Reporter::new(egui::Context::default(), event_tx),
                events,
                wanted: Wanted::idle(),
                writes: Vec::new(),
                push: None,
                epoch: 0,
            }
        }

        /// One press: a fresh epoch, as [`super::Epoch`] describes.
        fn press(&mut self, command: BleCommand) {
            self.epoch += 1;
            let _ = self.commands.send(BleRequest {
                epoch: self.epoch,
                command,
            });
        }

        /// A command that rides the current request rather than making a new
        /// one (a config write from a page, say).
        fn under_current_request(&self, command: BleCommand) {
            let _ = self.commands.send(BleRequest {
                epoch: self.epoch,
                command,
            });
        }

        fn drain(&mut self) {
            let mut inbox = Inbox {
                rx: &self.rx,
                wanted: &mut self.wanted,
                writes: &mut self.writes,
                push: &mut self.push,
            };
            assert!(inbox.drain(&self.report).is_ok(), "UI still there");
        }

        fn connect(mac: Option<&str>) -> BleCommand {
            BleCommand::Connect {
                mac: mac.map(str::to_string),
                chase: false,
            }
        }

        /// The epoch stamped on the events the worker is sending now.
        fn event_epochs(&self) -> Vec<Epoch> {
            self.report.status("marker");
            self.events.try_iter().map(|u| u.epoch).collect()
        }
    }

    /// The reported bug: Disconnect pressed while connecting to one board,
    /// then a connect to a different one. The session that was running has to
    /// end - both because it was told to stop and because it is now aimed at
    /// the wrong board - or the app connects to the first board and only then
    /// notices.
    #[test]
    fn disconnect_then_another_board_ends_the_running_session() {
        let mut w = Worker::new();
        w.press(Worker::connect(Some("AA:01")));
        w.drain();
        // What the session that is now scanning/connecting is serving.
        let first = w.wanted.target();
        assert!(w.wanted.interrupt(&first).is_none(), "nothing has changed");

        w.press(BleCommand::Disconnect);
        w.drain();
        assert_eq!(w.wanted.interrupt(&first), Some(Interrupt::Stopped));
        assert!(!w.wanted.connect && !w.wanted.scan);

        w.press(Worker::connect(Some("BB:02")));
        w.drain();
        // Still ended - now because it is the wrong board - and the next
        // session goes to the board that was actually asked for.
        assert_eq!(w.wanted.interrupt(&first), Some(Interrupt::Superseded));
        let second = w.wanted.target();
        assert_eq!(second.mac.as_deref(), Some("BB:02"));
        assert!(w.wanted.interrupt(&second).is_none());
    }

    /// Pressing Connect again is a request to start over, not a no-op: it is
    /// the way out of a link that is up but has stopped working.
    #[test]
    fn pressing_connect_again_starts_over() {
        let mut w = Worker::new();
        w.press(Worker::connect(Some("AA:01")));
        w.drain();
        let running = w.wanted.target();

        w.press(Worker::connect(Some("AA:01")));
        w.drain();
        assert_eq!(w.wanted.interrupt(&running), Some(Interrupt::Superseded));
        assert_eq!(w.wanted.mac.as_deref(), Some("AA:01"));
    }

    /// Chasing is a different way of connecting, so switching to it while a
    /// plain connect is running has to restart rather than be swallowed as
    /// "already connecting to that board".
    #[test]
    fn switching_to_chase_restarts() {
        let mut w = Worker::new();
        w.press(Worker::connect(Some("AA:01")));
        w.drain();
        let plain = w.wanted.target();

        w.press(BleCommand::Connect {
            mac: Some("AA:01".to_string()),
            chase: true,
        });
        w.drain();
        assert_eq!(w.wanted.interrupt(&plain), Some(Interrupt::Superseded));
        assert!(w.wanted.chase);
    }

    /// A scan drops any live link: only one board is ever connected, and the
    /// picker cannot fill itself from a session that is holding the radio.
    #[test]
    fn a_scan_ends_a_connect() {
        let mut w = Worker::new();
        w.press(Worker::connect(None));
        w.drain();
        let connecting = w.wanted.target();

        w.press(BleCommand::Scan);
        w.drain();
        assert_eq!(w.wanted.interrupt(&connecting), Some(Interrupt::Stopped));
        assert!(w.wanted.scan && !w.wanted.connect);
    }

    /// A queued write was aimed at the board that was selected when it was
    /// made. Disconnecting, or moving to another board, must not deliver it to
    /// whatever is connected next.
    #[test]
    fn writes_do_not_outlive_the_board_they_were_meant_for() {
        let mut w = Worker::new();
        w.press(Worker::connect(Some("AA:01")));
        w.under_current_request(BleCommand::Config(ConfigWrite::Flag {
            id: ble::CFG_PWR_EN,
            on: true,
        }));
        w.drain();
        assert_eq!(w.writes.len(), 1);

        w.press(BleCommand::Disconnect);
        w.drain();
        assert!(w.writes.is_empty(), "disconnect drops queued writes");

        // The same on a switch, without a disconnect in between.
        w.press(Worker::connect(Some("AA:01")));
        w.under_current_request(BleCommand::Config(ConfigWrite::Interval(2000)));
        w.drain();
        assert_eq!(w.writes.len(), 1);
        w.press(Worker::connect(Some("BB:02")));
        w.drain();
        assert!(w.writes.is_empty(), "switching boards drops queued writes");
    }

    /// A write made while connected to the board still selected is kept: only
    /// a change of board (or of mind) throws them away.
    #[test]
    fn a_write_survives_a_reconnect_to_the_same_board() {
        let mut w = Worker::new();
        w.press(Worker::connect(Some("AA:01")));
        w.under_current_request(BleCommand::Config(ConfigWrite::Interval(1500)));
        w.drain();
        w.press(Worker::connect(Some("AA:01")));
        w.drain();
        assert_eq!(w.writes.len(), 1);
    }

    /// Events are stamped with the request the worker is serving, which is
    /// what lets the UI tell the last board's parting words from this one's.
    #[test]
    fn events_carry_the_request_they_belong_to() {
        let mut w = Worker::new();
        // Before any command: the worker speaking for itself (no adapter, no
        // Bluetooth), which the UI never fences out.
        assert_eq!(w.event_epochs(), vec![0]);

        w.press(Worker::connect(None));
        w.drain();
        assert_eq!(w.event_epochs(), vec![1]);

        w.press(BleCommand::Disconnect);
        w.drain();
        assert_eq!(w.event_epochs(), vec![2]);
    }

    /// A command left over from a request that has already been replaced is
    /// dropped rather than applied on top of the newer one.
    #[test]
    fn a_command_from_a_replaced_request_is_ignored() {
        let mut w = Worker::new();
        w.press(Worker::connect(Some("AA:01")));
        w.drain();

        w.press(BleCommand::Disconnect);
        // Sent under the older request, and overtaken by the Disconnect above.
        let _ = w.commands.send(BleRequest {
            epoch: w.epoch - 1,
            command: Worker::connect(Some("AA:01")),
        });
        w.drain();
        assert!(!w.wanted.connect, "the stale connect did not revive the link");
    }

    /// The UI going away is the one thing that ends the worker itself.
    #[test]
    fn a_dropped_ui_reports_gone() {
        let mut w = Worker::new();
        let (dead, rx) = channel::<BleRequest>();
        drop(dead);
        let mut inbox = Inbox {
            rx: &rx,
            wanted: &mut w.wanted,
            writes: &mut w.writes,
            push: &mut w.push,
        };
        assert!(inbox.drain(&w.report).is_err());
    }

    /// The board parses `[id, len, value...]`, so the framing is what has to
    /// be right - a wrong length byte is read as a different value entirely.
    #[test]
    fn flag_and_seconds_framing() {
        let (b, n) = ConfigWrite::Flag {
            id: ble::CFG_PWR_EN,
            on: true,
        }
        .encode();
        assert_eq!(&b[..n], &[ble::CFG_PWR_EN, 1, 1]);

        let (b, n) = ConfigWrite::Flag {
            id: ble::CFG_WIO_SLEEP,
            on: false,
        }
        .encode();
        assert_eq!(&b[..n], &[ble::CFG_WIO_SLEEP, 1, 0]);

        // 300 s = 0x012C, little endian.
        let (b, n) = ConfigWrite::Seconds {
            id: ble::CFG_ESP_SLEEP_S,
            secs: 300,
        }
        .encode();
        assert_eq!(&b[..n], &[ble::CFG_ESP_SLEEP_S, 4, 0x2C, 0x01, 0, 0]);
    }

    #[test]
    fn interval_matches_gps_proto() {
        let (mine, n) = ConfigWrite::Interval(1500).encode();
        let (theirs, m) = packet::encode_config(packet::ConfigCommand::UpdateIntervalMs(1500));
        assert_eq!((&mine[..n], n), (&theirs[..m], m));
    }

    /// A name goes out as `[id, len, bytes]`, a blank one as a length of 0,
    /// and one the board would refuse is refused here first.
    #[test]
    fn name_framing_and_validation() {
        let (b, n) = ConfigWrite::name("sky-1").unwrap().encode();
        assert_eq!(&b[..n], &[ble::CFG_NAME, 5, b's', b'k', b'y', b'-', b'1']);

        let (b, n) = ConfigWrite::name("  ").unwrap().encode();
        assert_eq!(&b[..n], &[ble::CFG_NAME, 0]);

        let longest = "x".repeat(ble::NAME_LABEL_MAX);
        let (b, n) = ConfigWrite::name(&longest).unwrap().encode();
        assert_eq!(n, ble::CONFIG_WRITE_MAX);
        assert_eq!(&b[2..n], longest.as_bytes());

        assert!(ConfigWrite::name(&"x".repeat(ble::NAME_LABEL_MAX + 1)).is_none());
        assert!(ConfigWrite::name("has space").is_none());
        assert!(ConfigWrite::name("caf\u{e9}").is_none());
    }

    /// A chosen label is the board's name; the address tail an unnamed board
    /// advertises is not, and neither is a name from other firmware.
    #[test]
    fn board_label_tells_a_name_from_the_address_fallback() {
        let mac = "FF:C6:A1:53:50:47";
        assert_eq!(board_label("ws3gps-sky-1", Some(mac)), Some("sky-1"));
        assert_eq!(board_label("ws3gps-5047", Some(mac)), None);
        // A label that merely looks like an address tail is a name when the
        // address says it is not this board's.
        assert_eq!(board_label("ws3gps-beef", Some(mac)), Some("beef"));
        // Any spelling of the address finds the same fallback.
        assert_eq!(board_label("ws3gps-5047", Some("ff-c6-a1-53-50-47")), None);

        // Without an address the shape of the label has to do.
        assert_eq!(board_label("ws3gps-sky-1", None), Some("sky-1"));
        assert_eq!(board_label("ws3gps-5047", None), None);
        assert_eq!(board_label("ws3gps-beef", None), None);
        assert_eq!(board_label("ws3gps-BEEF", None), Some("BEEF"));

        assert_eq!(board_label("ws3gps-", Some(mac)), None);
        assert_eq!(board_label("GPS-C3", Some(mac)), None);
        assert_eq!(board_label("", None), None);
    }

    #[test]
    fn remote_event_carries_src_and_rejects_local() {
        use gps_proto::packet::{PositionPacket, FLAG_FIX};

        let packet = PositionPacket {
            lat_e7: 481_173_000,
            lon_e7: -1_226_760_000,
            flags: FLAG_FIX,
            sats: 6,
            ..PositionPacket::default()
        };
        let mut blob = [0u8; ble::REMOTE_LEN_V2];
        blob[0] = 7; // src address
        blob[1..3].copy_from_slice(&(-92i16).to_le_bytes());
        blob[3..ble::REMOTE_LEN].copy_from_slice(&packet.encode());
        blob[ble::REMOTE_AGE_OFF..].copy_from_slice(&41u16.to_le_bytes());

        match remote_event(&blob) {
            Some(BleEvent::Remote { src, rssi, packet: p, age_s }) => {
                assert_eq!(src, 7);
                assert_eq!(rssi, -92);
                assert_eq!(p, packet);
                assert_eq!(age_s, 41);
            }
            _ => panic!("expected a remote event"),
        }

        // Source 0 is the local GPS / "nothing heard yet"; not a remote.
        blob[0] = 0;
        assert!(remote_event(&blob).is_none());
        // A short blob decodes to nothing rather than panicking.
        assert!(remote_event(&blob[..ble::REMOTE_LEN - 1]).is_none());
    }

    /// A board running firmware from before the age field still reports
    /// positions, rather than every node vanishing on a firmware mismatch.
    #[test]
    fn remote_event_reads_a_board_without_the_age_field() {
        use gps_proto::packet::{PositionPacket, FLAG_FIX};

        let packet = PositionPacket {
            lat_e7: 481_173_000,
            lon_e7: -1_226_760_000,
            flags: FLAG_FIX,
            ..PositionPacket::default()
        };
        let mut blob = [0u8; ble::REMOTE_LEN];
        blob[0] = 7;
        blob[3..].copy_from_slice(&packet.encode());

        match remote_event(&blob) {
            Some(BleEvent::Remote { src, packet: p, age_s, .. }) => {
                assert_eq!(src, 7);
                assert_eq!(p, packet);
                assert_eq!(age_s, 0);
            }
            _ => panic!("expected a remote event"),
        }
    }

    #[test]
    fn node_ping_event_carries_the_flags_and_rejects_local() {
        let mut blob = [0u8; ble::NODE_PING_LEN];
        blob[0] = 3; // src address
        blob[1..3].copy_from_slice(&(-97i16).to_le_bytes());
        blob[3] = lora::PING_FLAG_GPS_PRESENT;
        blob[4..6].copy_from_slice(&214u16.to_le_bytes());
        blob[ble::NODE_PING_AGE_OFF..].copy_from_slice(&5u16.to_le_bytes());

        match node_ping_event(&blob) {
            Some(BleEvent::NodePing(p)) => {
                assert_eq!(p.src, 3);
                assert_eq!(p.rssi, -97);
                assert_eq!(p.uptime_s, 214);
                assert!(p.gps_present);
                assert!(!p.had_fix);
                assert_eq!(p.age_s, 5);
            }
            _ => panic!("expected a node ping event"),
        }

        // Both flags travel independently, and a bit this build does not
        // know is ignored rather than making the node disappear.
        blob[3] = lora::PING_FLAG_HAD_FIX | 0x80;
        match node_ping_event(&blob) {
            Some(BleEvent::NodePing(p)) => {
                assert!(!p.gps_present);
                assert!(p.had_fix);
            }
            _ => panic!("expected a node ping event"),
        }

        blob[0] = 0;
        assert!(node_ping_event(&blob).is_none());
        assert!(node_ping_event(&blob[..ble::NODE_PING_LEN - 1]).is_none());
    }

    /// Walk a two-chunk push through its whole life: OP_BEGIN framing, chunk
    /// sizes and sequence numbers, OP_END, and the final Done. The board
    /// parses these frames byte-by-byte, so the framing is what must be right.
    #[test]
    fn config_push_walks_begin_data_end() {
        let data: Vec<u8> = (0..=255).collect(); // 256 B: one full chunk + 64
        let crc = link::crc32(&data);
        let mut push = ConfigPush::new(data.clone());

        let begin = push.start().unwrap();
        assert_eq!(begin[0], ble::OP_BEGIN);
        assert_eq!(begin[1], ble::KIND_TOML);
        assert_eq!(begin[2..6], 256u32.to_le_bytes());
        assert_eq!(begin[6..10], crc.to_le_bytes());
        assert_eq!(begin[10..12], 0u16.to_le_bytes());
        // Only once: the transfer must not restart on a later pump tick.
        assert!(push.start().is_none());

        let ok = |value_u32| Ack {
            id: ble::ACK_ID_BULK,
            status: packet::ACK_OK,
            value_u32,
        };
        let frame = |step| match step {
            PushStep::Write(f) => f,
            _ => panic!("expected a write"),
        };

        let first = frame(push.on_ack(&ok(Some(0))));
        assert_eq!(first[0], ble::OP_DATA);
        assert_eq!(first[1..3], 0u16.to_le_bytes());
        assert_eq!(&first[3..], &data[..ble::BULK_DATA_MAX]);

        let second = frame(push.on_ack(&ok(Some(1))));
        assert_eq!(second[1..3], 1u16.to_le_bytes());
        assert_eq!(&second[3..], &data[ble::BULK_DATA_MAX..]);

        let end = frame(push.on_ack(&ok(Some(2))));
        assert_eq!(end, vec![ble::OP_END]);
        assert!(matches!(push.on_ack(&ok(None)), PushStep::Done));
    }

    /// A config that divides exactly into chunks must not send a trailing
    /// empty OP_DATA: the board reads a zero-length chunk as a framing error,
    /// so the push would fail at the last byte rather than the first.
    #[test]
    fn config_push_ends_cleanly_on_a_chunk_boundary() {
        let data = vec![0xABu8; ble::BULK_DATA_MAX * 2];
        let mut push = ConfigPush::new(data.clone());
        push.start().unwrap();
        let ok = Ack {
            id: ble::ACK_ID_BULK,
            status: packet::ACK_OK,
            value_u32: None,
        };

        let mut chunks = Vec::new();
        loop {
            match push.on_ack(&ok) {
                PushStep::Write(f) if f[0] == ble::OP_DATA => chunks.push(f[3..].to_vec()),
                PushStep::Write(f) => {
                    assert_eq!(f, vec![ble::OP_END]);
                    break;
                }
                other => panic!(
                    "expected a write, got {}",
                    match other {
                        PushStep::Done => "done",
                        _ => "a failure",
                    }
                ),
            }
        }
        assert_eq!(chunks.len(), 2);
        assert!(chunks.iter().all(|c| c.len() == ble::BULK_DATA_MAX));
        assert_eq!(chunks.concat(), data);
        assert!(matches!(push.on_ack(&ok), PushStep::Done));
    }

    /// An empty config still has to walk BEGIN -> END rather than stall: the
    /// board opened a transfer on the BEGIN and would sit on it otherwise.
    #[test]
    fn config_push_with_no_bytes_still_closes_the_transfer() {
        let mut push = ConfigPush::new(Vec::new());
        let begin = push.start().unwrap();
        assert_eq!(begin[2..6], 0u32.to_le_bytes());
        let ok = Ack {
            id: ble::ACK_ID_BULK,
            status: packet::ACK_OK,
            value_u32: None,
        };
        assert!(matches!(push.on_ack(&ok), PushStep::Write(f) if f == vec![ble::OP_END]));
        assert!(matches!(push.on_ack(&ok), PushStep::Done));
    }

    /// A NAK at any point fails the push, and the same status on the final
    /// op is blamed on the file's content - that is where the board parses
    /// it, so that is the only place its values can be refused.
    #[test]
    fn config_push_fails_on_nak() {
        let nak = |status| Ack {
            id: ble::ACK_ID_BULK,
            status,
            value_u32: None,
        };

        let mut push = ConfigPush::new(vec![1, 2, 3]);
        push.start().unwrap();
        assert!(matches!(
            push.on_ack(&nak(packet::ACK_BAD_VALUE)),
            PushStep::Fail(m) if m.contains("bad value or size")
        ));

        // A transfer the board would not even open: the console has one.
        let mut push = ConfigPush::new(vec![1, 2, 3]);
        push.start().unwrap();
        assert!(matches!(
            push.on_ack(&nak(ble::ACK_BAD_STATE)),
            PushStep::Fail(m) if m.contains("another transfer")
        ));

        let mut push = ConfigPush::new(vec![1, 2, 3]);
        push.start().unwrap();
        let ok = Ack {
            id: ble::ACK_ID_BULK,
            status: packet::ACK_OK,
            value_u32: Some(0),
        };
        push.on_ack(&ok); // data chunk
        push.on_ack(&ok); // OP_END
        assert!(matches!(
            push.on_ack(&nak(packet::ACK_BAD_VALUE)),
            PushStep::Fail(m) if m.contains("rejected the config")
        ));
    }

    /// A board that has never been named still sends a name - its
    /// address-derived one - so the only values that decode to nothing here
    /// are a board that sent nothing and a value that is not text.
    #[test]
    fn name_event_takes_a_name_and_nothing_else() {
        assert!(matches!(
            name_event(b"ws3gps-sky-1"),
            Some(BleEvent::Name(n)) if n == "ws3gps-sky-1"
        ));
        assert!(matches!(
            name_event(b"ws3gps-5047"),
            Some(BleEvent::Name(n)) if n == "ws3gps-5047"
        ));
        assert!(name_event(b"").is_none());
        assert!(name_event(&[0xFF, 0xFE]).is_none());
    }

    #[test]
    fn settings_event_rejects_a_newer_layout() {
        let good = Settings {
            pwr_en: true,
            sleep_interval_s: 30,
            ..Settings::default()
        };
        assert!(matches!(
            settings_event(&good.encode()),
            BleEvent::Settings(s) if s == good
        ));

        let mut newer = good.encode();
        newer[0] = ble::SETTINGS_VERSION + 1;
        assert!(matches!(
            settings_event(&newer),
            BleEvent::SettingsUnsupported
        ));
    }
}
