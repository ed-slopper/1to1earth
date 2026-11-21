//! Elevation, as Tellus reads it (`TellusElevationSource`): Mapterhorn's land DEM and
//! OpenWaters' sea floor, both Terrarium-encoded WebP tiles (metres = R * 256 + G + B / 256 -
//! 32768) in Web Mercator, 512 pixels a side. AWS Terrain Tiles (PNG, 256 pixels) stand in for a
//! tile neither has.
use crate::{config, fetch};
use std::{io::Cursor, sync::Arc};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Dem {
    /// tiles.mapterhorn.com, land (and coarse sea), zoom up to 12 everywhere.
    Land,
    /// tiles.openwaters.io seascape, the sea floor.
    Sea,
    /// AWS Terrain Tiles, the fallback.
    Aws,
}

impl Dem {
    fn url(self, z: u8, x: u32, y: u32) -> String {
        match self {
            Dem::Land => format!("https://tiles.mapterhorn.com/{z}/{x}/{y}.webp"),
            Dem::Sea => format!("https://tiles.openwaters.io/seascape/{z}/{x}/{y}.webp"),
            Dem::Aws => format!("https://s3.amazonaws.com/elevation-tiles-prod/terrarium/{z}/{x}/{y}.png"),
        }
    }
    fn dir(self) -> &'static str {
        match self {
            Dem::Land => "elevation-mapterhorn",
            Dem::Sea => "elevation-openwaters",
            Dem::Aws => "elevation-aws",
        }
    }
    fn ext(self) -> &'static str {
        if self == Dem::Aws { "png" } else { "webp" }
    }
    pub fn size(self) -> usize {
        if self == Dem::Aws { 256 } else { 512 }
    }
    fn max_zoom(self) -> u8 {
        match self {
            Dem::Land => 12,
            Dem::Sea => 12,
            Dem::Aws => 15,
        }
    }
    fn id(self) -> u64 {
        match self {
            Dem::Land => 0,
            Dem::Sea => 1,
            Dem::Aws => 2,
        }
    }
}

pub struct Tile {
    size: usize,
    h: Vec<f32>,
}

/// A tile the source answered it doesn't have.
const MISSING: usize = 0;

static TILES: fetch::Memo<Arc<Tile>> = fetch::Memo::new(512);

fn tile(dem: Dem, z: u8, x: u32, y: u32) -> Option<Arc<Tile>> {
    let key = dem.id() << 60 | (z as u64) << 52 | (x as u64) << 26 | y as u64;
    TILES.get(key, || load(dem, z, x, y).map(Arc::new))
}

fn load(dem: Dem, z: u8, x: u32, y: u32) -> Option<Tile> {
    let cfg = config::get();
    let path = cfg
        .cache
        .join(dem.dir())
        .join(z.to_string())
        .join(x.to_string())
        .join(format!("{y}.{}", dem.ext()));
    let none = path.with_extension("none");
    if none.exists() {
        return Some(Tile { size: MISSING, h: Vec::new() });
    }
    if let Some(bytes) = fetch::read(&path) {
        if let Some(t) = decode(dem, &bytes) {
            return Some(t);
        }
        let _ = std::fs::remove_file(&path);
    }
    match fetch::get(&dem.url(z, x, y), None) {
        Ok(Some(bytes)) => match decode(dem, &bytes) {
            Some(t) => {
                fetch::write(&path, &bytes);
                Some(t)
            }
            None => {
                eprintln!("[tellus] {} tile {z}/{x}/{y} didn't decode", dem.dir());
                Some(Tile { size: MISSING, h: Vec::new() })
            }
        },
        Ok(None) => {
            fetch::write(&none, b"");
            Some(Tile { size: MISSING, h: Vec::new() })
        }
        Err(e) => {
            eprintln!("[tellus] {e}");
            None
        }
    }
}

fn decode(dem: Dem, bytes: &[u8]) -> Option<Tile> {
    let (w, h, rgb, ch) = if dem == Dem::Aws {
        let mut d = png::Decoder::new(Cursor::new(bytes));
        d.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut r = d.read_info().ok()?;
        let mut buf = vec![0u8; r.output_buffer_size()?];
        let info = r.next_frame(&mut buf).ok()?;
        let ch = match info.color_type {
            png::ColorType::Rgb => 3,
            png::ColorType::Rgba => 4,
            _ => return None,
        };
        (info.width as usize, info.height as usize, buf, ch)
    } else {
        let mut d = image_webp::WebPDecoder::new(Cursor::new(bytes)).ok()?;
        let (w, h) = d.dimensions();
        let ch = if d.has_alpha() { 4 } else { 3 };
        let mut buf = vec![0u8; d.output_buffer_size()?];
        d.read_image(&mut buf).ok()?;
        (w as usize, h as usize, buf, ch)
    };
    if w != h || w == 0 || rgb.len() < w * h * ch {
        return None;
    }
    let h = rgb[..w * w * ch]
        .chunks_exact(ch)
        .map(|p| p[0] as f32 * 256.0 + p[1] as f32 + p[2] as f32 / 256.0 - 32768.0)
        .collect();
    Some(Tile { size: w, h })
}

/// The zoom whose pixels are about as wide as a block (Tellus's `selectZoom`), at most `max`.
pub fn zoom_for(dem: Dem, metres_per_block: f64, lat: f64) -> u8 {
    let world = 40_075_016.686 * lat.to_radians().cos().max(0.01);
    let z = (world / (dem.size() as f64 * metres_per_block.max(0.1))).log2().ceil();
    (z.max(0.0) as u8).min(dem.max_zoom())
}

/// Metres at Web Mercator `(u, v)` (0..1, v from the north) from `dem` at zoom `z`, with a cubic
/// filter; a tile the source lacks or can't give now falls back to coarser zooms, then to AWS.
/// None when nothing at all can be read (offline with nothing cached).
pub fn sample(dem: Dem, u: f64, v: f64, z: u8) -> Option<f64> {
    let mut z = z;
    loop {
        if let Some(e) = sample_at(dem, u, v, z) {
            return Some(e);
        }
        if z < 3 {
            break;
        }
        z -= 2;
    }
    if dem != Dem::Aws {
        let az = (z + 1).min(Dem::Aws.max_zoom());
        return sample(Dem::Aws, u, v, az.max(10));
    }
    None
}

fn sample_at(dem: Dem, u: f64, v: f64, z: u8) -> Option<f64> {
    let size = dem.size() as i64;
    let total = size << z;
    let n = total as f64;
    let px = u.rem_euclid(1.0) * n - 0.5;
    let py = (v.clamp(0.0, 1.0) * n - 0.5).clamp(0.0, n - 1.0);
    let (x0, y0) = (px.floor() as i64, py.floor() as i64);
    let (fx, fy) = (px - x0 as f64, py - y0 as f64);
    let mut last: Option<(i64, i64, Arc<Tile>)> = None;
    let mut get = |gx: i64, gy: i64| -> Option<f64> {
        let gx = gx.rem_euclid(total);
        let gy = gy.clamp(0, total - 1);
        let (tx, ty) = (gx / size, gy / size);
        let t = match &last {
            Some((x, y, t)) if *x == tx && *y == ty => t.clone(),
            _ => {
                let t = tile(dem, z, tx as u32, ty as u32)?;
                last = Some((tx, ty, t.clone()));
                t
            }
        };
        if t.size == MISSING {
            return None;
        }
        // Tiles of another size than expected: scale the pixel into it.
        let (lx, ly) = ((gx % size) as usize * t.size / size as usize, (gy % size) as usize * t.size / size as usize);
        Some(t.h[ly * t.size + lx] as f64)
    };
    let mut rows = [0.0; 4];
    for (j, row) in rows.iter_mut().enumerate() {
        let mut p = [0.0; 4];
        for (i, val) in p.iter_mut().enumerate() {
            *val = get(x0 + i as i64 - 1, y0 + j as i64 - 1)?;
        }
        *row = cubic(p, fx);
    }
    Some(cubic(rows, fy))
}

fn cubic(p: [f64; 4], t: f64) -> f64 {
    let v = 0.5
        * (2.0 * p[1]
            + (-p[0] + p[2]) * t
            + (2.0 * p[0] - 5.0 * p[1] + 4.0 * p[2] - p[3]) * t * t
            + (-p[0] + 3.0 * p[1] - 3.0 * p[2] + p[3]) * t * t * t);
    let (lo, hi) = (p[1].min(p[2]), p[1].max(p[2]));
    let pad = (hi - lo) * 0.25 + 1.0;
    v.clamp(lo - pad, hi + pad)
}
