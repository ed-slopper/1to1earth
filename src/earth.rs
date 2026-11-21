//! Tellus's terrain, column by column: its projection (`EarthProjection`), its height transform
//! (`TerrainHeightTransform`), its biome rules (`EarthBiomeSource`, with its land cover and
//! Köppen table) and its surface palettes (`EarthChunkGenerator.selectBaseSurfacePalette`).
//! Left for later: OSM roads, buildings and rivers, RESOLVE ecoregions, sea temperatures from
//! NOAA, Tellus's own trees and its mountain zone rules.
use crate::{
    config,
    elevation::{self, Dem},
    koppen, worldcover as wc,
};
use std::{collections::HashMap, f64::consts::PI, sync::OnceLock};

/// The Earth level's build range: all of Horizon's, Y -2032 to 2031, so 1:1 fits what it can.
/// Its noise settings, `1to1earth:earth`, come with the world's 1to1earth datapack, which the mod writes
/// into the world (`datapack.rs`), so vanilla's carvers and features write over the whole range.
pub const MIN_Y: i32 = -2032;
pub const HEIGHT: i32 = 4064;
pub const MAX_Y: i32 = MIN_Y + HEIGHT - 1;

const METERS_PER_DEGREE: f64 = 111_319.490_666_666_67;
const MAX_LAT: f64 = 85.051_128_779_806_59;

fn radius() -> f64 {
    METERS_PER_DEGREE * 180.0 / PI
}

/// The top water block of the sea.
pub fn sea() -> i32 {
    config::get().height_offset - 1
}

/// Block `x, z` to latitude and longitude (Tellus's Mercator projection: lat 0, lon 0 at the
/// origin, north toward -Z).
pub fn to_globe(x: f64, z: f64) -> (f64, f64) {
    let s = config::get().scale();
    let lon = x * s / METERS_PER_DEGREE;
    let lat = (2.0 * (-z * s / radius()).exp().atan() - PI / 2.0).to_degrees();
    (lat, lon)
}

pub fn to_block(lat: f64, lon: f64) -> (f64, f64) {
    let s = config::get().scale();
    let lat = lat.clamp(-MAX_LAT, MAX_LAT).to_radians();
    (lon * METERS_PER_DEGREE / s, -radius() * (PI / 4.0 + lat / 2.0).tan().ln() / s)
}

/// Web Mercator `u, v` (0..1) of a latitude and longitude.
fn mercator(lat: f64, lon: f64) -> (f64, f64) {
    let lat = lat.clamp(-MAX_LAT, MAX_LAT).to_radians();
    (lon / 360.0 + 0.5, 0.5 - (PI / 4.0 + lat / 2.0).tan().ln() / (2.0 * PI))
}

/// The stack: Horizon's levels stop at Y 2031, so the Earth is layers of one level each, the
/// way Java's stacked-dimension mods raise the height (Simply Stacked Dimensions, Vertically
/// Stacked Dimensions). Layer `l` holds global Y `local + l * SPAN`. Each layer overlaps each
/// neighbour by half, and a player passing `CROSS` from its middle moves on to the next, so
/// there are always about a thousand blocks of the same world in view above and below.
pub const SPAN: i32 = 2032;
pub const CROSS: i32 = 1024; // a move lands 8 blocks past the next layer's middle line, inside its band
pub const LAYERS: std::ops::RangeInclusive<i32> = -6..=6;
/// The lowest and highest global ground the stack holds (about -14,200 to 14,200).
pub const GLOBAL_MIN: i32 = MIN_Y - 6 * SPAN + 8;
pub const GLOBAL_MAX: i32 = MAX_Y + 6 * SPAN - 24;

/// The level id of layer `l`.
pub fn layer_dim(l: i32) -> String {
    match l {
        0 => "1to1earth:earth".to_string(),
        l if l > 0 => format!("1to1earth:earth_up{l}"),
        l => format!("1to1earth:earth_down{}", -l),
    }
}

/// Which layer a level id is.
pub fn layer_of(dim: &str) -> Option<i32> {
    LAYERS.clone().find(|l| layer_dim(*l) == dim)
}

/// The layer whose own range holds global `y` best (nearest its middle).
pub fn layer_for(y: i32) -> i32 {
    ((y as f64) / SPAN as f64).round().clamp(*LAYERS.start() as f64, *LAYERS.end() as f64) as i32
}

/// Metres above (or below) the sea to blocks, as `TerrainHeightTransform` does.
fn blocks(m: f64, lat: f64) -> f64 {
    let c = config::get();
    let scale = if m >= 0.0 { c.terrestrial_height_scale } else { c.oceanic_height_scale };
    let corr = if c.automatic_height_scaling { 1.0 / lat.to_radians().cos().max(0.05) } else { 1.0 };
    m * scale * corr / c.scale()
}

#[derive(Clone, Copy, Debug)]
pub struct Column {
    /// The top solid block.
    pub ground: i32,
    /// The top water block; none below `ground + 1`.
    pub water: i32,
    pub ocean: bool,
    pub cover: u8,
    pub koppen: &'static str,
    pub metres: f64,
    pub lat: f64,
}

impl Column {
    pub fn wet(&self) -> bool {
        self.water > self.ground
    }
}

pub fn column(x: i32, z: i32) -> Column {
    let cfg = config::get();
    let (lat, lon) = to_globe(x as f64 + 0.5, z as f64 + 0.5);
    let (u, v) = mercator(lat, lon);
    let zoom = elevation::zoom_for(Dem::Land, cfg.scale(), lat);
    let land = elevation::sample(Dem::Land, u, v, zoom).unwrap_or(0.0);
    let overview = ((cfg.scale() / 10.0).log2().floor().max(0.0) as u32).min(wc::MAX_OVERVIEW);
    let cover = wc::class(lat, lon, overview);
    let koppen = koppen::code(lat, lon);
    let sea_top = sea();
    let is_sea = matches!(cover, wc::NO_DATA | wc::WATER) && land <= 0.0
        || cover == wc::WATER && land < 5.0
        || cover == wc::NO_DATA && land < -1.0;
    if is_sea {
        let depth = elevation::sample(Dem::Sea, u, v, elevation::zoom_for(Dem::Sea, cfg.scale(), lat))
            .filter(|d| *d < 0.0)
            .unwrap_or(land.min(-1.0));
        let floor = (sea_top + blocks(depth, lat).floor() as i32).min(sea_top - 1).max(GLOBAL_MIN);
        return Column { ground: floor, water: sea_top, ocean: true, cover, koppen, metres: depth, lat };
    }
    let top = if land >= 0.0 {
        sea_top + (blocks(land, lat).ceil() as i32).max(1)
    } else {
        sea_top + blocks(land, lat).floor() as i32
    };
    let top = top.clamp(GLOBAL_MIN, GLOBAL_MAX);
    if cover == wc::WATER {
        // A lake or river: its surface where the survey has it, its bed below. Water a block off
        // the sea's level is the sea's (harbours, lagoons, estuaries).
        if top <= sea_top + 1 {
            return Column { ground: (top - 2).min(sea_top - 1), water: sea_top, ocean: true, cover, koppen, metres: land, lat };
        }
        return Column { ground: top - 2, water: top, ocean: false, cover, koppen, metres: land, lat };
    }
    Column { ground: top, water: i32::MIN, ocean: false, cover, koppen, metres: land, lat }
}

/// Tellus's land cover and Köppen to biome table (`biome_classification_system.csv`): the biome
/// for each pair, and for each cover class its first row as the fallback.
fn table() -> &'static (HashMap<(u8, &'static str), &'static str>, HashMap<u8, &'static str>) {
    static TABLE: OnceLock<(HashMap<(u8, &'static str), &'static str>, HashMap<u8, &'static str>)> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut pairs = HashMap::new();
        let mut first = HashMap::new();
        for line in crate::biome_table::CSV.lines().skip(1) {
            let cols = split_csv(line);
            if cols.len() < 5 {
                continue;
            }
            let Ok(esa) = cols[0].parse::<u8>() else { continue };
            let koppen = koppen::CODES.iter().find(|c| **c == cols[2]).copied().unwrap_or("");
            let biome = BIOMES.iter().find(|b| b.strip_prefix("minecraft:") == Some(cols[4])).copied();
            let Some(biome) = biome else { continue };
            pairs.insert((esa, koppen), biome);
            first.entry(esa).or_insert(biome);
        }
        (pairs, first)
    })
}

fn split_csv(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut start, mut quoted) = (0, false);
    for (i, ch) in line.char_indices() {
        match ch {
            '"' => quoted = !quoted,
            ',' if !quoted => {
                out.push(line[start..i].trim_matches('"'));
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(line[start..].trim_matches('"'));
    out
}

/// Every biome the Earth places.
pub const BIOMES: &[&str] = &[
    "minecraft:plains",
    "minecraft:badlands",
    "minecraft:bamboo_jungle",
    "minecraft:birch_forest",
    "minecraft:dark_forest",
    "minecraft:desert",
    "minecraft:forest",
    "minecraft:frozen_peaks",
    "minecraft:jungle",
    "minecraft:mangrove_swamp",
    "minecraft:meadow",
    "minecraft:old_growth_birch_forest",
    "minecraft:old_growth_pine_taiga",
    "minecraft:old_growth_spruce_taiga",
    "minecraft:savanna",
    "minecraft:savanna_plateau",
    "minecraft:sparse_jungle",
    "minecraft:stony_peaks",
    "minecraft:swamp",
    "minecraft:taiga",
    "minecraft:windswept_forest",
    "minecraft:windswept_gravelly_hills",
    "minecraft:windswept_hills",
    "minecraft:windswept_savanna",
    "minecraft:wooded_badlands",
    "minecraft:snowy_slopes",
    "minecraft:jagged_peaks",
    "minecraft:river",
    "minecraft:frozen_river",
    "minecraft:frozen_ocean",
    "minecraft:deep_frozen_ocean",
    "minecraft:cold_ocean",
    "minecraft:deep_cold_ocean",
    "minecraft:lukewarm_ocean",
    "minecraft:deep_lukewarm_ocean",
    "minecraft:warm_ocean",
];

/// The biome of a column, by Tellus's order of rules. `slope` is the steepest drop to a
/// neighbour in blocks, for its snow biomes.
pub fn biome(c: &Column, slope: i32) -> &'static str {
    if c.cover == wc::MANGROVE {
        return "minecraft:mangrove_swamp";
    }
    if c.ocean {
        // Sea surface temperature from latitude (Tellus reads NOAA's OISST).
        let sst = 28.0 - 0.0074 * c.lat * c.lat;
        let deep = c.metres <= -200.0;
        return match () {
            _ if sst <= -1.5 => if deep { "minecraft:deep_frozen_ocean" } else { "minecraft:frozen_ocean" },
            _ if sst >= 23.0 => if deep { "minecraft:deep_lukewarm_ocean" } else { "minecraft:warm_ocean" },
            _ if sst >= 13.0 => if deep { "minecraft:deep_lukewarm_ocean" } else { "minecraft:lukewarm_ocean" },
            _ => if deep { "minecraft:deep_cold_ocean" } else { "minecraft:cold_ocean" },
        };
    }
    if c.wet() {
        return if c.koppen.starts_with('E') { "minecraft:frozen_river" } else { "minecraft:river" };
    }
    let antarctic = c.lat < -60.0 && c.cover == wc::NO_DATA;
    if c.cover == wc::SNOW || antarctic {
        return match slope {
            s if s >= 3 => "minecraft:jagged_peaks",
            s if s >= 1 => "minecraft:snowy_slopes",
            _ => "minecraft:frozen_peaks",
        };
    }
    let (pairs, first) = table();
    pairs
        .get(&(c.cover, c.koppen))
        .or_else(|| first.get(&c.cover))
        .copied()
        .unwrap_or("minecraft:plains")
}

/// Tellus's climate group of a biome: 1 tropical, 2 dry, 3 temperate, 4 boreal, 5 snowy, 0 water.
fn climate_group(b: &str) -> u8 {
    match b {
        "minecraft:mangrove_swamp" | "minecraft:jungle" | "minecraft:bamboo_jungle" | "minecraft:sparse_jungle"
        | "minecraft:savanna" | "minecraft:savanna_plateau" | "minecraft:windswept_savanna" => 1,
        "minecraft:desert" | "minecraft:badlands" | "minecraft:wooded_badlands" => 2,
        "minecraft:frozen_peaks" | "minecraft:snowy_slopes" => 5,
        "minecraft:taiga" | "minecraft:old_growth_pine_taiga" | "minecraft:old_growth_spruce_taiga"
        | "minecraft:jagged_peaks" | "minecraft:windswept_gravelly_hills" => 4,
        b if b.contains("ocean") || b.contains("river") => 0,
        _ => 3,
    }
}

/// A surface palette: the top block, the top under water, the filler and how deep it goes.
#[derive(Clone, Copy)]
pub struct Palette {
    pub top: &'static str,
    pub wet_top: &'static str,
    pub filler: &'static str,
    pub depth: i32,
}

const fn p(top: &'static str, wet_top: &'static str, filler: &'static str, depth: i32) -> Palette {
    Palette { top, wet_top, filler, depth }
}
const DEFAULT: Palette = p("minecraft:grass_block", "minecraft:dirt", "minecraft:dirt", 4);
const STEPPE: Palette = p("minecraft:coarse_dirt", "minecraft:coarse_dirt", "minecraft:dirt", 4);
const PODZOLIC: Palette = p("minecraft:podzol", "minecraft:podzol", "minecraft:dirt", 4);
const ROOTED: Palette = p("minecraft:rooted_dirt", "minecraft:rooted_dirt", "minecraft:dirt", 4);
const MOSSY: Palette = p("minecraft:moss_block", "minecraft:moss_block", "minecraft:dirt", 4);
const DESERT: Palette = p("minecraft:sand", "minecraft:sand", "minecraft:sandstone", 5);
const BADLANDS: Palette = p("minecraft:red_sand", "minecraft:red_sand", "minecraft:terracotta", 4);
const BEACH: Palette = p("minecraft:sand", "minecraft:sand", "minecraft:sand", 4);
const SNOWY: Palette = p("minecraft:snow_block", "minecraft:snow_block", "minecraft:dirt", 4);
const GLACIER: Palette = p("minecraft:snow_block", "minecraft:snow_block", "minecraft:packed_ice", 5);
const SWAMP: Palette = DEFAULT;
const WETLAND: Palette = p("minecraft:mud", "minecraft:mud", "minecraft:packed_mud", 4);
const MANGROVE: Palette = p("minecraft:mud", "minecraft:mud", "minecraft:dirt", 4);
const STONY: Palette = p("minecraft:stone", "minecraft:stone", "minecraft:stone", 12);
const GRAVELLY: Palette = p("minecraft:gravel", "minecraft:gravel", "minecraft:stone", 4);

/// Tellus's alpine line: 200 blocks over the sea.
const ALPINE: i32 = 200;

pub fn palette(c: &Column, biome: &str, slope: i32, beach: bool, x: i32, z: i32) -> Palette {
    if c.ocean || c.wet() {
        let roll = percent(x, z, 0x0CEA);
        let top = if roll < 10 { "minecraft:gravel" } else if roll < 15 { "minecraft:clay" } else { "minecraft:sand" };
        return p(top, top, top, 4);
    }
    // Steep ground is bare rock (snow where the snow lies), as Tellus's mountain rules give.
    if slope >= 4 {
        return if c.cover == wc::SNOW && slope < 7 { GLACIER } else { STONY };
    }
    if beach && c.ground <= sea() + 3 && c.cover != wc::TREE {
        return BEACH;
    }
    let roll = percent(x, z, 23);
    let above = c.ground - sea();
    let group = climate_group(biome);
    let cold = group == 4 || group == 5;
    if c.cover == wc::TREE {
        return cover_palette(c.cover, group, cold, roll, above);
    }
    match biome {
        "minecraft:badlands" | "minecraft:wooded_badlands" => BADLANDS,
        "minecraft:desert" => DESERT,
        "minecraft:mangrove_swamp" => MANGROVE,
        "minecraft:swamp" => SWAMP,
        "minecraft:stony_peaks" => STONY,
        "minecraft:windswept_gravelly_hills" => GRAVELLY,
        _ => cover_palette(c.cover, group, cold, roll, above),
    }
}

fn cover_palette(cover: u8, group: u8, cold: bool, roll: u32, above: i32) -> Palette {
    match cover {
        wc::TREE => {
            if cold {
                if roll < 65 { PODZOLIC } else { DEFAULT }
            } else if group == 2 {
                if roll < 70 { STEPPE } else { DEFAULT }
            } else if group == 1 {
                if roll < 35 { ROOTED } else { DEFAULT }
            } else if roll < 20 {
                ROOTED
            } else {
                DEFAULT
            }
        }
        wc::SHRUB => {
            if above >= ALPINE && roll < 70 {
                GRAVELLY
            } else if group == 2 {
                STEPPE
            } else if cold {
                PODZOLIC
            } else if roll < 40 {
                STEPPE
            } else {
                DEFAULT
            }
        }
        wc::GRASS => {
            if above >= ALPINE && roll < 35 {
                GRAVELLY
            } else if group == 2 {
                if roll < 65 { STEPPE } else { DEFAULT }
            } else if cold && roll < 40 {
                PODZOLIC
            } else {
                DEFAULT
            }
        }
        wc::CROP => {
            if group == 2 { STEPPE } else if roll < 65 { ROOTED } else { DEFAULT }
        }
        wc::BUILT => {
            if roll < 45 { GRAVELLY } else { STONY }
        }
        wc::BARE => {
            if above >= ALPINE {
                STONY
            } else if group == 2 {
                if roll < 55 { DESERT } else { BADLANDS }
            } else if cold && roll >= 70 {
                STONY
            } else {
                GRAVELLY
            }
        }
        wc::SNOW => SNOWY,
        wc::WETLAND => {
            if roll < 40 { SWAMP } else { WETLAND }
        }
        wc::MANGROVE => MANGROVE,
        wc::MOSS => {
            if above >= ALPINE && roll < 60 {
                STONY
            } else if roll < 35 {
                MOSSY
            } else {
                DEFAULT
            }
        }
        _ => {
            if cold { PODZOLIC } else { DEFAULT }
        }
    }
}

/// A clustered, roughly even 0..100 roll (Tellus's `SurfacePatchNoise`: three octaves of cells
/// 96, 23 and 7 blocks wide).
pub fn percent(x: i32, z: i32, salt: u32) -> u32 {
    let v = 0.5 * value(x, z, 96, salt) + 0.33 * value(x, z, 23, salt ^ 0x9E37) + 0.17 * value(x, z, 7, salt ^ 0x7F4A);
    // The sum leans to the middle: spread it back out.
    let t = ((v - 0.5) * 1.9 + 0.5).clamp(0.0, 0.9999);
    (t * 100.0) as u32
}

fn value(x: i32, z: i32, cell: i32, salt: u32) -> f64 {
    let (cx, cz) = (x.div_euclid(cell), z.div_euclid(cell));
    let (fx, fz) = (
        (x.rem_euclid(cell) as f64 + 0.5) / cell as f64,
        (z.rem_euclid(cell) as f64 + 0.5) / cell as f64,
    );
    let r = |a: i32, b: i32| (hash(a, salt as i32, b) & 0xFFFF) as f64 / 65535.0;
    let s = |t: f64| t * t * (3.0 - 2.0 * t);
    let (sx, sz) = (s(fx), s(fz));
    let top = r(cx, cz) * (1.0 - sx) + r(cx + 1, cz) * sx;
    let bottom = r(cx, cz + 1) * (1.0 - sx) + r(cx + 1, cz + 1) * sx;
    top * (1.0 - sz) + bottom * sz
}

/// Stone below the soil: deepslate under Y 0, mixed in over the first 8 blocks.
pub fn deepslate(y: i32, hash: u32) -> bool {
    y < 0 || (y < 8 && (hash % 8) as i32 >= y)
}

pub fn hash(x: i32, y: i32, z: i32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x9E37_79B1)
        ^ (y as u32).wrapping_mul(0x85EB_CA77)
        ^ (z as u32).wrapping_mul(0xC2B2_AE3D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h
}
