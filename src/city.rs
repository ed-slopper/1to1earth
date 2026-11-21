//! Buildings and roads from Overture Maps (OpenStreetMap and others; ODbL), as Tellus draws them:
//! every building on its real footprint at its real height, with floors, windows, lit rooms and
//! a roof (pitched on houses), in materials by its kind and mapped colours; every road at its
//! class's width. Read from Overture's `buildings` and `transportation` PMTiles at zoom 14.
use crate::{
    config, earth, fetch, mvt,
    pmtiles::{self, Archive},
};
use std::{collections::HashMap, f64::consts::PI, sync::Arc};

const ZOOM: u8 = 14;

/// Every block this module writes, resolved by the generator at registration.
pub const BLOCKS: &[&str] = &[
    "minecraft:gray_concrete",
    "minecraft:light_gray_concrete",
    "minecraft:white_concrete",
    "minecraft:black_concrete",
    "minecraft:yellow_concrete",
    "minecraft:red_concrete",
    "minecraft:orange_concrete",
    "minecraft:brown_concrete",
    "minecraft:green_concrete",
    "minecraft:blue_concrete",
    "minecraft:light_blue_concrete",
    "minecraft:cyan_concrete",
    "minecraft:white_terracotta",
    "minecraft:light_gray_terracotta",
    "minecraft:terracotta",
    "minecraft:brown_terracotta",
    "minecraft:cyan_terracotta",
    "minecraft:bricks",
    "minecraft:stone_bricks",
    "minecraft:smooth_stone",
    "minecraft:smooth_sandstone",
    "minecraft:quartz_block",
    "minecraft:polished_andesite",
    "minecraft:oak_planks",
    "minecraft:spruce_planks",
    "minecraft:dark_oak_planks",
    "minecraft:deepslate_tiles",
    "minecraft:glass",
    "minecraft:light_blue_stained_glass",
    "minecraft:gray_stained_glass",
    "minecraft:glowstone",
    "minecraft:dirt_path",
    "minecraft:gravel",
    "minecraft:air",
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    House,
    Residential,
    Commercial,
    Industrial,
    Other,
}

pub struct Building {
    id: u64,
    /// Pieces of the footprint (one per tile it was cut into), each its own even-odd ring set, in
    /// block coordinates (x, z).
    pieces: Vec<Vec<Vec<[f64; 2]>>>,
    bbox: [f64; 4],
    /// Global Y of the ground floor, the bottom and top of its walls.
    base: i32,
    bottom: i32,
    top: i32,
    kind: Kind,
    /// Wall, window, roof, floor.
    wall: &'static str,
    window: &'static str,
    roof: &'static str,
    floor: &'static str,
    pitched: bool,
    floor_h: i32,
}

pub struct Road {
    pts: Vec<[f64; 2]>,
    bbox: [f64; 4],
    half: f64,
    surface: &'static str,
    marked: bool,
}

#[derive(Default)]
pub struct TileData {
    buildings: Vec<Arc<Building>>,
    roads: Vec<Arc<Road>>,
}

/// Ground metres a block spans at a latitude.
fn metres_per_block(lat: f64) -> f64 {
    config::get().scale() * lat.to_radians().cos()
}

/// Cities are drawn at 4 m a block and finer; coarser, a house would be a block.
pub fn enabled() -> bool {
    let c = config::get();
    c.buildings && c.world_scale <= 4.0
}

fn tile_lonlat(z: u8, x: u32, y: u32, px: f64, py: f64, extent: f64) -> (f64, f64) {
    let n = (1u64 << z) as f64;
    let lon = (x as f64 + px / extent) / n * 360.0 - 180.0;
    let t = PI * (1.0 - 2.0 * (y as f64 + py / extent) / n);
    (t.sinh().atan().to_degrees(), lon)
}

fn tile_of(lat: f64, lon: f64, z: u8) -> (u32, u32) {
    let n = (1u64 << z) as f64;
    let x = ((lon + 180.0) / 360.0 * n).floor().clamp(0.0, n - 1.0);
    let lat = lat.clamp(-85.05, 85.05).to_radians();
    let y = ((1.0 - (lat.tan() + 1.0 / lat.cos()).ln() / PI) / 2.0 * n).floor().clamp(0.0, n - 1.0);
    (x as u32, y as u32)
}

static TILES: fetch::Memo<Arc<TileData>> = fetch::Memo::new(96);

fn tile(x: u32, y: u32) -> Option<Arc<TileData>> {
    TILES.get((x as u64) << 32 | y as u64, || Some(Arc::new(load(x, y))))
}

fn load(x: u32, y: u32) -> TileData {
    let mut out = TileData::default();
    let c = config::get();
    if c.buildings {
        if let Some(a) = pmtiles::archive("buildings") {
            buildings(&a, x, y, &mut out);
        }
    }
    if c.roads {
        if let Some(a) = pmtiles::archive("transportation") {
            roads(&a, x, y, &mut out);
        }
    }
    out
}

fn ring_to_blocks(ring: &[[i32; 2]], x: u32, y: u32, extent: f64) -> Vec<[f64; 2]> {
    ring.iter()
        .map(|p| {
            let (lat, lon) = tile_lonlat(ZOOM, x, y, p[0] as f64, p[1] as f64, extent);
            let (bx, bz) = earth::to_block(lat, lon);
            [bx, bz]
        })
        .collect()
}

fn bbox_of(pts: impl Iterator<Item = [f64; 2]>) -> [f64; 4] {
    let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for p in pts {
        b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
    }
    b
}

fn hash_str(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
}

fn buildings(a: &Archive, x: u32, y: u32, out: &mut TileData) {
    let z = ZOOM.min(a.max_zoom());
    let Some(bytes) = a.tile(z, x, y) else { return };
    let layers = mvt::decode(&bytes, &["building", "building_part"]);
    let parts_of: std::collections::HashSet<String> = layers
        .iter()
        .filter(|l| l.name == "building_part")
        .flat_map(|l| l.features.iter())
        .filter_map(|f| f.tags.get("building_id").and_then(|v| v.str()).map(str::to_string))
        .collect();
    for layer in &layers {
        let extent = layer.extent as f64;
        for f in &layer.features {
            if f.kind != 3 || f.tags.get("is_underground").is_some_and(|v| v.truthy()) {
                continue;
            }
            let id_str = f.tags.get("id").and_then(|v| v.str()).unwrap_or("");
            // A building drawn by its parts: the parts, not the outline.
            if layer.name == "building" && parts_of.contains(id_str) {
                continue;
            }
            let rings: Vec<Vec<[f64; 2]>> = f.parts.iter().map(|r| ring_to_blocks(r, x, y, extent)).collect();
            if rings.is_empty() {
                continue;
            }
            let bbox = bbox_of(rings.iter().flatten().copied());
            let (cx, cz) = ((bbox[0] + bbox[2]) / 2.0, (bbox[1] + bbox[3]) / 2.0);
            let (lat, _) = earth::to_globe(cx, cz);
            let mpb = metres_per_block(lat);
            let tag = |k: &str| f.tags.get(k);
            let class = tag("class").and_then(|v| v.str()).unwrap_or("");
            let subtype = tag("subtype").and_then(|v| v.str()).unwrap_or("");
            let floors = tag("num_floors").and_then(|v| v.num());
            let kind = kind_of(class, subtype);
            let height_m = tag("height").and_then(|v| v.num()).filter(|h| *h > 1.0).or(floors.map(|n| n * 3.2 + 1.0)).unwrap_or(
                match kind {
                    Kind::House => 7.0,
                    Kind::Residential => 12.0,
                    Kind::Commercial => 14.0,
                    Kind::Industrial => 9.0,
                    Kind::Other => 7.0,
                },
            );
            let min_m = tag("min_height").and_then(|v| v.num()).unwrap_or(0.0).max(0.0);
            let id = hash_str(id_str) ^ (bbox[0] as i64 as u64).rotate_left(17) ^ (bbox[1] as i64 as u64);
            // The ground floor stands on the ground at the footprint's middle (the same in every
            // chunk the building is in).
            let ground = earth::column(cx.floor() as i32, cz.floor() as i32);
            if ground.ocean {
                continue;
            }
            let base = ground.ground;
            let h = ((height_m / mpb).round() as i32).max(3);
            let bottom = base + (min_m / mpb).round() as i32;
            let top = base + h;
            let seed = id as u32;
            let facade = tag("facade_color").and_then(|v| v.str()).and_then(parse_colour);
            let roofc = tag("roof_color").and_then(|v| v.str()).and_then(parse_colour);
            let (wall, window, roof, floor) = materials(kind, h as f64 * mpb, seed, facade, roofc);
            let shape = tag("roof_shape").and_then(|v| v.str()).unwrap_or("");
            let w = (bbox[2] - bbox[0]).min(bbox[3] - bbox[1]);
            let pitched = matches!(shape, "gabled" | "hipped" | "pyramidal" | "gambrel" | "mansard")
                || (shape.is_empty() && kind == Kind::House && w < 24.0);
            out.buildings.push(Arc::new(Building {
                id,
                pieces: vec![rings],
                bbox,
                base,
                bottom: bottom.max(base),
                top,
                kind,
                wall,
                window,
                roof,
                floor,
                pitched: pitched && layer.name == "building" && h <= (14.0 / mpb) as i32,
                floor_h: if mpb <= 1.25 { 4 } else { 3 },
            }));
        }
    }
}

fn kind_of(class: &str, subtype: &str) -> Kind {
    match (subtype, class) {
        (_, "house" | "detached" | "semidetached_house" | "terrace" | "bungalow" | "cabin" | "farm" | "hut" | "garage"
            | "garages" | "shed") => Kind::House,
        (_, "apartments" | "dormitory" | "residential") | ("residential", _) => Kind::Residential,
        (_, "commercial" | "office" | "retail" | "hotel" | "supermarket" | "hospital" | "university" | "school"
            | "college" | "civic" | "government" | "public") => Kind::Commercial,
        ("commercial" | "education" | "medical" | "civic" | "service" | "entertainment" | "transportation", _) => {
            Kind::Commercial
        }
        (_, "industrial" | "warehouse" | "manufacture" | "factory" | "hangar") | ("industrial" | "agricultural", _) => {
            Kind::Industrial
        }
        _ => Kind::Other,
    }
}

fn parse_colour(s: &str) -> Option<[u8; 3]> {
    let s = s.trim().trim_start_matches('#');
    let named = match s.to_ascii_lowercase().as_str() {
        "white" => Some([240, 240, 235]),
        "grey" | "gray" => Some([140, 140, 140]),
        "black" => Some([30, 30, 30]),
        "red" => Some([160, 50, 40]),
        "brown" => Some([115, 80, 55]),
        "yellow" | "beige" | "cream" => Some([225, 205, 150]),
        "blue" => Some([60, 90, 160]),
        "green" => Some([80, 120, 70]),
        "orange" => Some([200, 110, 50]),
        _ => None,
    };
    if named.is_some() {
        return named;
    }
    if s.len() == 6 {
        let v = u32::from_str_radix(s, 16).ok()?;
        return Some([(v >> 16) as u8, (v >> 8) as u8, v as u8]);
    }
    None
}

/// The facade block nearest a mapped colour.
fn nearest(c: [u8; 3]) -> &'static str {
    const P: &[(&str, [u8; 3])] = &[
        ("minecraft:white_concrete", [207, 213, 214]),
        ("minecraft:light_gray_concrete", [125, 125, 115]),
        ("minecraft:gray_concrete", [55, 58, 62]),
        ("minecraft:black_concrete", [8, 10, 15]),
        ("minecraft:bricks", [150, 97, 83]),
        ("minecraft:red_concrete", [142, 33, 33]),
        ("minecraft:orange_concrete", [224, 97, 1]),
        ("minecraft:yellow_concrete", [241, 175, 21]),
        ("minecraft:brown_concrete", [96, 60, 32]),
        ("minecraft:green_concrete", [73, 91, 36]),
        ("minecraft:blue_concrete", [45, 47, 143]),
        ("minecraft:light_blue_concrete", [36, 137, 199]),
        ("minecraft:cyan_concrete", [21, 119, 136]),
        ("minecraft:white_terracotta", [210, 178, 161]),
        ("minecraft:terracotta", [152, 94, 68]),
        ("minecraft:smooth_sandstone", [223, 214, 170]),
        ("minecraft:quartz_block", [236, 230, 223]),
        ("minecraft:stone_bricks", [122, 122, 122]),
    ];
    let d = |p: [u8; 3]| (0..3).map(|i| (p[i] as i32 - c[i] as i32).pow(2)).sum::<i32>();
    P.iter().min_by_key(|(_, p)| d(*p)).map(|(n, _)| *n).unwrap_or("minecraft:stone_bricks")
}

fn pick(list: &[&'static str], seed: u32) -> &'static str {
    list[(seed as usize) % list.len()]
}

fn materials(kind: Kind, height_m: f64, seed: u32, facade: Option<[u8; 3]>, roofc: Option<[u8; 3]>)
    -> (&'static str, &'static str, &'static str, &'static str) {
    let tall = height_m >= 24.0;
    let (walls, windows, roofs, floor): (&[&str], &[&str], &[&str], &str) = match kind {
        Kind::House => (
            &["minecraft:bricks", "minecraft:white_terracotta", "minecraft:light_gray_terracotta", "minecraft:oak_planks",
              "minecraft:spruce_planks", "minecraft:smooth_sandstone", "minecraft:white_concrete"],
            &["minecraft:glass"],
            &["minecraft:dark_oak_planks", "minecraft:deepslate_tiles", "minecraft:spruce_planks", "minecraft:brown_terracotta"],
            "minecraft:oak_planks",
        ),
        Kind::Industrial => (
            &["minecraft:light_gray_concrete", "minecraft:gray_concrete", "minecraft:smooth_stone", "minecraft:stone_bricks"],
            &["minecraft:gray_stained_glass"],
            &["minecraft:smooth_stone", "minecraft:gray_concrete"],
            "minecraft:smooth_stone",
        ),
        _ if tall || kind == Kind::Commercial => (
            &["minecraft:white_concrete", "minecraft:light_gray_concrete", "minecraft:quartz_block", "minecraft:polished_andesite",
              "minecraft:gray_concrete", "minecraft:smooth_stone", "minecraft:cyan_terracotta"],
            &["minecraft:light_blue_stained_glass", "minecraft:glass", "minecraft:gray_stained_glass"],
            &["minecraft:smooth_stone", "minecraft:gray_concrete"],
            "minecraft:smooth_stone",
        ),
        _ => (
            &["minecraft:bricks", "minecraft:light_gray_concrete", "minecraft:white_concrete", "minecraft:stone_bricks",
              "minecraft:smooth_sandstone", "minecraft:terracotta", "minecraft:white_terracotta"],
            &["minecraft:glass"],
            &["minecraft:smooth_stone", "minecraft:gray_concrete", "minecraft:dark_oak_planks"],
            "minecraft:oak_planks",
        ),
    };
    let wall = facade.map(nearest).unwrap_or_else(|| pick(walls, seed));
    let roof = roofc.map(nearest).unwrap_or_else(|| pick(roofs, seed >> 8));
    (wall, pick(windows, seed >> 16), roof, floor)
}

fn roads(a: &Archive, x: u32, y: u32, out: &mut TileData) {
    let z = ZOOM.min(a.max_zoom());
    let Some(bytes) = a.tile(z, x, y) else { return };
    for layer in mvt::decode(&bytes, &["segment"]) {
        let extent = layer.extent as f64;
        for f in &layer.features {
            if f.kind != 2 {
                continue;
            }
            let subtype = f.tags.get("subtype").and_then(|v| v.str()).unwrap_or("road");
            let class = f.tags.get("class").and_then(|v| v.str()).unwrap_or("");
            let (width_m, surface, marked) = match (subtype, class) {
                ("water", _) => continue,
                ("rail", _) => (3.0, "minecraft:gravel", false),
                (_, "motorway") => (16.0, "minecraft:gray_concrete", true),
                (_, "trunk") => (13.0, "minecraft:gray_concrete", true),
                (_, "primary") => (11.0, "minecraft:gray_concrete", true),
                (_, "secondary") => (9.0, "minecraft:gray_concrete", true),
                (_, "tertiary") => (8.0, "minecraft:gray_concrete", true),
                (_, "residential" | "unclassified" | "living_street") => (6.0, "minecraft:gray_concrete", false),
                (_, "service" | "parking_aisle" | "driveway") => (4.0, "minecraft:gray_concrete", false),
                (_, "pedestrian" | "footway" | "sidewalk" | "crosswalk" | "steps") => (2.5, "minecraft:stone_bricks", false),
                (_, "cycleway") => (2.5, "minecraft:light_gray_concrete", false),
                (_, "path" | "track" | "bridleway") => (2.5, "minecraft:dirt_path", false),
                _ => (5.0, "minecraft:gray_concrete", false),
            };
            for part in &f.parts {
                let pts = ring_to_blocks(part, x, y, extent);
                if pts.len() < 2 {
                    continue;
                }
                let (lat, _) = earth::to_globe(pts[0][0], pts[0][1]);
                let half = (width_m / metres_per_block(lat) / 2.0).max(0.6);
                let b = bbox_of(pts.iter().copied());
                out.roads.push(Arc::new(Road {
                    bbox: [b[0] - half, b[1] - half, b[2] + half, b[3] + half],
                    pts,
                    half,
                    surface,
                    marked,
                }));
            }
        }
    }
}

/// The buildings and roads that reach chunk-sized box `[x0, z0, x1, z1]` (blocks).
pub fn near(x0: f64, z0: f64, x1: f64, z1: f64) -> (Vec<Arc<Building>>, Vec<Arc<Road>>) {
    let (mut bs, mut rs): (Vec<Arc<Building>>, Vec<Arc<Road>>) = (Vec::new(), Vec::new());
    let (la0, lo0) = earth::to_globe(x0 - 32.0, z0 - 32.0);
    let (la1, lo1) = earth::to_globe(x1 + 32.0, z1 + 32.0);
    let (tx0, ty0) = tile_of(la0.max(la1), lo0.min(lo1), ZOOM);
    let (tx1, ty1) = tile_of(la0.min(la1), lo0.max(lo1), ZOOM);
    let hit = |b: &[f64; 4]| b[0] <= x1 && b[2] >= x0 && b[1] <= z1 && b[3] >= z0;
    let mut by_id: HashMap<u64, usize> = HashMap::new();
    for tx in tx0..=tx1 {
        for ty in ty0..=ty1 {
            let Some(t) = tile(tx, ty) else { continue };
            for b in &t.buildings {
                if !hit(&b.bbox) {
                    continue;
                }
                // A building cut by tile edges: its pieces together.
                if let Some(&i) = by_id.get(&b.id) {
                    let mut merged = (*bs[i]).clone_shape();
                    merged.pieces.extend(b.pieces.iter().cloned());
                    bs[i] = Arc::new(merged);
                } else {
                    by_id.insert(b.id, bs.len());
                    bs.push(b.clone());
                }
            }
            rs.extend(t.roads.iter().filter(|r| hit(&r.bbox)).cloned());
        }
    }
    (bs, rs)
}

impl Building {
    fn clone_shape(&self) -> Building {
        Building {
            id: self.id,
            pieces: self.pieces.clone(),
            bbox: self.bbox,
            base: self.base,
            bottom: self.bottom,
            top: self.top,
            kind: self.kind,
            wall: self.wall,
            window: self.window,
            roof: self.roof,
            floor: self.floor,
            pitched: false,
            floor_h: self.floor_h,
        }
    }

    fn contains(&self, x: f64, z: f64) -> bool {
        if x < self.bbox[0] || x > self.bbox[2] || z < self.bbox[1] || z > self.bbox[3] {
            return false;
        }
        self.pieces.iter().any(|rings| {
            let mut inside = false;
            for r in rings {
                for w in r.windows(2) {
                    let ([ax, az], [bx, bz]) = (w[0], w[1]);
                    if (az > z) != (bz > z) && x < ax + (z - az) * (bx - ax) / (bz - az) {
                        inside = !inside;
                    }
                }
            }
            inside
        })
    }

    /// Distance from a point to the footprint's outline, for a pitched roof.
    fn edge_distance(&self, x: f64, z: f64) -> f64 {
        let mut best = f64::MAX;
        for rings in &self.pieces {
            for r in rings {
                for w in r.windows(2) {
                    best = best.min(seg_dist(w[0], w[1], [x, z]).0);
                }
            }
        }
        best
    }
}

/// Distance from `p` to segment `a`-`b`, and how far along it the nearest point is.
fn seg_dist(a: [f64; 2], b: [f64; 2], p: [f64; 2]) -> (f64, f64) {
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dz * dz;
    let t = if len2 > 0.0 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dz) / len2).clamp(0.0, 1.0) } else { 0.0 };
    let (qx, qz) = (a[0] + t * dx, a[1] + t * dz);
    (((p[0] - qx).powi(2) + (p[1] - qz).powi(2)).sqrt(), t * len2.sqrt())
}

/// What a column of the chunk gets: a road surface at the ground, and the building over it.
pub struct Painter {
    buildings: Vec<Arc<Building>>,
    roads: Vec<Arc<Road>>,
}

impl Painter {
    pub fn for_chunk(x0: i32, z0: i32) -> Option<Painter> {
        if !enabled() {
            return None;
        }
        let (buildings, roads) = near(x0 as f64, z0 as f64, x0 as f64 + 16.0, z0 as f64 + 16.0);
        (!buildings.is_empty() || !roads.is_empty()).then_some(Painter { buildings, roads })
    }

    /// Paint column `x, z` whose ground is global `ground`; `set(global_y, block)`. Water
    /// columns get nothing.
    pub fn column(&self, x: i32, z: i32, ground: i32, set: &mut dyn FnMut(i32, &'static str)) {
        let (px, pz) = (x as f64 + 0.5, z as f64 + 0.5);
        // The road under it, if any (the widest one's surface wins).
        let mut road: Option<(&Road, f64, f64)> = None;
        for r in &self.roads {
            if px < r.bbox[0] || px > r.bbox[2] || pz < r.bbox[1] || pz > r.bbox[3] {
                continue;
            }
            for (i, w) in r.pts.windows(2).enumerate() {
                let (d, along) = seg_dist(w[0], w[1], [px, pz]);
                if d <= r.half && road.is_none_or(|(o, _, _)| r.half > o.half) {
                    road = Some((r, d, along + i as f64 * 1000.0));
                }
            }
        }
        if let Some((r, d, along)) = road {
            let dash = r.marked && d < 0.5 && (along.rem_euclid(1000.0) as i64 / 3) % 2 == 0;
            set(ground, if dash { "minecraft:white_concrete" } else { r.surface });
            for y in ground + 1..=ground + 2 {
                set(y, "minecraft:air");
            }
        }
        for b in &self.buildings {
            if !b.contains(px, pz) {
                continue;
            }
            let edge = !(b.contains(px + 1.0, pz) && b.contains(px - 1.0, pz) && b.contains(px, pz + 1.0) && b.contains(px, pz - 1.0));
            // Foundation up to the floor, and the hill inside cleared.
            for y in ground + 1..b.bottom {
                set(y, "minecraft:stone_bricks");
            }
            for y in b.bottom + 1..=ground.min(b.top + 8) {
                set(y, "minecraft:air");
            }
            set(b.bottom, if edge { b.wall } else { b.floor });
            let tall = b.kind == Kind::Commercial || b.top - b.base >= 24;
            for y in b.bottom + 1..b.top {
                let row = (y - b.base).rem_euclid(b.floor_h);
                let block = if edge {
                    let along = (x + z).rem_euclid(if b.kind == Kind::House { 3 } else { 4 });
                    let window_row = row == 2 || (tall && row == 1) || (tall && b.floor_h >= 4 && row == 3);
                    if window_row && along != 0 { b.window } else { b.wall }
                } else if row == 0 {
                    // A floor, with a light in it now and then: lit rooms at night.
                    if x.rem_euclid(6) == 0 && z.rem_euclid(6) == 0 { "minecraft:glowstone" } else { b.floor }
                } else {
                    "minecraft:air"
                };
                set(y, block);
            }
            if b.pitched {
                let rise = (b.edge_distance(px, pz) * 0.75).floor() as i32;
                for y in b.top..=b.top + rise {
                    set(y, b.roof);
                }
            } else {
                set(b.top, b.roof);
                if edge && b.top - b.base >= 12 {
                    set(b.top + 1, b.wall);
                }
            }
        }
    }
}
