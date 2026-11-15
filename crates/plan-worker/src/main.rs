//! VISOR planner worker. The web app (app/src/compute.rs) sends:
//!   {kind: "init", meta: JSON {w, h, cell, rover: [[e, n]...]}, slope: Float32Array, heights: Float32Array}   once
//!   {kind: "job",  meta: JSON {id, mode: "full" | "route" | "times", rules, stops: [[e, n]...], limit}}
//! and gets back {kind: "ready"} once, then for each job {kind: "done", meta: JSON {id, mode, limit, plan}, walk_back: Float32Array}
//! (walk_back empty for "route"; "times" answers {id, mode, times: [[s]...]} instead of a plan, null = no way). Jobs run in the order sent; the app ignores answers to jobs it no longer needs.
//! Light jobs (W3): mode "shadow" {sol, hour, west_lon, lat_pg} answers {sun: [elev, az]} + mask: Uint8Array (255 = shadow,
//! one byte per cell); mode "hours" {sol, west_lon, lat_pg, poly, legs: [[start, end, s]...]} answers {hours: [...]}.
//! Messages sent before "ready" are lost (the wasm loads asynchronously), so the app waits for it.

#[cfg(target_arch = "wasm32")]
mod web {
    use std::cell::RefCell;

    use js_sys::{Float32Array, Object, Reflect, Uint8Array};
    use route_core::json::{hours_to, plan_to, points_from, rules_from};
    use route_core::light::{box3, hours, timed_route, Heights};
    use route_core::sun::sun_at_sol;
    use route_core::{plan_loop, plan_route, times_between, CostGrid};
    use serde_json::{json, Value};
    use wasm_bindgen::{prelude::*, JsCast};
    use web_sys::{DedicatedWorkerGlobalScope, MessageEvent};

    struct Grid {
        w: usize,
        h: usize,
        cell: f64,
        slope: Vec<f32>,
        rover: Vec<(f64, f64)>,
        /// 3 x 3 m box-smoothed heights for shadows
        heights: Heights,
    }

    thread_local! { static GRID: RefCell<Option<Grid>> = const { RefCell::new(None) }; }

    fn get(o: &JsValue, k: &str) -> JsValue {
        Reflect::get(o, &k.into()).unwrap_or(JsValue::UNDEFINED)
    }

    fn reply(scope: &DedicatedWorkerGlobalScope, kind: &str, meta: &Value, wb: Option<Float32Array>) {
        reply_with(scope, kind, meta, wb.map(|a| ("walk_back", JsValue::from(a))));
    }

    fn reply_with(scope: &DedicatedWorkerGlobalScope, kind: &str, meta: &Value, data: Option<(&str, JsValue)>) {
        let o = Object::new();
        let _ = Reflect::set(&o, &"kind".into(), &kind.into());
        let _ = Reflect::set(&o, &"meta".into(), &meta.to_string().into());
        match data {
            Some((name, a)) => {
                let _ = Reflect::set(&o, &name.into(), &a);
                let buf = Reflect::get(&a, &"buffer".into()).unwrap_or(JsValue::UNDEFINED);
                let transfer = js_sys::Array::of1(&buf);
                let _ = scope.post_message_with_transfer(&o, &transfer);
            }
            None => {
                let _ = scope.post_message(&o);
            }
        }
    }

    fn handle(scope: &DedicatedWorkerGlobalScope, data: JsValue) {
        let kind = get(&data, "kind").as_string().unwrap_or_default();
        let meta: Value = get(&data, "meta").as_string().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null);
        match kind.as_str() {
            "init" => {
                let slope = get(&data, "slope").dyn_into::<Float32Array>().map(|a| a.to_vec()).unwrap_or_default();
                let raw = get(&data, "heights").dyn_into::<Float32Array>().map(|a| a.to_vec()).unwrap_or_default();
                let (w, h) = (meta["w"].as_u64().unwrap_or(0) as usize, meta["h"].as_u64().unwrap_or(0) as usize);
                let cell = meta["cell"].as_f64().unwrap_or(1.0);
                let heights = Heights::new(if raw.len() == w * h { box3(&raw, w, h) } else { vec![0.0; w * h] }, w, h, cell);
                let g = Grid {
                    w: meta["w"].as_u64().unwrap_or(0) as usize,
                    h: meta["h"].as_u64().unwrap_or(0) as usize,
                    cell: meta["cell"].as_f64().unwrap_or(1.0),
                    rover: points_from(&meta["rover"]),
                    slope,
                    heights,
                };
                GRID.with(|s| *s.borrow_mut() = Some(g));
            }
            "job" => GRID.with(|s| {
                let s = s.borrow();
                let Some(g) = s.as_ref() else { return };
                let f = |k: &str| meta[k].as_f64().unwrap_or(f64::NAN);
                if meta["mode"] == "shadow" {
                    let sun = sun_at_sol(f("sol"), f("hour"), f("west_lon"), f("lat_pg"));
                    let mask = g.heights.shadow_mask(sun.elevation as f32, sun.azimuth as f32);
                    let out = json!({ "id": meta["id"], "mode": "shadow", "sun": [sun.elevation, sun.azimuth], "sol": f("sol"), "hour": f("hour") });
                    reply_with(scope, "done", &out, Some(("mask", JsValue::from(Uint8Array::from(mask.as_slice())))));
                    return;
                }
                if meta["mode"] == "hours" {
                    let legs: Vec<(usize, usize, f64)> = meta["legs"].as_array().into_iter().flatten()
                        .filter_map(|l| Some((l[0].as_u64()? as usize, l[1].as_u64()? as usize, l[2].as_f64()?))).collect();
                    let route = timed_route(&points_from(&meta["poly"]), &legs);
                    let hs = hours(&route, &g.heights, f("sol"), f("west_lon"), f("lat_pg"));
                    let out = json!({ "id": meta["id"], "mode": "hours", "sol": f("sol"), "hours": hours_to(&hs) });
                    reply(scope, "done", &out, None);
                    return;
                }
                let Some(rules) = rules_from(&meta["rules"]) else { return };
                if g.slope.len() != g.w * g.h {
                    return;
                }
                let stops = points_from(&meta["stops"]);
                let limit = meta["limit"].as_f64().unwrap_or(rules.limit_deg);
                let cg = CostGrid::new(&g.slope, g.w, g.h, g.cell, &rules, &g.rover);
                if meta["mode"] == "times" {
                    let t: Vec<Vec<Option<f64>>> = times_between(&cg, &stops).into_iter()
                        .map(|r| r.into_iter().map(|x| x.is_finite().then_some(x)).collect()).collect();
                    reply(scope, "done", &json!({ "id": meta["id"], "mode": "times", "times": t }), None);
                    return;
                }
                let route_only = meta["mode"] == "route";
                let plan = if route_only { plan_route(&cg, &stops, limit) } else { plan_loop(&cg, &stops, limit) };
                let out = json!({ "id": meta["id"], "mode": meta["mode"], "limit": limit, "plan": plan_to(&plan) });
                let wb = (!route_only).then(|| {
                    let f: Vec<f32> = plan.walk_back.iter().map(|&t| t as f32).collect();
                    Float32Array::from(f.as_slice())
                });
                reply(scope, "done", &out, wb);
            }),
            _ => {}
        }
    }

    pub fn start() {
        console_error_panic_hook::set_once();
        let scope = DedicatedWorkerGlobalScope::from(JsValue::from(js_sys::global()));
        let s2 = scope.clone();
        let onmessage = Closure::wrap(Box::new(move |m: MessageEvent| handle(&s2, m.data())) as Box<dyn Fn(MessageEvent)>);
        scope.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
        onmessage.forget();
        reply(&scope, "ready", &Value::Null, None);
    }
}

fn main() {
    #[cfg(target_arch = "wasm32")]
    web::start();
}
