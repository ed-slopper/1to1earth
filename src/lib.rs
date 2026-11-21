//! Tellus for Horizon: the real Earth from geographic data, a port of Tellus's terrain (Yucareux,
//! LGPL-3.0) to a Horizon native mod, in a dimension of its own, `tellus:earth`. A registered chunk
//! generator builds the terrain from Mapterhorn elevation, ESA WorldCover land cover and the
//! Köppen climate map, a registered biome source places Tellus's biomes, and vanilla's carvers
//! and features (caves, trees, grass, ores, snow) finish it.
//!
//! `/earth` takes you there, `/earth tp <latitude> <longitude>` anywhere on it, `/earth where`
//! says where you are and `/earth leave` takes you back to the Overworld.
mod biome_table;
mod config;
mod datapack;
mod earth;
mod elevation;
mod fetch;
mod koppen;
mod map;
mod worldcover;

use mod_api::{
    BiomeSource, BlockKey, BlockState, ChunkAccess, ChunkGenerator, Command, CommandArgumentValue,
    CommandSource, DimensionSettings, Flow, Hooks, PlayerLoggedInEvent, RegisterCommandsEvent,
    Registrar, RegistrationContext, RunModCommandEvent, ServerHost, ServerTickEvent, ServerTickPhase,
    Text, VanillaStages,
};
use mod_api::{
    ClientHost, ClientTickPostEvent, MouseButtonPostEvent, ScreenMouseButtonPressedPreEvent,
    ScreenMouseButtonReleasedPreEvent, ScreenMouseDraggedPreEvent, ScreenMouseScrolledPreEvent,
    ScreenRenderPostEvent,
};
use std::{cell::Cell, collections::HashMap};

pub const DIMENSION: &str = "tellus:earth";

/// Every block the generator writes.
const BLOCKS: &[&str] = &[
    "minecraft:stone",
    "minecraft:deepslate",
    "minecraft:bedrock",
    "minecraft:water",
    "minecraft:ice",
    "minecraft:dirt",
    "minecraft:grass_block",
    "minecraft:podzol",
    "minecraft:coarse_dirt",
    "minecraft:mud",
    "minecraft:sand",
    "minecraft:sandstone",
    "minecraft:gravel",
    "minecraft:clay",
    "minecraft:andesite",
    "minecraft:snow_block",
    "minecraft:packed_ice",
    "minecraft:rooted_dirt",
    "minecraft:moss_block",
    "minecraft:packed_mud",
    "minecraft:red_sand",
    "minecraft:terracotta",
];

#[derive(Default)]
struct Mod;

#[mod_api::horizon_mod]
impl Hooks for Mod {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let keys = BLOCKS.iter().map(|n| (*n, r.block_key(n))).collect();
        r.register_chunk_generator("earth", Terrain { keys });
        r.register_biome_source("earth", Biomes);
        // The stack: one level per layer, all from the same generator (`earth::LAYERS`).
        for l in earth::LAYERS {
            let name = earth::layer_dim(l);
            r.register_dimension(
                name.trim_start_matches("tellus:"),
                DimensionSettings {
                    min_y: earth::MIN_Y,
                    height: earth::HEIGHT,
                    logical_height: earth::HEIGHT,
                    generator: "tellus:earth".into(),
                    noise_settings: "tellus:earth".into(),
                    mod_biome_source: Some("tellus:earth".into()),
                    sea_level: Some(earth::sea() + 1),
                    // One cloud deck for the whole stack, at global Y 1,264 (about 1,200 m).
                    cloud_height: Some((1264 - l * earth::SPAN) as f32),
                    ..Default::default()
                },
            );
        }
    }

    // --- the Earth map (map.rs) ---
    fn screen_render_post(&self, host: &dyn ClientHost, event: &mut ScreenRenderPostEvent<'_>) {
        map::draw(host, &mut *event.draw, event.screen);
    }

    fn screen_mouse_button_pressed_pre(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenMouseButtonPressedPreEvent<'_>,
    ) -> Flow {
        if map::pressed(host, event.screen, event.button) { Flow::Cancel } else { Flow::Continue }
    }

    fn screen_mouse_button_released_pre(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenMouseButtonReleasedPreEvent<'_>,
    ) -> Flow {
        if map::released(host, event.screen, event.button) { Flow::Cancel } else { Flow::Continue }
    }

    fn screen_mouse_dragged_pre(&self, host: &dyn ClientHost, event: &mut ScreenMouseDraggedPreEvent<'_>) -> Flow {
        if map::dragged(host, event.screen, event.delta) { Flow::Cancel } else { Flow::Continue }
    }

    fn screen_mouse_scrolled_pre(&self, host: &dyn ClientHost, event: &mut ScreenMouseScrolledPreEvent<'_>) -> Flow {
        if map::scrolled(host, event.screen, event.scroll[1]) { Flow::Cancel } else { Flow::Continue }
    }

    fn mouse_button_post(&self, host: &mut dyn ClientHost, _: &mut MouseButtonPostEvent) {
        map::act(host);
    }

    fn client_tick_post(&self, host: &mut dyn ClientHost, _: &mut ClientTickPostEvent) {
        map::act(host);
    }

    fn server_tick(&self, host: &mut dyn ServerHost, event: &mut ServerTickEvent<'_>) {
        if !matches!(event.phase, ServerTickPhase::ServerTickEnd { .. }) {
            return;
        }
        // Players crossing a layer's edge go on to the next layer, where the same ground is.
        for l in earth::LAYERS {
            let Some(level) = host.level(&earth::layer_dim(l)) else { continue };
            let Some(players) = level.online_players(host) else { continue };
            for player in players {
                let entity = player.entity();
                let Some([x, y, z]) = entity.position(host) else { continue };
                let to = if y > earth::CROSS as f64 && l < *earth::LAYERS.end() {
                    l + 1
                } else if y < -earth::CROSS as f64 && l > *earth::LAYERS.start() {
                    l - 1
                } else {
                    continue;
                };
                let ny = y - ((to - l) * earth::SPAN) as f64;
                let velocity = entity.velocity(host);
                let rotation = entity.rotation(host);
                if entity.teleport_to_level(host, &earth::layer_dim(to), [x, ny, z], rotation) {
                    if let Some(v) = velocity {
                        let _ = entity.set_velocity(host, v);
                    }
                }
            }
        }
    }

    fn register_commands(&self, _: &mut dyn ServerHost, event: &mut RegisterCommandsEvent<'_>) {
        event.commands.push(Command::new("earth"));
        event.commands.push(Command::new("earth where"));
        event.commands.push(Command::new("earth leave"));
        event
            .commands
            .push(Command::op("earth tp <place:text>", 2));
    }

    fn run_mod_command(&self, host: &mut dyn ServerHost, event: &mut RunModCommandEvent<'_>) -> Flow {
        let command = &event.command;
        if command.literals.first().map(String::as_str) != Some("earth") {
            return Flow::Continue;
        }
        let CommandSource::Player { player, .. } = &event.source else {
            return Flow::Cancel;
        };
        let player = *player;
        let say = |host: &mut dyn ServerHost, line: String| {
            let _ = player.tell(host, Text::literal(line));
        };
        match command.literals.get(1).map(String::as_str) {
            None => {
                let (lat, lon) = config::get().spawn;
                go(host, player, lat, lon);
            }
            Some("tp") => {
                // "47.47 -122.07", "47.47, -122.07" or "47.47,-122.07", as maps copy them.
                let text = match command.argument("place") {
                    Some(CommandArgumentValue::String(t)) => t.clone(),
                    _ => String::new(),
                };
                let nums: Vec<f64> = text
                    .split(|c: char| c == ',' || c.is_whitespace())
                    .filter(|w| !w.is_empty())
                    .filter_map(|w| w.trim_end_matches(['°', 'N', 'E']).parse().ok())
                    .collect();
                let (Some(&lat), Some(&lon)) = (nums.first(), nums.get(1)) else {
                    say(host, "Usage: /earth tp <latitude> <longitude>, e.g. /earth tp 47.4703, -122.0753".into());
                    return Flow::Cancel;
                };
                if lat.abs() > 85.0 || lon.abs() > 180.0 {
                    say(host, "Latitude -85 to 85, longitude -180 to 180.".into());
                    return Flow::Cancel;
                }
                go(host, player, lat, lon);
            }
            Some("where") => {
                let [x, _, z] = command.position;
                let (lat, lon) = earth::to_globe(x, z);
                say(host, format!("Here is {lat:.5}, {lon:.5} (on the Earth: /earth tp {lat:.5} {lon:.5})."));
            }
            Some("leave") => {
                let entity = player.entity();
                let _ = entity.teleport_to_level(host, "minecraft:overworld", [0.5, 100.0, 0.5], None);
                say(host, "Back to the Overworld.".into());
            }
            _ => {}
        }
        Flow::Cancel
    }

    fn player_logged_in(&self, host: &mut dyn ServerHost, event: &mut PlayerLoggedInEvent<'_>) {
        let Some(world) = host.world_dir() else {
            return;
        };
        let Some(player) = host.player(event.player) else {
            return;
        };
        if !datapack::ensure(&world) {
            let _ = player.tell(
                host,
                Text::literal("Tellus added its datapack to this world. Save and quit, then open the world again to go to the Earth (/earth)."),
            );
            return;
        }
        // A player's first time in this world: straight onto the Earth, at the spawn.
        let marker = world.join("tellus").join("visited").join(format!("{:032x}", event.player));
        if marker.exists() {
            return;
        }
        fetch::write(&marker, b"");
        let (lat, lon) = config::get().spawn;
        go(host, player, lat, lon);
        let _ = player.tell(
            host,
            Text::literal("Welcome to the Earth. /earth tp <latitude> <longitude> goes anywhere, /earth leave goes back."),
        );
    }
}

/// Send a player to a place on the Earth, on its ground (or its water).
fn go(host: &mut dyn ServerHost, player: mod_api::PlayerRef, lat: f64, lon: f64) {
    let (x, z) = earth::to_block(lat, lon);
    let (bx, bz) = (x.floor() as i32, z.floor() as i32);
    let c = earth::column(bx, bz);
    let y = if c.wet() { c.water } else { c.ground } + 1;
    // The layer that holds that height (`earth::SPAN`): Everest is two up, the trenches two down.
    let l = earth::layer_for(y);
    let local = y - l * earth::SPAN;
    let ok = player.entity().teleport_to_level(
        host,
        &earth::layer_dim(l),
        [bx as f64 + 0.5, local as f64, bz as f64 + 0.5],
        None,
    );
    let line = if ok {
        format!(
            "{lat:.4}, {lon:.4}: {:.0} m above the sea (Y {y} on the Earth, layer {l}: {bx} {local} {bz}), Köppen {}.",
            c.metres, c.koppen
        )
    } else {
        "The Earth didn't open. If this world is new to Tellus, save and quit, then open it again.".to_string()
    };
    let _ = player.tell(host, Text::literal(line));
}

struct Terrain {
    keys: HashMap<&'static str, BlockKey>,
}

/// Columns read round a chunk, for slopes and beaches.
const B: i32 = 3;
const W: i32 = 16 + 2 * B;

impl ChunkGenerator for Terrain {
    fn build_terrain(&self, chunk: &mut dyn ChunkAccess) {
        let states: HashMap<&str, BlockState> =
            self.keys.iter().map(|(n, k)| (*n, chunk.state(*k))).collect();
        let st = |n: &str| states.get(n).copied().unwrap_or(states["minecraft:stone"]);
        let (bedrock, water, ice, stone, deep) = (
            st("minecraft:bedrock"),
            st("minecraft:water"),
            st("minecraft:ice"),
            st("minecraft:stone"),
            st("minecraft:deepslate"),
        );
        let pos = chunk.pos();
        let (x0, z0) = (pos.x * 16, pos.z * 16);
        let min_y = chunk.min_y();
        let max_y = min_y + chunk.height() - 1;
        // This chunk's layer of the stack: its blocks are global Y minus `off`.
        let layer = earth::layer_of(chunk.level()).unwrap_or(0);
        let off = layer * earth::SPAN;
        let mut cols = Vec::with_capacity((W * W) as usize);
        for dz in -B..16 + B {
            for dx in -B..16 + B {
                cols.push(earth::column(x0 + dx, z0 + dz));
            }
        }
        let at = |dx: i32, dz: i32| &cols[((dz + B) * W + dx + B) as usize];
        for dz in 0..16 {
            for dx in 0..16 {
                let c = at(dx, dz);
                let top_here = if c.wet() { c.water } else { c.ground };
                if top_here - off < min_y {
                    continue; // all air in this layer
                }
                let slope = slope_at(&at, dx, dz);
                let beach = !c.ocean && (-B..=B).any(|oz| (-B..=B).any(|ox| at(dx + ox, dz + oz).ocean));
                let biome = earth::biome(c, slope);
                let (x, z) = (x0 + dx, z0 + dz);
                let pal = earth::palette(c, biome, slope, beach, x, z);
                let top = st(if c.wet() { pal.wet_top } else { pal.top });
                let filler = st(pal.filler);
                let soil = c.ground - pal.depth;
                let bottom = layer == *earth::LAYERS.start();
                for ly in min_y..=(c.ground - off).min(max_y) {
                    let y = ly + off;
                    let s = if bottom
                        && ly < min_y + 5
                        && (ly == min_y || earth::hash(x, ly, z) % 5 < (min_y + 5 - ly) as u32)
                    {
                        bedrock
                    } else if y == c.ground {
                        top
                    } else if y > soil {
                        filler
                    } else if earth::deepslate(y, earth::hash(x, y, z)) {
                        deep
                    } else {
                        stone
                    };
                    chunk.set_block([x, ly, z], s);
                }
                if c.wet() {
                    for ly in (c.ground + 1 - off).max(min_y)..=(c.water - off).min(max_y) {
                        chunk.set_block([x, ly, z], water);
                    }
                    if biome.starts_with("minecraft:frozen") || biome == "minecraft:deep_frozen_ocean" {
                        chunk.set_block([x, c.water - off, z], ice);
                    }
                }
            }
        }
    }

    fn vanilla_stages(&self) -> VanillaStages {
        // No vanilla carvers: they cut a fixed band of each level (about Y -64 to 180), which in the
        // stack's upper layers is open sky, and fill it with their aquifer's lava and water.
        VanillaStages { surface: false, carvers: false, structures: false, features: true }
    }
}

fn slope_at<'a>(at: &impl Fn(i32, i32) -> &'a earth::Column, dx: i32, dz: i32) -> i32 {
    let g = at(dx, dz).ground;
    [(1, 0), (-1, 0), (0, 1), (0, -1)]
        .iter()
        .map(|(ox, oz)| (at(dx + ox, dz + oz).ground - g).abs())
        .max()
        .unwrap_or(0)
}

struct Biomes;

thread_local! {
    /// The last few columns' biomes: the game asks for each quart of a column, bottom to top.
    static RECENT: Cell<[(i32, i32, u16); 4]> = const { Cell::new([(i32::MIN, i32::MIN, 0); 4]) };
    static NEXT: Cell<usize> = const { Cell::new(0) };
}

impl BiomeSource for Biomes {
    fn possible_biomes(&self) -> Vec<String> {
        earth::BIOMES.iter().map(|b| b.to_string()).collect()
    }

    fn noise_biome(&self, [qx, _, qz]: [i32; 3], _seed: i64) -> u16 {
        let recent = RECENT.get();
        if let Some(hit) = recent.iter().find(|(x, z, _)| *x == qx && *z == qz) {
            return hit.2;
        }
        let (bx, bz) = ((qx << 2) + 2, (qz << 2) + 2);
        let c = earth::column(bx, bz);
        let slope = [(1, 0), (0, 1)]
            .iter()
            .map(|(ox, oz)| (earth::column(bx + ox, bz + oz).ground - c.ground).abs())
            .max()
            .unwrap_or(0);
        let name = earth::biome(&c, slope);
        let id = earth::BIOMES.iter().position(|b| *b == name).unwrap_or(0) as u16;
        let mut recent = recent;
        let slot = NEXT.get();
        recent[slot] = (qx, qz, id);
        NEXT.set((slot + 1) % recent.len());
        RECENT.set(recent);
        id
    }
}
