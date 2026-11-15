//! Trust layers draped on the terrain (the project's claim on the map):
//! - key 2, slope bands from the CORRECTED slope (map slope at 3 m scale minus the measured bias):
//!   single hatching = caution band (route.json caution_deg up to the limit), cross hatching = over the route's slope limit;
//! - key 4, rover-verified ground: white dots within verified_radius_m of a localized rover position;
//! - key 8, route agreement: how many uncertainty runs walked each cell (corridor rule; continuous shading);
//! - key 0, map readiness: vertical lines where the map's 3 m slope differs from an independent HiRISE DTM (doc rule, part B v2).
//! Patterns, not colour, carry the meaning (HUD rule). Drawn by a GPU shader (overlay.wesl) on top of the photo: the
//! layer switches and the slope limit/bias are shader uniforms, so toggling or re-planning costs nothing on the CPU
//! (the earlier CPU bake of a 9.5 MB texture froze the page for about 220 ms per change, measured).
//! `band` and `hatched` are the CPU reference of the shader's patterns (the panel's profile uses `band`).
use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageSampler};
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, Extent3d, ShaderType, TextureDimension, TextureFormat};
use bevy::shader::ShaderRef;

use crate::scene::controls::{tapped, Act};
use crate::scene::terrain::Grid;

/// Texels per metre in the overlay (2: 0.5 m texels, the photo's resolution; 1106 x 2156 for the VISOR grid).
pub const TEX_PER_M: usize = 2;
/// Hatch period and line width in texels (4 m and 1 m on the ground). overlay.wesl uses the same numbers (tested).
#[cfg_attr(not(test), allow(dead_code))]
pub const HATCH_PERIOD: usize = 8;
#[cfg_attr(not(test), allow(dead_code))]
pub const HATCH_WIDTH: usize = 2;

pub const SHOW_BANDS: u32 = 1;
pub const SHOW_VERIFIED: u32 = 2;
pub const SHOW_DENSITY: u32 = 4;
pub const SHOW_SHADOW: u32 = 8;
pub const SHOW_READY: u32 = 16;
/// Plan only: contour lines every 1 m (bold every 5 m) from the 1 m DEM heights
pub const SHOW_CONTOURS: u32 = 32;
/// Plan only: the route painted on the ground, coloured by corrected slope, with a direction ramp
pub const SHOW_ROUTE: u32 = 64;
/// Lines of equal walk-back time to the exit (every 2 min, overlay.wesl), from the live plan
pub const SHOW_BACK: u32 = 128;
/// Shading from terrain shape (DEM curvature over ~3 m): hollows a little darker, crests a little lighter; both modes
pub const SHOW_SHAPE: u32 = 256;

/// Slope band of a corrected slope: 0 below caution, 1 caution, 2 over the limit.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn band(corrected_deg: f32, caution_deg: f32, limit_deg: f32) -> u8 {
    if corrected_deg > limit_deg {
        2
    } else if corrected_deg >= caution_deg {
        1
    } else {
        0
    }
}

/// Is texel (x, y) on a hatch line? Band 1: one diagonal; band 2: both diagonals (cross hatching). CPU reference of the
/// shader's pattern (tests check the shader's constants and zoomed-out means against it).
#[cfg_attr(not(test), allow(dead_code))]
pub fn hatched(x: usize, y: usize, b: u8) -> bool {
    let d1 = (x + y) % HATCH_PERIOD < HATCH_WIDTH;
    let d2 = (x + HATCH_PERIOD * 1000 - y) % HATCH_PERIOD < HATCH_WIDTH;
    match b {
        1 => d1,
        2 => d1 || d2,
        _ => false,
    }
}

/// The shadow texture (R8, one texel per grid cell, row 0 north), filled by the planner worker.
pub fn shadow_image(w: usize, h: usize) -> Image {
    shadow_image_from(w, h, vec![0; w * h])
}

/// The readiness texture (R8, one texel per grid cell, the class byte as stored in readiness.bin).
pub fn ready_image(w: usize, h: usize, classes: Vec<u8>) -> Image {
    data_image(w, h, TextureFormat::R8Unorm, classes)
}

pub fn shadow_image_from(w: usize, h: usize, mask: Vec<u8>) -> Image {
    data_image(w, h, TextureFormat::R8Unorm, mask)
}

/// The marks texture (RG8, 2 texels per metre, row 0 north): R = 255 where the texel centre lies within `radius_m` of a
/// rover position (the shader draws the dots), G = route agreement of the texel's grid cell (round(255 x share of runs)).
pub fn marks_texels(w: usize, h: usize, pts: &[(f32, f32)], radius_m: f32, usage: Option<&[u8]>) -> Vec<u8> {
    let (tw, th) = (w * TEX_PER_M, h * TEX_PER_M);
    let mut px = vec![0u8; tw * th * 2];
    let k = TEX_PER_M as f32;
    for &(e, n) in pts {
        let (x0, x1) = (((e - radius_m) * k).floor().max(0.0) as usize, (((e + radius_m) * k).ceil() as usize).min(tw));
        let (y0, y1) = (((h as f32 - n - radius_m) * k).floor().max(0.0) as usize, (((h as f32 - n + radius_m) * k).ceil() as usize).min(th));
        for y in y0..y1 {
            for x in x0..x1 {
                let (te, tn) = ((x as f32 + 0.5) / k, h as f32 - (y as f32 + 0.5) / k);
                if (te - e).powi(2) + (tn - n).powi(2) <= radius_m * radius_m {
                    px[(y * tw + x) * 2] = 255;
                }
            }
        }
    }
    if let Some(u) = usage.filter(|u| u.len() == w * h) {
        for y in 0..th {
            for x in 0..tw {
                px[(y * tw + x) * 2 + 1] = u[(y / TEX_PER_M) * w + x / TEX_PER_M];
            }
        }
    }
    px
}

/// Shader settings (overlay.wesl OverlayParams).
#[derive(ShaderType, Clone, Copy, Debug, Default, PartialEq, Reflect)]
pub struct OverlayParams {
    pub flags: u32,
    pub bias: f32,
    pub caution: f32,
    pub limit: f32,
    pub density: Vec4,
    /// selection and hover rings drawn on the ground (scene/select.rs): east, north, radius (m), on (1/0)
    pub sel: Vec4,
    pub hover: Vec4,
    /// Walk light on the photo (rgb multiplier; 1 in Plan and in full daylight): day, twilight, night (plan/daylight.rs)
    pub light: Vec4,
}

/// The terrain's material: the photo (StandardMaterial, unlit) plus the trust layers.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct Overlay {
    #[uniform(100)]
    pub params: OverlayParams,
    #[texture(101, sample_type = "float", filterable = false)]
    pub slope: Handle<Image>,
    #[texture(102, sample_type = "float", filterable = false)]
    pub marks: Handle<Image>,
    /// cast shadow at the chosen sol and hour (sunlight.rs): one byte per grid cell, 255 = shadow
    #[texture(103, sample_type = "float", filterable = false)]
    pub shadow: Handle<Image>,
    /// data readiness class per grid cell (readiness.bin): 0 unknown, 1 agrees, 2 partial, 3 disagrees, 4 rover-checked
    #[texture(104, sample_type = "float", filterable = false)]
    pub ready: Handle<Image>,
    /// highlight mask at 2 texels per metre (scene/select.rs): r = selected leg, g = watch-chip problem (out-of-sight
    /// stretches, rings round named stations)
    #[texture(105, sample_type = "float", filterable = false)]
    pub hl: Handle<Image>,
    /// DEM heights (m above the grid's z_offset), one texel per grid cell, for the contour lines
    #[texture(106, sample_type = "float", filterable = false)]
    pub height: Handle<Image>,
    /// walk-back time to the exit (s) per grid cell from the live plan (hud::WalkBack); 1e9 = no way back
    #[texture(107, sample_type = "float", filterable = false)]
    pub back: Handle<Image>,
}

pub const SHADER: &str = "embedded://visor_app/scene/overlay.wesl";

impl MaterialExtension for Overlay {
    fn fragment_shader() -> ShaderRef {
        SHADER.into()
    }
}

pub type TerrainMaterial = ExtendedMaterial<StandardMaterial, Overlay>;

/// Registers the terrain material and embeds overlay.wesl in the binary (no extra file to fetch).
pub struct OverlayPlugin;

impl Plugin for OverlayPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "overlay.wesl");
        app.add_plugins(MaterialPlugin::<TerrainMaterial>::default());
    }
}

fn data_image(w: usize, h: usize, format: TextureFormat, data: Vec<u8>) -> Image {
    let mut img = Image::new(Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 }, TextureDimension::D2, data,
                             format, RenderAssetUsages::RENDER_WORLD);
    img.sampler = ImageSampler::nearest();
    img
}

/// The slope texture (R32Float, one texel per grid cell).
pub fn slope_image(slope: &Grid) -> Image {
    data_image(slope.w, slope.h, TextureFormat::R32Float, slope.z.iter().flat_map(|v| v.to_le_bytes()).collect())
}

/// Channels per texel of the highlight mask: r selected leg, g watch-chip problem, b route, a phase along the route.
pub const MASK_CH: usize = 4;

/// Walk-back field as a texture (R32Float, one texel per grid cell, row 0 north): seconds, no way back = 1e9.
pub fn back_texels(s: &[f32]) -> Vec<u8> {
    s.iter().flat_map(|v| (if v.is_finite() { *v } else { 1.0e9 }).to_le_bytes()).collect()
}

/// Display smoothing of the walk-back field (box of 2 r + 1 cells, finite cells only; a cell with no way back stays so):
/// the raw field follows every steep cell, so its time lines broke into fragments. Lines only: the HUD and cards use the
/// exact field.
#[cfg_attr(not(test), allow(dead_code))]
pub fn smooth_back(s: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let mut tmp = Vec::new();
    let mut out = Vec::new();
    smooth_back_into(s, w, h, r, &mut tmp, &mut out);
    out
}

/// `smooth_back` into caller-owned buffers (reused across re-plans).
pub fn smooth_back_into(s: &[f32], w: usize, h: usize, r: usize, tmp: &mut Vec<(f64, u32)>, out: &mut Vec<f32>) {
    // separable box: rows then columns, carrying (sum, count) of finite cells
    tmp.clear();
    tmp.resize(w * h, (0.0f64, 0u32));
    for y in 0..h {
        for x in 0..w {
            let (mut sum, mut n) = (0.0f64, 0u32);
            for xx in x.saturating_sub(r)..(x + r + 1).min(w) {
                let v = s[y * w + xx];
                if v.is_finite() { sum += v as f64; n += 1; }
            }
            tmp[y * w + x] = (sum, n);
        }
    }
    out.clear();
    out.resize(w * h, f32::INFINITY);
    for y in 0..h {
        for x in 0..w {
            if !s[y * w + x].is_finite() {
                continue;
            }
            let (mut sum, mut n) = (0.0f64, 0u32);
            for yy in y.saturating_sub(r)..(y + r + 1).min(h) {
                let (a, b) = tmp[yy * w + x];
                sum += a; n += b;
            }
            out[y * w + x] = if n > 0 { (sum / n as f64) as f32 } else { s[y * w + x] };
        }
    }
}

/// Box radius (cells = m) of the walk-back display smoothing. VISUAL.
pub const BACK_SMOOTH_M: usize = 3;

/// Upload the live plan's walk-back field to the terrain material when the planner publishes a new one.
pub fn update_back(back: Option<Res<crate::ui::hud::WalkBack>>, ov: Option<ResMut<Overlays>>, mut mats: ResMut<Assets<TerrainMaterial>>,
                   mut images: ResMut<Assets<Image>>, mut tmp: Local<Vec<(f64, u32)>>, mut sm: Local<Vec<f32>>) {
    let (Some(back), Some(mut ov)) = (back, ov) else { return };
    if !back.is_changed() {
        return;
    }
    let Some(h) = mats.get(&ov.material).map(|m| m.extension.back.clone()) else { return };
    smooth_back_into(&back.s, back.w, back.h, BACK_SMOOTH_M, &mut tmp, &mut sm);
    // write into the texture's own buffer when it already has the right size (no new image per re-plan)
    let reused = images.get_mut(&h).is_some_and(|mut img| {
        let ok = img.texture_descriptor.size.width as usize == back.w && img.texture_descriptor.size.height as usize == back.h;
        if ok {
            let mut buf = img.data.take().unwrap_or_default();
            buf.clear();
            buf.extend(sm.iter().flat_map(|v| (if v.is_finite() { *v } else { 1.0e9 }).to_le_bytes()));
            img.data = Some(buf);
        }
        ok
    });
    if !reused {
        images.insert(&h, data_image(back.w, back.h, TextureFormat::R32Float, back_texels(&sm))).ok();
    }
    let _ = mats.get_mut(&ov.material);
    if !ov.has_back {
        ov.has_back = true;
        ov.push(&mut mats);
    }
}

/// Highlight mask (Rgba8, 2 texels per metre, all zero until something is drawn).
pub fn highlight_image(w: usize, h: usize) -> Image {
    data_image(w * TEX_PER_M, h * TEX_PER_M, TextureFormat::Rgba8Unorm, vec![0; w * TEX_PER_M * h * TEX_PER_M * MASK_CH])
}

pub fn marks_image(w: usize, h: usize, px: Vec<u8>) -> Image {
    data_image(w * TEX_PER_M, h * TEX_PER_M, TextureFormat::Rg8Unorm, px)
}

/// A 1 x 1 stand-in until the real textures exist (the material must always have both bound).
pub fn placeholder(format: TextureFormat) -> Image {
    let bytes = match format { TextureFormat::R32Float => 4, TextureFormat::R8Unorm => 1, _ => 2 };
    data_image(1, 1, format, vec![0; bytes])
}

/// Layer switches per mode ([Plan, Walk]) and the current slope rules; pushed to the material as uniforms.
#[derive(Resource)]
pub struct Overlays {
    pub material: Handle<TerrainMaterial>,
    /// [mode][bands, verified, density, shadow, readiness]. The layers are map symbols: Walk starts with them off (hatching
    /// underfoot read as dark ground); keys 2, 4, 8, 9 and 0 toggle them in the current mode.
    pub show: [[bool; 5]; 2],
    /// a shadow mask has arrived (sunlight.rs)
    pub has_shadow: bool,
    /// the bundle has a readiness layer (readiness.bin)
    pub has_ready: bool,
    /// the route layer is on (key 5): the shader paints it in Plan
    pub route_on: bool,
    /// walk-back lines (key =) and whether a walk-back field has been uploaded
    pub back_on: bool,
    pub has_back: bool,
    /// selection / hover rings (scene/select.rs)
    pub sel: Vec4,
    pub hover: Vec4,
    /// Walk light on the photo (walkscene.rs: terrain_tint); used in Walk only
    pub light: Vec4,
    pub mode: usize,
    pub bias: f32,
    pub caution: f32,
    pub limit: f32,
}

impl Overlays {
    pub fn new(material: Handle<TerrainMaterial>, bias: f32, caution: f32, limit: f32) -> Self {
        // calm by default: photo, route, stops and Curiosity's path only; the left rail turns layers on
        Overlays { material, show: [[false; 5], [false; 5]], has_shadow: false, has_ready: false, route_on: true, back_on: true, has_back: false, sel: Vec4::ZERO, hover: Vec4::ZERO, light: Vec4::ONE, mode: 0, bias, caution, limit }
    }

    pub fn params(&self) -> OverlayParams {
        let s = self.show[self.mode];
        let flags = (s[0] as u32) * SHOW_BANDS | (s[1] as u32) * SHOW_VERIFIED | (s[2] as u32) * SHOW_DENSITY
            | ((s[3] && self.has_shadow) as u32) * SHOW_SHADOW | ((s[4] && self.has_ready) as u32) * SHOW_READY
            | SHOW_CONTOURS | (self.route_on as u32) * SHOW_ROUTE | ((self.back_on && self.has_back) as u32) * SHOW_BACK | SHOW_SHAPE;
        let d = crate::ui::palette::DENSITY;
        OverlayParams { flags, bias: self.bias, caution: self.caution, limit: self.limit,
                        density: Vec4::new(d.0 as f32 / 255.0, d.1 as f32 / 255.0, d.2 as f32 / 255.0, 1.0), sel: self.sel, hover: self.hover,
                        light: if self.mode == 1 { self.light } else { Vec4::ONE } }
    }

    /// Write the uniforms (only when they changed, so the material is not re-prepared every frame).
    pub fn push(&self, mats: &mut Assets<TerrainMaterial>) {
        let p = self.params();
        if mats.get(&self.material).is_some_and(|m| m.extension.params != p) {
            if let Some(mut m) = mats.get_mut(&self.material) {
                m.extension.params = p;
            }
        }
    }
}

pub fn toggle(keys: Res<ButtonInput<KeyCode>>, mode: Res<crate::scene::walk::Mode>, layers: Option<ResMut<Overlays>>,
              mut mats: ResMut<Assets<TerrainMaterial>>) {
    let Some(mut l) = layers else { return };
    let m = if *mode == crate::scene::walk::Mode::Walk { 1 } else { 0 };
    let keys3 = [tapped(&keys, Act::LayerSlope), tapped(&keys, Act::LayerVerified), tapped(&keys, Act::LayerDensity),
                 tapped(&keys, Act::LayerShadow), tapped(&keys, Act::LayerReady)];
    let back = tapped(&keys, Act::LayerBack);
    if !(keys3.iter().any(|k| *k) || back || m != l.mode) {
        return;
    }
    if back {
        l.back_on = !l.back_on;
    }
    l.mode = m;
    for (i, k) in keys3.iter().enumerate() {
        if *k {
            l.show[m][i] = !l.show[m][i];
        }
    }
    l.push(&mut mats);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walk_back_smoothing_averages_finite_cells_and_keeps_no_way_back() {
        let s = [0.0, 60.0, 120.0, f32::INFINITY];
        let o = smooth_back(&s, 4, 1, 1);
        assert_eq!(o[1], 60.0);
        assert_eq!(o[2], 90.0, "the no-way-back neighbour is left out of the mean");
        assert!(o[3].is_infinite(), "no way back stays no way back");
    }

    #[test]
    fn walk_back_texels_keep_seconds_and_mark_no_way_back() {
        let b = back_texels(&[90.0, f32::INFINITY]);
        assert_eq!(f32::from_le_bytes([b[0], b[1], b[2], b[3]]), 90.0);
        assert_eq!(f32::from_le_bytes([b[4], b[5], b[6], b[7]]), 1.0e9);
        assert!(include_str!("overlay.wesl").contains("tb / 2.0"), "lines every 2 min, as the legend says");
    }

    #[test]
    fn bands_follow_the_corrected_slope() {
        assert_eq!(band(14.9, 15.0, 20.0), 0);
        assert_eq!(band(15.0, 15.0, 20.0), 1);
        assert_eq!(band(20.0, 15.0, 20.0), 1, "at the limit is allowed (route blocks only over it)");
        assert_eq!(band(20.1, 15.0, 20.0), 2);
    }

    #[test]
    fn cross_hatching_covers_more_than_single_and_matches_the_shader_means() {
        let cover = |b: u8| (0..64).flat_map(|y| (0..64).map(move |x| (x, y))).filter(|&(x, y)| hatched(x, y, b)).count() as f64 / 4096.0;
        assert_eq!(cover(0), 0.0);
        // overlay.wesl replaces the pattern by these means when zoomed out
        assert!((cover(1) - 0.25).abs() < 1e-9 && (cover(2) - 0.4375).abs() < 1e-9, "{} {}", cover(1), cover(2));
    }

    #[test]
    fn shader_uses_the_same_pattern_constants() {
        let src = include_str!("overlay.wesl");
        assert!(src.contains(&format!("const HATCH_PERIOD: i32 = {HATCH_PERIOD};")));
        assert!(src.contains(&format!("const HATCH_WIDTH: i32 = {HATCH_WIDTH};")));
        assert!(src.contains(&format!("const TEX_PER_M: i32 = {TEX_PER_M};")));
    }

    #[test]
    fn verified_marks_stay_inside_the_radius_and_agreement_is_per_cell() {
        let usage: Vec<u8> = (0..400).map(|i| (i % 256) as u8).collect();
        let px = marks_texels(20, 20, &[(10.0, 10.0)], 3.0, Some(&usage));
        let (tw, k) = (20 * TEX_PER_M, TEX_PER_M as f32);
        let mut any = false;
        for (i, p) in px.chunks_exact(2).enumerate() {
            let (x, y) = (i % tw, i / tw);
            if p[0] > 0 {
                any = true;
                let (e, n) = ((x as f32 + 0.5) / k, 20.0 - (y as f32 + 0.5) / k);
                assert!((e - 10.0).powi(2) + (n - 10.0).powi(2) <= 9.0 + 1e-3, "outside the radius at {e},{n}");
            }
            assert_eq!(p[1], usage[(y / TEX_PER_M) * 20 + x / TEX_PER_M]);
        }
        assert!(any);
        assert!(marks_texels(20, 20, &[], 3.0, Some(&usage[..10])).chunks_exact(2).all(|p| p[1] == 0), "wrong size: no agreement");
    }

    #[test]
    fn switches_set_the_shader_flags() {
        let mut o = Overlays::new(Handle::default(), 3.4, 15.0, 20.0);
        assert_eq!(o.params().flags, SHOW_CONTOURS | SHOW_ROUTE | SHOW_SHAPE, "calm by default: photo, contours, route, shape shading");
        o.show[0] = [true; 5];
        assert_eq!(o.params().flags, SHOW_BANDS | SHOW_VERIFIED | SHOW_DENSITY | SHOW_CONTOURS | SHOW_ROUTE | SHOW_SHAPE, "no shadow flag before a mask arrives");
        o.has_shadow = true;
        assert_eq!(o.params().flags, SHOW_BANDS | SHOW_VERIFIED | SHOW_DENSITY | SHOW_SHADOW | SHOW_CONTOURS | SHOW_ROUTE | SHOW_SHAPE, "no readiness flag without the layer");
        o.has_ready = true;
        assert_eq!(o.params().flags, SHOW_BANDS | SHOW_VERIFIED | SHOW_DENSITY | SHOW_SHADOW | SHOW_READY | SHOW_CONTOURS | SHOW_ROUTE | SHOW_SHAPE);
        o.mode = 1;
        let ground = SHOW_CONTOURS | SHOW_ROUTE | SHOW_SHAPE;
        assert_eq!(o.params().flags, ground, "Walk: map symbols off, ground markings on");
        o.show[1][1] = true;
        assert_eq!(o.params().flags, SHOW_VERIFIED | ground);
        assert_eq!(o.params().bias, 3.4);
    }
}
