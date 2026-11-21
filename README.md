# Tellus for Horizon

A port of [Tellus](https://github.com/Yucareux/Tellus) (Yucareux, LGPL-3.0), real-Earth terrain for
Minecraft, to a [Horizon](https://github.com/horizon-rust/horizon) native mod in Rust.

The Earth is a dimension of its own, `tellus:earth`, stacked 13 levels high so heights are true (see below). Its terrain comes
from a registered chunk generator, and a registered biome source places its biomes. Vanilla's
carvers and features finish it: caves, trees, grass, ores and snow.

## Play

- `Tellus.bat` plays the **Tellus** instance, which is Horizon with this mod built in.
- **The first time in a new world**, the mod writes its datapack into the world. That datapack holds
  the Earth level's noise settings, because Horizon reads worldgen data from world datapacks, not
  from a mod's own data. Save and quit, open the world again, and you land on the Earth.
- **Earth Map:** press Esc, then **Earth Map** at the pause screen's top left.
  - The map is shaded relief of the whole planet, with the sea in blue and a red dot where you are.
    Closer in, lakes and rivers from the ESA land cover appear a moment after the relief.
  - Drag to move it, scroll to zoom.
  - **Click a spot to go there:** the line under the map shows the `/earth tp` command for the spot
    under the pointer.
- Commands:
  - `/earth` goes to the spawn (Everest by default).
  - `/earth tp <latitude> <longitude>` goes anywhere (needs operator level 2 or cheats); pasted
  map coordinates work too: `/earth tp 47.470256, -122.075310`.
  - `/earth where` tells you where you are.
  - `/earth leave` takes you back to the Overworld.
- Settings are in `tellus.json`, in the instance folder (`%APPDATA%\horizon\instances\tellus`).
  They use Tellus's names. Change them before generating a world.

  | Setting | Default |
  |---|---|
  | `world_scale` | 1 (1:1; Tellus's own default is 30) |
  | `true_scale_latitude` | the spawn's latitude |
  | `terrestrial_height_scale`, `oceanic_height_scale` | 1 |
  | `height_offset` | 64 (sea level) |
  | `automatic_height_scaling` | on |
  | `spawn_latitude`, `spawn_longitude` | Everest |

  `true_scale_latitude` is where a block is exactly `world_scale` metres, across and up. Mercator
  stretches the map by sec(latitude), so Tellus's 1:1 is only true at the equator.

## Height: the stacked Earth

One Horizon level can't go past Y 2031: block positions store Y in 12 bits, and Horizon keeps the
vanilla limit, as Java's height mods do (Beyond Y, World Height Limit). So the Earth is a **stack
of 13 levels**, the way Java's stacked-dimension mods go higher (Simply Stacked Dimensions,
Vertically Stacked Dimensions).

- **Layers:** `tellus:earth` is layer 0. Above it are `tellus:earth_up1` to `_up6`, below it
  `tellus:earth_down1` to `_down6`. Each is a full Y -2032 to 2031 level.
- **Heights:** layer `l` holds global Y `local + l * 2032`, so every height is true. Everest stands
  at its full 8,848 m and the Mariana Trench at its full depth. The stack holds about Y -14,200 to
  14,200.
- **Overlap:** neighbouring layers overlap by half and carry the same terrain. Pass 1,024 blocks
  above or below a layer's middle and you move to the next one at the same x and z, keeping your
  speed. There are always about a thousand blocks of world in view above and below you.
- **Teleporting:** `/earth tp` picks the right layer by itself.
- **Clouds:** they sit at one real height, about 1,200 m, in every layer.
- **Limits:**
  - You can't see from one layer into the next, past about a thousand blocks.
  - Vanilla's cave carvers are off, because they cut a fixed band of each level, which is open sky
    in the upper layers.
  - Each level saves its own blocks, so a build in the overlap is only in the layer it was made in.

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
