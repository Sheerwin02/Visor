// VISOR relight: Lambertian shading + heightmap cast shadows from a DEM. Compute version (for the measurement
// and for tests). The same `lambert` and `in_shadow` functions are meant to be pasted into the Bevy terrain
// material's fragment shader so the slider, the measurement and the test renders share one lighting model.
// STATUS: tested only against tools/misleading_slope.py and a synthetic wall on a software adapter; NOT tested in Bevy.
// Conventions: row index increases southward; azimuth is compass degrees from north, clockwise (east = 90).
struct Params {
  width: u32, height: u32, steps: u32, _pad: u32,
  cell: f32, sun_el: f32, sun_az: f32, max_dist: f32,   // radians for angles, metres for distances
};
@group(0) @binding(0) var<uniform> P: Params;
@group(0) @binding(1) var<storage, read> dem: array<f32>;
@group(0) @binding(2) var<storage, read_write> shade: array<f32>;    // lit brightness, 0 where shadowed
@group(0) @binding(3) var<storage, read_write> shadow: array<u32>;   // 1 if in cast shadow

fn z(c: i32, r: i32) -> f32 {
  let cc = clamp(c, 0, i32(P.width) - 1);
  let rr = clamp(r, 0, i32(P.height) - 1);
  return dem[u32(rr) * P.width + u32(cc)];
}
fn z_bilinear(x: f32, y: f32) -> f32 {   // x = column, y = row (cell units)
  let x0 = i32(floor(x)); let y0 = i32(floor(y));
  let fx = x - floor(x);  let fy = y - floor(y);
  let a = mix(z(x0, y0), z(x0 + 1, y0), fx);
  let b = mix(z(x0, y0 + 1), z(x0 + 1, y0 + 1), fx);
  return mix(a, b, fy);
}
fn lambert(c: i32, r: i32) -> f32 {
  // central differences; matches numpy.gradient in the interior. gx = dz/dEast, gy = dz/dSouth (rows).
  let gx = (z(c + 1, r) - z(c - 1, r)) / (2.0 * P.cell);
  let gy = (z(c, r + 1) - z(c, r - 1)) / (2.0 * P.cell);
  let ch = cos(P.sun_el); let sh = sin(P.sun_el);
  let d = (sh + ch * (cos(P.sun_az) * gy - sin(P.sun_az) * gx)) / sqrt(1.0 + gx * gx + gy * gy);
  return clamp(d, 0.0, 1.0);
}
fn in_shadow(c: i32, r: i32) -> bool {
  let z0 = z(c, r);
  let dx = sin(P.sun_az);     // east  -> +column
  let dy = -cos(P.sun_az);    // north -> -row
  let t = tan(P.sun_el);
  let step_m = P.max_dist / f32(P.steps);
  for (var i: u32 = 1u; i <= P.steps; i = i + 1u) {
    let dist = step_m * f32(i);
    let x = f32(c) + dx * dist / P.cell;
    let y = f32(r) + dy * dist / P.cell;
    if (x < 0.0 || y < 0.0 || x > f32(P.width - 1u) || y > f32(P.height - 1u)) { return false; }
    if (z_bilinear(x, y) > z0 + t * dist + 1e-4) { return true; }
  }
  return false;
}
@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
  if (id.x >= P.width || id.y >= P.height) { return; }
  let c = i32(id.x); let r = i32(id.y);
  let i = id.y * P.width + id.x;
  let s = in_shadow(c, r);
  shadow[i] = select(0u, 1u, s);
  shade[i] = select(lambert(c, r), 0.0, s);
}
