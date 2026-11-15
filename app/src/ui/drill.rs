//! Drill-in card model (pure, tested): what VISOR knows at the selected map object (scene/select.rs), as a few numbers.
//! Station: walk-back, map vs rover tilt, nearest chemistry, samples, the science question, photo, Curiosity's 360 view.
//! Curiosity stop: measured tilt vs the map's. Chemistry target: instrument, sol, oxides. Leg: time, steepest, rover-driven
//! share, same-path share, metres out of sight of the exit. Every number comes from the bundle or the live plan.
use serde_json::Value;

use crate::scene::checks::Check;
use crate::scene::select::Pick;
use crate::text::drill as t;
use crate::ui::model::{DrillModel, Pano, PanelModel};

fn f1(x: f64) -> String {
    format!("{x:.1}")
}

fn check_word(label: &str) -> &'static str {
    match label { "agrees" => t::CHECK_AGREES, "partial" => t::CHECK_PARTIAL, _ => t::CHECK_OFF }
}

/// Eight-point compass word for a bearing (deg).
pub fn compass8(b: f32) -> &'static str {
    ["N", "NE", "E", "SE", "S", "SW", "W", "NW"][((b.rem_euclid(360.0) + 22.5) / 45.0) as usize % 8]
}

pub fn model(p: Pick, at: (f32, f32), panel: Option<&PanelModel>, checks: &[Check], chem: Option<&Value>, panos: Option<&Value>) -> DrillModel {
    let mut m = DrillModel { open: true, can_walk: true, at, ..Default::default() };
    match p {
        Pick::Station(i) => m.station = Some(i),
        Pick::Leg(i) => m.leg = Some(i),
        _ => {}
    }
    match p {
        Pick::Station(i) => {
            m.kind = t::KIND_STATION;
            let Some(s) = panel.and_then(|pm| pm.stop_cards.iter().find(|s| s.index == i)) else { return m };
            m.title = s.label.clone();
            if let Some(b) = s.walk_back_s {
                m.rows.push((t::R_BACK, format!("{} min", f1(b / 60.0))));
            }
            if let Some((l, a, r)) = &s.ground {
                let side = if s.map_understates() { t::GROUND_STEEPER } else if s.map_overstates() { t::GROUND_GENTLER } else { check_word(l) };
                m.rows.push((t::R_TILT, format!("{}\u{b0} / {}\u{b0} \u{b7} {side}", f1(*a), f1(*r))));
            }
            if let Some(c) = s.chem.first() {
                let v = |x: Option<f64>| x.map(f1).unwrap_or_else(|| "\u{2014}".into());
                m.rows.push((t::R_CHEM, format!("CaO {} \u{b7} SO\u{2083} {} wt% ({}, {:.0} m)", v(c.4), v(c.7), c.1, c.3)));
            }
            if !s.bags.is_empty() {
                m.rows.push((t::R_BAGS, s.bags.join(", ")));
            }
            // Curiosity measured steeper ground than the map: where to stand and work instead (the target stays)
            if let (Some((x, y, d, sl)), Some((_, a, r))) = (s.work_spot, s.ground.as_ref()) {
                let dir = compass8(crate::scene::walk::bearing_deg(s.e, s.n, x, y));
                m.rows.push((t::R_WORK, crate::text::fill(t::WORK_V, &[("x", format!("{d:.0}")), ("label", dir.to_string()),
                    ("a", format!("{sl:.0}")), ("b", format!("{:.0}", r - a))])));
            }
            // the station card in the inspector already shows the question and the photo: numbers and actions only here
            m.pano = panos.and_then(|d| Pano::from_doc(d, &s.id));
        }
        Pick::Rover(i) => {
            m.kind = t::KIND_ROVER;
            let Some(c) = checks.get(i) else { return m };
            m.title = crate::text::fill(t::ROVER_TITLE, &[("sol", c.sol.to_string())]);
            m.rows.push((t::R_ROVER_TILT, format!("{}\u{b0}", f1(c.rover_tilt as f64))));
            m.rows.push((t::R_MAP_TILT, format!("{}\u{b0} \u{b7} {}", f1(c.dem_tilt as f64), check_word(&c.label))));
            if let Some((d, name)) = panel.and_then(|pm| pm.stop_cards.iter()
                .map(|s| (((s.e - c.e).powi(2) + (s.n - c.n).powi(2)).sqrt(), s.label.clone())).min_by(|a, b| a.0.total_cmp(&b.0))) {
                m.rows.push((t::R_NEAREST, format!("{name}, {d:.0} m")));
            }
            m.note = t::ROVER_NOTE.into();
        }
        Pick::Chem(j) => {
            m.kind = t::KIND_CHEM;
            let Some(r) = chem.and_then(|d| d["records"].get(j)) else { return m };
            m.title = r["target"].as_str().unwrap_or("").replace('_', " ");
            m.rows.push((t::R_INSTRUMENT, format!("{} \u{b7} sol {}", r["instrument"].as_str().unwrap_or(""), r["sol"].as_i64().unwrap_or(0))));
            let o = &r["oxides"];
            let v = |k: &str| o[k].as_f64().map(f1).unwrap_or_else(|| "\u{2014}".into());
            let fe = o["FeO"].as_f64().or(o["FeOT"].as_f64()).map(f1).unwrap_or_else(|| "\u{2014}".into());
            m.rows.push((t::R_OXIDES, format!("{} \u{b7} {} \u{b7} {fe} \u{b7} {}", v("CaO"), v("SiO2"), v("SO3"))));
            m.note = t::CHEM_NOTE.into();
        }
        Pick::Leg(i) => {
            m.kind = t::KIND_LEG;
            let Some(pm) = panel else { return m };
            let Some(l) = pm.legs.get(i) else { return m };
            m.title = l.label.clone();
            m.rows.push((t::R_WALK, format!("{} min \u{b7} {} m", l.minutes, l.metres)));
            m.rows.push((t::R_STEEP, format!("{}\u{b0}", l.max_slope)));
            m.rows.push((t::R_VERIFIED, l.verified.clone()));
            if let Some(Some(a)) = pm.leg_agree.get(i) {
                m.rows.push((t::R_SAME, format!("{}%", (a * 100.0).round() as i64)));
            }
            let (s0, s1) = (pm.leg_starts.get(i).copied().unwrap_or(0), pm.leg_starts.get(i + 1).copied().unwrap_or(pm.samples.len()));
            let hidden: f32 = pm.samples.get(s0..s1.min(pm.samples.len())).unwrap_or(&[]).windows(2).filter(|w| !w[1].seen).map(|w| w[1].d - w[0].d).sum();
            m.rows.push((t::R_HIDDEN, format!("{} m", hidden.round() as i64)));
            m.profile = pm.samples.get(s0..(s1 + 1).min(pm.samples.len())).unwrap_or(&[]).to_vec();
            (m.caution, m.limit) = (pm.caution, pm.limit);
            m.can_walk = l.minutes != "\u{2014}";
        }
        // written by select::relay_run (it holds the coverage numbers)
        Pick::Relay | Pick::Work(_) => m.can_walk = false,
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::model::{LegRow, StopSummary};

    #[test]
    fn each_kind_gets_its_numbers() {
        let stop = StopSummary { index: 2, label: "3 R\u{ed}o Fr\u{ed}o".into(), id: "rio_frio".into(), walk_back_s: Some(168.0),
                                 ground: Some(("disagrees".into(), 11.47, 17.533)), bags: vec!["B2".into()], question: "Q?".into(), ..Default::default() };
        let pm = PanelModel { stop_cards: vec![stop], legs: vec![LegRow { label: "1 \u{2192} 2".into(), minutes: "1.3".into(), metres: "29".into(),
                              max_slope: "15.6".into(), verified: "24%".into(), possible: true, ..Default::default() }], ..Default::default() };
        let s = model(Pick::Station(2), (0.0, 0.0), Some(&pm), &[], None, None);
        assert_eq!(s.title, "3 R\u{ed}o Fr\u{ed}o");
        assert!(s.rows.iter().any(|r| r.0 == t::R_TILT && r.1.contains("11.5") && r.1.contains("17.5") && r.1.contains(t::GROUND_STEEPER)), "{:?}", s.rows);
        assert!(s.rows.iter().any(|r| r.0 == t::R_BACK && r.1 == "2.8 min"));
        let c = Check { e: 0.0, n: 0.0, label: "agrees".into(), sol: 4600, rover_tilt: 8.0, dem_tilt: 9.0, pose: None };
        let r = model(Pick::Rover(0), (0.0, 0.0), Some(&pm), &[c], None, None);
        assert!(r.title.contains("4600") && r.rows[0].1 == "8.0\u{b0}");
        let doc = serde_json::json!({"records": [{"target": "El_Tapado", "instrument": "APXS", "sol": 4635, "oxides": {"CaO": 5.0, "SiO2": 40.0, "FeOT": 18.2, "SO3": 7.1}}]});
        let ch = model(Pick::Chem(0), (0.0, 0.0), None, &[], Some(&doc), None);
        assert_eq!(ch.title, "El Tapado");
        assert!(ch.rows[1].1.contains("18.2") && ch.rows[1].1.starts_with("5.0"));
        let l = model(Pick::Leg(0), (0.0, 0.0), Some(&pm), &[], None, None);
        assert!(l.rows.iter().any(|r| r.0 == t::R_WALK && r.1 == "1.3 min \u{b7} 29 m"));
        assert!(model(Pick::Leg(5), (0.0, 0.0), Some(&pm), &[], None, None).rows.is_empty(), "unknown leg: no invented numbers");
    }
}
