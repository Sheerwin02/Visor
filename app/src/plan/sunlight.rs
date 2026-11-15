//! When to walk (best-hour rule fixed in advance).
//! For the chosen sol: the start-hour table under the best-hour rule (computed by the planner worker for the current
//! route), the best hour, and DEM-scale cast shadows at the chosen hour, drawn by the terrain shader (horizontal
//! hatching, key 9). The photo is not relit: it keeps its own HiRISE light, shown next to the computed sun.
use std::sync::Mutex;

use bevy::prelude::*;
use route_core::light::{best, HourResult, BAND, LOW_SUN_DEG, TOWARD_CONE_DEG};

use crate::plan::compute::{Compute, Done, Job, LightJob, Mode};
use crate::ui::model::{LightModel, LightRow, UiBridge};
use crate::text::{fill, light as t};

/// The study sols (doc: the boxwork traverse) and the default (Nevado Sajama drilling completed).
pub const SOL_RANGE: (f64, f64) = (4534.0, 4865.0);
pub const DEFAULT_SOL: f64 = 4718.0;

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))] // sent by the web panel
pub enum LightEvent {
    Sol(f64),
    Hour(f64),
}

pub static EVENTS: Mutex<Vec<LightEvent>> = Mutex::new(Vec::new());
/// Light answers from the planner worker (the planner's `replan` drains the shared queue and forwards them here).
static ANSWERS: Mutex<Vec<Done>> = Mutex::new(Vec::new());

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn send(e: LightEvent) {
    if let Ok(mut q) = EVENTS.lock() {
        q.push(e);
    }
}

pub fn stash(d: Done) {
    if let Ok(mut q) = ANSWERS.lock() {
        q.push(d);
    }
}

#[derive(Resource)]
pub struct SunLight {
    pub sol: f64,
    /// chosen start hour (None: take the best, or the nearest when none passes)
    pub hour: Option<f64>,
    pub hours: Option<Vec<HourResult>>,
    /// sun for the shadow on screen: (elevation, azimuth, sol, hour)
    pub sun: Option<(f64, f64, f64, f64)>,
    /// shadow texture (one byte per cell), bound to the terrain material
    pub mask: Handle<Image>,
    next_id: u64,
    hours_id: u64,
    shadow_id: u64,
    asked_shadow: Option<(f64, f64)>,
}

impl SunLight {
    pub fn new(mask: Handle<Image>) -> Self {
        SunLight { sol: DEFAULT_SOL, hour: None, hours: None, sun: None, mask, next_id: 1 << 40, hours_id: 0, shadow_id: 0, asked_shadow: None }
    }
}

fn hour_label(h: f64) -> String {
    format!("{:02}:00", h as u32)
}

fn fmt_date((y, m, d): (i64, u32, u32)) -> String {
    const M: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!("{d} {} {y}", M[(m.clamp(1, 12) - 1) as usize])
}

/// The weather rows for a sol from conditions.json (None: no file in the bundle). Values are shown as the feed gives them.
pub fn weather(doc: Option<&serde_json::Value>, sol: f64) -> (Vec<(String, String)>, String, String, String, String) {
    let Some(doc) = doc else { return Default::default() };
    let src = &doc["source"];
    let source = fill(t::WX_SOURCE, &[("last", doc["science_archive"]["last_sol"].as_i64().map(|x| x.to_string()).unwrap_or_default())]);
    let url = src["url"].as_str().unwrap_or("").to_string();
    let disc = src["disclaimer"].as_str().unwrap_or("").to_string();
    let rec = doc["sols"].as_array().and_then(|a| a.iter().find(|r| r["sol"].as_f64() == Some(sol.round())));
    let Some(r) = rec else { return (Vec::new(), t::WX_NONE.to_string(), source, url, disc) };
    let n = |k: &str| r[k].as_f64().map(|x| format!("{}", x.round() as i64));
    let range = |lo: &str, hi: &str| match (n(lo), n(hi)) {
        (Some(a), Some(b)) => Some(fill(t::WX_RANGE_C, &[("lo", a), ("hi", b)])),
        _ => None,
    };
    let mut rows = Vec::new();
    if let Some(v) = range("air_min_c", "air_max_c") {
        rows.push((t::WX_AIR.to_string(), v));
    }
    if let Some(v) = range("ground_min_c", "ground_max_c") {
        rows.push((t::WX_GROUND.to_string(), v));
    }
    if let Some(v) = n("pressure_pa") {
        rows.push((t::WX_PRESSURE.to_string(), fill(t::WX_PA, &[("pa", v)])));
    }
    if let Some(v) = r["uv_index"].as_str() {
        rows.push((t::WX_UV.to_string(), v.to_string()));
    }
    let note = if rows.is_empty() { t::WX_NONE.to_string() } else { String::new() };
    (rows, note, source, url, disc)
}

/// RAD for a sol from radiation.json: an optional (label, value) row and a note (rule: stop sols only, context only).
pub fn radiation(doc: Option<&serde_json::Value>, sol: f64) -> (Option<(String, String)>, String) {
    let Some(doc) = doc else { return (None, String::new()) };
    let sols = doc["sols"].as_array().cloned().unwrap_or_default();
    let Some(r) = sols.iter().find(|r| r["sol"].as_f64() == Some(sol.round())) else {
        let list: Vec<String> = sols.iter().filter_map(|r| r["sol"].as_i64().map(|x| x.to_string())).collect();
        return (None, fill(t::RAD_STOP_ONLY, &[("sols", list.join(", "))]));
    };
    let sol_s = format!("{}", sol.round() as i64);
    let f = |k: &str| r[k].as_f64().map(|x| format!("{x:.1}"));
    match (r["archived"].as_bool(), r["valid"].as_bool(), f("b_median"), f("b_p10"), f("b_p90")) {
        (Some(true), Some(true), Some(med), Some(p10), Some(p90)) => (
            Some((t::RAD_LABEL.to_string(), fill(t::RAD_VALUE, &[("med", med), ("p10", p10), ("p90", p90)]))),
            fill(t::RAD_NOTE, &[("n", r["b_n"].as_i64().unwrap_or(0).to_string()), ("sol", sol_s)]),
        ),
        _ => (None, fill(t::RAD_NONE, &[("sol", sol_s)])),
    }
}

/// Median of the stop sols' median dose rates (RAD silicon detector B, uGy/h) and how many valid sols it uses.
pub fn rad_rate(doc: Option<&serde_json::Value>) -> Option<(f64, usize)> {
    let mut v: Vec<f64> = doc?["sols"].as_array()?.iter()
        .filter(|r| r["archived"].as_bool() == Some(true) && r["valid"].as_bool() == Some(true))
        .filter_map(|r| r["b_median"].as_f64()).collect();
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    let n = v.len();
    Some((if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 }, n))
}

/// The DONKI line (the same for every sol: the whole study period) and its events.
pub fn space_weather(doc: Option<&serde_json::Value>) -> (String, Vec<(String, String, String)>) {
    let Some(doc) = doc else { return (String::new(), Vec::new()) };
    let sw = &doc["space_weather"];
    if sw["note"].is_string() {
        return (t::SW_NA.to_string(), Vec::new());
    }
    let range = |i: usize| sw["range"][i].as_str().unwrap_or("").to_string();
    let ev: Vec<(String, String, String)> = sw["events"].as_array().cloned().unwrap_or_default().iter().map(|e| (
        e["start_utc"].as_str().unwrap_or("").chars().take(10).collect(),
        e["type"].as_str().unwrap_or("").to_string(),
        e["link"].as_str().unwrap_or("").to_string(),
    )).collect();
    let text = fill(if ev.is_empty() { t::SW_NONE } else { t::SW_SOME }, &[("a", range(0)), ("b", range(1))]);
    (text, ev)
}

/// The panel section for the current state (plain data; tested natively).
pub fn model(st: &SunLight, site: (f64, f64), photo: Option<&serde_json::Value>, wx: Option<&serde_json::Value>,
             rad: Option<&serde_json::Value>) -> LightModel {
    let ((y, mo, d), ls) = route_core::sun::sol_noon(st.sol, site.0, site.1);
    // whole degrees, never "-0"; azimuths in 0..359 (north is 0, not 360)
    let one = |x: f64| format!("{}", x.round() as i64);
    let deg = |x: f64| format!("{}", (x.round() as i64).rem_euclid(360));
    let rows: Vec<LightRow> = st.hours.as_ref().map(|hs| {
        let b = best(hs).ok();
        hs.iter().enumerate().map(|(i, r)| LightRow {
            hour: r.start_ltst,
            label: hour_label(r.start_ltst),
            sun: format!("{} / {}", one(r.start_elev), deg(r.start_az)),
            passes: r.passes,
            toward: if r.passes { one(r.toward_low_sun_m) } else { String::new() },
            shadow: if r.passes { one(r.shadow_m) } else { String::new() },
            best: b == Some(i),
            selected: st.hour == Some(r.start_ltst),
        }).collect()
    }).unwrap_or_default();
    let status = match st.hours.as_ref() {
        None => t::COMPUTING.to_string(),
        Some(hs) => match best(hs) {
            Ok(i) => fill(t::BEST_NOTE, &[("hour", hour_label(hs[i].start_ltst))]),
            Err(n) => fill(t::NONE_NOTE, &[("hour", n.map(|i| hour_label(hs[i].start_ltst)).unwrap_or_default())]),
        },
    };
    let (sun_text, sun_az, warn) = match st.sun {
        Some((el, az, _, h)) => (fill(t::SUN_NOW, &[("hour", hour_label(h)), ("el", one(el)), ("az", deg(az))]), Some(az),
                                 (180.0..=270.0).contains(&az) && el > 0.0),
        None => (String::new(), None, false),
    };
    let (photo_text, photo_az) = match photo {
        Some(p) => (fill(t::PHOTO_LIGHT, &[("el", one(p["sun_elevation_deg"].as_f64().unwrap_or(f64::NAN))),
                                            ("az", deg(p["sun_azimuth_deg"].as_f64().unwrap_or(f64::NAN))),
                                            ("obs", p["observation"].as_str().unwrap_or("").to_string()),
                                            ("time", format!("{:02}:{:02}", p["local_time_h"].as_f64().unwrap_or(0.0) as u32,
                                                             ((p["local_time_h"].as_f64().unwrap_or(0.0).fract()) * 60.0).round() as u32))]),
                     p["sun_azimuth_deg"].as_f64()),
        None => (String::new(), None),
    };
    let (mut wx_rows, wx_note, wx_source, wx_url, wx_disclaimer) = weather(wx, st.sol);
    let (rad_row, rad_note) = radiation(rad, st.sol);
    wx_rows.extend(rad_row);
    let (sw_text, sw_events) = space_weather(rad);
    LightModel {
        sol: st.sol,
        sol_range: SOL_RANGE,
        earth_date: fmt_date((y, mo, d)),
        date: fill(t::SOL_DATE, &[("sol", format!("{}", st.sol as i64)), ("date", fmt_date((y, mo, d))), ("ls", one(ls))]),
        intro: fill(t::INTRO, &[("lo", one(BAND.0)), ("hi", one(BAND.1))]),
        ties: fill(t::TIES, &[("low", one(LOW_SUN_DEG)), ("cone", one(TOWARD_CONE_DEG))]),
        rows,
        status,
        shadow_text: st.sun.map(|(_, _, s, h)| fill(t::SHADOW_LABEL, &[("hour", hour_label(h)), ("sol", format!("{}", s as i64))])).unwrap_or_default(),
        sun_text,
        sun_az,
        photo_text,
        photo_az,
        warn_south: warn,
        curve: Vec::new(),
        band: BAND,
        hour_label: st.hour.map(hour_label).unwrap_or_default(),
        wx_title: fill(t::WX_TITLE, &[("sol", format!("{}", st.sol as i64))]),
        wx_rows,
        wx_note,
        wx_source,
        wx_url,
        wx_disclaimer,
        rad_note,
        rad_rate: rad_rate(rad),
        sw_text,
        sw_events,
    }
}

/// Sun height along the walk (metres from the start, degrees) for a start hour: one point per ~10 route points.
pub fn curve(poly: &[(f64, f64)], legs: &[(usize, usize, f64)], sol: f64, hour: f64, site: (f64, f64)) -> Vec<(f64, f64)> {
    let route = route_core::light::timed_route(poly, legs);
    let mut d = 0.0;
    let mut out = Vec::new();
    for (i, p) in route.iter().enumerate() {
        if i > 0 {
            d += ((p.0 - route[i - 1].0).powi(2) + (p.1 - route[i - 1].1).powi(2)).sqrt();
        }
        if i % 10 == 0 || i + 1 == route.len() {
            let s = route_core::sun::sun_at_sol(sol, hour + p.2 / route_core::light::MARS_HOUR_S, site.0, site.1);
            out.push((d, s.elevation));
        }
    }
    out
}

/// Ask for the hour table when the sol or the route changes, the shadow when the sol or hour changes; apply answers.
#[allow(clippy::too_many_arguments)]
pub fn update_light(
    inputs: Option<Res<crate::plan::planner::PlanInputs>>,
    last: Option<Res<crate::plan::share::LastPlan>>,
    docs: Option<Res<crate::BundleDocs>>,
    st: Option<ResMut<SunLight>>,
    mut compute: ResMut<Compute>,
    mut overlays: Option<ResMut<crate::scene::trust::Overlays>>,
    mut mats: ResMut<Assets<crate::scene::trust::TerrainMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut bridge: ResMut<UiBridge>,
    mut started: Local<bool>,
    #[allow(unused_mut, unused_variables)] mut written: Local<Option<(f64, Option<f64>)>>,
) {
    let (Some(inputs), Some(last), Some(mut st)) = (inputs, last, st) else { return };
    if last.poly.is_empty() {
        return;
    }
    let mut changed = false;
    let mut want_hours = !*started || last.is_changed();
    *started = true;
    for e in EVENTS.lock().map(|mut q| q.drain(..).collect::<Vec<_>>()).unwrap_or_default() {
        match e {
            LightEvent::Sol(x) if x.is_finite() => {
                st.sol = x.round().clamp(SOL_RANGE.0, SOL_RANGE.1);
                st.hour = None;
                st.hours = None;
                want_hours = true;
            }
            LightEvent::Hour(h) => st.hour = Some(h),
            _ => {}
        }
        changed = true;
    }
    // the address holds the sol and start hour too (after the planner has written its link)
    #[cfg(target_arch = "wasm32")]
    if *written != Some((st.sol, st.hour)) {
        let q = crate::plan::share::web::query();
        if q.contains("v=") {
            crate::plan::share::web::set_query(&crate::plan::share::with_light(&q, st.sol, st.hour));
            *written = Some((st.sol, st.hour));
        }
    }
    let light = |sol: f64, hour: f64| LightJob { sol, hour, west_lon: inputs.site.0, lat_pg: inputs.site.1, poly: last.poly.clone(),
                                                 legs: last.legs.clone() };
    if want_hours {
        st.next_id += 1;
        st.hours_id = st.next_id;
        let rules = inputs.rules(&inputs.defaults());
        compute.submit(Job { id: st.hours_id, mode: Mode::Hours, rules, stops: Vec::new(), limit: 0.0, light: Some(light(st.sol, 0.0)) }, &inputs);
        changed = true;
    }
    for d in ANSWERS.lock().map(|mut q| q.drain(..).collect::<Vec<_>>()).unwrap_or_default() {
        match d.mode {
            Mode::Hours if d.id == st.hours_id => {
                if let Some(hs) = d.hours {
                    if st.hour.is_none() {
                        st.hour = match best(&hs) {
                            Ok(i) => Some(hs[i].start_ltst),
                            Err(n) => n.map(|i| hs[i].start_ltst),
                        };
                    }
                    st.hours = Some(hs);
                    changed = true;
                }
            }
            Mode::Shadow if d.id == st.shadow_id => {
                if let Some((m, el, az)) = d.mask {
                    // replace the whole image: GPU-only images drop their CPU copy after upload, so an in-place write is lost
                    if m.len() == inputs.w * inputs.h {
                        let _ = images.insert(st.mask.id(), crate::scene::trust::shadow_image_from(inputs.w, inputs.h, m));
                    }
                    let (s, h) = st.asked_shadow.unwrap_or((st.sol, 0.0));
                    st.sun = Some((el, az, s, h));
                    if let Some(o) = overlays.as_mut() {
                        o.has_shadow = true;
                        o.push(&mut mats);
                    }
                    changed = true;
                }
            }
            _ => {}
        }
    }
    if let Some(h) = st.hour {
        if st.asked_shadow != Some((st.sol, h)) {
            st.asked_shadow = Some((st.sol, h));
            st.next_id += 1;
            st.shadow_id = st.next_id;
            let rules = inputs.rules(&inputs.defaults());
            compute.submit(Job { id: st.shadow_id, mode: Mode::Shadow, rules, stops: Vec::new(), limit: 0.0, light: Some(light(st.sol, h)) }, &inputs);
        }
    }
    if changed {
        let photo = docs.as_ref().map(|d| &d.manifest["photo_light"]).filter(|v| v.is_object());
        let mut m = model(&st, inputs.site, photo, docs.as_ref().and_then(|d| d.conditions.as_ref()),
                          docs.as_ref().and_then(|d| d.radiation.as_ref()));
        if let Some(h) = st.hour {
            m.curve = curve(&last.poly, &last.legs, st.sol, h, inputs.site);
        }
        bridge.set_light(m);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weather_rows_come_from_the_file_and_gaps_stay_gaps() {
        let doc = serde_json::json!({"source": {"url": "u", "disclaimer": "d"}, "science_archive": {"last_sol": 4843},
            "sols": [{"sol": 4718, "air_min_c": -80, "air_max_c": -26, "ground_min_c": null, "ground_max_c": 5, "pressure_pa": 818,
                      "uv_index": "Moderate"}]});
        let (rows, note, source, url, _) = weather(Some(&doc), 4718.0);
        assert_eq!(rows.len(), 3, "ground skipped: one end missing");
        assert!(rows[0].1.contains("-80") && rows[0].1.contains("-26"));
        assert!(note.is_empty() && source.contains("4843") && url == "u");
        let (rows, note, ..) = weather(Some(&doc), 4719.0);
        assert!(rows.is_empty() && !note.is_empty(), "a sol without a record says so");
        assert!(weather(None, 4718.0).0.is_empty() && weather(None, 4718.0).1.is_empty(), "no file: no section");
    }

    #[test]
    fn radiation_only_for_archived_valid_stop_sols() {
        let doc = serde_json::json!({"sols": [
            {"sol": 4711, "archived": true, "valid": true, "b_n": 87, "b_median": 6.7, "b_p10": 6.2, "b_p90": 7.1},
            {"sol": 4618, "archived": false}],
            "space_weather": {"range": ["2025-05-08", "2026-04-13"], "events": [], "note": null}});
        let (row, note) = radiation(Some(&doc), 4711.0);
        assert!(row.unwrap().1.contains("6.7") && note.contains("87"));
        let (row, note) = radiation(Some(&doc), 4618.0);
        assert!(row.is_none() && note.contains("4618"), "archived=false: says no archived value");
        let (row, note) = radiation(Some(&doc), 4700.0);
        assert!(row.is_none() && note.contains("4711"), "not a stop sol: lists the stop sols");
        let (text, ev) = space_weather(Some(&doc));
        assert!(ev.is_empty() && text.contains("2025-05-08"));
        assert!(radiation(None, 4711.0).0.is_none() && space_weather(None).0.is_empty());
    }

    #[test]
    fn model_marks_the_best_hour_and_the_south_warning() {
        let row = |h: f64, pass: bool, toward: f64| HourResult { start_ltst: h, passes: pass, min_elev: 20.0, max_elev: 30.0, toward_low_sun_m: toward,
                                                        away_m: 0.0, shadow_m: 0.0, margin: 5.0, start_elev: 20.0, start_az: 90.0 };
        let mut st = SunLight::new(Handle::default());
        st.hours = Some(vec![row(7.0, false, 0.0), row(8.0, true, 10.0), row(9.0, true, 0.0)]);
        st.hour = Some(9.0);
        st.sun = Some((30.0, 250.0, 4718.0, 9.0));
        let m = model(&st, (222.62, -4.8623), None, None, None);
        assert!(m.rows[2].best && m.rows[2].selected && !m.rows[1].best, "{:?}", m.rows);
        assert!(m.rows[0].toward.is_empty(), "failing hours show no tie-break numbers");
        assert!(m.status.contains("09:00"), "{}", m.status);
        assert!(m.warn_south, "azimuth 250 is light from the south-west");
        assert!(m.date.contains("13 Nov 2025") && m.date.contains("4718"), "{}", m.date);
        st.sun = Some((30.0, 100.0, 4718.0, 9.0));
        assert!(!model(&st, (222.62, -4.8623), None, None, None).warn_south);
        st.hours = Some(vec![HourResult { start_elev: -0.3, start_az: 359.8, ..row(6.0, false, 0.0) }]);
        assert_eq!(model(&st, (222.62, -4.8623), None, None, None).rows[0].sun, "0 / 0", "no -0 and no 360");
    }

    #[test]
    fn eva_dose_rate_is_the_median_of_the_stop_sols() {
        let r: serde_json::Value = serde_json::from_slice(&std::fs::read("../assets/radiation.json").unwrap()).unwrap();
        let (rate, n) = rad_rate(Some(&r)).unwrap();
        assert_eq!(n, 6);
        // stop-sol medians 7.17/7.08/7.06/6.97/6.99/6.70: median of six = (6.99 + 7.06) / 2
        assert!((rate - 7.02).abs() < 0.02, "{rate}");
    }
}