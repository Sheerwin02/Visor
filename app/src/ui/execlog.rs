//! Execute mode: the EVA log (Playbook-style). Per station: actual arrive / leave times (Mars local time "HH:MM", typed or
//! stamped from the EVA clock), task check-offs, bags collected and a note. Planned vs actual per station and how far the
//! crew is behind (the latest stamped time against its planned time), which feeds the "if we run late" rule.
//! Kept per viewer in the browser (localStorage) and exported as CSV. Pure logic here; the views are in views/.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
use std::collections::BTreeMap;

use serde_json::{json, Value};

/// One station's log. `tasks` / `bags`: index into the station's tasks / bags -> done.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct StationLog {
    pub arrive: String,
    pub leave: String,
    pub tasks: BTreeMap<usize, bool>,
    pub bags: BTreeMap<usize, bool>,
    pub note: String,
}

/// The whole log, by stop id.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ExecLog(pub BTreeMap<String, StationLog>);

/// "HH:MM" or "HH:MM:SS" -> minutes after midnight (local Mars time); None if not a time.
pub fn parse_hm(s: &str) -> Option<f64> {
    let p: Vec<&str> = s.trim().split(':').collect();
    if p.len() < 2 || p.len() > 3 {
        return None;
    }
    let h: u32 = p[0].parse().ok()?;
    let m: u32 = p[1].parse().ok()?;
    let sec: u32 = if p.len() == 3 { p[2].parse().ok()? } else { 0 };
    (h < 24 && m < 60 && sec < 60).then(|| h as f64 * 60.0 + m as f64 + sec as f64 / 60.0)
}

/// The station plan as the log needs it: stop id, name, planned arrive / leave (minutes after midnight).
#[derive(Clone, Debug, PartialEq)]
pub struct Planned {
    pub id: String,
    pub name: String,
    pub arrive: f64,
    pub leave: f64,
}

/// Minutes behind the plan (negative = ahead): the latest stamped time (leave before arrive at the same station) against
/// its planned time. None until something is stamped.
pub fn behind_min(plan: &[Planned], log: &ExecLog) -> Option<f64> {
    plan.iter().rev().find_map(|p| {
        let l = log.0.get(&p.id)?;
        parse_hm(&l.leave).map(|t| t - p.leave).or_else(|| parse_hm(&l.arrive).map(|t| t - p.arrive))
    })
}

/// Planned vs actual per station: (name, planned arrive, actual arrive, planned minutes, actual minutes) as display numbers.
pub fn deltas(plan: &[Planned], log: &ExecLog) -> Vec<(String, Option<f64>, Option<f64>)> {
    plan.iter().map(|p| {
        let l = log.0.get(&p.id);
        let a = l.and_then(|l| parse_hm(&l.arrive));
        let d = l.and_then(|l| Some(parse_hm(&l.leave)? - parse_hm(&l.arrive)?));
        (p.name.clone(), a.map(|a| a - p.arrive), d.map(|d| d - (p.leave - p.arrive)))
    }).collect()
}

fn hm(m: f64) -> String {
    let t = m.round() as i64;
    format!("{:02}:{:02}", t.div_euclid(60).rem_euclid(24), t.rem_euclid(60))
}

/// CSV of the log: one row per station.
pub fn to_csv(plan: &[Planned], log: &ExecLog, tasks_total: &BTreeMap<String, (usize, usize)>) -> String {
    let mut out = String::from("station,planned_arrive,actual_arrive,planned_leave,actual_leave,arrive_delta_min,station_delta_min,tasks_done,tasks_total,bags_collected,bags_total,note\n");
    for (p, (_, da, dd)) in plan.iter().zip(deltas(plan, log)) {
        let l = log.0.get(&p.id).cloned().unwrap_or_default();
        let (nt, nb) = tasks_total.get(&p.id).copied().unwrap_or((0, 0));
        let f = |x: Option<f64>| x.map(|v| format!("{v:.1}")).unwrap_or_default();
        out += &format!("{},{},{},{},{},{},{},{},{},{},{},{}\n", p.name.replace(',', " "), hm(p.arrive), l.arrive, hm(p.leave), l.leave,
                        f(da), f(dd), l.tasks.values().filter(|v| **v).count(), nt, l.bags.values().filter(|v| **v).count(), nb,
                        l.note.replace([',', '\n'], " "));
    }
    out
}

pub fn to_json(log: &ExecLog) -> Value {
    let m: serde_json::Map<String, Value> = log.0.iter().map(|(k, l)| (k.clone(), json!({
        "arrive": l.arrive, "leave": l.leave, "note": l.note,
        "tasks": l.tasks.iter().filter(|x| *x.1).map(|x| *x.0).collect::<Vec<_>>(),
        "bags": l.bags.iter().filter(|x| *x.1).map(|x| *x.0).collect::<Vec<_>>(),
    }))).collect();
    json!({"v": 1, "stations": m})
}

pub fn from_json(v: &Value) -> ExecLog {
    let idx = |x: &Value| -> BTreeMap<usize, bool> {
        x.as_array().into_iter().flatten().filter_map(|i| i.as_u64()).map(|i| (i as usize, true)).collect()
    };
    ExecLog(v["stations"].as_object().into_iter().flatten().map(|(k, s)| (k.clone(), StationLog {
        arrive: s["arrive"].as_str().unwrap_or("").into(), leave: s["leave"].as_str().unwrap_or("").into(),
        note: s["note"].as_str().unwrap_or("").into(), tasks: idx(&s["tasks"]), bags: idx(&s["bags"]),
    })).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> Vec<Planned> {
        vec![Planned { id: "a".into(), name: "2 A".into(), arrive: 510.0, leave: 563.0 },
             Planned { id: "b".into(), name: "3 B".into(), arrive: 565.0, leave: 618.0 }]
    }

    #[test]
    fn times_parse_and_lateness_follows_the_latest_stamp() {
        assert_eq!(parse_hm("08:30"), Some(510.0));
        assert_eq!(parse_hm("8:30:30"), Some(510.5));
        assert_eq!(parse_hm("25:00"), None);
        assert_eq!(parse_hm("x"), None);
        let mut log = ExecLog::default();
        assert_eq!(behind_min(&plan(), &log), None);
        log.0.entry("a".into()).or_default().arrive = "08:42".into();
        assert_eq!(behind_min(&plan(), &log), Some(12.0));
        log.0.get_mut("a").unwrap().leave = "09:50".into();
        assert_eq!(behind_min(&plan(), &log), Some(27.0), "leave 09:50 vs planned 09:23");
        log.0.entry("b".into()).or_default().arrive = "09:20".into();
        assert_eq!(behind_min(&plan(), &log), Some(-5.0), "the latest station wins");
        let d = deltas(&plan(), &log);
        assert_eq!(d[0], ("2 A".into(), Some(12.0), Some(15.0)), "68 min at A vs 53 planned");
    }

    #[test]
    fn log_round_trips_through_json_and_exports_csv() {
        let mut log = ExecLog::default();
        let s = log.0.entry("a".into()).or_default();
        s.arrive = "08:42".into();
        s.tasks.insert(2, true);
        s.bags.insert(0, true);
        s.note = "veins, dusty".into();
        assert_eq!(from_json(&to_json(&log)), log);
        let tot: BTreeMap<String, (usize, usize)> = [("a".to_string(), (5, 1)), ("b".to_string(), (4, 2))].into();
        let csv = to_csv(&plan(), &log, &tot);
        assert!(csv.lines().nth(1).unwrap().starts_with("2 A,08:30,08:42,09:23,,12.0,,1,5,1,1,veins  dusty"), "{csv}");
        assert_eq!(csv.lines().count(), 3);
    }
}
