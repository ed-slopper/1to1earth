//! The Earth map: a "Map" button on the pause screen opens a screen of the mod's own with a map
//! of the whole planet (Web Mercator, shaded relief from the elevation tiles the terrain reads).
//! Drag to move it, scroll to zoom, and click a spot to go there (`/earth tp <lat> <lon>`).
//!
//! The screen hooks only read the client, so they leave what to do in `STATE`, and the hooks that
//! can act (`client_tick_post`, `mouse_button_post`) open the screen, upload the picture and send
//! the command. The picture is drawn on a worker thread from cached or downloaded tiles.
use crate::{
    earth,
    elevation::{self, Dem},
};
use mod_api::{ClientHost, ScreenView};
use std::{
    f64::consts::PI,
    sync::Mutex,
};

pub const MOD_ID: &str = "1to1earth";
/// The image slot every client mod shares (Horizon's `MOD_IMAGE_SPRITE`, 1024 x 1024).
const SPRITE: &str = "horizon/modimage";
const SLOT: u32 = 1024;
/// The pause screen's button, from the screen's top left, in GUI pixels.
const BUTTON: [f32; 4] = [6.0, 6.0, 74.0, 20.0];
/// The map's margins inside its screen.
const MARGIN: [f32; 4] = [10.0, 26.0, 10.0, 22.0];
/// Map zoom: the world is 256 * 2^zoom map pixels wide.
const MIN_ZOOM: f64 = 0.0;
const MAX_ZOOM: f64 = 16.0;

/// The part of the world a picture covers: its centre (Web Mercator 0..1), zoom and size in texels.
#[derive(Clone, Copy, PartialEq, Debug)]
struct View {
    u: f64,
    v: f64,
    zoom: f64,
    w: u32,
    h: u32,
}

impl View {
    fn world(&self) -> f64 {
        256.0 * 2f64.powf(self.zoom)
    }
}

struct State {
    /// The map's view now.
    view: View,
    /// The pause button was pressed: open the map.
    open: bool,
    /// A click on the map: go there.
    go: Option<(f64, f64)>,
    pressed: Option<[f32; 2]>,
    dragged: bool,
    /// The picture uploaded to the slot, and the view it shows.
    shown: Option<View>,
    /// Whether the shown picture has the land cover's lakes and rivers in it.
    shown_water: bool,
    /// A finished picture waiting for upload, and whether it has its water.
    ready: Option<(View, bool, Vec<u8>)>,
    /// A picture is being drawn for this view.
    drawing: Option<View>,
    /// The map rect on screen, in GUI pixels, as last drawn.
    rect: [f32; 4],
    /// Centred on the player the first time the map opens.
    centred: bool,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap();
    let s = g.get_or_insert_with(|| State {
        view: View { u: 0.5, v: 0.5, zoom: 1.0, w: 512, h: 288 },
        open: false,
        go: None,
        pressed: None,
        dragged: false,
        shown: None,
        shown_water: false,
        ready: None,
        drawing: None,
        rect: [0.0; 4],
        centred: false,
    });
    f(s)
}

fn to_latlon(u: f64, v: f64) -> (f64, f64) {
    let lon = (u - 0.5) * 360.0;
    let lat = (2.0 * ((0.5 - v) * 2.0 * PI).exp().atan() - PI / 2.0).to_degrees();
    (lat, lon)
}

fn to_uv(lat: f64, lon: f64) -> (f64, f64) {
    let lat = lat.clamp(-85.05, 85.05).to_radians();
    (lon / 360.0 + 0.5, 0.5 - (PI / 4.0 + lat / 2.0).tan().ln() / (2.0 * PI))
}

fn is_map(host: &dyn ClientHost, screen: &ScreenView<'_>) -> bool {
    screen.id == "mod" && host.mod_screen() == Some(MOD_ID)
}

fn inside(r: [f32; 4], p: [f32; 2]) -> bool {
    p[0] >= r[0] && p[1] >= r[1] && p[0] < r[0] + r[2] && p[1] < r[1] + r[3]
}

/// The map rect for a screen.
fn map_rect(screen: &ScreenView<'_>) -> [f32; 4] {
    let [x, y, w, h] = screen.rect;
    [x + MARGIN[0], y + MARGIN[1], (w - MARGIN[0] - MARGIN[2]).max(16.0), (h - MARGIN[1] - MARGIN[3]).max(16.0)]
}

/// The Web Mercator point under GUI point `p` of the map.
fn uv_at(view: &View, rect: [f32; 4], p: [f32; 2]) -> (f64, f64) {
    // GUI pixels to map pixels: the picture spans the rect, `view.w` texels across.
    let scale = view.w as f64 / rect[2] as f64;
    let dx = (p[0] - rect[0] - rect[2] / 2.0) as f64 * scale;
    let dy = (p[1] - rect[1] - rect[3] / 2.0) as f64 * scale;
    (view.u + dx / view.world(), (view.v + dy / view.world()).clamp(0.0, 1.0))
}

// --- the pause screen's button -------------------------------------------------------------

pub fn draw(host: &dyn ClientHost, draw: &mut dyn mod_api::DrawContext, screen: &ScreenView<'_>) {
    if screen.id == "paused" {
        let r = [screen.rect[0] + BUTTON[0], screen.rect[1] + BUTTON[1], BUTTON[2], BUTTON[3]];
        let hot = inside(r, screen.mouse);
        draw.fill(r, if hot { 0xC0_3A6E_A5 } else { 0xA0_0000_00 });
        draw.fill([r[0], r[1] + r[3] - 1.0, r[2], 1.0], 0xFF_FFFF_FF);
        let label = "Earth Map";
        let tw = draw.text_width(label);
        draw.text(label, [r[0] + (r[2] - tw) / 2.0, r[1] + 6.0], 0xFFFF_FFFF, true);
        return;
    }
    if !is_map(host, screen) {
        return;
    }
    let rect = map_rect(screen);
    let (view, shown) = with(|s| {
        s.rect = rect;
        // The picture's size follows the rect's shape, at most the slot.
        let aspect = rect[3] / rect[2];
        let w = (rect[2] * 1.0).clamp(64.0, SLOT as f32) as u32;
        let h = ((w as f32) * aspect).clamp(32.0, SLOT as f32) as u32;
        s.view.w = w;
        s.view.h = h;
        (s.view, s.shown)
    });
    draw.fill(screen.rect, 0xE0_0B10_18);
    draw.fill([rect[0] - 1.0, rect[1] - 1.0, rect[2] + 2.0, rect[3] + 2.0], 0xFF_6080_A0);
    draw.fill(rect, 0xFF_1B3A5C);
    if let Some(p) = shown {
        // The picture where it falls in the current view (moved or zoomed since it was drawn).
        let scale_now = view.w as f64 / rect[2] as f64; // map px per GUI px now
        let k = view.world() / p.world(); // current map px per picture map px
        let gw = p.w as f64 * k / scale_now;
        let gh = p.h as f64 * k / scale_now;
        let cx = rect[0] as f64 + rect[2] as f64 / 2.0 + (p.u - view.u) * view.world() / scale_now;
        let cy = rect[1] as f64 + rect[3] as f64 / 2.0 + (p.v - view.v) * view.world() / scale_now;
        if draw.push_clip(rect).is_ok() {
            draw.sprite(
                SPRITE,
                [0.0, 0.0, p.w as f32, p.h as f32],
                [(cx - gw / 2.0) as f32, (cy - gh / 2.0) as f32, gw as f32, gh as f32],
                None,
            );
            // Where the player is.
            if let Some((plat, plon)) = player_latlon(host) {
                let (pu, pv) = to_uv(plat, plon);
                let mut dx = (pu - view.u) * view.world();
                if dx > view.world() / 2.0 {
                    dx -= view.world();
                }
                let x = rect[0] as f64 + rect[2] as f64 / 2.0 + dx / scale_now;
                let y = rect[1] as f64 + rect[3] as f64 / 2.0 + (pv - view.v) * view.world() / scale_now;
                draw.fill([x as f32 - 3.0, y as f32 - 3.0, 6.0, 6.0], 0xFF_FFFF_FF);
                draw.fill([x as f32 - 2.0, y as f32 - 2.0, 4.0, 4.0], 0xFF_E0_2020);
            }
            let _ = draw.pop_clip();
        }
    } else {
        draw.text("Loading the map...", [rect[0] + 8.0, rect[1] + 8.0], 0xFFFF_FFFF, true);
    }
    let title = "Earth Map: click to go there, drag to move, scroll to zoom";
    draw.text(title, [screen.rect[0] + 10.0, screen.rect[1] + 9.0], 0xFFFF_FFFF, true);
    let line = if inside(rect, screen.mouse) {
        let (u, v) = uv_at(&view, rect, screen.mouse);
        let (lat, lon) = to_latlon(u.rem_euclid(1.0), v);
        format!("/earth tp {lat:.6} {lon:.6}")
    } else {
        String::new()
    };
    draw.text(&line, [rect[0], rect[1] + rect[3] + 7.0], 0xFF_C8D8F0, true);
}

fn player_latlon(host: &dyn ClientHost) -> Option<(f64, f64)> {
    let player = host.local_player()?;
    let [x, _, z] = player.entity().position(host)?;
    Some(earth::to_globe(x, z))
}

// --- input -----------------------------------------------------------------------------------

/// A press on a screen: the pause button, or the start of a click or drag on the map. True when
/// it was ours.
pub fn pressed(host: &dyn ClientHost, screen: &ScreenView<'_>, button: i32) -> bool {
    if screen.id == "paused" {
        let r = [screen.rect[0] + BUTTON[0], screen.rect[1] + BUTTON[1], BUTTON[2], BUTTON[3]];
        if button == 0 && inside(r, screen.mouse) {
            with(|s| s.open = true);
            return true;
        }
        return false;
    }
    if !is_map(host, screen) {
        return false;
    }
    with(|s| {
        if button == 0 && inside(s.rect, screen.mouse) {
            s.pressed = Some(screen.mouse);
            s.dragged = false;
        }
    });
    true
}

pub fn dragged(host: &dyn ClientHost, screen: &ScreenView<'_>, delta: [f32; 2]) -> bool {
    if !is_map(host, screen) {
        return false;
    }
    with(|s| {
        if s.pressed.is_none() {
            return;
        }
        if let Some(p) = s.pressed {
            if (screen.mouse[0] - p[0]).abs() + (screen.mouse[1] - p[1]).abs() > 3.0 {
                s.dragged = true;
            }
        }
        let scale = s.view.w as f64 / s.rect[2].max(1.0) as f64;
        s.view.u -= delta[0] as f64 * scale / s.view.world();
        s.view.v = (s.view.v - delta[1] as f64 * scale / s.view.world()).clamp(0.0, 1.0);
        s.view.u = s.view.u.rem_euclid(1.0);
    });
    true
}

pub fn released(host: &dyn ClientHost, screen: &ScreenView<'_>, button: i32) -> bool {
    if !is_map(host, screen) {
        return false;
    }
    with(|s| {
        if button == 0 && s.pressed.take().is_some() && !s.dragged && inside(s.rect, screen.mouse) {
            let (u, v) = uv_at(&s.view, s.rect, screen.mouse);
            s.go = Some(to_latlon(u.rem_euclid(1.0), v));
        }
        s.dragged = false;
    });
    true
}

pub fn scrolled(host: &dyn ClientHost, screen: &ScreenView<'_>, scroll: f64) -> bool {
    if !is_map(host, screen) {
        return false;
    }
    with(|s| {
        // Zoom about the point under the pointer: it stays put.
        let before = uv_at(&s.view, s.rect, screen.mouse);
        s.view.zoom = (s.view.zoom + scroll.signum() * 0.5).clamp(MIN_ZOOM, MAX_ZOOM);
        let after = uv_at(&s.view, s.rect, screen.mouse);
        s.view.u = (s.view.u + before.0 - after.0).rem_euclid(1.0);
        s.view.v = (s.view.v + before.1 - after.1).clamp(0.0, 1.0);
    });
    true
}

// --- acting: opening, teleporting, the picture --------------------------------------------------

/// From the hooks that can act on the client: open the map, go where it was clicked, start and
/// upload pictures.
pub fn act(host: &mut dyn ClientHost) {
    let (open, go) = with(|s| (std::mem::take(&mut s.open), s.go.take()));
    if open {
        host.close_screen();
        if host.mod_screen_open(MOD_ID) {
            let here = player_latlon(&*host);
            with(|s| {
                if !s.centred {
                    if let Some((lat, lon)) = here {
                        let (u, v) = to_uv(lat, lon);
                        s.view.u = u;
                        s.view.v = v;
                        s.view.zoom = 9.0;
                    }
                    s.centred = true;
                }
            });
        }
    }
    if let Some((lat, lon)) = go {
        host.mod_screen_close(MOD_ID);
        if let Err(e) = host.send_chat(&format!("/earth tp {lat:.6} {lon:.6}")) {
            eprintln!("[1to1earth] map teleport: {e}");
        }
    }
    if host.mod_screen() != Some(MOD_ID) {
        return;
    }
    // Upload a finished picture; start the next when the view moved. A view is drawn twice when
    // close enough for lakes: relief first (quick), then again with the land cover's water.
    let ready = with(|s| s.ready.take());
    if let Some((view, water, rgba)) = ready {
        if host.mod_image(view.w, view.h, &rgba) {
            with(|s| {
                s.shown = Some(view);
                s.shown_water = water;
            });
        }
    }
    let start = with(|s| {
        let want = s.view;
        if s.drawing.is_some() || s.ready.is_some() {
            return None;
        }
        let water = if s.shown != Some(want) {
            false
        } else if !s.shown_water && water_view(&want) {
            true
        } else {
            return None;
        };
        s.drawing = Some(want);
        Some((want, water))
    });
    if let Some((view, water)) = start {
        std::thread::spawn(move || {
            let rgba = picture(view, water);
            with(|s| {
                s.drawing = None;
                // A relief picture always shows (it is moved into place while dragging); a lakes
                // pass for a view already left is dropped.
                if !water || s.view == view {
                    s.ready = Some((view, water, rgba));
                }
            });
        });
    }
}

/// Texel size in metres at a view's middle.
fn texel_metres(view: &View) -> f64 {
    let (lat, _) = to_latlon(view.u, view.v);
    40_075_016.0 * lat.to_radians().cos() / view.world()
}

/// Close enough to draw lakes from the land cover (texels under about 700 m): further out a view
/// spans hundreds of the land cover's files.
fn water_view(view: &View) -> bool {
    texel_metres(view) < 700.0
}

/// The picture of `view`: shaded relief, sea in blues by depth.
fn picture(view: View, water: bool) -> Vec<u8> {
    let (w, h) = (view.w as i64, view.h as i64);
    let world = view.world();
    // The tile zoom whose pixels are about one picture texel (512-pixel tiles).
    let tz = (world / 512.0).log2().round().clamp(0.0, 12.0) as u8;
    let total = (512i64) << tz;
    let u0 = view.u - w as f64 / 2.0 / world;
    let v0 = view.v - h as f64 / 2.0 / world;
    let at = |i: i64, j: i64| -> Option<f32> {
        let v = v0 + (j as f64 + 0.5) / world;
        if !(0.0..1.0).contains(&v) {
            return None;
        }
        let u = (u0 + (i as f64 + 0.5) / world).rem_euclid(1.0);
        elevation::pixel(Dem::Land, tz, (u * total as f64) as i64, (v * total as f64) as i64)
    };
    let mut grid = vec![f32::NAN; ((w + 2) * (h + 2)) as usize];
    for j in -1..=h {
        for i in -1..=w {
            if let Some(e) = at(i, j) {
                grid[((j + 1) * (w + 2) + i + 1) as usize] = e;
            }
        }
    }
    let g = |i: i64, j: i64| grid[((j + 1) * (w + 2) + i + 1) as usize];
    // Metres a texel, at the view's middle latitude, for the shading's slope.
    let texel_m = texel_metres(&view);
    // The land cover overview whose pixels are about a texel (10 m at overview 0).
    let overview = ((texel_m / 10.0).log2().floor().max(0.0) as u32).min(crate::worldcover::MAX_OVERVIEW);
    let u0w = view.u - w as f64 / 2.0 / world;
    let v0w = view.v - h as f64 / 2.0 / world;
    let wet = |i: i64, j: i64| -> bool {
        if !water {
            return false;
        }
        let (lat, lon) = to_latlon((u0w + (i as f64 + 0.5) / world).rem_euclid(1.0), v0w + (j as f64 + 0.5) / world);
        crate::worldcover::class(lat, lon, overview) == crate::worldcover::WATER
    };
    let mut out = vec![0u8; (w * h * 4) as usize];
    for j in 0..h {
        for i in 0..w {
            let e = g(i, j);
            let o = ((j * w + i) * 4) as usize;
            let rgb = if e.is_nan() {
                [27, 58, 92]
            } else if e > 0.0 && wet(i, j) {
                // A lake or river (the land cover's water above the sea).
                [64, 122, 196]
            } else if e <= 0.0 {
                let d = (-e / 6000.0).clamp(0.0, 1.0);
                [
                    (70.0 - 45.0 * d) as u8,
                    (130.0 - 80.0 * d) as u8,
                    (190.0 - 80.0 * d) as u8,
                ]
            } else {
                let (ex, ey) = (g(i + 1, j) - g(i - 1, j), g(i, j + 1) - g(i, j - 1));
                let (ex, ey) = (if ex.is_nan() { 0.0 } else { ex }, if ey.is_nan() { 0.0 } else { ey });
                // Lit from the north west.
                // Slope toward the light (north west) over two texels, exaggerated a little.
                let slope = -((ex + ey) as f64) / (2.0 * texel_m.max(1.0));
                let shade = (1.0 + 0.8 * slope).clamp(0.55, 1.35);
                let base = hypsometric(e);
                [
                    (base[0] as f64 * shade).min(255.0) as u8,
                    (base[1] as f64 * shade).min(255.0) as u8,
                    (base[2] as f64 * shade).min(255.0) as u8,
                ]
            };
            out[o..o + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        }
    }
    out
}

/// Land colour by height: lowland green to upland tan, rock grey, snow white.
fn hypsometric(e: f32) -> [u8; 3] {
    const STOPS: [(f32, [u8; 3]); 6] = [
        (0.0, [92, 140, 78]),
        (300.0, [130, 160, 90]),
        (1000.0, [190, 175, 120]),
        (2000.0, [160, 130, 100]),
        (3500.0, [150, 145, 140]),
        (5000.0, [245, 245, 250]),
    ];
    for win in STOPS.windows(2) {
        let ((a, ca), (b, cb)) = (win[0], win[1]);
        if e <= b {
            let t = ((e - a) / (b - a)).clamp(0.0, 1.0);
            return [0, 1, 2].map(|c| (ca[c] as f32 + (cb[c] as f32 - ca[c] as f32) * t) as u8);
        }
    }
    STOPS[STOPS.len() - 1].1
}
