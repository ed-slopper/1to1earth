//! The world's settings, with Tellus's defaults (`EarthGeneratorSettings`): the world scale in
//! metres per block, the height scales and offset, and where `/earth` takes a new player.
//!
//! Read once from `tellus.json` in the game's folder (the server's on a dedicated server), or the
//! file `TELLUS_CONFIG` names. A missing file is written with the defaults so it can be edited.
//! Change the scale before a world is made: chunks already generated keep the old one.
use mod_api::serde_json::{self, Value, json};
use std::{path::PathBuf, sync::OnceLock};

#[derive(Clone, Debug)]
pub struct Config {
    /// Metres per block across (Tellus's `world_scale`, default 30).
    pub world_scale: f64,
    /// Multipliers on land heights and sea depths (`terrestrial_height_scale`, `oceanic_height_scale`).
    pub terrestrial_height_scale: f64,
    pub oceanic_height_scale: f64,
    /// The sea's level: Y of sea-level ground (`height_offset`, default 64).
    pub height_offset: i32,
    /// Heights scaled by the projection's stretch at each latitude, so slopes keep their real
    /// steepness (`automatic_height_scaling`, default on).
    pub automatic_height_scaling: bool,
    /// Where a player first arrives and `/earth` goes (default Everest, as Tellus).
    pub spawn: (f64, f64),
    /// Where the downloads are kept.
    pub cache: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            world_scale: 30.0,
            terrestrial_height_scale: 1.0,
            oceanic_height_scale: 1.0,
            height_offset: 64,
            automatic_height_scaling: true,
            spawn: (27.9881, 86.925),
            cache: PathBuf::from("tellus").join("cache"),
        }
    }
}

pub fn get() -> &'static Config {
    static CONFIG: OnceLock<Config> = OnceLock::new();
    CONFIG.get_or_init(load)
}

fn load() -> Config {
    let mut c = Config::default();
    let path = std::env::var_os("TELLUS_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("tellus.json"));
    match std::fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str::<Value>(&text) {
            Ok(v) => {
                if let Some(s) = v["world_scale"].as_f64().filter(|s| *s >= 1.0 && *s <= 1000.0) {
                    c.world_scale = s;
                }
                if let Some(s) = v["terrestrial_height_scale"].as_f64().filter(|s| *s > 0.0 && *s <= 50.0) {
                    c.terrestrial_height_scale = s;
                }
                if let Some(s) = v["oceanic_height_scale"].as_f64().filter(|s| *s > 0.0 && *s <= 50.0) {
                    c.oceanic_height_scale = s;
                }
                if let Some(o) = v["height_offset"].as_i64().filter(|o| (-512..=512).contains(o)) {
                    c.height_offset = o as i32;
                }
                if let Some(b) = v["automatic_height_scaling"].as_bool() {
                    c.automatic_height_scaling = b;
                }
                if let (Some(lat), Some(lon)) = (v["spawn_latitude"].as_f64(), v["spawn_longitude"].as_f64()) {
                    if lat.abs() <= 85.0 && lon.abs() <= 180.0 {
                        c.spawn = (lat, lon);
                    }
                }
                if let Some(dir) = v["cache"].as_str() {
                    c.cache = PathBuf::from(dir);
                }
            }
            Err(e) => eprintln!("[tellus] {} is not JSON ({e}); using the defaults", path.display()),
        },
        Err(_) => {
            let text = serde_json::to_string_pretty(&json!({
                "world_scale": c.world_scale,
                "terrestrial_height_scale": c.terrestrial_height_scale,
                "oceanic_height_scale": c.oceanic_height_scale,
                "height_offset": c.height_offset,
                "automatic_height_scaling": c.automatic_height_scaling,
                "spawn_latitude": c.spawn.0,
                "spawn_longitude": c.spawn.1,
                "cache": c.cache.to_string_lossy(),
            }))
            .unwrap_or_default();
            let _ = std::fs::write(&path, text);
        }
    }
    eprintln!(
        "[tellus] 1:{} scale, heights x{} / x{}, sea at Y {}, spawn {:.4}, {:.4}, cache {}",
        c.world_scale,
        c.terrestrial_height_scale,
        c.oceanic_height_scale,
        c.height_offset,
        c.spawn.0,
        c.spawn.1,
        c.cache.display()
    );
    c
}
