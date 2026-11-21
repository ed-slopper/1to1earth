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
mod worldcover;

use mod_api::{
    BiomeSource, BlockKey, BlockState, ChunkAccess, ChunkGenerator, Command, CommandArgumentValue,
    CommandSource, DimensionSettings, Flow, Hooks, PlayerLoggedInEvent, RegisterCommandsEvent,
    Registrar, RegistrationContext, RunModCommandEvent, ServerHost, Text, VanillaStages,
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
        r.register_dimension(
            "earth",
            DimensionSettings {
                min_y: earth::MIN_Y,
                height: earth::HEIGHT,
                logical_height: earth::HEIGHT,
                generator: "tellus:earth".into(),
                noise_settings: "tellus:earth".into(),
                mod_biome_source: Some("tellus:earth".into()),
                sea_level: Some(earth::sea() + 1),
                ..Default::default()
            },
        );
    }

    fn register_commands(&self, _: &mut dyn ServerHost, event: &mut RegisterCommandsEvent<'_>) {
        event.commands.push(Command::new("earth"));
        event.commands.push(Command::new("earth where"));
        event.commands.push(Command::new("earth leave"));
        event
            .commands
            .push(Command::op("earth tp <latitude:number> <longitude:number>", 2));
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
                let num = |name| match command.argument(name) {
                    Some(CommandArgumentValue::Number(v)) => Some(*v),
                    _ => None,
                };
                let (Some(lat), Some(lon)) = (num("latitude"), num("longitude")) else {
                    say(host, "Usage: /earth tp <latitude> <longitude>".into());
                    return Flow::Cancel;
                };
                if lat.abs() > 84.0 || lon.abs() > 180.0 {
                    say(host, "Latitude -84 to 84, longitude -180 to 180.".into());
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
    let ok = player
        .entity()
        .teleport_to_level(host, DIMENSION, [bx as f64 + 0.5, y as f64, bz as f64 + 0.5], None);
    let line = if ok {
        format!("{lat:.4}, {lon:.4}: block {bx} {y} {bz}, {:.0} m above the sea, Köppen {}.", c.metres, c.koppen)
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
                let slope = slope_at(&at, dx, dz);
                let beach = !c.ocean && (-B..=B).any(|oz| (-B..=B).any(|ox| at(dx + ox, dz + oz).ocean));
                let biome = earth::biome(c, slope);
                let (x, z) = (x0 + dx, z0 + dz);
                let pal = earth::palette(c, biome, slope, beach, x, z);
                let top = st(if c.wet() { pal.wet_top } else { pal.top });
                let filler = st(pal.filler);
                let soil = c.ground - pal.depth;
                for y in min_y..=c.ground.min(max_y) {
                    let s = if y < min_y + 5
                        && (y == min_y || earth::hash(x, y, z) % 5 < (min_y + 5 - y) as u32)
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
                    chunk.set_block([x, y, z], s);
                }
                if c.wet() {
                    for y in c.ground + 1..=c.water.min(max_y) {
                        chunk.set_block([x, y, z], water);
                    }
                    if biome.starts_with("minecraft:frozen") || biome == "minecraft:deep_frozen_ocean" {
                        chunk.set_block([x, c.water, z], ice);
                    }
                }
            }
        }
    }

    fn vanilla_stages(&self) -> VanillaStages {
        VanillaStages { surface: false, carvers: true, structures: false, features: true }
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
