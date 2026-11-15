//! One light material for every Walk-mode AR overlay (armat.wesl): vertex colour x tint, alpha blended, unlit, no shadows.
//! Why: each new combination of StandardMaterial features (vertex colours, blending, additive) compiles another copy of the
//! full PBR shader, and under ANGLE/D3D11 each took seconds on the main thread: entering Walk froze for about 11 s
//! (profiled: 10.9 s in getProgramParameter). This material compiles one short program.
use bevy::pbr::{Material, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;
use std::sync::OnceLock;

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct ArMaterial {
    #[uniform(0)]
    pub tint: Vec4,
    /// x, y: distance from the camera (m) where the overlay starts and ends fading in (alpha 0 closer than x); 0, 0 = no fade
    #[uniform(1)]
    pub fade: Vec4,
}

/// Beams fade in between these distances from the camera, so a beam never cuts through the astronaut in front of you. VISUAL.
pub const BEAM_FADE_M: (f32, f32) = (5.0, 12.0);
pub static BEAM_MAT: OnceLock<Handle<ArMaterial>> = OnceLock::new();

pub fn beam_handle() -> Handle<ArMaterial> {
    BEAM_MAT.get().cloned().unwrap_or_default()
}

pub const AR_SHADER: &str = "embedded://visor_app/scene/armat.wesl";
pub const SUIT_SHADER: &str = "embedded://visor_app/scene/suit.wesl";

/// The astronauts' material (suit.wesl): vertex colour lit by the Walk sun; see the shader for why not StandardMaterial.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct SuitMaterial {
    #[uniform(0)]
    pub params: SuitParams,
}

#[derive(bevy::render::render_resource::ShaderType, Clone, Copy, Debug, Default, PartialEq, Reflect)]
pub struct SuitParams {
    pub sun: Vec4,
    pub light: Vec4,
}

impl Material for SuitMaterial {
    fn fragment_shader() -> ShaderRef {
        SUIT_SHADER.into()
    }
}

pub static SUIT_MAT: OnceLock<Handle<SuitMaterial>> = OnceLock::new();

impl Material for ArMaterial {
    fn fragment_shader() -> ShaderRef {
        AR_SHADER.into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
    fn depth_bias(&self) -> f32 {
        50.0
    }
}

pub struct ArPlugin;

impl Plugin for ArPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "armat.wesl");
        bevy::asset::embedded_asset!(app, "suit.wesl");
        app.add_plugins((MaterialPlugin::<ArMaterial>::default(), MaterialPlugin::<SuitMaterial>::default()));
    }
}

/// The shared AR material (created once when the scene is built; spawners clone the handle).
pub static AR_MAT: OnceLock<Handle<ArMaterial>> = OnceLock::new();

pub fn init(mats: &mut Assets<ArMaterial>, suits: &mut Assets<SuitMaterial>) {
    let _ = AR_MAT.set(mats.add(ArMaterial { tint: Vec4::ONE, fade: Vec4::ZERO }));
    let _ = BEAM_MAT.set(mats.add(ArMaterial { tint: Vec4::ONE, fade: Vec4::new(BEAM_FADE_M.0, BEAM_FADE_M.1, 0.0, 0.0) }));
    let p = SuitParams { sun: Vec3::new(0.5, 0.6, 0.0).normalize().extend(0.0),
                         light: Vec4::new(crate::scene::walkscene::AMBIENT, crate::scene::walkscene::DIRECT, 0.12, 0.0) };
    let _ = SUIT_MAT.set(suits.add(SuitMaterial { params: p }));
}

pub fn handle() -> Handle<ArMaterial> {
    AR_MAT.get().cloned().unwrap_or_default()
}

/// The same colour on every vertex (for meshes that have one colour).
pub fn colours(n: usize, c: [f32; 4]) -> Vec<[f32; 4]> {
    vec![c; n]
}
