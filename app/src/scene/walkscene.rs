//! Walk-mode scene: the terrain is lit by the Mars24 sun for the chosen sol and start hour (sunlight.rs), with real-time
//! shadows (one cascade: the WebGL2 limit), an ILLUSTRATIVE sky dome (sky.wesl) and haze in the horizon colour.
//! Plan mode keeps the unlit photo (the HiRISE photo carries its own lighting).
//! Lighting is an approximation, labelled in the HUD: the photo already contains the shading of its own sun (31 deg high,
//! from 303 deg), so lit slopes are partly shaded twice; the light is kept soft (strong ambient) to limit that. Normals come
//! from the 3 m smoothed heights (terrain.rs) so the 1 m DEM noise (gate FAIL) does not show as pitting.
use bevy::light::{CascadeShadowConfigBuilder, NotShadowCaster};
use bevy::pbr::MaterialPlugin;
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

use crate::scene::trust::{Overlays, TerrainMaterial};
use crate::plan::daylight::Daylight;
use crate::scene::walk::Mode;

/// Camera exposure factor for Bevy's default exposure (EV100 9.7): 1 / (1.2 x 2^9.7).
const EXPOSURE: f32 = 1.0 / (1.2 * 831.75);
/// Lit terrain = albedo x (AMBIENT + DIRECT x cos(sun angle)): soft, so the photo's own shading is not doubled harshly.
/// VISUAL choices, not measured Martian light levels.
pub const AMBIENT: f32 = 0.55;
pub const DIRECT: f32 = 0.6;
/// Shadow range around the walker (m); one cascade on WebGL2.
pub const SHADOW_RANGE_M: f32 = 160.0;
/// Sky dome radius (m): inside the camera's far plane, outside the terrain crop seen from the walk.
pub const SKY_RADIUS_M: f32 = 900.0;
/// Sun used before Mars24 has answered (deg): only for the first frames.
const FALLBACK_SUN: (f64, f64) = (35.0, 90.0);

/// ILLUSTRATIVE sky colours (sRGB): butterscotch zenith, paler horizon, bluish glow near the sun (day; JPL: the sky
/// "would look more of a butterscotch color to the human eye"), and a blue-grey twilight (Curiosity's blue sunsets).
/// Brightness scales with plan::daylight (twilight up to 2 h, then night).
pub const ZENITH: (f32, f32, f32) = (0.56, 0.43, 0.34);
pub const HORIZON: (f32, f32, f32) = (0.80, 0.65, 0.52);
pub const GLOW: (f32, f32, f32) = (0.80, 0.84, 0.88);
pub const TWI_ZENITH: (f32, f32, f32) = (0.22, 0.27, 0.36);
pub const TWI_HORIZON: (f32, f32, f32) = (0.50, 0.55, 0.62);
/// Suit fill light (share), scaled with the sky but kept so the crew stays visible at night. VISUAL.
const SUIT_FILL: f32 = 0.12;
const SUIT_FILL_NIGHT: f32 = 0.06;

#[derive(ShaderType, Clone, Copy, Debug, Default, PartialEq, Reflect)]
pub struct SkyParams {
    pub sun: Vec4,
    pub zenith: Vec4,
    pub horizon: Vec4,
    pub glow: Vec4,
    /// x: star visibility 0..1 (illustrative)
    pub night: Vec4,
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct SkyMaterial {
    #[uniform(0)]
    pub params: SkyParams,
}

pub const SKY_SHADER: &str = "embedded://visor_app/scene/sky.wesl";

impl Material for SkyMaterial {
    fn fragment_shader() -> ShaderRef {
        SKY_SHADER.into()
    }
}

pub struct WalkScenePlugin;

impl Plugin for WalkScenePlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "sky.wesl");
        app.add_plugins(MaterialPlugin::<SkyMaterial>::default());
    }
}

#[derive(Component)]
pub struct SunLightTag;

#[derive(Component)]
pub struct SkyDome;

fn lin(c: (f32, f32, f32)) -> Vec4 {
    let l = Color::srgb(c.0, c.1, c.2).to_linear();
    Vec4::new(l.red, l.green, l.blue, 1.0)
}

/// Unit vector toward the sun in world axes (x east, y up, z = -north) from elevation and compass azimuth (degrees).
pub fn sun_dir(el_deg: f64, az_deg: f64) -> Vec3 {
    let (el, az) = (el_deg.to_radians() as f32, az_deg.to_radians() as f32);
    Vec3::new(az.sin() * el.cos(), el.sin(), -az.cos() * el.cos())
}

fn mix3(a: (f32, f32, f32), b: (f32, f32, f32), k: f32) -> (f32, f32, f32) {
    (a.0 + (b.0 - a.0) * k, a.1 + (b.1 - a.1) * k, a.2 + (b.2 - a.2) * k)
}

fn scale3(a: (f32, f32, f32), k: f32) -> (f32, f32, f32) {
    (a.0 * k, a.1 * k, a.2 * k)
}

/// Sky colours (sRGB) for a light state: day colours moved toward the twilight blue, then dimmed with the sky.
pub fn sky_colors(d: &Daylight) -> [(f32, f32, f32); 3] {
    let k = d.sky;
    [scale3(mix3(ZENITH, TWI_ZENITH, d.blue), k), scale3(mix3(HORIZON, TWI_HORIZON, d.blue), k),
     scale3(GLOW, (k * 1.3).min(1.0))]
}

/// Sky uniforms for a sun direction and light state.
pub fn sky_params_at(dir: Vec3, d: &Daylight) -> SkyParams {
    let [z, h, g] = sky_colors(d);
    SkyParams { sun: dir.extend(dir.y.asin()), zenith: lin(z), horizon: lin(h), glow: lin(g), night: Vec4::new(d.stars, 0.0, 0.0, 0.0) }
}

/// Sky uniforms for a sun direction in full daylight.
pub fn sky_params(dir: Vec3) -> SkyParams {
    sky_params_at(dir, &Daylight::DAY)
}

/// Multiplier on the terrain photo in Walk (the terrain shader draws the photo unlit, so the sun light alone cannot dim
/// it): the sky brightness, tinted toward blue as the sky turns to twilight. VISUAL.
pub fn terrain_tint(d: &Daylight) -> Vec4 {
    let b = 0.45 * d.blue;
    Vec4::new(d.sky * (1.0 - 0.25 * b), d.sky * (1.0 - 0.15 * b), d.sky, 1.0)
}

/// The haze colour in Walk mode (the sky's horizon) for a light state.
pub fn horizon_color_at(d: &Daylight) -> Color {
    let h = sky_colors(d)[1];
    Color::srgb(h.0, h.1, h.2)
}

/// The haze colour in Walk mode in full daylight.
pub fn horizon_color() -> Color {
    horizon_color_at(&Daylight::DAY)
}

/// An inside-out sphere (faces point inward) for the sky dome.
fn dome_mesh(r: f32) -> Mesh {
    let mut m = Sphere::new(r).mesh().uv(48, 24);
    if let Some(bevy::mesh::Indices::U32(idx)) = m.indices_mut() {
        for t in idx.chunks_exact_mut(3) {
            t.swap(1, 2);
        }
    }
    m
}

/// Spawn the sun light (off until Walk) and the sky dome (hidden until Walk).
pub fn spawn(commands: &mut Commands, meshes: &mut Assets<Mesh>, skies: &mut Assets<SkyMaterial>) {
    commands.spawn((
        DirectionalLight { illuminance: 0.0, shadow_maps_enabled: false, ..default() },
        CascadeShadowConfigBuilder { num_cascades: 1, minimum_distance: 0.5, maximum_distance: SHADOW_RANGE_M,
                                     first_cascade_far_bound: SHADOW_RANGE_M, overlap_proportion: 0.2 }.build(),
        Transform::from_translation(Vec3::ZERO).looking_to(-sun_dir(FALLBACK_SUN.0, FALLBACK_SUN.1), Vec3::Y),
        SunLightTag,
        Name::new("sun"),
    ));
    let mat = skies.add(SkyMaterial { params: sky_params(sun_dir(FALLBACK_SUN.0, FALLBACK_SUN.1)) });
    commands.spawn((Mesh3d(meshes.add(dome_mesh(SKY_RADIUS_M))), MeshMaterial3d(mat), NotShadowCaster, Visibility::Hidden, SkyDome,
                    Name::new("sky dome")));
}

/// Switch lighting, sky and haze colour with the mode; in Walk, keep the dome on the camera and the sun where Mars24 puts it.
#[allow(clippy::too_many_arguments)]
pub fn update(
    mode: Res<Mode>,
    sunlight: Option<Res<crate::plan::sunlight::SunLight>>,
    mut overlays: Option<ResMut<Overlays>>,
    mut terrain_mats: ResMut<Assets<TerrainMaterial>>,
    mut skies: ResMut<Assets<SkyMaterial>>,
    mut suits: ResMut<Assets<crate::scene::armat::SuitMaterial>>,
    mut ambient: ResMut<GlobalAmbientLight>,
    mut lights: Query<(&mut DirectionalLight, &mut Transform), (With<SunLightTag>, Without<SkyDome>, Without<Camera3d>)>,
    mut domes: Query<(&mut Transform, &mut Visibility, &MeshMaterial3d<SkyMaterial>), (With<SkyDome>, Without<Camera3d>)>,
    cams: Query<&Transform, (With<Camera3d>, Without<SkyDome>, Without<SunLightTag>)>,
    mut fogs: Query<&mut DistanceFog>,
    mut last_sun: Local<Option<(f64, f64)>>,
    warm: Res<crate::scene::walk::WalkWarm>,
    warm_cams: Query<&Transform, (With<crate::WarmUp>, Without<SkyDome>, Without<SunLightTag>)>,
    sun_now: Option<Res<crate::plan::simrun::SunNow>>,
    mut plan_fog_color: Local<Option<Color>>,
) {
    let walk = *mode == Mode::Walk || warm.0;
    if mode.is_changed() || warm.is_changed() {
        if let Some(o) = overlays.as_ref() {
            if let Some(mut m) = terrain_mats.get_mut(&o.material) {
                m.base.unlit = !walk;
                m.base.perceptual_roughness = 1.0;
                m.base.reflectance = 0.0;
            }
        }
        ambient.brightness = if walk { AMBIENT / EXPOSURE } else { 80.0 };
        for (mut l, _) in &mut lights {
            l.illuminance = if walk { DIRECT * std::f32::consts::PI / EXPOSURE } else { 0.0 };
            l.shadow_maps_enabled = walk;
        }
        for (_, mut v, _) in &mut domes {
            *v = if walk { Visibility::Visible } else { Visibility::Hidden };
        }
        for mut f in &mut fogs {
            if plan_fog_color.is_none() {
                *plan_fog_color = Some(f.color);
            }
            f.color = if walk { horizon_color() } else { plan_fog_color.unwrap_or(f.color) };
        }
    }
    if !walk {
        return;
    }
    // the simulation clock's sun (start hour + elapsed), else the chosen start hour's sun
    let now = sun_now.as_deref().copied().unwrap_or_default();
    let sun = now.0
        .or_else(|| sunlight.as_ref().and_then(|s| s.sun.map(|(el, az, _, _)| (el, az))))
        .unwrap_or(FALLBACK_SUN);
    // day / twilight / night from the simulation clock; without a running plan, the start hour's sun (daylight if up)
    let day = now.1.unwrap_or_else(|| if sun.0 > 0.0 { crate::plan::daylight::light(sun.0, 0.0) } else { crate::plan::daylight::light(sun.0, f64::INFINITY) });
    let dir = sun_dir(sun.0, sun.1);
    let moved = last_sun.is_none_or(|l| (l.0 - sun.0).abs() > 0.05 || (l.1 - sun.1).abs() > 0.05) || mode.is_changed() || warm.is_changed();
    if moved {
        *last_sun = Some(sun);
        // the sun's light comes from above the horizon only; ambient and haze dim with the sky
        let up = if dir.y > 0.0 { dir } else { Vec3::new(dir.x, 0.05, dir.z).normalize_or(Vec3::Y) };
        for (mut l, mut t) in &mut lights {
            *t = Transform::from_translation(Vec3::ZERO).looking_to(-up, Vec3::Y);
            l.illuminance = DIRECT * day.direct * std::f32::consts::PI / EXPOSURE;
            l.shadow_maps_enabled = day.direct > 0.0;
        }
        ambient.brightness = AMBIENT * day.sky / EXPOSURE;
        if let Some(o) = overlays.as_mut() {
            let tint = terrain_tint(&day);
            if o.light != tint {
                o.light = tint;
                o.push(&mut terrain_mats);
            }
        }
        for mut f in &mut fogs {
            f.color = horizon_color_at(&day);
        }
        for (_, _, m) in &domes {
            if let Some(mut s) = skies.get_mut(&m.0) {
                s.params = sky_params_at(dir, &day);
            }
        }
        if let Some(mut s) = crate::scene::armat::SUIT_MAT.get().and_then(|h| suits.get_mut(h)) {
            s.params.sun = up.extend(0.0);
            s.params.light = Vec4::new(AMBIENT * day.sky, DIRECT * day.direct, (SUIT_FILL * day.sky).max(SUIT_FILL_NIGHT), 0.0);
        }
    }
    // the dome sits on the view that draws it (the warm-up camera while loading)
    if let Some(cam) = warm_cams.iter().next().or_else(|| cams.iter().next()) {
        for (mut t, _, _) in &mut domes {
            t.translation = cam.translation;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sun_direction_matches_compass_and_elevation() {
        let east = sun_dir(0.0, 90.0);
        assert!((east - Vec3::X).length() < 1e-5, "{east:?}");
        let north = sun_dir(0.0, 0.0);
        assert!((north - Vec3::NEG_Z).length() < 1e-5, "north is -z: {north:?}");
        let up = sun_dir(90.0, 123.0);
        assert!((up.y - 1.0).abs() < 1e-5);
        let p = sky_params(sun_dir(30.0, 200.0));
        assert!((p.sun.w - 30f32.to_radians()).abs() < 1e-5, "w carries the elevation in radians");
    }

    #[test]
    fn lighting_constants_give_the_intended_soft_balance() {
        // flat ground under a high sun: ambient + direct = about the photo's own brightness, never far above it
        let total = AMBIENT + DIRECT;
        assert!(total > 0.9 && total < 1.3, "{total}");
        assert!(AMBIENT >= DIRECT * 0.8, "ambient must stay strong (the photo already holds shading)");
    }

    #[test]
    fn dome_faces_inward() {
        let m = dome_mesh(10.0);
        let pos = match m.attribute(Mesh::ATTRIBUTE_POSITION).unwrap() {
            bevy::mesh::VertexAttributeValues::Float32x3(v) => v.clone(),
            _ => panic!(),
        };
        let Some(bevy::mesh::Indices::U32(idx)) = m.indices() else { panic!() };
        // average over triangles: the face normal should point toward the centre
        let mut inward = 0;
        let mut n = 0;
        for t in idx.chunks_exact(3) {
            let (a, b, c) = (Vec3::from(pos[t[0] as usize]), Vec3::from(pos[t[1] as usize]), Vec3::from(pos[t[2] as usize]));
            let nrm = (b - a).cross(c - a);
            if nrm.length() < 1e-6 {
                continue;
            }
            n += 1;
            if nrm.dot((a + b + c) / 3.0) < 0.0 {
                inward += 1;
            }
        }
        assert!(inward * 10 >= n * 9, "{inward} of {n} faces inward");
    }
}
