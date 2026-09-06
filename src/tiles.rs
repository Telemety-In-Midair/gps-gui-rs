//! Selectable map tile sources.
//!
//! Two providers: OpenStreetMap (the standard raster and OpenTopoMap) and
//! ArcGIS (Esri's streets and outdoor styles, and its satellite imagery). Every
//! source is a slippy-map XYZ raster, so they share the [`walkers::HttpTiles`]
//! widget stack and the on-disk HTTP cache (keyed by URL) with no special
//! handling - each source's tiles simply live under its own cache keys.
//!
//! The ArcGIS styles come as 512 px tiles, which walkers draws one zoom level
//! deeper than the tile's own level; [`MapSource::tile_level`] is that
//! conversion, for the code that names tiles by the map zoom they are seen at.

use walkers::sources::{Attribution, OpenStreetMap, TileSource};
use walkers::TileId;

/// The ArcGIS Location Platform key the app ships with. Meant to be sent from
/// a client, which is what an API key of this kind is for; `[map] arcgis_key`
/// replaces it with one of your own.
pub const DEFAULT_ARCGIS_KEY: &str = "AAPTaLEDPfRF5U1j7zW75V1JFOA..Xc-jWFA4nw_J9JGF520ijV9FtoW5UBoJJcBohtVfL8943P8CnNM_klLzhgVeIquD8NPCHiI2I4Apx67odVpe4aX4GrxyZeKH3wQwjo-FJov_0zCS-jYpygDzhv2XHV-obT5J3VzmfsnSm8WNuLyxSB-SOySpk8u4_-Y4LYJSPCoET_qamLAiW8C_G5487h07pcbNG5Y_6RxMb-HdUTaz6tJIQXTjW6WO98ZjI9TtkhLY2qUFEob4CA..AT1_gK1J2dfP";

/// The deepest map zoom a region download or the offline fallback reaches,
/// whatever a source could serve below it. The sources that go deeper still
/// draw there when zoomed in live.
pub const MAX_MAP_ZOOM: u8 = 19;

/// The tile size walkers lays the map out in; a source serving bigger tiles
/// is drawn a level deeper per doubling.
const BASE_TILE_PX: u32 = 256;

const ARCGIS_STATIC: &str =
    "https://static-map-tiles-api.arcgis.com/arcgis/rest/services/static-basemap-tiles-service/v1";
const ARCGIS_IMAGERY: &str =
    "https://ibasemaps-api.arcgis.com/arcgis/rest/services/World_Imagery/MapServer/tile";

/// Who serves the tiles: the `[map] tiles` setting.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TileProvider {
    /// OpenStreetMap and OpenTopoMap. No key, and a two-connection courtesy
    /// limit on downloads.
    #[default]
    Osm,
    /// Esri's ArcGIS basemaps under an API key: the one provider with
    /// satellite imagery.
    ArcGis,
}

impl TileProvider {
    /// Every provider, in the order the Settings page offers them.
    pub const ALL: [Self; 2] = [Self::Osm, Self::ArcGis];

    /// The TOML spelling, the one [`Self::parse`] reads back.
    pub fn as_str(self) -> &'static str {
        match self {
            TileProvider::Osm => "osm",
            TileProvider::ArcGis => "arcgis",
        }
    }

    /// The label on the Settings page's picker.
    pub fn label(self) -> &'static str {
        match self {
            TileProvider::Osm => "OpenStreetMap",
            TileProvider::ArcGis => "ArcGIS",
        }
    }

    /// Parse the TOML `map.tiles` string (case-insensitive).
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_lowercase().as_str() {
            "osm" | "openstreetmap" => Ok(TileProvider::Osm),
            "arcgis" | "esri" => Ok(TileProvider::ArcGis),
            other => Err(format!(
                "invalid map.tiles {other:?}, expected \"osm\" or \"arcgis\""
            )),
        }
    }

    /// The layers this provider has, in the order the map's layer button
    /// cycles them.
    pub fn layers(self) -> &'static [MapLayer] {
        match self {
            TileProvider::Osm => &[MapLayer::Standard, MapLayer::Topo],
            TileProvider::ArcGis => &[MapLayer::Standard, MapLayer::Topo, MapLayer::Satellite],
        }
    }

    /// Whether this provider has `layer` at all.
    pub fn has(self, layer: MapLayer) -> bool {
        self.layers().contains(&layer)
    }
}

/// What kind of map is on screen: the map bar's layer button cycles it
/// through the layers the provider has.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum MapLayer {
    /// The standard street map.
    #[default]
    Standard,
    /// Topographic: contours and terrain.
    Topo,
    /// Satellite imagery, which only ArcGIS has.
    Satellite,
}

impl MapLayer {
    /// The layer after this one on `provider`, wrapping round; the first one
    /// when the provider does not have this one.
    pub fn next(self, provider: TileProvider) -> MapLayer {
        let layers = provider.layers();
        match layers.iter().position(|&l| l == self) {
            Some(i) => layers[(i + 1) % layers.len()],
            None => layers[0],
        }
    }
}

/// Esri basemap styles, one per layer the ArcGIS provider has.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ArcGisStyle {
    /// The streets basemap, as 512 px static tiles.
    Streets,
    /// The outdoor basemap (terrain, contours, trails), as 512 px static
    /// tiles.
    Outdoor,
    /// World Imagery: satellite and aerial photography, 256 px JPEG, no
    /// labels.
    Imagery,
}

/// One tile server and style: what a tile URL is built from. Also the key the
/// map keeps its tile widgets under and what the offline code names a cache
/// entry by, so a change of key is a change of source.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum MapSource {
    /// Standard OpenStreetMap raster.
    OpenStreetMap,
    /// OpenTopoMap topographic raster.
    OpenTopoMap,
    /// An Esri basemap under an API key.
    ArcGis { style: ArcGisStyle, key: String },
}

/// The key the ArcGIS requests carry: the configured one, or the built-in
/// one when nothing (or only whitespace) is configured.
pub fn arcgis_key(configured: &str) -> String {
    let key = configured.trim();
    if key.is_empty() {
        DEFAULT_ARCGIS_KEY.to_string()
    } else {
        key.to_string()
    }
}

impl MapSource {
    /// The source for `layer` on `provider`, `key` being the configured
    /// `[map] arcgis_key`. A layer the provider does not have falls back to
    /// its standard map, so a stale layer choice still draws something.
    pub fn new(provider: TileProvider, layer: MapLayer, key: &str) -> Self {
        match provider {
            TileProvider::Osm => match layer {
                MapLayer::Topo => MapSource::OpenTopoMap,
                MapLayer::Standard | MapLayer::Satellite => MapSource::OpenStreetMap,
            },
            TileProvider::ArcGis => MapSource::ArcGis {
                style: match layer {
                    MapLayer::Standard => ArcGisStyle::Streets,
                    MapLayer::Topo => ArcGisStyle::Outdoor,
                    MapLayer::Satellite => ArcGisStyle::Imagery,
                },
                key: arcgis_key(key),
            },
        }
    }

    /// Whether `other` is served by the same provider under the same key, so
    /// the two are worth keeping warm together.
    pub fn same_provider(&self, other: &MapSource) -> bool {
        match (self, other) {
            (MapSource::ArcGis { key: a, .. }, MapSource::ArcGis { key: b, .. }) => a == b,
            (MapSource::ArcGis { .. }, _) | (_, MapSource::ArcGis { .. }) => false,
            _ => true,
        }
    }

    /// How many zoom levels deeper than its own level a tile of this source
    /// is drawn: 0 for 256 px tiles, 1 for 512 px.
    pub fn zoom_offset(&self) -> u8 {
        (self.tile_size() / BASE_TILE_PX).trailing_zeros() as u8
    }

    /// The tile level serving map zoom `map_zoom` on this source.
    pub fn tile_level(&self, map_zoom: u8) -> u8 {
        map_zoom.saturating_sub(self.zoom_offset())
    }

    /// The deepest map zoom a region download offers on this source: the
    /// deepest level it serves, seen at the map zoom it is drawn at, and never
    /// past [`MAX_MAP_ZOOM`].
    pub fn download_max_zoom(&self) -> u8 {
        self.max_zoom()
            .saturating_add(self.zoom_offset())
            .min(MAX_MAP_ZOOM)
    }

    /// Concurrent fetches a region download may run. The OSM tile usage
    /// policy allows two; the ArcGIS services state no such limit, so they
    /// get what walkers itself uses when browsing.
    pub fn parallel_downloads(&self) -> usize {
        match self {
            MapSource::OpenStreetMap | MapSource::OpenTopoMap => 2,
            MapSource::ArcGis { .. } => 6,
        }
    }

    /// Rough size of one cached tile in KB, for the download estimate. The
    /// 512 px PNG styles are big; the JPEG imagery is not.
    pub fn tile_kb(&self) -> u64 {
        match self {
            MapSource::OpenStreetMap => 15,
            MapSource::OpenTopoMap => 40,
            MapSource::ArcGis { style: ArcGisStyle::Imagery, .. } => 20,
            MapSource::ArcGis { .. } => 120,
        }
    }
}

impl TileSource for MapSource {
    fn tile_url(&self, tile: TileId) -> String {
        match self {
            MapSource::OpenStreetMap => OpenStreetMap.tile_url(tile),
            MapSource::OpenTopoMap => format!(
                "https://tile.opentopomap.org/{}/{}/{}.png",
                tile.zoom, tile.x, tile.y
            ),
            MapSource::ArcGis { style, key } => match style {
                ArcGisStyle::Streets | ArcGisStyle::Outdoor => {
                    let name = match style {
                        ArcGisStyle::Streets => "streets",
                        _ => "outdoor",
                    };
                    format!(
                        "{ARCGIS_STATIC}/arcgis/{name}/static/tile/{}/{}/{}?token={key}",
                        tile.zoom, tile.y, tile.x
                    )
                }
                ArcGisStyle::Imagery => format!(
                    "{ARCGIS_IMAGERY}/{}/{}/{}?token={key}",
                    tile.zoom, tile.y, tile.x
                ),
            },
        }
    }

    fn attribution(&self) -> Attribution {
        match self {
            MapSource::OpenStreetMap => OpenStreetMap.attribution(),
            MapSource::OpenTopoMap => Attribution {
                text: "OpenTopoMap (CC-BY-SA), OpenStreetMap contributors, SRTM",
                url: "https://opentopomap.org/about",
                logo_light: None,
                logo_dark: None,
            },
            MapSource::ArcGis { style: ArcGisStyle::Imagery, .. } => Attribution {
                text: "Powered by Esri. Esri, Maxar, Earthstar Geographics, and the GIS User Community",
                url: "https://www.esri.com/en-us/legal/terms/data-attributions",
                logo_light: None,
                logo_dark: None,
            },
            MapSource::ArcGis { .. } => Attribution {
                text: "Powered by Esri. Esri, TomTom, Garmin, FAO, NOAA, USGS, OpenStreetMap contributors",
                url: "https://www.esri.com/en-us/legal/terms/data-attributions",
                logo_light: None,
                logo_dark: None,
            },
        }
    }

    fn tile_size(&self) -> u32 {
        match self {
            MapSource::ArcGis { style: ArcGisStyle::Streets | ArcGisStyle::Outdoor, .. } => 512,
            _ => BASE_TILE_PX,
        }
    }

    /// The deepest tile level the server has: OSM's 19, OpenTopoMap's 17,
    /// the static basemap service's 22 (seen at map zoom 23), and 19 for the
    /// imagery, which the service advertises deeper but has data for only
    /// here and there.
    fn max_zoom(&self) -> u8 {
        match self {
            MapSource::OpenStreetMap => OpenStreetMap.max_zoom(),
            MapSource::OpenTopoMap => 17,
            MapSource::ArcGis { style: ArcGisStyle::Imagery, .. } => 19,
            MapSource::ArcGis { .. } => 22,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(zoom: u8, x: u32, y: u32) -> TileId {
        TileId { x, y, zoom }
    }

    #[test]
    fn provider_parse_round_trips_and_is_lenient() {
        for provider in TileProvider::ALL {
            assert_eq!(TileProvider::parse(provider.as_str()).unwrap(), provider);
        }
        assert_eq!(TileProvider::parse(" ArcGIS ").unwrap(), TileProvider::ArcGis);
        assert_eq!(TileProvider::parse("esri").unwrap(), TileProvider::ArcGis);
        assert_eq!(TileProvider::parse("OpenStreetMap").unwrap(), TileProvider::Osm);
        assert!(TileProvider::parse("google").is_err());
    }

    #[test]
    fn layers_cycle_within_the_provider() {
        assert_eq!(MapLayer::Standard.next(TileProvider::Osm), MapLayer::Topo);
        assert_eq!(MapLayer::Topo.next(TileProvider::Osm), MapLayer::Standard);
        assert_eq!(MapLayer::Topo.next(TileProvider::ArcGis), MapLayer::Satellite);
        assert_eq!(MapLayer::Satellite.next(TileProvider::ArcGis), MapLayer::Standard);
        // A layer the provider lacks starts the cycle over.
        assert_eq!(MapLayer::Satellite.next(TileProvider::Osm), MapLayer::Standard);
        assert!(!TileProvider::Osm.has(MapLayer::Satellite));
        assert!(TileProvider::ArcGis.has(MapLayer::Satellite));
    }

    #[test]
    fn osm_urls_are_the_slippy_scheme() {
        assert_eq!(
            MapSource::new(TileProvider::Osm, MapLayer::Standard, "").tile_url(tile(10, 511, 340)),
            "https://tile.openstreetmap.org/10/511/340.png"
        );
        assert_eq!(
            MapSource::new(TileProvider::Osm, MapLayer::Topo, "").tile_url(tile(10, 511, 340)),
            "https://tile.opentopomap.org/10/511/340.png"
        );
        // No satellite on OSM: the standard map stands in.
        assert_eq!(
            MapSource::new(TileProvider::Osm, MapLayer::Satellite, ""),
            MapSource::OpenStreetMap
        );
    }

    #[test]
    fn arcgis_urls_put_row_before_column_and_carry_the_key() {
        let streets = MapSource::new(TileProvider::ArcGis, MapLayer::Standard, "k1");
        assert_eq!(
            streets.tile_url(tile(10, 511, 340)),
            "https://static-map-tiles-api.arcgis.com/arcgis/rest/services/static-basemap-tiles-service/v1/arcgis/streets/static/tile/10/340/511?token=k1"
        );
        let outdoor = MapSource::new(TileProvider::ArcGis, MapLayer::Topo, "k1");
        assert!(outdoor
            .tile_url(tile(3, 1, 2))
            .ends_with("/arcgis/outdoor/static/tile/3/2/1?token=k1"));
        let imagery = MapSource::new(TileProvider::ArcGis, MapLayer::Satellite, "k1");
        assert_eq!(
            imagery.tile_url(tile(10, 511, 340)),
            "https://ibasemaps-api.arcgis.com/arcgis/rest/services/World_Imagery/MapServer/tile/10/340/511?token=k1"
        );
    }

    #[test]
    fn blank_key_means_the_built_in_one() {
        let source = MapSource::new(TileProvider::ArcGis, MapLayer::Standard, "  ");
        assert!(source
            .tile_url(tile(0, 0, 0))
            .ends_with(&format!("?token={DEFAULT_ARCGIS_KEY}")));
        let own = MapSource::new(TileProvider::ArcGis, MapLayer::Standard, " mine ");
        assert!(own.tile_url(tile(0, 0, 0)).ends_with("?token=mine"));
        assert_ne!(source, own);
    }

    #[test]
    fn big_tiles_sit_one_level_up() {
        let streets = MapSource::new(TileProvider::ArcGis, MapLayer::Standard, "");
        assert_eq!(streets.tile_size(), 512);
        assert_eq!(streets.zoom_offset(), 1);
        assert_eq!(streets.tile_level(15), 14);
        assert_eq!(streets.tile_level(0), 0);
        // The service's deepest level is 22, seen at 23, capped for downloads.
        assert_eq!(streets.download_max_zoom(), MAX_MAP_ZOOM);

        let imagery = MapSource::new(TileProvider::ArcGis, MapLayer::Satellite, "");
        assert_eq!(imagery.zoom_offset(), 0);
        assert_eq!(imagery.tile_level(15), 15);
        assert_eq!(imagery.download_max_zoom(), 19);

        assert_eq!(MapSource::OpenStreetMap.zoom_offset(), 0);
        assert_eq!(MapSource::OpenStreetMap.download_max_zoom(), 19);
        assert_eq!(MapSource::OpenTopoMap.download_max_zoom(), 17);
    }

    #[test]
    fn same_provider_is_by_service_and_key() {
        let a = MapSource::new(TileProvider::ArcGis, MapLayer::Standard, "k");
        let b = MapSource::new(TileProvider::ArcGis, MapLayer::Satellite, "k");
        let c = MapSource::new(TileProvider::ArcGis, MapLayer::Standard, "other");
        assert!(a.same_provider(&b));
        assert!(!a.same_provider(&c));
        assert!(MapSource::OpenStreetMap.same_provider(&MapSource::OpenTopoMap));
        assert!(!MapSource::OpenStreetMap.same_provider(&a));
    }

    #[test]
    fn download_limits_follow_the_policy() {
        assert_eq!(MapSource::OpenStreetMap.parallel_downloads(), 2);
        assert_eq!(MapSource::OpenTopoMap.parallel_downloads(), 2);
        assert!(MapSource::new(TileProvider::ArcGis, MapLayer::Satellite, "").parallel_downloads() > 2);
    }
}
