//! Recorded GPS points: which source produced them and when. Powers both the
//! map tracks and the searchable points list page.

use std::time::SystemTime;

use walkers::Position;

/// Where a recorded point came from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PointSource {
    /// The phone's own GNSS (a simulated loop on desktop).
    Phone,
    /// The connected board's own GPS (Wio-S3 / esp32c3 beacon).
    Esp,
    /// A remote node heard over LoRa and relayed by the connected board,
    /// keyed by its LoRa address (1-255). Each address is its own track.
    Remote(u8),
}

impl PointSource {
    /// The generic name of the source, before any name is applied to it.
    ///
    /// The phone is the BLE central of the link the board sits on the far end
    /// of, which is the name it goes by everywhere else in the system. The
    /// connected board's own name and a remote node's nickname live where the
    /// config and the link are in reach, so the pages resolve those through
    /// the app and fall back to these.
    pub fn label(self) -> String {
        match self {
            PointSource::Phone => "Central".to_string(),
            PointSource::Esp => "Beacon".to_string(),
            PointSource::Remote(addr) => format!("Node {addr}"),
        }
    }
}

/// One recorded track point.
#[derive(Clone, Copy)]
pub struct TrackPoint {
    pub pos: Position,
    pub source: PointSource,
    pub time: SystemTime,
}

impl TrackPoint {
    /// "lat lon" with 5 decimals (about meter precision); what the points
    /// list shows and what the search matches against.
    pub fn coord_text(&self) -> String {
        format!("{:.5} {:.5}", self.pos.y(), self.pos.x())
    }

    /// Substring search across the source's label and the coordinates.
    /// `label` is the name the list prints for this point's source, which the
    /// page resolves (the board's name, a node's nickname) rather than the
    /// generic one. `query` must already be lowercase; the label is folded to
    /// match, so searching is case-insensitive however it is capitalized.
    pub fn matches(&self, label: &str, query: &str) -> bool {
        label.to_lowercase().contains(query) || self.coord_text().contains(query)
    }
}

/// Compact "how long ago" text for the points list.
pub fn age_text(now: SystemTime, then: SystemTime) -> String {
    let secs = now.duration_since(then).unwrap_or_default().as_secs();
    if secs < 60 {
        format!("{secs} s")
    } else if secs < 3600 {
        format!("{} min", secs / 60)
    } else if secs < 86400 {
        format!("{} h", secs / 3600)
    } else {
        format!("{} d", secs / 86400)
    }
}

/// Parse "lat, lon" or "lat lon" into decimal degrees. `None` unless it is
/// exactly two finite numbers within the valid latitude/longitude range.
///
/// The one place a coordinate is typed rather than measured: the desktop
/// manual position bar and the logging reference point both take it, and both
/// have to refuse the same things - half a coordinate, a third number, or a
/// value off the globe.
pub fn parse_lat_lon(s: &str) -> Option<(f64, f64)> {
    let mut parts = s
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|p| !p.is_empty());
    let lat: f64 = parts.next()?.parse().ok()?;
    let lon: f64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None; // trailing junk
    }
    // A NaN fails every range test, so this rejects the non-finite spellings
    // `f64` parses ("nan", "inf") as well as the out-of-range numbers.
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return None;
    }
    Some((lat, lon))
}

#[cfg(test)]

mod tests {
    use super::*;
    use std::time::Duration;
    use walkers::lat_lon;

    fn point(source: PointSource) -> TrackPoint {
        TrackPoint {
            pos: lat_lon(51.4779, -0.0015),
            source,
            time: SystemTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn search_matches_source_and_coordinates() {
        let p = point(PointSource::Esp);
        let label = p.source.label();
        assert!(p.matches(&label, ""));
        assert!(p.matches(&label, "beacon"));
        assert!(p.matches(&label, "51.477"));
        assert!(p.matches(&label, "-0.0015"));
        assert!(!p.matches(&label, "central"));
        assert!(!p.matches(&label, "52."));
    }

    #[test]
    fn source_search_ignores_label_case() {
        // The query arrives lowercased; a capitalized label still matches.
        let p = point(PointSource::Phone);
        assert!(p.matches(&p.source.label(), "central"));
    }

    /// The label searched is the one the page prints, so a board named on
    /// the board is found by that name and not by the generic one.
    #[test]
    fn search_uses_the_label_the_page_resolved() {
        let p = point(PointSource::Esp);
        assert!(p.matches("sky-1", "sky"));
        assert!(!p.matches("sky-1", "beacon"));
    }

    #[test]
    fn remote_nodes_are_named_and_searchable_by_address() {
        let p = point(PointSource::Remote(7));
        let label = p.source.label();
        assert_eq!(label, "Node 7");
        assert!(p.matches(&label, "node 7"));
        assert!(p.matches(&label, "node"));
        assert!(!p.matches(&label, "central"));
    }

    #[test]
    fn ages_scale_units() {
        let base = SystemTime::UNIX_EPOCH;
        let at = |secs| base + Duration::from_secs(secs);
        assert_eq!(age_text(at(5), base), "5 s");
        assert_eq!(age_text(at(120), base), "2 min");
        assert_eq!(age_text(at(7200), base), "2 h");
        assert_eq!(age_text(at(200_000), base), "2 d");
        // Clock skew must not panic.
        assert_eq!(age_text(base, at(5)), "0 s");
    }

    /// Typed coordinates, on the desktop manual position bar and the logging
    /// reference point.
    #[test]
    fn coordinates_parse_with_either_separator() {
        assert_eq!(parse_lat_lon("51.4779, -0.0015"), Some((51.4779, -0.0015)));
        assert_eq!(parse_lat_lon("51.4779 -0.0015"), Some((51.4779, -0.0015)));
        assert_eq!(
            parse_lat_lon("  51.4779 ,  -0.0015  "),
            Some((51.4779, -0.0015))
        );
        assert_eq!(parse_lat_lon("0 0"), Some((0.0, 0.0)));
        // The poles and the antimeridian are places, so the ends are inclusive.
        assert_eq!(parse_lat_lon("90, 180"), Some((90.0, 180.0)));
        assert_eq!(parse_lat_lon("-90, -180"), Some((-90.0, -180.0)));
    }

    /// Half a coordinate, or one with something after it, is a coordinate
    /// still being typed rather than one to move the marker to.
    #[test]
    fn incomplete_and_trailing_input_is_refused() {
        assert_eq!(parse_lat_lon(""), None);
        assert_eq!(parse_lat_lon("   "), None);
        assert_eq!(parse_lat_lon("51.4779"), None);
        assert_eq!(parse_lat_lon("51.4779,"), None);
        assert_eq!(parse_lat_lon("51.4779, -0.0015, 12"), None);
        assert_eq!(parse_lat_lon("51.4779, -0.0015 m"), None);
        assert_eq!(parse_lat_lon("north, west"), None);
    }

    /// Out of range is a typo, not a place - and the non-finite spellings
    /// `f64` accepts have to be caught here, since a NaN would move the marker
    /// somewhere no comparison could get it back from.
    #[test]
    fn out_of_range_and_non_finite_coordinates_are_refused() {
        assert_eq!(parse_lat_lon("90.001, 0"), None);
        assert_eq!(parse_lat_lon("-90.001, 0"), None);
        assert_eq!(parse_lat_lon("0, 180.001"), None);
        assert_eq!(parse_lat_lon("0, -180.001"), None);
        assert_eq!(parse_lat_lon("nan, 0"), None);
        assert_eq!(parse_lat_lon("0, nan"), None);
        assert_eq!(parse_lat_lon("inf, 0"), None);
        assert_eq!(parse_lat_lon("0, -inf"), None);
    }
}
