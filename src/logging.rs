//! Logging mode: every report from every source appended to a wide CSV, and
//! the series behind the Logging page's graph.
//!
//! A row is written whenever a source *reports* - a phone fix, the connected
//! board's fix, a remote node's position or ping, a telemetry blob - rather
//! than on a timer. That is what keeps the pairing the whole feature exists
//! for: a node's report carries both the signal it came in at and the position
//! it came from, so distance and RSSI land on the same row and can be plotted
//! against each other. A periodic sample would either miss reports or repeat
//! them, and either way would have to guess which RSSI belonged to which fix.
//!
//! Columns a given row has nothing to say about are left empty rather than
//! zeroed: a spreadsheet reads an empty cell as "no reading", and 0 dBm is a
//! reading.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use egui::Color32;

use crate::config::{remote_color, MarkerColors};

/// Rows kept in memory for the graph. The file on disk keeps everything; this
/// is only what can be drawn, and a plot of more points than the screen has
/// pixels is not a better plot. At the notify rates the board uses this is
/// several hours of recording.
const MAX_ROWS: usize = 50_000;

/// Where a logged row came from. The CSV writes this as a `source` column plus
/// an `addr` one, so a remote node stays one source per address without the
/// column count depending on how many nodes are on the air.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum LogSource {
    /// The control device's own GNSS.
    Phone,
    /// The connected board's own GPS.
    Board,
    /// A remote node heard over LoRa, by address.
    Node(u8),
    /// The connected board's link/health telemetry, which has no position.
    Telemetry,
}

impl LogSource {
    /// The `source` column, and the label a graph series goes by.
    pub fn as_str(self) -> &'static str {
        match self {
            LogSource::Phone => "phone",
            LogSource::Board => "board",
            LogSource::Node(_) => "node",
            LogSource::Telemetry => "telemetry",
        }
    }

    /// The `addr` column: a remote node's LoRa address, empty for the rest.
    fn addr(self) -> String {
        match self {
            LogSource::Node(addr) => addr.to_string(),
            _ => String::new(),
        }
    }

    /// The color this source draws in, matching the map: the same three
    /// groups (you, the connected board, the nodes) read the same way on the
    /// graph as they do on the map. Telemetry describes the connected board
    /// and shares its color.
    pub fn color(self, colors: MarkerColors) -> Color32 {
        match self {
            LogSource::Phone => colors.track,
            LogSource::Board | LogSource::Telemetry => colors.fixed,
            LogSource::Node(addr) => remote_color(addr),
        }
    }
}

/// One logged report. Every measurement is optional because every source
/// reports a different subset, and an absent one must not read as a zero.
#[derive(Clone, Copy, Debug)]
pub struct LogRow {
    pub time: SystemTime,
    pub source: LogSource,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub alt_m: Option<f64>,
    pub speed_mps: Option<f64>,
    pub course_deg: Option<f64>,
    pub sats: Option<u8>,
    /// Whether the reporting receiver had a fix. A node ping is a report
    /// without one, which is the difference between "out of range" and
    /// "up, but looking for the sky".
    pub fix: Option<bool>,
    pub rssi_dbm: Option<i16>,
    pub snr_db: Option<f64>,
    /// Great-circle distance from the control device to this row's position.
    /// The plot of signal against range is this against `rssi_dbm`.
    pub dist_user_m: Option<f64>,
    /// The same to the fixed reference coordinate from the config, when one
    /// is set. Lets a run be measured against a surveyed point rather than
    /// against a control device that is itself moving.
    pub dist_ref_m: Option<f64>,
    /// Where the control device was when the row was recorded, so a distance
    /// column can be recomputed from the file afterwards.
    pub user_lat: Option<f64>,
    pub user_lon: Option<f64>,
    pub rx_count: Option<u32>,
    pub tx_count: Option<u32>,
    pub secs_since_rx: Option<u16>,
}

impl LogRow {
    /// A row with nothing but its source and time filled in; the caller sets
    /// whatever that report actually carried.
    pub fn new(source: LogSource, time: SystemTime) -> Self {
        Self {
            time,
            source,
            lat: None,
            lon: None,
            alt_m: None,
            speed_mps: None,
            course_deg: None,
            sats: None,
            fix: None,
            rssi_dbm: None,
            snr_db: None,
            dist_user_m: None,
            dist_ref_m: None,
            user_lat: None,
            user_lon: None,
            rx_count: None,
            tx_count: None,
            secs_since_rx: None,
        }
    }

    /// Seconds since the epoch, the graph's time axis and the CSV's `unix_s`.
    pub fn unix_s(&self) -> f64 {
        self.time
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or_default()
    }

    /// This row as a CSV line, in [`CSV_HEADER`] order.
    fn csv_line(&self) -> String {
        // Coordinates get 7 decimals (the resolution the packets carry);
        // everything else 2, which is past the precision of the reading.
        let mut out = String::with_capacity(160);
        out.push_str(&iso8601(self.time));
        out.push(',');
        out.push_str(&format!("{:.3}", self.unix_s()));
        out.push(',');
        out.push_str(self.source.as_str());
        out.push(',');
        out.push_str(&self.source.addr());
        for v in [self.lat, self.lon] {
            out.push(',');
            if let Some(v) = v {
                out.push_str(&format!("{v:.7}"));
            }
        }
        for v in [self.alt_m, self.speed_mps, self.course_deg] {
            out.push(',');
            if let Some(v) = v {
                out.push_str(&format!("{v:.2}"));
            }
        }
        out.push(',');
        if let Some(v) = self.sats {
            out.push_str(&v.to_string());
        }
        out.push(',');
        if let Some(v) = self.fix {
            out.push_str(if v { "1" } else { "0" });
        }
        out.push(',');
        if let Some(v) = self.rssi_dbm {
            out.push_str(&v.to_string());
        }
        out.push(',');
        if let Some(v) = self.snr_db {
            out.push_str(&format!("{v:.2}"));
        }
        for v in [self.dist_user_m, self.dist_ref_m] {
            out.push(',');
            if let Some(v) = v {
                out.push_str(&format!("{v:.2}"));
            }
        }
        for v in [self.user_lat, self.user_lon] {
            out.push(',');
            if let Some(v) = v {
                out.push_str(&format!("{v:.7}"));
            }
        }
        for v in [self.rx_count, self.tx_count] {
            out.push(',');
            if let Some(v) = v {
                out.push_str(&v.to_string());
            }
        }
        out.push(',');
        if let Some(v) = self.secs_since_rx {
            out.push_str(&v.to_string());
        }
        out.push('\n');
        out
    }
}

/// The CSV header, and with it the column order every row is written in.
pub const CSV_HEADER: &str = "time_utc,unix_s,source,addr,lat,lon,alt_m,speed_mps,course_deg,\
sats,fix,rssi_dbm,snr_db,dist_user_m,dist_ref_m,user_lat,user_lon,rx_count,tx_count,secs_since_rx\n";

/// A plottable column. The graph picks one for each axis; time is the other
/// option on the X axis and is not a stat, since it is on every row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LogStat {
    /// Distance from the control device to the reporting source.
    Distance,
    /// Distance from the reporting source to the fixed reference coordinate.
    DistanceRef,
    Rssi,
    Snr,
    Speed,
    Sats,
    Altitude,
}

impl LogStat {
    /// Every stat, in the order the pickers list them.
    pub const ALL: [LogStat; 7] = [
        LogStat::Distance,
        LogStat::DistanceRef,
        LogStat::Rssi,
        LogStat::Snr,
        LogStat::Speed,
        LogStat::Sats,
        LogStat::Altitude,
    ];

    pub fn label(self) -> &'static str {
        match self {
            LogStat::Distance => "Distance to me",
            LogStat::DistanceRef => "Distance to reference",
            LogStat::Rssi => "RSSI",
            LogStat::Snr => "SNR",
            LogStat::Speed => "Speed",
            LogStat::Sats => "Satellites",
            LogStat::Altitude => "Altitude",
        }
    }

    /// Axis unit suffix, empty for a bare count.
    pub fn unit(self) -> &'static str {
        match self {
            LogStat::Distance | LogStat::DistanceRef | LogStat::Altitude => "m",
            LogStat::Rssi => "dBm",
            LogStat::Snr => "dB",
            LogStat::Speed => "m/s",
            LogStat::Sats => "",
        }
    }

    /// This stat's value on a row, or `None` when that row does not carry it.
    pub fn value(self, row: &LogRow) -> Option<f64> {
        match self {
            LogStat::Distance => row.dist_user_m,
            LogStat::DistanceRef => row.dist_ref_m,
            LogStat::Rssi => row.rssi_dbm.map(f64::from),
            LogStat::Snr => row.snr_db,
            LogStat::Speed => row.speed_mps,
            LogStat::Sats => row.sats.map(f64::from),
            LogStat::Altitude => row.alt_m,
        }
    }
}

/// What the graph's X axis measures. Time is the ordinary case; a stat on both
/// axes is the scatter that answers "how does the signal fall off with range".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LogAxis {
    Time,
    Stat(LogStat),
}

impl LogAxis {
    pub fn label(self) -> &'static str {
        match self {
            LogAxis::Time => "Time",
            LogAxis::Stat(s) => s.label(),
        }
    }

    /// The X value for a row: seconds into the plotted window for time (the
    /// caller subtracts the origin), or the stat's own value.
    pub fn value(self, row: &LogRow) -> Option<f64> {
        match self {
            LogAxis::Time => Some(row.unix_s()),
            LogAxis::Stat(s) => s.value(row),
        }
    }
}

/// The recorder: the open CSV file, and the rows kept for the graph.
///
/// The file is the record and the rows are the view of it. They are allowed to
/// disagree in one direction only - the file may hold more than memory does,
/// never less - so an export always prefers the file and falls back to the
/// rows only when there is no file to read.
#[derive(Default)]
pub struct Logger {
    rows: Vec<LogRow>,
    file: Option<File>,
    path: Option<PathBuf>,
    started: Option<SystemTime>,
    /// Rows dropped off the front of `rows` by [`MAX_ROWS`]. They are still in
    /// the file; the page says so rather than letting the graph quietly
    /// misrepresent the length of the run.
    dropped: usize,
    written: usize,
}

impl Logger {
    pub fn is_recording(&self) -> bool {
        self.file.is_some()
    }

    pub fn rows(&self) -> &[LogRow] {
        &self.rows
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn started(&self) -> Option<SystemTime> {
        self.started
    }

    pub fn dropped(&self) -> usize {
        self.dropped
    }

    /// Rows written to the file since the last [`Self::start`]. Counted per
    /// recording rather than per launch, so it answers "is this run getting
    /// anything" - which is what it is on screen for.
    pub fn written(&self) -> usize {
        self.written
    }

    /// Open `path` and start appending. An existing file is appended to rather
    /// than truncated - stopping and starting again is a pause, not a new run,
    /// and losing an hour of driving to a mis-tap is not worth the tidier
    /// file. The header is written only when the file is new or empty.
    pub fn start(&mut self, path: &str) -> Result<(), String> {
        let path = PathBuf::from(path);
        if path.as_os_str().is_empty() {
            return Err("No log file path".to_string());
        }
        let fresh = std::fs::metadata(&path)
            .map(|m| m.len() == 0)
            .unwrap_or(true);
        let mut file = File::options()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if fresh {
            file.write_all(CSV_HEADER.as_bytes())
                .map_err(|e| format!("{}: {e}", path.display()))?;
        }
        self.file = Some(file);
        self.path = Some(path);
        self.started = Some(SystemTime::now());
        self.written = 0;
        Ok(())
    }

    /// Close the file. The rows stay in memory so the graph survives a stop.
    pub fn stop(&mut self) {
        self.file = None;
    }

    /// Append a row: to the file when recording, and to the in-memory window
    /// either way.
    ///
    /// Every row is flushed as it is written. A phone can drop the app at any
    /// moment and a buffered tail would take the most recent - and most likely
    /// to matter - part of the run with it; the rates here are a handful of
    /// rows a second, so the cost is nothing.
    pub fn push(&mut self, row: LogRow) -> Result<(), String> {
        let mut err = None;
        if let Some(file) = &mut self.file {
            let line = row.csv_line();
            err = file
                .write_all(line.as_bytes())
                .and_then(|()| file.flush())
                .err()
                .map(|e| e.to_string());
            if err.is_none() {
                self.written += 1;
            }
        }
        self.rows.push(row);
        if self.rows.len() > MAX_ROWS {
            let excess = self.rows.len() - MAX_ROWS;
            self.rows.drain(..excess);
            self.dropped += excess;
        }
        match err {
            // A write that failed stops the recording rather than repeating
            // the same error once per report: the file is gone (a card pulled,
            // a directory removed) and every later row would fail the same way.
            Some(e) => {
                self.file = None;
                Err(e)
            }
            None => Ok(()),
        }
    }

    /// Drop the recorded rows. The file is left alone - it is on disk and is
    /// the user's; this only empties the graph.
    pub fn clear_rows(&mut self) {
        self.rows.clear();
        self.dropped = 0;
    }

    /// The CSV to hand to an export: the file when there is one to read (it
    /// holds the whole run), otherwise the rows still in memory.
    pub fn export_text(&self) -> Result<String, String> {
        if let Some(path) = &self.path {
            if let Ok(text) = std::fs::read_to_string(path) {
                return Ok(text);
            }
        }
        if self.rows.is_empty() {
            return Err("Nothing recorded yet".to_string());
        }
        let mut out = String::from(CSV_HEADER);
        for row in &self.rows {
            out.push_str(&row.csv_line());
        }
        Ok(out)
    }
}

/// `YYYY-MM-DDTHH:MM:SSZ` for a timestamp, the CSV's `time_utc`.
///
/// UTC, and said so in the column: the alternative is local time, which needs
/// the device's zone rules and makes a file recorded either side of a change
/// non-monotonic. `unix_s` beside it is what a plot should sort on regardless.
pub fn iso8601(t: SystemTime) -> String {
    let (y, mo, d, h, mi, s) = civil(t);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// `YYYYMMDD-HHMMSS` for a timestamp: a filename that sorts by time.
pub fn stamp(t: SystemTime) -> String {
    let (y, mo, d, h, mi, s) = civil(t);
    format!("{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}")
}

/// A default log filename for a run started at `t`.
pub fn default_log_name(t: SystemTime) -> String {
    format!("gps-log-{}.csv", stamp(t))
}

/// Break a timestamp into UTC year, month, day, hour, minute, second.
fn civil(t: SystemTime) -> (i64, u32, u32, u64, u64, u64) {
    let secs = match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        // Before 1970 only happens on a device with an unset clock; clamp
        // rather than render a negative date.
        Err(_) => 0,
    };
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400) as u64;
    let (y, mo, d) = civil_from_days(days);
    (y, mo, d, tod / 3600, (tod % 3600) / 60, tod % 60)
}

/// Days since 1970-01-01 to a proleptic Gregorian date (Howard Hinnant's
/// `civil_from_days`). Written out rather than pulled in with a date crate:
/// this is the only calendar arithmetic in the app, and it has to build for
/// Android as well.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    // Shift the epoch to 0000-03-01 so a leap day lands at the end of a year.
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // day of era, [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day of year (March 1 first)
    let mp = (5 * doy + 2) / 153; // month, [0, 11] with March = 0
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (y + i64::from(m <= 2), m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn iso_dates_round_the_calendar() {
        assert_eq!(iso8601(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601(at(86_399)), "1970-01-01T23:59:59Z");
        assert_eq!(iso8601(at(86_400)), "1970-01-02T00:00:00Z");
        // A leap day, and the day after it.
        assert_eq!(iso8601(at(951_782_400)), "2000-02-29T00:00:00Z");
        assert_eq!(iso8601(at(951_868_800)), "2000-03-01T00:00:00Z");
        // 1900 was not a leap year, 2024 was.
        assert_eq!(iso8601(at(1_709_164_800)), "2024-02-29T00:00:00Z");
        assert_eq!(iso8601(at(1_764_547_199)), "2025-11-30T23:59:59Z");
    }

    #[test]
    fn stamp_sorts_by_time() {
        assert_eq!(stamp(at(1_709_164_800)), "20240229-000000");
        assert!(stamp(at(1_709_164_800)) < stamp(at(1_709_251_200)));
        assert_eq!(default_log_name(UNIX_EPOCH), "gps-log-19700101-000000.csv");
    }

    #[test]
    fn a_clock_before_the_epoch_does_not_panic() {
        assert_eq!(
            iso8601(UNIX_EPOCH - Duration::from_secs(60)),
            "1970-01-01T00:00:00Z"
        );
    }

    #[test]
    fn empty_columns_stay_empty_rather_than_zero() {
        let mut row = LogRow::new(LogSource::Node(3), UNIX_EPOCH);
        row.rssi_dbm = Some(-92);
        let line = row.csv_line();
        let fields: Vec<&str> = line.trim_end().split(',').collect();
        assert_eq!(fields.len(), CSV_HEADER.trim_end().split(',').count());
        assert_eq!(fields[0], "1970-01-01T00:00:00Z");
        assert_eq!(fields[2], "node");
        assert_eq!(fields[3], "3");
        // lat/lon were never set, so nothing is claimed for them.
        assert_eq!(fields[4], "");
        assert_eq!(fields[5], "");
        assert_eq!(fields[11], "-92");
    }

    #[test]
    fn a_row_with_no_address_leaves_the_addr_column_empty() {
        let line = LogRow::new(LogSource::Telemetry, UNIX_EPOCH).csv_line();
        let fields: Vec<&str> = line.trim_end().split(',').collect();
        assert_eq!(fields[2], "telemetry");
        assert_eq!(fields[3], "");
    }

    #[test]
    fn stopping_and_starting_continues_the_same_file() {
        let path = std::env::temp_dir().join("gps-gui-rs-log-append-test.csv");
        let _ = std::fs::remove_file(&path);
        let path = path.to_str().unwrap().to_string();

        let mut log = Logger::default();
        log.start(&path).unwrap();
        assert!(log.is_recording());
        log.push(LogRow::new(LogSource::Phone, UNIX_EPOCH)).unwrap();
        log.stop();
        assert!(!log.is_recording());
        assert_eq!(log.written(), 1);

        // Starting again appends: a stop is a pause, not the end of the run.
        log.start(&path).unwrap();
        log.push(LogRow::new(LogSource::Board, UNIX_EPOCH)).unwrap();
        // Counted per recording, so this is the second run's row, not both.
        assert_eq!(log.written(), 1);

        let text = log.export_text().unwrap();
        let lines: Vec<&str> = text.lines().collect();
        // One header, however many times recording was started.
        assert_eq!(lines.len(), 3, "{text}");
        assert_eq!(format!("{}\n", lines[0]), CSV_HEADER);
        assert!(lines[1].contains("phone"));
        assert!(lines[2].contains("board"));
        // Both rows are still on the graph, which the stop did not touch.
        assert_eq!(log.rows().len(), 2);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_export_with_no_file_falls_back_to_the_rows() {
        let mut log = Logger::default();
        // Nothing recorded and nothing written: an export has nothing to say.
        assert!(log.export_text().is_err());
        // Rows arrive even while stopped, and are what an export then carries.
        log.push(LogRow::new(LogSource::Node(2), UNIX_EPOCH))
            .unwrap();
        let text = log.export_text().unwrap();
        assert!(text.starts_with(CSV_HEADER));
        assert!(text.contains(",node,2,"));
    }

    /// The graph keeps a bounded window; the file keeps everything. What
    /// scrolled off has to be counted, because the page says so - a run that
    /// silently lost its first hour would read as a run that never had one.
    #[test]
    fn the_graph_window_drops_the_oldest_and_says_how_many() {
        let mut log = Logger::default();
        for i in 0..MAX_ROWS as u64 {
            log.push(LogRow::new(LogSource::Phone, at(i))).unwrap();
        }
        assert_eq!(log.rows().len(), MAX_ROWS);
        assert_eq!(log.dropped(), 0);
        assert_eq!(log.rows()[0].time, at(0));

        // One past the window: the oldest row goes, not the newest.
        log.push(LogRow::new(LogSource::Board, at(MAX_ROWS as u64)))
            .unwrap();
        assert_eq!(log.rows().len(), MAX_ROWS);
        assert_eq!(log.dropped(), 1);
        assert_eq!(log.rows()[0].time, at(1));
        assert_eq!(log.rows().last().unwrap().source, LogSource::Board);

        // Emptying the graph resets the count with it: nothing has scrolled
        // off a window that was cleared on purpose.
        log.clear_rows();
        assert!(log.rows().is_empty());
        assert_eq!(log.dropped(), 0);
    }

    #[test]
    fn stats_read_the_columns_they_name() {
        let mut row = LogRow::new(LogSource::Node(1), UNIX_EPOCH);
        row.dist_user_m = Some(1200.0);
        row.rssi_dbm = Some(-101);
        assert_eq!(LogStat::Distance.value(&row), Some(1200.0));
        assert_eq!(LogStat::Rssi.value(&row), Some(-101.0));
        // Nothing set the reference distance, so the stat has no value here
        // rather than a zero that would plot as "at the reference point".
        assert_eq!(LogStat::DistanceRef.value(&row), None);
    }
}
