# Tellus for Horizon

A port of [Tellus](https://github.com/Yucareux/Tellus) (Yucareux, LGPL-3.0), real-Earth terrain for
Minecraft, to a [Horizon](https://github.com/horizon-rust/horizon) native mod in Rust.

The Earth is a dimension of its own, `tellus:earth`, running from Y -1024 to 2031. Its terrain comes
from a registered chunk generator, and a registered biome source places its biomes. Vanilla's
carvers and features finish it: caves, trees, grass, ores and snow.

## Play

- `Tellus.bat` plays the **Tellus** instance, which is Horizon with this mod built in.
- **The first time in a new world**, the mod writes its datapack into the world. That datapack holds
  the Earth level's noise settings, because Horizon reads worldgen data from world datapacks, not
  from a mod's own data. Save and quit, open the world again, and you land on the Earth.
- Commands:
  - `/earth` goes to the spawn (Everest by default).
  - `/earth tp <latitude> <longitude>` goes anywhere (needs operator level 2 or cheats).
  - `/earth where` tells you where you are.
  - `/earth leave` takes you back to the Overworld.
- Settings are in `tellus.json`, in the instance folder (`%APPDATA%\horizon\instances\tellus`). They
  use Tellus's names and defaults:

  | Setting | Default |
  |---|---|
  | `world_scale` | 30 metres per block |
  | `terrestrial_height_scale` | 1 |
  | `oceanic_height_scale` | 1 |
  | `height_offset` | 64 (sea level) |
  | `automatic_height_scaling` | on |
  | `spawn_latitude`, `spawn_longitude` | Everest |

  Change them before generating a world.

## What is ported

| Part | Tellus source | Here |
|---|---|---|
| Projection: Mercator, `world_scale` metres per block | `EarthProjection` | `earth.rs` |
| Height: metres × height scale × sec(lat) / scale + offset | `TerrainHeightTransform` | `earth.rs` |
| Elevation: Mapterhorn (land) and OpenWaters (sea floor) Terrarium WebP tiles; AWS Terrain Tiles fallback | `TellusElevationSource` | `elevation.rs` |
| Land cover: ESA WorldCover 2021 COGs by HTTP range | `WorldCoverCogSource` | `worldcover.rs` |
| Climate: Tellus's Köppen-Geiger GeoTIFF, downloaded once from Tellus's repository | `TellusKoppenSource` | `koppen.rs` |
| Biomes: land cover × Köppen table (`biome_classification_system.csv`), mangroves, oceans, rivers, snow | `EarthBiomeSource`, `BiomeClassification` | `earth.rs`, `biome_table.rs` |
| Surface palettes by biome, land cover and climate group | `EarthChunkGenerator.selectBaseSurfacePalette` | `earth.rs` |

**Not yet:**

- OSM/Overture roads, buildings and mapped rivers (Tellus's Arnis-derived code)
- RESOLVE ecoregions
- NOAA sea temperatures (approximated from latitude)
- Tellus's own trees and mountain zone rules
- Distant Horizons far terrain
- Real-time weather and time
- The preview and settings screens

Downloads are cached in `tellus\cache` in the instance folder.

## Build

```
horizon-launcher native-lock .        # only when dependencies change; keep the game's versions
horizon-launcher native-build . --instance tellus --trust-native-code
```

`Rebuild.bat` runs the build.

Notes:

- The lock must use the game's versions of any crate the prebuilt game also has.
- A mod can't link a second TLS stack, so downloads go through the system's `curl`.
- Only `.rs` and `.toml` files under `src/` are staged, so data tables are embedded as Rust source.
