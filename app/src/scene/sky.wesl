// Walk-mode sky dome (walkscene.rs: SkyMaterial). ILLUSTRATIVE: a colour gradient from horizon to zenith, a bluish glow
// around the sun and the sun disc, at the sun position computed by Mars24 for the chosen sol and hour. Not a radiative-
// transfer model (Bevy's Atmosphere needs compute shaders, which WebGL2 lacks). Colours are linear RGB (Tonemapping::None).
import bevy_pbr::render::{forward_io::VertexOutput, mesh_view_bindings::view};

struct SkyParams {
    // xyz: unit vector toward the sun (world: x east, y up, z south); w: sun elevation in radians
    sun: vec4<f32>,
    zenith: vec4<f32>,
    horizon: vec4<f32>,
    glow: vec4<f32>,
    // x: star visibility 0..1 (illustrative stars, not a catalogue)
    night: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> sky: SkyParams;

fn hash3(p: vec3<f32>) -> f32 {
    return fract(sin(dot(p, vec3<f32>(127.1, 311.7, 74.7))) * 43758.5453);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let d = normalize(in.world_position.xyz - view.world_position);
    let up = clamp(d.y, -1.0, 1.0);
    // horizon to zenith; below the horizon the horizon colour, slightly darker (the dome is mostly hidden by terrain there)
    var c = mix(sky.horizon.rgb, sky.zenith.rgb, smoothstep(0.0, 0.55, max(up, 0.0)));
    c = select(c, sky.horizon.rgb * 0.85, up < 0.0);
    // glow around the sun (stronger when the sun is low), then the disc (~0.35 deg angular radius as seen from Mars)
    let cosang = dot(d, sky.sun.xyz);
    let low = 1.0 - smoothstep(0.0, 0.6, sky.sun.w);
    let glow = pow(max(cosang, 0.0), 24.0) * (0.35 + 0.45 * low);
    c = mix(c, sky.glow.rgb, clamp(glow, 0.0, 1.0));
    // stars above the horizon once the sky is dark: one in ~400 direction cells, a soft point in its cell
    if (sky.night.x > 0.001 && up > 0.02) {
        let q = d * 220.0;
        let cell = floor(q);
        let h = hash3(cell);
        if (h > 0.9975) {
            let centre = cell + vec3<f32>(0.5);
            let r = length(q - centre);
            let b = (h - 0.9975) / 0.0025;
            c += vec3<f32>(0.85, 0.88, 1.0) * sky.night.x * (0.25 + 0.75 * b) * (1.0 - smoothstep(0.15, 0.45, r));
        }
    }
    let disc = smoothstep(0.99996, 0.99999, cosang);
    c = mix(c, vec3<f32>(1.0, 0.98, 0.94), disc * step(-0.02, sky.sun.y));
    return vec4<f32>(c, 1.0);
}
