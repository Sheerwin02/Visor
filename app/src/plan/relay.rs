//! Relay / EV2 position suggestion for the stretches of the route out of sight of the exit. Background: the D-RATS 2022
//! quicklook report notes terrain "can easily block line-of-sight communications", and Desert RATS used a relay crew
//! member not otherwise on the EVA. A person or antenna standing at the relay point sees the exit (so it can pass
//! messages) and some of the route the exit cannot see.
//! Rule (fixed before any result): candidates on a RELAY_STEP_M grid, reachable within RELAY_MAX_BACK_S of the exit (the
//! live walk-back field), keep the highest RELAY_HIGH_SHARE of them (high ground sees more), must see the exit; score =
//! route length seen by the exit or the relay; best score wins, ties to the shorter walk-back. Sight lines are the app's
//! terrain-only test (panel::sight_line, eyes 1.7 m both ends): the 1 m map misses the 1-2 m ridges, so real coverage is
//! likely lower, and the card says so.
use crate::scene::terrain::Grid;
use crate::ui::panel::{sight_line, Sample};

pub const RELAY_STEP_M: f32 = 10.0;
pub const RELAY_MAX_BACK_S: f32 = 360.0;
pub const RELAY_HIGH_SHARE: f32 = 0.15;

#[derive(Clone, Debug, PartialEq)]
pub struct Relay {
    pub e: f32,
    pub n: f32,
    /// share of the route length in sight of the exit, before and with the relay
    pub before: f32,
    pub after: f32,
    /// walk-back from the relay point to the exit (s), straight-line distance from the exit (m)
    pub back_s: f32,
    pub from_exit_m: f32,
    /// stretches still out of sight with the relay (for the map)
    pub hidden_after: Vec<Vec<(f32, f32)>>,
}

/// Best relay point, or None when no candidate adds sight of the route.
pub fn suggest(g: &Grid, samples: &[Sample], back: &dyn Fn(f32, f32) -> Option<f32>) -> Option<Relay> {
    suggest_on(g, samples, back, RELAY_STEP_M, RELAY_HIGH_SHARE)
}

/// `suggest` with the candidate grid step (m) and the share of highest candidates kept as parameters (tests use a fine grid).
pub fn suggest_on(g: &Grid, samples: &[Sample], back: &dyn Fn(f32, f32) -> Option<f32>, step: f32, high_share: f32) -> Option<Relay> {
    let (ex, ey) = samples.first().map(|s| (s.e, s.n))?;
    let len = |i: usize| if i == 0 { 0.0 } else { samples[i].d - samples[i - 1].d };
    let total: f32 = samples.last().map(|s| s.d).unwrap_or(0.0).max(1e-3);
    let before: f32 = (0..samples.len()).filter(|&i| samples[i].seen).map(len).sum::<f32>() / total;
    let hidden: Vec<usize> = (0..samples.len()).filter(|&i| !samples[i].seen).collect();
    if hidden.is_empty() {
        return None;
    }
    // candidates: reachable within RELAY_MAX_BACK_S, the highest share of them, in sight of the exit
    let (w_m, h_m) = (g.w as f32 * g.cell, g.h as f32 * g.cell);
    let mut cands: Vec<(f32, f32, f32, f32)> = Vec::new(); // e, n, height, back
    let mut e = step / 2.0;
    while e < w_m {
        let mut n = step / 2.0;
        while n < h_m {
            if let Some(b) = back(e, n).filter(|b| *b <= RELAY_MAX_BACK_S) {
                cands.push((e, n, g.height_at(e, n), b));
            }
            n += step;
        }
        e += step;
    }
    cands.sort_by(|a, b| b.2.total_cmp(&a.2));
    cands.truncate(((cands.len() as f32 * high_share).ceil() as usize).max(1));
    let mut best: Option<(f32, f32, (f32, f32, f32, f32))> = None; // seen length, back, candidate
    for c in cands.into_iter().filter(|c| sight_line(g, ex, ey, c.0, c.1)) {
        let gained: f32 = hidden.iter().filter(|&&i| sight_line(g, c.0, c.1, samples[i].e, samples[i].n)).map(|&i| len(i)).sum();
        if gained > 0.0 && best.is_none_or(|b| gained > b.0 + 1e-3 || ((gained - b.0).abs() <= 1e-3 && c.3 < b.1)) {
            best = Some((gained, c.3, c));
        }
    }
    let (gained, _, c) = best?;
    Some(Relay { e: c.0, n: c.1, before, after: (before + gained / total).min(1.0), back_s: c.3,
                 from_exit_m: ((c.0 - ex).powi(2) + (c.1 - ey).powi(2)).sqrt(), hidden_after: hidden_runs_with(g, samples, c.0, c.1) })
}

/// Route stretches still out of sight of the exit with a relay at (e, n), as runs for the map.
fn hidden_runs_with(g: &Grid, samples: &[Sample], e: f32, n: f32) -> Vec<Vec<(f32, f32)>> {
    let mut runs: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut cur: Vec<(f32, f32)> = Vec::new();
    for i in 1..samples.len() {
        let vis = samples[i].seen || sight_line(g, e, n, samples[i].e, samples[i].n);
        if !vis {
            if cur.is_empty() {
                cur.push((samples[i - 1].e, samples[i - 1].n));
            }
            cur.push((samples[i].e, samples[i].n));
        } else if !cur.is_empty() {
            runs.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        runs.push(cur);
    }
    runs
}

/// A relay the planner placed by hand (or the suggestion, once placed), scored with the same terrain-only sight test.
#[derive(Clone, Debug, PartialEq)]
pub struct RelayAt {
    /// share of the route length in sight of the exit, and in contact (exit, or a relay that itself sees the exit)
    pub before: f32,
    pub after: f32,
    /// the relay spot sees the exit (else it cannot pass messages and adds no contact)
    pub sees_exit: bool,
    /// walk-back from the spot under the slope rule (None = the exit cannot be reached from it), straight-line metres
    pub back_s: Option<f32>,
    pub from_exit_m: f32,
    pub hidden_after: Vec<Vec<(f32, f32)>>,
}

pub fn evaluate(g: &Grid, samples: &[Sample], back: &dyn Fn(f32, f32) -> Option<f32>, e: f32, n: f32) -> Option<RelayAt> {
    let (ex, ey) = samples.first().map(|s| (s.e, s.n))?;
    let len = |i: usize| if i == 0 { 0.0 } else { samples[i].d - samples[i - 1].d };
    let total: f32 = samples.last().map(|s| s.d).unwrap_or(0.0).max(1e-3);
    let before: f32 = (0..samples.len()).filter(|&i| samples[i].seen).map(len).sum::<f32>() / total;
    let sees_exit = sight_line(g, ex, ey, e, n);
    let gained: f32 = if sees_exit {
        (0..samples.len()).filter(|&i| !samples[i].seen && sight_line(g, e, n, samples[i].e, samples[i].n)).map(len).sum()
    } else { 0.0 };
    let hidden_after = if sees_exit { hidden_runs_with(g, samples, e, n) } else { hidden_runs_with(g, samples, ex, ey) };
    Some(RelayAt { before, after: (before + gained / total).min(1.0), sees_exit, back_s: back(e, n),
                   from_exit_m: ((e - ex).powi(2) + (n - ey).powi(2)).sqrt(), hidden_after })
}

/// Blocked plan fix: the nearest spot to (e, n), within `max_r` m, that the exit can be reached from under the rules (the
/// live walk-back field is finite there). Rings 1 m apart, 48 directions each: (east, north, distance m).
pub fn nearest_reachable(back: &dyn Fn(f32, f32) -> Option<f32>, e: f32, n: f32, max_r: f32) -> Option<(f32, f32, f32)> {
    if back(e, n).is_some() {
        return None;
    }
    let mut r = 1.0;
    while r <= max_r {
        let best = (0..48).filter_map(|k| {
            let a = k as f32 / 48.0 * std::f32::consts::TAU;
            let (x, y) = (e + r * a.sin(), n + r * a.cos());
            back(x, y).map(|t| (x, y, t))
        }).min_by(|a, b| a.2.total_cmp(&b.2));
        if let Some((x, y, _)) = best {
            return Some((x, y, r));
        }
        r += 1.0;
    }
    None
}

/// Steep-target fix: the gentlest map ground RING_MIN_M..RING_MAX_M from a station (about within reach of its outcrop),
/// by corrected slope; ties to the nearer spot. None unless it is at least 2 deg gentler than the map at the target.
/// Returns (east, north, distance m, corrected slope deg). The station itself does not move (the science target stays).
pub fn work_spot(slope: &Grid, bias: f32, e: f32, n: f32) -> Option<(f32, f32, f32, f32)> {
    let here = slope.height_at(e, n) - bias;
    let mut best: Option<(f32, f32, f32, f32)> = None;
    let mut r = WORK_MIN_M;
    while r <= WORK_MAX_M + 1e-3 {
        for k in 0..24 {
            let a = k as f32 / 24.0 * std::f32::consts::TAU;
            let (x, y) = (e + r * a.sin(), n + r * a.cos());
            if x < 0.0 || y < 0.0 || x > slope.w as f32 * slope.cell || y > slope.h as f32 * slope.cell {
                continue;
            }
            let s = (slope.height_at(x, y) - bias).max(0.0);
            if best.is_none_or(|b| s < b.3 - 0.1) {
                best = Some((x, y, r, s));
            }
        }
        r += 0.5;
    }
    best.filter(|b| b.3 <= here - 2.0)
}

/// Working-spot search ring (m from the station): about the reach of a crew member working at an outcrop. VISUAL.
pub const WORK_MIN_M: f32 = 2.0;
pub const WORK_MAX_M: f32 = 6.0;

/// How far (m) to look for reachable ground around a blocked stop. VISUAL / usability choice.
pub const FIX_MAX_R_M: f32 = 40.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_spot_finds_the_gentle_side_within_reach() {
        // slope 20 deg everywhere except a 5 deg strip east of e = 53
        let (w, h) = (100usize, 100usize);
        let z: Vec<f32> = (0..w * h).map(|i| if (i % w) as f32 + 0.5 > 53.0 { 5.0 } else { 20.0 }).collect();
        let g = Grid { w, h, cell: 1.0, z, top: 20.0 };
        let (x, _, d, s) = work_spot(&g, 0.0, 50.0, 50.0).expect("a gentler spot");
        assert!(x > 53.0 && (WORK_MIN_M..=WORK_MAX_M).contains(&d) && s < 6.0);
        let flat = Grid { w, h, cell: 1.0, z: vec![10.0; w * h], top: 10.0 };
        assert!(work_spot(&flat, 0.0, 50.0, 50.0).is_none(), "nothing gentler nearby: no suggestion");
    }

    #[test]
    fn nearest_reachable_ground_is_found_outside_a_blocked_patch() {
        // blocked within 5 m of (50, 50)
        let back = |e: f32, n: f32| if ((e - 50.0).powi(2) + (n - 50.0).powi(2)).sqrt() < 5.0 { None } else { Some(100.0) };
        let (x, y, d) = nearest_reachable(&back, 50.0, 50.0, 40.0).unwrap();
        assert!((5.0..=6.0).contains(&d) && ((x - 50.0).hypot(y - 50.0) - d).abs() < 1e-3);
        assert!(nearest_reachable(&back, 80.0, 80.0, 40.0).is_none(), "already reachable: nothing to fix");
        assert!(nearest_reachable(&|_, _| None, 0.0, 0.0, 10.0).is_none(), "nothing reachable in range");
    }

    #[test]
    fn a_hill_beside_a_wall_sees_round_it() {
        // 40 x 20 m flat; a 5 m wall at e 18..20 for n < 15 hides the route's east half from the exit at (2, 5); a 10 m hill
        // at (10, 18) north of the wall's end sees both the exit and the far side
        let (w, h) = (40usize, 20usize);
        let mut z = vec![0.0f32; w * h];
        for r in 0..h {
            for c in 0..w {
                let n = (h - 1 - r) as f32 + 0.5;
                if (18..20).contains(&c) && n < 15.0 {
                    z[r * w + c] = 5.0;
                }
                if (9..12).contains(&c) && n > 16.5 {
                    z[r * w + c] = 10.0;
                }
            }
        }
        let g = Grid { w, h, cell: 1.0, z, top: 10.0 };
        let mut samples = Vec::new();
        for k in 0..=36 {
            let (e, n) = (2.0 + k as f32, 5.0);
            samples.push(Sample { d: k as f32, e, n, z: 0.0, s: 0.0, seen: sight_line(&g, 2.0, 5.0, e, n) });
        }
        assert!(samples.iter().any(|s| !s.seen), "the wall hides part of the route");
        let r = suggest_on(&g, &samples, &|_, _| Some(60.0), 1.0, 0.1).expect("a relay point");
        assert!(r.after > r.before, "{r:?}");
        assert!(g.height_at(r.e, r.n) > 0.0, "the relay stands on raised ground (the wall top or the hill): {r:?}");
        assert!(sight_line(&g, 2.0, 5.0, r.e, r.n), "the relay sees the exit");
        // the same spot scored by hand gives the same coverage; a spot behind the wall that cannot see the exit adds none
        let at = evaluate(&g, &samples, &|_, _| Some(60.0), r.e, r.n).unwrap();
        assert!(at.sees_exit && (at.after - r.after).abs() < 1e-4 && (at.before - r.before).abs() < 1e-4, "{at:?}");
        let blind = evaluate(&g, &samples, &|_, _| None, 30.0, 5.0).unwrap();
        assert!(!blind.sees_exit && blind.after == blind.before && blind.back_s.is_none(), "{blind:?}");
        // nothing hidden: no suggestion
        let all_seen: Vec<Sample> = samples.iter().map(|s| Sample { seen: true, ..*s }).collect();
        assert!(suggest_on(&g, &all_seen, &|_, _| Some(60.0), 1.0, 0.1).is_none());
        // nothing reachable: no suggestion
        assert!(suggest_on(&g, &samples, &|_, _| None, 1.0, 0.1).is_none());
    }
}
