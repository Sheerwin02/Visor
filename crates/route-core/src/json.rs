//! JSON forms of the planner's inputs and outputs, for messages between the web app and its planner worker
//! (crates/plan-worker). The big walk-back field travels separately as a Float32Array; everything else is here.
use serde_json::{json, Value};

use crate::light::HourResult;
use crate::{Leg, Plan, Rules};

pub fn rules_to(r: &Rules) -> Value {
    json!({ "flat_mps": r.flat_mps, "k": r.k, "bias_deg": r.bias_deg, "limit_deg": r.limit_deg, "verified_r_m": r.verified_r_m })
}

pub fn rules_from(v: &Value) -> Option<Rules> {
    let f = |k: &str| v[k].as_f64();
    Some(Rules { flat_mps: f("flat_mps")?, k: f("k")?, bias_deg: f("bias_deg")?, limit_deg: f("limit_deg")?, verified_r_m: f("verified_r_m")? })
}

pub fn points_to(p: &[(f64, f64)]) -> Value {
    Value::Array(p.iter().map(|&(e, n)| json!([e, n])).collect())
}

pub fn points_from(v: &Value) -> Vec<(f64, f64)> {
    v.as_array().into_iter().flatten().filter_map(|p| Some((p[0].as_f64()?, p[1].as_f64()?))).collect()
}

pub fn hours_to(h: &[HourResult]) -> Value {
    Value::Array(h.iter().map(|r| json!({
        "start_ltst": r.start_ltst, "passes": r.passes, "min_elev": r.min_elev, "max_elev": r.max_elev,
        "toward_low_sun_m": r.toward_low_sun_m, "away_m": r.away_m, "shadow_m": r.shadow_m, "margin": r.margin,
        "start_elev": r.start_elev, "start_az": r.start_az,
    })).collect())
}

pub fn hours_from(v: &Value) -> Vec<HourResult> {
    v.as_array().into_iter().flatten().filter_map(|r| {
        let f = |k: &str| r[k].as_f64();
        Some(HourResult { start_ltst: f("start_ltst")?, passes: r["passes"].as_bool()?, min_elev: f("min_elev")?, max_elev: f("max_elev")?,
                          toward_low_sun_m: f("toward_low_sun_m")?, away_m: f("away_m")?, shadow_m: f("shadow_m")?, margin: f("margin")?,
                          start_elev: f("start_elev")?, start_az: f("start_az")? })
    }).collect()
}

/// Everything in a plan except the walk-back field (JSON has no infinity: a missing walk-back time is null).
pub fn plan_to(p: &Plan) -> Value {
    let legs: Vec<Value> = p.legs.iter().map(|l| json!({
        "from": l.from, "to": l.to, "possible": l.possible, "length_m": l.length_m, "time_s": l.time_s,
        "max_corrected_slope_deg": l.max_corrected_slope_deg, "share_in_rover_verified": l.share_in_rover_verified,
        "cells_over_limit_inside_verified": l.cells_over_limit_inside_verified, "poly_start": l.poly_start, "poly_end": l.poly_end,
    })).collect();
    json!({
        "legs": legs, "polyline_en": points_to(&p.polyline_en), "walk_back_stops": p.walk_back_stops,
        "blocked_cells": p.blocked_cells, "verified_cells": p.verified_cells, "total_length_m": p.total_length_m,
        "total_time_s": p.total_time_s, "all_legs_possible": p.all_legs_possible,
    })
}

/// The inverse of `plan_to`; `walk_back` is the separately sent field (empty for a route-only plan).
pub fn plan_from(v: &Value, walk_back: Vec<f64>) -> Option<Plan> {
    let u = |x: &Value, k: &str| x[k].as_u64().map(|n| n as usize);
    let legs = v["legs"].as_array()?.iter().map(|l| Some(Leg {
        from: u(l, "from")?, to: u(l, "to")?, possible: l["possible"].as_bool()?, length_m: l["length_m"].as_f64()?,
        time_s: l["time_s"].as_f64()?, max_corrected_slope_deg: l["max_corrected_slope_deg"].as_f64()?,
        share_in_rover_verified: l["share_in_rover_verified"].as_f64()?,
        cells_over_limit_inside_verified: u(l, "cells_over_limit_inside_verified")?, poly_start: u(l, "poly_start")?, poly_end: u(l, "poly_end")?,
    })).collect::<Option<Vec<Leg>>>()?;
    Some(Plan {
        legs,
        polyline_en: points_from(&v["polyline_en"]),
        walk_back,
        walk_back_stops: v["walk_back_stops"].as_array()?.iter().map(|x| x.as_f64()).collect(),
        blocked_cells: u(v, "blocked_cells")?,
        verified_cells: u(v, "verified_cells")?,
        total_length_m: v["total_length_m"].as_f64()?,
        total_time_s: v["total_time_s"].as_f64()?,
        all_legs_possible: v["all_legs_possible"].as_bool()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{plan_loop, plan_route, CostGrid};

    #[test]
    fn plan_and_rules_round_trip_and_route_only_matches() {
        let (w, h) = (40, 20);
        let mut slope = vec![5.0f32; w * h];
        for r in 0..h {
            slope[r * w + 20] = 40.0; // a wall with a gap: the route must go round
        }
        slope[5 * w + 20] = 5.0;
        let rules = Rules { flat_mps: 0.6, k: 3.5, bias_deg: 3.0, limit_deg: 20.0, verified_r_m: 3.0 };
        assert_eq!(rules_from(&rules_to(&rules)), Some(rules));
        let g = CostGrid::new(&slope, w, h, 1.0, &rules, &[]);
        let stops = [(2.5, 10.5), (35.5, 10.5)];
        let full = plan_loop(&g, &stops, 20.0);
        let back = plan_from(&plan_to(&full), full.walk_back.clone()).unwrap();
        assert_eq!(back.polyline_en, full.polyline_en);
        assert_eq!(back.total_time_s, full.total_time_s);
        assert_eq!(back.walk_back_stops, full.walk_back_stops);
        assert_eq!(back.legs.len(), full.legs.len());
        let hr = HourResult { start_ltst: 8.0, passes: true, min_elev: 29.4, max_elev: 33.9, toward_low_sun_m: 0.0, away_m: 20.0,
                              shadow_m: 0.0, margin: 11.1, start_elev: 29.4, start_az: 82.9 };
        assert_eq!(hours_from(&hours_to(&[hr.clone()])), vec![hr]);
        let route = plan_route(&g, &stops, 20.0);
        assert_eq!(route.polyline_en, full.polyline_en, "route-only gives the same route");
        assert_eq!(route.total_time_s, full.total_time_s);
        assert!(route.walk_back.is_empty() && route.walk_back_stops.iter().all(|x| x.is_none()));
    }
}
