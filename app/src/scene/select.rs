//! One selection for the Plan map: a click on the map, in the
//! north-up map or the 3D view, picks the nearest station, Curiosity rover-check stop, chemistry target or route leg within
//! PICK_PX of the pointer. The selection is shared: the station card opens in the inspector for a station, a drill-in card
//! (ui/views/drill.rs) shows the numbers measured there, the camera flies into an oblique 3D view of the spot, and a ring
//! (or the leg itself) is drawn over the map. Clicking a station label or a station in the panel selects the same way.
//! The ring and leg are an SVG overlay positioned by projecting world points (no new GPU pipeline: under ANGLE every new
//! pipeline compile froze the page, see armat.rs).
use std::sync::Mutex;

use bevy::prelude::*;

use crate::scene::camera::Orbit;
use crate::scene::mapview::{in_map_area, MapCam, PlanView};
use crate::scene::walk::{Ground, Mode, Stops};
use crate::ui::model::{DrillModel, UiBridge};

/// Pick radius on screen (CSS px), converted to metres at the clicked spot. VISUAL / usability choice.
pub const PICK_PX: f32 = 14.0;
/// Fly-in target: orbit distance (m), pitch below the horizon (deg) and duration (s). VISUAL.
pub const FLY_RADIUS_M: f32 = 45.0;
pub const FLY_PITCH_DEG: f32 = 35.0;
pub const FLY_S: f32 = 0.9;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pick {
    /// stop index in walking order
    Station(usize),
    /// index into checks::Checks.items (rover_check.json stops)
    Rover(usize),
    /// index into chemistry.json records
    Chem(usize),
    /// leg index (stop i to stop i + 1)
    Leg(usize),
    /// the placed relay / EV2 position (scene/place.rs)
    Relay,
    /// a station's placed working spot (stop index in walking order; scene/place.rs)
    Work(usize),
}

#[derive(Resource, Default, Debug)]
pub struct Selection {
    pub pick: Option<Pick>,
    /// map position of the selected object (e, n); for a leg, the clicked point on it
    pub at: (f32, f32),
    pub changed: bool,
}

impl Selection {
    pub fn set(&mut self, pick: Option<Pick>, at: (f32, f32)) {
        self.pick = pick;
        self.at = at;
        self.changed = true;
    }
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub enum SelEvent {
    /// an HTML marker over the map was clicked (rover-check marks)
    Pick(Pick, (f32, f32)),
    BackToMap,
    Close,
    WalkFrom,
}

pub static EVENTS: Mutex<Vec<SelEvent>> = Mutex::new(Vec::new());

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn send(e: SelEvent) {
    if let Ok(mut q) = EVENTS.lock() {
        q.push(e);
    }
}

/// Distance from p to segment a-b, and the nearest point on it.
pub fn seg_dist(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> (f32, (f32, f32)) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l2 = dx * dx + dy * dy;
    let t = if l2 > 0.0 { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
    let q = (a.0 + t * dx, a.1 + t * dy);
    (((p.0 - q.0).powi(2) + (p.1 - q.1).powi(2)).sqrt(), q)
}

/// Object at (e, n) within `r` metres, in tiers: a station within STATION_SHARE of `r` (stations are what people aim
/// at, and each sits on a rover stop), else the nearest Curiosity stop or chemistry target, else the nearest leg.
pub fn nearest(points: &[(Pick, (f32, f32))], legs: &[Vec<(f32, f32)>], e: f32, n: f32, r: f32) -> Option<(Pick, (f32, f32))> {
    let dist = |at: &(f32, f32)| ((at.0 - e).powi(2) + (at.1 - n).powi(2)).sqrt();
    let best = |ok: &dyn Fn(&Pick) -> bool, lim: f32| points.iter().filter(|(p, at)| ok(p) && dist(at) <= lim)
        .min_by(|a, b| dist(&a.1).total_cmp(&dist(&b.1))).copied();
    if let Some(x) = best(&|p| matches!(p, Pick::Station(_)), r * STATION_SHARE) {
        return Some(x);
    }
    if let Some(x) = best(&|p| !matches!(p, Pick::Station(_)), r) {
        return Some(x);
    }
    let mut leg: Option<(f32, Pick, (f32, f32))> = None;
    for (i, poly) in legs.iter().enumerate() {
        for s in poly.windows(2) {
            let (d, q) = seg_dist((e, n), s[0], s[1]);
            if d <= r && leg.is_none_or(|b| d < b.0) {
                leg = Some((d, Pick::Leg(i), q));
            }
        }
    }
    leg.map(|b| (b.1, b.2))
}

/// Share of the pick radius within which a station wins over anything nearer. VISUAL / usability choice.
pub const STATION_SHARE: f32 = 0.7;

/// Ground point (e, n) under a window point, through the active camera.
fn ground_at(cam: &Camera, ct: &GlobalTransform, g: &crate::scene::terrain::Grid, px: Vec2) -> Option<(f32, f32)> {
    let ray = cam.viewport_to_world(ct, px).ok()?;
    let t = g.ray_hit(ray.origin, *ray.direction, 1.0e5)?;
    let p = ray.origin + *ray.direction * t;
    Some((p.x, -p.z))
}

/// What is under the pointer on the Plan map (hover): the object, its position and a short name for the readout.
#[derive(Resource, Default, Debug)]
pub struct Hover {
    pub pick: Option<Pick>,
    pub at: (f32, f32),
    pub label: String,
}

/// Short name of a picked object for the hover readout.
pub fn hover_label(p: Pick, stops: Option<&Stops>, checks: Option<&crate::scene::checks::Checks>, chem: Option<&serde_json::Value>,
                   panel: Option<&crate::ui::model::PanelModel>) -> String {
    use crate::text::drill as t;
    match p {
        Pick::Station(i) => stops.and_then(|s| s.0.get(i)).map(|s| crate::scene::labels::label_text(i, &s.name)).unwrap_or_default(),
        Pick::Rover(i) => checks.and_then(|c| c.items.get(i)).map(|c| crate::text::fill(t::HOVER_ROVER, &[("sol", c.sol.to_string())])).unwrap_or_default(),
        Pick::Chem(j) => chem.and_then(|d| d["records"].get(j))
            .map(|r| format!("{} \u{b7} {}", r["target"].as_str().unwrap_or("").replace('_', " "), r["instrument"].as_str().unwrap_or(""))).unwrap_or_default(),
        Pick::Leg(i) => panel.and_then(|pm| pm.legs.get(i)).map(|l| format!("{} \u{b7} {} min", l.label, l.minutes)).unwrap_or_default(),
        Pick::Relay => t::KIND_RELAY.to_string(),
        Pick::Work(_) => crate::text::place::KIND_WORK.to_string(),
    }
}

fn set_canvas_cursor(pointer: bool) {
    #[cfg(target_arch = "wasm32")]
    if let Some(el) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.get_element_by_id("visor")) {
        use wasm_bindgen::JsCast;
        if let Ok(h) = el.dyn_into::<web_sys::HtmlElement>() {
            let _ = h.style().set_property("cursor", if pointer { "pointer" } else { "" });
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = pointer;
}

/// Plan map pointer: every frame, the object under the pointer (within PICK_PX) becomes the hover (pointer cursor, name in
/// the readout, preview ring); a click (press and release without dragging, inside the free map area) selects it, and a
/// click on open ground clears the selection.
#[allow(clippy::too_many_arguments)]
pub fn pick(
    windows: Query<&Window>,
    buttons: Res<ButtonInput<MouseButton>>,
    mode: Res<Mode>,
    cams: Query<(&Camera, &GlobalTransform), With<Orbit>>,
    ground: Option<Res<Ground>>,
    stops: Option<Res<Stops>>,
    checks: Option<Res<crate::scene::checks::Checks>>,
    data: Option<Res<crate::scene::inspect::InspectData>>,
    route: Option<Res<crate::ui::hud::RouteData>>,
    settings: Option<Res<crate::plan::planner::PlanSettings>>,
    bridge: Res<UiBridge>,
    mut sel: ResMut<Selection>,
    mut hover: ResMut<Hover>,
    mut press: Local<Option<Vec2>>,
) {
    let (Ok(win), Ok((cam, ct)), Some(g)) = (windows.single(), cams.single(), ground) else { return };
    let size = Vec2::new(win.width(), win.height());
    let cursor = win.cursor_position();
    let active = *mode == Mode::Plan && !settings.as_ref().is_some_and(|s| s.picking);
    // what is under the pointer
    let mut hit: Option<(Pick, (f32, f32))> = None;
    let mut ground_pt: Option<(f32, f32)> = None;
    if let (true, Some(c)) = (active, cursor) {
        if in_map_area(c, size) && !buttons.pressed(MouseButton::Right) {
            if let Some((e, n)) = ground_at(cam, ct, &g.0, c) {
                ground_pt = Some((e, n));
                let r = ground_at(cam, ct, &g.0, c + Vec2::new(PICK_PX, 0.0))
                    .map(|q| ((q.0 - e).powi(2) + (q.1 - n).powi(2)).sqrt()).unwrap_or(3.0).clamp(0.5, 60.0);
                let mut pts: Vec<(Pick, (f32, f32))> = Vec::new();
                if let Some(s) = stops.as_ref() {
                    pts.extend(s.0.iter().enumerate().map(|(i, x)| (Pick::Station(i), (x.e, x.n))));
                }
                if let Some(c) = checks.as_ref() {
                    pts.extend(c.items.iter().enumerate().map(|(i, x)| (Pick::Rover(i), (x.e, x.n))));
                }
                if let Some(doc) = data.as_ref().and_then(|d| d.chem.as_ref()) {
                    pts.extend(doc["records"].as_array().into_iter().flatten().enumerate()
                        .filter_map(|(i, x)| Some((Pick::Chem(i), (x["e_m"].as_f64()? as f32, x["n_m"].as_f64()? as f32)))));
                }
                let legs: Vec<Vec<(f32, f32)>> = route.as_ref()
                    .map(|rd| rd.legs.iter().map(|&(a, b)| rd.poly.get(a..=b.min(rd.poly.len().saturating_sub(1))).unwrap_or(&[]).to_vec()).collect())
                    .unwrap_or_default();
                hit = nearest(&pts, &legs, e, n, r);
            }
        }
    }
    let hp = hit.map(|h| h.0);
    if hover.pick != hp {
        // station pins have their own hover style: mirror the canvas hover onto the pin
        if let Some(Pick::Station(i)) = hover.pick {
            crate::scene::labels::set_attrs(&crate::scene::labels::label_id(i), &[("data-hover", "0")]);
        }
        if let Some(Pick::Station(i)) = hp {
            crate::scene::labels::set_attrs(&crate::scene::labels::label_id(i), &[("data-hover", "1")]);
        }
        set_canvas_cursor(hp.is_some());
        hover.label = hp.map(|p| hover_label(p, stops.as_deref(), checks.as_deref(), data.as_ref().and_then(|d| d.chem.as_ref()),
                                              bridge.panel.as_ref())).unwrap_or_default();
        hover.pick = hp;
    }
    if let Some((_, at)) = hit {
        hover.at = at;
    }
    // click
    if !active {
        *press = None;
        return;
    }
    if buttons.just_pressed(MouseButton::Left) {
        *press = cursor;
    }
    if !buttons.just_released(MouseButton::Left) {
        return;
    }
    let (Some(p0), Some(p1)) = (press.take(), cursor) else { return };
    if p0.distance(p1) >= 4.0 || !in_map_area(p1, size) {
        return;
    }
    // Move relay / working spot: this click places it (scene/place.rs)
    if let Some(gp) = ground_pt {
        if crate::scene::place::click(gp, stops.as_deref()) {
            return;
        }
    }
    match (hit, ground_pt) {
        (Some((p, at)), _) => sel.set(Some(p), at),
        (None, Some(gp)) if sel.pick.is_some() => sel.set(None, gp),
        _ => {}
    }
}

/// Fly-in state: orbit parameters interpolated from `from` to `to` while `t` < 1.
#[derive(Resource, Default)]
pub struct FlyTo {
    pub from: Option<(Vec3, f32, f32, f32)>,
    pub to: (Vec3, f32, f32, f32),
    pub t: f32,
}

fn wrap_pi(a: f32) -> f32 {
    (a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

/// Advance the fly-in (before orbit_controls, which settles the camera on the terrain and writes the transform). A mouse
/// press hands control back to the user at once.
pub fn fly(time: Res<Time>, buttons: Res<ButtonInput<MouseButton>>, mut f: ResMut<FlyTo>, mut orbits: Query<&mut Orbit>) {
    let Some(from) = f.from else { return };
    if buttons.get_just_pressed().next().is_some() {
        f.from = None;
        return;
    }
    f.t = (f.t + time.delta_secs() / FLY_S).min(1.0);
    let s = f.t * f.t * (3.0 - 2.0 * f.t);
    let to = f.to;
    if let Ok(mut o) = orbits.single_mut() {
        o.focus = from.0.lerp(to.0, s);
        o.yaw = from.1 + wrap_pi(to.1 - from.1) * s;
        o.pitch = from.2 + (to.2 - from.2) * s;
        o.radius = from.3 + (to.3 - from.3) * s;
    }
    if f.t >= 1.0 {
        f.from = None;
    }
}

/// A station card opened from elsewhere (map label, panel table, timeline, card arrows) selects that station.
pub fn follow_card(mode: Res<Mode>, card: Option<Res<crate::ui::card::CardState>>, stops: Option<Res<Stops>>, mut sel: ResMut<Selection>) {
    let (Some(card), Some(stops)) = (card, stops) else { return };
    if *mode != Mode::Plan || !card.is_changed() {
        return;
    }
    if let Some(i) = card.open {
        if sel.pick != Some(Pick::Station(i)) {
            if let Some(s) = stops.0.get(i) {
                sel.set(Some(Pick::Station(i)), (s.e, s.n));
            }
        }
    }
}

/// React to a new selection: open the station card, publish the drill-in card, fly the 3D view to the spot.
#[allow(clippy::too_many_arguments)]
pub fn apply(
    mut sel: ResMut<Selection>,
    mut view: ResMut<PlanView>,
    map: Option<Res<MapCam>>,
    mut orbits: Query<&mut Orbit>,
    ground: Option<Res<Ground>>,
    mut fly: ResMut<FlyTo>,
    mut bridge: ResMut<UiBridge>,
    checks: Option<Res<crate::scene::checks::Checks>>,
    data: Option<Res<crate::scene::inspect::InspectData>>,
    docs: Option<Res<crate::BundleDocs>>,
    card: Option<Res<crate::ui::card::CardState>>,
) {
    if !sel.changed {
        return;
    }
    sel.changed = false;
    let Some(p) = sel.pick else {
        bridge.set_drill(DrillModel::default());
        return;
    };
    let chem = data.as_ref().and_then(|d| d.chem.as_ref());
    let panos = docs.as_ref().and_then(|d| d.panos.as_ref());
    // placed points' cards are written by place::eval (they hold the coverage numbers)
    if !matches!(p, Pick::Relay | Pick::Work(_)) {
        let m = crate::ui::drill::model(p, sel.at, bridge.panel.as_ref(), checks.as_ref().map(|c| c.items.as_slice()).unwrap_or(&[]), chem, panos);
        bridge.set_drill(m);
    }
    if let Pick::Station(i) = p {
        if card.as_ref().is_none_or(|c| c.open != Some(i)) {
            crate::ui::card::send(crate::ui::card::CardEvent::Open(i));
        }
    }
    // fly: from the map, start straight down over the map centre at the map's scale, then tilt into 3D
    let (Ok(mut o), Some(g)) = (orbits.single_mut(), ground) else { return };
    if *view == PlanView::Map {
        if let Some(m) = map.as_ref() {
            o.focus = Vec3::new(m.e, g.0.height_at(m.e, m.n), -m.n);
            o.radius = (m.height_m * 0.9).clamp(o.min_radius, o.max_radius);
            o.pitch = 89f32.to_radians();
            o.yaw = 0.0;
        }
        *view = PlanView::Orbit;
    }
    let (e, n) = sel.at;
    let to = (Vec3::new(e, g.0.height_at(e, n), -n), o.yaw, FLY_PITCH_DEG.to_radians(), FLY_RADIUS_M.clamp(o.min_radius, o.max_radius));
    *fly = FlyTo { from: Some((o.focus, o.yaw, o.pitch, o.radius)), to, t: 0.0 };
}

/// A watch chip's problem drawn over the map (Plan tab chips): the route stretches out of sight of the exit, or rings on
/// the stations named by the chip.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub enum Highlight {
    Hidden,
    Stops(Vec<usize>),
    /// stretches still out of sight with the suggested relay
    Runs(Vec<Vec<(f32, f32)>>),
    /// safer working spots beside steep targets: (target, spot) pairs
    Spots(Vec<((f32, f32), (f32, f32))>),
}

pub static HIGHLIGHT: Mutex<Option<Highlight>> = Mutex::new(None);
/// Bumped on every highlight change, so `shade` can tell a new highlight without formatting it every frame.
pub static HIGHLIGHT_GEN: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn highlight(h: Option<Highlight>) {
    if let Ok(mut g) = HIGHLIGHT.lock() {
        *g = h;
    }
    HIGHLIGHT_GEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn current_highlight() -> Option<Highlight> {
    HIGHLIGHT.lock().ok().and_then(|g| g.clone())
}

/// Runs of the profile where the exit is out of sight, as point lists (e, n).
pub fn hidden_runs(samples: &[crate::ui::panel::Sample]) -> Vec<Vec<(f32, f32)>> {
    let mut out: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut cur: Vec<(f32, f32)> = Vec::new();
    for w in samples.windows(2) {
        if !w[1].seen {
            if cur.is_empty() {
                cur.push((w[0].e, w[0].n));
            }
            cur.push((w[1].e, w[1].n));
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// A station pin being dragged (set by the pin's pointer handlers in labels.rs, read by `drag`): stop index, window
/// position of the pointer (CSS px), moved past the drag threshold, released.
#[derive(Clone, Copy, Debug, Default)]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub struct Drag {
    pub i: usize,
    pub start: (f32, f32),
    pub pos: (f32, f32),
    pub moved: bool,
    pub done: bool,
}

pub static DRAG: Mutex<Option<Drag>> = Mutex::new(None);
/// Set when a pin drag ends, so the click that follows does not open the card.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub static DRAGGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Where the dragged station would go (stop index, east, north), while dragging.
#[derive(Resource, Default)]
pub struct DragPreview(pub Option<(usize, f32, f32)>);

/// Pin drag: follow the pointer on the ground; on release, move the station (planner re-plans the route).
pub fn drag(mode: Res<Mode>, cams: Query<(&Camera, &GlobalTransform), With<Orbit>>, ground: Option<Res<Ground>>,
            mut preview: ResMut<DragPreview>) {
    let Some(d) = DRAG.lock().ok().and_then(|g| *g) else {
        if preview.0.is_some() {
            preview.0 = None;
        }
        return;
    };
    let (Ok((cam, ct)), Some(g)) = (cams.single(), ground) else { return };
    let at = if d.moved && *mode == Mode::Plan { ground_at(cam, ct, &g.0, Vec2::new(d.pos.0, d.pos.1)) } else { None };
    if d.done {
        if let (Some((e, n)), true) = (at, d.moved) {
            crate::plan::planner::send(crate::plan::planner::PanelEvent::Move(d.i, e, n));
        }
        if let Ok(mut g) = DRAG.lock() {
            *g = None;
        }
        preview.0 = None;
        return;
    }
    let want = at.map(|(e, n)| (d.i, e, n));
    if preview.0 != want {
        preview.0 = want;
    }
}

/// Plan 3D: Curiosity's model where it parked at the selected stop (its PLACES heading, pitch and roll): a rover-check stop,
/// or a station's recorded pose. Hidden in the map view (from straight above it hides the ground) and in Walk.
#[allow(clippy::too_many_arguments)]
pub fn show_rover(
    sel: Res<Selection>,
    mode: Res<Mode>,
    view: Res<PlanView>,
    checks: Option<Res<crate::scene::checks::Checks>>,
    poses: Option<Res<crate::scene::curiosity::Poses>>,
    stops: Option<Res<Stops>>,
    ground: Option<Res<Ground>>,
    mut q: Query<(&mut Transform, &mut Visibility), With<crate::scene::curiosity::SelRover>>,
) {
    let Some(g) = ground else { return };
    let pose: Option<(f32, f32, f32, f32, f32)> = if *mode != Mode::Plan || *view != PlanView::Orbit { None } else {
        match sel.pick {
            Some(Pick::Rover(i)) => checks.as_ref().and_then(|c| c.items.get(i)).and_then(|c| c.pose.map(|p| (c.e, c.n, p.0, p.1, p.2))),
            Some(Pick::Station(i)) => {
                // the recorded pose at this station's position (walking order can differ from stops.json order); a custom or
                // moved stop is not where Curiosity parked, so it gets none
                let at = stops.as_ref().and_then(|s| s.0.get(i)).filter(|s| !s.id.starts_with("custom-")).map(|s| (s.e, s.n));
                at.and_then(|(e, n)| poses.as_ref().and_then(|p| p.0.iter().find(|x| (x.e - e).hypot(x.n - n) < 1.5)))
                    .map(|p| (p.e, p.n, p.yaw_deg, p.pitch_deg, p.roll_deg))
            }
            _ => None,
        }
    };
    for (mut t, mut vis) in &mut q {
        let want = if pose.is_some() { Visibility::Visible } else { Visibility::Hidden };
        if let Some((e, n, yaw, pitch, roll)) = pose {
            let nt = Transform::from_xyz(e, g.0.height_at(e, n), -n).with_rotation(crate::scene::curiosity::rotation(yaw, pitch, roll));
            if *t != nt {
                *t = nt;
            }
        }
        if *vis != want {
            *vis = want;
        }
    }
}

/// The suggested relay point, once found (the Walk HUD's comms check uses it too).
pub static RELAY_AT: Mutex<Option<(f32, f32)>> = Mutex::new(None);

/// "Suggest relay" was pressed (Plan tab / Risk): relay_run computes on the next frame.
pub static RELAY_ASK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn ask_relay() {
    RELAY_ASK.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Find the relay / EV2 point for the plan on screen (plan/relay.rs), show it: fly there, ring it, dash what stays hidden,
/// and a card with the before / after sight of the route and the caveat.
pub fn relay_run(mut bridge: ResMut<UiBridge>, ground: Option<Res<Ground>>, back: Option<Res<crate::ui::hud::WalkBack>>) {
    if !RELAY_ASK.swap(false, std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    use crate::text::drill as t;
    let (Some(g), Some(back)) = (ground, back) else { return };
    let Some(pm) = bridge.panel.clone() else { return };
    match crate::plan::relay::suggest(&g.0, &pm.samples, &|e, n| back.at(e, n)) {
        // the suggestion is placed at once (a pin with Move / Remove: scene/place.rs scores and shows it)
        Some(r) => crate::scene::place::send(crate::scene::place::PlaceAct::Relay(r.e, r.n)),
        None => bridge.set_drill(DrillModel { open: true, kind: t::KIND_RELAY, title: t::RELAY_NONE.to_string(), ..Default::default() }),
    }
}

/// Distance along the loop (m) of the profile sample nearest to `at`.
pub fn route_dist(samples: &[crate::ui::panel::Sample], at: (f32, f32)) -> Option<f32> {
    samples.iter().min_by(|a, b| ((a.e - at.0).powi(2) + (a.n - at.1).powi(2)).total_cmp(&((b.e - at.0).powi(2) + (b.n - at.1).powi(2)))).map(|q| q.d)
}

/// Buttons on the drill-in card.
#[allow(clippy::too_many_arguments)]
pub fn events(
    mut sel: ResMut<Selection>,
    mut view: ResMut<PlanView>,
    map: Option<ResMut<MapCam>>,
    mut mode: ResMut<Mode>,
    bridge: Res<UiBridge>,
    mut fly: ResMut<FlyTo>,
) {
    let evs: Vec<SelEvent> = EVENTS.lock().map(|mut q| q.drain(..).collect()).unwrap_or_default();
    if evs.is_empty() {
        return;
    }
    let mut map = map;
    for ev in evs {
        match ev {
            SelEvent::Pick(p, at) => sel.set(Some(p), at),
            SelEvent::BackToMap => {
                fly.from = None;
                if let Some(m) = map.as_mut() {
                    m.e = sel.at.0;
                    m.n = sel.at.1;
                    m.user_moved = true;
                }
                *view = PlanView::Map;
            }
            SelEvent::Close => {
                let at = sel.at;
                sel.set(None, at);
            }
            SelEvent::WalkFrom => {
                // the EVA simulation places the walker: seek it to the route point nearest the selection (for a station,
                // its arrival), then switch to Walk
                if let Some(d) = bridge.panel.as_ref().and_then(|pm| route_dist(&pm.samples, sel.at)) {
                    crate::plan::simrun::send(crate::plan::simrun::SimEvent::SeekDist(d));
                }
                *mode = Mode::Walk;
            }
        }
    }
}

/// Paint thick polylines into an Rg8 mask (2 texels per metre over a w x h m grid, row 0 north): `ch` 0 = r, 1 = g;
/// coverage 255 within `half_w` metres of a line, anti-aliased over one texel.
pub fn paint(mask: &mut [u8], w: usize, h: usize, lines: &[Vec<(f32, f32)>], half_w: f32, ch: usize) {
    let k = crate::scene::trust::TEX_PER_M as f32;
    let (tw, th) = (w * crate::scene::trust::TEX_PER_M, h * crate::scene::trust::TEX_PER_M);
    for line in lines {
        for sgm in line.windows(2) {
            let (a, b) = (sgm[0], sgm[1]);
            let pad = half_w + 1.0;
            let (x0, x1) = (((a.0.min(b.0) - pad) * k).floor().max(0.0) as usize, (((a.0.max(b.0) + pad) * k).ceil() as usize).min(tw));
            let (y0, y1) = ((((h as f32 - a.1.max(b.1)) - pad) * k).floor().max(0.0) as usize, ((((h as f32 - a.1.min(b.1)) + pad) * k).ceil() as usize).min(th));
            for y in y0..y1 {
                for x in x0..x1 {
                    let p = ((x as f32 + 0.5) / k, h as f32 - (y as f32 + 0.5) / k);
                    let (d, _) = seg_dist(p, a, b);
                    let cov = ((half_w + 0.5 / k - d) * k).clamp(0.0, 1.0);
                    let i = (y * tw + x) * crate::scene::trust::MASK_CH + ch;
                    let v = (cov * 255.0).round() as u8;
                    if v > mask[i] {
                        mask[i] = v;
                    }
                }
            }
        }
    }
}

/// Paint the route into the mask: coverage in b (within `half_w` m), and in a the phase of the distance walked along it
/// (0..1 over `period` m), so the shader can show the walking direction. Where segments overlap, the nearer one wins.
pub fn paint_route(mask: &mut [u8], w: usize, h: usize, poly: &[(f32, f32)], half_w: f32, period: f32) {
    let k = crate::scene::trust::TEX_PER_M as f32;
    let (tw, th) = (w * crate::scene::trust::TEX_PER_M, h * crate::scene::trust::TEX_PER_M);
    let mut along = 0.0f32;
    for sgm in poly.windows(2) {
        let (a, b) = (sgm[0], sgm[1]);
        let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        let pad = half_w + 1.0;
        let (x0, x1) = (((a.0.min(b.0) - pad) * k).floor().max(0.0) as usize, (((a.0.max(b.0) + pad) * k).ceil() as usize).min(tw));
        let (y0, y1) = ((((h as f32 - a.1.max(b.1)) - pad) * k).floor().max(0.0) as usize, ((((h as f32 - a.1.min(b.1)) + pad) * k).ceil() as usize).min(th));
        for y in y0..y1 {
            for x in x0..x1 {
                let p = ((x as f32 + 0.5) / k, h as f32 - (y as f32 + 0.5) / k);
                let (d, q) = seg_dist(p, a, b);
                let cov = (((half_w + 0.5 / k - d) * k).clamp(0.0, 1.0) * 255.0).round() as u8;
                let i = (y * tw + x) * crate::scene::trust::MASK_CH;
                if cov > 0 && cov >= mask[i + 2] {
                    mask[i + 2] = cov;
                    let s = along + ((q.0 - a.0).powi(2) + (q.1 - a.1).powi(2)).sqrt();
                    mask[i + 3] = ((s / period).fract() * 255.0) as u8;
                }
            }
        }
        along += len;
    }
}

/// Route width painted on the ground (half, m) and the length of one direction ramp (m). VISUAL.
pub const ROUTE_HALF_M: f32 = 0.55;
pub const ROUTE_PERIOD_M: f32 = 4.0;

/// A circle as a closed polyline (for ringed stations in the mask).
pub fn circle(c: (f32, f32), r: f32) -> Vec<(f32, f32)> {
    (0..=48).map(|i| { let a = i as f32 / 48.0 * std::f32::consts::TAU; (c.0 + r * a.cos(), c.1 + r * a.sin()) }).collect()
}

/// Selection, hover and the watch-chip problem drawn by the terrain shader (overlay.wesl): rings as uniforms, the selected
/// leg and the chip's stretches as a mask. Ring radius is about RING_PX on screen (at least RING_MIN_M).
#[allow(clippy::too_many_arguments)]
pub fn shade(
    mode: Res<Mode>,
    sel: Res<Selection>,
    hover: Res<Hover>,
    windows: Query<&Window>,
    cams: Query<(&Camera, &GlobalTransform), With<Orbit>>,
    ground: Option<Res<Ground>>,
    route: Option<Res<crate::ui::hud::RouteData>>,
    stops: Option<Res<Stops>>,
    bridge: Res<UiBridge>,
    overlays: Option<ResMut<crate::scene::trust::Overlays>>,
    mut mats: ResMut<Assets<crate::scene::trust::TerrainMaterial>>,
    mut images: ResMut<Assets<Image>>,
    drag_preview: Option<Res<DragPreview>>,
    route_layer: Query<&Visibility, With<crate::scene::layers::RouteLayer>>,
    (walk_target, walker): (Option<Res<crate::scene::walk::WalkTarget>>, Option<Res<crate::scene::walk::Walker>>),
    mut last_key: Local<Option<ShadeKey>>,
) {
    let (Some(mut ov), Some(g)) = (overlays, ground) else { return };
    let plan = *mode == Mode::Plan;
    // metres per RING_PX at the window centre
    let ring_m = match (windows.single(), cams.single()) {
        (Ok(win), Ok((cam, ct))) => {
            let c = Vec2::new(win.width() / 2.0, win.height() / 2.0);
            match (ground_at(cam, ct, &g.0, c), ground_at(cam, ct, &g.0, c + Vec2::new(RING_PX, 0.0))) {
                (Some(a), Some(b)) => ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt(),
                _ => RING_MIN_M,
            }
        }
        _ => RING_MIN_M,
    }.clamp(RING_MIN_M, 40.0);
    let wt = walk_target.as_ref().and_then(|t| t.0).filter(|_| !plan);
    let sel_v = match (wt, sel.pick) {
        (Some((e, n, _)), _) => Vec4::new(e, n, 1.2, 1.0),
        (_, Some(p)) if plan && !matches!(p, Pick::Leg(_)) => Vec4::new(sel.at.0, sel.at.1, ring_m, 1.0),
        _ => Vec4::ZERO,
    };
    let hov_v = match (drag_preview.as_ref().and_then(|p| p.0), hover.pick) {
        (Some((_, e, n)), _) if plan => Vec4::new(e, n, ring_m, 1.0),
        _ => match hover.pick {
        Some(p) if plan && Some(p) != sel.pick && !matches!(p, Pick::Leg(_)) => Vec4::new(hover.at.0, hover.at.1, ring_m * 0.75, 1.0),
        _ => Vec4::ZERO,
        },
    };
    if ov.sel != sel_v || ov.hover != hov_v {
        ov.sel = sel_v;
        ov.hover = hov_v;
        ov.push(&mut mats);
    }
    // mask: the selected (or hovered) leg, and the active watch chip
    let leg = match (sel.pick, hover.pick) {
        (Some(Pick::Leg(i)), _) => Some(i),
        (_, Some(Pick::Leg(i))) => Some(i),
        _ => None,
    };
    let hl = if plan { current_highlight() } else { None };
    let route_on = route_layer.iter().all(|v| *v != Visibility::Hidden);
    if ov.route_on != route_on {
        ov.route_on = route_on;
        ov.push(&mut mats);
    }
    let leg = if plan { leg } else { None };
    let nroute = route.as_ref().map(|r| r.poly.len()).unwrap_or(0);
    let gen = HIGHLIGHT_GEN.load(std::sync::atomic::Ordering::Relaxed);
    let way = wt.and_then(|(e, n, _)| walker.as_ref().map(|w| ((w.e, w.n), (e, n))));
    // the way to a walk target repaints when the target changes (not every step of the walker); compared every frame, so
    // a plain tuple (no string formatting)
    let way_key = wt.map(|(e, n, _)| ((e * 10.0).round() as i32, (n * 10.0).round() as i32));
    let key: ShadeKey = (leg, hl.is_some(), gen, nroute, bridge.panel.as_ref().map(|p| p.samples.len()).unwrap_or(0), way_key);
    if last_key.as_ref() == Some(&key) {
        return;
    }
    let Some(handle) = mats.get(&ov.material).map(|m| m.extension.hl.clone()) else { return };
    let (w, h) = (g.0.w, g.0.h);
    // paint into the texture's own buffer (no 4.8 MB allocation per change; load profile: malloc was ~0.3 s)
    let size = w * h * crate::scene::trust::TEX_PER_M * crate::scene::trust::TEX_PER_M * crate::scene::trust::MASK_CH;
    let mut mask = images.get_mut(&handle).and_then(|mut img| img.data.take()).unwrap_or_default();
    if mask.len() == size { mask.fill(0) } else { mask = vec![0u8; size] }
    if let (Some(i), Some(rd)) = (leg, route.as_ref()) {
        if let Some(&(a, b)) = rd.legs.get(i) {
            let line = rd.poly.get(a..=b.min(rd.poly.len().saturating_sub(1))).unwrap_or(&[]).to_vec();
            paint(&mut mask, w, h, &[line], 0.4, 0);
        }
    }
    if let Some(rd) = route.as_ref() {
        paint_route(&mut mask, w, h, &crate::scene::layers::chaikin(&rd.poly, 2), ROUTE_HALF_M, ROUTE_PERIOD_M);
    }
    if let Some((a, b)) = way {
        paint(&mut mask, w, h, &[vec![a, b]], 0.3, 0);
    }
    match hl {
        Some(Highlight::Hidden) => {
            let runs = bridge.panel.as_ref().map(|pm| hidden_runs(&pm.samples)).unwrap_or_default();
            paint(&mut mask, w, h, &runs, 1.0, 1);
        }
        Some(Highlight::Stops(ix)) => {
            let rings: Vec<Vec<(f32, f32)>> = ix.iter().filter_map(|&i| stops.as_ref().and_then(|s| s.0.get(i)).map(|s| circle((s.e, s.n), 5.0))).collect();
            paint(&mut mask, w, h, &rings, 0.6, 1);
        }
        Some(Highlight::Runs(runs)) => paint(&mut mask, w, h, &runs, 1.0, 1),
        Some(Highlight::Spots(sp)) => {
            // a ring on the working spot and a dashed link from the target to it
            let rings: Vec<Vec<(f32, f32)>> = sp.iter().map(|(_, s)| circle(*s, 1.4)).collect();
            paint(&mut mask, w, h, &rings, 0.35, 1);
            let links: Vec<Vec<(f32, f32)>> = sp.iter().map(|(t, s)| vec![*t, *s]).collect();
            paint(&mut mask, w, h, &links, 0.2, 1);
        }
        None => {}
    }
    if let Some(mut img) = images.get_mut(&handle) {
        img.data = Some(mask);
        *last_key = Some(key);
    }
    // touching the material makes the new mask visible this frame
    let _ = mats.get_mut(&ov.material);
}

/// What `shade` compares each frame before repainting the mask.
type ShadeKey = (Option<usize>, bool, u32, usize, usize, Option<(i32, i32)>);

/// Ring size on screen (px) and its smallest size on the ground (m). VISUAL.
pub const RING_PX: f32 = 16.0;
pub const RING_MIN_M: f32 = 2.5;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stations_win_over_rover_stops_and_points_over_legs() {
        let pts = [(Pick::Rover(3), (10.0, 10.0)), (Pick::Station(1), (10.5, 10.0)), (Pick::Chem(7), (30.0, 30.0))];
        let legs = vec![vec![(0.0, 9.8), (40.0, 9.8)]];
        assert_eq!(nearest(&pts, &legs, 10.1, 10.0, 3.0).map(|x| x.0), Some(Pick::Station(1)), "station on the same spot wins");
        assert_eq!(nearest(&pts, &legs, 29.0, 30.0, 3.0).map(|x| x.0), Some(Pick::Chem(7)));
        let (p, at) = nearest(&pts, &legs, 25.0, 10.5, 3.0).unwrap();
        assert_eq!(p, Pick::Leg(0), "only the leg is near");
        assert!((at.0 - 25.0).abs() < 1e-4 && (at.1 - 9.8).abs() < 1e-4, "the clicked point on the leg");
        assert!(nearest(&pts, &legs, 25.0, 20.0, 3.0).is_none(), "nothing within reach");
    }

    #[test]
    fn hidden_runs_split_where_the_exit_is_seen() {
        let s = |d: f32, seen: bool| crate::ui::panel::Sample { d, e: d, n: 0.0, z: 0.0, s: 0.0, seen };
        let runs = hidden_runs(&[s(0.0, true), s(1.0, false), s(2.0, false), s(3.0, true), s(4.0, false)]);
        assert_eq!(runs, vec![vec![(0.0, 0.0), (1.0, 0.0), (2.0, 0.0)], vec![(3.0, 0.0), (4.0, 0.0)]]);
        assert!(hidden_runs(&[s(0.0, true), s(1.0, true)]).is_empty());
    }

    #[test]
    fn paint_covers_the_line_and_not_beyond() {
        let (w, h) = (10, 10);
        let mut m = vec![0u8; w * h * 4 * crate::scene::trust::MASK_CH];
        paint(&mut m, w, h, &[vec![(1.0, 5.0), (9.0, 5.0)]], 0.5, 0);
        let at = |e: f32, n: f32, ch: usize| { let (x, y) = ((e * 2.0) as usize, ((h as f32 - n) * 2.0) as usize); m[(y * w * 2 + x) * crate::scene::trust::MASK_CH + ch] };
        assert_eq!(at(5.0, 5.0, 0), 255, "on the line");
        assert_eq!(at(5.0, 7.0, 0), 0, "2 m away");
        assert_eq!(at(5.0, 5.0, 1), 0, "other channel untouched");
        assert!(circle((0.0, 0.0), 2.0).iter().all(|p| ((p.0 * p.0 + p.1 * p.1).sqrt() - 2.0).abs() < 1e-4));
    }

    #[test]
    fn route_paint_marks_coverage_and_a_rising_phase() {
        let (w, h) = (20, 10);
        let mut m = vec![0u8; w * h * 4 * crate::scene::trust::MASK_CH];
        paint_route(&mut m, w, h, &[(1.0, 5.0), (19.0, 5.0)], 0.5, 4.0);
        let px = |e: f32, n: f32| ((((h as f32 - n) * 2.0) as usize) * w * 2 + (e * 2.0) as usize) * crate::scene::trust::MASK_CH;
        assert_eq!(m[px(5.0, 5.0) + 2], 255, "on the route");
        assert_eq!(m[px(5.0, 7.0) + 2], 0, "off the route");
        let (a, b) = (m[px(2.0, 5.0) + 3], m[px(4.0, 5.0) + 3]);
        assert!(b > a, "phase grows in the walking direction: {a} {b}");
    }

    #[test]
    fn segment_distance_clamps_to_the_ends() {
        assert_eq!(seg_dist((5.0, 3.0), (0.0, 0.0), (10.0, 0.0)), (3.0, (5.0, 0.0)));
        assert_eq!(seg_dist((-4.0, 3.0), (0.0, 0.0), (10.0, 0.0)).0, 5.0);
        assert_eq!(wrap_pi(3.0 * std::f32::consts::PI).abs(), std::f32::consts::PI);
    }
}
