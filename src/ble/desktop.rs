//! Desktop BLE worker built on btleplug (bluez on Linux).
//!
//! Runs a single-threaded tokio runtime on a dedicated thread. The worker is
//! a reconnect loop: scan (filtered by the GPS service UUID, or pinned to a
//! MAC), connect, subscribe to position + ack notifications, then pump
//! notifications and commands until something breaks, and start over.
//!
//! Every step that can block is wrapped in [`while_wanted`], so a press does
//! not have to wait out a connect attempt that may sit for tens of seconds.

use std::future::Future;
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

use btleplug::api::{
    Central, CharPropFlags, Manager as _, Peripheral as _, PeripheralProperties, ScanFilter,
    WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::future::Either;
use futures::StreamExt;
use gps_proto::packet::{self, PositionPacket};
use midair_proto::ble;
use midair_proto::link::Telemetry;
use uuid::Uuid;

use super::{
    node_ping_event, radio_config_event, remote_event, settings_event, Aborted, BleEvent,
    BleHandle, BleRequest, DiscoveredDevice, Ended, Inbox, Interrupt, PushStep, Reporter, Target,
    Wanted, CMD_POLL, PUSH_ACK_TIMEOUT,
};

const SERVICE_UUID: Uuid = Uuid::from_u128(packet::SERVICE_UUID_U128);
const POSITION_UUID: Uuid = Uuid::from_u128(packet::POSITION_UUID_U128);
const CONFIG_UUID: Uuid = Uuid::from_u128(packet::CONFIG_UUID_U128);
const ACK_UUID: Uuid = Uuid::from_u128(packet::ACK_UUID_U128);
// Board-status characteristics served by the Wio-S3 board on top of the
// shared gps-proto service. Absent on the older esp32c3 beacon, so treated as
// optional (see `connected`).
const TELEMETRY_UUID: Uuid = Uuid::from_u128(ble::TELEMETRY_UUID_U128);
const LOG_UUID: Uuid = Uuid::from_u128(ble::LOG_UUID_U128);
const SETTINGS_UUID: Uuid = Uuid::from_u128(ble::SETTINGS_UUID_U128);
// The board's current radio config, read on connect and notified on change.
// Wio-S3 only, so optional like the other board-status characteristics.
const RADIO_CONFIG_UUID: Uuid = Uuid::from_u128(ble::RADIO_CONFIG_UUID_U128);
// Remote-position characteristic: the connected board relays a LoRa node's
// position here, tagged with the node's address. Absent on the esp32c3 beacon,
// so optional like the other board-status characteristics.
const REMOTE_UUID: Uuid = Uuid::from_u128(ble::REMOTE_UUID_U128);
// The same for a node that is up but has no fix to report. Newer than the
// remote-position characteristic, so a board may serve one without the other.
const NODE_PING_UUID: Uuid = Uuid::from_u128(ble::NODE_PING_UUID_U128);
// Bulk-transfer characteristic (radio TOML config), Wio-S3 only.
const BULK_UUID: Uuid = Uuid::from_u128(ble::BULK_UUID_U128);

pub fn spawn(ctx: egui::Context) -> BleHandle {
    let (event_tx, event_rx) = channel();
    let (cmd_tx, cmd_rx) = channel();

    std::thread::spawn(move || {
        let report = Reporter::new(ctx, event_tx);
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                report.status(format!("tokio runtime failed: {e}"));
                return;
            }
        };
        rt.block_on(worker(&report, cmd_rx));
    });

    BleHandle {
        events: event_rx,
        commands: cmd_tx,
    }
}

/// Await `op` while still servicing commands, and give up on it the moment the
/// session is stopped or superseded. The operation is dropped where it stands,
/// which is the point: a btleplug connect can sit for tens of seconds, and a
/// Disconnect that waits it out is a delay, not a disconnect.
async fn while_wanted<T>(
    op: impl Future<Output = T>,
    report: &Reporter,
    inbox: &mut Inbox<'_>,
    target: &Target,
) -> Result<T, Aborted> {
    futures::pin_mut!(op);
    loop {
        let tick = tokio::time::sleep(CMD_POLL);
        futures::pin_mut!(tick);
        match futures::future::select(op.as_mut(), tick).await {
            Either::Left((out, _)) => return Ok(out),
            Either::Right(_) => inbox.check(report, target)?,
        }
    }
}

async fn worker(report: &Reporter, cmd_rx: Receiver<BleRequest>) {
    let manager = match Manager::new().await {
        Ok(m) => m,
        Err(e) => {
            report.status(format!("BLE unavailable: {e}"));
            return;
        }
    };

    let mut wanted = Wanted::idle();
    let mut writes = Vec::new();
    let mut push = None;
    let mut inbox = Inbox {
        rx: &cmd_rx,
        wanted: &mut wanted,
        writes: &mut writes,
        push: &mut push,
    };

    loop {
        if inbox.drain(report).is_err() {
            return; // UI has gone away.
        }
        if !inbox.wanted.connect && !inbox.wanted.scan {
            // A push cannot go anywhere without a link; fail it rather than
            // hold the Radio page in "sending" until some later connect.
            inbox.fail_push(report);
            tokio::time::sleep(CMD_POLL).await;
            continue;
        }

        let adapter = match manager.adapters().await.ok().and_then(|a| a.into_iter().next()) {
            Some(a) => a,
            None => {
                report.status("no Bluetooth adapter found");
                tokio::time::sleep(Duration::from_secs(3)).await;
                continue;
            }
        };

        if inbox.wanted.scan {
            if let Err(e) = discover(&adapter, report, &mut inbox).await {
                report.status(format!("{e}; retrying"));
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            continue;
        }

        // One connect attempt; on any failure fall through, wait, retry.
        match session(&adapter, report, &mut inbox).await {
            // Stopped or superseded by the UI: whatever it wants now is
            // served on the next pass, with no retry pause in between.
            Ok(()) | Err(Ended::Quietly) => {}
            Err(Ended::Failed(e)) => {
                report.send(BleEvent::Connected(false));
                report.status(format!("{e}; retrying"));
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
        // A push does not survive its session: half a transfer is dropped by
        // the board, and silently restarting one on reconnect would resend a
        // config the user asked for once.
        inbox.fail_push(report);
    }
}

/// Scan without connecting, reporting every board that answers, until the UI
/// asks for something else. Unlike the scan inside [`session`] this never stops
/// at the first hit: the picker wants the whole list, and a board that starts
/// advertising late still has to appear.
///
/// Boards are re-reported for as long as the scan runs rather than only on
/// first sight, so the signal strength the picker shows keeps up with a board
/// being carried around.
async fn discover(
    adapter: &Adapter,
    report: &Reporter,
    inbox: &mut Inbox<'_>,
) -> Result<(), String> {
    report.status("scanning for boards...");
    adapter
        .start_scan(ScanFilter {
            services: vec![SERVICE_UUID],
        })
        .await
        .map_err(|e| format!("scan failed: {e}"))?;

    let result = loop {
        if inbox.drain(report).is_err() || !inbox.wanted.scan {
            break Ok(());
        }
        let peripherals = match adapter.peripherals().await {
            Ok(p) => p,
            Err(e) => break Err(format!("scan failed: {e}")),
        };
        for p in peripherals {
            // The adapter remembers devices from earlier scans, so match on
            // what is being advertised now rather than trusting the cache.
            if let Ok(Some(props)) = p.properties().await {
                if is_beacon(&props) {
                    report.send(BleEvent::Discovered(DiscoveredDevice {
                        address: p.address().to_string(),
                        name: props.local_name.clone(),
                        rssi: props.rssi,
                    }));
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    };

    let _ = adapter.stop_scan().await;
    if result.is_ok() {
        report.status("scan stopped");
    }
    result
}

/// Scan for the beacon, connect, run one connected session, then always
/// disconnect so the next reconnect starts from clean device state.
///
/// The board this is for is fixed at the start ([`Wanted::target`]) rather
/// than read live: a press that changes it ends this session instead of
/// quietly redirecting it, so the connect, the subscribes and the state the UI
/// is shown all belong to one board.
async fn session(adapter: &Adapter, report: &Reporter, inbox: &mut Inbox<'_>) -> Result<(), Ended> {
    let target = inbox.wanted.target();

    report.status(if target.chase {
        "waiting for a wake window..."
    } else {
        "scanning for GPS beacon..."
    });
    let filter = ScanFilter {
        services: vec![SERVICE_UUID],
    };
    adapter
        .start_scan(filter)
        .await
        .map_err(|e| format!("scan failed: {e}"))?;

    // Poll discovered peripherals until one matches (by MAC when pinned,
    // otherwise by advertised service or name).
    let peripheral = loop {
        if inbox.check(report, &target).is_err() {
            let _ = adapter.stop_scan().await;
            return Err(Ended::Quietly);
        }
        if let Some(p) = find_match(adapter, target.mac.as_deref()).await {
            break p;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    };
    let _ = adapter.stop_scan().await;

    // Clear any half-open connection left from a previous session. bluez keeps
    // the device object across disconnects; connecting to one it still believes
    // is connected wedges (the central never completes the link) until the
    // process restarts, so force a clean slate first.
    if peripheral.is_connected().await.unwrap_or(false) {
        let _ = peripheral.disconnect().await;
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    // Run the connected session, then unconditionally disconnect. bluez does
    // not tear the link down for us on error, and a lingering half-open device
    // is exactly what blocks the next reconnect. That includes a session cut
    // short mid-connect: the attempt we dropped may still land.
    let result = connected(&peripheral, report, inbox, &target).await;
    let _ = peripheral.disconnect().await;
    report.send(BleEvent::Connected(false));
    result
}

/// Connect to `peripheral`, subscribe, and pump notifications until the UI
/// stops wanting this session ([`Ended::Quietly`]) or the link fails
/// ([`Ended::Failed`]). The caller disconnects the peripheral afterward
/// regardless of the outcome.
async fn connected(
    peripheral: &Peripheral,
    report: &Reporter,
    inbox: &mut Inbox<'_>,
    target: &Target,
) -> Result<(), Ended> {
    let addr = peripheral.address();
    report.status(format!("connecting to {addr}..."));
    while_wanted(peripheral.connect(), report, inbox, target)
        .await?
        .map_err(|e| format!("connect failed: {e}"))?;
    while_wanted(peripheral.discover_services(), report, inbox, target)
        .await?
        .map_err(|e| format!("discovery failed: {e}"))?;

    let chars = peripheral.characteristics();
    let position = chars
        .iter()
        .find(|c| c.uuid == POSITION_UUID && c.properties.contains(CharPropFlags::NOTIFY))
        .cloned()
        .ok_or("position characteristic missing")?;
    let ack = chars
        .iter()
        .find(|c| c.uuid == ACK_UUID)
        .cloned()
        .ok_or("ack characteristic missing")?;
    let config = chars
        .iter()
        .find(|c| c.uuid == CONFIG_UUID)
        .cloned()
        .ok_or("config characteristic missing")?;
    // Optional board-status characteristics (Wio-S3 only).
    let telemetry = chars
        .iter()
        .find(|c| c.uuid == TELEMETRY_UUID && c.properties.contains(CharPropFlags::NOTIFY))
        .cloned();
    let log = chars
        .iter()
        .find(|c| c.uuid == LOG_UUID && c.properties.contains(CharPropFlags::NOTIFY))
        .cloned();
    let settings = chars.iter().find(|c| c.uuid == SETTINGS_UUID).cloned();
    let radio_config = chars.iter().find(|c| c.uuid == RADIO_CONFIG_UUID).cloned();
    let remote = chars
        .iter()
        .find(|c| c.uuid == REMOTE_UUID && c.properties.contains(CharPropFlags::NOTIFY))
        .cloned();
    let node_ping = chars
        .iter()
        .find(|c| c.uuid == NODE_PING_UUID && c.properties.contains(CharPropFlags::NOTIFY))
        .cloned();
    let bulk = chars.iter().find(|c| c.uuid == BULK_UUID).cloned();

    peripheral
        .subscribe(&position)
        .await
        .map_err(|e| format!("subscribe failed: {e}"))?;
    peripheral
        .subscribe(&ack)
        .await
        .map_err(|e| format!("subscribe failed: {e}"))?;
    if let Some(c) = &telemetry {
        let _ = peripheral.subscribe(c).await;
    }
    if let Some(c) = &log {
        let _ = peripheral.subscribe(c).await;
    }
    if let Some(c) = &remote {
        let _ = peripheral.subscribe(c).await;
    }
    if let Some(c) = &node_ping {
        let _ = peripheral.subscribe(c).await;
    }
    // Subscribe before the read below, so a change the board makes between the
    // two (a clamped interval, say) still reaches us.
    if let Some(c) = &settings {
        let _ = peripheral.subscribe(c).await;
    }
    if let Some(c) = &radio_config {
        let _ = peripheral.subscribe(c).await;
    }

    let mut notifications = peripheral
        .notifications()
        .await
        .map_err(|e| format!("notification stream failed: {e}"))?;

    // A press during the subscribes above wins: the UI is never told about a
    // link it has already asked to be rid of.
    inbox.check(report, target)?;
    report.send(BleEvent::Connected(true));
    report.status(format!("connected to {addr}"));

    // Populate the board controls from the board itself rather than assuming
    // defaults for settings it holds in flash across power cycles.
    if let Some(c) = &settings {
        match peripheral.read(c).await {
            Ok(v) => {
                report.send(settings_event(&v));
            }
            Err(e) => {
                report.status(format!("settings read failed: {e}"));
            }
        }
    }
    // Same for the radio config. A board that has not reported one yet reads
    // back all-zero (`radio_config_event` returns `None`); the notify that
    // follows once it does carries the real value.
    if let Some(c) = &radio_config {
        if let Ok(v) = peripheral.read(c).await {
            if let Some(e) = radio_config_event(&v) {
                report.send(e);
            }
        }
    }

    let mut since_check = 0u32;
    // When the outstanding bulk op's ack must have arrived by; meaningless
    // while no push is running.
    let mut push_deadline = Instant::now();
    loop {
        // Apply queued config writes.
        for w in inbox.writes.drain(..) {
            let (buf, n) = w.encode();
            if let Err(e) = peripheral
                .write(&config, &buf[..n], WriteType::WithResponse)
                .await
            {
                report.status(format!("config write failed: {e}"));
            }
        }

        // Start a queued config push, and give up on one whose ack never came.
        if let Some(p) = inbox.push.as_mut() {
            if let Some(frame) = p.start() {
                match &bulk {
                    Some(c) => {
                        push_deadline = Instant::now() + PUSH_ACK_TIMEOUT;
                        if let Err(e) = peripheral
                            .write(c, &frame, WriteType::WithResponse)
                            .await
                        {
                            *inbox.push = None;
                            report.send(BleEvent::ConfigPushed(Err(format!(
                                "bulk write failed: {e}"
                            ))));
                        }
                    }
                    None => {
                        *inbox.push = None;
                        report.send(BleEvent::ConfigPushed(Err(
                            "this board has no bulk-transfer characteristic".to_string(),
                        )));
                    }
                }
            } else if Instant::now() >= push_deadline {
                *inbox.push = None;
                // Best effort: free the board's transfer state for a retry.
                if let Some(c) = &bulk {
                    let _ = peripheral
                        .write(c, &[ble::OP_ABORT], WriteType::WithResponse)
                        .await;
                }
                report.send(BleEvent::ConfigPushed(Err(
                    "no ack from the board".to_string(),
                )));
            }
        }

        // Wait briefly for a notification, then service commands again.
        match tokio::time::timeout(Duration::from_millis(250), notifications.next()).await {
            Ok(Some(n)) => {
                if n.uuid == POSITION_UUID {
                    if let Some(p) = PositionPacket::decode(&n.value) {
                        report.send(BleEvent::Fix(p));
                    }
                } else if n.uuid == ACK_UUID {
                    if let Some(a) = packet::parse_ack(&n.value) {
                        if a.id == ble::ACK_ID_BULK {
                            // A bulk ack paces the running push; it is not a
                            // setting ack, so it never reaches the Beacon page.
                            if let Some(p) = inbox.push.as_mut() {
                                push_deadline = Instant::now() + PUSH_ACK_TIMEOUT;
                                match p.on_ack(&a) {
                                    PushStep::Write(frame) => {
                                        // `bulk` exists: the push could not
                                        // have started without it.
                                        if let Some(c) = &bulk {
                                            if let Err(e) = peripheral
                                                .write(c, &frame, WriteType::WithResponse)
                                                .await
                                            {
                                                *inbox.push = None;
                                                report.send(BleEvent::ConfigPushed(Err(
                                                    format!("bulk write failed: {e}"),
                                                )));
                                            }
                                        }
                                    }
                                    PushStep::Done => {
                                        *inbox.push = None;
                                        report.send(BleEvent::ConfigPushed(Ok(
                                            "Config sent. The board applied and stored it."
                                                .to_string(),
                                        )));
                                    }
                                    PushStep::Fail(e) => {
                                        *inbox.push = None;
                                        report.send(BleEvent::ConfigPushed(Err(e)));
                                    }
                                }
                            }
                        } else {
                            report.send(BleEvent::Ack(a));
                        }
                    }
                } else if n.uuid == TELEMETRY_UUID {
                    if let Some(t) = Telemetry::decode(&n.value) {
                        report.send(BleEvent::Telemetry(t));
                    }
                } else if n.uuid == LOG_UUID {
                    report.send(BleEvent::Log(String::from_utf8_lossy(&n.value).into_owned()));
                } else if n.uuid == REMOTE_UUID {
                    if let Some(e) = remote_event(&n.value) {
                        report.send(e);
                    }
                } else if n.uuid == NODE_PING_UUID {
                    if let Some(e) = node_ping_event(&n.value) {
                        report.send(e);
                    }
                } else if n.uuid == SETTINGS_UUID {
                    report.send(settings_event(&n.value));
                } else if n.uuid == RADIO_CONFIG_UUID {
                    if let Some(e) = radio_config_event(&n.value) {
                        report.send(e);
                    }
                }
            }
            Ok(None) => return Err("connection lost".into()),
            Err(_) => {
                // Timeout: periodically confirm the link is still up (the
                // stream does not always end on disconnect).
                since_check += 1;
                if since_check >= 8 {
                    since_check = 0;
                    if !peripheral.is_connected().await.unwrap_or(false) {
                        return Err("connection lost".into());
                    }
                }
            }
        }

        if inbox.drain(report).is_err() {
            return Err(Ended::Quietly);
        }
        // Neither is a failure, so neither costs the retry pause: the caller
        // drops the link and the worker serves the new request straight away.
        match inbox.wanted.interrupt(target) {
            Some(Interrupt::Stopped) => {
                report.status("disconnected");
                return Err(Ended::Quietly);
            }
            Some(Interrupt::Superseded) => {
                report.status("starting over");
                return Err(Ended::Quietly);
            }
            None => {}
        }
    }
}

/// Whether this advertisement is one of our boards: it offers the GPS service,
/// or it goes by one of the firmwares' names.
///
/// The name is the fallback, not the test: it is scan-response data, and the
/// two firmwares do not share one. The service UUID is in the advertisement
/// itself and is the same on both, which is why renaming a board cannot lose
/// it.
fn is_beacon(props: &PeripheralProperties) -> bool {
    let name = props.local_name.as_deref();
    props.services.contains(&SERVICE_UUID)
        || name == Some(packet::DEVICE_NAME)
        || name == Some(ble::DEVICE_NAME)
}

/// Find a discovered peripheral matching the pinned MAC (case-insensitive) or,
/// with no MAC, the first board that answers.
async fn find_match(adapter: &Adapter, mac: Option<&str>) -> Option<Peripheral> {
    let peripherals = adapter.peripherals().await.ok()?;
    for p in peripherals {
        if let Some(mac) = mac {
            if p.address().to_string().eq_ignore_ascii_case(mac) {
                return Some(p);
            }
            continue;
        }
        if let Ok(Some(props)) = p.properties().await {
            if is_beacon(&props) {
                return Some(p);
            }
        }
    }
    None
}
