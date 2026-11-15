//! Mission brief (inspector's first tab): existing results turned into decisions, per rules
//! written before any finding was shown. Each finding has a verdict, why (the evidence) and changes if
//! (what would flip it). No new measurements and no typed numbers: every value comes from the bundle or the live models.
//! Pure functions here are tested natively on the real bundle; `update` publishes the model through UiBridge.
use bevy::prelude::*;
use serde_json::Value;

use crate::text::{brief as t, fill};
use crate::ui::model::{LightModel, PanelModel, UiBridge};

/// Legs whose corridor agreement is below this share are named "least certain" (rule 2; a design choice, not measured).
pub const WEAK_LEG_SHARE: f64 = 0.10;

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Finding {
    pub key: &'static str,
    pub title: &'static str,
    pub verdict: String,
    pub why: String,
    pub changes_if: String,
    /// the headline number shown first (already formatted; empty = none)
    pub figure: String,
    /// "ok", "watch" or "info": the status dot
    pub level: &'static str,
}

impl Finding {
    fn fig(mut self, figure: String, level: &'static str) -> Self {
        self.figure = figure;
        self.level = level;
        self
    }
}

/// A signed number with a real minus sign.
fn signed(x: f64, dp: usize) -> String {
    format!("{x:+.dp$}").replace('-', "\u{2212}")
}

/// One input's Spearman correlation with walking time (rule 1 bars).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct DriverBar {
    pub label: &'static str,
    pub rho: f64,
    pub driver: bool,
}

/// One study sol on the season strip: sol, air min, air max (deg C as the feed gives them).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SeasonSol {
    pub sol: f64,
    pub min: f64,
    pub max: f64,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct BriefModel {
    pub findings: Vec<Finding>,
    pub drivers: Vec<DriverBar>,
    /// leg indices named least certain (also marked on the EVA timeline)
    pub weak_legs: Vec<usize>,
    pub season: Vec<SeasonSol>,
    pub sol: f64,
    /// data readiness table (readiness.rs; shown in the Trust tab)
    pub readiness: Vec<crate::ui::readiness::ReadyRow>,
    /// "Open in NASA Mars Trek" link for the whole patch (About tab)
    pub trek_link: String,
}

/// The ensemble the app shows (correlation length = shown_correlation_length_m).
fn shown(ens: &Value) -> Option<&Value> {
    let l = ens["shown_correlation_length_m"].as_f64();
    ens["ensembles"].as_array()?.iter().find(|e| e["correlation_length_m"].as_f64() == l)
}

/// Rule 1: Spearman correlation of walking time with each once-per-run input; the largest |rho| is the driver.
pub fn drivers(ens: &Value) -> Option<(Vec<DriverBar>, Finding)> {
    let e = shown(ens)?;
    let sp = &e["spearman_time_vs"];
    let names = [("speed", t::IN_SPEED), ("k", t::IN_K), ("bias", t::IN_BIAS), ("limit", t::IN_LIMIT)];
    let mut bars: Vec<DriverBar> = names.iter().filter_map(|(k, label)| sp[*k].as_f64().map(|rho| DriverBar { label, rho, driver: false })).collect();
    let top = (0..bars.len()).max_by(|&a, &b| bars[a].rho.abs().total_cmp(&bars[b].rho.abs()))?;
    bars[top].driver = true;
    let d = bars[top].label.to_string();
    let runs = e["runs"].as_u64().unwrap_or(0).to_string();
    let len = format!("{}", e["correlation_length_m"].as_f64().unwrap_or(f64::NAN));
    let rho = signed(bars[top].rho, 2);
    Some((bars, Finding {
        key: "drive",
        title: t::DRIVE_TITLE,
        verdict: fill(t::DRIVE_VERDICT, &[("driver", d.clone())]),
        why: fill(t::DRIVE_WHY, &[("runs", runs), ("d", len)]),
        changes_if: fill(t::DRIVE_CHANGES, &[("driver", d.to_lowercase())]),
        ..Default::default()
    }.fig(fill(t::FIG_RHO, &[("rho", rho)]), "info")))
}

/// Rule 2: legs of the published route kept within 3 m by fewer than WEAK_LEG_SHARE of the runs. `agree` is
/// PanelModel.leg_agree (empty or all None for other plans). None = omit the finding (no weak legs).
pub fn weak_legs(agree: &[Option<f64>], labels: &[String], runs: &str) -> Option<(Vec<usize>, Finding)> {
    let pct = format!("{}", (WEAK_LEG_SHARE * 100.0).round() as i64);
    if agree.iter().all(|a| a.is_none()) {
        return Some((Vec::new(), Finding { key: "legs", title: t::LEGS_TITLE, verdict: t::LEGS_OTHER.to_string(),
                                           ..Default::default() }.fig(String::new(), "info")));
    }
    let weak: Vec<usize> = agree.iter().enumerate().filter(|(_, a)| a.is_some_and(|x| x < WEAK_LEG_SHARE)).map(|(i, _)| i).collect();
    if weak.is_empty() {
        return None;
    }
    let fig = fill(t::FIG_LEGS, &[("n", weak.len().to_string()), ("total", agree.len().to_string())]);
    let names: Vec<String> = weak.iter().map(|&i| labels.get(i).cloned().unwrap_or_else(|| format!("{}", i + 1))).collect();
    Some((weak, Finding {
        key: "legs",
        title: t::LEGS_TITLE,
        verdict: fill(t::LEGS_VERDICT, &[("legs", names.join("; "))]),
        why: fill(t::LEGS_WHY, &[("pct", pct), ("runs", runs.to_string())]),
        changes_if: t::LEGS_CHANGES.to_string(),
        ..Default::default()
    }.fig(fig, "watch")))
}

/// Rule 3: the live robustness lines (first line = verdict, the rest = why).
pub fn stability(robust: &[String]) -> Option<Finding> {
    let (first, rest) = robust.split_first()?;
    Some(Finding { key: "stab", title: t::STAB_TITLE, verdict: first.clone(), why: rest.join(" "), changes_if: t::STAB_CHANGES.to_string(),
                   ..Default::default() }.fig(String::new(), "info"))
}

/// Rule 4: the rover check's signed median and N (manifest measured_values) with the held-out outcome.
pub fn map_vs_ground(manifest: &Value) -> Option<Finding> {
    let mv = &manifest["measured_values"];
    let rc = &mv["rover_check"];
    let bias = rc["signed_median_deg"].as_f64()?;
    let n = rc["n"].as_i64()?.to_string();
    let rcb = format!("{bias:+}");
    let h = &mv["heldout_bias"];
    let why = match (h["outcome"].as_str(), h["pooled_heldout_median_abs_uncorrected"].as_f64(), h["pooled_heldout_median_abs_corrected"].as_f64()) {
        (Some(o), Some(a), Some(b)) => fill(t::MAP_WHY, &[("rc_bias", rcb), ("n", n), ("outcome", o.to_string()), ("a", format!("{a}")), ("b", format!("{b}"))]),
        _ => fill(t::MAP_WHY_NOHELD, &[("rc_bias", rcb), ("n", n)]),
    };
    Some(Finding { key: "map", title: t::MAP_TITLE, verdict: if bias > 0.0 { t::MAP_STEEPER } else { t::MAP_GENTLER }.to_string(), why,
                   changes_if: t::MAP_CHANGES.to_string(), ..Default::default() }
         .fig(fill(t::FIG_DEG, &[("x", signed(bias, 1))]), "watch"))
}

/// Rule 5: the best row of the sol's hour table; while the table is computing (or no hour passes) the status line.
pub fn start_hour(l: &LightModel) -> Option<Finding> {
    if l.status.is_empty() && l.rows.is_empty() {
        return None;
    }
    let sol = format!("{}", l.sol as i64);
    let changes_if = fill(t::HOUR_CHANGES, &[("lo", format!("{}", l.band.0)), ("hi", format!("{}", l.band.1))]);
    Some(match l.rows.iter().find(|r| r.best) {
        Some(r) => Finding { key: "hour", title: t::HOUR_TITLE, verdict: fill(t::HOUR_VERDICT, &[("hour", r.label.clone()), ("sol", sol)]),
                             why: fill(t::HOUR_WHY, &[("el", r.sun.clone()), ("x", r.toward.clone()), ("z", r.shadow.clone())]), changes_if,
                             ..Default::default() }.fig(r.label.clone(), "ok"),
        None => Finding { key: "hour", title: t::HOUR_TITLE, verdict: l.status.clone(), changes_if, ..Default::default() }
            .fig(String::new(), if l.rows.is_empty() { "info" } else { "watch" }),
    })
}

/// Rule 6: over valid stop sols, if (max - min of the silicon-detector medians) is smaller than the smallest within-sol
/// p10..p90 range, RAD "does not separate the stop sols"; otherwise the sol with the highest median is named.
pub fn radiation(doc: &Value) -> Option<Finding> {
    let v: Vec<(i64, f64, f64)> = doc["sols"].as_array()?.iter()
        .filter(|s| s["archived"].as_bool() == Some(true) && s["valid"].as_bool() == Some(true))
        .filter_map(|s| Some((s["sol"].as_i64()?, s["b_median"].as_f64()?, s["b_p90"].as_f64()? - s["b_p10"].as_f64()?)))
        .collect();
    if v.is_empty() {
        return None;
    }
    let lo = v.iter().map(|x| x.1).fold(f64::INFINITY, f64::min);
    let hi = v.iter().map(|x| x.1).fold(f64::NEG_INFINITY, f64::max);
    let spread = v.iter().map(|x| x.2).fold(f64::INFINITY, f64::min);
    let verdict = if hi - lo < spread {
        t::RAD_FLAT.to_string()
    } else {
        let top = v.iter().max_by(|a, b| a.1.total_cmp(&b.1)).map(|x| x.0).unwrap_or_default();
        fill(t::RAD_HIGH, &[("sol", top.to_string())])
    };
    Some(Finding { key: "rad", title: t::RAD_TITLE, verdict,
                   why: fill(t::RAD_WHY, &[("lo", format!("{lo:.2}")), ("hi", format!("{hi:.2}")), ("n", v.len().to_string()), ("x", format!("{spread:.2}"))]),
                   changes_if: t::RAD_CHANGES.to_string(), ..Default::default() }
         .fig(fill(t::FIG_RAD, &[("lo", format!("{lo:.1}")), ("hi", format!("{hi:.1}"))]), "info"))
}

/// Rule 7: the season strip (air min and max for every study sol with a record) and the chosen sol's air maximum ranked
/// among them (share of sols with a lower maximum = "colder").
pub fn weather(doc: &Value, sol: f64) -> Option<(Vec<SeasonSol>, Finding)> {
    let season: Vec<SeasonSol> = doc["sols"].as_array()?.iter()
        .filter_map(|r| Some(SeasonSol { sol: r["sol"].as_f64()?, min: r["air_min_c"].as_f64()?, max: r["air_max_c"].as_f64()? }))
        .collect();
    if season.is_empty() {
        return None;
    }
    let s = format!("{}", sol.round() as i64);
    let n = season.len();
    let f = match season.iter().find(|x| x.sol == sol.round()) {
        Some(me) => {
            let colder = season.iter().filter(|x| x.max < me.max).count();
            Finding { key: "wx", title: t::WX_TITLE,
                      verdict: fill(t::WX_VERDICT, &[("sol", s), ("hi", format!("{}", me.max.round() as i64)),
                                                     ("pct", format!("{}", (colder as f64 / n as f64 * 100.0).round() as i64))]),
                      why: fill(t::WX_WHY, &[("n", n.to_string())]), changes_if: t::WX_CHANGES.to_string(), ..Default::default() }
                .fig(fill(t::FIG_C, &[("x", signed(me.max.round(), 0).trim_start_matches('+').to_string())]), "info")
        }
        None => Finding { key: "wx", title: t::WX_TITLE, verdict: fill(t::WX_NONE, &[("sol", s)]),
                          why: fill(t::WX_WHY, &[("n", n.to_string())]), changes_if: t::WX_CHANGES.to_string(), ..Default::default() }
                .fig(String::new(), "info"),
    };
    Some((season, f))
}

/// The whole brief, in the rules' order. Missing inputs drop their finding.
pub fn build(panel: &PanelModel, light: Option<&LightModel>, manifest: &Value, ens: Option<&Value>, rad: Option<&Value>,
             cond: Option<&Value>) -> BriefModel {
    let mut b = BriefModel { sol: light.map(|l| l.sol).unwrap_or(f64::NAN), ..Default::default() };
    if let Some((bars, f)) = ens.and_then(drivers) {
        b.drivers = bars;
        b.findings.push(f);
    }
    let labels: Vec<String> = panel.legs.iter().map(|l| l.label.clone()).collect();
    if ens.is_some() {
        if let Some((weak, f)) = weak_legs(&panel.leg_agree, &labels, &panel.runs) {
            b.weak_legs = weak;
            b.findings.push(f);
        }
    }
    b.findings.extend(stability(&panel.controls.robust));
    b.findings.extend(map_vs_ground(manifest));
    b.findings.extend(light.and_then(start_hour));
    b.findings.extend(rad.and_then(radiation));
    if let (Some(doc), Some(l)) = (cond, light) {
        if let Some((season, f)) = weather(doc, l.sol) {
            b.season = season;
            b.findings.push(f);
        }
    }
    b
}

/// The EVA plan (crew.rs): stations x minutes, walking, egress/ingress, reserve. First in the brief.
pub fn eva_plan(c: &crate::plan::crew::CrewModel, panel: &PanelModel, ops: &Value) -> Option<Finding> {
    if !c.available {
        return None;
    }
    let n = panel.stops.len().saturating_sub(1).max(1);
    let x = format!("{}", c.station_min.round() as i64);
    let walk = format!("{}", (panel.walk_s / 60.0).round() as i64);
    let p90 = format!("{}", (panel.time_range.map(|r| r.1).unwrap_or(panel.walk_s) / 60.0).round() as i64);
    let num = |k: &str| ops[k]["value"].as_f64().map(|v| format!("{v}")).unwrap_or_default();
    let verdict = if c.even_split {
        fill(t::EVA_VERDICT, &[("hours", format!("{}", c.hours)), ("n", n.to_string()), ("x", x.clone()), ("walk", walk), ("p90", p90),
                               ("eg", num("egress_ingress_h")), ("res", num("emergency_extension_h"))])
    } else {
        fill(t::EVA_VERDICT_TYPED, &[("hours", format!("{}", c.hours)), ("x", x.clone()),
                                     ("margin", c.lines.iter().find(|l| l.contains(&x)).cloned().unwrap_or_default())])
    };
    Some(Finding { key: "eva", title: t::EVA_TITLE, verdict, why: fill(t::EVA_WHY, &[("n", n.to_string())]), changes_if: t::EVA_CHANGES.to_string(),
                   ..Default::default() }
         .fig(fill(t::EVA_FIG, &[("n", n.to_string()), ("x", x)]), if c.go { "ok" } else { "watch" }))
}

/// Order of the brief: decisions first (EVA plan, map vs ground, weak legs, start hour), context after.
const ORDER: [&str; 8] = ["eva", "map", "legs", "hour", "stab", "wx", "rad", "drive"];

/// Rebuild the brief when the panel or the light model changes and publish it.
pub fn update(docs: Option<Res<crate::BundleDocs>>, mut bridge: ResMut<UiBridge>) {
    let Some(docs) = docs else { return };
    if !bridge.is_changed() {
        return;
    }
    let Some(panel) = bridge.panel.as_ref() else { return };
    let mut m = build(panel, bridge.light.as_ref(), &docs.manifest, docs.ensemble.as_ref(), docs.radiation.as_ref(), docs.conditions.as_ref());
    if let (Some(c), Some(ops)) = (bridge.crew.as_ref(), docs.eva_ops.as_ref()) {
        m.findings.extend(eva_plan(c, panel, ops));
    }
    m.findings.sort_by_key(|f| ORDER.iter().position(|k| *k == f.key).unwrap_or(ORDER.len()));
    m.trek_link = docs.trek.as_ref().and_then(|t| crate::scene::basemap::trek_link(t, None, 0.0)).unwrap_or_default();
    m.readiness = crate::ui::readiness::rows_with_trek(&docs.manifest, docs.readiness.as_ref(), docs.ridges.as_ref(), docs.conditions.as_ref(),
                                                       docs.radiation.as_ref(), docs.navcam.as_ref(), docs.trek.as_ref());
    if let Some(r) = docs.sun_check.as_ref().and_then(crate::ui::readiness::sun_row) {
        let at = m.readiness.iter().position(|x| x.data == crate::text::readiness::REMS_DATA).unwrap_or(m.readiness.len());
        m.readiness.insert(at, r);
    }
    if bridge.brief.as_ref() != Some(&m) {
        bridge.set_brief(m);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::model::LightRow;
    use serde_json::json;

    fn read(name: &str) -> Value {
        serde_json::from_slice(&std::fs::read(format!("../assets/{name}")).unwrap()).unwrap()
    }

    #[test]
    fn headline_numbers_come_from_the_bundle() {
        let m = read("manifest.json");
        let f = map_vs_ground(&m).unwrap();
        let bias = m["measured_values"]["rover_check"]["signed_median_deg"].as_f64().unwrap();
        assert_eq!(f.figure, fill(t::FIG_DEG, &[("x", signed(bias, 1))]));
        assert_eq!(f.level, "watch");
        let (_, d) = drivers(&read("route_ensemble.json")).unwrap();
        assert!(d.figure.contains('\u{2212}') || d.figure.contains('+'), "{}", d.figure);
        let (w, l) = weak_legs(&[Some(0.5), Some(0.05)], &["a".into(), "b".into()], "10").unwrap();
        assert_eq!((w.len(), l.figure.clone(), l.level), (1, fill(t::FIG_LEGS, &[("n", "1".into()), ("total", "2".into())]), "watch"));
    }

    #[test]
    fn driver_is_the_largest_absolute_correlation_in_the_shown_ensemble() {
        let ens = read("route_ensemble.json");
        let (bars, f) = drivers(&ens).unwrap();
        let e = shown(&ens).unwrap();
        let top = ["speed", "k", "bias", "limit"].iter().max_by(|a, b| e["spearman_time_vs"][**a].as_f64().unwrap().abs()
            .total_cmp(&e["spearman_time_vs"][**b].as_f64().unwrap().abs())).unwrap();
        assert_eq!(bars.len(), 4);
        assert_eq!(bars.iter().filter(|b| b.driver).count(), 1);
        let d = bars.iter().find(|b| b.driver).unwrap();
        assert_eq!(d.rho, e["spearman_time_vs"][*top].as_f64().unwrap());
        assert!(f.verdict.starts_with(d.label));
        assert!(f.why.contains(&e["runs"].as_u64().unwrap().to_string()));
    }

    #[test]
    fn weak_legs_follow_the_threshold_and_other_plans_say_so() {
        let labels: Vec<String> = (1..=4).map(|i| format!("L{i}")).collect();
        let (w, f) = weak_legs(&[Some(0.5), Some(0.05), Some(0.10), Some(0.0)], &labels, "1000").unwrap();
        assert_eq!(w, vec![1, 3]);
        assert!(f.verdict.contains("L2") && f.verdict.contains("L4") && !f.verdict.contains("L3"));
        assert!(weak_legs(&[Some(0.5), Some(0.2)], &labels, "1000").is_none());
        let (w, f) = weak_legs(&[None, None], &labels, "").unwrap();
        assert!(w.is_empty());
        assert_eq!(f.verdict, t::LEGS_OTHER);
    }

    #[test]
    fn weak_legs_on_the_real_ensemble_match_the_corridor_counts() {
        let ens = read("route_ensemble.json");
        let agree = crate::ui::panel::leg_agreement(&ens);
        let labels: Vec<String> = (0..agree.len()).map(|i| format!("leg{i}")).collect();
        let (w, _) = weak_legs(&agree, &labels, "").unwrap();
        let expect: Vec<usize> = agree.iter().enumerate().filter(|(_, a)| a.unwrap() < WEAK_LEG_SHARE).map(|(i, _)| i).collect();
        assert_eq!(w, expect);
        assert!(!w.is_empty());
    }

    #[test]
    fn map_vs_ground_uses_manifest_values_and_the_held_out_outcome() {
        let m = read("manifest.json");
        let f = map_vs_ground(&m).unwrap();
        let rc = &m["measured_values"]["rover_check"];
        assert!(f.why.contains(&format!("{:+}", rc["signed_median_deg"].as_f64().unwrap())));
        assert!(f.why.contains(&rc["n"].as_i64().unwrap().to_string()));
        if let Some(o) = m["measured_values"]["heldout_bias"]["outcome"].as_str() {
            assert!(f.why.contains(o));
        }
        let steeper = rc["signed_median_deg"].as_f64().unwrap() > 0.0;
        assert_eq!(f.verdict, if steeper { t::MAP_STEEPER } else { t::MAP_GENTLER });
        let bare = json!({"measured_values": {"rover_check": {"signed_median_deg": -1.0, "n": 3}}});
        let f = map_vs_ground(&bare).unwrap();
        assert_eq!(f.verdict, t::MAP_GENTLER);
        assert!(f.why.contains("not in this bundle"));
    }

    #[test]
    fn radiation_rule_on_the_real_bundle_and_a_separating_case() {
        let r = read("radiation.json");
        let v: Vec<&Value> = r["sols"].as_array().unwrap().iter().filter(|s| s["valid"].as_bool() == Some(true)).collect();
        let med: Vec<f64> = v.iter().map(|s| s["b_median"].as_f64().unwrap()).collect();
        let spread = v.iter().map(|s| s["b_p90"].as_f64().unwrap() - s["b_p10"].as_f64().unwrap()).fold(f64::INFINITY, f64::min);
        let range = med.iter().cloned().fold(f64::NEG_INFINITY, f64::max) - med.iter().cloned().fold(f64::INFINITY, f64::min);
        let f = radiation(&r).unwrap();
        assert_eq!(f.verdict == t::RAD_FLAT, range < spread);
        let two = json!({"sols": [
            {"sol": 1, "archived": true, "valid": true, "b_median": 5.0, "b_p10": 4.9, "b_p90": 5.1},
            {"sol": 2, "archived": true, "valid": true, "b_median": 9.0, "b_p10": 8.9, "b_p90": 9.1},
            {"sol": 3, "archived": true, "valid": false, "b_median": 99.0, "b_p10": 0.0, "b_p90": 0.1}]});
        assert_eq!(radiation(&two).unwrap().verdict, fill(t::RAD_HIGH, &[("sol", "2".into())]));
    }

    #[test]
    fn weather_ranks_the_sol_and_reports_gaps() {
        let doc = json!({"sols": [
            {"sol": 10, "air_min_c": -80, "air_max_c": -30},
            {"sol": 11, "air_min_c": -79, "air_max_c": -20},
            {"sol": 12, "air_min_c": -78, "air_max_c": -10},
            {"sol": 13, "air_min_c": -77, "air_max_c": -20}]});
        let (season, f) = weather(&doc, 12.0).unwrap();
        assert_eq!(season.len(), 4);
        assert!(f.verdict.contains("75%"), "{}", f.verdict);
        let (_, f) = weather(&doc, 10.0).unwrap();
        assert!(f.verdict.contains(" 0%"), "{}", f.verdict);
        let (_, f) = weather(&doc, 14.0).unwrap();
        assert_eq!(f.verdict, fill(t::WX_NONE, &[("sol", "14".into())]));
        let real = read("conditions.json");
        let gap = real["missing_sols"][0].as_f64().unwrap();
        let (season, f) = weather(&real, gap).unwrap();
        assert_eq!(season.len(), real["sols"].as_array().unwrap().len());
        assert!(f.verdict.contains("No REMS"));
    }

    #[test]
    fn start_hour_names_the_best_row_or_shows_the_status() {
        let mut l = LightModel { sol: 4718.0, band: (15.0, 60.0), status: "s".into(), ..Default::default() };
        l.rows = vec![LightRow { label: "08:00".into(), sun: "20 / 90".into(), passes: true, toward: "3".into(), shadow: "4".into(), best: true, ..Default::default() },
                      LightRow { label: "09:00".into(), passes: true, ..Default::default() }];
        let f = start_hour(&l).unwrap();
        assert!(f.verdict.contains("08:00") && f.verdict.contains("4718"));
        assert!(f.why.contains("20 / 90"));
        l.rows[0].best = false;
        assert_eq!(start_hour(&l).unwrap().verdict, "s");
    }

    #[test]
    fn brief_from_the_real_bundle_keeps_rule_order() {
        let ens = read("route_ensemble.json");
        let panel = PanelModel { leg_agree: crate::ui::panel::leg_agreement(&ens), controls: crate::ui::model::Controls {
            robust: vec!["a".into(), "b".into()], ..Default::default() }, ..Default::default() };
        let light = LightModel { sol: 4718.0, ..Default::default() };
        let b = build(&panel, Some(&light), &read("manifest.json"), Some(&ens), Some(&read("radiation.json")), Some(&read("conditions.json")));
        let keys: Vec<&str> = b.findings.iter().map(|f| f.key).collect();
        assert_eq!(keys, vec!["drive", "legs", "stab", "map", "rad", "wx"]);
        assert!(!b.weak_legs.is_empty() && !b.season.is_empty() && b.drivers.len() == 4);
    }
}
