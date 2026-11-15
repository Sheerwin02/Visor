// AR overlay material (armat.rs): unlit vertex colour x tint, alpha blended. One small shader for every Walk-mode overlay
// (route ribbon, chevrons, beams, working circles, rover footprints), so entering Walk compiles one short program instead of
// several variants of the full PBR shader (each took seconds under ANGLE/D3D11, measured). Meshes must carry
// ATTRIBUTE_COLOR.
import bevy_pbr::render::{forward_io::VertexOutput, mesh_view_bindings::view};

struct ArParams {
    tint: vec4<f32>,
}

// x, y: fade-in distances from the camera (m); y = 0: no fade (beams fade so they never cut through the suit in front)
struct ArFade {
    near_far: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> ar: ArParams;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var<uniform> fade: ArFade;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    var c = in.color * ar.tint;
    if (fade.near_far.y > 0.0) {
        let d = distance(in.world_position.xyz, view.world_position);
        c.a = c.a * smoothstep(fade.near_far.x, fade.near_far.y, d);
    }
    return c;
}
