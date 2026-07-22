//! BLE central for the ESP32-C3 GPS beacon (esp32c3-gps firmware).
//!
//! Mirrors the gps source design: a background worker owns the platform BLE
//! stack and talks to the UI over channels. Events flow UI-ward (fixes, acks,
//! status), commands flow worker-ward (connect, config writes). Desktop uses
//! btleplug; Android drives the platform Bluetooth API through a small dex
//! shim loaded at runtime (see android.rs).
//!
//! The wire protocol lives in the shared gps-proto crate.

use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use gps_proto::packet::{self, Ack};
use midair_proto::{ble, link};
pub use gps_proto::packet::PositionPacket;
pub use midair_proto::ble::Settings;
pub use midair_proto::link::Telemetry;

#[cfg(not(target_os = "android"))]
mod desktop;

#[cfg(target_os = "android")]
mod android;

/// One board seen while scanning, for the Beacon page's device picker.
///
/// Every board runs the same firmware and so advertises the same name, which
/// makes `name` near-useless for telling two apart - the address is the
/// identity, and the readable label comes from the nicknames in the app config.
/// `rssi` is what actually distinguishes them in the field: the board in your
/// hand is the loud one.
#[derive(Clone, Debug, PartialEq)]
pub struct DiscoveredDevice {
    pub address: String,
    pub name: Option<String>,
    pub rssi: Option<i16>,
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
    Remote {
        src: u8,
        rssi: i16,
        packet: PositionPacket,
    },
    /// A config ack: the device confirmed (or rejected) a setting.
    Ack(Ack),
    /// Board telemetry (LoRa link, GPS, SD) from the esp32c6-gps board.
    Telemetry(Telemetry),
    /// The latest WIO status/log line (ASCII) relayed by the board.
    Log(String),
    /// The board's own view of its power and sleep settings, read on connect
    /// and notified on every change - including changes the board makes by
    /// itself, such as clamping an interval. This, not the UI, is the
    /// authority on what the board is set to.
    Settings(Settings),
    /// The settings blob did not decode: the board's layout version is newer
    /// than this build knows. Its settings are unreadable, not defaulted.
    SettingsUnsupported,
    /// A [`BleCommand::PushConfig`] finished: the board applied and stored the
    /// config, or why it did not.
    ConfigPushed(Result<String, String>),
}

/// UI -> worker.
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
    /// characteristic. The board forwards it to the WIO-E5, which applies it
    /// live and stores it. The outcome comes back as
    /// [`BleEvent::ConfigPushed`].
    PushConfig(Vec<u8>),
    /// Drop the connection and stay idle until the next `Connect`.
    Disconnect,
}

/// One config-characteristic write, `[id, len, value...]`. The gps-proto
/// notify interval and the esp32c6-gps board ids share the characteristic and
/// differ only in the id and the width of the value.
#[derive(Clone, Copy, Debug)]
pub enum ConfigWrite {
    /// Position notify interval in ms (gps-proto `CFG_UPDATE_INTERVAL_MS`).
    Interval(u32),
    /// A board on/off setting: the power rail, WIO sleep or GPS backup mode.
    Flag { id: u8, on: bool },
    /// A board interval in seconds: the wake-check cadence.
    Seconds { id: u8, secs: u32 },
}

impl ConfigWrite {
    /// The encoded write and its length.
    pub fn encode(&self) -> ([u8; 6], usize) {
        match *self {
            ConfigWrite::Interval(ms) => {
                packet::encode_config(packet::ConfigCommand::UpdateIntervalMs(ms))
            }
            ConfigWrite::Flag { id, on } => {
                let mut b = [0u8; 6];
                b[0] = id;
                b[1] = 1;
                b[2] = on as u8;
                (b, 3)
            }
            ConfigWrite::Seconds { id, secs } => {
                let mut b = [0u8; 6];
                b[0] = id;
                b[1] = 4;
                b[2..6].copy_from_slice(&secs.to_le_bytes());
                (b, 6)
            }
        }
    }
}

/// How long a push waits for a bulk ack before giving up. Each op is one UART
/// round-trip inside the board (at most a 2 s WIO timeout), so a link this
/// quiet is dead, not slow.
const PUSH_ACK_TIMEOUT: Duration = Duration::from_secs(10);

/// One radio-config push through the board's bulk characteristic, advanced one
/// ack at a time: OP_BEGIN opens the transfer, each OP_DATA carries one chunk,
/// and OP_END has the WIO verify and apply the file. The board forwards every
/// op to the WIO over their UART link before acking it (id
/// [`ble::ACK_ID_BULK`] on the ack characteristic), so the next op is only
/// sent once the previous ack
/// is in - pacing by ack is what keeps this from overrunning that link.
struct ConfigPush {
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
/// the OP_END ack: the WIO only parses the file there, so a WIO rejection at
/// that point is usually the file's content rather than the link.
fn push_error(status: u8, at_end: bool) -> String {
    match status {
        ble::ACK_WIO_ERROR if at_end => "the WIO-E5 rejected the config: check for a value \
                                         out of range or a string that is not one of the choices"
            .to_string(),
        packet::ACK_BAD_VALUE => "the board rejected it (bad value or size)".to_string(),
        ble::ACK_BAD_STATE => "another transfer is already running".to_string(),
        ble::ACK_WIO_ERROR => "the board could not reach the WIO-E5 (link error)".to_string(),
        ble::ACK_WIO_TIMEOUT => "the board could not reach the WIO-E5 (no reply)".to_string(),
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

/// Decode a remote-position blob (`[src u8, rssi i16le, PositionPacket]`) from
/// the remote characteristic into a [`BleEvent::Remote`]. `None` for a short
/// blob, an undecodable packet, or `src` 0 - the board notifies the remote
/// slot with a zero source when nothing has been heard yet, and 0 is the local
/// GPS in any case, delivered on the position characteristic instead.
fn remote_event(bytes: &[u8]) -> Option<BleEvent> {
    if bytes.len() < midair_proto::ble::REMOTE_LEN {
        return None;
    }
    let src = bytes[0];
    if src == 0 {
        return None;
    }
    let rssi = i16::from_le_bytes([bytes[1], bytes[2]]);
    let packet = PositionPacket::decode(&bytes[3..])?;
    Some(BleEvent::Remote { src, rssi, packet })
}

/// The UI's handle to the BLE worker.
pub struct BleHandle {
    pub events: Receiver<BleEvent>,
    pub commands: Sender<BleCommand>,
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
        let mut blob = [0u8; ble::REMOTE_LEN];
        blob[0] = 7; // src address
        blob[1..3].copy_from_slice(&(-92i16).to_le_bytes());
        blob[3..].copy_from_slice(&packet.encode());

        match remote_event(&blob) {
            Some(BleEvent::Remote { src, rssi, packet: p }) => {
                assert_eq!(src, 7);
                assert_eq!(rssi, -92);
                assert_eq!(p, packet);
            }
            _ => panic!("expected a remote event"),
        }

        // Source 0 is the local GPS / "nothing heard yet"; not a remote.
        blob[0] = 0;
        assert!(remote_event(&blob).is_none());
        // A short blob decodes to nothing rather than panicking.
        assert!(remote_event(&blob[..ble::REMOTE_LEN - 1]).is_none());
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

    /// A NAK at any point fails the push, and a WIO NAK on the final op is
    /// blamed on the file's content rather than the link.
    #[test]
    fn config_push_fails_on_nak() {
        let nak = Ack {
            id: ble::ACK_ID_BULK,
            status: ble::ACK_WIO_ERROR,
            value_u32: None,
        };

        let mut push = ConfigPush::new(vec![1, 2, 3]);
        push.start().unwrap();
        assert!(matches!(
            push.on_ack(&nak),
            PushStep::Fail(m) if m.contains("link error")
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
            push.on_ack(&nak),
            PushStep::Fail(m) if m.contains("rejected the config")
        ));
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
