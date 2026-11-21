# 1to1earth

The real Earth at 1:1 in [Horizon](https://github.com/horizon-rust/horizon), as a native mod:
- true heights from Everest to the trenches
- land cover and Köppen climate biomes
- buildings and roads from Overture Maps
- an Earth Map to pick where to go

It's a Rust port of [Tellus](https://github.com/Yucareux/Tellus) (Yucareux, LGPL-3.0), real-Earth
terrain for Minecraft.

## Install

The mod is `1to1earth.hmod` on this repository's
[Releases](https://github.com/ed-slopper/1to1earth/releases).

**In the Horizon launcher:** go to **Mods**, choose **Add from GitHub**, paste
`https://github.com/ed-slopper/1to1earth` and accept the native-code notice. The launcher builds it
into your game. On the command line:

```
horizon-launcher mods add-github https://github.com/ed-slopper/1to1earth --trust-native-code
```

**By hand:**

1. Put `1to1earth.hmod` in your instance's `horizon-native-mods\` folder.
2. Add the line `mod.1to1earth:true` to the instance's `horizon-mods.txt`.
3. Build it: `horizon-launcher native-build --instance <instance> --trust-native-code`

It's built for Horizon's Nov 21 2026 Nightly (`f037744`), Minecraft 26.3. Native mods run unsandboxed,
so read the source first.

## Play

- `1to1earth.bat` plays the **1to1earth** instance. `Rebuild.bat` packages the source, installs it and builds it.
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
- Settings are in `1to1earth.json`, in the instance folder (`%APPDATA%\horizon\instances\tellus`).
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

- **Layers:** `1to1earth:earth` is layer 0. Above it are `1to1earth:earth_up1` to `_up6`, below it
  `1to1earth:earth_down1` to `_down6`. Each is a full Y -2032 to 2031 level.
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

## Buildings and roads

Like WorldCraft and Tellus, buildings and roads come from [Overture Maps](https://overturemaps.org)
(OpenStreetMap and other open data, ODbL). Only the tiles a chunk needs are range-read from
Overture's `buildings` and `transportation` PMTiles archives, at zoom 14. They're cached in
`1to1earth\cache\overture`, and a world keeps the Overture release it first read.

- **Buildings** stand on their real footprints at their mapped height. Where no height is mapped,
  it comes from the floor count, or a typical height for the kind of building.
  - Each has floors with lights in them, rows of windows, a foundation on slopes, and a flat roof
    with a parapet, or a pitched roof on houses.
  - Walls, windows and roofs are picked by kind (house, apartments, commercial or tall, industrial),
    or by the mapped facade and roof colours where Overture has them.
- **Roads** are painted at their class's width, from 16 m motorways to 2.5 m footpaths, with dashed
  centre lines on main roads. Paths are dirt paths and rail lines gravel beds.
- **Settings:** `buildings` and `roads` in `1to1earth.json` turn them off. They're only drawn at 4 m a
  block or finer.
- **Not yet:** bridges and tunnels, doors and interiors, building parts' own roof shapes, and street
  furniture.

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

Downloads are cached in `1to1earth\cache` in the instance folder.

## Build

```
horizon-launcher native-lock .        # only when dependencies change; keep the game's versions
cargo horizon-mod build          # packs target/horizon-mod/1to1earth.hmod
horizon-launcher native-build . --instance <instance> --trust-native-code
```

`Rebuild.bat` runs the build.

Notes:

- The lock must use the game's versions of any crate the prebuilt game also has.
- A mod can't link a second TLS stack, so downloads go through the system's `curl`.
- Only `.rs` and `.toml` files under `src/` are staged, so data tables are embedded as Rust source.
