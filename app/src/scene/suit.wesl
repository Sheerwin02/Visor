// Suit material (armat.rs: SuitMaterial): vertex colour x (ambient + direct x max(N.L, 0)) with the Walk sun direction.
// A small lit shader instead of StandardMaterial's full PBR variant with vertex colours, whose first compile froze Walk for
// about 6 s under ANGLE/D3D11 (measured). Lighting is VISUAL (soft, as the terrain in walkscene.rs).
import bevy_pbr::render::forward_io::VertexOutput;

struct SuitParams {
    // xyz: unit vector toward the sun (world)
    sun: vec4<f32>,
    // x: ambient, y: direct, z: self-light so the white suit does not read grey in its own shade
    light: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> suit: SuitParams;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let n = normalize(in.world_normal);
    let k = suit.light.x + suit.light.y * max(dot(n, suit.sun.xyz), 0.0) + suit.light.z;
    return vec4<f32>(in.color.rgb * k, 1.0);
}
