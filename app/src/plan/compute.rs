//! Where the planner runs. In the browser every plan is computed in a Web Worker (crates/plan-worker), so the page never
//! freezes while route_core searches the grid (on the main thread one plan blocked the page for about 110 to 200 ms,
//! measured). Natively (tests) and if the worker cannot start, jobs run inline and answer at once. Either way the
//! answers arrive through `take()`, and the planner applies only the newest one it asked for.
use std::sync::Mutex;

use bevy::prelude::*;
use route_core::light::{box3, hours, timed_route, HourResult, Heights};
use route_core::sun::sun_at_sol;
use route_core::{plan_loop, plan_route, times_between, CostGrid, Plan, Rules};

use crate::plan::planner::PlanInputs;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// legs, route and the walk-back field
    Full,
    /// legs and route only (the robustness check)
    Route,
    /// walking times between every pair of stops (best order, time-budget fit)
    Times,
    /// DEM-scale shadow mask for a sol and hour (W3)
    Shadow,
    /// the best-hour table for a sol and the current route (W3)
    Hours,
}

/// What a light job needs (W3): the sol, hour, site (west longitude, planetographic latitude) and, for Hours, the route.
#[derive(Debug, Clone, Default)]
pub struct LightJob {
    pub sol: f64,
    pub hour: f64,
    pub west_lon: f64,
    pub lat_pg: f64,
    pub poly: Vec<(f64, f64)>,
    pub legs: Vec<(usize, usize, f64)>,
}

#[derive(Debug, Clone)]
pub struct Job {
    pub id: u64,
    pub mode: Mode,
    pub rules: Rules,
    pub stops: Vec<(f64, f64)>,
    pub limit: f64,
    pub light: Option<LightJob>,
}

#[derive(Debug, Clone)]
pub struct Done {
    pub id: u64,
    pub mode: Mode,
    pub limit: f64,
    /// Full and Route jobs
    pub plan: Option<Plan>,
    /// Times jobs (infinity = no way between the two stops)
    pub times: Option<Vec<Vec<f64>>>,
    /// Shadow jobs: one byte per cell (255 = shadow), and the sun (elevation, azimuth)
    pub mask: Option<(Vec<u8>, f64, f64)>,
    /// Hours jobs
    pub hours: Option<Vec<HourResult>>,
}

static DONE: Mutex<Vec<Done>> = Mutex::new(Vec::new());

pub fn take() -> Vec<Done> {
    DONE.lock().map(|mut q| q.drain(..).collect()).unwrap_or_default()
}

fn push(d: Done) {
    if let Ok(mut q) = DONE.lock() {
        q.push(d);
    }
}

/// Run a job here and now (native builds, tests, or no worker).
pub fn run_inline(job: &Job, inputs: &PlanInputs) -> Done {
    let g = CostGrid::new(&inputs.slope, inputs.w, inputs.h, inputs.cell, &job.rules, &inputs.rover);
    let mut d = Done { id: job.id, mode: job.mode, limit: job.limit, plan: None, times: None, mask: None, hours: None };
    match job.mode {
        Mode::Full => d.plan = Some(plan_loop(&g, &job.stops, job.limit)),
        Mode::Route => d.plan = Some(plan_route(&g, &job.stops, job.limit)),
        Mode::Times => d.times = Some(times_between(&g, &job.stops)),
        Mode::Shadow | Mode::Hours => {
            let l = job.light.clone().unwrap_or_default();
            let heights = Heights::new(box3(&inputs.heights, inputs.w, inputs.h), inputs.w, inputs.h, inputs.cell);
            if job.mode == Mode::Shadow {
                let s = sun_at_sol(l.sol, l.hour, l.west_lon, l.lat_pg);
                d.mask = Some((heights.shadow_mask(s.elevation as f32, s.azimuth as f32), s.elevation, s.azimuth));
            } else {
                d.hours = Some(hours(&timed_route(&l.poly, &l.legs), &heights, l.sol, l.west_lon, l.lat_pg));
            }
        }
    }
    d
}

#[derive(Resource, Default)]
pub struct Compute {
    #[cfg(target_arch = "wasm32")]
    web: Option<web::Worker>,
    /// worker creation failed: run inline
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    inline: bool,
}

impl Compute {
    pub fn submit(&mut self, job: Job, inputs: &PlanInputs) {
        #[cfg(target_arch = "wasm32")]
        if !self.inline {
            if self.web.is_none() {
                match web::Worker::start(inputs) {
                    Some(w) => self.web = Some(w),
                    None => {
                        warn!("planner worker could not start: planning on the main thread");
                        self.inline = true;
                    }
                }
            }
            if let Some(w) = self.web.as_mut() {
                w.send(job);
                return;
            }
        }
        push(run_inline(&job, inputs));
    }

    /// Send queued jobs once the worker reports it is ready (call every frame).
    pub fn pump(&mut self) {
        #[cfg(target_arch = "wasm32")]
        if let Some(w) = self.web.as_mut() {
            w.pump();
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    use std::sync::atomic::{AtomicBool, Ordering};

    use js_sys::{Array, Float32Array, Object, Reflect};
    use route_core::json::{hours_from, plan_from, points_to, rules_to};
    use serde_json::{json, Value};
    use wasm_bindgen::{prelude::*, JsCast};

    use super::{push, Done, Job, Mode, PlanInputs};

    static READY: AtomicBool = AtomicBool::new(false);

    pub struct Worker {
        w: web_sys::Worker,
        init: Option<Object>,
        queue: Vec<Job>,
    }

    fn msg(kind: &str, meta: &Value) -> Object {
        let o = Object::new();
        let _ = Reflect::set(&o, &"kind".into(), &kind.into());
        let _ = Reflect::set(&o, &"meta".into(), &meta.to_string().into());
        o
    }

    fn on_message(e: web_sys::MessageEvent) {
        let d = e.data();
        let kind = Reflect::get(&d, &"kind".into()).ok().and_then(|k| k.as_string()).unwrap_or_default();
        if kind == "ready" {
            READY.store(true, Ordering::SeqCst);
            return;
        }
        let Some(meta) = Reflect::get(&d, &"meta".into()).ok().and_then(|m| m.as_string())
            .and_then(|s| serde_json::from_str::<Value>(&s).ok()) else { return };
        let wb: Vec<f64> = Reflect::get(&d, &"walk_back".into()).ok().and_then(|a| a.dyn_into::<Float32Array>().ok())
            .map(|a| a.to_vec().into_iter().map(|t| if t.is_finite() { t as f64 } else { f64::INFINITY }).collect())
            .unwrap_or_default();
        let id = meta["id"].as_u64().unwrap_or(0);
        let limit = meta["limit"].as_f64().unwrap_or(f64::NAN);
        let empty = |mode| Done { id, mode, limit, plan: None, times: None, mask: None, hours: None };
        if meta["mode"] == "shadow" {
            let mask = Reflect::get(&d, &"mask".into()).ok().and_then(|a| a.dyn_into::<js_sys::Uint8Array>().ok()).map(|a| a.to_vec());
            if let Some(m) = mask {
                let sun = |i: usize| meta["sun"][i].as_f64().unwrap_or(f64::NAN);
                push(Done { mask: Some((m, sun(0), sun(1))), ..empty(Mode::Shadow) });
            }
            return;
        }
        if meta["mode"] == "hours" {
            push(Done { hours: Some(hours_from(&meta["hours"])), ..empty(Mode::Hours) });
            return;
        }
        if meta["mode"] == "times" {
            let t: Vec<Vec<f64>> = meta["times"].as_array().into_iter().flatten()
                .map(|r| r.as_array().into_iter().flatten().map(|x| x.as_f64().unwrap_or(f64::INFINITY)).collect()).collect();
            push(Done { times: Some(t), ..empty(Mode::Times) });
            return;
        }
        let mode = if meta["mode"] == "route" { Mode::Route } else { Mode::Full };
        if let Some(plan) = plan_from(&meta["plan"], wb) {
            push(Done { plan: Some(plan), ..empty(mode) });
        }
    }

    impl Worker {
        /// Start plan-worker.js (Trunk output next to the page) through a small loader script.
        pub fn start(inputs: &PlanInputs) -> Option<Worker> {
            let doc = web_sys::window()?.document()?;
            let base = doc.base_uri().ok()??;
            let url = |f: &str| web_sys::Url::new_with_base(f, &base).ok().map(|u| u.href());
            let (js, wasm) = (url("plan-worker.js")?, url("plan-worker_bg.wasm")?);
            let script = Array::of1(&format!("importScripts({js:?});wasm_bindgen({wasm:?});").into());
            let opts = web_sys::BlobPropertyBag::new();
            opts.set_type("text/javascript");
            let blob = web_sys::Blob::new_with_str_sequence_and_options(&script, &opts).ok()?;
            let w = web_sys::Worker::new(&web_sys::Url::create_object_url_with_blob(&blob).ok()?).ok()?;
            let cb = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(on_message);
            w.set_onmessage(Some(cb.as_ref().unchecked_ref()));
            cb.forget();
            let rover: Vec<(f64, f64)> = inputs.rover.clone();
            let init = msg("init", &json!({ "w": inputs.w, "h": inputs.h, "cell": inputs.cell, "rover": points_to(&rover) }));
            let _ = Reflect::set(&init, &"slope".into(), &Float32Array::from(inputs.slope.as_slice()));
            let _ = Reflect::set(&init, &"heights".into(), &Float32Array::from(inputs.heights.as_slice()));
            Some(Worker { w, init: Some(init), queue: Vec::new() })
        }

        pub fn send(&mut self, job: Job) {
            // only the newest job of each kind matters: drop older queued ones
            self.queue.retain(|j| j.mode != job.mode);
            self.queue.push(job);
            self.pump();
        }

        pub fn pump(&mut self) {
            if !READY.load(Ordering::SeqCst) {
                return;
            }
            if let Some(init) = self.init.take() {
                let _ = self.w.post_message(&init);
            }
            for j in self.queue.drain(..) {
                let mode = match j.mode { Mode::Full => "full", Mode::Route => "route", Mode::Times => "times",
                                          Mode::Shadow => "shadow", Mode::Hours => "hours" };
                let l = j.light.clone().unwrap_or_default();
                let meta = json!({ "id": j.id, "mode": mode, "sol": l.sol, "hour": l.hour, "west_lon": l.west_lon, "lat_pg": l.lat_pg,
                                   "poly": points_to(&l.poly), "legs": l.legs.iter().map(|&(a, b, s)| json!([a, b, s])).collect::<Vec<_>>(),
                                   "rules": rules_to(&j.rules), "stops": points_to(&j.stops), "limit": j.limit });
                let _ = self.w.post_message(&msg("job", &meta));
            }
        }
    }
}
