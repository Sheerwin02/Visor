// Trust layers drawn on the terrain photo by the GPU (trust.rs: TerrainMaterial). Same patterns as the CPU reference in
// trust.rs (band, hatched; HATCH_PERIOD 8 and HATCH_WIDTH 2 texels at 2 texels per metre), so the legend and the panel's
// profile still match the map. Layer switches and the slope limit/bias are uniforms: changing them costs nothing on the CPU.
// Structure follows Bevy 0.20.1's extended_material.wesl example.
import bevy_pbr::render::pbr_fragment::pbr_input_from_standard_material;

@if(PREPASS_PIPELINE)
import bevy_pbr::{
    prepass::io::{VertexOutput, FragmentOutput},
    deferred::functions::deferred_output,
};
@else
import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::main_pass_post_lighting_processing,
};

struct OverlayParams {
    // bit 1 slope bands, bit 2 rover-verified dots, bit 4 route agreement, bit 8 cast shadow, bit 16 map readiness
    flags: u32,
    bias: f32,
    caution: f32,
    limit: f32,
    // route agreement colour (palette.rs DENSITY), sRGB 0..1
    density: vec4<f32>,
    // selection and hover rings on the ground (select.rs): east, north, radius (m), on
    sel: vec4<f32>,
    hover: vec4<f32>,
    // Walk light on the photo (rgb multiplier, 1 in Plan and full daylight): twilight and night dim it (plan/daylight.rs)
    light: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> params: OverlayParams;
// map slope at 3 m scale, degrees, one texel per grid cell (row 0 north)
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var slope_tex: texture_2d<f32>;
// r: rover-verified (1 inside the radius), g: share of uncertainty runs on the cell; 2 texels per metre
@group(constants::MATERIAL_BIND_GROUP) @binding(102) var marks_tex: texture_2d<f32>;
// cast shadow at the chosen sol and hour (1 = shadow), one texel per grid cell (DEM scale only)
@group(constants::MATERIAL_BIND_GROUP) @binding(103) var shadow_tex: texture_2d<f32>;
// data readiness class per grid cell (byte / 255): 0 unknown, 1 agrees, 2 partial, 3 disagrees, 4 rover-checked
@group(constants::MATERIAL_BIND_GROUP) @binding(104) var ready_tex: texture_2d<f32>;
// highlight mask, 2 texels per metre (select.rs): r = selected leg, g = watch-chip problem
@group(constants::MATERIAL_BIND_GROUP) @binding(105) var hl_tex: texture_2d<f32>;
// DEM heights (m), one texel per grid cell: contour lines (flag 32, Plan only)
@group(constants::MATERIAL_BIND_GROUP) @binding(106) var height_tex: texture_2d<f32>;
// walk-back time to the exit (s), one texel per grid cell; 1e9 = no way back (flag 128, Plan only)
@group(constants::MATERIAL_BIND_GROUP) @binding(107) var back_tex: texture_2d<f32>;

const TEX_PER_M: i32 = 2;
const HATCH_PERIOD: i32 = 8;
const HATCH_WIDTH: i32 = 2;

fn to_srgb(c: vec3<f32>) -> vec3<f32> {
    return pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    return pow(max(c, vec3<f32>(0.0)), vec3<f32>(2.2));
}

// straight-alpha "over" in sRGB space, as the old CPU compositing did
fn over(dst: vec3<f32>, src: vec3<f32>, a: f32) -> vec3<f32> {
    return src * a + dst * (1.0 - a);
}

// Bilinear read of a texture at continuous texel coordinates (textureLoad only: the data textures are not filterable).
fn bilinear(tex: texture_2d<f32>, p: vec2<f32>) -> vec4<f32> {
    let size = vec2<i32>(textureDimensions(tex));
    let q = p - vec2<f32>(0.5);
    let i0 = vec2<i32>(floor(q));
    let f = q - floor(q);
    let hi = size - vec2<i32>(1);
    let c00 = textureLoad(tex, clamp(i0, vec2<i32>(0), hi), 0);
    let c10 = textureLoad(tex, clamp(i0 + vec2<i32>(1, 0), vec2<i32>(0), hi), 0);
    let c01 = textureLoad(tex, clamp(i0 + vec2<i32>(0, 1), vec2<i32>(0), hi), 0);
    let c11 = textureLoad(tex, clamp(i0 + vec2<i32>(1, 1), vec2<i32>(0), hi), 0);
    return mix(mix(c00, c10, f.x), mix(c01, c11, f.x), f.y);
}

// Bilinear over the walk-back field that skips "no way back" texels (1e9): a plain blend lets a tiny weight of 1e9 swamp
// its reachable neighbours, so every steep patch punched holes, rings and dots into the 2-minute lines. Reachable texels are
// re-weighted; where most of the 2 x 2 is blocked the spot counts as blocked (no line drawn there).
fn bilinear_back(tex: texture_2d<f32>, p: vec2<f32>) -> f32 {
    let size = vec2<i32>(textureDimensions(tex));
    let q = p - vec2<f32>(0.5);
    let i0 = vec2<i32>(floor(q));
    let f = q - floor(q);
    let hi = size - vec2<i32>(1);
    let v = vec4<f32>(textureLoad(tex, clamp(i0, vec2<i32>(0), hi), 0).r,
                      textureLoad(tex, clamp(i0 + vec2<i32>(1, 0), vec2<i32>(0), hi), 0).r,
                      textureLoad(tex, clamp(i0 + vec2<i32>(0, 1), vec2<i32>(0), hi), 0).r,
                      textureLoad(tex, clamp(i0 + vec2<i32>(1, 1), vec2<i32>(0), hi), 0).r);
    let w = vec4<f32>((1.0 - f.x) * (1.0 - f.y), f.x * (1.0 - f.y), (1.0 - f.x) * f.y, f.x * f.y);
    let ok = step(v, vec4<f32>(1.0e8));
    let ws = dot(w, ok);
    let t = dot(w * ok, v) / max(ws, 1e-6);
    return select(1.0e9, t, ws >= 0.5);
}

// Anti-aliased stripes: coverage of lines `width` texels wide every `period` texels along coordinate u; aa = fwidth(u).
fn stripe(u: f32, period: f32, width: f32, aa: f32) -> f32 {
    let f = fract(u / period);
    let d = min(f, 1.0 - f) * period;
    return 1.0 - smoothstep(width * 0.5 - aa, width * 0.5 + aa, d);
}

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);

    let dims = vec2<f32>(textureDimensions(marks_tex));
    let t = in.uv * dims;
    // texels per screen pixel: when many texels fall in one pixel, a pattern is replaced by its mean coverage (no shimmer)
    let fp = max(length(dpdx(t)), length(dpdy(t)));
    let blur = clamp(fp - 1.0, 0.0, 1.0);
    // derivatives in uniform control flow, for anti-aliased lines and dots
    let aa_d = max(fwidth(t.x + t.y), 1e-3);
    let aa_y = max(fwidth(t.y), 1e-3);
    let aa_p = max(fwidth(t.x), 1e-3);
    let period = f32(HATCH_PERIOD);
    let width = f32(HATCH_WIDTH);
    let cell = t / f32(TEX_PER_M);
    // ground position in metres (terrain.rs: east = u x width, north = (1 - v) x height, 1 m cells)
    let gd = vec2<f32>(textureDimensions(slope_tex));
    let world = vec2<f32>(in.uv.x * gd.x, (1.0 - in.uv.y) * gd.y);
    let d_sel = distance(world, params.sel.xy);
    let d_hov = distance(world, params.hover.xy);
    let aa_s = max(fwidth(d_sel), 1e-3);
    let aa_h = max(fwidth(d_hov), 1e-3);
    // contours: height and its screen derivative in uniform control flow
    let hz = bilinear(height_tex, cell).r;
    let aa_z = max(fwidth(hz), 1e-4);
    // walk-back minutes and their screen derivative (uniform control flow)
    let tb = bilinear_back(back_tex, cell) / 60.0;
    let aa_b = max(fwidth(tb), 1e-4);

    var c = to_srgb(pbr_input.material.base_color.rgb);
    // shading from terrain shape (flag 256): curvature of the DEM over 3 m (Laplacian of bilinear heights): hollows a
    // little darker, crests a little lighter, so the boxwork relief reads at eye height; derived from the DEM only
    let gd0 = vec2<f32>(textureDimensions(height_tex));
    let cc = in.uv * gd0;
    let kc = 3.0;
    let h0 = bilinear(height_tex, cc).r;
    let lap = (bilinear(height_tex, cc + vec2<f32>(kc, 0.0)).r + bilinear(height_tex, cc - vec2<f32>(kc, 0.0)).r
             + bilinear(height_tex, cc + vec2<f32>(0.0, kc)).r + bilinear(height_tex, cc - vec2<f32>(0.0, kc)).r - 4.0 * h0) / (kc * kc);
    if ((params.flags & 256u) != 0u) {
        let k = clamp(lap * 10.0, -1.0, 1.0);
        c = c * (1.0 - 0.28 * max(k, 0.0));
        c = c + (vec3<f32>(1.0) - c) * (0.14 * max(-k, 0.0));
    }
    if ((params.flags & 1u) != 0u) {
        // corrected slope, smooth between cells; band edges blended over half a degree (no pixel steps)
        let corr = bilinear(slope_tex, cell).r - params.bias;
        let w1 = smoothstep(params.caution - 0.25, params.caution + 0.25, corr);
        let w2 = smoothstep(params.limit - 0.25, params.limit + 0.25, corr);
        let s1 = stripe(t.x + t.y, period, width, aa_d);
        let s2 = stripe(t.x - t.y, period, width, aa_d);
        let cover = mix(max(s1 * w1, s2 * w2), 0.25 * w1 + 0.1875 * w2, blur);
        c = over(c, vec3<f32>(18.0, 14.0, 12.0) / 255.0, cover * 150.0 / 255.0);
    }
    let m = bilinear(marks_tex, t);
    if ((params.flags & 2u) != 0u && m.r > 0.5) {
        // round dots every 4 texels (2 m)
        let g = fract(t / 4.0) - vec2<f32>(0.5);
        let d = length(g) * 4.0;
        let dot = 1.0 - smoothstep(0.9 - aa_p, 0.9 + aa_p, d);
        let cover = mix(dot, 0.16, blur) * smoothstep(0.5, 0.7, m.r);
        c = over(c, vec3<f32>(1.0), cover * 235.0 / 255.0);
    }
    // shadow: a light dark wash plus thin horizontal lines (distinct from the diagonal slope hatching), smooth edges
    if ((params.flags & 8u) != 0u) {
        let inside = smoothstep(0.35, 0.65, bilinear(shadow_tex, cell).r);
        if (inside > 0.0) {
            c = over(c, vec3<f32>(0.0), 0.22 * inside);
            let line = mix(stripe(t.y, 4.0, 1.0, aa_y), 0.25, blur);
            c = over(c, vec3<f32>(18.0, 14.0, 12.0) / 255.0, line * 0.55 * inside);
        }
    }
    // map readiness: vertical lines (distinct from diagonal slope hatching and horizontal shadow lines); sparse where the two
    // HiRISE DTMs' 3 m slopes differ a little (partial), dense where they differ more (disagrees). Classes are categories:
    // nearest cell, no blending between them.
    if ((params.flags & 16u) != 0u) {
        let rd = vec2<i32>(textureDimensions(ready_tex));
        let k = i32(round(textureLoad(ready_tex, clamp(vec2<i32>(floor(cell)), vec2<i32>(0), rd - vec2<i32>(1)), 0).r * 255.0));
        if (k == 2 || k == 3) {
            let per = select(8.0, 4.0, k == 3);
            let line = mix(stripe(t.x, per, 1.0, aa_p), 1.0 / per, blur);
            c = over(c, vec3<f32>(240.0, 236.0, 228.0) / 255.0, line * 0.45);
        }
    }
    if ((params.flags & 32u) != 0u) {
        // 1 m contours, bold every 5 m; each fades out when its lines would crowd closer than ~3 px
        let d1 = abs(fract(hz + 0.5) - 0.5);
        let d5 = abs(fract(hz / 5.0 + 0.5) - 0.5) * 5.0;
        let l1 = (1.0 - smoothstep(0.6 * aa_z, 1.4 * aa_z, d1)) * (1.0 - smoothstep(0.25, 0.4, aa_z));
        let l5 = (1.0 - smoothstep(1.0 * aa_z, 2.0 * aa_z, d5)) * (1.0 - smoothstep(1.2, 2.0, aa_z));
        c = over(c, vec3<f32>(0.08, 0.05, 0.03), 0.17 * l1);
        c = over(c, vec3<f32>(0.08, 0.05, 0.03), 0.38 * l5);
    }
    if ((params.flags & 128u) != 0u && tb < 8.5) {
        // equal walk-back time to the exit every 2 min: thin violet lines with a dark halo, so "how far from safety" reads
        // anywhere on the map; the beyond-6-minute ground gets a faint cool wash
        let dk = abs(fract(tb / 2.0 + 0.5) - 0.5) * 2.0;
        let ln = (1.0 - smoothstep(0.6 * aa_b, 1.5 * aa_b, dk)) * (1.0 - smoothstep(0.35, 0.6, aa_b)) * step(1.0, tb);
        let halo = (1.0 - smoothstep(1.5 * aa_b, 2.6 * aa_b, dk)) * (1.0 - smoothstep(0.35, 0.6, aa_b)) * step(1.0, tb);
        c = over(c, vec3<f32>(0.02, 0.05, 0.08), 0.35 * halo);
        // pale violet: Curiosity's path is cyan, the two must not read as one network
        c = over(c, vec3<f32>(0.83, 0.72, 1.0), 0.8 * ln);
        c = over(c, vec3<f32>(0.3, 0.55, 0.85), 0.10 * smoothstep(5.5, 6.5, tb));
    }
    if ((params.flags & 4u) != 0u && m.g > 0.0) {
        c = over(c, params.density.rgb, 230.0 / 255.0 * sqrt(m.g));
    }
    // highlights draped on the ground (hidden behind hills, unlike a screen overlay)
    let hl = bilinear(hl_tex, t);
    if ((params.flags & 64u) != 0u && hl.b > 0.02) {
        // the route, coloured by the corrected slope under it: pale below the caution angle, orange to the limit, red over it;
        // a brightness ramp along the walking direction (phase in a) reads as flow; dark edge for contrast on the photo
        let corr = bilinear(slope_tex, cell).r - params.bias;
        let k1 = smoothstep(params.caution - 4.0, params.caution, corr);
        let k2 = smoothstep(params.limit - 1.0, params.limit + 0.5, corr);
        let col = mix(mix(vec3<f32>(1.0, 0.93, 0.62), vec3<f32>(1.0, 0.6, 0.12), k1), vec3<f32>(0.92, 0.16, 0.12), k2);
        let rd = vec2<i32>(textureDimensions(hl_tex));
        let ph = textureLoad(hl_tex, clamp(vec2<i32>(floor(t)), vec2<i32>(0), rd - vec2<i32>(1)), 0).a;
        let flow = mix(0.62 + 0.38 * ph, 0.85, blur);
        c = over(c, vec3<f32>(0.05, 0.03, 0.02), 0.6 * smoothstep(0.02, 0.3, hl.b));
        c = over(c, col * flow, smoothstep(0.35, 0.65, hl.b));
    }
    if (hl.g > 0.02) {
        // watch-chip problem: pink dashes along the stretch / ring, with a dark edge for contrast
        let dash = mix(stripe(t.x + t.y, 8.0, 4.0, aa_d), 0.5, blur);
        c = over(c, vec3<f32>(0.0), 0.45 * smoothstep(0.02, 0.25, hl.g));
        c = over(c, vec3<f32>(1.0, 0.42, 0.6), 0.95 * smoothstep(0.35, 0.6, hl.g) * dash);
    }
    if (hl.r > 0.02) {
        // selected leg: dark halo then an accent band
        c = over(c, vec3<f32>(0.0), 0.5 * smoothstep(0.02, 0.25, hl.r));
        c = over(c, vec3<f32>(1.0, 0.71, 0.24), 0.95 * smoothstep(0.35, 0.6, hl.r));
    }
    if (params.hover.w > 0.5) {
        let w = max(params.hover.z * 0.05, 0.08);
        let ring = 1.0 - smoothstep(w - aa_h, w + aa_h, abs(d_hov - params.hover.z));
        c = over(c, vec3<f32>(1.0), 0.9 * ring);
    }
    if (params.sel.w > 0.5) {
        let w = max(params.sel.z * 0.055, 0.1);
        let halo = 1.0 - smoothstep(1.8 * w - aa_s, 1.8 * w + aa_s, abs(d_sel - params.sel.z));
        let ring = 1.0 - smoothstep(w - aa_s, w + aa_s, abs(d_sel - params.sel.z));
        c = over(c, vec3<f32>(0.0), 0.4 * halo);
        c = over(c, vec3<f32>(1.0, 0.71, 0.24), ring);
    }
    pbr_input.material.base_color = vec4<f32>(to_linear(c) * params.light.rgb, pbr_input.material.base_color.a);

    var out: FragmentOutput;
@if(PREPASS_PIPELINE) {
    out = deferred_output(in, pbr_input);
} @else {
    // the terrain is unlit (the photo carries real lighting); fog, tonemapping as for the standard material
    out.color = main_pass_post_lighting_processing(pbr_input, pbr_input.material.base_color);
}
    return out;
}
