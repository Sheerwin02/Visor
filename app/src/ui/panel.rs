//! Planning panel logic (Plan mode, left side): summary, height/slope profile, leg rows, all pure and unit-tested. `model`
//! turns a plan into the PanelModel that the Leptos panel (ui/views) renders. Every number comes from the bundle or the live
//! plan; every word from text/en.json. Slope bands use trust::band, the same function as the map's hatching.
use std::sync::Mutex;

use serde_json::Value;

use crate::ui::model::{Controls, LegRow, PanelModel};
use crate::scene::terrain::Grid;
use crate::scene::trust::band;

/// Map point under the pointer on the profile (shown as a ring on the map by pin.rs), and a click request to centre on.
pub static HOVER: Mutex<Option<(f32, f32)>> = Mutex::new(None);
pub static GOTO: Mutex<Option<(f32, f32)>> = Mutex::new(None);

#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub feasible: bool,
    pub legs: usize,
    pub length_m: f64,
    pub walk_s: f64,
    /// steepest corrected slope on a walkable leg, and that leg's index
    pub worst_deg: f64,
    pub worst_leg: usize,
    pub over_limit_cells: i64,
    /// share of the walkable route length on rover-verified ground
    pub verified_share: f64,
    /// longest walk-back time (s) over the stops, and that stop's index
    pub longest_back_s: f64,
    pub longest_back_stop: usize,
}

pub fn summary(route: &Value, stops: &[Value]) -> Summary {
    let legs = route["legs"].as_array().cloned().unwrap_or_default();
    let ok: Vec<(usize, &Value)> = legs.iter().enumerate().filter(|(_, l)| l["possible"].as_bool() == Some(true)).collect();
    let f = |v: &Value, k: &str| v[k].as_f64().unwrap_or(0.0);
    let (mut worst_deg, mut worst_leg) = (-1.0, 0);
    for (i, l) in &ok {
        if f(l, "max_corrected_slope_deg") > worst_deg {
            worst_deg = f(l, "max_corrected_slope_deg");
            worst_leg = *i;
        }
    }
    let len: f64 = ok.iter().map(|(_, l)| f(l, "length_m")).sum();
    let verified = if len > 0.0 { ok.iter().map(|(_, l)| f(l, "share_in_rover_verified") * f(l, "length_m")).sum::<f64>() / len } else { 0.0 };
    let (mut longest_back_s, mut longest_back_stop) = (-1.0, 0);
    for (i, s) in stops.iter().enumerate() {
        if let Some(b) = s["walk_back_s"].as_f64() {
            if b > longest_back_s {
                longest_back_s = b;
                longest_back_stop = i;
            }
        }
    }
    Summary {
        feasible: route["all_legs_possible"].as_bool().unwrap_or(false),
        legs: legs.len(),
        length_m: f(route, "total_length_m"),
        walk_s: f(route, "total_time_s"),
        worst_deg,
        worst_leg,
        over_limit_cells: ok.iter().map(|(_, l)| l["cells_over_limit_inside_verified"].as_i64().unwrap_or(0)).sum(),
        verified_share: verified,
        longest_back_s,
        longest_back_stop,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    /// metres along the loop
    pub d: f32,
    pub e: f32,
    pub n: f32,
    /// height above the loop's lowest point (m)
    pub z: f32,
    /// corrected slope: map slope at 3 m scale minus the bias, floored at 0 (deg)
    pub s: f32,
    /// a person at the exit (eye 1.7 m) can see a person here (eye 1.7 m) over the terrain (sight_line)
    pub seen: bool,
}

/// Line of sight between two people standing at (e0, n0) and (e1, n1), eyes walk::EYE_M above the terrain (the app's
/// eye-height ASSUMPTION at both ends; no habitat or antenna height is assumed). Samples the straight line every half cell;
/// blocked where the terrain rises above it. Terrain only (the 1 m map does not resolve the 1-2 m ridges: real sight
/// lines may be shorter).
pub fn sight_line(g: &Grid, e0: f32, n0: f32, e1: f32, n1: f32) -> bool {
    let eye = crate::scene::walk::EYE_M;
    let (z0, z1) = (g.height_at(e0, n0) + eye, g.height_at(e1, n1) + eye);
    let len = ((e1 - e0).powi(2) + (n1 - n0).powi(2)).sqrt();
    let steps = (len / (g.cell * 0.5)).ceil() as usize;
    (1..steps).all(|k| {
        let f = k as f32 / steps as f32;
        g.height_at(e0 + (e1 - e0) * f, n0 + (n1 - n0) * f) <= z0 + (z1 - z0) * f
    })
}

/// One sample per route point (cell centres, so heights and slopes are exact grid values).
pub fn profile(poly: &[(f32, f32)], heights: &Grid, slope: &Grid, bias_deg: f32) -> Vec<Sample> {
    let mut out: Vec<Sample> = Vec::with_capacity(poly.len());
    let mut d = 0.0;
    for (i, &(e, n)) in poly.iter().enumerate() {
        if i > 0 {
            let (pe, pn) = poly[i - 1];
            d += ((e - pe).powi(2) + (n - pn).powi(2)).sqrt();
        }
        let seen = poly.first().is_none_or(|&(x0, y0)| sight_line(heights, x0, y0, e, n));
        out.push(Sample { d, e, n, z: heights.height_at(e, n), s: (slope.height_at(e, n) - bias_deg).max(0.0), seen });
    }
    let zmin = out.iter().map(|p| p.z).fold(f32::MAX, f32::min);
    for p in &mut out {
        p.z -= zmin;
    }
    out
}

/// Index of the sample nearest to distance `d` (samples sorted by d).
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn nearest(samples: &[Sample], d: f32) -> usize {
    match samples.binary_search_by(|p| p.d.total_cmp(&d)) {
        Ok(i) => i,
        Err(0) => 0,
        Err(i) if i >= samples.len() => samples.len() - 1,
        Err(i) => if (samples[i].d - d).abs() < (d - samples[i - 1].d).abs() { i } else { i - 1 },
    }
}

/// Contiguous runs of the same non-zero slope band along the profile: (band, d0, d1), split halfway between samples.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn band_runs(samples: &[Sample], caution: f32, limit: f32) -> Vec<(u8, f32, f32)> {
    let mut runs: Vec<(u8, f32, f32)> = Vec::new();
    for i in 0..samples.len() {
        let b = band(samples[i].s, caution, limit);
        let d0 = if i == 0 { 0.0 } else { (samples[i - 1].d + samples[i].d) / 2.0 };
        let d1 = if i + 1 == samples.len() { samples[i].d } else { (samples[i].d + samples[i + 1].d) / 2.0 };
        match runs.last_mut() {
            Some(last) if b > 0 && last.0 == b && (last.2 - d0).abs() < 1e-4 => last.2 = d1,
            _ if b > 0 => runs.push((b, d0, d1)),
            _ => {}
        }
    }
    runs
}

/// "Nice" axis step (1, 2, 5 x 10^k) giving at most `max_ticks` ticks over `span`.
#[cfg_attr(not(test), allow(dead_code))] // used by the tests; the timeline labels the lanes directly
pub fn nice_step(span: f32, max_ticks: usize) -> f32 {
    let raw = span / max_ticks.max(1) as f32;
    let p = 10f32.powi(raw.log10().floor() as i32);
    for m in [1.0, 2.0, 5.0, 10.0] {
        if m * p >= raw - 1e-6 {
            return m * p;
        }
    }
    10.0 * p
}

/// Build the panel model from the plan (route.json schema), the stops (with walk-back times), the manifest's measured values,
/// the profile and the planner settings. Pure: tested natively against the real bundle.
pub fn model(route: &Value, stops: &[Value], manifest: &Value, samples: Vec<Sample>, controls: Controls) -> PanelModel {
    use crate::ui::hud::minutes;
    use crate::scene::labels::short_name;
    let a = &route["assumptions"];
    let mv = &manifest["measured_values"];
    let num = |v: &Value| match v.as_f64() {
        Some(x) if x.fract() == 0.0 => format!("{}", x as i64),
        Some(x) => format!("{x}"),
        None => v.as_str().unwrap_or("").to_string(),
    };
    let vals: Vec<(&'static str, String)> = vec![
        ("speed_flat_mps", num(&a["speed_flat_mps"])),
        ("slope_limit_deg", num(&a["slope_limit_deg"])),
        ("caution_deg", num(&a["caution_deg"])),
        ("verified_radius_m", num(&a["verified_radius_m"])),
        ("rc_bias", num(&a["bias_deg"])),
        ("rc_median", num(&mv["rover_check"]["median_abs_deg"])),
        ("rc_n", num(&mv["rover_check"]["n"])),
        // display rounding only; the bundle keeps the exact values
        ("gate_patch_pct", format!("{:.1}", mv["gate"]["patch_share_pct"].as_f64().unwrap_or(f64::NAN))),
        ("gate_ref_pct", format!("{:.1}", mv["gate"]["reference_share_pct"].as_f64().unwrap_or(f64::NAN))),
        ("speed_src_lo", num(&a["speed_source_range_mps"][0])),
        ("speed_src_hi", num(&a["speed_source_range_mps"][1])),
    ];
    let mut vals = vals;
    // walking time over the speed options: every edge time scales as 1 / flat speed, so the range is exact
    let speeds = &controls.speeds;
    if let (Some(&vmin), Some(&vmax)) = (speeds.iter().min_by(|a, b| a.total_cmp(b)), speeds.iter().max_by(|a, b| a.total_cmp(b))) {
        let t_now = route["total_time_s"].as_f64().unwrap_or(0.0);
        let v_now = a["speed_flat_mps"].as_f64().unwrap_or(vmax);
        vals.push(("t_fast", minutes((t_now * v_now / vmax) as f32)));
        vals.push(("t_slow", minutes((t_now * v_now / vmin) as f32)));
        vals.push(("v_fast", format!("{vmax}")));
        vals.push(("v_slow", format!("{vmin}")));
    }
    let sm = summary(route, stops);
    let n = stops.len().max(1);
    let label = |i: usize| format!("{} {}", i % n + 1, short_name(stops.get(i % n).and_then(|x| x["name"].as_str()).unwrap_or("")));
    let legs = route["legs"].as_array().cloned().unwrap_or_default();
    let rows = legs
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let ok = l["possible"].as_bool() == Some(true);
            let f = |k: &str| l[k].as_f64().unwrap_or(0.0);
            let dash = "\u{2014}".to_string();
            LegRow {
                label: format!("{} \u{2192} {}", label(i), label(i + 1)),
                metres: if ok { format!("{}", f("length_m").round() as i64) } else { dash.clone() },
                minutes: if ok { minutes(f("time_s") as f32) } else { dash.clone() },
                max_slope: if ok { format!("{:.1}", f("max_corrected_slope_deg")) } else { dash.clone() },
                verified: if ok { format!("{}%", (f("share_in_rover_verified") * 100.0).round() as i64) } else { dash },
                corridor: String::new(),
                corridor_tip: String::new(),
                possible: ok,
            }
        })
        .collect();
    let ps = &mv["rover_check"]["per_stop"];
    PanelModel {
        feasible: sm.feasible,
        legs_n: sm.legs,
        walk_s: sm.walk_s,
        length_m: sm.length_m,
        back_s: sm.longest_back_s,
        back_stop: label(sm.longest_back_stop),
        worst_deg: sm.worst_deg,
        worst_leg: format!("{} \u{2192} {}", sm.worst_leg + 1, (sm.worst_leg + 1) % n + 1),
        over_limit_cells: sm.over_limit_cells,
        verified_pct: (sm.verified_share * 100.0).round() as i64,
        vals,
        samples,
        leg_starts: legs.iter().filter_map(|l| l["poly_start"].as_u64().map(|x| x as usize)).collect(),
        caution: a["caution_deg"].as_f64().unwrap_or(f64::NAN) as f32,
        limit: a["slope_limit_deg"].as_f64().unwrap_or(f64::NAN) as f32,
        legs: rows,
        stops: (0..stops.len()).map(label).collect(),
        controls,
        rover_counts: (num(&ps["agrees"]), num(&ps["partial"]), num(&ps["disagrees"])),
        range_note: String::new(),
        time_range: None,
        range_unsettled: false,
        fixes: Vec::new(),
        leg_secs: legs.iter().map(|l| if l["possible"].as_bool() == Some(true) { l["time_s"].as_f64().unwrap_or(0.0) } else { 0.0 }).collect(),
        leg_agree: Vec::new(),
        stop_cards: Vec::new(),
        runs: String::new(),
    }
}

/// p10 and p90 walking time (s) of the shown ensemble.
pub fn time_range(ens: &Value) -> Option<(f64, f64)> {
    let shown = ens["shown_correlation_length_m"].as_f64();
    let e = ens["ensembles"].as_array()?.iter().find(|e| e["correlation_length_m"].as_f64() == shown)?;
    Some((e["time_s"]["p10"].as_f64()?, e["time_s"]["p90"].as_f64()?))
}

/// The shown ensemble's percentiles had not settled (its "converged" is false).
pub fn range_unsettled(ens: &Value) -> bool {
    let shown = ens["shown_correlation_length_m"].as_f64();
    ens["ensembles"].as_array().and_then(|a| a.iter().find(|e| e["correlation_length_m"].as_f64() == shown))
        .is_some_and(|e| e["converged"].as_bool() == Some(false))
}

/// Per leg: share of the shown ensemble's runs that keep the leg within 3 m of the plan (corridor rule).
pub fn leg_agreement(ens: &Value) -> Vec<Option<f64>> {
    let shown = ens["shown_correlation_length_m"].as_f64();
    ens["corridor"]["ensembles"].as_array().and_then(|a| a.iter().find(|e| e["correlation_length_m"].as_f64() == shown))
        .and_then(|c| c["legs"].as_array())
        .map(|legs| legs.iter().map(|l| {
            let (a, n) = (l["agree_runs"].as_f64()?, l["runs"].as_f64()?);
            (n > 0.0).then(|| a / n)
        }).collect())
        .unwrap_or_default()
}

/// Corridor rule results for the shown ensemble: per leg (share of runs that keep the leg within 3 m, and its tooltip),
/// and one sentence for the whole loop. None without corridor results.
pub fn corridor(ens: &Value) -> Option<(Vec<(String, String)>, String)> {
    use crate::text::plan_panel as p;
    let shown = ens["shown_correlation_length_m"].as_f64();
    let c = ens["corridor"]["ensembles"].as_array()?.iter().find(|e| e["correlation_length_m"].as_f64() == shown)?;
    let pct = |a: u64, n: u64| if n == 0 { 0 } else { (a as f64 / n as f64 * 100.0).round() as i64 };
    let legs = c["legs"].as_array()?.iter().map(|l| {
        let (a, n) = (l["agree_runs"].as_u64().unwrap_or(0), l["runs"].as_u64().unwrap_or(0));
        (format!("{}%", pct(a, n)), crate::text::fill(p::CORRIDOR_TIP, &[("agree", a.to_string()), ("runs", n.to_string())]))
    }).collect();
    let lp = &c["loop"];
    let (a, n) = (lp["agree_runs"].as_u64().unwrap_or(0), lp["runs"].as_u64().unwrap_or(0));
    let med = lp["share_within_3m"]["p50"].as_f64().unwrap_or(f64::NAN);
    let note = crate::text::fill(p::CORRIDOR_NOTE, &[("agree", a.to_string()), ("runs", n.to_string()),
                                                     ("pct", format!("{}", (med * 100.0).round() as i64))]);
    Some((legs, note))
}

/// The uncertainty sentence under the time tile, from route_ensemble.json (the ensemble the app shows). Applies only to the
/// published plan; for other settings it says so instead of quoting the range.
pub fn range_note(ens: &Value, is_default: bool, budget_min: f64) -> String {
    use crate::ui::hud::minutes;
    use crate::text::plan_panel as p;
    let shown = ens["shown_correlation_length_m"].as_f64();
    let Some(e) = ens["ensembles"].as_array().and_then(|a| a.iter().find(|e| e["correlation_length_m"].as_f64() == shown)) else {
        return String::new();
    };
    let t = |k: &str| e["time_s"][k].as_f64().unwrap_or(f64::NAN);
    let runs = e["runs"].as_u64().unwrap_or(0).to_string();
    let vals = [("runs", runs.clone()), ("t_p10", minutes(t("p10") as f32)), ("t_p50", minutes(t("p50") as f32)),
                ("t_p90", minutes(t("p90") as f32))];
    let mut out = crate::text::fill(p::RANGE_NOTE, &vals);
    let spare_s = budget_min * 60.0 - t("p90");
    let b = format!("{budget_min}");
    out += " ";
    out += &if spare_s >= 0.0 {
        crate::text::fill(p::RANGE_SPARE, &[("spare", minutes(spare_s as f32)), ("budget", b)])
    } else {
        crate::text::fill(p::RANGE_OVER, &[("over", minutes(-spare_s as f32)), ("budget", b)])
    };
    if e["converged"].as_bool() == Some(false) {
        out += " ";
        out += &crate::text::fill(p::RANGE_UNSETTLED, &[("runs", runs)]);
    }
    if !is_default {
        out += " ";
        out += p::RANGE_OTHER;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(w: usize, h: usize, v: &[f32]) -> Grid {
        let b: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
        Grid::from_bytes(&b, w, h, 1.0).unwrap()
    }

    #[test]
    fn profile_samples_grid_cells_along_the_route() {
        // 4 x 3 grid, row 0 north; the route runs along the middle row
        let heights = grid(4, 3, &[1., 2., 3., 4., 5., 6., 7., 8., 9., 10., 11., 12.]);
        let slopes = grid(4, 3, &[0., 0., 0., 0., 10., 20., 30., 40., 0., 0., 0., 0.]);
        let p = profile(&[(0.5, 1.5), (1.5, 1.5), (2.5, 1.5)], &heights, &slopes, 5.0);
        assert_eq!(p.iter().map(|x| x.z).collect::<Vec<_>>(), vec![0.0, 1.0, 2.0], "heights 5, 6, 7 relative to the lowest");
        assert_eq!(p.iter().map(|x| x.s).collect::<Vec<_>>(), vec![5.0, 15.0, 25.0], "slope minus bias");
        assert_eq!(p.iter().map(|x| x.d).collect::<Vec<_>>(), vec![0.0, 1.0, 2.0]);
        assert_eq!(nearest(&p, 1.4), 1);
        assert_eq!(nearest(&p, -3.0), 0);
        assert_eq!(nearest(&p, 99.0), 2);
    }

    #[test]
    fn corridor_reads_the_shown_ensemble() {
        let ens = serde_json::json!({"shown_correlation_length_m": 10.0, "corridor": {"ensembles": [
            {"correlation_length_m": 3.0, "loop": {"agree_runs": 9, "runs": 9, "share_within_3m": {"p50": 1.0}}, "legs": []},
            {"correlation_length_m": 10.0, "loop": {"agree_runs": 1, "runs": 1000, "share_within_3m": {"p50": 0.569}},
             "legs": [{"agree_runs": 429, "runs": 1000}]}]}});
        let (legs, note) = corridor(&ens).unwrap();
        assert_eq!(legs[0].0, "43%");
        assert!(legs[0].1.contains("429 of 1000"), "{}", legs[0].1);
        assert!(note.contains("1 of 1000") && note.contains("57%"), "{note}");
        assert!(corridor(&serde_json::json!({})).is_none());
    }

    #[test]
    fn range_note_quotes_the_shown_ensemble_and_the_budget() {
        let ens = serde_json::json!({"shown_correlation_length_m": 10.0, "ensembles": [
            {"correlation_length_m": 3.0, "runs": 800, "converged": true, "time_s": {"p10": 60.0, "p50": 120.0, "p90": 180.0}},
            {"correlation_length_m": 10.0, "runs": 1000, "converged": false, "time_s": {"p10": 600.0, "p50": 900.0, "p90": 1200.0}}]});
        let s = range_note(&ens, true, 30.0);
        assert!(s.contains("1000 runs") && s.contains("10 to 20 min") && s.contains("median 15"), "{s}");
        assert!(s.contains("leaves 10 min of the 30 min budget"), "{s}");
        assert!(s.contains("not fully settled"), "{s}");
        let s = range_note(&ens, false, 15.0);
        assert!(s.contains("5.0 min more than the 15 min budget") && s.contains("does not apply"), "{s}");
    }

    #[test]
    fn band_runs_merge_neighbours_and_skip_flat_ground() {
        let s: Vec<Sample> = [0.0, 16.0, 17.0, 25.0, 3.0].iter().enumerate()
            .map(|(i, &v)| Sample { d: i as f32 * 10.0, e: 0.0, n: 0.0, z: 0.0, s: v, seen: true }).collect();
        assert_eq!(band_runs(&s, 15.0, 20.0), vec![(1, 5.0, 25.0), (2, 25.0, 35.0)]);
    }

    #[test]
    fn nice_axis_steps() {
        assert_eq!(nice_step(451.0, 5), 100.0);
        assert_eq!(nice_step(23.0, 5), 5.0);
        assert_eq!(nice_step(12.0, 4), 5.0);
    }

    #[test]
    fn summary_picks_the_worst_leg_and_longest_walk_back() {
        let route: Value = serde_json::from_str(r#"{"all_legs_possible": true, "total_length_m": 30, "total_time_s": 40,
            "legs": [{"possible": true, "length_m": 10, "max_corrected_slope_deg": 5, "share_in_rover_verified": 1.0, "cells_over_limit_inside_verified": 0},
                     {"possible": true, "length_m": 20, "max_corrected_slope_deg": 12, "share_in_rover_verified": 0.25, "cells_over_limit_inside_verified": 2}]}"#).unwrap();
        let stops: Vec<Value> = serde_json::from_str(r#"[{"walk_back_s": 0}, {"walk_back_s": 25}]"#).unwrap();
        let s = summary(&route, &stops);
        assert!(s.feasible);
        assert_eq!((s.worst_deg, s.worst_leg), (12.0, 1));
        assert_eq!((s.longest_back_s, s.longest_back_stop), (25.0, 1));
        assert_eq!(s.over_limit_cells, 2);
        assert!((s.verified_share - 0.5).abs() < 1e-9, "length-weighted: (10 x 1 + 20 x 0.25) / 30");
    }

    #[test]
    fn a_wall_between_two_people_blocks_the_sight_line() {
        // 9 x 1 strip, flat at 0 except a 3 m wall at x 4
        let mut z = vec![0.0f32; 9];
        let flat = Grid { w: 9, h: 1, cell: 1.0, z: z.clone(), top: 0.0 };
        assert!(sight_line(&flat, 0.5, 0.5, 8.5, 0.5));
        z[4] = 3.0;
        let wall = Grid { w: 9, h: 1, cell: 1.0, z, top: 3.0 };
        assert!(!sight_line(&wall, 0.5, 0.5, 8.5, 0.5), "a 3 m wall is above two 1.7 m eyes");
    }

    #[test]
    fn sight_of_the_exit_on_the_published_route() {
        let r: Value = serde_json::from_slice(&std::fs::read("../assets/route.json").unwrap()).unwrap();
        let g = Grid::from_bytes(&std::fs::read("../assets/heightmap.bin").unwrap(), 553, 1078, 1.0).unwrap();
        let poly: Vec<(f32, f32)> = r["polyline_en"].as_array().unwrap().iter().map(|p| (p[0].as_f64().unwrap() as f32, p[1].as_f64().unwrap() as f32)).collect();
        let s = profile(&poly, &g, &g, 0.0);
        assert!(s[0].seen, "the exit sees itself");
        let len: f32 = s.windows(2).filter(|w| !w[1].seen).map(|w| w[1].d - w[0].d).sum();
        println!("published route: {:.0} of {:.0} m out of sight of the exit", len, s.last().unwrap().d);
    }
}