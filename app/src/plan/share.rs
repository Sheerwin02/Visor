//! Share link and plan export.
//!
//! The planner settings live in the page address (`?v=1&b=...&lim=20&spd=0.6&corr=1&stops=exit,bococo,@120.5_340.2&must=...&bud=19`),
//! so the address bar always reopens the plan on screen. `v` is the link format version; `b` is a short id of the data bundle
//! (a non-cryptographic hash of the manifest's per-file sha256 values): a link made with another bundle still opens, with a
//! warning. Custom stops are written as `@east_north` in metres. The export is one JSON file: the plan in route.json's schema
//! plus provenance (app version, bundle id, each bundle file's sha256 from the manifest, data sources, the share link).
use std::sync::Mutex;

use bevy::prelude::*;
use serde_json::{json, Value};

use crate::plan::planner::{PlanInputs, PlanSettings};
use crate::text::{fill, share as t};
use crate::scene::walk::Stop;

pub const LINK_VERSION: u32 = 1;

/// The plan on screen, in route.json's schema, kept for the export buttons.
#[derive(Resource, Default)]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))] // read by the browser's export buttons
pub struct LastPlan {
    pub route: Value,
    pub stops: Vec<Value>,
    /// the route (east, north metres) and its legs (first and last polyline index, walking time s), for the light table
    pub poly: Vec<(f64, f64)>,
    pub legs: Vec<(usize, usize, f64)>,
}

/// What the share buttons ask for (queued by the web panel, handled by `handle`).
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub enum ShareAction {
    CopyLink,
    Json,
    Csv,
    GeoJson,
}

pub static ACTIONS: Mutex<Vec<ShareAction>> = Mutex::new(Vec::new());

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn act(a: ShareAction) {
    if let Ok(mut q) = ACTIONS.lock() {
        q.push(a);
    }
}

/// Carry out queued share actions (browser only; on native builds the queue is just emptied).
pub fn handle(last: Option<Res<LastPlan>>, docs: Option<Res<crate::BundleDocs>>, edits: Res<crate::plan::edits::StationEdits>,
              mut bridge: ResMut<crate::ui::model::UiBridge>, placed: Option<Res<crate::scene::place::Placements>>) {
    let actions: Vec<ShareAction> = ACTIONS.lock().map(|mut q| q.drain(..).collect()).unwrap_or_default();
    let (Some(last), Some(docs)) = (last, docs) else { return };
    for a in actions {
        #[cfg(target_arch = "wasm32")]
        match a {
            ShareAction::CopyLink => {
                let msg = if web::copy(&web::href()) { t::COPIED } else { t::COPY_FAILED };
                if let Some(mut m) = bridge.panel.clone() {
                    m.controls.note = msg.to_string();
                    bridge.set_panel(m);
                }
            }
            ShareAction::Json => {
                let v = export_json(&last.route, &last.stops, &docs.manifest, &web::href(), &web::now_utc());
                web::download("visor_plan.json", "application/json", &serde_json::to_string_pretty(&v).unwrap_or_default());
            }
            ShareAction::Csv => web::download("visor_legs.csv", "text/csv", &export_csv(&last.route, &last.stops)),
            ShareAction::GeoJson => {
                let v = export_geojson(&last.poly, &last.stops, &docs.stops_doc, docs.rover_check.as_ref(), &docs.manifest, &edits,
                                       &placed.as_ref().map(|p| p.p.clone()).unwrap_or_default());
                web::download("visor_eva.geojson", "application/geo+json", &serde_json::to_string_pretty(&v).unwrap_or_default());
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = (a, &last, &docs, &edits, &mut bridge, &placed);
    }
}

/// Short id of the data bundle: FNV-1a 64 over the manifest's file names and sha256 values, in manifest order (12 hex digits).
pub fn bundle_id(manifest: &Value) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    if let Some(files) = manifest["files"].as_object() {
        for (name, f) in files {
            for b in name.bytes().chain(f["sha256"].as_str().unwrap_or("").bytes()) {
                h ^= b as u64;
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }
    format!("{:012x}", h >> 16)
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn num(x: f64) -> String {
    let r = (x * 10.0).round() / 10.0;
    format!("{r}")
}

/// The query string (without '?') for these settings.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn encode(s: &PlanSettings, bundle: &str) -> String {
    let stops: Vec<String> = s.stops.iter().map(|x| {
        if x.id.starts_with("custom-") { format!("@{}_{}", num(x.e as f64), num(x.n as f64)) } else { x.id.clone() }
    }).collect();
    let mut q = format!("v={LINK_VERSION}&b={bundle}&lim={}&spd={}&corr={}&stops={}", num(s.limit_deg), num(s.flat_mps),
                        if s.correct { 1 } else { 0 }, stops.join(","));
    let must: Vec<String> = s.stops.iter().enumerate().filter(|(_, x)| s.must.contains(&x.id))
        .map(|(i, x)| if x.id.starts_with("custom-") { format!("{i}") } else { x.id.clone() }).collect();
    if !must.is_empty() {
        q += &format!("&must={}", must.join(","));
    }
    q + &format!("&bud={}", num(s.budget_min))
}

/// Keys the sun section adds to the link: the sol and the start hour (local true solar time, whole hours).
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
const LIGHT_KEYS: [&str; 2] = ["sol", "hr"];

/// `query` with its sol/hour replaced by these (hour None: the best hour is chosen again when the link opens).
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn with_light(query: &str, sol: f64, hour: Option<f64>) -> String {
    let mut parts: Vec<String> = query.trim_start_matches('?').split('&').filter(|kv| !kv.is_empty())
        .filter(|kv| !LIGHT_KEYS.iter().any(|k| kv.split('=').next() == Some(*k))).map(str::to_string).collect();
    parts.push(format!("sol={}", sol.round() as i64));
    if let Some(h) = hour {
        parts.push(format!("hr={}", h.round() as i64));
    }
    parts.join("&")
}

/// The sol/hour part of an existing query, kept when the planner rewrites the address.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn light_part(query: &str) -> Option<(f64, Option<f64>)> {
    let sol = param(query, "sol")?.parse::<f64>().ok()?;
    Some((sol, param(query, "hr").and_then(|x| x.parse::<f64>().ok())))
}

/// Sol and start hour from a link; out-of-range values are skipped with a warning (same rule as the plan settings).
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn decode_light(query: &str, sols: (f64, f64), hours: (f64, f64)) -> (Option<f64>, Option<f64>, Vec<String>) {
    let mut warn = Vec::new();
    if param(query, "v").is_none() {
        return (None, None, warn);
    }
    let mut pick = |key: &str, (lo, hi): (f64, f64)| match param(query, key).map(|x| x.parse::<f64>()) {
        None => None,
        Some(Ok(x)) if (lo..=hi).contains(&x) && x.fract() == 0.0 => Some(x),
        Some(_) => {
            warn.push(fill(t::BAD_VALUE, &[("x", key.into())]));
            None
        }
    };
    let sol = pick("sol", sols);
    let hr = pick("hr", hours);
    (sol, hr, warn)
}

/// Keys for the placed relay and working spots (scene/place.rs): rl=E_N, ws=ID@E_N,ID@E_N (metres, 0.1 m).
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub const PLACE_KEYS: [&str; 2] = ["rl", "ws"];

/// `query` with its placed-point keys replaced by these.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn with_placed(query: &str, p: &crate::scene::place::Placed) -> String {
    let mut parts: Vec<String> = query.trim_start_matches('?').split('&').filter(|kv| !kv.is_empty())
        .filter(|kv| !PLACE_KEYS.iter().any(|k| kv.split('=').next() == Some(*k))).map(str::to_string).collect();
    if let Some((e, n)) = p.relay {
        parts.push(format!("rl={e:.1}_{n:.1}"));
    }
    if !p.work.is_empty() {
        parts.push(format!("ws={}", p.work.iter().map(|(id, (e, n))| format!("{id}@{e:.1}_{n:.1}")).collect::<Vec<_>>().join(",")));
    }
    parts.join("&")
}

/// Placed points from a link (bad pieces are skipped).
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn decode_placed(query: &str) -> crate::scene::place::Placed {
    let pt = |v: &str| v.split_once('_').and_then(|(a, b)| Some((a.parse::<f32>().ok()?, b.parse::<f32>().ok()?)))
        .filter(|p| p.0.is_finite() && p.1.is_finite());
    crate::scene::place::Placed {
        relay: param(query, "rl").and_then(pt),
        work: param(query, "ws").map(|v| v.split(',').filter_map(|w| w.split_once('@').and_then(|(id, at)| Some((id.to_string(), pt(at)?))))
            .filter(|w| !w.0.is_empty() && w.0.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')).collect()).unwrap_or_default(),
    }
}

fn param<'a>(q: &'a str, key: &str) -> Option<&'a str> {
    q.trim_start_matches('?').split('&').find_map(|kv| kv.split_once('=').filter(|(k, _)| *k == key).map(|(_, v)| v))
}

/// Settings from a query string. No `v` = an ordinary address: the published plan, no warning. Anything the link gets wrong
/// is skipped and named in the returned warnings (shown in the panel's note); a link never stops the app from opening.
pub fn decode(query: &str, inputs: &PlanInputs, bundle: &str) -> (PlanSettings, Vec<String>) {
    let mut s = inputs.defaults();
    let mut warn = Vec::new();
    let Some(v) = param(query, "v") else { return (s, warn) };
    if v != LINK_VERSION.to_string() {
        warn.push(fill(t::BAD_VERSION, &[("v", v.to_string())]));
        return (s, warn);
    }
    if param(query, "b").is_some_and(|b| b != bundle) {
        warn.push(t::OTHER_BUNDLE.to_string());
    }
    let lr = &inputs.assumptions["limit_range_deg"];
    let (lo, hi) = (lr[0].as_f64().unwrap_or(0.0), lr[1].as_f64().unwrap_or(90.0));
    if let Some(x) = param(query, "lim").and_then(|x| x.parse::<f64>().ok()) {
        if (lo..=hi).contains(&x) { s.limit_deg = x } else { warn.push(fill(t::BAD_VALUE, &[("x", "lim".into())])) }
    }
    if let Some(x) = param(query, "spd").and_then(|x| x.parse::<f64>().ok()) {
        if x > 0.0 && x <= 5.0 { s.flat_mps = x } else { warn.push(fill(t::BAD_VALUE, &[("x", "spd".into())])) }
    }
    if let Some(x) = param(query, "corr") {
        s.correct = x != "0";
    }
    if let Some(x) = param(query, "bud").and_then(|x| x.parse::<f64>().ok()).filter(|x| *x > 0.0) {
        s.budget_min = x;
    }
    let exit = inputs.published_stops[0].clone();
    let (w, h) = (inputs.w as f64 * inputs.cell, inputs.h as f64 * inputs.cell);
    if let Some(list) = param(query, "stops") {
        let mut stops = vec![exit.clone()];
        for item in list.replace("%2C", ",").split(',').filter(|x| !x.is_empty()) {
            if let Some(en) = item.strip_prefix('@').or_else(|| item.strip_prefix("%40")) {
                let p: Option<(f64, f64)> = en.split_once('_').and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)));
                match p.filter(|(e, n)| (0.0..w).contains(e) && (0.0..h).contains(n)) {
                    Some((e, n)) => {
                        s.custom_count += 1;
                        let k = s.custom_count;
                        let st = Stop { id: format!("custom-{k}"), name: fill(crate::text::plan_panel::CUSTOM_STOP, &[("k", k.to_string())]),
                                        e: e as f32, n: n as f32 };
                        s.pool.push(st.clone());
                        stops.push(st);
                    }
                    None => warn.push(fill(t::BAD_STOP, &[("x", item.to_string())])),
                }
            } else if item == exit.id {
                // the exit is always first; a repeated exit is ignored
            } else if let Some(st) = inputs.published_stops.iter().find(|x| x.id == item) {
                if !stops.iter().any(|x| x.id == st.id) {
                    stops.push(st.clone());
                }
            } else {
                warn.push(fill(t::BAD_STOP, &[("x", item.to_string())]));
            }
        }
        if stops.len() >= 2 { s.stops = stops } else { warn.push(t::TOO_FEW.to_string()) }
    }
    if let Some(list) = param(query, "must") {
        for item in list.replace("%2C", ",").split(',') {
            let id = item.parse::<usize>().ok().and_then(|i| s.stops.get(i)).map(|x| x.id.clone())
                .or_else(|| s.stops.iter().find(|x| x.id == item).map(|x| x.id.clone()));
            if let Some(id) = id.filter(|id| *id != exit.id && !s.must.contains(id)) {
                s.must.push(id);
            }
        }
    }
    (s, warn)
}

/// The export file: the plan (route.json schema), its stops, the settings, and where every input came from.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn export_json(route: &Value, stops: &[Value], manifest: &Value, link: &str, generated_utc: &str) -> Value {
    let files: Vec<Value> = manifest["files"].as_object().map(|m| m.iter().map(|(name, f)| json!({
        "name": name, "sha256": f["sha256"], "bytes": f["bytes"] })).collect()).unwrap_or_default();
    json!({
        "what": t::EXPORT_WHAT,
        "plan": route,
        "stops": stops,
        "provenance": {
            "generated_utc": generated_utc,
            "app": { "name": "VISOR", "version": env!("CARGO_PKG_VERSION") },
            "share_link": link,
            "bundle": { "id": bundle_id(manifest), "name": manifest["bundle"], "version": manifest["version"],
                        "built_utc": manifest["built_utc"], "files": files },
            "sources": manifest["sources"],
            "measured_values": manifest["measured_values"],
            "how": t::EXPORT_HOW,
        },
    })
}

/// The legs as CSV (one row per leg).
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn export_csv(route: &Value, stops: &[Value]) -> String {
    let mut out = String::from("leg,from,to,length_m,time_s,max_corrected_slope_deg,share_in_rover_verified,possible\n");
    let name = |i: usize| stops.get(i % stops.len().max(1)).and_then(|s| s["name"].as_str()).unwrap_or("").replace(',', " ");
    for (i, l) in route["legs"].as_array().into_iter().flatten().enumerate() {
        out += &format!("{},{},{},{},{},{},{},{}\n", i + 1, name(i), name(i + 1), l["length_m"], l["time_s"],
                        l["max_corrected_slope_deg"], l["share_in_rover_verified"], l["possible"]);
    }
    out
}

/// GeoJSON (RFC 7946 coordinate order lon, lat; Mars 2000 sphere, planetocentric, east longitude: the grid's own
/// projection, see the manifest): the route as a LineString, each station as a Point with its science fields (question,
/// bags, walk-back, rover check), and Curiosity's rover-checked stops as Points. For QGIS / JMARS / Mars Trek.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn export_geojson(poly: &[(f64, f64)], stops: &[Value], stops_doc: &Value, rover: Option<&Value>, manifest: &Value,
                      edits: &crate::plan::edits::StationEdits, placed: &crate::scene::place::Placed) -> Value {
    let g = &manifest["grid"];
    let ll = |e: f64, n: f64| crate::scene::basemap::latlon(g, e, n).map(|(la, lo)| json!([round6(lo), round6(la)]));
    let mut f = Vec::new();
    let line: Vec<Value> = poly.iter().filter_map(|&(e, n)| ll(e, n)).collect();
    if line.len() > 1 {
        f.push(json!({"type": "Feature", "geometry": {"type": "LineString", "coordinates": line},
                      "properties": {"kind": "route", "note": "planned walking route (VISOR)"}}));
    }
    for (i, s) in stops.iter().enumerate() {
        let (Some(e), Some(n)) = (s["e_m"].as_f64(), s["n_m"].as_f64()) else { continue };
        let id = s["id"].as_str().unwrap_or("");
        let d = stops_doc["stops"].as_array().and_then(|a| a.iter().find(|x| x["id"] == id));
        let tv = crate::plan::edits::tasks_value(id, stops_doc, edits);
        let tasks = tv.as_array();
        let question = tasks.and_then(|t| t.iter().find(|x| x["step"] == "question")).and_then(|x| x["text"].as_str());
        let bags: Vec<Value> = tasks.map(|t| t.iter().filter_map(|x| x["bags"].as_array()).flatten().cloned().collect()).unwrap_or_default();
        f.push(json!({"type": "Feature", "geometry": {"type": "Point", "coordinates": ll(e, n)},
                      "properties": {"kind": "station", "order": i + 1, "id": id, "name": s["name"], "sol": d.map(|x| x["sol"].clone()),
                                     "question": question, "bags": bags, "walk_back_s": s["walk_back_s"],
                                     "rover_check": d.map(|x| x["rover_check"].clone()), "drill_pair": d.is_some_and(|x| x["pair"].is_object()),
                                     "plan_by": if edits.0.contains_key(id) { "planner" } else if d.is_some() { "VISOR suggestion from cited facts" } else { "none" }}}));
    }
    for r in rover.and_then(|r| r["stops"].as_array()).into_iter().flatten() {
        let (Some(e), Some(n)) = (r["e_m"].as_f64(), r["n_m"].as_f64()) else { continue };
        f.push(json!({"type": "Feature", "geometry": {"type": "Point", "coordinates": ll(e, n)},
                      "properties": {"kind": "rover_check", "sol": r["sol"], "label": r["label"], "map_tilt_deg": r["dem_tilt"],
                                     "rover_tilt_deg": r["rover_tilt"]}}));
    }
    // points the planner placed (scene/place.rs): relay / EV2 position and working spots beside steep targets
    if let Some((e, n)) = placed.relay {
        f.push(json!({"type": "Feature", "geometry": {"type": "Point", "coordinates": ll(e as f64, n as f64)},
                      "properties": {"kind": "relay", "note": "relay / EV2 position placed by the planner (terrain-only sight test on the 1 m map)"}}));
    }
    for (id, (e, n)) in &placed.work {
        f.push(json!({"type": "Feature", "geometry": {"type": "Point", "coordinates": ll(*e as f64, *n as f64)},
                      "properties": {"kind": "working_spot", "station_id": id, "note": "where to stand to work at the station's steep target (placed by the planner)"}}));
    }
    json!({"type": "FeatureCollection", "name": "VISOR EVA plan",
           "crs_note": format!("Mars, {} ; coordinates lon (east), lat (planetocentric) in degrees", g["projection"].as_str().unwrap_or("")),
           "features": f})
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn round6(x: f64) -> f64 {
    (x * 1e6).round() / 1e6
}

#[cfg(target_arch = "wasm32")]
pub mod web {
    //! Browser side: keep the address bar in step with the plan, copy the link, download files.
    use wasm_bindgen::{JsCast, JsValue};

    pub fn query() -> String {
        web_sys::window().and_then(|w| w.location().search().ok()).unwrap_or_default()
    }

    /// Replace the address (no new history entry), keeping `mode=walk` if present.
    pub fn set_query(q: &str) {
        let Some(w) = web_sys::window() else { return };
        let keep = query().split('&').find(|kv| kv.trim_start_matches('?') == "mode=walk").map(|_| "&mode=walk").unwrap_or("");
        let path = w.location().pathname().unwrap_or_default();
        if let Ok(h) = w.history() {
            let _ = h.replace_state_with_url(&JsValue::NULL, "", Some(&format!("{path}?{q}{keep}")));
        }
    }

    pub fn href() -> String {
        web_sys::window().and_then(|w| w.location().href().ok()).unwrap_or_default()
    }

    /// navigator.clipboard.writeText(text), looked up at run time (the typed web-sys API is still behind an unstable flag).
    pub fn copy(text: &str) -> bool {
        let Some(w) = web_sys::window() else { return false };
        let nav: JsValue = w.navigator().into();
        let Ok(clip) = js_sys::Reflect::get(&nav, &"clipboard".into()) else { return false };
        let Ok(f) = js_sys::Reflect::get(&clip, &"writeText".into()) else { return false };
        f.dyn_ref::<js_sys::Function>().is_some_and(|f| f.call1(&clip, &text.into()).is_ok())
    }

    pub fn now_utc() -> String {
        js_sys::Date::new_0().to_iso_string().into()
    }

    /// Offer `text` as a file download.
    pub fn download(name: &str, mime: &str, text: &str) {
        let Some(doc) = web_sys::window().and_then(|w| w.document()) else { return };
        let parts = js_sys::Array::of1(&text.into());
        let opts = web_sys::BlobPropertyBag::new();
        opts.set_type(mime);
        let Ok(blob) = web_sys::Blob::new_with_str_sequence_and_options(&parts, &opts) else { return };
        let Ok(url) = web_sys::Url::create_object_url_with_blob(&blob) else { return };
        if let Ok(a) = doc.create_element("a").map(|e| e.unchecked_into::<web_sys::HtmlAnchorElement>()) {
            a.set_href(&url);
            a.set_download(name);
            a.click();
        }
        let _ = web_sys::Url::revoke_object_url(&url);
    }
}

#[cfg(test)]
mod tests {
    use super::{decode_placed, with_placed};

    #[test]
    fn placed_points_round_trip_through_the_link() {
        let p = crate::scene::place::Placed { relay: Some((120.25, 300.0)), work: vec![("rio_frio".into(), (10.0, 20.5)), ("custom-3".into(), (1.0, 2.0))] };
        let q = with_placed("v=1&lim=20&rl=1_1", &p);
        assert!(q.starts_with("v=1&lim=20&") && q.matches("rl=").count() == 1, "{q}");
        let back = decode_placed(&q);
        assert!((back.relay.unwrap().0 - 120.2).abs() < 0.11);
        assert_eq!(back.work.len(), 2);
        assert_eq!(back.work[0].0, "rio_frio");
        assert_eq!(with_placed(&q, &Default::default()), "v=1&lim=20");
        let bad = decode_placed("rl=x_1&ws=a@1_2,<b>@1_2,c@nope");
        assert!(bad.relay.is_none() && bad.work.len() == 1);
    }

    #[test]
    fn geojson_has_route_stations_and_rover_points_in_lon_lat() {
        let m: Value = serde_json::from_slice(&std::fs::read("../assets/manifest.json").unwrap()).unwrap();
        let sd: Value = serde_json::from_slice(&std::fs::read("../assets/stops.json").unwrap()).unwrap();
        let rc: Value = serde_json::from_slice(&std::fs::read("../assets/rover_check.json").unwrap()).unwrap();
        let stops: Vec<Value> = sd["stops"].as_array().unwrap().clone();
        let poly: Vec<(f64, f64)> = stops.iter().map(|s| (s["e_m"].as_f64().unwrap(), s["n_m"].as_f64().unwrap())).collect();
        let pl = crate::scene::place::Placed { relay: Some((100.0, 200.0)), work: vec![("rio_frio".into(), (10.0, 20.0))] };
        let v = export_geojson(&poly, &stops, &sd, Some(&rc), &m, &Default::default(), &pl);
        let kinds: Vec<&str> = v["features"].as_array().unwrap().iter().filter_map(|f| f["properties"]["kind"].as_str()).collect();
        assert!(kinds.contains(&"relay") && kinds.contains(&"working_spot"), "{kinds:?}");
        let f = v["features"].as_array().unwrap();
        assert_eq!(f.iter().filter(|x| x["properties"]["kind"] == "station").count(), 6);
        assert_eq!(f.iter().filter(|x| x["properties"]["kind"] == "rover_check").count(), 85);
        let valle = f.iter().find(|x| x["properties"]["id"] == "valle_de_la_luna").unwrap();
        let (lon, lat) = (valle["geometry"]["coordinates"][0].as_f64().unwrap(), valle["geometry"]["coordinates"][1].as_f64().unwrap());
        let want = sd["stops"].as_array().unwrap().iter().find(|s| s["id"] == "valle_de_la_luna").unwrap();
        assert!((lon - want["lon"].as_f64().unwrap()).abs() < 2e-5 && (lat - want["lat"].as_f64().unwrap()).abs() < 2e-5, "{lon} {lat} vs {want}");
        assert_eq!(valle["properties"]["drill_pair"], true);
        assert_eq!(valle["properties"]["bags"].as_array().unwrap().len(), 1);
    }

    use super::*;
    use crate::plan::planner::tests_support::inputs;

    #[test]
    fn link_round_trip_reproduces_the_settings() {
        let inp = inputs();
        let mut s = inp.defaults();
        s.limit_deg = 18.0;
        s.flat_mps = 0.4;
        s.correct = false;
        s.stops.swap(1, 2);
        s.custom_count = 1;
        let c = Stop { id: "custom-1".into(), name: fill(crate::text::plan_panel::CUSTOM_STOP, &[("k", "1".into())]), e: 12.5, n: 15.0 };
        s.pool.push(c.clone());
        s.stops.push(c);
        s.must = vec![s.stops[2].id.clone(), "custom-1".into()];
        s.budget_min = 25.0;
        let q = encode(&s, "abc");
        let (back, warn) = decode(&q, &inp, "abc");
        assert!(warn.is_empty(), "{warn:?} {q}");
        assert_eq!(back.limit_deg, 18.0);
        assert_eq!(back.flat_mps, 0.4);
        assert!(!back.correct);
        assert_eq!(back.stops, s.stops, "{q}");
        let mut m1 = back.must.clone();
        m1.sort();
        let mut m2 = s.must.clone();
        m2.sort();
        assert_eq!(m1, m2);
        assert_eq!(back.budget_min, 25.0);
        assert_eq!(back.pool.len(), s.pool.len());
    }

    #[test]
    fn sol_and_hour_round_trip_and_survive_a_replan() {
        let q = with_light("v=1&b=x&lim=20", 4718.0, Some(8.0));
        assert_eq!(q, "v=1&b=x&lim=20&sol=4718&hr=8");
        assert_eq!(with_light(&q, 4600.0, None), "v=1&b=x&lim=20&sol=4600", "replaced, not repeated");
        assert_eq!(decode_light(&q, (4534.0, 4865.0), (6.0, 17.0)), (Some(4718.0), Some(8.0), vec![]));
        let (s, h, w) = decode_light("v=1&sol=99&hr=8.5", (4534.0, 4865.0), (6.0, 17.0));
        assert!(s.is_none() && h.is_none() && w.len() == 2, "out of range or not whole: skipped with a warning");
        assert_eq!(decode_light("sol=4718", (4534.0, 4865.0), (6.0, 17.0)).0, None, "no v: an ordinary address");
        let (sol, hr) = light_part(&q).unwrap();
        assert_eq!(with_light("v=1&b=y", sol, hr), "v=1&b=y&sol=4718&hr=8");
    }

    #[test]
    fn plain_address_gives_the_published_plan_and_bad_links_warn() {
        let inp = inputs();
        let (s, w) = decode("?mode=walk", &inp, "abc");
        assert!(w.is_empty() && s == inp.defaults());
        let (_, w) = decode("v=1&b=other&lim=99&stops=a,nope", &inp, "abc");
        assert_eq!(w.len(), 4, "{w:?}"); // other bundle, bad limit, unknown stop, too few stops
        let (s, w) = decode("v=9", &inp, "abc");
        assert_eq!(w.len(), 1);
        assert!(s == inp.defaults());
    }

    #[test]
    fn bundle_id_changes_with_any_file_hash() {
        let a = json!({"files": {"x.bin": {"sha256": "00"}, "y.json": {"sha256": "11"}}});
        let b = json!({"files": {"x.bin": {"sha256": "00"}, "y.json": {"sha256": "12"}}});
        assert_eq!(bundle_id(&a).len(), 12);
        assert_ne!(bundle_id(&a), bundle_id(&b));
    }

    #[test]
    fn export_names_every_bundle_file_with_its_hash() {
        let m = json!({"bundle": "B", "version": 1, "files": {"x.bin": {"sha256": "aa", "bytes": 3}, "y.json": {"sha256": "bb", "bytes": 4}},
                       "sources": [{"name": "S"}], "measured_values": {}});
        let v = export_json(&json!({"legs": []}), &[], &m, "http://x/?v=1", "2026-11-12T00:00:00Z");
        let files = v["provenance"]["bundle"]["files"].as_array().unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[1]["sha256"], "bb");
        assert_eq!(v["provenance"]["bundle"]["id"], bundle_id(&m));
        assert_eq!(v["provenance"]["share_link"], "http://x/?v=1");
    }

    #[test]
    fn csv_has_one_row_per_leg() {
        let route = json!({"legs": [{"length_m": 1.0, "time_s": 2.0, "max_corrected_slope_deg": 3.0, "share_in_rover_verified": 0.5, "possible": true},
                                    {"length_m": 4.0, "time_s": 5.0, "max_corrected_slope_deg": 6.0, "share_in_rover_verified": 0.0, "possible": true}]});
        let stops = vec![json!({"name": "Exit"}), json!({"name": "A, B"})];
        let csv = export_csv(&route, &stops);
        assert_eq!(csv.lines().count(), 3);
        assert!(csv.contains("2,A  B,Exit,"), "{csv}");
    }
}
