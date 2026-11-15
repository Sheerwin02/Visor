//! Data readiness, part A (rule fixed in advance): one row per data kind with its detail and what it is
//! ready for, built only from results already in the bundle (manifest measured values, readiness.json from the part B
//! check, conditions.json, radiation.json, navcam_check.json). No number is typed here; a row whose source is missing is
//! left out.
use serde_json::Value;

use crate::text::{fill, readiness as t};

#[derive(Clone, Debug, PartialEq, Default)]
pub struct ReadyRow {
    pub data: &'static str,
    pub detail: String,
    pub verdict: String,
    /// a data name built at run time (Trek layers); shown instead of `data` when set
    pub title: String,
    /// status chip: "ok" (use), "limited", "no" (not used) or "context"
    pub level: &'static str,
}

impl ReadyRow {
    pub fn with_title(mut self, t: String) -> Self {
        self.title = t;
        self
    }
    pub fn lvl(mut self, l: &'static str) -> Self {
        self.level = l;
        self
    }
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn name(&self) -> String {
        if self.title.is_empty() { self.data.to_string() } else { self.title.clone() }
    }
}

fn num(v: &Value) -> Option<f64> {
    v.as_f64()
}

fn fmt(x: f64) -> String {
    let s = format!("{x:.1}");
    s.strip_suffix(".0").map(str::to_string).unwrap_or(s)
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn rows(manifest: &Value, ready: Option<&Value>, ridges: Option<&Value>, cond: Option<&Value>, rad: Option<&Value>,
            navcam: Option<&Value>) -> Vec<ReadyRow> {
    rows_with_trek(manifest, ready, ridges, cond, rad, navcam, None)
}

/// Our sun model vs NASA Mars Trek's SPICE sun-angle service (sun_check.json, tools/trek_sun.py).
pub fn sun_row(v: &Value) -> Option<ReadyRow> {
    let o = v["outcome"].as_str()?;
    let n = v["n"].as_i64()?;
    let sols = v["sols"].as_array().map(|a| a.len()).unwrap_or(0);
    let (a, b) = (num(&v["max_abs_d_elevation_deg"])?, num(&v["max_abs_d_azimuth_deg"])?);
    Some(ReadyRow { title: String::new(), level: if o == "PASS" { "ok" } else { "limited" }, data: t::SUN_DATA,
                    detail: fill(t::SUN_RES, &[("n", n.to_string()), ("sols", sols.to_string())]),
                    verdict: fill(t::SUN_VERDICT, &[("outcome", o.to_string()), ("a", format!("{a:.2}")), ("b", format!("{b:.2}"))]) })
}

/// As `rows`, plus one row per NASA Mars Trek layer (trek_layers.json): its resolution and size over the patch.
pub fn trek_rows(trek: &Value) -> Vec<ReadyRow> {
    let mut out = Vec::new();
    let fmt_m = |m: f64| if m >= 10.0 { format!("{}", m.round() as i64) } else { format!("{m:.2}") };
    let fmt_px = |p: f64| if p >= 10.0 { format!("{}", p.round() as i64) } else if p >= 1.0 { format!("{p:.1}") } else { format!("{p:.2}") };
    for l in trek["layers"].as_array().cloned().unwrap_or_default() {
        let (Some(short), Some(m)) = (l["short"].as_str(), l["m_per_px"].as_f64()) else { continue };
        let a = fmt_px(l["patch_px"][0].as_f64().unwrap_or(0.0));
        let b = fmt_px(l["patch_px"][1].as_f64().unwrap_or(0.0));
        let level = if l["data_share"].as_f64() == Some(0.0) { "no" } else { "context" };
        let verdict = if l["data_share"].as_f64() == Some(0.0) {
            fill(t::TREK_VERDICT_EMPTY, &[("x", fmt_m(m))])
        } else if l["file"].is_string() && l["patch_px"][0].as_f64().unwrap_or(0.0) < crate::scene::basemap::MIN_PATCH_PX {
            fill(t::TREK_VERDICT_COARSE, &[("a", a.clone()), ("b", b.clone())])
        } else if l["file"].is_string() && l["kind"] == "Imagery" {
            let d = l["offset_vs_usgs_photo"]["east_m"].as_f64().unwrap_or(0.0).abs().max(l["offset_vs_usgs_photo"]["north_m"].as_f64().unwrap_or(0.0).abs());
            fill(t::TREK_VERDICT_BASEMAP, &[("a", a), ("b", b), ("d", format!("{}", d.round() as i64))])
        } else if l["file"].is_string() {
            fill(t::TREK_VERDICT_THERMAL, &[("a", a), ("b", b)])
        } else if l["patch_px"][0].as_f64().unwrap_or(0.0).max(l["patch_px"][1].as_f64().unwrap_or(0.0)) < 1.0 {
            t::TREK_VERDICT_SUBPIXEL.to_string()
        } else {
            fill(t::TREK_VERDICT, &[("a", a), ("b", b)])
        };
        out.push(ReadyRow { title: String::new(), level: "", data: "", detail: fill(t::TREK_RES, &[("x", fmt_m(m))]), verdict }.with_title(fill(t::TREK_DATA, &[("label", short.to_string())])).lvl(level));
    }
    if trek["excluded"].as_array().is_some_and(|e| !e.is_empty()) {
        out.push(ReadyRow { title: String::new(), level: "", data: t::TREK_EXCLUDED, detail: String::new(), verdict: t::TREK_EXCLUDED_VERDICT.to_string() }.lvl("no"));
    }
    out
}

pub fn rows_with_trek(manifest: &Value, ready: Option<&Value>, ridges: Option<&Value>, cond: Option<&Value>, rad: Option<&Value>,
                      navcam: Option<&Value>, trek: Option<&Value>) -> Vec<ReadyRow> {
    let mv = &manifest["measured_values"];
    let mut out = Vec::new();
    let g = &mv["gate"];
    if let (Some(d), Some(a), Some(b)) = (g["decision"].as_str(), num(&g["patch_share_pct"]), num(&g["reference_share_pct"])) {
        out.push(ReadyRow { title: String::new(), level: "", data: t::DEM_DATA, detail: t::DEM_RES.into(),
                            verdict: fill(t::DEM_VERDICT, &[("outcome", d.into()), ("a", fmt(a)), ("b", fmt(b))]) }.lvl(if d == "PASS" { "ok" } else { "limited" }));
    }
    let rc = &mv["rover_check"];
    if let (Some(bias), Some(n)) = (num(&rc["signed_median_deg"]), rc["n"].as_i64()) {
        let held = mv["heldout_bias"]["outcome"].as_str().unwrap_or(t::NOT_RUN);
        out.push(ReadyRow { title: String::new(), level: "", data: t::SLOPE_DATA, detail: t::SLOPE_RES.into(),
                            verdict: fill(t::SLOPE_VERDICT, &[("rc_bias", format!("{bias:+}")), ("n", n.to_string()), ("outcome", held.into())]) }.lvl(if held == "PASS" { "ok" } else { "limited" }));
    }
    if let Some(r) = ready {
        let res = &r["result"];
        let (e, n) = (num(&res["shift_m"]["east"]).unwrap_or(0.0), num(&res["shift_m"]["north"]).unwrap_or(0.0));
        if let (Some(v), Some(agree), Some(med), Some(a)) = (r["verdict"].as_str(), num(&res["share"]["agree"]), num(&res["median_abs_slope_diff_deg"]),
                                                             num(&r["thresholds_deg"]["agree"])) {
            out.push(ReadyRow { title: String::new(), level: "", data: t::PROV_DATA, detail: fill(t::PROV_RES, &[("x", format!("{}", e.hypot(n).round() as i64))]),
                                verdict: fill(t::PROV_VERDICT, &[("outcome", v.into()), ("a", fmt(a)), ("pct", format!("{}", (agree * 100.0).round() as i64)),
                                                                 ("z", fmt(med))]) }.lvl("limited"));
        }
    }
    if let Some(a) = ridges.map(|r| &r["check_a"]) {
        if let (Some(o), Some(c), Some(n)) = (a["outcome"].as_str(), a["correct"].as_i64(), a["n"].as_i64()) {
            out.push(ReadyRow { title: String::new(), level: "", data: t::RIDGE_DATA, detail: t::RIDGE_RES.into(),
                                verdict: fill(t::RIDGE_VERDICT, &[("outcome", o.into()), ("n", c.to_string()), ("total", n.to_string())]) }.lvl(if o == "PASS" { "ok" } else { "no" }));
        }
    }
    out.push(ReadyRow { title: String::new(), level: "", data: t::PHOTO_DATA, detail: t::PHOTO_RES.into(), verdict: t::PHOTO_VERDICT.into() }.lvl("ok"));
    out.push(ReadyRow { title: String::new(), level: "", data: t::PLACES_DATA, detail: t::PLACES_RES.into(), verdict: t::PLACES_VERDICT.into() }.lvl("ok"));
    out.push(ReadyRow { title: String::new(), level: "", data: t::CHEM_DATA, detail: t::CHEM_RES.into(), verdict: t::CHEM_VERDICT.into() }.lvl("ok"));
    if let Some(c) = cond {
        let n = c["sols"].as_array().map(|a| a.len()).unwrap_or(0);
        let miss = c["missing_sols"].as_array().map(|a| a.len()).unwrap_or(0);
        out.push(ReadyRow { title: String::new(), level: "", data: t::REMS_DATA, detail: t::REMS_RES.into(),
                            verdict: fill(t::REMS_VERDICT, &[("n", n.to_string()), ("total", (n + miss).to_string())]) }.lvl("context"));
    }
    if let Some(r) = rad {
        let n = r["sols"].as_array().map(|a| a.iter().filter(|s| s["valid"].as_bool() == Some(true)).count()).unwrap_or(0);
        out.push(ReadyRow { title: String::new(), level: "", data: t::RAD_DATA, detail: t::RAD_RES.into(), verdict: fill(t::RAD_VERDICT, &[("n", n.to_string())]) }.lvl("context"));
    }
    if let Some(c) = navcam.and_then(|v| v["comparison"].as_str()) {
        out.push(ReadyRow { title: String::new(), level: "", data: t::NAVCAM_DATA, detail: t::NAVCAM_RES.into(), verdict: fill(t::NAVCAM_VERDICT, &[("outcome", c.into())]) }.lvl("limited"));
    }
    if let Some(tk) = trek {
        out.extend(trek_rows(tk));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(name: &str) -> Value {
        serde_json::from_slice(&std::fs::read(format!("../assets/{name}")).unwrap()).unwrap()
    }

    #[test]
    fn rows_from_the_real_bundle_carry_the_measured_values() {
        let m = read("manifest.json");
        let r = read("readiness.json");
        let rg = read("ridges.json");
        let rows = rows(&m, Some(&r), Some(&rg), Some(&read("conditions.json")), Some(&read("radiation.json")), Some(&read("navcam_check.json")));
        assert_eq!(rows.len(), 10, "{rows:#?}");
        assert!(rows[3].verdict.starts_with(rg["check_a"]["outcome"].as_str().unwrap()));
        let gate = &m["measured_values"]["gate"];
        assert!(rows[0].verdict.contains(gate["decision"].as_str().unwrap()));
        assert!(rows[1].verdict.contains(&format!("{:+}", m["measured_values"]["rover_check"]["signed_median_deg"].as_f64().unwrap())));
        assert!(rows[2].verdict.starts_with(r["verdict"].as_str().unwrap()));
        let agree = (r["result"]["share"]["agree"].as_f64().unwrap() * 100.0).round() as i64;
        assert!(rows[2].verdict.contains(&format!("{agree}%")));
        assert!(rows.iter().all(|x| !x.verdict.contains('{')), "unfilled placeholder");
        assert!(rows.iter().all(|x| ["ok", "limited", "no", "context"].contains(&x.level)), "every row has a status");
        assert_eq!(rows[3].level, if rg["check_a"]["outcome"] == "PASS" { "ok" } else { "no" });
    }

    #[test]
    fn missing_sources_drop_their_rows() {
        let rows = rows(&serde_json::json!({}), None, None, None, None, None);
        assert_eq!(rows.iter().map(|r| r.data).collect::<Vec<_>>(), vec![t::PHOTO_DATA, t::PLACES_DATA, t::CHEM_DATA]);
    }
}

#[cfg(test)]
mod trek_tests {
    use super::*;

    #[test]
    fn trek_rows_name_every_layer_with_its_resolution() {
        let t: Value = serde_json::from_slice(&std::fs::read("../assets/trek_layers.json").unwrap()).unwrap();
        let rows = trek_rows(&t);
        let n = t["layers"].as_array().unwrap().len() + usize::from(t["excluded"].as_array().is_some_and(|e| !e.is_empty()));
        assert_eq!(rows.len(), n);
        assert!(rows.iter().all(|r| !r.name().is_empty() && !r.verdict.contains('{') && !r.level.is_empty()), "{rows:#?}");
    }
}
