//! Climate: the Köppen-Geiger map Tellus bundles (Beck et al., about 1 km, 43200 x 21600, one
//! byte a pixel, LZW tiles of 256), downloaded once from Tellus's repository at a fixed commit and
//! kept in the cache; tiles are decoded as they're read.
use crate::{config, fetch};
use std::sync::{Arc, OnceLock};

const URL: &str = "https://raw.githubusercontent.com/Yucareux/Tellus/c38a4d2f4d732056cdde79578920c8c66e4e189b/src/main/resources/tellus/koppen/koppen_geiger_0p00833333.tif";
const WIDTH: i64 = 43200;
const HEIGHT: i64 = 21600;
const PER_DEGREE: f64 = 120.0;
const TILE: i64 = 256;

pub const CODES: [&str; 31] = [
    "", "Af", "Am", "Aw", "BWh", "BWk", "BSh", "BSk", "Csa", "Csb", "Csc", "Cwa", "Cwb", "Cwc", "Cfa",
    "Cfb", "Cfc", "Dsa", "Dsb", "Dsc", "Dsd", "Dwa", "Dwb", "Dwc", "Dwd", "Dfa", "Dfb", "Dfc", "Dfd",
    "ET", "EF",
];

struct Map {
    bytes: Vec<u8>,
    offsets: Vec<u64>,
    counts: Vec<u64>,
}

/// The map, or None when it couldn't be had (then every place reads as temperate, Cfb).
fn map() -> Option<&'static Map> {
    static MAP: OnceLock<Option<Map>> = OnceLock::new();
    MAP.get_or_init(|| {
        let path = config::get().cache.join("koppen").join("koppen_geiger_0p00833333.tif");
        let bytes = match fetch::read(&path) {
            Some(b) => b,
            None => match fetch::get(URL, None) {
                Ok(Some(b)) => {
                    fetch::write(&path, &b);
                    b
                }
                other => {
                    eprintln!("[tellus] the climate map couldn't be downloaded: {:?}", other.err());
                    return None;
                }
            },
        };
        let parsed = parse(&bytes);
        if parsed.is_none() {
            eprintln!("[tellus] the climate map is unreadable; delete {} to fetch it again", path.display());
        }
        parsed.map(|(offsets, counts)| Map { bytes, offsets, counts })
    })
    .as_ref()
}

fn parse(b: &[u8]) -> Option<(Vec<u64>, Vec<u64>)> {
    if b.get(..4)? != b"II*\0" {
        return None;
    }
    let u16_at = |at: usize| Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?) as u64);
    let u32_at = |at: usize| Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?) as u64);
    let ifd = u32_at(4)? as usize;
    let entries = u16_at(ifd)? as usize;
    let (mut offsets, mut counts) = (Vec::new(), Vec::new());
    for e in 0..entries {
        let at = ifd + 2 + e * 12;
        let (tag, ty, count) = (u16_at(at)?, u16_at(at + 2)?, u32_at(at + 4)?);
        if tag != 324 && tag != 325 {
            continue;
        }
        let start = u32_at(at + 8)? as usize;
        let list: Option<Vec<u64>> = (0..count as usize)
            .map(|i| if ty == 3 { u16_at(start + i * 2) } else { u32_at(start + i * 4) })
            .collect();
        if tag == 324 { offsets = list? } else { counts = list? }
    }
    let tiles = ((WIDTH + TILE - 1) / TILE * ((HEIGHT + TILE - 1) / TILE)) as usize;
    (offsets.len() == tiles && counts.len() == tiles).then_some((offsets, counts))
}

static TILES: fetch::Memo<Arc<Vec<u8>>> = fetch::Memo::new(128);

fn pixel(px: i64, py: i64) -> u8 {
    let Some(m) = map() else { return 15 };
    let (px, py) = (px.rem_euclid(WIDTH), py.clamp(0, HEIGHT - 1));
    let across = (WIDTH + TILE - 1) / TILE;
    let index = ((py / TILE) * across + px / TILE) as usize;
    let Some(tile) = TILES.get(index as u64, || {
        let (o, c) = (m.offsets[index] as usize, m.counts[index] as usize);
        let raw = m.bytes.get(o..o + c)?;
        let out = weezl::decode::Decoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8)
            .decode(raw)
            .ok()?;
        (out.len() >= (TILE * TILE) as usize).then(|| Arc::new(out))
    }) else {
        return 0;
    };
    tile[((py % TILE) * TILE + px % TILE) as usize]
}

/// The Köppen code at a place; over the sea, the nearest land's within about 10 km.
pub fn code(lat: f64, lon: f64) -> &'static str {
    let px = ((lon + 180.0) * PER_DEGREE).floor() as i64;
    let py = ((90.0 - lat) * PER_DEGREE).floor() as i64;
    for r in 0i64..=8 {
        let mut best = 0;
        for dy in -r..=r {
            for dx in -r..=r {
                if dx.abs().max(dy.abs()) != r {
                    continue;
                }
                let v = pixel(px + dx, py + dy);
                if v > 0 && (v as usize) < CODES.len() {
                    best = v;
                    break;
                }
            }
            if best > 0 {
                break;
            }
        }
        if best > 0 {
            return CODES[best as usize];
        }
    }
    "Cfb"
}
