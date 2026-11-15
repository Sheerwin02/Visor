//! Runs the EVA simulation (sim.rs) in Walk mode: rebuilds it when the plan or the crew's EVA plan (crew.rs EvaPlan: station
//! time, egress, ingress) changes, advances
//! the clock while playing, drives EV1 (the walker) from it unless the user takes manual control (W A S D pauses), and
//! publishes the transport / event log / clock model to the UI. The sun follows the clock (walkscene.rs reads `sun_now`).
use bevy::prelude::*;
use leafwing_input_manager::prelude::ActionState;
use std::sync::Mutex;

use crate::plan::sim::{self, EventKind, Sim};
use crate::scene::controls::Act;
use crate::scene::walk::{Mode, Pace, SlopeMap, Stops, Walker};
use crate::text::{fill, sim as t};
use crate::ui::model::UiBridge;

/// Playback speeds offered (x real time). VISUAL/UX choice.
pub const SPEEDS: [f32; 5] = [1.0, 10.0, 60.0, 300.0, 1200.0];

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub enum SimEvent {
    Toggle,
    Start,
    Prev,
    Next,
    Speed(f32),
    /// jump to the first time the walk reaches this distance along the route (m)
    SeekDist(f32),
    SeekTime(f32),
    Csv,
}

pub static EVENTS: Mutex<Vec<SimEvent>> = Mutex::new(Vec::new());

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn send(e: SimEvent) {
    if let Ok(mut q) = EVENTS.lock() {
        q.push(e);
    }
}

#[derive(Resource)]
pub struct SimRun {
    pub sim: Option<Sim>,
    pub t: f32,
    pub playing: bool,
    pub speed: f32,
    /// the user is steering (W A S D); the clock is paused and does not move the walker
    pub manual: bool,
    key: Option<(usize, usize, u64, u64)>,
    last_applied: f32,
    /// sun elevations over the EVA +- the twilight window, keyed by (sol, start hour, end s) bits (daylight.rs)
    /// and the sol's sunrise / sunset hours
    /// (and the equation of time at noon, deg: the mission clock LMST = LTST - EOT)
    day: Option<((u64, u64, u32), crate::plan::daylight::Series, (Option<f64>, Option<f64>, f64))>,
}

impl Default for SimRun {
    fn default() -> Self {
        SimRun { sim: None, t: 0.0, playing: false, speed: SPEEDS[1], manual: false, key: None, last_applied: -1.0, day: None }
    }
}

/// One row of the event log.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct LogRow {
    pub t: f32,
    pub clock: String,
    pub text: String,
    /// a safety alert raised from the route data (not a timetable event)
    pub alert: bool,
}

/// One block of the whole EVA on the time axis: "egress", "walk", "station", "ingress".
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Block {
    pub kind: &'static str,
    /// the stop (walking order) for a station block
    pub stop: Option<usize>,
    pub t0: f32,
    pub t1: f32,
    pub label: String,
}

/// The station the crew is at now: name, time left there, and its plan (step label, text, inference).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct StationNow {
    /// stop id (key of the EVA log)
    pub id: String,
    pub name: String,
    pub left: String,
    pub tasks: Vec<(&'static str, String, bool)>,
    pub egress: bool,
    /// the planner wrote this station's plan (no citations)
    pub yours: bool,
    /// Curiosity's Navcam photo taken here (stops.json photo) with its view direction: what the rover saw at this spot
    pub photo: Option<crate::ui::model::Photo>,
    /// the Navcam 3D point cloud belongs to this stop (manifest navcam_4718_points.bin "stop"): say what the patch is
    pub navcam3d: bool,
    /// Curiosity's Navcam 360-degree mosaic taken here (panos.json)
    pub pano: Option<crate::ui::model::Pano>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct SimModel {
    pub active: bool,
    pub playing: bool,
    pub manual: bool,
    pub speed: f32,
    pub speeds: Vec<f32>,
    pub t: f32,
    pub total: f32,
    /// distance walked (m) for the playhead on the distance-based EVA timeline
    pub dist: f32,
    pub clock: String,
    pub elapsed: String,
    pub task_left: String,
    pub dwell_note: String,
    pub log: Vec<LogRow>,
    /// index of the last log row at or before t
    pub current: Option<usize>,
    /// the latest alert within ALERT_SHOW_S of the simulation clock (Walk HUD banner)
    pub alert: Option<String>,
    /// the whole EVA on a time axis (blocks), the EVA length and its emergency reserve (s), the sun's elevation over it
    /// (t s, elevation deg; every 5 min from the start to the end of the reserve), the start hour's clock labels
    pub blocks: Vec<Block>,
    pub eva_s: f32,
    pub reserve_s: f32,
    pub sun: Vec<(f32, f32)>,
    pub start_h: f64,
    pub station: Option<StationNow>,
    /// twilight / night stretches on the EVA clock: (t0 s, t1 s, 1 twilight | 2 night)
    pub dark: Vec<(f32, f32, u8)>,
    /// first twilight / night inside the EVA or its reserve ("Twilight from 17:39"), for the timeline and the Walk HUD
    pub dark_note: Option<String>,
    /// light now (Walk HUD): label, class (day / twilight / night) and tooltip
    pub daylight: (String, &'static str, String),
}

/// Step label for a station task.
pub fn step_label(step: &str) -> &'static str {
    match step {
        "question" => t::STEP_QUESTION,
        "observe" => t::STEP_OBSERVE,
        "image" => t::STEP_IMAGE,
        "sample" => t::STEP_SAMPLE,
        _ => t::STEP_COMPARE,
    }
}

/// The station plan of a stop (stops.json tasks), by stop id.
pub fn tasks_of(tasks: &serde_json::Value) -> Vec<(&'static str, String, bool)> {
    tasks.as_array()
        .map(|ts| ts.iter().filter_map(|x| Some((step_label(x["step"].as_str()?), x["text"].as_str()?.to_string(),
                                                  x["inference"].as_bool().unwrap_or(false)))).collect())
        .unwrap_or_default()
}

/// The whole EVA as blocks (names in walking order).
pub fn blocks(s: &Sim, names: &[String]) -> Vec<Block> {
    let nm = |k: usize| names.get(k).cloned().unwrap_or_else(|| format!("{}", k + 1));
    let n = s.segs.len();
    s.segs.iter().enumerate().map(|(i, g)| match *g {
        sim::Seg::Walk { leg, t0, t1, .. } => Block { kind: "walk", stop: None, t0, t1, label: fill(t::BLOCK_WALK, &[("leg", format!("{}", leg + 1))]) },
        sim::Seg::Dwell { stop: 0, t0, t1 } if i == 0 => Block { kind: "egress", stop: None, t0, t1, label: t::BLOCK_EGRESS.to_string() },
        sim::Seg::Dwell { stop: 0, t0, t1 } if i + 1 == n => Block { kind: "ingress", stop: None, t0, t1, label: t::BLOCK_INGRESS.to_string() },
        sim::Seg::Dwell { stop, t0, t1 } => Block { kind: "station", stop: Some(stop), t0, t1, label: nm(stop) },
    }).collect()
}

/// Local true solar time at t, as HH:MM:SS (start hour + t in Mars hours).
pub fn clock(start_h: f64, t: f32) -> String {
    let h = start_h + t as f64 / route_core::light::MARS_HOUR_S;
    let s = (h * 3600.0).round() as i64;
    format!("{:02}:{:02}:{:02}", (s / 3600) % 24, (s / 60) % 60, s % 60)
}

/// h:mm for long spans (EVA time left).
pub fn hmm(t: f32) -> String {
    let m = (t.max(0.0) / 60.0).round() as i64;
    format!("{}:{:02}", m / 60, m % 60)
}

pub fn mmss(t: f32) -> String {
    let s = t.max(0.0).round() as i64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Event log texts (names in walking order: "2 Bococo").
pub fn log_rows(s: &Sim, names: &[String], start_h: f64) -> Vec<LogRow> {
    let nm = |k: usize| names.get(k).cloned().unwrap_or_else(|| format!("{}", k + 1));
    s.events.iter().map(|e| LogRow {
        t: e.t,
        clock: clock(start_h, e.t),
        text: match e.kind {
            EventKind::Depart(k) => fill(t::DEPART, &[("stop", nm(k))]),
            EventKind::Arrive(k) => fill(t::ARRIVE, &[("stop", nm(k))]),
            EventKind::Steepest(k, x) => fill(t::STEEPEST, &[("leg", format!("{}", k + 1)), ("x", format!("{}", x.round() as i64))]),
            EventKind::End => fill(t::END, &[("stop", nm(0))]),
            EventKind::Egress => fill(t::EGRESS, &[("x", mmss(s.egress_s))]),
            EventKind::Ingress => fill(t::INGRESS, &[("x", mmss(s.ingress_s))]),
            EventKind::Done => t::DONE.to_string(),
        },
        alert: false,
    }).collect()
}

/// Alerts a mission controller would call during the walk, from the plan's route profile at the time the walker reaches
/// each point: losing / regaining sight of the exit (terrain line of sight, 1.7 m eyes) and the start of each stretch whose
/// corrected slope is over the caution angle (its steepest value and length).
pub fn alerts(s: &Sim, samples: &[crate::ui::panel::Sample], caution: f32, limit: f32, start_h: f64) -> Vec<LogRow> {
    let mut out = Vec::new();
    let row = |t_s: f32, text: String| LogRow { t: t_s, clock: clock(start_h, t_s), text, alert: true };
    // sight changes along the route, with flicker removed: a lost-then-regained (or regained-then-lost) pair closer than
    // SIGHT_DEBOUNCE_S is dropped (walking along a sight edge would otherwise alert every few seconds)
    let mut changes: Vec<(f32, bool)> = samples.windows(2).filter(|w| w[0].seen != w[1].seen).map(|w| (s.time_at_dist(w[1].d), w[1].seen)).collect();
    let mut k = 0;
    while k + 1 < changes.len() {
        if changes[k + 1].0 - changes[k].0 < SIGHT_DEBOUNCE_S {
            changes.drain(k..k + 2);
            k = k.saturating_sub(1);
        } else {
            k += 1;
        }
    }
    for (ts, seen) in changes {
        out.push(row(ts, if seen { t::ALERT_SEEN } else { t::ALERT_UNSEEN }.to_string()));
    }
    // steep stretches: cells over the caution angle, merged across gaps under ALERT_GAP_M; a stretch alerts when it is at
    // least ALERT_MIN_M long or reaches the slope limit (single 1 m map cells just over the caution angle are noise, and
    // noisy alerts teach crews to ignore alerts)
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for (k, q) in samples.iter().enumerate() {
        if q.s >= caution {
            match runs.last_mut() {
                Some(r) if q.d - samples[r.1].d <= ALERT_GAP_M => r.1 = k,
                _ => runs.push((k, k)),
            }
        }
    }
    for (i, j) in runs {
        let mx = samples[i..=j].iter().map(|q| q.s).fold(0.0f32, f32::max);
        let len = samples[j].d - samples[i].d;
        if len < ALERT_MIN_M && mx < limit {
            continue;
        }
        // the alert comes a little before the stretch (about 10 m at walking pace)
        let t0 = s.time_at_dist((samples[i].d - 10.0).max(0.0));
        out.push(row(t0, fill(t::ALERT_STEEP, &[("x", format!("{}", mx.round() as i64)), ("d", format!("{}", len.max(1.0).round() as i64))])));
    }
    out
}

/// Steep-stretch alerts: shortest stretch that alerts below the limit, and the gap that still joins two stretches (m).
pub const ALERT_MIN_M: f32 = 5.0;
pub const ALERT_GAP_M: f32 = 4.0;
/// Sight changes closer than this in simulation time (s) cancel out (no flicker alerts; a long station visit out of sight
/// still alerts).
pub const SIGHT_DEBOUNCE_S: f32 = 60.0;

/// How long (simulation seconds) an alert stays on the Walk HUD. VISUAL.
pub const ALERT_SHOW_S: f32 = 45.0;

/// The sun (elevation, azimuth deg) at simulation time t and the light state it gives (daylight.rs), for walkscene.rs:
/// None outside Walk or without a plan.
#[derive(Resource, Default, Clone, Copy, PartialEq)]
pub struct SunNow(pub Option<(f64, f64)>, pub Option<crate::plan::daylight::Daylight>);

#[allow(clippy::too_many_arguments)]
pub fn update(
    time: Res<Time>,
    mode: Res<Mode>,
    acts: Query<&ActionState<Act>>,
    last: Option<Res<crate::plan::share::LastPlan>>,
    eva: Res<crate::plan::crew::EvaPlan>,
    edits: Res<crate::plan::edits::StationEdits>,
    slope: Option<Res<SlopeMap>>,
    pace: Option<Res<Pace>>,
    stops: Res<Stops>,
    inputs: Option<Res<crate::plan::planner::PlanInputs>>,
    sunlight: Option<Res<crate::plan::sunlight::SunLight>>,
    docs: Option<Res<crate::BundleDocs>>,
    walker: Option<ResMut<Walker>>,
    mut run: ResMut<SimRun>,
    mut sun_now: ResMut<SunNow>,
    (mut bridge, mut walk_target, mut log_cache): (ResMut<UiBridge>, Option<ResMut<crate::scene::walk::WalkTarget>>, Local<(u64, Vec<LogRow>)>),
) {
    let (Some(last), Some(slope), Some(pace)) = (last, slope, pace) else { return };
    if last.poly.is_empty() {
        return;
    }
    let dwell_s = eva.station_s as f32;
    let key = (last.poly.len(), last.legs.len(), eva.station_s.to_bits(), eva.egress_s.to_bits() ^ eva.ingress_s.to_bits().rotate_left(1));
    if run.key != Some(key) || last.is_changed() {
        let g = &slope.0;
        let s = sim::build(&last.poly, &last.legs, |e, n| pace.speed(g.height_at(e, n)), |e, n| (g.height_at(e, n) - pace.bias_deg).max(0.0), dwell_s,
                           eva.egress_s as f32, eva.ingress_s as f32);
        run.t = run.t.min(s.total);
        run.sim = Some(s);
        run.key = Some(key);
        run.last_applied = -1.0;
    }
    let total = run.sim.as_ref().map(|s| s.total).unwrap_or(0.0);
    for e in EVENTS.lock().map(|mut q| q.drain(..).collect::<Vec<_>>()).unwrap_or_default() {
        let marks = run.sim.as_ref().map(|s| s.stop_times()).unwrap_or_default();
        // a transport button takes the walker back from click-to-walk: while a walk target is set the walker heads for it
        // whatever the clock does (and the clock re-pauses every frame), so play / pause / seek clear it first
        let had_target = walk_target.as_ref().is_some_and(|t| t.0.is_some());
        if !matches!(e, SimEvent::Speed(_) | SimEvent::Csv) && had_target {
            if let Some(t) = walk_target.as_mut() {
                t.0 = None;
            }
        }
        match e {
            // pause while walking to a clicked spot: just stop there (the clock stays paused)
            SimEvent::Toggle if had_target => run.playing = false,
            SimEvent::Toggle => {
                if run.t >= total {
                    run.t = 0.0;
                }
                run.playing = !run.playing;
                run.manual = false;
            }
            SimEvent::Start => { run.t = 0.0; run.manual = false; }
            SimEvent::Prev => { run.t = marks.iter().rev().copied().find(|&m| m < run.t - 0.5).unwrap_or(0.0); run.manual = false; }
            SimEvent::Next => { run.t = marks.iter().copied().find(|&m| m > run.t + 0.5).unwrap_or(total); run.manual = false; }
            SimEvent::Speed(x) => run.speed = x,
            SimEvent::SeekDist(d) => { run.t = run.sim.as_ref().map(|s| s.time_at_dist(d)).unwrap_or(0.0); run.manual = false; }
            SimEvent::SeekTime(x) => { run.t = x.clamp(0.0, total); run.manual = false; }
            SimEvent::Csv => export_csv(&run, &slope.0, &pace, sunlight.as_deref(), inputs.as_deref()),
        }
    }
    let walk = *mode == Mode::Walk;
    // manual control: any movement key pauses the clock and leaves the walker to walk.rs
    if walk && (acts.single().is_ok_and(|a| a.pressed(&Act::Forward) || a.pressed(&Act::Back) || a.pressed(&Act::Left) || a.pressed(&Act::Right))
        || walk_target.as_ref().is_some_and(|t| t.0.is_some())) {
        run.manual = true;
        run.playing = false;
    }
    if walk && run.playing {
        run.t = (run.t + time.delta_secs() * run.speed).min(total);
        if run.t >= total {
            run.playing = false;
        }
    }
    let start_h = sunlight.as_ref().and_then(|s| s.hour).unwrap_or(8.0);
    if let (true, false, Some(s), Some(mut w)) = (walk, run.manual, run.sim.as_ref(), walker) {
        if (run.t - run.last_applied).abs() > 1e-4 {
            let st = s.state(run.t);
            w.e = st.e;
            w.n = st.n;
            w.heading = st.heading;
            w.next = st.next % stops.0.len().max(1);
            run.last_applied = run.t;
        }
    }
    // the sun follows the clock; the elevation series (cached) gives twilight and night
    let sol = sunlight.as_ref().map(|s| s.sol);
    let hour_s = route_core::light::MARS_HOUR_S;
    let end_s = (eva.eva_s + eva.reserve_s).max(total as f64);
    if let (Some(sol), Some(i)) = (sol, inputs.as_ref()) {
        let dkey = (sol.to_bits(), start_h.to_bits(), end_s as u32);
        if run.day.as_ref().map(|d| d.0) != Some(dkey) {
            let pad = crate::plan::daylight::TWILIGHT_H + 0.5;
            let site = i.site;
            let series = crate::plan::daylight::Series::new(start_h - pad, start_h + end_s / hour_s + pad,
                                                            |h| route_core::sun::sun_at_sol(sol, h, site.0, site.1).elevation);
            let full = crate::plan::daylight::Series::new(0.0, 24.0, |h| route_core::sun::sun_at_sol(sol, h, site.0, site.1).elevation);
            let (rise, set) = full.rise_set();
            let eot = route_core::sun::sun_at_sol(sol, 12.0, site.0, site.1).eot;
            run.day = Some((dkey, series, (rise, set, eot)));
        }
    }
    let now = match (walk, sol, inputs.as_ref()) {
        (true, Some(sol), Some(i)) => {
            let h = start_h + run.t as f64 / hour_s;
            let s = route_core::sun::sun_at_sol(sol, h, i.site.0, i.site.1);
            let d = run.day.as_ref().map(|d| d.1.at(h, s.elevation));
            Some(((s.elevation, s.azimuth), d))
        }
        _ => None,
    };
    let now = SunNow(now.map(|x| x.0), now.and_then(|x| x.1));
    if *sun_now != now {
        *sun_now = now;
    }
    // UI
    let Some(s) = run.sim.as_ref() else { return };
    // the log (events + route alerts, a scan of every route sample) changes only with the plan, not with the clock: cached
    let log_key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        run.key.hash(&mut h);
        start_h.to_bits().hash(&mut h);
        s.total.to_bits().hash(&mut h);
        if let Some(pm) = bridge.panel.as_ref() {
            pm.stops.hash(&mut h);
            pm.samples.len().hash(&mut h);
            pm.samples.last().map(|q| q.d.to_bits()).hash(&mut h);
            pm.caution.to_bits().hash(&mut h);
            pm.limit.to_bits().hash(&mut h);
        }
        h.finish()
    };
    let names: Vec<String> = bridge.panel.as_ref().map(|p| p.stops.clone()).unwrap_or_default();
    if log_cache.0 != log_key || log_cache.1.is_empty() {
        let mut rows = log_rows(s, &names, start_h);
        if let Some(pm) = bridge.panel.as_ref() {
            rows.extend(alerts(s, &pm.samples, pm.caution, pm.limit, start_h));
            rows.sort_by(|a, b| a.t.total_cmp(&b.t));
        }
        *log_cache = (log_key, rows);
    }
    let log = &log_cache.1;
    let current = log.iter().rposition(|r| r.t <= run.t + 1e-3);
    let alert = log.iter().rev().find(|r| r.alert && r.t <= run.t + 1e-3 && run.t - r.t <= ALERT_SHOW_S).map(|r| r.text.clone());
    let ops = docs.as_ref().and_then(|d| d.eva_ops.as_ref());
    let _ = ops;
    // EVA time left (the EVA length minus the clock; egress and ingress are inside the EVA now)
    let task_left = if eva.eva_s > 0.0 { fill(t::TASK_LEFT, &[("x", hmm(eva.eva_s as f32 - run.t))]) } else { String::new() };
    let st_now = s.state(run.t);
    let station = st_now.at_stop.map(|k| {
        let seg_end = s.segs.iter().find_map(|g| match *g {
            sim::Seg::Dwell { stop, t0, t1 } if stop == k && run.t >= t0 - 1e-3 && run.t <= t1 + 1e-3 => Some(t1),
            _ => None,
        }).unwrap_or(run.t);
        let egress = k == 0;
        let id = stops.0.get(k).map(|x| x.id.clone()).unwrap_or_default();
        StationNow {
            id: id.clone(),
            yours: edits.0.contains_key(&id),
            name: names.get(k).cloned().unwrap_or_default(),
            left: fill(t::STATION_LEFT, &[("x", mmss(seg_end - run.t))]),
            tasks: if egress { Vec::new() } else { docs.as_ref().map(|d| tasks_of(&crate::plan::edits::tasks_value(&id, &d.stops_doc, &edits))).unwrap_or_default() },
            egress,
            pano: if egress { None } else { docs.as_ref().and_then(|d| d.panos.as_ref()).and_then(|p| crate::ui::model::Pano::from_doc(p, &id)) },
            navcam3d: docs.as_ref().is_some_and(|d| d.manifest["files"][crate::scene::navcam::FILE]["stop"] == id.as_str()),
            photo: if egress { None } else {
                docs.as_ref().and_then(|d| d.stops_doc["stops"].as_array()?.iter().find(|x| x["id"] == id.as_str())
                    .and_then(|x| crate::ui::card::photo_of(&x["photo"], &names.get(k).cloned().unwrap_or_default(), false)))
            },
        }
    });
    let sun: Vec<(f32, f32)> = match (sol, inputs.as_ref()) {
        (Some(sol), Some(i)) => {
            let end = (eva.eva_s + eva.reserve_s).max(s.total as f64);
            (0..=((end / 300.0).ceil() as i64)).map(|k| {
                let tt = k as f64 * 300.0;
                let x = route_core::sun::sun_at_sol(sol, start_h + tt / route_core::light::MARS_HOUR_S, i.site.0, i.site.1);
                (tt as f32, x.elevation as f32)
            }).collect()
        }
        _ => Vec::new(),
    };
    let (dark, dark_note, daylight) = daylight_model(run.day.as_ref().map(|d| (&d.1, d.2)), start_h, end_s, sun_now.0.map(|s| s.0),
                                                     sun_now.1, run.t);
    let m = SimModel {
        active: walk,
        playing: run.playing,
        manual: run.manual,
        speed: run.speed,
        speeds: SPEEDS.to_vec(),
        t: run.t,
        total: s.total,
        dist: s.state(run.t).dist,
        clock: clock(start_h, run.t),
        elapsed: fill(t::ELAPSED, &[("x", mmss(run.t)), ("total", mmss(s.total))]),
        task_left,
        dwell_note: if s.dwell_s > 0.0 {
            fill(if eva.even_split { t::DWELL_SPLIT } else { t::DWELL }, &[("x", format!("{}", (s.dwell_s / 60.0).round() as i64))])
        } else { t::NO_DWELL.to_string() },
        log: log.clone(),
        current,
        alert,
        blocks: blocks(s, &names),
        eva_s: eva.eva_s as f32,
        reserve_s: eva.reserve_s as f32,
        sun,
        start_h,
        station,
        dark,
        dark_note,
        daylight,
    };
    bridge.set_sim(m);
}

/// Twilight / night runs on the EVA clock, the first one as a note, and the light label for the Walk HUD.
#[allow(clippy::type_complexity)]
fn daylight_model(series: Option<(&crate::plan::daylight::Series, (Option<f64>, Option<f64>, f64))>, start_h: f64, end_s: f64, el_now: Option<f64>,
                  now: Option<crate::plan::daylight::Daylight>, t_now: f32)
                  -> (Vec<(f32, f32, u8)>, Option<String>, (String, &'static str, String)) {
    use crate::plan::daylight::Phase;
    let hour_s = route_core::light::MARS_HOUR_S;
    let Some((s, (rise, set, eot))) = series else { return (Vec::new(), None, (String::new(), "day", String::new())) };
    let dark: Vec<(f32, f32, u8)> = s.dark_runs(start_h, start_h + end_s / hour_s).into_iter().map(|(h0, h1, p)| {
        (((h0 - start_h) * hour_s) as f32, ((h1 - start_h) * hour_s) as f32, if p == Phase::Night { 2 } else { 1 })
    }).collect();
    let hm = |h: f64| { let m = (h.rem_euclid(24.0) * 60.0).round() as i64; format!("{:02}:{:02}", (m / 60) % 24, m % 60) };
    // the first dark stretch starts on the 5-minute grid; name the exact sunset / end of twilight when one is that close
    let note = dark.first().map(|r| {
        let h = start_h + r.0 as f64 / hour_s;
        let exact = s.crossings.iter().copied().chain(s.crossings.iter().map(|c| c + crate::plan::daylight::TWILIGHT_H))
            .find(|c| (c - h).abs() <= crate::plan::daylight::STEP_H).unwrap_or(h);
        fill(if r.2 == 2 { t::DARK_NIGHT } else { t::DARK_TWILIGHT }, &[("time", hm(exact))])
    });
    let none = || "\u{2013}".to_string();
    let (rise, set) = (rise.map(hm).unwrap_or_else(none), set.map(hm).unwrap_or_else(none));
    let el = el_now.unwrap_or_else(|| s.el_at(start_h + t_now as f64 / hour_s));
    let phase = now.map(|d| d.phase).unwrap_or_else(|| s.at(start_h + t_now as f64 / hour_s, el).phase);
    let (label, class) = match phase { Phase::Day => (t::DAY, "day"), Phase::Twilight => (t::TWILIGHT, "twilight"), Phase::Night => (t::NIGHT, "night") };
    // Curiosity's mission clock is local MEAN solar time: LMST = LTST - EOT (eot in degrees, 4 min per degree)
    let off = (eot * 4.0).round() as i64;
    let y = fill(if off >= 0 { t::LMST_EARLIER } else { t::LMST_LATER }, &[("x", off.abs().to_string())]);
    let tip = fill(t::DAYLIGHT_TIP, &[("el", format!("{:.0}", el)), ("rise", rise), ("set", set), ("y", y)]);
    (dark, note, (label.to_string(), class, tip))
}

/// The simulated EVA, one row per second: time, LTST, east, north, map and corrected slope, sun elevation and azimuth,
/// state. Downloaded as CSV (web).
fn export_csv(run: &SimRun, slope: &crate::scene::terrain::Grid, pace: &Pace, sunlight: Option<&crate::plan::sunlight::SunLight>,
              inputs: Option<&crate::plan::planner::PlanInputs>) {
    let Some(s) = run.sim.as_ref() else { return };
    let start_h = sunlight.and_then(|x| x.hour).unwrap_or(8.0);
    let sol = sunlight.map(|x| x.sol).unwrap_or(f64::NAN);
    let mut out = String::from("t_s,ltst,east_m,north_m,slope_map_deg,slope_corrected_deg,sun_elevation_deg,sun_azimuth_deg,state\n");
    let mut tt = 0.0f32;
    while tt <= s.total + 1e-3 {
        let st = s.state(tt);
        let sm = slope.height_at(st.e, st.n);
        let (el, az) = inputs.map(|i| {
            let x = route_core::sun::sun_at_sol(sol, start_h + tt as f64 / route_core::light::MARS_HOUR_S, i.site.0, i.site.1);
            (x.elevation, x.azimuth)
        }).unwrap_or((f64::NAN, f64::NAN));
        let state = match st.at_stop { Some(k) => format!("at stop {}", k + 1), None if st.done => "done".into(), None => format!("to stop {}", st.next + 1) };
        out += &format!("{:.0},{},{:.2},{:.2},{:.1},{:.1},{:.2},{:.2},{}\n", tt, clock(start_h, tt), st.e, st.n, sm,
                        (sm - pace.bias_deg).max(0.0), el, az, state);
        tt += 1.0;
    }
    #[cfg(target_arch = "wasm32")]
    crate::plan::share::web::download("visor_eva_simulation.csv", "text/csv", &out);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = out;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_adds_mars_time_to_the_start_hour() {
        assert_eq!(clock(8.0, 0.0), "08:00:00");
        let one_mars_hour = route_core::light::MARS_HOUR_S as f32;
        assert_eq!(clock(8.0, one_mars_hour), "09:00:00");
        assert_eq!(mmss(125.4), "2:05");
    }

    #[test]
    fn log_names_stops_and_steepest_legs() {
        let poly = vec![(0.0, 0.0), (10.0, 0.0), (0.0, 0.0)];
        let s = sim::build(&poly, &[(0, 1, 10.0), (1, 2, 10.0)], |_, _| 1.0, |_, _| 4.0, 0.0, 0.0, 0.0);
        let rows = log_rows(&s, &["1 Exit".into(), "2 Bococo".into()], 8.0);
        assert!(rows.iter().any(|r| r.text.contains("2 Bococo")) && rows.iter().any(|r| r.text.contains("1 Exit")));
        assert!(rows.windows(2).all(|w| w[0].t <= w[1].t));
        assert!(rows.iter().all(|r| !r.text.contains('{')));
    }
}
