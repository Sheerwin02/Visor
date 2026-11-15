//! Live loop planner: the planner changes the stops (order, add by clicking the map, remove) and the assumptions (slope
//! limit, flat walking speed, rover correction on/off) in the panel; the loop is re-planned with route_core (the same rule
//! as tools/route.py, checked against it on the real bundle) and everything that depends on it is rebuilt: route line,
//! flags and labels, top-bar data, walk-back field, slope hatching, panel. Default settings reproduce the published plan.
use std::sync::Mutex;

use bevy::prelude::*;
use route_core::{Plan, Rules, Tours, MAX_TOUR_STOPS};
use bevy::platform::time::Instant;
use serde_json::{json, Value};

use crate::ui::hud::{RouteData, WalkBack};
use crate::scene::layers::{RouteLayer, StopsLayer};
use crate::scene::mapview::{MapCam, PlanView};
use crate::ui::model::{Controls, UiBridge};
use crate::ui::chrome::fill;
use crate::scene::terrain::Grid;
use crate::scene::walk::{Ground, Mode, Pace, Stop, Stops, Walker};

/// What the panel's controls send. Queued by the DOM handlers (panel.rs), applied by `apply_events`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))] // some are only sent by the web panel's buttons
pub enum PanelEvent {
    Limit(f64),
    Speed(f64),
    Correct(bool),
    MoveUp(usize),
    MoveDown(usize),
    Remove(usize),
    StartPick,
    CancelPick,
    Reset,
    BestOrder,
    FitBudget(f64),
    ToggleMust(usize),
    /// keep only the exit and add stations by clicking the map (until Esc / Done)
    NewPlan,
    /// apply a saved plan's share-link query (no page reload)
    Apply(String),
    /// a station dragged on the map to (east, north): a Curiosity stop becomes a custom stop "<name> (moved)" (its NASA
    /// facts are about the original spot); the exit stays where it is
    Move(usize, f32, f32),
}

pub static EVENTS: Mutex<Vec<PanelEvent>> = Mutex::new(Vec::new());

/// Move station i to (e, n). A stop from stops.json becomes a new custom stop (named "<name> (moved)"), so nothing NASA
/// reported there is shown for the new spot; a custom stop just moves. Must-visit marks follow the stop.
pub fn move_stop(s: &mut PlanSettings, i: usize, e: f32, n: f32) {
    let Some(old) = s.stops.get(i).cloned() else { return };
    if old.id.starts_with("custom-") {
        s.stops[i].e = e;
        s.stops[i].n = n;
        if let Some(p) = s.pool.iter_mut().find(|p| p.id == old.id) {
            p.e = e;
            p.n = n;
        }
        return;
    }
    s.custom_count += 1;
    let k = s.custom_count;
    let name = fill(crate::text::plan_panel::MOVED_STOP, &[("name", crate::scene::labels::short_name(&old.name).to_string())]);
    let stop = Stop { id: format!("custom-{k}"), name, e, n };
    if let Some(m) = s.must.iter_mut().find(|m| **m == old.id) {
        *m = stop.id.clone();
    }
    s.pool.push(stop.clone());
    s.stops[i] = stop;
}

pub fn send(e: PanelEvent) {
    if let Ok(mut q) = EVENTS.lock() {
        q.push(e);
    }
}

/// Everything the planner needs that does not change: the slope grid, rover positions and the published assumptions.
#[derive(Resource)]
pub struct PlanInputs {
    pub slope: Vec<f32>,
    pub w: usize,
    pub h: usize,
    pub cell: f64,
    pub rover: Vec<(f64, f64)>,
    /// route.json "assumptions" (the published rules; bias = the rover check's signed median)
    pub assumptions: Value,
    pub published_stops: Vec<Stop>,
    /// loop length (m) and walking time (s) of the published plan, for comparison
    pub published: (f64, f64),
    /// the 1 m DEM heights (shadows, W3) and the site for the sun: west longitude, planetographic latitude
    pub heights: Vec<f32>,
    pub site: (f64, f64),
}

impl PlanInputs {
    fn a(&self, k: &str) -> f64 {
        self.assumptions[k].as_f64().unwrap_or(f64::NAN)
    }
    pub fn defaults(&self) -> PlanSettings {
        PlanSettings {
            limit_deg: self.a("slope_limit_deg"),
            flat_mps: self.a("speed_flat_mps"),
            correct: true,
            stops: self.published_stops.clone(),
            picking: false,
            custom_count: 0,
            pool: self.published_stops.clone(),
            must: Vec::new(),
            // the budget box starts at the published plan's walking time, rounded up to a whole minute
            budget_min: (self.published.1 / 60.0).ceil(),
            note: String::new(),
            multi_pick: false,
        }
    }
    pub fn rules(&self, s: &PlanSettings) -> Rules {
        Rules { flat_mps: s.flat_mps, k: self.a("tobler_k"), bias_deg: if s.correct { self.a("bias_deg") } else { 0.0 },
                limit_deg: s.limit_deg, verified_r_m: self.a("verified_radius_m") }
    }
    pub fn slope_grid(&self) -> Grid {
        let top = self.slope.iter().cloned().fold(f32::MIN, f32::max);
        Grid { w: self.w, h: self.h, cell: self.cell as f32, z: self.slope.clone(), top }
    }
}

#[derive(Resource, Debug, Clone, PartialEq)]
pub struct PlanSettings {
    pub limit_deg: f64,
    pub flat_mps: f64,
    pub correct: bool,
    /// stops in walking order; the first is the exit and stays first
    pub stops: Vec<Stop>,
    /// waiting for a click on the map to add a stop
    pub picking: bool,
    pub custom_count: usize,
    /// every stop known so far (published + added on the map); the budget fit chooses from these
    pub pool: Vec<Stop>,
    /// ids of must-visit stops
    pub must: Vec<String>,
    pub budget_min: f64,
    /// result of the last automatic action
    pub note: String,
    /// building a new plan: every map click adds a station (picking stays on)
    pub multi_pick: bool,
}

impl PlanSettings {
    /// The same plan (rules, stops in order, must-visit, budget), ignoring UI state: picking a stop, the note, the pool.
    /// Used to decide whether the published plan's uncertainty results apply.
    pub fn same_plan(&self, o: &PlanSettings) -> bool {
        self.limit_deg == o.limit_deg && self.flat_mps == o.flat_mps && self.correct == o.correct && self.budget_min == o.budget_min
            && self.must == o.must && self.stops.len() == o.stops.len()
            && self.stops.iter().zip(&o.stops).all(|(a, b)| a.id == b.id && a.e == b.e && a.n == b.n)
    }
}

/// A "best order" or "fit the budget" request waiting for its table of walking times between stops (computed by the
/// planner worker, so the page does not freeze; the exact tour search on the table is fast and runs here).
#[derive(Resource, Default)]
pub struct TourAsk {
    pub id: u64,
    pub action: Option<TourAction>,
    /// the answer, once it arrives (stored by `replan`, used by `apply_events`)
    pub times: Option<(u64, Vec<Vec<f64>>)>,
}

#[derive(Debug, Clone)]
pub enum TourAction {
    Best { stops: Vec<Stop> },
    Fit { min: f64, pool: Vec<Stop>, must_mask: usize },
}

fn en(stops: &[Stop]) -> Vec<(f64, f64)> {
    stops.iter().map(|x| (x.e as f64, x.n as f64)).collect()
}

/// Apply a tour answer to the settings: the new stop order, and a note saying what changed.
fn apply_tour(s: &mut PlanSettings, action: TourAction, t: &[Vec<f64>]) {
    let Some(tours) = Tours::new(t) else {
        s.note = fill(crate::text::plan_panel::TOO_MANY, &[("total", MAX_TOUR_STOPS.to_string())]);
        return;
    };
    match action {
        TourAction::Best { stops } => {
            let n = stops.len();
            s.note = match tours.best_all() {
                None => crate::text::plan_panel::FEASIBLE_NO.to_string(),
                Some((best, order)) => {
                    let before: f64 = (0..n).map(|i| t[i][(i + 1) % n]).sum();
                    if before - best < 0.5 {
                        crate::text::plan_panel::BEST_ORDER_SAME.to_string()
                    } else {
                        s.stops = order.iter().map(|&i| stops[i].clone()).collect();
                        fill(crate::text::plan_panel::BEST_ORDER_NOTE,
                             &[("minutes", crate::ui::hud::minutes(best as f32)), ("saved", crate::ui::hud::minutes((before - best) as f32))])
                    }
                }
            };
        }
        TourAction::Fit { min, pool, must_mask } => {
            let budget = format!("{min}");
            s.note = match tours.best_within(min * 60.0, must_mask) {
                None => fill(crate::text::plan_panel::FIT_NONE, &[("budget", budget)]),
                Some((time, order)) => {
                    s.stops = order.iter().map(|&i| pool[i].clone()).collect();
                    fill(crate::text::plan_panel::FIT_NOTE, &[("k", order.len().to_string()), ("total", pool.len().to_string()),
                                                             ("budget", budget), ("minutes", crate::ui::hud::minutes(time as f32))])
                }
            };
        }
    }
}

/// The plan in route.json's schema (legs, totals, assumptions), and stops with their walk-back times, for the panel.
pub fn plan_json(plan: &Plan, rules: &Rules, inputs: &PlanInputs, stops: &[Stop]) -> (Value, Vec<Value>) {
    let mut a = inputs.assumptions.clone();
    a["speed_flat_mps"] = json!(rules.flat_mps);
    a["slope_limit_deg"] = json!(rules.limit_deg);
    a["bias_deg"] = json!(rules.bias_deg);
    let r1 = |x: f64| (x * 10.0).round() / 10.0;
    let legs: Vec<Value> = plan.legs.iter().map(|l| json!({
        "possible": l.possible, "length_m": r1(l.length_m), "time_s": l.time_s.round(),
        "max_corrected_slope_deg": r1(l.max_corrected_slope_deg), "share_in_rover_verified": (l.share_in_rover_verified * 1000.0).round() / 1000.0,
        "cells_over_limit_inside_verified": l.cells_over_limit_inside_verified, "poly_start": l.poly_start, "poly_end": l.poly_end })).collect();
    let route = json!({ "assumptions": a, "legs": legs, "total_length_m": r1(plan.total_length_m),
                        "total_time_s": plan.total_time_s.round(), "all_legs_possible": plan.all_legs_possible });
    let stops_v = stops.iter().zip(&plan.walk_back_stops)
        .map(|(s, wb)| json!({ "id": s.id, "name": s.name, "e_m": s.e, "n_m": s.n, "walk_back_s": wb.map(|t| t.round()) }))
        .collect();
    (route, stops_v)
}

pub fn apply_events(keys: Res<ButtonInput<KeyCode>>, inputs: Option<Res<PlanInputs>>, settings: Option<ResMut<PlanSettings>>,
                    mut ask: ResMut<TourAsk>, mut compute: ResMut<crate::plan::compute::Compute>, docs: Option<Res<crate::BundleDocs>>) {
    let (Some(inputs), Some(mut s)) = (inputs, settings) else { return };
    // a tour answer for the newest request: apply it (changing the settings re-plans)
    if let Some((id, t)) = ask.times.take() {
        if id == ask.id {
            if let Some(action) = ask.action.take() {
                apply_tour(&mut s, action, &t);
            }
        }
    }
    let mut events: Vec<PanelEvent> = EVENTS.lock().map(|mut q| q.drain(..).collect()).unwrap_or_default();
    if keys.just_pressed(KeyCode::Escape) && s.picking {
        events.push(PanelEvent::CancelPick);
    }
    for e in events {
        let n = s.stops.len();
        match e {
            PanelEvent::Limit(v) => s.limit_deg = v,
            PanelEvent::Speed(v) => s.flat_mps = v,
            PanelEvent::Correct(v) => s.correct = v,
            // the exit (index 0) stays first
            PanelEvent::MoveUp(i) if i >= 2 && i < n => s.stops.swap(i, i - 1),
            PanelEvent::MoveDown(i) if i >= 1 && i + 1 < n => s.stops.swap(i, i + 1),
            PanelEvent::Remove(i) if i >= 1 && i < n && n > 2 => {
                s.stops.remove(i);
            }
            PanelEvent::Move(i, e, nn) if i >= 1 && i < n => move_stop(&mut s, i, e, nn),
            PanelEvent::StartPick => { s.picking = true; s.note = crate::text::plan_panel::ADD_STOP_NOTE.to_string(); }
            PanelEvent::CancelPick => { s.picking = false; s.multi_pick = false; }
            PanelEvent::NewPlan => {
                s.stops.truncate(1);
                s.must.clear();
                s.picking = true;
                s.multi_pick = true;
                s.note = crate::text::plan_panel::NEW_PLAN_NOTE.to_string();
            }
            PanelEvent::Apply(q) => {
                if let Some(d) = docs.as_ref() {
                    let (mut ns, warn) = crate::plan::share::decode(&q, &inputs, &crate::plan::share::bundle_id(&d.manifest));
                    ns.note = warn.join(" ");
                    *s = ns;
                }
            }
            PanelEvent::Reset => *s = inputs.defaults(),
            PanelEvent::ToggleMust(i) if i >= 1 && i < n => {
                let id = s.stops[i].id.clone();
                if let Some(k) = s.must.iter().position(|m| *m == id) {
                    s.must.remove(k);
                } else {
                    s.must.push(id);
                }
            }
            PanelEvent::BestOrder => {
                let stops = s.stops.clone();
                if stops.len() > MAX_TOUR_STOPS {
                    s.note = fill(crate::text::plan_panel::TOO_MANY, &[("total", MAX_TOUR_STOPS.to_string())]);
                } else {
                    ask.id += 1;
                    ask.action = Some(TourAction::Best { stops: stops.clone() });
                    compute.submit(crate::plan::compute::Job { id: ask.id, mode: crate::plan::compute::Mode::Times, rules: inputs.rules(&s),
                                                         stops: en(&stops), limit: s.limit_deg, light: None }, &inputs);
                }
            }
            PanelEvent::FitBudget(min) if min > 0.0 => {
                s.budget_min = min;
                let pool = s.pool.clone();
                if pool.len() > MAX_TOUR_STOPS {
                    s.note = fill(crate::text::plan_panel::TOO_MANY, &[("total", MAX_TOUR_STOPS.to_string())]);
                } else {
                    let must_mask = pool.iter().enumerate().skip(1)
                        .filter(|(_, p)| s.must.contains(&p.id)).fold(0usize, |m, (i, _)| m | 1 << (i - 1));
                    ask.id += 1;
                    ask.action = Some(TourAction::Fit { min, pool: pool.clone(), must_mask });
                    compute.submit(crate::plan::compute::Job { id: ask.id, mode: crate::plan::compute::Mode::Times, rules: inputs.rules(&s),
                                                         stops: en(&pool), limit: s.limit_deg, light: None }, &inputs);
                }
            }
            _ => {}
        }
    }
}

/// While picking, a left click on the map (press and release without dragging) adds a stop there, before the return leg.
pub fn pick_stop(
    windows: Query<&Window>,
    buttons: Res<ButtonInput<MouseButton>>,
    mode: Res<Mode>,
    view: Res<PlanView>,
    map: Option<Res<MapCam>>,
    settings: Option<ResMut<PlanSettings>>,
    mut press_at: Local<Option<Vec2>>,
) {
    let (Ok(win), Some(map), Some(mut s)) = (windows.single(), map, settings) else { return };
    if !s.picking || *mode != Mode::Plan || *view != PlanView::Map {
        *press_at = None;
        return;
    }
    let cursor = win.cursor_position();
    if buttons.just_pressed(MouseButton::Left) {
        *press_at = cursor;
    }
    if buttons.just_released(MouseButton::Left) {
        if let (Some(p0), Some(p1)) = (*press_at, cursor) {
            if p0.distance(p1) < 4.0 && crate::scene::mapview::in_map_area(p1, Vec2::new(win.width(), win.height())) {
                let (e, n) = map.to_map(p1, Vec2::new(win.width(), win.height()));
                s.custom_count += 1;
                let k = s.custom_count;
                let name = fill(crate::text::plan_panel::CUSTOM_STOP, &[("k", k.to_string())]);
                let stop = Stop { id: format!("custom-{k}"), name, e, n };
                s.pool.push(stop.clone());
                s.stops.push(stop);
                s.picking = s.multi_pick;
            }
        }
        *press_at = None;
    }
}

/// Ask for a new plan when the settings change (and once at start); when the newest answer arrives (from the planner
/// worker on the web, at once natively), rebuild everything that depends on the plan and start the robustness check.
#[allow(clippy::too_many_arguments)]
pub fn replan(
    mut commands: Commands,
    inputs: Option<Res<PlanInputs>>,
    settings: Option<Res<PlanSettings>>,
    ground: Option<Res<Ground>>,
    docs: Option<Res<crate::BundleDocs>>,
    mut bridge: ResMut<UiBridge>,
    mut pace: Option<ResMut<Pace>>,
    mut walker: Option<ResMut<Walker>>,
    mut overlays: Option<ResMut<crate::scene::trust::Overlays>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut terrain_mats: ResMut<Assets<crate::scene::trust::TerrainMaterial>>,
    route_layers: Query<(Entity, &Visibility), With<RouteLayer>>,
    stop_layers: Query<(Entity, &Visibility), With<StopsLayer>>,
    (mut compute, mut job, mut robust, mut asked_at, mut ask, edits): (ResMut<crate::plan::compute::Compute>, ResMut<PlanJob>, ResMut<Robust>,
                                                                       Local<Option<Instant>>, ResMut<TourAsk>, Res<crate::plan::edits::StationEdits>),
) {
    let (Some(inputs), Some(s), Some(ground), Some(docs)) = (inputs, settings, ground, docs) else { return };
    let rules = inputs.rules(&s);
    let stops_en: Vec<(f64, f64)> = s.stops.iter().map(|x| (x.e as f64, x.n as f64)).collect();
    // a new plan waits for its first station (the planner needs the exit and at least one more stop): show it as it is
    if s.is_changed() && s.stops.len() < 2 {
        if let Some(mut pm) = bridge.panel.clone() {
            pm.controls.picking = s.picking;
            pm.controls.note = s.note.clone();
            pm.stops = s.stops.iter().map(|x| x.name.clone()).collect();
            pm.stop_cards = crate::ui::card::summaries(&s.stops, &docs.stops_doc, &edits);
            bridge.set_panel(pm);
        }
    }
    if s.is_changed() && s.stops.len() >= 2 {
        job.id += 1;
        *asked_at = Some(Instant::now());
        crate::ui::chrome::attr("mission-bar", "aria-busy", "true");
        compute.submit(crate::plan::compute::Job { id: job.id, mode: crate::plan::compute::Mode::Full, rules, stops: stops_en.clone(), limit: rules.limit_deg, light: None }, &inputs);
    }
    compute.pump();
    let mut newest = None;
    let mut robust_done = false;
    for d in crate::plan::compute::take() {
        match d.mode {
            crate::plan::compute::Mode::Full if d.id == job.id => newest = d.plan,
            crate::plan::compute::Mode::Times if d.id == ask.id => ask.times = d.times.map(|t| (d.id, t)),
            crate::plan::compute::Mode::Shadow | crate::plan::compute::Mode::Hours => crate::plan::sunlight::stash(d),
            crate::plan::compute::Mode::Route if d.id == robust.id => {
                let Some(p) = d.plan.as_ref() else { continue };
                let same = p.all_legs_possible && p.polyline_en == robust.current;
                robust.results.push((d.limit, same, p.all_legs_possible, p.total_time_s));
                if robust.pending.is_empty() {
                    robust_done = true;
                } else {
                    robust.next(&mut compute, &inputs);
                }
            }
            _ => {}
        }
    }
    if robust_done {
        let range = (inputs.assumptions["limit_range_deg"][0].as_f64().unwrap_or(s.limit_deg),
                     inputs.assumptions["limit_range_deg"][1].as_f64().unwrap_or(s.limit_deg));
        if let Some(mut m) = bridge.panel.clone() {
            m.controls.robust = robust_summary(s.limit_deg, range, &robust.results);
            bridge.set_panel(m);
        }
    }
    let Some(plan) = newest else { return };
    let ms = asked_at.map(|t| t.elapsed().as_secs_f64() * 1e3).unwrap_or(0.0);
    info!("plan {} back after {:.0} ms: {:.0} m, {:.0} s, all legs possible: {}", job.id, ms, plan.total_length_m, plan.total_time_s,
          plan.all_legs_possible);
    crate::ui::chrome::attr("mission-bar", "data-replan-ms", &format!("{ms:.0}"));
    crate::ui::chrome::attr("mission-bar", "aria-busy", "false");

    // top-bar data, walk-back field, stops, walking pace
    let poly: Vec<(f32, f32)> = plan.polyline_en.iter().map(|&(e, n)| (e as f32, n as f32)).collect();
    let legs = plan.legs.iter().map(|l| (l.poly_start, l.poly_end)).collect();
    commands.insert_resource(RouteData { poly: poly.clone(), legs });
    commands.insert_resource(WalkBack { w: inputs.w, h: inputs.h, cell: inputs.cell as f32,
                                        s: plan.walk_back.iter().map(|&t| t as f32).collect() });
    commands.insert_resource(Stops(s.stops.clone()));
    if let Some(p) = pace.as_mut() {
        p.flat_mps = rules.flat_mps as f32;
        p.bias_deg = rules.bias_deg as f32;
    }
    if let Some(w) = walker.as_mut() {
        if w.next >= s.stops.len() {
            w.next = 1 % s.stops.len().max(1);
        }
    }
    // route line and flags (keep their layer visibility)
    let route_vis = route_layers.iter().next().map(|(_, v)| *v).unwrap_or(Visibility::Visible);
    let stops_vis = stop_layers.iter().next().map(|(_, v)| *v).unwrap_or(Visibility::Visible);
    for (e, _) in route_layers.iter().chain(stop_layers.iter()) {
        commands.entity(e).despawn();
    }
    let route_e = crate::scene::layers::spawn_route(&mut commands, &mut meshes, &mut materials, &ground.0, &poly);
    commands.entity(route_e).insert(route_vis);
    let stops_e = crate::scene::layers::spawn_flags(&mut commands, &mut meshes, &mut materials, &ground.0, &s.stops);
    commands.entity(stops_e).insert(stops_vis);
    crate::scene::labels::create_labels(&Stops(s.stops.clone()));
    // slope hatching depends on the limit and the bias: two shader uniforms
    if let Some(o) = overlays.as_mut() {
        o.bias = rules.bias_deg as f32;
        o.caution = inputs.a("caution_deg") as f32;
        o.limit = rules.limit_deg as f32;
        o.push(&mut terrain_mats);
    }
    // panel
    let (route_v, stops_v) = plan_json(&plan, &rules, &inputs, &s.stops);
    let slope_g = inputs.slope_grid();
    let samples = crate::ui::panel::profile(&poly, &ground.0, &slope_g, rules.bias_deg as f32);
    {
        let controls = Controls {
            limit_deg: s.limit_deg,
            limit_range: (inputs.assumptions["limit_range_deg"][0].as_f64().unwrap_or(s.limit_deg), inputs.assumptions["limit_range_deg"][1].as_f64().unwrap_or(s.limit_deg)),
            speed: s.flat_mps,
            speeds: inputs.assumptions["speed_options_mps"].as_array().map(|a| a.iter().filter_map(|v| v.as_f64()).collect()).unwrap_or_default(),
            correct: s.correct,
            measured_bias: inputs.a("bias_deg"),
            picking: s.picking,
            is_default: s.same_plan(&inputs.defaults()),
            published: inputs.published,
            must: s.stops.iter().map(|x| s.must.contains(&x.id)).collect(),
            budget_min: s.budget_min,
            note: s.note.clone(),
            robust: vec![crate::text::plan_panel::ROBUST_CHECKING.to_string()],
        };
        let range = docs.ensemble.as_ref().map(|e| crate::ui::panel::range_note(e, controls.is_default, controls.budget_min)).unwrap_or_default();
        let is_default = controls.is_default;
        let mut pm = crate::ui::panel::model(&route_v, &stops_v, &docs.manifest, samples, controls);
        pm.range_note = range;
        // corridor results apply to the published legs only
        pm.time_range = docs.ensemble.as_ref().filter(|_| is_default).and_then(crate::ui::panel::time_range);
        pm.range_unsettled = pm.time_range.is_some() && docs.ensemble.as_ref().is_some_and(crate::ui::panel::range_unsettled);
        pm.leg_agree = docs.ensemble.as_ref().filter(|_| is_default).map(crate::ui::panel::leg_agreement).unwrap_or_default();
        pm.stop_cards = crate::ui::card::summaries(&s.stops, &docs.stops_doc, &edits);
        for c in pm.stop_cards.iter_mut() {
            c.latlon = crate::scene::basemap::latlon(&docs.manifest["grid"], c.e as f64, c.n as f64);
            c.walk_back_s = stops_v.get(c.index).and_then(|v| v["walk_back_s"].as_f64());
            if c.map_understates() {
                c.work_spot = crate::plan::relay::work_spot(&slope_g, rules.bias_deg as f32, c.e, c.n);
            }
            if let Some(ch) = docs.chem.as_ref() {
                (c.chem, c.chem_total) = crate::scene::chem::near(ch, c.e, c.n, crate::ui::views_consts::CHEM_R_M, 6);
            }
        }
        // blocked plan: for each stop the exit cannot be reached from, the nearest reachable spot (plan/relay.rs)
        if !plan.all_legs_possible {
            let wb = WalkBack { w: inputs.w, h: inputs.h, cell: inputs.cell as f32, s: plan.walk_back.iter().map(|&t| t as f32).collect() };
            pm.fixes = pm.stop_cards.iter().filter(|c| c.index > 0).filter_map(|c| {
                crate::plan::relay::nearest_reachable(&|e, n| wb.at(e, n), c.e, c.n, crate::plan::relay::FIX_MAX_R_M)
                    .map(|(e, n, d)| (c.index, c.label.clone(), e, n, d))
            }).collect();
        }
        pm.runs = docs.ensemble.as_ref().and_then(|e| {
            let shown = e["shown_correlation_length_m"].as_f64();
            e["ensembles"].as_array()?.iter().find(|x| x["correlation_length_m"].as_f64() == shown)?["runs"].as_u64()
        }).map(|n| n.to_string()).unwrap_or_default();
        if let Some((legs, note)) = docs.ensemble.as_ref().and_then(crate::ui::panel::corridor).filter(|_| is_default) {
            for (row, (v, tip)) in pm.legs.iter_mut().zip(legs) {
                row.corridor = v;
                row.corridor_tip = tip;
            }
            pm.range_note = format!("{} {note}", pm.range_note);
        }
        bridge.set_panel(pm);
    }
    // robustness check for this plan: every other whole-degree limit in the range, route only, one job at a time
    let range = (inputs.assumptions["limit_range_deg"][0].as_f64().unwrap_or(s.limit_deg),
                 inputs.assumptions["limit_range_deg"][1].as_f64().unwrap_or(s.limit_deg));
    *robust = Robust {
        id: job.id,
        pending: (range.0.round() as i64..=range.1.round() as i64).map(|x| x as f64).filter(|x| (*x - s.limit_deg).abs() > 1e-9).rev().collect(),
        results: Vec::new(),
        current: plan.polyline_en.clone(),
        rules: Some(rules),
        stops: stops_en,
    };
    robust.next(&mut compute, &inputs);
    // the address bar always holds this plan's share link; the export buttons use the same plan
    #[cfg(target_arch = "wasm32")]
    {
        use crate::plan::share as sh;
        let q = sh::encode(&s, &sh::bundle_id(&docs.manifest));
        let q = match sh::light_part(&sh::web::query()) { Some((sol, hr)) => sh::with_light(&q, sol, hr), None => q };
        // the placed relay / working spots stay in the link (scene/place.rs)
        let q = sh::with_placed(&q, &sh::decode_placed(&sh::web::query()));
        sh::web::set_query(&q);
    }
    let legs = plan.legs.iter().filter(|l| l.possible).map(|l| (l.poly_start, l.poly_end, l.time_s)).collect();
    commands.insert_resource(crate::plan::share::LastPlan { route: route_v, stops: stops_v, poly: plan.polyline_en.clone(), legs });
}


/// One result of the robustness check: slope limit, same route as now, every leg possible, walking time (s).
pub type LimitResult = (f64, bool, bool, f64);

/// The robustness check: after each new plan, the same stops are re-planned (route only, in the worker) at every
/// whole-degree slope limit in the allowed range, one job at a time; the panel shows the summary when all are back.
#[derive(Resource, Default)]
pub struct Robust {
    /// id of the plan the check belongs to (answers for older plans are ignored)
    pub id: u64,
    pending: Vec<f64>,
    results: Vec<LimitResult>,
    current: Vec<(f64, f64)>,
    rules: Option<Rules>,
    stops: Vec<(f64, f64)>,
}

impl Robust {
    fn next(&mut self, compute: &mut crate::plan::compute::Compute, inputs: &PlanInputs) {
        let (Some(limit), Some(mut rules)) = (self.pending.pop(), self.rules) else { return };
        rules.limit_deg = limit;
        compute.submit(crate::plan::compute::Job { id: self.id, mode: crate::plan::compute::Mode::Route, rules, stops: self.stops.clone(), limit, light: None }, inputs);
    }
}

/// The plan job in flight: its id (answers to older jobs are ignored).
#[derive(Resource, Default)]
pub struct PlanJob {
    pub id: u64,
}

/// Robustness summary lines (text/en.json plan_panel.robust_*), from the results at other limits.
pub fn robust_summary(current: f64, range: (f64, f64), results: &[LimitResult]) -> Vec<String> {
    use crate::text::plan_panel as p;
    let mut all: Vec<LimitResult> = results.to_vec();
    all.push((current, true, true, f64::NAN));
    all.sort_by(|a, b| a.0.total_cmp(&b.0));
    let i = all.iter().position(|r| r.0 == current).unwrap_or(0);
    let (mut lo, mut hi) = (i, i);
    while lo > 0 && all[lo - 1].1 {
        lo -= 1;
    }
    while hi + 1 < all.len() && all[hi + 1].1 {
        hi += 1;
    }
    let mut out = vec![fill(p::ROBUST_SAME, &[("lo", format!("{}", all[lo].0)), ("hi", format!("{}", all[hi].0)),
                                             ("min_deg", format!("{}", range.0)), ("max_deg", format!("{}", range.1))])];
    if lo > 0 {
        let r = all[lo - 1];
        out.push(if r.2 {
            fill(p::ROBUST_STRICTER_CHANGE, &[("x", format!("{}", r.0)), ("minutes", crate::ui::hud::minutes(r.3 as f32))])
        } else {
            fill(p::ROBUST_STRICTER_IMPOSSIBLE, &[("x", format!("{}", r.0))])
        });
    }
    if hi + 1 < all.len() {
        let r = all[hi + 1];
        out.push(fill(p::ROBUST_LOOSER_CHANGE, &[("x", format!("{}", r.0)), ("minutes", crate::ui::hud::minutes(r.3 as f32))]));
    }
    out
}


#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;

    /// A small synthetic planner input (40 x 20 cells, three stops), shared by the planner and share-link tests.
    pub(crate) fn inputs() -> PlanInputs {
        let (w, h) = (40, 20);
        PlanInputs {
            slope: vec![5.0; w * h],
            w,
            h,
            cell: 1.0,
            rover: vec![],
            assumptions: json!({ "speed_flat_mps": 0.8, "tobler_k": 3.5, "bias_deg": 3.4, "slope_limit_deg": 20.0, "verified_radius_m": 3.0,
                                 "caution_deg": 15.0, "speed_options_mps": [0.6, 0.8, 1.0], "limit_range_deg": [15.0, 25.0] }),
            published_stops: vec![
                Stop { id: "a".into(), name: "A".into(), e: 2.5, n: 10.5 },
                Stop { id: "b".into(), name: "B".into(), e: 20.5, n: 10.5 },
                Stop { id: "c".into(), name: "C".into(), e: 30.5, n: 5.5 },
            ],
            published: (0.0, 0.0),
            heights: vec![0.0; w * h],
            site: (222.62, -4.86),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::tests_support::inputs;
    use route_core::{plan_loop, CostGrid};

    fn run(events: Vec<PanelEvent>) -> PlanSettings {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::input::InputPlugin));
        let i = inputs();
        app.insert_resource(i.defaults()).insert_resource(i).init_resource::<TourAsk>().init_resource::<crate::plan::compute::Compute>()
            .add_systems(Update, (apply_events, collect_times).chain());
        for e in events {
            send(e);
        }
        app.update();
        app.world().resource::<PlanSettings>().clone()
    }

    /// What `replan` does with a Times answer, without the rest of the scene (tests only).
    fn collect_times(mut ask: ResMut<TourAsk>) {
        for d in crate::plan::compute::take() {
            if d.mode == crate::plan::compute::Mode::Times && d.id == ask.id {
                ask.times = d.times.map(|t| (d.id, t));
            }
        }
    }

    #[test]
    fn events_edit_the_plan_but_the_exit_stays_first() {
        let _guard = TEST_LOCK.lock();
        let s = run(vec![PanelEvent::MoveUp(1), PanelEvent::Remove(0)]);
        assert_eq!(s.stops[0].id, "a", "exit cannot move or be removed");
        let s = run(vec![PanelEvent::MoveUp(2)]);
        assert_eq!(s.stops.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), vec!["a", "c", "b"]);
        let s = run(vec![PanelEvent::Remove(1), PanelEvent::Remove(1)]);
        assert_eq!(s.stops.len(), 2, "never fewer than two stops");
        let s = run(vec![PanelEvent::Limit(16.0), PanelEvent::Speed(0.6), PanelEvent::Correct(false), PanelEvent::Reset]);
        assert_eq!(s, inputs().defaults(), "reset returns to the published settings");
    }

    #[test]
    fn dragging_a_station_makes_a_moved_custom_stop_and_never_moves_the_exit() {
        let _guard = TEST_LOCK.lock();
        let s = run(vec![PanelEvent::Move(2, 10.0, 20.0)]);
        assert!(s.stops[2].id.starts_with("custom-") && s.stops[2].name.contains("(moved)") && (s.stops[2].e, s.stops[2].n) == (10.0, 20.0));
        let id = s.stops[2].id.clone();
        let s2 = run(vec![PanelEvent::Move(2, 10.0, 20.0), PanelEvent::Move(2, 11.0, 21.0)]);
        assert_eq!((s2.stops[2].id.clone(), s2.stops[2].e), (id, 11.0), "a moved stop moves again in place");
        let s3 = run(vec![PanelEvent::Move(0, 1.0, 1.0)]);
        assert_ne!((s3.stops[0].e, s3.stops[0].n), (1.0, 1.0), "the exit stays");
    }

    #[test]
    fn rules_follow_the_settings_and_the_rover_switch() {
        let i = inputs();
        let mut s = i.defaults();
        assert_eq!(i.rules(&s).bias_deg, 3.4);
        s.correct = false;
        s.flat_mps = 0.6;
        let r = i.rules(&s);
        assert_eq!((r.bias_deg, r.flat_mps), (0.0, 0.6), "correction off uses the map's own slopes");
    }

    #[test]
    fn plan_json_keeps_the_route_schema() {
        let i = inputs();
        let s = i.defaults();
        let r = i.rules(&s);
        let g = CostGrid::new(&i.slope, i.w, i.h, i.cell, &r, &i.rover);
        let stops: Vec<(f64, f64)> = s.stops.iter().map(|x| (x.e as f64, x.n as f64)).collect();
        let p = plan_loop(&g, &stops, r.limit_deg);
        let (route, stops_v) = plan_json(&p, &r, &i, &s.stops);
        assert_eq!(route["legs"].as_array().unwrap().len(), 3);
        assert!(route["all_legs_possible"].as_bool().unwrap());
        assert_eq!(route["assumptions"]["bias_deg"], json!(3.4));
        assert_eq!(stops_v[0]["walk_back_s"], json!(0.0), "zero at the exit");
    }

    #[test]
    fn robustness_summary_finds_the_range_with_the_same_route() {
        // current 20; same route 18..22; at 17 a leg is impossible; at 23 the route changes
        let mut r: Vec<LimitResult> = vec![(17.0, false, false, f64::NAN), (23.0, false, true, 700.0)];
        r.extend([18.0, 19.0, 21.0, 22.0].map(|x| (x, true, true, 800.0)));
        let lines = robust_summary(20.0, (17.0, 23.0), &r);
        assert!(lines[0].contains("18") && lines[0].contains("22"), "{lines:?}");
        assert!(lines[1].contains("17") && lines[1].contains("not every leg"), "{lines:?}");
        assert!(lines[2].contains("23"), "{lines:?}");
    }

    #[test]
    fn best_order_event_reorders_and_reports() {
        let _guard = TEST_LOCK.lock();
        // stops at 0 (exit), 30, 10, 20 along a flat line: best order visits 10, 20, 30
        let mut i = inputs();
        i.published_stops = vec![
            Stop { id: "x".into(), name: "X".into(), e: 2.5, n: 10.5 },
            Stop { id: "far".into(), name: "Far".into(), e: 32.5, n: 10.5 },
            Stop { id: "near".into(), name: "Near".into(), e: 12.5, n: 10.5 },
            Stop { id: "mid".into(), name: "Mid".into(), e: 22.5, n: 10.5 },
        ];
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::input::InputPlugin));
        app.insert_resource(i.defaults()).insert_resource(i).init_resource::<TourAsk>().init_resource::<crate::plan::compute::Compute>()
            .add_systems(Update, (apply_events, collect_times).chain());
        send(PanelEvent::BestOrder);
        app.update(); // asks for the table of walking times (inline natively, the worker on the web)
        app.update(); // applies the answer
        let s = app.world().resource::<PlanSettings>();
        let ids: Vec<&str> = s.stops.iter().map(|x| x.id.as_str()).collect();
        assert!(ids == ["x", "near", "mid", "far"] || ids == ["x", "far", "mid", "near"], "{ids:?}");
        assert!(!s.note.is_empty());
    }

    /// apply_events reads a global queue: keep the event tests from interleaving.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn picking_a_stop_does_not_change_which_plan_this_is() {
        let mut a = PlanSettings { limit_deg: 20.0, flat_mps: 0.6, correct: true, stops: vec![], picking: false, custom_count: 0, pool: vec![],
                                   must: vec![], budget_min: 19.0, note: String::new(), multi_pick: false };
        let b = a.clone();
        a.picking = true;
        a.note = "x".into();
        assert!(a.same_plan(&b));
        a.limit_deg = 18.0;
        assert!(!a.same_plan(&b));
    }
}