//! Offline region downloads: fetch every tile covering a lat/lon box across a
//! zoom range, through the same HTTP cache the live map reads.
//!
//! walkers' disk cache is an HTTP cache keyed by request URL
//! (http-cache-reqwest backed by cacache). Pre-downloading a region therefore
//! just means issuing the same GET requests through an identically configured
//! client pointed at the same cache directory: the map later serves those
//! areas from disk exactly as if they had been browsed online.
//!
//! Tiles are named by their own level here. A source with 512 px tiles draws
//! each level one map zoom deeper than its number, so the callers that think
//! in map zooms go through [`MapSource::tile_level`] first.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use http_cache_reqwest::{CACacheManager, Cache, CacheMode, HttpCache, HttpCacheOptions};
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use walkers::sources::TileSource;
use walkers::{Position, TileId};

use crate::tiles::{MapSource, MAX_MAP_ZOOM};

/// One user agent for both the map widget and this downloader, so every cache
/// entry is written and read with identical headers (and the tile server sees
/// a single identifiable client).
pub const USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

/// Latitude where the web-mercator projection cuts off.
const MAX_LAT: f64 = 85.05112877980659;

/// Slippy-map index of the tile containing (`lon`, `lat`) at `zoom`.
fn tile_xy(lon: f64, lat: f64, zoom: u8) -> (u32, u32) {
    let n = (1u64 << zoom) as f64;
    let x = ((lon + 180.0) / 360.0 * n).floor();
    let lat = lat.clamp(-MAX_LAT, MAX_LAT).to_radians();
    let y = ((1.0 - lat.tan().asinh() / std::f64::consts::PI) / 2.0 * n).floor();
    (x.clamp(0.0, n - 1.0) as u32, y.clamp(0.0, n - 1.0) as u32)
}

/// Inclusive (x0, y0, x1, y1) tile bounds covering the box spanned by `a` and
/// `b` at `zoom`. The corners may come in any order; tile y grows southward.
fn tile_bounds(a: Position, b: Position, zoom: u8) -> (u32, u32, u32, u32) {
    let (min_lon, max_lon) = if a.x() <= b.x() { (a.x(), b.x()) } else { (b.x(), a.x()) };
    let (min_lat, max_lat) = if a.y() <= b.y() { (a.y(), b.y()) } else { (b.y(), a.y()) };
    let (x0, y0) = tile_xy(min_lon, max_lat, zoom);
    let (x1, y1) = tile_xy(max_lon, min_lat, zoom);
    (x0, y0, x1, y1)
}

/// How many tiles the box needs over tile levels 0 through `max_level`. The
/// low levels are included because they cost next to nothing (a handful of
/// tiles each) and keep zooming out working offline.
pub fn tile_count(a: Position, b: Position, max_level: u8) -> u64 {
    (0..=max_level)
        .map(|zoom| {
            let (x0, y0, x1, y1) = tile_bounds(a, b, zoom);
            u64::from(x1 - x0 + 1) * u64::from(y1 - y0 + 1)
        })
        .sum()
}

/// Every tile for the box over tile levels 0 through `max_level`, coarsest
/// first, so a canceled download still leaves usable low-zoom coverage behind.
pub fn region_tiles(a: Position, b: Position, max_level: u8) -> Vec<TileId> {
    let mut tiles = Vec::new();
    for zoom in 0..=max_level {
        let (x0, y0, x1, y1) = tile_bounds(a, b, zoom);
        for x in x0..=x1 {
            for y in y0..=y1 {
                tiles.push(TileId { x, y, zoom });
            }
        }
    }
    tiles
}

/// The cacache key walkers and the downloader store a tile under: the HTTP
/// cache keys by `"<METHOD>:<url>"`, and every tile is a plain GET. The URL
/// (hence the key) differs per source, so each source has its own cache
/// entries - and a source under a different API key has its own again, the
/// key being part of the URL.
fn tile_cache_key(source: &MapSource, tile: TileId) -> String {
    format!("GET:{}", source.tile_url(tile))
}

/// Whether the `source` tile drawn under `pos` at map zoom `zoom` is already
/// in the on-disk cache.
fn tile_cached_at(cache_dir: &Path, source: &MapSource, pos: Position, zoom: u8) -> bool {
    let level = source.tile_level(zoom);
    let (x, y) = tile_xy(pos.x(), pos.y(), level);
    matches!(
        cacache::metadata_sync(cache_dir, tile_cache_key(source, TileId { x, y, zoom: level })),
        Ok(Some(_))
    )
}

/// The integer map zoom nearest to `current` whose `source` tile covering
/// `pos` is present in the cache. `None` when the current level already has
/// one (no change needed) or when no level does. Ties prefer the higher (more
/// detailed) zoom.
pub fn nearest_cached_zoom(
    cache_dir: &Path,
    source: &MapSource,
    pos: Position,
    current: u8,
) -> Option<u8> {
    if tile_cached_at(cache_dir, source, pos, current) {
        return None;
    }
    (0..=MAX_MAP_ZOOM)
        .filter(|&z| z != current && tile_cached_at(cache_dir, source, pos, z))
        .min_by_key(|&z| (z.abs_diff(current), MAX_MAP_ZOOM - z))
}

/// Quick reachability probe against the source's tile server. `true` when a
/// response of any status came back; `false` on a connection error or timeout.
/// Lets us tell "offline" from "not downloaded yet" before snapping the map
/// zoom.
async fn tile_server_reachable(client: &reqwest::Client, source: &MapSource) -> bool {
    let url = source.tile_url(TileId { x: 0, y: 0, zoom: 0 });
    client.head(url).send().await.is_ok()
}

/// On a background thread: if the tile server is NOT reachable, find the zoom
/// level nearest `current_zoom` whose tile covering `pos` is cached and send it
/// over `tx` for the UI to apply. Does nothing when online, when reachability
/// cannot be determined, or when no cached fallback exists - so it only ever
/// fires as an offline convenience for the center button.
pub fn spawn_offline_zoom(
    cache_dir: PathBuf,
    source: MapSource,
    pos: Position,
    current_zoom: u8,
    tx: Sender<f64>,
    ctx: egui::Context,
) {
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(runtime) => runtime,
            Err(e) => {
                log::error!("offline zoom: failed to start runtime: {e}");
                return;
            }
        };
        // Only act on a DEFINITE connection failure: a client-build error or a
        // successful probe both leave the zoom untouched.
        let offline = runtime.block_on(async {
            match reqwest::Client::builder()
                .user_agent(USER_AGENT)
                .timeout(Duration::from_secs(3))
                .build()
            {
                Ok(client) => !tile_server_reachable(&client, &source).await,
                Err(_) => false,
            }
        });
        if !offline {
            return;
        }
        if let Some(zoom) = nearest_cached_zoom(&cache_dir, &source, pos, current_zoom) {
            if tx.send(f64::from(zoom)).is_ok() {
                ctx.request_repaint();
            }
        }
    });
}

/// Shared state between the UI and the download worker.
pub struct DownloadProgress {
    /// Number of tiles the download started with.
    pub total: usize,
    /// Tiles attempted so far (successful or failed).
    pub done: AtomicUsize,
    /// Tiles that could not be fetched.
    pub failed: AtomicUsize,
    /// Set by the UI to stop the worker early.
    pub cancel: AtomicBool,
}

impl DownloadProgress {
    fn new(total: usize) -> Self {
        Self {
            total,
            done: AtomicUsize::new(0),
            failed: AtomicUsize::new(0),
            cancel: AtomicBool::new(false),
        }
    }

    pub fn finished(&self) -> bool {
        self.done.load(Ordering::Relaxed) >= self.total
    }
}

/// Download `tiles` of `source` into the HTTP cache at `cache_dir` on a
/// background thread. Progress (and the cancel flag) is shared through the
/// returned handle; the UI is woken through `ctx` as tiles complete.
pub fn spawn_download(
    cache_dir: PathBuf,
    source: MapSource,
    tiles: Vec<TileId>,
    ctx: egui::Context,
) -> Arc<DownloadProgress> {
    let progress = Arc::new(DownloadProgress::new(tiles.len()));
    let worker = progress.clone();

    std::thread::spawn(move || {
        match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(runtime) => {
                runtime.block_on(download_all(cache_dir, &source, tiles, &worker, &ctx))
            }
            Err(e) => {
                log::error!("offline download: failed to start runtime: {e}");
                worker.failed.store(worker.total, Ordering::Relaxed);
                worker.done.store(worker.total, Ordering::Relaxed);
            }
        }
        ctx.request_repaint();
    });

    progress
}

/// The same client construction as walkers' own tile fetches (HTTP cache
/// keyed by URL in `cache_dir`, standard cache-header handling).
fn cached_client(cache_dir: PathBuf) -> Result<ClientWithMiddleware, reqwest::Error> {
    let client = reqwest::Client::builder().user_agent(USER_AGENT).build()?;
    Ok(ClientBuilder::new(client)
        .with(Cache(HttpCache {
            mode: CacheMode::Default,
            manager: CACacheManager {
                path: cache_dir,
                remove_opts: Default::default(),
            },
            options: HttpCacheOptions::default(),
        }))
        .build())
}

async fn download_all(
    cache_dir: PathBuf,
    source: &MapSource,
    tiles: Vec<TileId>,
    progress: &DownloadProgress,
    ctx: &egui::Context,
) {
    let client = match cached_client(cache_dir) {
        Ok(client) => client,
        Err(e) => {
            log::error!("offline download: failed to build HTTP client: {e}");
            progress.failed.store(progress.total, Ordering::Relaxed);
            progress.done.store(progress.total, Ordering::Relaxed);
            return;
        }
    };

    futures::stream::iter(tiles.into_iter().map(|tile| {
        let client = &client;
        async move {
            if progress.cancel.load(Ordering::Relaxed) {
                return;
            }
            if !fetch_tile(client, &source.tile_url(tile)).await {
                progress.failed.fetch_add(1, Ordering::Relaxed);
            }
            progress.done.fetch_add(1, Ordering::Relaxed);
            ctx.request_repaint();
        }
    }))
    .buffer_unordered(source.parallel_downloads())
    .for_each(|()| async {})
    .await;
}

/// One tile GET through the caching client. Reading the body to the end is
/// what lets the cache middleware finish storing the response.
async fn fetch_tile(client: &ClientWithMiddleware, url: &str) -> bool {
    match client.get(url).send().await {
        Ok(response) => match response.error_for_status() {
            Ok(response) => response.bytes().await.is_ok(),
            Err(e) => {
                log::warn!("offline download: {}: {e}", redact_token(url));
                false
            }
        },
        Err(e) => {
            log::warn!("offline download: {}: {e}", redact_token(url));
            false
        }
    }
}

/// The URL with the value of its `token` query parameter blanked, for the
/// log: the built-in key is public, but a key typed into the settings need
/// not end up in a log file.
fn redact_token(url: &str) -> String {
    match url.find("token=") {
        Some(at) => {
            let end = url[at..].find('&').map_or(url.len(), |i| at + i);
            format!("{}token=...{}", &url[..at], &url[end..])
        }
        None => url.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use walkers::lat_lon;

    #[test]
    fn tile_xy_matches_known_tiles() {
        assert_eq!(tile_xy(0.0, 0.0, 0), (0, 0));
        assert_eq!(tile_xy(0.0, 0.0, 1), (1, 1));
        // Greenwich at zoom 10 lands on the well-known London tile.
        assert_eq!(tile_xy(-0.0015, 51.4779, 10), (511, 340));
    }

    #[test]
    fn poles_clamp_into_range() {
        assert_eq!(tile_xy(-180.0, 90.0, 2), (0, 0));
        assert_eq!(tile_xy(180.0, -90.0, 2), (3, 3));
    }

    #[test]
    fn corners_in_any_order() {
        let a = lat_lon(51.0, -0.2);
        let b = lat_lon(51.2, 0.1);
        assert_eq!(tile_bounds(a, b, 12), tile_bounds(b, a, 12));
    }

    #[test]
    fn count_matches_enumeration() {
        let a = lat_lon(51.4779, -0.0015);
        let b = lat_lon(51.5, 0.02);
        for zoom in [0u8, 5, 12, 14] {
            assert_eq!(region_tiles(a, b, zoom).len() as u64, tile_count(a, b, zoom));
        }
    }

    #[test]
    fn whole_world_at_zoom_zero_is_one_tile() {
        assert_eq!(tile_count(lat_lon(85.0, -179.9), lat_lon(-85.0, 179.9), 0), 1);
    }

    #[test]
    fn cache_key_is_the_get_url() {
        let tile = TileId { x: 511, y: 340, zoom: 10 };
        assert_eq!(
            tile_cache_key(&MapSource::OpenStreetMap, tile),
            "GET:https://tile.openstreetmap.org/10/511/340.png"
        );
    }

    #[test]
    fn token_is_blanked_in_logs() {
        assert_eq!(
            redact_token("https://x/tile/1/2/3?token=SECRET"),
            "https://x/tile/1/2/3?token=..."
        );
        assert_eq!(
            redact_token("https://x/t?token=SECRET&f=png"),
            "https://x/t?token=...&f=png"
        );
        assert_eq!(redact_token("https://x/t.png"), "https://x/t.png");
    }
}
