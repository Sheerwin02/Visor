//! Light for the walk (best-hour rule, fixed before any hour was computed).
//! - Shadows: hard, DEM-scale only. Heights are the 1 m DEM smoothed with a 3 x 3 m box (the scale of the route's slopes);
//!   a cell is shadowed if a ray from its centre toward the sun meets higher ground (step 0.5 cell, bilinear heights, stop
//!   above the highest point or off the grid = lit). Same ray march as terrain-core's in_shadow (tested there against Python
//!   and relight.wgsl); checked against it here. Ridges 1-2 m tall are below this model: their shadows are not shown.
//! - Best hour: for each whole-hour start (local true solar time), the sun along the walk; pass = 15-45 deg high at every
//!   route point; tie-breaks: least length toward a low sun, least length in shadow, largest margin to the band.
//!   Times along a leg are spread in proportion to length (an approximation; legs are a few minutes long).
use crate::sun::{direction, sun_at_sol};

/// Seconds in one Mars hour (1/24 of a mean sol: 1.0274912517 Earth days, GISS C-2).
pub const MARS_HOUR_S: f64 = 1.0274912517 * 86400.0 / 24.0;

pub const BAND: (f64, f64) = (15.0, 45.0);
pub const LOW_SUN_DEG: f64 = 25.0;
pub const TOWARD_CONE_DEG: f64 = 30.0;
pub const FIRST_HOUR: u32 = 6;
pub const LAST_HOUR: u32 = 17;

/// 3 x 3 box mean (edge cells average the neighbours that exist).
pub fn box3(z: &[f32], w: usize, h: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; w * h];
    for r in 0..h {
        for c in 0..w {
            let (mut s, mut n) = (0.0f32, 0.0f32);
            for rr in r.saturating_sub(1)..(r + 2).min(h) {
                for cc in c.saturating_sub(1)..(c + 2).min(w) {
                    s += z[rr * w + cc];
                    n += 1.0;
                }
            }
            out[r * w + c] = s / n;
        }
    }
    out
}

pub struct Heights {
    pub w: usize,
    pub h: usize,
    pub cell: f64,
    pub z: Vec<f32>,
    pub zmax: f32,
}

impl Heights {
    pub fn new(z: Vec<f32>, w: usize, h: usize, cell: f64) -> Heights {
        let zmax = z.iter().cloned().fold(f32::MIN, f32::max);
        Heights { w, h, cell, z, zmax }
    }

    fn at(&self, c: i64, r: i64) -> f32 {
        self.z[r.clamp(0, self.h as i64 - 1) as usize * self.w + c.clamp(0, self.w as i64 - 1) as usize]
    }

    fn bilinear(&self, x: f32, y: f32) -> f32 {
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let (xi, yi) = (x0 as i64, y0 as i64);
        let a = self.at(xi, yi) * (1.0 - fx) + self.at(xi + 1, yi) * fx;
        let b = self.at(xi, yi + 1) * (1.0 - fx) + self.at(xi + 1, yi + 1) * fx;
        a * (1.0 - fy) + b * fy
    }

    /// Is the cell (column c, row r; row 0 north) in cast shadow for a sun at (elevation, azimuth) degrees?
    pub fn in_shadow(&self, c: usize, r: usize, el_deg: f32, az_deg: f32) -> bool {
        if el_deg <= 0.0 {
            return true; // night
        }
        let z0 = self.at(c as i64, r as i64);
        let a = az_deg.to_radians();
        let (dx, dy) = (a.sin(), -a.cos());
        let t = el_deg.to_radians().tan();
        let step = 0.5 * self.cell as f32;
        let mut dist = step;
        loop {
            if z0 + t * dist > self.zmax {
                return false;
            }
            let x = c as f32 + dx * dist / self.cell as f32;
            let y = r as f32 + dy * dist / self.cell as f32;
            if x < 0.0 || y < 0.0 || x > (self.w - 1) as f32 || y > (self.h - 1) as f32 {
                return false;
            }
            if self.bilinear(x, y) > z0 + t * dist + 1e-4 {
                return true;
            }
            dist += step;
        }
    }

    /// Shadow mask for the whole grid: 255 = shadow, 0 = lit (row-major, row 0 north).
    pub fn shadow_mask(&self, el_deg: f32, az_deg: f32) -> Vec<u8> {
        let mut m = vec![0u8; self.w * self.h];
        for r in 0..self.h {
            for c in 0..self.w {
                if self.in_shadow(c, r, el_deg, az_deg) {
                    m[r * self.w + c] = 255;
                }
            }
        }
        m
    }

    fn cell_of(&self, e: f64, n: f64) -> Option<(usize, usize)> {
        let (c, r) = ((e / self.cell).floor(), ((self.h as f64 * self.cell - n) / self.cell).floor());
        (c >= 0.0 && r >= 0.0 && (c as usize) < self.w && (r as usize) < self.h).then(|| (c as usize, r as usize))
    }
}

/// One candidate start hour.
#[derive(Debug, Clone, PartialEq)]
pub struct HourResult {
    pub start_ltst: f64,
    pub passes: bool,
    pub min_elev: f64,
    pub max_elev: f64,
    /// metres walked toward a low sun (heading within the cone of the sun's azimuth, sun under LOW_SUN_DEG)
    pub toward_low_sun_m: f64,
    /// metres walked straight away from the sun (heading within the cone of the anti-sun azimuth)
    pub away_m: f64,
    /// metres walked through DEM-scale cast shadow
    pub shadow_m: f64,
    /// smallest distance of the sun's height to either band edge during the walk (negative = outside the band)
    pub margin: f64,
    /// sun at the start of the walk
    pub start_elev: f64,
    pub start_az: f64,
}

fn ang_diff(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(360.0);
    d.min(360.0 - d)
}

/// Route points with the time (s from the start of the walk) each is reached: legs from `plan_legs` as
/// (poly_start, poly_end, time_s) of the polyline `poly` (east, north metres); within a leg, time in proportion to length.
pub fn timed_route(poly: &[(f64, f64)], plan_legs: &[(usize, usize, f64)]) -> Vec<(f64, f64, f64)> {
    let mut out: Vec<(f64, f64, f64)> = Vec::new();
    let mut t0 = 0.0;
    for &(a, b, secs) in plan_legs {
        if b <= a || b >= poly.len() {
            continue;
        }
        let seg: Vec<f64> = (a..b).map(|i| ((poly[i + 1].0 - poly[i].0).powi(2) + (poly[i + 1].1 - poly[i].1).powi(2)).sqrt()).collect();
        let len: f64 = seg.iter().sum();
        if out.is_empty() {
            out.push((poly[a].0, poly[a].1, t0));
        }
        let mut acc = 0.0;
        for (k, s) in seg.iter().enumerate() {
            acc += s;
            out.push((poly[a + k + 1].0, poly[a + k + 1].1, t0 + if len > 0.0 { secs * acc / len } else { 0.0 }));
        }
        t0 += secs;
    }
    out
}

/// Evaluate every candidate start hour on `sol` for the timed route (see `timed_route`).
pub fn hours(route: &[(f64, f64, f64)], heights: &Heights, sol: f64, west_lon: f64, lat_pg: f64) -> Vec<HourResult> {
    (FIRST_HOUR..=LAST_HOUR).map(|h| {
        let start = h as f64;
        let sun_at = |t: f64| sun_at_sol(sol, start + t / MARS_HOUR_S, west_lon, lat_pg);
        let s0 = sun_at(0.0);
        let (mut lo, mut hi, mut margin) = (f64::MAX, f64::MIN, f64::MAX);
        for p in route {
            let el = sun_at(p.2).elevation;
            lo = lo.min(el);
            hi = hi.max(el);
            margin = margin.min((el - BAND.0).min(BAND.1 - el));
        }
        let (mut toward, mut away, mut shadow) = (0.0, 0.0, 0.0);
        for w in route.windows(2) {
            let (a, b) = (w[0], w[1]);
            let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
            if len == 0.0 {
                continue;
            }
            let heading = (b.0 - a.0).atan2(b.1 - a.1).to_degrees().rem_euclid(360.0);
            let s = sun_at(0.5 * (a.2 + b.2));
            if s.elevation < LOW_SUN_DEG && ang_diff(heading, s.azimuth) <= TOWARD_CONE_DEG {
                toward += len;
            }
            if ang_diff(heading, s.azimuth + 180.0) <= TOWARD_CONE_DEG {
                away += len;
            }
            if let Some((c, r)) = heights.cell_of(0.5 * (a.0 + b.0), 0.5 * (a.1 + b.1)) {
                if heights.in_shadow(c, r, s.elevation as f32, s.azimuth as f32) {
                    shadow += len;
                }
            }
        }
        HourResult { start_ltst: start, passes: lo >= BAND.0 && hi <= BAND.1, min_elev: lo, max_elev: hi, toward_low_sun_m: toward,
                     away_m: away, shadow_m: shadow, margin, start_elev: s0.elevation, start_az: s0.azimuth }
    }).collect()
}

/// The best hour by the rule: among passing hours, least toward-low-sun length, then least shadow, then largest margin.
/// Err(nearest) when no hour passes: the hour whose sun leaves the band least (largest margin, negative).
pub fn best(results: &[HourResult]) -> Result<usize, Option<usize>> {
    let pass: Vec<usize> = (0..results.len()).filter(|&i| results[i].passes).collect();
    if pass.is_empty() {
        return Err((0..results.len()).max_by(|&a, &b| results[a].margin.total_cmp(&results[b].margin)));
    }
    let key = |i: usize| (results[i].toward_low_sun_m, results[i].shadow_m, -results[i].margin);
    Ok(*pass.iter().min_by(|&&a, &&b| {
        let (x, y) = (key(a), key(b));
        x.0.total_cmp(&y.0).then(x.1.total_cmp(&y.1)).then(x.2.total_cmp(&y.2))
    }).unwrap())
}

/// Sun direction helper for callers drawing the sun (x east, y north, z up).
pub fn sun_dir(elevation: f64, azimuth: f64) -> [f64; 3] {
    direction(elevation, azimuth)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wall_casts_a_shadow_of_the_right_length() {
        // 5 m wall on flat ground, sun from the east at 20 deg: shadow west of it, about 5 / tan 20 = 13.7 m long
        let (w, h) = (80, 10);
        let mut z = vec![0.0f32; w * h];
        for r in 0..h {
            z[r * w + 60] = 5.0;
        }
        let g = Heights::new(z, w, h, 1.0);
        let m = g.shadow_mask(20.0, 90.0);
        let row = 5;
        let shaded = (0..60).filter(|&c| m[row * w + c] == 255).count();
        assert!((12..=15).contains(&shaded), "shadow length {shaded} cells");
        assert_eq!(m[row * w + 70], 0, "the sunny side is lit");
        assert_eq!(g.shadow_mask(-1.0, 90.0)[0], 255, "night");
    }

    #[test]
    fn same_ray_march_as_terrain_core() {
        // a bumpy synthetic surface: identical shadow decisions to terrain-core's tested in_shadow (same step)
        let (w, h) = (60, 50);
        let z: Vec<f32> = (0..w * h).map(|i| {
            let (x, y) = ((i % w) as f32, (i / w) as f32);
            3.0 * (x * 0.31).sin() * (y * 0.23).cos() + 0.05 * x
        }).collect();
        let ours = Heights::new(z.clone(), w, h, 1.0);
        let reference = terrain_core::Grid::new(w, h, 1.0, z);
        for (el, az) in [(15.0f32, 80.0f32), (30.0, 280.0), (45.0, 200.0)] {
            for r in 0..h {
                for c in 0..w {
                    let steps = 400; // 0.5 m steps over 200 m: past every ray's exit on this grid
                    assert_eq!(ours.in_shadow(c, r, el, az), reference.in_shadow(c, r, el, az, steps, 0.5 * steps as f32),
                               "cell {c},{r} at {el}/{az}");
                }
            }
        }
    }

    #[test]
    fn box3_keeps_a_plane_and_smooths_a_spike() {
        let (w, h) = (5, 5);
        let plane: Vec<f32> = (0..25).map(|i| (i % 5) as f32).collect();
        let b = box3(&plane, w, h);
        assert!((b[12] - 2.0).abs() < 1e-6);
        let mut spike = vec![0.0f32; 25];
        spike[12] = 9.0;
        assert!((box3(&spike, w, h)[12] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn hours_follow_the_rule() {
        // flat ground, a walk heading due east then back west, sol 4718 at the VISOR site
        let g = Heights::new(vec![0.0; 100 * 100], 100, 100, 1.0);
        let poly = vec![(10.5, 50.5), (90.5, 50.5), (10.5, 50.5)];
        let route = timed_route(&poly, &[(0, 1, 200.0), (1, 2, 200.0)]);
        assert_eq!(route.len(), 3);
        assert!((route[2].2 - 400.0).abs() < 1e-9);
        let r = hours(&route, &g, 4718.0, 222.62, -4.8623);
        assert_eq!(r.len(), (LAST_HOUR - FIRST_HOUR + 1) as usize);
        // noon: sun far above 45 deg -> fails; 08:00 (29.4 deg, Python table) passes
        let at = |h: u32| &r[(h - FIRST_HOUR) as usize];
        assert!(!at(12).passes && at(8).passes, "{:?} {:?}", at(12), at(8));
        assert_eq!(at(8).shadow_m, 0.0, "flat ground casts no shadow");
        // 07:00 starts at 14.6 deg (under the band): fails; morning low sun from the east: walking east = toward it
        assert!(!at(7).passes);
        let b = best(&r).unwrap();
        assert!(r[b].passes);
        assert!(r.iter().filter(|x| x.passes).all(|x| (x.toward_low_sun_m, x.shadow_m) >= (r[b].toward_low_sun_m, r[b].shadow_m)));
        assert_eq!(best(&[]), Err(None));
    }
}
