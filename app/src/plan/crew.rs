//! Survival margin and two-person EVA (method fixed in advance). Turns the plan's own
//! numbers into planning answers with values from eva_ops.json (NASA ConOps quotes with pages; lunar, labelled for Mars):
//! - station time available = (EVA length - egress/ingress) - walking time (p90 of the uncertainty runs for the published
//!   plan, else the plan's time), total and per station; GO if >= 0 (and >= the planner's typed minutes per stop, if any);
//! - the emergency hour is a reserve, never planned in;
//! - rescue check: the partner must bring an incapacitated crewmember from the stop with the longest walk-back to the exit
//!   within the ConOps limit; the output is the slowest drag pace that still meets it (no drag speed is assumed).
//! - station time used by the EVA simulation: the planner's typed minutes per stop, else the station time available
//!   split evenly over the stations (so the simulated EVA fills the chosen EVA length, walking at its p90 time);
//! - egress/ingress: NASA gives one hour for both; EGRESS_SHARE of it is spent going out (ASSUMPTION, labelled in the UI).
//! No other number is typed here: every constant comes from eva_ops.json or the plan.
use bevy::prelude::*;
use serde_json::Value;
use std::sync::Mutex;

use crate::text::{crew as t, fill};
use crate::ui::model::{PanelModel, UiBridge};

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub enum CrewEvent {
    Hours(f64),
    /// planned minutes per stop (None: not planned)
    Planned(Option<f64>),
}

pub static EVENTS: Mutex<Vec<CrewEvent>> = Mutex::new(Vec::new());

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn send(e: CrewEvent) {
    if let Ok(mut q) = EVENTS.lock() {
        q.push(e);
    }
}

/// The planner's crew settings (None hours = the eva_ops.json default).
#[derive(Resource, Default, Clone, Debug, PartialEq)]
pub struct CrewState {
    pub hours: Option<f64>,
    pub planned_min: Option<f64>,
}

/// Share of NASA's egress/ingress hour spent on egress (the rest is ingress). ASSUMPTION: NASA gives only the total.
pub const EGRESS_SHARE: f64 = 0.5;

/// What the EVA simulation uses (published by `update`): seconds per station, egress, ingress, the EVA length and its
/// emergency reserve; `even_split` = station time is the even split, not the planner's typed minutes.
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq)]
pub struct EvaPlan {
    pub station_s: f64,
    pub egress_s: f64,
    pub ingress_s: f64,
    pub eva_s: f64,
    pub reserve_s: f64,
    pub even_split: bool,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct CrewModel {
    pub available: bool,
    pub hours: f64,
    pub options: Vec<f64>,
    pub go: bool,
    /// one line each: task time, walking time, station time available, margin (if planned), reserve
    pub lines: Vec<String>,
    pub verdict: String,
    pub rescue: String,
    pub rescue_pass: bool,
    pub roles: (String, String),
    pub roles_label: String,
    pub source_note: String,
    pub source_url: String,
    pub planned_min: Option<f64>,
    /// minutes per station used by the simulation, and whether it is the even split
    pub station_min: f64,
    pub even_split: bool,
    pub plan: EvaPlan,
}

fn num(v: &Value) -> Option<f64> {
    v.as_f64()
}

fn mins(x: f64) -> String {
    format!("{}", x.round() as i64)
}

/// The crew block for the current plan. `walk` = (walking seconds, basis text).
pub fn model(ops: &Value, st: &CrewState, panel: &PanelModel) -> CrewModel {
    let (Some(def), Some(egress), Some(extra), Some(limit)) = (num(&ops["eva_hours"]["default"]), num(&ops["egress_ingress_h"]["value"]),
                                                                num(&ops["emergency_extension_h"]["value"]), num(&ops["rescue_limit_min"]["value"]))
    else {
        return CrewModel::default();
    };
    let options: Vec<f64> = ops["eva_hours"]["options"].as_array().map(|a| a.iter().filter_map(|x| x.as_f64()).collect()).unwrap_or_default();
    let hours = st.hours.filter(|h| options.iter().any(|o| (o - h).abs() < 1e-9)).unwrap_or(def);
    let page = |k: &str| ops[k]["page"].as_i64().map(|p| p.to_string()).unwrap_or_default();
    let (walk_s, basis) = match panel.time_range {
        Some((_, p90)) => (p90, fill(t::BASIS_P90, &[("runs", panel.runs.clone())])),
        None => (panel.walk_s, t::BASIS_PLAN.to_string()),
    };
    let stations = panel.stops.len().saturating_sub(1).max(1);
    let task_min = (hours - egress) * 60.0;
    let walk_min = walk_s / 60.0;
    let avail = task_min - walk_min;
    let per = avail / stations as f64;
    let mut lines = vec![
        fill(t::TASK, &[("hours", format!("{hours}")), ("task", mins(task_min)), ("x", format!("{egress}")), ("page", page("egress_ingress_h"))]),
        fill(t::WALK, &[("walk", mins(walk_min)), ("basis", basis)]),
        fill(t::AVAIL, &[("avail", mins(avail)), ("n", stations.to_string()), ("per", mins(per))]),
    ];
    let mut go = avail >= 0.0 && panel.feasible;
    let typed = st.planned_min.filter(|p| p.is_finite() && *p >= 0.0);
    let station_min = typed.unwrap_or(per.max(0.0));
    let plan = EvaPlan { station_s: station_min * 60.0, egress_s: egress * EGRESS_SHARE * 3600.0, ingress_s: egress * (1.0 - EGRESS_SHARE) * 3600.0,
                         eva_s: hours * 3600.0, reserve_s: extra * 3600.0, even_split: typed.is_none() };
    if let Some(p) = typed {
        let margin = avail - p * stations as f64;
        go &= margin >= 0.0;
        lines.push(fill(if margin >= 0.0 { t::MARGIN_SPARE } else { t::MARGIN_OVER },
                        &[("planned", mins(p)), ("n", stations.to_string()), ("margin", mins(margin.abs()))]));
    }
    lines.push(fill(t::RESERVE, &[("reserve", format!("{extra}")), ("page", page("emergency_extension_h"))]));
    // rescue: walk-back at walking pace must fit the limit; then the drag may be this much slower
    let wb_min = panel.back_s / 60.0;
    let pass = wb_min.is_finite() && wb_min <= limit;
    let rescue = if pass {
        let frac = wb_min / limit;
        let speed = panel.controls.speed;
        fill(t::RESCUE_PASS, &[("wb", format!("{:.1}", wb_min)), ("stop", panel.back_stop.clone()), ("limit", mins(limit)),
                               ("pct", format!("{}", (frac * 100.0).ceil() as i64)), ("pace", format!("{:.2}", frac * speed)),
                               ("page", page("rescue_limit_min"))])
    } else {
        fill(t::RESCUE_FAIL, &[("wb", format!("{:.1}", wb_min)), ("stop", panel.back_stop.clone()), ("limit", mins(limit)),
                               ("page", page("rescue_limit_min"))])
    };
    let src = ops["sources"]["conops"]["url"].as_str().unwrap_or("").to_string();
    CrewModel {
        available: true,
        hours,
        options,
        go,
        verdict: if go { t::GO.to_string() } else { t::NO_GO.to_string() },
        lines,
        rescue,
        rescue_pass: pass,
        roles: (fill(t::EV1, &[("x", ops["roles"]["ev1"].as_str().unwrap_or("").to_string())]),
                fill(t::EV2, &[("x", ops["roles"]["ev2"].as_str().unwrap_or("").to_string())])),
        roles_label: ops["roles"]["label"].as_str().unwrap_or("").to_string(),
        source_note: fill(t::SOURCE, &[("page", page("mars"))]),
        source_url: src,
        planned_min: st.planned_min,
        station_min,
        even_split: typed.is_none(),
        plan,
    }
}

/// Apply crew events and publish the crew model when the plan or the settings change.
pub fn update(docs: Option<Res<crate::BundleDocs>>, mut st: ResMut<CrewState>, mut bridge: ResMut<UiBridge>, mut eva: ResMut<EvaPlan>) {
    let ev: Vec<CrewEvent> = EVENTS.lock().map(|mut q| q.drain(..).collect()).unwrap_or_default();
    for e in ev {
        match e {
            CrewEvent::Hours(h) => st.hours = Some(h),
            CrewEvent::Planned(p) => st.planned_min = p,
        }
    }
    let (Some(docs), Some(panel)) = (docs, bridge.panel.as_ref()) else { return };
    let Some(ops) = docs.eva_ops.as_ref() else { return };
    let m = model(ops, &st, panel);
    if *eva != m.plan {
        *eva = m.plan;
    }
    if bridge.crew.as_ref() != Some(&m) {
        bridge.set_crew(m);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::model::Controls;

    fn ops() -> Value {
        serde_json::from_slice(&std::fs::read("../assets/eva_ops.json").unwrap()).unwrap()
    }

    fn panel(walk_s: f64, range: Option<(f64, f64)>, back_s: f64, stops: usize) -> PanelModel {
        PanelModel { feasible: true, walk_s, time_range: range, back_s, back_stop: "5 Valle".into(), runs: "1000".into(),
                     stops: (0..stops).map(|i| format!("{i}")).collect(), controls: Controls { speed: 0.6, ..Default::default() },
                     ..Default::default() }
    }

    #[test]
    fn station_time_uses_the_p90_for_the_published_plan_and_the_sourced_overhead() {
        let o = ops();
        let m = model(&o, &CrewState::default(), &panel(1109.0, Some((827.5, 1744.0)), 432.0, 6));
        let def = o["eva_hours"]["default"].as_f64().unwrap();
        let egress = o["egress_ingress_h"]["value"].as_f64().unwrap();
        assert_eq!(m.hours, def);
        let avail = (def - egress) * 60.0 - 1744.0 / 60.0;
        assert!(m.lines[2].contains(&mins(avail)) && m.lines[2].contains(&mins(avail / 5.0)), "{:?}", m.lines);
        assert!(m.lines[1].contains("1000"), "p90 basis named: {:?}", m.lines[1]);
        assert!(m.go);
        // another plan: no range, the plan's own time
        let m2 = model(&o, &CrewState::default(), &panel(1109.0, None, 432.0, 6));
        assert!(m2.lines[2].contains(&mins((def - egress) * 60.0 - 1109.0 / 60.0)));
    }

    #[test]
    fn planned_minutes_set_the_margin_and_can_turn_it_no_go() {
        let o = ops();
        let st = CrewState { hours: Some(4.0), planned_min: Some(60.0) };
        let m = model(&o, &st, &panel(1744.0, None, 432.0, 6));
        // 4 h - 1 h = 180 min; walk 29 min; 5 stops x 60 = 300 -> over by 149
        assert!(!m.go && m.verdict == t::NO_GO, "{:?}", m.lines);
        assert!(m.lines.iter().any(|l| l.contains("149")), "{:?}", m.lines);
        let ok = model(&o, &CrewState { hours: Some(8.0), planned_min: Some(30.0) }, &panel(1744.0, None, 432.0, 6));
        assert!(ok.go);
        let bad_hours = model(&o, &CrewState { hours: Some(5.0), planned_min: None }, &panel(1744.0, None, 432.0, 6));
        assert_eq!(bad_hours.hours, o["eva_hours"]["default"].as_f64().unwrap(), "hours outside the options fall back to the default");
    }

    #[test]
    fn rescue_reports_the_slowest_drag_pace_against_the_limit() {
        let o = ops();
        let limit = o["rescue_limit_min"]["value"].as_f64().unwrap();
        let m = model(&o, &CrewState::default(), &panel(1109.0, None, 432.0, 6));
        assert!(m.rescue_pass);
        let frac = 432.0 / 60.0 / limit;
        assert!(m.rescue.contains(&format!("{:.2}", frac * 0.6)) && m.rescue.contains("5 Valle"), "{}", m.rescue);
        let far = model(&o, &CrewState::default(), &panel(1109.0, None, (limit + 1.0) * 60.0, 6));
        assert!(!far.rescue_pass && !far.rescue.contains('{'));
    }

    #[test]
    fn no_ops_file_no_block() {
        assert!(!model(&serde_json::json!({}), &CrewState::default(), &panel(1.0, None, 1.0, 2)).available);
    }

    #[test]
    fn station_time_is_the_even_split_unless_typed() {
        let o = ops();
        let p = panel(1744.0, Some((1000.0, 1744.0)), 432.0, 6);
        let m = model(&o, &CrewState { hours: Some(6.0), planned_min: None }, &p);
        // (6 h - 1 h) * 60 - 1744 s / 60 over 5 stations
        let want = ((6.0 - 1.0) * 60.0 - 1744.0 / 60.0) / 5.0;
        assert!((m.station_min - want).abs() < 1e-9 && m.even_split, "{} vs {want}", m.station_min);
        assert!((m.plan.egress_s + m.plan.ingress_s - 3600.0).abs() < 1e-9, "NASA's 1 h, split");
        let typed = model(&o, &CrewState { hours: Some(6.0), planned_min: Some(20.0) }, &p);
        assert!((typed.station_min - 20.0).abs() < 1e-9 && !typed.even_split);
    }
}