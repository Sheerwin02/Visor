//! Placed plan points: the relay / EV2 position and safer working spots beside steep targets. Each one is a point the
//! planner sets with a click: "Place" puts VISOR's suggestion (plan/relay.rs) on the map, "Move" arms the next map click
//! (scene/select.rs pick), "Remove" drops it. Placed points get a pin on the map and in Walk, a drill-in card with their
//! numbers (re-scored on every re-plan), the Plan / Risk chips, the Walk comms check (relay) and the share link.
//! Numbers are the app's terrain-only tests on the 1 m map (sight lines with 1.7 m eyes, corrected 3 m slope, walk-back
//! field): the 1-2 m ridges the map cannot see may block more, and the cards say so.
use std::sync::Mutex;

use bevy::prelude::*;

use crate::scene::select::{Pick, Selection};
use crate::scene::walk::{Ground, SlopeMap, Stops};
use crate::ui::model::{DrillModel, UiBridge};

/// Actions from the UI (chips, drill-card buttons, pins) and the armed map click.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub enum PlaceAct {
    /// put the relay at this point (the suggestion, or a map click while armed)
    Relay(f32, f32),
    RemoveRelay,
    /// working spot for the station with this stop id
    Work(String, f32, f32),
    /// several at once (the Plan chip places every suggested spot)
    WorkAll(Vec<(String, f32, f32)>),
    RemoveWork(String),
    /// the next map click places this point (Move ...)
    Arm(Arm),
    Disarm,
    /// a pin was clicked
    SelectRelay,
    SelectWork(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Arm {
    Relay,
    Work(String),
}

pub static ACTS: Mutex<Vec<PlaceAct>> = Mutex::new(Vec::new());
/// What the next map click places (read by select::pick).
pub static ARMED: Mutex<Option<Arm>> = Mutex::new(None);

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn send(a: PlaceAct) {
    if let Ok(mut q) = ACTS.lock() {
        q.push(a);
    }
}

pub fn armed() -> Option<Arm> {
    ARMED.lock().ok().and_then(|g| g.clone())
}

/// The placed points (stop ids for working spots, so they follow a station through re-orders).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Placed {
    pub relay: Option<(f32, f32)>,
    pub work: Vec<(String, (f32, f32))>,
}

#[derive(Resource, Default, Debug)]
pub struct Placements {
    pub p: Placed,
    pub gen: u32,
}

/// What the panel shows about the placed points (UiBridge -> inspector chips).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PlacedModel {
    /// in contact before / with the relay (share of the route), and whether the relay spot sees the exit
    pub relay: Option<(f32, f32, bool)>,
    /// stop ids with a placed working spot
    pub work: Vec<String>,
    /// hint while a map click is armed
    pub armed: Option<String>,
}

/// What `eval` compares each frame before doing any work.
type EvalKey = (u32, usize, Option<u32>, usize, Option<Pick>, (u32, u32), Option<Arm>, bool);

/// Map pick radius for a working spot: further than this from its station it is not "beside" the target. VISUAL.
pub const WORK_REACH_M: f32 = 8.0;

/// Apply the queued actions; Esc cancels an armed click.
pub fn events(mut pl: ResMut<Placements>, mut sel: ResMut<Selection>, keys: Res<ButtonInput<KeyCode>>, stops: Option<Res<Stops>>,
              mut loaded: Local<bool>) {
    // a shared link carries the placed points (plan/share.rs rl= / ws=)
    #[cfg(target_arch = "wasm32")]
    if !*loaded {
        *loaded = true;
        let p = crate::plan::share::decode_placed(&crate::plan::share::web::query());
        if p != Placed::default() {
            pl.p = p;
            pl.gen += 1;
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = &mut loaded;
    if keys.just_pressed(KeyCode::Escape) && armed().is_some() {
        set_armed(None);
        pl.gen += 1;
    }
    let acts: Vec<PlaceAct> = ACTS.lock().map(|mut q| q.drain(..).collect()).unwrap_or_default();
    if acts.is_empty() {
        return;
    }
    let index_of = |id: &str| stops.as_ref().and_then(|s| s.0.iter().position(|x| x.id == id));
    for a in acts {
        match a {
            PlaceAct::Relay(e, n) => {
                pl.p.relay = Some((e, n));
                set_armed(None);
                sel.set(Some(Pick::Relay), (e, n));
            }
            PlaceAct::RemoveRelay => {
                pl.p.relay = None;
                if sel.pick == Some(Pick::Relay) {
                    let at = sel.at;
                    sel.set(None, at);
                }
            }
            PlaceAct::Work(id, e, n) => {
                pl.p.work.retain(|w| w.0 != id);
                pl.p.work.push((id.clone(), (e, n)));
                set_armed(None);
                if let Some(i) = index_of(&id) {
                    sel.set(Some(Pick::Work(i)), (e, n));
                }
            }
            PlaceAct::WorkAll(v) => {
                let first = v.first().cloned();
                for (id, e, n) in v {
                    pl.p.work.retain(|w| w.0 != id);
                    pl.p.work.push((id, (e, n)));
                }
                if let Some((id, e, n)) = first {
                    if let Some(i) = index_of(&id) {
                        sel.set(Some(Pick::Work(i)), (e, n));
                    }
                }
            }
            PlaceAct::RemoveWork(id) => {
                pl.p.work.retain(|w| w.0 != id);
                if let Some(i) = index_of(&id) {
                    if sel.pick == Some(Pick::Work(i)) {
                        // back to the station itself
                        if let Some(s) = stops.as_ref().and_then(|s| s.0.get(i)) {
                            sel.set(Some(Pick::Station(i)), (s.e, s.n));
                        }
                    }
                }
            }
            PlaceAct::Arm(a) => set_armed(Some(a)),
            PlaceAct::Disarm => set_armed(None),
            PlaceAct::SelectRelay => {
                if let Some(r) = pl.p.relay {
                    sel.set(Some(Pick::Relay), r);
                }
            }
            PlaceAct::SelectWork(id) => {
                if let (Some(i), Some(w)) = (index_of(&id), pl.p.work.iter().find(|w| w.0 == id)) {
                    sel.set(Some(Pick::Work(i)), w.1);
                }
            }
        }
        pl.gen += 1;
    }
}

fn set_armed(a: Option<Arm>) {
    if let Ok(mut g) = ARMED.lock() {
        *g = a;
    }
}

/// Re-score the placed points when they or the plan change; publish the panel model, the Walk comms relay, the pins and
/// the card of a selected placed point.
#[allow(clippy::too_many_arguments)]
pub fn eval(
    pl: Res<Placements>,
    sel: Res<Selection>,
    mut bridge: ResMut<UiBridge>,
    ground: Option<Res<Ground>>,
    slope: Option<Res<SlopeMap>>,
    back: Option<Res<crate::ui::hud::WalkBack>>,
    stops: Option<Res<Stops>>,
    data: Option<Res<crate::scene::inspect::InspectData>>,
    mut last: Local<Option<EvalKey>>,
) {
    let (Some(g), Some(back), Some(stops)) = (ground, back, stops) else { return };
    // cheap key first (runs every frame); the panel model is cloned only when something changed
    let Some(pm_ref) = bridge.panel.as_ref() else { return };
    // a new station card from select::apply has no buttons yet: that re-runs this too
    let key: EvalKey = (pl.gen, pm_ref.samples.len(), pm_ref.samples.last().map(|s| s.d.to_bits()), stops.0.len(), sel.pick,
                        (sel.at.0.to_bits(), sel.at.1.to_bits()), armed(), bridge.drill.as_ref().is_some_and(|d| d.acts.is_empty()));
    if last.as_ref() == Some(&key) {
        return;
    }
    *last = Some(key);
    let pm = pm_ref.clone();
    use crate::text::place as t;
    let bias = data.as_ref().map(|d| d.bias).unwrap_or(0.0);
    let relay = pl.p.relay.and_then(|(e, n)| crate::plan::relay::evaluate(&g.0, &pm.samples, &|x, y| back.at(x, y), e, n));
    // the Walk comms check counts a relay only when it sees the exit
    if let Ok(mut r) = crate::scene::select::RELAY_AT.lock() {
        *r = pl.p.relay.filter(|_| relay.as_ref().is_some_and(|x| x.sees_exit));
    }
    let work: Vec<(usize, String, (f32, f32))> = pl.p.work.iter()
        .filter_map(|(id, at)| stops.0.iter().position(|s| &s.id == id).map(|i| (i, id.clone(), *at))).collect();
    let armed_hint = armed().map(|a| match a {
        Arm::Relay => t::ARMED_RELAY.to_string(),
        Arm::Work(id) => crate::text::fill(t::ARMED_WORK, &[("name", stops.0.iter().position(|s| s.id == id)
            .map(|i| crate::scene::labels::label_text(i, &stops.0[i].name)).unwrap_or_default())]),
    });
    let model = PlacedModel { relay: relay.as_ref().map(|r| (r.before, r.after, r.sees_exit)), work: work.iter().map(|w| w.1.clone()).collect(),
                              armed: armed_hint.clone() };
    bridge.set_placed(model);
    pins(pl.p.relay, &work, &stops);
    #[cfg(target_arch = "wasm32")]
    {
        use crate::plan::share as sh;
        let q = sh::web::query();
        let nq = sh::with_placed(&q, &pl.p);
        if nq != q.trim_start_matches('?') {
            sh::web::set_query(&nq);
        }
        if let Some(b) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.body()) {
            let _ = b.set_attribute("data-placing", if armed().is_some() { "1" } else { "0" });
        }
    }
    // the card of a selected placed point
    let pct = |x: f32| format!("{}%", (x * 100.0).round() as i64);
    match sel.pick {
        Some(Pick::Relay) => {
            let Some(r) = relay else { return };
            let mut rows = vec![(t::R_CONTACT, format!("{} \u{2192} {}", pct(r.before), pct(r.after))),
                                (crate::text::drill::R_FROM_EXIT, match r.back_s {
                                    Some(b) => format!("{:.0} m \u{b7} {:.1} min back", r.from_exit_m, b / 60.0),
                                    None => format!("{:.0} m \u{b7} {}", r.from_exit_m, t::NO_WAY_BACK),
                                })];
            if !r.sees_exit {
                rows.push((t::R_WARN, t::NO_EXIT_SIGHT.to_string()));
            } else if r.back_s.is_some_and(|b| b > crate::plan::relay::RELAY_MAX_BACK_S) {
                rows.push((t::R_WARN, crate::text::fill(t::FAR, &[("x", format!("{:.0}", crate::plan::relay::RELAY_MAX_BACK_S / 60.0))])));
            }
            let mut m = DrillModel { open: true, kind: t::KIND_RELAY, title: t::RELAY_TITLE.to_string(), rows,
                                     note: armed_hint.clone().unwrap_or_else(|| crate::text::drill::RELAY_NOTE.to_string()), at: sel.at, ..Default::default() };
            m.acts = vec![(t::MOVE.to_string(), t::MOVE_TIP, PlaceAct::Arm(Arm::Relay), true),
                          (t::REMOVE.to_string(), t::REMOVE_TIP, PlaceAct::RemoveRelay, false)];
            if armed_hint.is_some() {
                m.acts[0] = (t::CANCEL.to_string(), t::CANCEL_TIP, PlaceAct::Disarm, true);
            }
            bridge.set_drill(m);
            crate::scene::select::highlight(Some(crate::scene::select::Highlight::Runs(r.hidden_after)));
        }
        Some(Pick::Work(i)) => {
            let Some((_, id, (x, y))) = work.iter().find(|w| w.0 == i).cloned() else { return };
            let Some(s) = stops.0.get(i) else { return };
            let d = (x - s.e).hypot(y - s.n);
            let dir = crate::ui::drill::compass8(crate::scene::walk::bearing_deg(s.e, s.n, x, y));
            let corr = |e: f32, n: f32| slope.as_ref().map(|sg| (sg.0.height_at(e, n) - bias).max(0.0));
            let mut rows = vec![(t::R_FROM_TARGET, format!("{d:.1} m {dir}"))];
            if let (Some(a), Some(b)) = (corr(x, y), corr(s.e, s.n)) {
                rows.push((t::R_SLOPE, format!("{a:.0}\u{b0} here \u{b7} {b:.0}\u{b0} at the target")));
            }
            let card = pm.stop_cards.iter().find(|c| c.index == i);
            if let Some((_, a, r)) = card.and_then(|c| c.ground.as_ref()) {
                rows.push((crate::text::drill::R_TILT, format!("{a:.1}\u{b0} / {r:.1}\u{b0}")));
            }
            rows.push((crate::text::drill::R_BACK, match back.at(x, y) { Some(b) => format!("{:.1} min", b / 60.0), None => t::NO_WAY_BACK.to_string() }));
            if d > crate::plan::relay::WORK_MAX_M + 0.5 {
                rows.push((t::R_WARN, crate::text::fill(t::OUT_OF_REACH, &[("x", format!("{:.0}", crate::plan::relay::WORK_MAX_M))])));
            }
            let mut m = DrillModel { open: true, kind: t::KIND_WORK, title: crate::text::fill(t::WORK_TITLE, &[("name", crate::scene::labels::label_text(i, &s.name))]),
                                     rows, note: armed_hint.clone().unwrap_or_else(|| t::WORK_NOTE.to_string()), at: (x, y), ..Default::default() };
            m.acts = vec![(t::MOVE.to_string(), t::MOVE_WORK_TIP, PlaceAct::Arm(Arm::Work(id.clone())), true),
                          (t::REMOVE.to_string(), t::REMOVE_TIP, PlaceAct::RemoveWork(id.clone()), false)];
            if armed_hint.is_some() {
                m.acts[0] = (t::CANCEL.to_string(), t::CANCEL_TIP, PlaceAct::Disarm, true);
            }
            bridge.set_drill(m);
            crate::scene::select::highlight(Some(crate::scene::select::Highlight::Spots(vec![((s.e, s.n), (x, y))])));
        }
        Some(Pick::Station(i)) => {
            let (Some(s), Some(mut m)) = (stops.0.get(i), bridge.drill.clone()) else { return };
            if !m.open || m.station != Some(i) {
                return;
            }
            let sugg = pm.stop_cards.iter().find(|c| c.index == i).and_then(|c| c.work_spot);
            m.acts = station_acts(&pl.p, &s.id, sugg);
            match armed_hint {
                Some(h) => {
                    m.note = h;
                    m.acts = vec![(t::CANCEL.to_string(), t::CANCEL_TIP, PlaceAct::Disarm, true)];
                }
                None => m.note.clear(),
            }
            bridge.set_drill(m);
        }
        _ => {}
    }
}

/// Buttons a station's drill-in card gets for its working spot (select::apply).
pub fn station_acts(placed: &Placed, id: &str, suggestion: Option<(f32, f32, f32, f32)>) -> Vec<(String, &'static str, PlaceAct, bool)> {
    use crate::text::place as t;
    if placed.work.iter().any(|w| w.0 == id) {
        return vec![(t::SHOW_WORK.to_string(), t::SHOW_WORK_TIP, PlaceAct::SelectWork(id.to_string()), false)];
    }
    let mut v = Vec::new();
    if let Some((x, y, d, _)) = suggestion {
        v.push((crate::text::fill(t::USE_WORK, &[("x", format!("{d:.0}"))]), t::USE_WORK_TIP, PlaceAct::Work(id.to_string(), x, y), true));
    }
    v.push((t::SET_WORK.to_string(), t::SET_WORK_TIP, PlaceAct::Arm(Arm::Work(id.to_string())), false));
    v
}

/// A map click while armed: place the point there (a working spot only within WORK_REACH_M of its station).
pub fn click(at: (f32, f32), stops: Option<&Stops>) -> bool {
    let Some(a) = armed() else { return false };
    match a {
        Arm::Relay => send(PlaceAct::Relay(at.0, at.1)),
        Arm::Work(id) => {
            let near = stops.and_then(|s| s.0.iter().find(|x| x.id == id)).map(|s| (s.e, s.n));
            match near {
                Some((e, n)) => {
                    // clamp to the reach ring around the station, so a stray click does not put it across the map
                    let (dx, dy) = (at.0 - e, at.1 - n);
                    let d = dx.hypot(dy);
                    let k = if d > WORK_REACH_M { WORK_REACH_M / d } else { 1.0 };
                    send(PlaceAct::Work(id, e + dx * k, n + dy * k));
                }
                None => send(PlaceAct::Disarm),
            }
        }
    }
    true
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn relay_pin_id() -> &'static str {
    "place-relay"
}

/// Pin elements for the placed points (recreated when the set changes; positions in `pins_follow`).
fn pins(relay: Option<(f32, f32)>, work: &[(usize, String, (f32, f32))], stops: &Stops) {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsCast;
        let Some(doc) = web_sys::window().and_then(|w| w.document()) else { return };
        let Some(body) = doc.body() else { return };
        // drop the previous pins (their ids are in PIN_PTS)
        for (id, _) in PIN_PTS.lock().map(|g| g.clone()).unwrap_or_default() {
            if let Some(el) = doc.get_element_by_id(&id) {
                el.remove();
            }
        }
        let mut pts: Vec<(String, (f32, f32))> = Vec::new();
        let mut make = |id: String, html: String, title: String, at: (f32, f32), act: PlaceAct| {
            let Ok(el) = doc.create_element("div") else { return };
            el.set_id(&id);
            el.set_class_name("place-pin");
            el.set_inner_html(&html);
            let _ = el.set_attribute("title", &title);
            pts.push((id.clone(), at));
            let _ = el.set_attribute("style", "display:none");
            let cb = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || send(act.clone()));
            let _ = el.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref());
            cb.forget();
            let _ = body.append_child(&el);
        };
        use crate::text::place as t;
        if let Some(r) = relay {
            make(relay_pin_id().to_string(), format!("<b>R</b><span>{}</span>", t::PIN_RELAY), t::PIN_TIP.to_string(), r, PlaceAct::SelectRelay);
        }
        for (i, id, at) in work {
            let name = stops.0.get(*i).map(|s| crate::scene::labels::short_name(&s.name).to_string()).unwrap_or_default();
            make(format!("place-work-{i}"), format!("<b>W</b><span>{}</span>", i + 1), crate::text::fill(t::PIN_WORK_TIP, &[("name", name)]),
                 *at, PlaceAct::SelectWork(id.clone()));
        }
        if let Ok(mut g) = PIN_PTS.lock() {
            *g = pts;
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (relay, work, stops);
}

/// Pin ids and ground points, written by `pins`, followed by `pins_follow`.
static PIN_PTS: Mutex<Vec<(String, (f32, f32))>> = Mutex::new(Vec::new());

/// Every frame: keep the pins on their ground points (Plan and Walk), hidden behind the camera.
pub fn pins_follow(cams: Query<(&Camera, &GlobalTransform), With<Camera3d>>, ground: Option<Res<Ground>>, mut cache: Local<Vec<String>>) {
    let (Ok((cam, ct)), Some(g)) = (cams.single(), ground) else { return };
    // nothing placed (the usual case): no work at all
    let Ok(guard) = PIN_PTS.lock() else { return };
    if guard.is_empty() && cache.is_empty() {
        return;
    }
    let pts: Vec<(String, (f32, f32))> = guard.clone();
    drop(guard);
    let css: Vec<String> = pts.iter().map(|(id, (e, n))| {
        let p = Vec3::new(*e, g.0.height_at(*e, *n), -*n);
        let front = (p - ct.translation()).dot(*ct.forward()) > 0.0;
        match cam.world_to_viewport(ct, p) {
            Ok(v) if front => format!("{id}|left:{}px;top:{}px", v.x.round() as i32, v.y.round() as i32),
            _ => format!("{id}|display:none"),
        }
    }).collect();
    if *cache == css {
        return;
    }
    for (k, (id, _)) in pts.iter().enumerate() {
        if cache.get(k) != css.get(k) {
            crate::ui::chrome::style(id, css[k].split_once('|').map(|x| x.1).unwrap_or(""));
        }
    }
    *cache = css;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn station_buttons_offer_the_suggestion_then_show_the_placed_spot() {
        let mut p = Placed::default();
        let a = station_acts(&p, "rio_frio", Some((10.0, 20.0, 6.0, 5.0)));
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].2, PlaceAct::Work("rio_frio".into(), 10.0, 20.0));
        assert!(a[0].0.contains('6'), "{}", a[0].0);
        assert_eq!(a[1].2, PlaceAct::Arm(Arm::Work("rio_frio".into())));
        // no suggestion: only "set by click"
        assert_eq!(station_acts(&p, "x", None).len(), 1);
        p.work.push(("rio_frio".into(), (10.0, 20.0)));
        let b = station_acts(&p, "rio_frio", None);
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].2, PlaceAct::SelectWork("rio_frio".into()));
    }
}
