//! terrain-core: slope, Lambertian shading, heightmap cast shadows and the "looks flat" measurement.
//! Conventions (same as tools/misleading_slope.py and tools/shader/relight.wgsl):
//! row index increases southward; azimuth is compass degrees from north, clockwise (east = 90);
//! sun elevation in degrees above the horizon.
//! STATUS: tested against Python-generated fixtures on synthetic data only. Not run on the real DEM.
//! No smoothing here (the Python script smooths 2 m by default; do smoothing in `prep` if wanted).
//! Gradient edges follow numpy.gradient (one-sided at the border); the WGSL shader clamps instead, so edge pixels differ.

use rayon::prelude::*;

pub struct Grid {
    pub w: usize,
    pub h: usize,
    pub cell: f32, // metres per pixel
    pub z: Vec<f32>, // row-major, len = w * h
    pub zmax: f32,   // highest elevation in the grid, used to stop shadow rays early
}

impl Grid {
    pub fn new(w: usize, h: usize, cell: f32, z: Vec<f32>) -> Self {
        assert_eq!(z.len(), w * h, "z length must be w*h");
        let zmax = z.iter().cloned().fold(f32::MIN, f32::max);
        Grid { w, h, cell, z, zmax }
    }
    #[inline]
    pub fn at(&self, c: i64, r: i64) -> f32 {
        let c = c.clamp(0, self.w as i64 - 1) as usize;
        let r = r.clamp(0, self.h as i64 - 1) as usize;
        self.z[r * self.w + c]
    }
    #[inline]
    fn bilinear(&self, x: f32, y: f32) -> f32 {
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let (xi, yi) = (x0 as i64, y0 as i64);
        let a = self.at(xi, yi) * (1.0 - fx) + self.at(xi + 1, yi) * fx;
        let b = self.at(xi, yi + 1) * (1.0 - fx) + self.at(xi + 1, yi + 1) * fx;
        a * (1.0 - fy) + b * fy
    }
    /// (dz/dEast, dz/dSouth) per metre, numpy.gradient style.
    pub fn gradient(&self, c: usize, r: usize) -> (f32, f32) {
        let d = |lo: f32, hi: f32, span: f32| (hi - lo) / (span * self.cell);
        let (ci, ri) = (c as i64, r as i64);
        let gx = if c == 0 { d(self.at(0, ri), self.at(1, ri), 1.0) }
            else if c == self.w - 1 { d(self.at(ci - 1, ri), self.at(ci, ri), 1.0) }
            else { d(self.at(ci - 1, ri), self.at(ci + 1, ri), 2.0) };
        let gy = if r == 0 { d(self.at(ci, 0), self.at(ci, 1), 1.0) }
            else if r == self.h - 1 { d(self.at(ci, ri - 1), self.at(ci, ri), 1.0) }
            else { d(self.at(ci, ri - 1), self.at(ci, ri + 1), 2.0) };
        (gx, gy)
    }
    pub fn slope_deg(&self, c: usize, r: usize) -> f32 {
        let (gx, gy) = self.gradient(c, r);
        gx.hypot(gy).atan().to_degrees()
    }
    /// Lambertian brightness 0..1 (no shadows, no ambient light).
    pub fn lambert(&self, c: usize, r: usize, el_deg: f32, az_deg: f32) -> f32 {
        let (gx, gy) = self.gradient(c, r);
        let (h, a) = (el_deg.to_radians(), az_deg.to_radians());
        let d = (h.sin() + h.cos() * (a.cos() * gy - a.sin() * gx)) / (1.0 + gx * gx + gy * gy).sqrt();
        d.clamp(0.0, 1.0)
    }
    /// Heightmap ray-march toward the sun, same algorithm as relight.wgsl (fixed step = max_dist / steps).
    pub fn in_shadow(&self, c: usize, r: usize, el_deg: f32, az_deg: f32, steps: u32, max_dist: f32) -> bool {
        let z0 = self.at(c as i64, r as i64);
        let a = az_deg.to_radians();
        let (dx, dy) = (a.sin(), -a.cos()); // east -> +column, north -> -row
        let t = el_deg.to_radians().tan();
        let step_m = max_dist / steps as f32;
        for i in 1..=steps {
            let dist = step_m * i as f32;
            if z0 + t * dist > self.zmax { return false; } // ray is above every point of the grid: cannot be shadowed
            let x = c as f32 + dx * dist / self.cell;
            let y = r as f32 + dy * dist / self.cell;
            if x < 0.0 || y < 0.0 || x > (self.w - 1) as f32 || y > (self.h - 1) as f32 { return false; }
            if self.bilinear(x, y) > z0 + t * dist + 1e-4 { return true; }
        }
        false
    }
    /// Shade and shadow maps for the whole grid, rows in parallel. Shadowed pixels get brightness 0.
    pub fn relight(&self, el_deg: f32, az_deg: f32, steps: u32, max_dist: f32) -> (Vec<f32>, Vec<bool>) {
        let rows: Vec<(Vec<f32>, Vec<bool>)> = (0..self.h).into_par_iter().map(|r| {
            let mut s = Vec::with_capacity(self.w);
            let mut sh = Vec::with_capacity(self.w);
            for c in 0..self.w {
                let shadowed = in_shadow_opt(self, c, r, el_deg, az_deg, steps, max_dist);
                sh.push(shadowed);
                s.push(if shadowed { 0.0 } else { self.lambert(c, r, el_deg, az_deg) });
            }
            (s, sh)
        }).collect();
        let (mut shade, mut shadow) = (Vec::with_capacity(self.w * self.h), Vec::with_capacity(self.w * self.h));
        for (s, sh) in rows { shade.extend(s); shadow.extend(sh); }
        (shade, shadow)
    }
}

fn in_shadow_opt(g: &Grid, c: usize, r: usize, el: f32, az: f32, steps: u32, max_dist: f32) -> bool {
    steps > 0 && g.in_shadow(c, r, el, az, steps, max_dist)
}

/// Result of the looks-flat measurement for one (band, sun) setting.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Share {
    pub n_steep: usize,        // steep pixels inside the mask (slope >= band)
    pub n_shadowed: usize,     // of those, in cast shadow (only when shadows are on)
    pub n_looks_flat: usize,   // lit steep pixels whose shade is within tol of flat shade
}
impl Share {
    /// Share of ALL steep pixels that are lit and look flat (shadowed steep ground is counted as not looking flat).
    pub fn looks_flat_share(&self) -> Option<f32> {
        if self.n_steep == 0 { None } else { Some(self.n_looks_flat as f32 / self.n_steep as f32) }
    }
}

/// mask = (row0, row1, col0, col1) half-open. `steps = 0` turns shadows off (matches the Python script).
pub fn looks_flat(g: &Grid, mask: (usize, usize, usize, usize), band_deg: f32, el_deg: f32, az_deg: f32,
                  tol: f32, steps: u32, max_dist: f32) -> Share {
    let flat = el_deg.to_radians().sin();
    let (r0, r1, c0, c1) = mask;
    let counts = (r0..r1.min(g.h)).into_par_iter().map(|r| {
        let (mut n, mut ns, mut nf) = (0usize, 0usize, 0usize);
        for c in c0..c1.min(g.w) {
            if g.slope_deg(c, r) < band_deg { continue; }
            n += 1;
            if in_shadow_opt(g, c, r, el_deg, az_deg, steps, max_dist) { ns += 1; continue; }
            if (g.lambert(c, r, el_deg, az_deg) - flat).abs() <= tol * flat { nf += 1; }
        }
        (n, ns, nf)
    }).reduce(|| (0, 0, 0), |a, b| (a.0 + b.0, a.1 + b.1, a.2 + b.2));
    Share { n_steep: counts.0, n_shadowed: counts.1, n_looks_flat: counts.2 }
}

/// One pass over the mask for ONE sun setting, counting for every (band, tol) pair at once.
/// `bands` and `tols` index the result as [band_index * tols.len() + tol_index].
#[derive(Debug, Clone, Copy, Default)]
pub struct MultiShare {
    pub n_steep: usize,
    pub n_shadowed: usize,       // steep pixels in cast shadow (counted whether or not shadows are used)
    pub looks_flat_no_shadow: usize, // looks flat by shading alone, shadows ignored
    pub looks_flat_lit: usize,   // looks flat AND not in cast shadow
}
pub fn looks_flat_multi(g: &Grid, mask: (usize, usize, usize, usize), bands: &[f32], tols: &[f32],
                        el_deg: f32, az_deg: f32, steps: u32, max_dist: f32) -> Vec<MultiShare> {
    let flat = el_deg.to_radians().sin();
    let k = bands.len() * tols.len();
    let (r0, r1, c0, c1) = mask;
    (r0..r1.min(g.h)).into_par_iter().map(|r| {
        let mut acc = vec![MultiShare::default(); k];
        for c in c0..c1.min(g.w) {
            let sl = g.slope_deg(c, r);
            if sl < bands.iter().cloned().fold(f32::MAX, f32::min) { continue; }
            let shadowed = in_shadow_opt(g, c, r, el_deg, az_deg, steps, max_dist);
            let diff = (g.lambert(c, r, el_deg, az_deg) - flat).abs();
            for (bi, &b) in bands.iter().enumerate() {
                if sl < b { continue; }
                for (ti, &t) in tols.iter().enumerate() {
                    let a = &mut acc[bi * tols.len() + ti];
                    a.n_steep += 1;
                    if shadowed { a.n_shadowed += 1; }
                    if diff <= t * flat { a.looks_flat_no_shadow += 1; if !shadowed { a.looks_flat_lit += 1; } }
                }
            }
        }
        acc
    }).reduce(|| vec![MultiShare::default(); k], |mut a, b| {
        for i in 0..k { a[i].n_steep += b[i].n_steep; a[i].n_shadowed += b[i].n_shadowed;
            a[i].looks_flat_no_shadow += b[i].looks_flat_no_shadow; a[i].looks_flat_lit += b[i].looks_flat_lit; }
        a
    })
}
