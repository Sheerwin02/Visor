//! Saved plans and their comparison. A saved plan = a name, the share-link query (all planner settings and stops), the
//! planner's own station plans, and a snapshot of the numbers a planner compares options on. Kept per viewer in the
//! browser; "Open" applies the query and the station plans without a page reload.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
use serde_json::{json, Value};

/// The numbers compared between plans.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Metrics {
    pub walk_min: f64,
    pub length_m: f64,
    pub steepest_deg: f64,
    pub back_min: f64,
    pub unseen_m: f64,
    pub verified_pct: f64,
    pub stations: usize,
    pub bags: usize,
    pub end: String,
    pub go: bool,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct SavedPlan {
    pub name: String,
    pub query: String,
    pub edits: Value,
    pub m: Metrics,
}

/// Which way is better for a row: lower, higher, or not ranked.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Better {
    Lower,
    Higher,
    None,
}

/// Comparison rows: (label key, values as text, index of the best plan if one is strictly best).
pub fn rows(plans: &[&Metrics]) -> Vec<(&'static str, Vec<String>, Option<usize>)> {
    use crate::text::plans as t;
    let num: Vec<(&'static str, Better, Box<dyn Fn(&Metrics) -> f64>, Box<dyn Fn(f64) -> String>)> = vec![
        (t::R_WALK, Better::Lower, Box::new(|m: &Metrics| m.walk_min), Box::new(|v| format!("{v:.0} min"))),
        (t::R_STEEP, Better::Lower, Box::new(|m: &Metrics| m.steepest_deg), Box::new(|v| format!("{v:.0}\u{b0}"))),
        (t::R_BACK, Better::Lower, Box::new(|m: &Metrics| m.back_min), Box::new(|v| format!("{v:.1} min"))),
        (t::R_UNSEEN, Better::Lower, Box::new(|m: &Metrics| m.unseen_m), Box::new(|v| format!("{v:.0} m"))),
        (t::R_VERIFIED, Better::Higher, Box::new(|m: &Metrics| m.verified_pct), Box::new(|v| format!("{v:.0}%"))),
        (t::R_STATIONS, Better::Higher, Box::new(|m: &Metrics| m.stations as f64), Box::new(|v| format!("{v:.0}"))),
        (t::R_BAGS, Better::Higher, Box::new(|m: &Metrics| m.bags as f64), Box::new(|v| format!("{v:.0}"))),
        (t::R_LENGTH, Better::None, Box::new(|m: &Metrics| m.length_m), Box::new(|v| format!("{v:.0} m"))),
    ];
    let mut out: Vec<_> = num.iter().map(|(k, b, f, fmt)| {
        let v: Vec<f64> = plans.iter().map(|m| f(m)).collect();
        // compare at the precision shown
        let shown: Vec<String> = v.iter().map(|x| fmt(*x)).collect();
        let best = match b {
            Better::None => None,
            _ => {
                let key = |x: f64| if *b == Better::Lower { -x } else { x };
                let top = v.iter().map(|x| key((x * 10.0).round() / 10.0)).fold(f64::MIN, f64::max);
                let at: Vec<usize> = (0..v.len()).filter(|&i| key((v[i] * 10.0).round() / 10.0) == top).collect();
                (at.len() == 1 && v.len() > 1).then(|| at[0])
            }
        };
        (*k, shown, best)
    }).collect();
    out.push((t::R_END, plans.iter().map(|m| m.end.clone()).collect(), None));
    out.push((t::R_VERDICT, plans.iter().map(|m| if m.go { t::GO.to_string() } else { t::NO_GO.to_string() }).collect(), None));
    out
}

pub fn to_json(p: &[SavedPlan]) -> Value {
    json!({"v": 1, "plans": p.iter().map(|x| json!({"name": x.name, "query": x.query, "edits": x.edits, "m": {
        "walk_min": x.m.walk_min, "length_m": x.m.length_m, "steepest_deg": x.m.steepest_deg, "back_min": x.m.back_min,
        "unseen_m": x.m.unseen_m, "verified_pct": x.m.verified_pct, "stations": x.m.stations, "bags": x.m.bags, "end": x.m.end, "go": x.m.go}})).collect::<Vec<_>>()})
}

pub fn from_json(v: &Value) -> Vec<SavedPlan> {
    v["plans"].as_array().into_iter().flatten().filter_map(|x| {
        let m = &x["m"];
        Some(SavedPlan { name: x["name"].as_str()?.to_string(), query: x["query"].as_str()?.to_string(), edits: x["edits"].clone(),
            m: Metrics { walk_min: m["walk_min"].as_f64()?, length_m: m["length_m"].as_f64().unwrap_or(0.0), steepest_deg: m["steepest_deg"].as_f64()?,
                         back_min: m["back_min"].as_f64()?, unseen_m: m["unseen_m"].as_f64().unwrap_or(0.0), verified_pct: m["verified_pct"].as_f64().unwrap_or(0.0),
                         stations: m["stations"].as_u64().unwrap_or(0) as usize, bags: m["bags"].as_u64().unwrap_or(0) as usize,
                         end: m["end"].as_str().unwrap_or("").to_string(), go: m["go"].as_bool().unwrap_or(false) } })
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(walk: f64, steep: f64, unseen: f64, ver: f64, st: usize) -> Metrics {
        Metrics { walk_min: walk, length_m: 400.0, steepest_deg: steep, back_min: 7.0, unseen_m: unseen, verified_pct: ver, stations: st, bags: 5,
                  end: "13:40".into(), go: true }
    }

    #[test]
    fn best_values_are_marked_per_row_and_ties_are_not() {
        let (a, b) = (m(18.0, 22.0, 362.0, 40.0, 5), m(24.0, 15.0, 120.0, 75.0, 4));
        let r = rows(&[&a, &b]);
        let get = |k: &str| r.iter().find(|x| x.0 == k).unwrap().clone();
        assert_eq!(get(crate::text::plans::R_WALK).2, Some(0));
        assert_eq!(get(crate::text::plans::R_STEEP).2, Some(1));
        assert_eq!(get(crate::text::plans::R_UNSEEN).2, Some(1));
        assert_eq!(get(crate::text::plans::R_VERIFIED).2, Some(1));
        assert_eq!(get(crate::text::plans::R_STATIONS).2, Some(0));
        assert_eq!(get(crate::text::plans::R_BACK).2, None, "a tie has no winner");
        assert_eq!(get(crate::text::plans::R_LENGTH).2, None, "length is not ranked");
        assert_eq!(rows(&[&a]).iter().filter(|x| x.2.is_some()).count(), 0, "one plan: nothing to rank");
    }

    #[test]
    fn saved_plans_round_trip() {
        let p = vec![SavedPlan { name: "Direct".into(), query: "?v=1&stops=exit,bococo".into(), edits: json!({"custom-1": []}), m: m(18.0, 22.0, 362.0, 40.0, 5) }];
        assert_eq!(from_json(&to_json(&p)), p);
    }
}
