//! VISOR loop planner: the same rule as tools/route.py (fixed before the route was computed), in Rust,
//! so the web app can re-plan live when the planner changes the stops or the assumptions.
//!
//! - corrected slope = map slope at 3 m scale - bias, floored at 0 (bias = the rover check's signed median, or 0 when the
//!   planner switches the rover correction off);
//! - speed = flat_mps x exp(-k x tan(corrected slope)) (Tobler-shaped relative penalty; flat speed and k are ASSUMPTIONS);
//! - a cell is blocked if its corrected slope > limit_deg (ASSUMPTION), unless it is rover-verified: its centre lies within
//!   verified_r_m of a localized rover position;
//! - edges join the 8 neighbours of a 1 m grid; cost = step length x mean(1/speed) of the two cells; an edge touching a
//!   blocked cell does not exist;
//! - each leg = minimum walking time (Dijkstra); walk-back = Dijkstra from the exit over the whole grid.
//! Checked against the Python reference on the real bundle (tests/real_bundle.rs). No measured value is typed here.
pub mod corridor;
pub mod ensemble;
pub mod light;
pub mod sun;
#[cfg(feature = "json")]
pub mod json;

use std::cmp::Ordering;
use std::collections::BinaryHeap;

/// The routing assumptions (route.json "assumptions", plus the switch for the rover correction via bias_deg).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rules {
    pub flat_mps: f64,
    pub k: f64,
    pub bias_deg: f64,
    pub limit_deg: f64,
    pub verified_r_m: f64,
}

/// Per-cell costs for one set of rules. Row 0 is the north edge; cell (r, c) centre = ((c + 0.5) cell, (h - r - 0.5) cell).
pub struct CostGrid {
    pub w: usize,
    pub h: usize,
    pub cell: f64,
    /// corrected slope (deg), floored at 0
    pub corr: Vec<f64>,
    pub inv_speed: Vec<f64>,
    pub verified: Vec<bool>,
    pub blocked: Vec<bool>,
}

/// (row, col) of the cell holding map position (e, n), as tools/route.py to_cell.
pub fn to_cell(e: f64, n: f64, h: usize, cell: f64) -> (isize, isize) {
    (((h as f64 * cell - n) / cell).floor() as isize, (e / cell).floor() as isize)
}

impl CostGrid {
    pub fn new(slope_deg: &[f32], w: usize, h: usize, cell: f64, rules: &Rules, rover_en: &[(f64, f64)]) -> CostGrid {
        CostGrid::with_error(slope_deg, w, h, cell, rules, rover_en, None)
    }

    /// As `new`, with a per-cell slope error (degrees) subtracted together with the bias: corrected = max(0, slope - bias - err).
    /// Used by the uncertainty ensemble (ensemble.rs); `None` is the published rule.
    pub fn with_error(slope_deg: &[f32], w: usize, h: usize, cell: f64, rules: &Rules, rover_en: &[(f64, f64)],
                      err: Option<&[f64]>) -> CostGrid {
        assert_eq!(slope_deg.len(), w * h, "slope grid size");
        if let Some(e) = err {
            assert_eq!(e.len(), w * h, "error field size");
        }
        let mut verified = vec![false; w * h];
        let k = (rules.verified_r_m / cell).ceil() as isize + 1;
        let r2 = rules.verified_r_m * rules.verified_r_m;
        for &(e, n) in rover_en {
            let (r0, c0) = to_cell(e, n, h, cell);
            for r in (r0 - k).max(0)..(r0 + k + 1).min(h as isize) {
                for c in (c0 - k).max(0)..(c0 + k + 1).min(w as isize) {
                    let (ce, cn) = ((c as f64 + 0.5) * cell, (h as f64 - r as f64 - 0.5) * cell);
                    if (ce - e).powi(2) + (cn - n).powi(2) <= r2 {
                        verified[r as usize * w + c as usize] = true;
                    }
                }
            }
        }
        let corr: Vec<f64> = slope_deg.iter().enumerate()
            .map(|(i, &s)| (s as f64 - rules.bias_deg - err.map_or(0.0, |e| e[i])).max(0.0)).collect();
        let inv_speed = corr.iter().map(|c| 1.0 / (rules.flat_mps * (-rules.k * c.to_radians().tan()).exp())).collect();
        let blocked = corr.iter().zip(&verified).map(|(c, v)| *c > rules.limit_deg && !v).collect();
        CostGrid { w, h, cell, corr, inv_speed, verified, blocked }
    }

    pub fn centre(&self, i: usize) -> (f64, f64) {
        let (r, c) = (i / self.w, i % self.w);
        ((c as f64 + 0.5) * self.cell, (self.h as f64 - r as f64 - 0.5) * self.cell)
    }

    pub fn index(&self, e: f64, n: f64) -> Option<usize> {
        let (r, c) = to_cell(e, n, self.h, self.cell);
        (r >= 0 && c >= 0 && (r as usize) < self.h && (c as usize) < self.w).then(|| r as usize * self.w + c as usize)
    }

    /// Minimum walking time (s) from `src` to every cell (infinity = unreachable), and each cell's predecessor.
    /// With `target`, stops as soon as the target's time is final (times of cells not yet settled are then upper bounds).
    pub fn dijkstra(&self, src: usize, target: Option<usize>) -> (Vec<f64>, Vec<u32>) {
        let n = self.w * self.h;
        let mut dist = vec![f64::INFINITY; n];
        let mut pred = vec![u32::MAX; n];
        let mut heap = BinaryHeap::new();
        dist[src] = 0.0;
        heap.push(Item(0.0, src));
        const NB: [(isize, isize); 8] = [(-1, -1), (-1, 0), (-1, 1), (0, -1), (0, 1), (1, -1), (1, 0), (1, 1)];
        let diag = self.cell * std::f64::consts::SQRT_2;
        while let Some(Item(d, i)) = heap.pop() {
            if d > dist[i] {
                continue;
            }
            if Some(i) == target {
                break;
            }
            if self.blocked[i] {
                continue; // no edges touch a blocked cell
            }
            let (r, c) = ((i / self.w) as isize, (i % self.w) as isize);
            for (dr, dc) in NB {
                let (rr, cc) = (r + dr, c + dc);
                if rr < 0 || cc < 0 || rr >= self.h as isize || cc >= self.w as isize {
                    continue;
                }
                let j = rr as usize * self.w + cc as usize;
                if self.blocked[j] {
                    continue;
                }
                let step = if dr != 0 && dc != 0 { diag } else { self.cell };
                let nd = d + step * 0.5 * (self.inv_speed[i] + self.inv_speed[j]);
                if nd < dist[j] {
                    dist[j] = nd;
                    pred[j] = i as u32;
                    heap.push(Item(nd, j));
                }
            }
        }
        (dist, pred)
    }
}

/// Min-heap item (time, cell).
struct Item(f64, usize);
impl PartialEq for Item {
    fn eq(&self, o: &Self) -> bool {
        self.0 == o.0
    }
}
impl Eq for Item {}
impl PartialOrd for Item {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Item {
    fn cmp(&self, o: &Self) -> Ordering {
        o.0.total_cmp(&self.0).then_with(|| o.1.cmp(&self.1))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Leg {
    pub from: usize,
    pub to: usize,
    pub possible: bool,
    pub length_m: f64,
    pub time_s: f64,
    pub max_corrected_slope_deg: f64,
    pub share_in_rover_verified: f64,
    pub cells_over_limit_inside_verified: usize,
    /// first and last index of this leg in Plan::polyline_en
    pub poly_start: usize,
    pub poly_end: usize,
}

#[derive(Debug, Clone)]
pub struct Plan {
    pub legs: Vec<Leg>,
    /// cell centres along the whole loop (e, n), legs joined at their shared stop
    pub polyline_en: Vec<(f64, f64)>,
    /// time back to the exit (stop 0) from every cell; infinity = no way back under the rules
    pub walk_back: Vec<f64>,
    /// walk-back time at each stop (None = no way back)
    pub walk_back_stops: Vec<Option<f64>>,
    pub blocked_cells: usize,
    pub verified_cells: usize,
    pub total_length_m: f64,
    pub total_time_s: f64,
    pub all_legs_possible: bool,
}

fn path(pred: &[u32], src: usize, dst: usize) -> Option<Vec<usize>> {
    let mut out = vec![dst];
    let mut k = dst;
    while k != src {
        let p = pred[k];
        if p == u32::MAX {
            return None;
        }
        k = p as usize;
        out.push(k);
    }
    out.reverse();
    Some(out)
}

/// Plan the loop through `stops` (map metres, first = exit) and back, as tools/route.py plan_loop.
pub fn plan_loop(g: &CostGrid, stops: &[(f64, f64)], limit_deg: f64) -> Plan {
    plan_loop_opts(g, stops, limit_deg, true)
}

/// The route only: legs, polyline and totals, without the walk-back field (a Dijkstra over the whole grid, about
/// three quarters of a plan's time on the real bundle: 66 of 85 ms natively). Used by the robustness check,
/// which only compares routes. `walk_back` is then empty and `walk_back_stops` all None.
pub fn plan_route(g: &CostGrid, stops: &[(f64, f64)], limit_deg: f64) -> Plan {
    plan_loop_opts(g, stops, limit_deg, false)
}

fn plan_loop_opts(g: &CostGrid, stops: &[(f64, f64)], limit_deg: f64, with_walk_back: bool) -> Plan {
    let ids: Vec<Option<usize>> = stops.iter().map(|&(e, n)| g.index(e, n)).collect();
    let m = stops.len();
    let mut legs = Vec::new();
    let mut poly: Vec<(f64, f64)> = Vec::new();
    for i in 0..m {
        let (a, b) = (ids[i], ids[(i + 1) % m]);
        let mut leg = Leg { from: i, to: (i + 1) % m, possible: false, length_m: 0.0, time_s: 0.0, max_corrected_slope_deg: 0.0,
                            share_in_rover_verified: 0.0, cells_over_limit_inside_verified: 0, poly_start: 0, poly_end: 0 };
        if let (Some(a), Some(b)) = (a, b) {
            let (dist, pred) = g.dijkstra(a, Some(b));
            if dist[b].is_finite() {
                if let Some(p) = path(&pred, a, b) {
                    let pts: Vec<(f64, f64)> = p.iter().map(|&k| g.centre(k)).collect();
                    leg.possible = true;
                    leg.time_s = dist[b];
                    leg.length_m = pts.windows(2).map(|w| ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt()).sum();
                    leg.max_corrected_slope_deg = p.iter().map(|&k| g.corr[k]).fold(0.0, f64::max);
                    leg.share_in_rover_verified = p.iter().filter(|&&k| g.verified[k]).count() as f64 / p.len() as f64;
                    leg.cells_over_limit_inside_verified = p.iter().filter(|&&k| g.corr[k] > limit_deg).count();
                    leg.poly_start = if poly.is_empty() { 0 } else { poly.len() - 1 };
                    if poly.is_empty() {
                        poly.extend(pts);
                    } else {
                        poly.extend(pts.into_iter().skip(1));
                    }
                    leg.poly_end = poly.len() - 1;
                }
            }
        }
        legs.push(leg);
    }
    let walk_back = match (with_walk_back, ids.first().copied().flatten()) {
        (false, _) => Vec::new(),
        (true, Some(exit)) => g.dijkstra(exit, None).0,
        (true, None) => vec![f64::INFINITY; g.w * g.h],
    };
    let walk_back_stops = ids.iter().map(|id| id.and_then(|k| walk_back.get(k).copied()).filter(|t| t.is_finite())).collect();
    let ok: Vec<&Leg> = legs.iter().filter(|l| l.possible).collect();
    Plan {
        total_length_m: ok.iter().map(|l| l.length_m).sum(),
        total_time_s: ok.iter().map(|l| l.time_s).sum(),
        all_legs_possible: legs.iter().all(|l| l.possible),
        blocked_cells: g.blocked.iter().filter(|&&b| b).count(),
        verified_cells: g.verified.iter().filter(|&&v| v).count(),
        legs,
        polyline_en: poly,
        walk_back,
        walk_back_stops,
    }
}

/// Walking time (s) between every pair of stops under these rules (infinity = no way between them). Symmetric costs, so
/// one search per stop, each stopping as soon as every other stop is settled.
pub fn times_between(g: &CostGrid, stops: &[(f64, f64)]) -> Vec<Vec<f64>> {
    let ids: Vec<Option<usize>> = stops.iter().map(|&(e, n)| g.index(e, n)).collect();
    let m = stops.len();
    let mut t = vec![vec![f64::INFINITY; m]; m];
    for i in 0..m {
        t[i][i] = 0.0;
        let Some(src) = ids[i] else { continue };
        let targets: Vec<usize> = ids.iter().enumerate().filter(|(j, _)| *j != i).filter_map(|(_, x)| *x).collect();
        let dist = g.times_to(src, &targets);
        for j in 0..m {
            if let Some(k) = ids[j] {
                if j != i {
                    t[i][j] = dist[k];
                }
            }
        }
    }
    t
}

impl CostGrid {
    /// Dijkstra from `src` that stops once every cell in `targets` is settled (their times are then final).
    pub fn times_to(&self, src: usize, targets: &[usize]) -> Vec<f64> {
        let n = self.w * self.h;
        let mut dist = vec![f64::INFINITY; n];
        let mut settled = vec![false; n];
        let mut left: std::collections::HashSet<usize> = targets.iter().copied().collect();
        let mut heap = BinaryHeap::new();
        dist[src] = 0.0;
        heap.push(Item(0.0, src));
        const NB: [(isize, isize); 8] = [(-1, -1), (-1, 0), (-1, 1), (0, -1), (0, 1), (1, -1), (1, 0), (1, 1)];
        let diag = self.cell * std::f64::consts::SQRT_2;
        while let Some(Item(d, i)) = heap.pop() {
            if settled[i] || d > dist[i] {
                continue;
            }
            settled[i] = true;
            left.remove(&i);
            if left.is_empty() {
                break;
            }
            if self.blocked[i] {
                continue;
            }
            let (r, c) = ((i / self.w) as isize, (i % self.w) as isize);
            for (dr, dc) in NB {
                let (rr, cc) = (r + dr, c + dc);
                if rr < 0 || cc < 0 || rr >= self.h as isize || cc >= self.w as isize {
                    continue;
                }
                let j = rr as usize * self.w + cc as usize;
                if self.blocked[j] {
                    continue;
                }
                let step = if dr != 0 && dc != 0 { diag } else { self.cell };
                let nd = d + step * 0.5 * (self.inv_speed[i] + self.inv_speed[j]);
                if nd < dist[j] {
                    dist[j] = nd;
                    heap.push(Item(nd, j));
                }
            }
        }
        dist
    }
}

/// Largest number of stops the exact tour search handles (2^(n-1) x n states).
pub const MAX_TOUR_STOPS: usize = 12;

/// Exact shortest closed tours from stop 0 (Held-Karp): for every set of stops, the least total time to start at stop 0,
/// visit them all and return to stop 0, and the order that achieves it.
pub struct Tours {
    n: usize,
    /// dp[mask * n + j]: least time from 0 through the stops in `mask` (bit k = stop k + 1), ending at stop j (j >= 1)
    dp: Vec<f64>,
    parent: Vec<u8>,
    t: Vec<Vec<f64>>,
}

impl Tours {
    pub fn new(times: &[Vec<f64>]) -> Option<Tours> {
        let n = times.len();
        if n == 0 || n > MAX_TOUR_STOPS {
            return None;
        }
        let k = n - 1; // stops other than the exit
        let full = 1usize << k;
        let mut dp = vec![f64::INFINITY; full * n];
        let mut parent = vec![u8::MAX; full * n];
        for j in 1..n {
            dp[(1 << (j - 1)) * n + j] = times[0][j];
            parent[(1 << (j - 1)) * n + j] = 0;
        }
        for mask in 1..full {
            for j in 1..n {
                let bj = 1 << (j - 1);
                if mask & bj == 0 {
                    continue;
                }
                let cur = dp[mask * n + j];
                if !cur.is_finite() {
                    continue;
                }
                for x in 1..n {
                    let bx = 1 << (x - 1);
                    if mask & bx != 0 {
                        continue;
                    }
                    let nm = mask | bx;
                    let v = cur + times[j][x];
                    if v < dp[nm * n + x] {
                        dp[nm * n + x] = v;
                        parent[nm * n + x] = j as u8;
                    }
                }
            }
        }
        Some(Tours { n, dp, parent, t: times.to_vec() })
    }

    /// Best closed tour over exactly the stops in `mask` (bit k = stop k + 1): (total time, order starting with 0).
    pub fn best(&self, mask: usize) -> Option<(f64, Vec<usize>)> {
        let n = self.n;
        if mask == 0 {
            return Some((0.0, vec![0]));
        }
        let (mut best, mut end) = (f64::INFINITY, 0);
        for j in 1..n {
            if mask & (1 << (j - 1)) != 0 {
                let v = self.dp[mask * n + j] + self.t[j][0];
                if v < best {
                    best = v;
                    end = j;
                }
            }
        }
        if !best.is_finite() {
            return None;
        }
        let mut order = vec![];
        let (mut m, mut j) = (mask, end);
        while j != 0 {
            order.push(j);
            let p = self.parent[m * n + j] as usize;
            m &= !(1 << (j - 1));
            j = p;
        }
        order.push(0);
        order.reverse();
        Some((best, order))
    }

    /// Best order visiting every stop.
    pub fn best_all(&self) -> Option<(f64, Vec<usize>)> {
        self.best((1 << (self.n - 1)) - 1)
    }

    /// The most stops that fit in `budget_s` (must-visit stops always included; bit k = stop k + 1), shortest time on ties.
    pub fn best_within(&self, budget_s: f64, must_mask: usize) -> Option<(f64, Vec<usize>)> {
        let full = 1usize << (self.n - 1);
        let mut pick: Option<(u32, f64, usize)> = None;
        for mask in 0..full {
            if mask & must_mask != must_mask {
                continue;
            }
            if let Some((t, _)) = self.best(mask) {
                if t <= budget_s {
                    let c = mask.count_ones();
                    if pick.is_none_or(|(pc, pt, _)| c > pc || (c == pc && t < pt)) {
                        pick = Some((c, t, mask));
                    }
                }
            }
        }
        pick.and_then(|(_, _, mask)| self.best(mask))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(bias: f64) -> Rules {
        Rules { flat_mps: 0.8, k: 3.5, bias_deg: bias, limit_deg: 20.0, verified_r_m: 3.0 }
    }

    /// Same cases as `python tools/route.py selftest`.
    #[test]
    fn wall_blocks_verified_zone_opens_it_and_flat_time_is_distance_over_speed() {
        let (w, h) = (60, 60);
        let mut s = vec![5.0f32; w * h];
        for r in 0..h {
            s[r * w + 30] = 40.0; // a steep wall across the grid at column 30
        }
        let stops = [(10.5, 30.5), (50.5, 30.5)];
        let g = CostGrid::new(&s, w, h, 1.0, &rules(3.0), &[]);
        let p = plan_loop(&g, &stops, 20.0);
        assert!(!p.all_legs_possible, "wall must block without a verified zone");
        assert!(!p.walk_back[31..w].iter().any(|t| t.is_finite()), "no way back across the wall");
        let g = CostGrid::new(&s, w, h, 1.0, &rules(3.0), &[(30.5, 30.5)]);
        let p = plan_loop(&g, &stops, 20.0);
        assert!(p.all_legs_possible, "verified zone must open the wall");
        let l = &p.legs[0];
        assert!(l.cells_over_limit_inside_verified >= 1 && (40.0..=45.0).contains(&l.length_m), "{l:?}");
        let flat = CostGrid::new(&vec![3.0f32; w * h], w, h, 1.0, &rules(3.0), &[]);
        let p = plan_loop(&flat, &stops, 20.0);
        assert!((p.legs[0].time_s - 40.0 / 0.8).abs() < 1e-6, "{:?}", p.legs[0]);
        assert_eq!(p.walk_back_stops[1], Some(p.legs[1].time_s));
        assert_eq!(p.polyline_en[p.legs[0].poly_start], (10.5, 30.5));
        assert_eq!(p.polyline_en[p.legs[0].poly_end], (50.5, 30.5));
    }

    #[test]
    fn tours_are_exact_and_budget_picks_the_most_stops() {
        // 5 stops on a line at 0, 40, 10, 30, 20 (times = distances); exit = stop 0 at position 0
        let pos = [0.0f64, 40.0, 10.0, 30.0, 20.0];
        let t: Vec<Vec<f64>> = pos.iter().map(|a| pos.iter().map(|b| (a - b).abs()).collect()).collect();
        let tours = Tours::new(&t).unwrap();
        let (best, order) = tours.best_all().unwrap();
        assert_eq!(best, 80.0, "out to 40 and back");
        let visited: Vec<f64> = order.iter().map(|&i| pos[i]).collect();
        let monotone = |v: &[f64]| v.windows(2).all(|w| w[1] >= w[0]) || v.windows(2).all(|w| w[1] <= w[0]);
        assert!(monotone(&visited[1..]), "visits along the line: {visited:?}");
        // budget 45 s: most stops is {10, 20} (tour 40 s); must-visit stop 3 (at 30) forces 60 s > 45 s
        let (tb, ob) = tours.best_within(45.0, 0).unwrap();
        assert_eq!((tb, ob.len()), (40.0, 3), "{ob:?}");
        assert!(tours.best_within(45.0, 1 << (3 - 1)).is_none(), "must-visit at 30 does not fit in 45 s");
    }

    #[test]
    fn times_between_matches_leg_times() {
        let flat = CostGrid::new(&vec![3.0f32; 60 * 60], 60, 60, 1.0, &rules(3.0), &[]);
        let stops = [(10.5, 30.5), (50.5, 30.5), (30.5, 10.5)];
        let t = times_between(&flat, &stops);
        let p = plan_loop(&flat, &stops, 20.0);
        assert!((t[0][1] - p.legs[0].time_s).abs() < 1e-9 && (t[1][2] - p.legs[1].time_s).abs() < 1e-9);
        assert!((t[0][1] - t[1][0]).abs() < 1e-9, "symmetric");
    }

    #[test]
    fn rover_correction_off_blocks_more_ground() {
        let s: Vec<f32> = (0..400).map(|i| 18.0 + (i % 7) as f32).collect(); // 18..24 deg
        let on = CostGrid::new(&s, 20, 20, 1.0, &rules(3.4), &[]);
        let off = CostGrid::new(&s, 20, 20, 1.0, &rules(0.0), &[]);
        assert!(off.blocked.iter().filter(|&&b| b).count() > on.blocked.iter().filter(|&&b| b).count());
    }
}
