//! PMTiles v3 archives over HTTP range requests (Tellus's `PmTilesRangeReader`): the header, the
//! gzip'd root and leaf directories, and one tile's bytes, each fetched once and kept on disk.
//! Overture's `buildings.pmtiles` and `transportation.pmtiles` are 100+ GB: only what a chunk
//! needs is read.
use crate::{config, fetch};
use std::{
    io::Read,
    sync::{Arc, OnceLock},
};

const BUCKET: &str = "https://overturemaps-extras-us-west-2.s3.us-west-2.amazonaws.com";

/// The Overture release a world reads: the newest when first asked, then kept in the cache so a
/// world's later chunks read the same data as its first.
pub fn release() -> &'static str {
    static R: OnceLock<String> = OnceLock::new();
    R.get_or_init(|| {
        let file = config::get().cache.join("overture-release.txt");
        if let Some(b) = fetch::read(&file) {
            let s = String::from_utf8_lossy(&b).trim().to_string();
            if !s.is_empty() {
                return s;
            }
        }
        let listing = fetch::get(&format!("{BUCKET}/?list-type=2&prefix=tiles/&delimiter=/"), None)
            .ok()
            .flatten()
            .map(|b| String::from_utf8_lossy(&b).to_string())
            .unwrap_or_default();
        let mut releases: Vec<&str> = listing
            .split("<Prefix>tiles/")
            .skip(1)
            .filter_map(|s| s.split('/').next())
            .filter(|s| s.len() >= 12 && s.as_bytes()[4] == b'-')
            .collect();
        releases.sort();
        let r = releases.last().copied().unwrap_or("2026-11-18.0").to_string();
        fetch::write(&file, r.as_bytes());
        eprintln!("[1to1earth] Overture release {r}");
        r
    })
}

pub struct Archive {
    url: String,
    dir: std::path::PathBuf,
    root: Vec<Entry>,
    leaf_offset: u64,
    data_offset: u64,
    max_zoom: u8,
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    id: u64,
    offset: u64,
    length: u32,
    run: u32,
}

/// `theme` (`buildings`, `transportation`), opened once; None while it can't be read.
pub fn archive(theme: &'static str) -> Option<Arc<Archive>> {
    static ARCHIVES: fetch::Memo<Arc<Archive>> = fetch::Memo::new(8);
    let key = theme.bytes().fold(0u64, |h, b| h.wrapping_mul(31).wrapping_add(b as u64));
    ARCHIVES.get(key, || open(theme).map(Arc::new))
}

fn open(theme: &str) -> Option<Archive> {
    let rel = release();
    let url = format!("{BUCKET}/tiles/{rel}/{theme}.pmtiles");
    let dir = config::get().cache.join("overture").join(rel).join(theme);
    let head_path = dir.join("head.bin");
    let head = match fetch::read(&head_path) {
        Some(b) if b.len() >= 127 => b,
        _ => {
            let b = fetch::get(&url, Some((0, 16383))).ok()??;
            if b.len() < 127 || &b[..7] != b"PMTiles" || b[7] != 3 {
                eprintln!("[1to1earth] {theme}.pmtiles isn't a PMTiles v3 archive");
                return None;
            }
            fetch::write(&head_path, &b);
            b
        }
    };
    let u = |i: usize| u64::from_le_bytes(head[8 + i * 8..16 + i * 8].try_into().unwrap());
    let (root_off, root_len, leaf_off, data_off) = (u(0), u(1), u(4), u(6));
    let internal_gzip = head[97] == 2;
    let max_zoom = head[101];
    let raw = if root_off + root_len <= head.len() as u64 {
        head[root_off as usize..(root_off + root_len) as usize].to_vec()
    } else {
        fetch::get(&url, Some((root_off, root_off + root_len - 1))).ok()??
    };
    let root = directory(&if internal_gzip { gunzip(&raw)? } else { raw })?;
    Some(Archive { url, dir, root, leaf_offset: leaf_off, data_offset: data_off, max_zoom })
}

pub fn gunzip(b: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(b).take(64 << 20).read_to_end(&mut out).ok()?;
    Some(out)
}

fn varint(b: &[u8], at: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *b.get(*at)?;
        *at += 1;
        v |= ((byte & 0x7f) as u64) << shift;
        if byte < 0x80 {
            return Some(v);
        }
    }
    None
}

fn directory(b: &[u8]) -> Option<Vec<Entry>> {
    let mut at = 0;
    let n = varint(b, &mut at)? as usize;
    if n > 10_000_000 {
        return None;
    }
    let mut e = vec![Entry { id: 0, offset: 0, length: 0, run: 0 }; n];
    let mut last = 0u64;
    for x in e.iter_mut() {
        last += varint(b, &mut at)?;
        x.id = last;
    }
    for x in e.iter_mut() {
        x.run = varint(b, &mut at)? as u32;
    }
    for x in e.iter_mut() {
        x.length = varint(b, &mut at)? as u32;
    }
    for i in 0..n {
        let v = varint(b, &mut at)?;
        e[i].offset = if v == 0 && i > 0 { e[i - 1].offset + e[i - 1].length as u64 } else { v - 1 };
    }
    Some(e)
}

/// PMTiles' tile id: tiles of lower zooms first, then the Hilbert curve's order.
fn tile_id(z: u8, x: u32, y: u32) -> u64 {
    // Wikipedia's xy2d, as the PMTiles reference implementation does it.
    let base: u64 = (0..z as u64).map(|i| 1u64 << (2 * i)).sum();
    let n = 1u64 << z;
    let (mut x, mut y) = (x as u64, y as u64);
    let mut d = 0u64;
    let mut s = n / 2;
    while s > 0 {
        let rx = ((x & s) > 0) as u64;
        let ry = ((y & s) > 0) as u64;
        d += s * s * ((3 * rx) ^ ry);
        if ry == 0 {
            if rx == 1 {
                x = n - 1 - x;
                y = n - 1 - y;
            }
            std::mem::swap(&mut x, &mut y);
        }
        s /= 2;
    }
    base + d
}

fn find(dir: &[Entry], id: u64) -> Option<Entry> {
    let i = dir.partition_point(|e| e.id <= id);
    if i == 0 {
        return None;
    }
    let e = dir[i - 1];
    // run 0: a leaf directory; otherwise tiles id..id+run share it.
    if e.run == 0 || id < e.id + e.run as u64 { Some(e) } else { None }
}

static LEAVES: fetch::Memo<Arc<Vec<Entry>>> = fetch::Memo::new(256);

impl Archive {
    pub fn max_zoom(&self) -> u8 {
        self.max_zoom
    }

    fn leaf(&self, e: Entry) -> Option<Arc<Vec<Entry>>> {
        let key = (self.dir.to_string_lossy().len() as u64) << 56 ^ e.offset;
        LEAVES.get(key, || {
            let path = self.dir.join("leaves").join(format!("{}.dir", e.offset));
            let raw = match fetch::read(&path) {
                Some(b) => b,
                None => {
                    let start = self.leaf_offset + e.offset;
                    let b = fetch::get(&self.url, Some((start, start + e.length as u64 - 1))).ok()??;
                    fetch::write(&path, &b);
                    b
                }
            };
            directory(&gunzip(&raw)?).map(Arc::new)
        })
    }

    /// The tile's vector tile bytes (gunzipped); Some(empty) where the archive has none.
    pub fn tile(&self, z: u8, x: u32, y: u32) -> Option<Vec<u8>> {
        let path = self.dir.join(z.to_string()).join(x.to_string()).join(format!("{y}.mvt.gz"));
        if let Some(raw) = fetch::read(&path) {
            if raw.is_empty() {
                return Some(Vec::new());
            }
            if let Some(t) = gunzip(&raw) {
                return Some(t);
            }
        }
        let id = tile_id(z, x, y);
        let mut entry = find(&self.root, id);
        let mut depth = 0;
        while let Some(e) = entry {
            if e.run > 0 {
                break;
            }
            depth += 1;
            if depth > 4 {
                return None;
            }
            let leaf = self.leaf(e)?;
            entry = find(&leaf, id);
        }
        let Some(e) = entry else {
            fetch::write(&path, b"");
            return Some(Vec::new());
        };
        let start = self.data_offset + e.offset;
        let raw = fetch::get(&self.url, Some((start, start + e.length as u64 - 1))).ok()??;
        fetch::write(&path, &raw);
        gunzip(&raw)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn hilbert_ids() {
        // From the PMTiles spec's test vectors.
        assert_eq!(super::tile_id(0, 0, 0), 0);
        assert_eq!(super::tile_id(1, 0, 0), 1);
        assert_eq!(super::tile_id(1, 0, 1), 2);
        assert_eq!(super::tile_id(1, 1, 1), 3);
        assert_eq!(super::tile_id(1, 1, 0), 4);
        assert_eq!(super::tile_id(2, 0, 0), 5);
    }
}
