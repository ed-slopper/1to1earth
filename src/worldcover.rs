//! Land cover: ESA WorldCover 2021 v200, 10 m classes, one cloud-optimised GeoTIFF per 3 x 3
//! degree square on AWS Open Data. Only each file's header and the 1024-pixel blocks touched are
//! fetched, by HTTP range requests, and kept on disk (as Earth Platform does).
use crate::{config, fetch};
use std::{io::Read, sync::Arc};

pub const TREE: u8 = 10;
pub const SHRUB: u8 = 20;
pub const GRASS: u8 = 30;
pub const CROP: u8 = 40;
pub const BUILT: u8 = 50;
pub const BARE: u8 = 60;
pub const SNOW: u8 = 70;
pub const WATER: u8 = 80;
pub const WETLAND: u8 = 90;
pub const MANGROVE: u8 = 95;
pub const MOSS: u8 = 100;
pub const NO_DATA: u8 = 0;

const BASE: &str = "https://esa-worldcover.s3.eu-central-1.amazonaws.com/v200/2021/map/";
const PER_DEGREE: i64 = 12000;
const SQUARE: i32 = 3;
pub const MAX_OVERVIEW: u32 = 6;
const BLOCK: usize = 1024;

struct Level {
    width: u32,
    offsets: Vec<u64>,
    counts: Vec<u64>,
}

struct Header {
    levels: Vec<Level>,
    deflate: bool,
}

static HEADERS: fetch::Memo<Arc<Option<Header>>> = fetch::Memo::new(256);
static BLOCKS: fetch::Memo<Arc<Vec<u8>>> = fetch::Memo::new(96);

fn name(south: i32, west: i32) -> String {
    format!(
        "ESA_WorldCover_10m_2021_v200_{}{:02}{}{:03}_Map",
        if south < 0 { 'S' } else { 'N' },
        south.abs(),
        if west < 0 { 'W' } else { 'E' },
        west.abs()
    )
}

/// The class at a latitude and longitude, from overview `overview` (0 is the 10 m image, each
/// one after it half as fine). NO_DATA for open sea and where it can't be read.
pub fn class(lat: f64, lon: f64, overview: u32) -> u8 {
    let lon = (lon + 180.0).rem_euclid(360.0) - 180.0;
    let mut south = (lat.floor() as i32).div_euclid(SQUARE) * SQUARE;
    if south >= 90 {
        south = 90 - SQUARE;
    }
    let west = (lon.floor() as i32).div_euclid(SQUARE) * SQUARE;
    let full = SQUARE as i64 * PER_DEGREE;
    let fx = ((lon - west as f64) * PER_DEGREE as f64) as i64;
    let fy = (((south + SQUARE) as f64 - lat) * PER_DEGREE as f64).max(0.0) as i64;
    let last = (full >> overview) - 1;
    let px = ((fx.min(full - 1)) >> overview).min(last);
    let py = ((fy.min(full - 1)) >> overview).min(last);
    let square = ((south + 90) * 1000 + (west + 180)) as u64;
    let Some(header) = header(square, south, west) else {
        return NO_DATA;
    };
    let Some(h) = header.as_ref() else {
        return NO_DATA;
    };
    let Some(level) = h.levels.get(overview as usize) else {
        return NO_DATA;
    };
    let across = (level.width as usize).div_ceil(BLOCK);
    let (row, col) = ((py as usize) / BLOCK, (px as usize) / BLOCK);
    let index = row * across + col;
    let key = square << 32 | (overview as u64) << 28 | index as u64;
    let Some(block) = BLOCKS.get(key, || read_block(h, level, south, west, overview, index).map(Arc::new))
    else {
        return NO_DATA;
    };
    if block.is_empty() {
        return NO_DATA;
    }
    block[(py as usize % BLOCK) * BLOCK + px as usize % BLOCK]
}

fn header(square: u64, south: i32, west: i32) -> Option<Arc<Option<Header>>> {
    HEADERS.get(square, || {
        let cfg = config::get();
        let dir = cfg.cache.join("worldcover-2021-v200-range");
        let n = name(south, west);
        if dir.join(format!("{n}.none")).exists() {
            return Some(Arc::new(None));
        }
        let cached = dir.join(format!("{n}.header"));
        if let Some(bytes) = fetch::read(&cached) {
            if let Ok(h) = parse(&bytes) {
                return Some(Arc::new(Some(h)));
            }
        }
        let mut size = 65536u64;
        loop {
            match fetch::get(&format!("{BASE}{n}.tif"), Some((0, size - 1))) {
                Ok(None) => {
                    fetch::write(&dir.join(format!("{n}.none")), b"");
                    return Some(Arc::new(None));
                }
                Ok(Some(bytes)) => match parse(&bytes) {
                    Ok(h) => {
                        fetch::write(&cached, &bytes);
                        return Some(Arc::new(Some(h)));
                    }
                    Err(Short) if size < (4 << 20) && bytes.len() as u64 >= size => size *= 4,
                    Err(_) => {
                        eprintln!("[tellus] land cover header {n} is unreadable");
                        return Some(Arc::new(None));
                    }
                },
                Err(e) => {
                    eprintln!("[tellus] land cover header {n}: {e}");
                    return None;
                }
            }
        }
    })
}

fn read_block(h: &Header, level: &Level, south: i32, west: i32, overview: u32, index: usize) -> Option<Vec<u8>> {
    let cfg = config::get();
    let n = name(south, west);
    let path = cfg
        .cache
        .join("worldcover-2021-v200-range")
        .join(&n)
        .join(overview.to_string())
        .join(format!("{index}.z"));
    let (&offset, &count) = (level.offsets.get(index)?, level.counts.get(index)?);
    if offset == 0 || count == 0 {
        return Some(vec![NO_DATA; BLOCK * BLOCK]);
    }
    if let Some(raw) = fetch::read(&path) {
        if let Some(b) = decode(&raw, h.deflate) {
            return Some(b);
        }
    }
    if count > (8 << 20) {
        return None;
    }
    match fetch::get(&format!("{BASE}{n}.tif"), Some((offset, offset + count - 1))) {
        Ok(Some(raw)) if raw.len() as u64 == count => {
            let b = decode(&raw, h.deflate)?;
            fetch::write(&path, &raw);
            Some(b)
        }
        Ok(_) => Some(Vec::new()),
        Err(e) => {
            eprintln!("[tellus] land cover block {n}/{overview}/{index}: {e}");
            None
        }
    }
}

fn decode(raw: &[u8], deflate: bool) -> Option<Vec<u8>> {
    if !deflate {
        return (raw.len() == BLOCK * BLOCK).then(|| raw.to_vec());
    }
    let mut out = Vec::with_capacity(BLOCK * BLOCK);
    flate2::read::ZlibDecoder::new(raw)
        .take((BLOCK * BLOCK) as u64)
        .read_to_end(&mut out)
        .ok()?;
    (out.len() == BLOCK * BLOCK).then_some(out)
}

struct Short;

/// The image file directories of the tiled 8-bit TIFF: the image and its overviews.
fn parse(b: &[u8]) -> Result<Header, Short> {
    if b.len() < 8 {
        return Err(Short);
    }
    let le = b[0] == b'I';
    let u16_at = |at: usize| -> Result<u64, Short> {
        let s = b.get(at..at + 2).ok_or(Short)?;
        Ok(if le { u16::from_le_bytes([s[0], s[1]]) } else { u16::from_be_bytes([s[0], s[1]]) } as u64)
    };
    let u32_at = |at: usize| -> Result<u64, Short> {
        let s = b.get(at..at + 4).ok_or(Short)?;
        let a = [s[0], s[1], s[2], s[3]];
        Ok(if le { u32::from_le_bytes(a) } else { u32::from_be_bytes(a) } as u64)
    };
    let value = |at: usize, ty: u64| if ty == 3 { u16_at(at + 8) } else { u32_at(at + 8) };
    let array = |at: usize, ty: u64, count: u64| -> Result<Vec<u64>, Short> {
        let size = if ty == 3 { 2 } else { 4 };
        let start = if count * size <= 4 { at + 8 } else { u32_at(at + 8)? as usize };
        (0..count as usize)
            .map(|i| if ty == 3 { u16_at(start + i * 2) } else { u32_at(start + i * 4) })
            .collect()
    };
    let mut next = u32_at(4)? as usize;
    let mut levels = Vec::new();
    let mut deflate = true;
    while next != 0 && levels.len() <= MAX_OVERVIEW as usize {
        let entries = u16_at(next)? as usize;
        let (mut width, mut compression) = (0, 1);
        let (mut offsets, mut counts) = (Vec::new(), Vec::new());
        for e in 0..entries {
            let at = next + 2 + e * 12;
            let tag = u16_at(at)?;
            let ty = u16_at(at + 2)?;
            let count = u32_at(at + 4)?;
            match tag {
                256 => width = value(at, ty)? as u32,
                259 => compression = value(at, ty)?,
                324 => offsets = array(at, ty, count)?,
                325 => counts = array(at, ty, count)?,
                _ => {}
            }
        }
        deflate = compression != 1;
        levels.push(Level { width, offsets, counts });
        next = u32_at(next + 2 + entries * 12)? as usize;
    }
    if levels.is_empty() {
        return Err(Short);
    }
    Ok(Header { levels, deflate })
}
