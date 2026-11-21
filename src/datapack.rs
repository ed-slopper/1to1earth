//! The world's 1to1earth datapack: the Earth level's noise settings (`1to1earth:earth`, Y -2032 to 2031),
//! which vanilla's carvers and features need over the level's whole height. Horizon reads worldgen
//! data from a world's datapacks, not a mod's own data, so the mod writes the pack into each world
//! it opens; a world that didn't have it yet takes it the next time it is opened.
use std::path::Path;

pub const NAME: &str = "1to1earth_earth";

const MCMETA: &str = r#"{
  "pack": {
    "description": "Tellus: the Earth level's noise settings (Y -2032 to 2031)",
    "min_format": 121,
    "max_format": 121
  }
}
"#;

const NOISE_SETTINGS: &str = r#"{
  "noise": {"min_y": -2032, "height": 4064},
  "default_block": "minecraft:stone",
  "default_fluid": "minecraft:water",
  "noise_router": {
    "chunk_surface_level": "minecraft:overworld/chunk_surface_level",
    "continents": "minecraft:overworld/continents",
    "depth": "minecraft:overworld/depth",
    "erosion": "minecraft:overworld/erosion",
    "final_density": "minecraft:overworld/final_density",
    "ridges": "minecraft:overworld/ridges",
    "temperature": "minecraft:overworld/temperature",
    "vegetation": "minecraft:overworld/vegetation"
  },
  "material_rule": "minecraft:overworld",
  "spawn_target": [],
  "sea_level": 64,
  "disable_mob_generation": false,
  "legacy_random_source": false
}
"#;

/// Make sure `world` has the pack, as this build writes it. True when it was there already, so
/// the open world has read it; false when it was missing or older, and the world must be opened
/// again before the Earth level can open.
pub fn ensure(world: &Path) -> bool {
    let dir = world.join("datapacks").join(NAME);
    let file = dir.join("data/1to1earth/worldgen/noise_settings/earth.json");
    let current = std::fs::read_to_string(&file).ok().as_deref() == Some(NOISE_SETTINGS);
    if !current {
        crate::fetch::write(&dir.join("pack.mcmeta"), MCMETA.as_bytes());
        crate::fetch::write(&file, NOISE_SETTINGS.as_bytes());
    }
    current
}
