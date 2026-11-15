//! Stop names on screen (HTML labels over the canvas), placed above each flag in both modes. Hidden when the flag is behind
//! the camera, off screen, or the stops layer is off (key 6). Names are the part of the stop name before ":".
use bevy::prelude::*;

use crate::scene::layers::{StopsLayer, POLE_M};
use crate::ui::chrome::style;
use crate::scene::walk::{Ground, Stops};

/// Last CSS written per label, so the DOM is only touched when a label moves by a pixel or more.
#[derive(Resource, Default)]
pub struct LabelCache(pub Vec<String>, pub Vec<String>);

/// Set attributes on a label element (web); a no-op natively.
pub fn set_attrs(id: &str, attrs: &[(&str, &str)]) {
    #[cfg(target_arch = "wasm32")]
    if let Some(el) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.get_element_by_id(id)) {
        for (k, v) in attrs {
            let _ = el.set_attribute(k, v);
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (id, attrs);
}

/// Approximate label box (CSS px) for a label of `chars` characters: 12 px system font, 6 px padding each side.
pub fn label_box(chars: usize) -> Vec2 {
    Vec2::new(7.0 * chars as f32 + 15.0, 18.0)
}

/// Place labels anchored at `anchors` (bottom-centre, CSS px, None = not shown) so they do not overlap: each label, in stop
/// order, moves up one label height at a time until it is clear of the ones already placed. Returns the placed anchors.
pub fn declutter(anchors: &[Option<Vec2>], sizes: &[Vec2]) -> Vec<Option<Vec2>> {
    let mut placed: Vec<(Vec2, Vec2)> = Vec::new(); // (bottom-centre, size)
    anchors
        .iter()
        .zip(sizes)
        .map(|(a, &sz)| {
            let mut p = (*a)?;
            let hits = |p: Vec2, placed: &Vec<(Vec2, Vec2)>| {
                placed.iter().any(|(q, qs)| (p.x - q.x).abs() < (sz.x + qs.x) / 2.0 + 2.0 && (p.y - q.y).abs() < (sz.y + qs.y) / 2.0 + 1.0)
            };
            for _ in 0..40 {
                if !hits(p, &placed) {
                    break;
                }
                p.y -= sz.y + 2.0;
            }
            placed.push((p, sz));
            Some(p)
        })
        .collect()
}

/// Short label for a stop name: "Bococo: where ridges meet" -> "Bococo".
pub fn short_name(name: &str) -> &str {
    name.split(':').next().unwrap_or(name).trim()
}

/// Numbered label, matching the top bar ("Stop 2 of 6") and the panel's profile and leg table: "2 Bococo".
pub fn label_text(i: usize, name: &str) -> String {
    format!("{} {}", i + 1, short_name(name))
}

/// Pin markup: a numbered disc (on the stop in Plan) and the short name. Plain text in it: names come from stops.json or
/// the planner's "Custom stop k", escaped anyway.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn pin_html(i: usize, name: &str) -> String {
    let esc = short_name(name).replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    format!("<b class=\"pin-n\">{}</b><span class=\"pin-name\">{esc}</span>", i + 1)
}

/// Pin badge class for a station: its rover check, with "under" where Curiosity measured steeper ground than the map
/// (the hazard direction); empty without a check.
pub fn pin_badge(s: &crate::ui::model::StopSummary) -> String {
    if s.map_understates() {
        return "under".into();
    }
    s.ground.as_ref().map(|g| g.0.clone()).unwrap_or_default()
}

pub fn label_id(i: usize) -> String {
    format!("flag-label-{i}")
}

/// Create one label element per stop (web only).
pub fn create_labels(stops: &Stops) {
    #[cfg(target_arch = "wasm32")]
    {
        let Some(doc) = web_sys::window().and_then(|w| w.document()) else { return };
        let Some(body) = doc.body() else { return };
        for (i, s) in stops.0.iter().enumerate() {
            if let Some(el) = doc.get_element_by_id(&label_id(i)) {
                el.set_inner_html(&pin_html(i, &s.name));
                continue;
            }
            if let Ok(el) = doc.create_element("div") {
                use wasm_bindgen::JsCast;
                el.set_id(&label_id(i));
                el.set_class_name("flag-label");
                let _ = el.set_attribute("title", crate::text::stop_card::OPEN_HINT);
                // a click opens the card of whichever stop is at this position in the walking order now
                let cb = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || {
                    // the click that ends a drag does not open the card
                    if !crate::scene::select::DRAGGED.swap(false, std::sync::atomic::Ordering::Relaxed) {
                        crate::ui::card::send(crate::ui::card::CardEvent::Open(i));
                    }
                });
                let _ = el.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref());
                cb.forget();
                // drag a station pin to move it (Plan mode; the exit, index 0, stays)
                if i > 0 {
                    let down = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::PointerEvent)>::new(move |ev: web_sys::PointerEvent| {
                        let walk = web_sys::window().and_then(|w| w.document()).and_then(|d| d.body())
                            .and_then(|b| b.get_attribute("data-mode")).is_some_and(|m| m == "walk");
                        if walk || ev.button() != 0 {
                            return;
                        }
                        if let Some(t) = ev.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) {
                            let _ = t.set_pointer_capture(ev.pointer_id());
                        }
                        let p = (ev.client_x() as f32, ev.client_y() as f32);
                        if let Ok(mut g) = crate::scene::select::DRAG.lock() {
                            *g = Some(crate::scene::select::Drag { i, start: p, pos: p, moved: false, done: false });
                        }
                    });
                    let mv = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::PointerEvent)>::new(move |ev: web_sys::PointerEvent| {
                        if let Ok(mut g) = crate::scene::select::DRAG.lock() {
                            if let Some(d) = g.as_mut().filter(|d| d.i == i && !d.done) {
                                d.pos = (ev.client_x() as f32, ev.client_y() as f32);
                                if (d.pos.0 - d.start.0).hypot(d.pos.1 - d.start.1) > 5.0 {
                                    d.moved = true;
                                }
                            }
                        }
                    });
                    let up = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::PointerEvent)>::new(move |_ev: web_sys::PointerEvent| {
                        if let Ok(mut g) = crate::scene::select::DRAG.lock() {
                            if let Some(d) = g.as_mut().filter(|d| d.i == i) {
                                d.done = true;
                                if d.moved {
                                    crate::scene::select::DRAGGED.store(true, std::sync::atomic::Ordering::Relaxed);
                                }
                            }
                        }
                    });
                    let _ = el.add_event_listener_with_callback("pointerdown", down.as_ref().unchecked_ref());
                    let _ = el.add_event_listener_with_callback("pointermove", mv.as_ref().unchecked_ref());
                    let _ = el.add_event_listener_with_callback("pointerup", up.as_ref().unchecked_ref());
                    let _ = el.add_event_listener_with_callback("pointercancel", up.as_ref().unchecked_ref());
                    down.forget();
                    mv.forget();
                    up.forget();
                }
                el.set_inner_html(&pin_html(i, &s.name));
                let _ = el.set_attribute("style", "display:none");
                let _ = body.append_child(&el);
            }
        }
        // stops removed by the planner: drop their labels
        let mut i = stops.0.len();
        while let Some(el) = doc.get_element_by_id(&label_id(i)) {
            el.remove();
            i += 1;
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = stops;
}

pub fn update_labels(
    cams: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    stops: Res<Stops>,
    ground: Option<Res<Ground>>,
    layer: Query<&Visibility, With<StopsLayer>>,
    mut cache: ResMut<LabelCache>,
    mode: Res<crate::scene::walk::Mode>,
    walker: Option<Res<crate::scene::walk::Walker>>,
    sel: Option<Res<crate::scene::select::Selection>>,
    drag: Option<Res<crate::scene::select::DragPreview>>,
    bridge: Res<crate::ui::model::UiBridge>,
    mut last_sel: Local<Option<usize>>,
    mut last_badges: Local<Vec<(String, bool)>>,
) {
    // pin badges from the plan: rover check at the station (direction of disagreement included) and the drill pair
    let badges: Vec<(String, bool)> = stops.0.iter().enumerate().map(|(i, _)| {
        bridge.panel.as_ref().and_then(|pm| pm.stop_cards.iter().find(|s| s.index == i)).map(|s| (pin_badge(s), s.pair)).unwrap_or_default()
    }).collect();
    if *last_badges != badges {
        for (i, (rc, pair)) in badges.iter().enumerate() {
            set_attrs(&label_id(i), &[("data-rc", rc.as_str()), ("data-pair", if *pair { "1" } else { "0" })]);
        }
        *last_badges = badges;
    }
    // the selected station's pin (scene/select.rs)
    let picked = match sel.as_ref().and_then(|s| s.pick) {
        Some(crate::scene::select::Pick::Station(i)) => Some(i),
        _ => None,
    };
    if *last_sel != picked {
        if let Some(i) = *last_sel {
            set_attrs(&label_id(i), &[("data-sel", "0")]);
        }
        if let Some(i) = picked {
            set_attrs(&label_id(i), &[("data-sel", "1")]);
        }
        *last_sel = picked;
    }
    // Walk: AR plates on every stop in view, with the straight-line distance; the next stop is marked
    let walk = *mode == crate::scene::walk::Mode::Walk;
    // Walk shows only the next stop's plate (every stop's plate piled up on the horizon)
    let only: Option<usize> = if walk { walker.as_ref().map(|w| w.next % stops.0.len().max(1)) } else { None };
    if walk {
        if let Some(w) = walker.as_ref() {
            let next = w.next % stops.0.len().max(1);
            for (i, s) in stops.0.iter().enumerate() {
                let d = ((s.e - w.e).powi(2) + (s.n - w.n).powi(2)).sqrt();
                let v = format!("{}|{}", d.round() as i64, i == next);
                if cache.1.get(i) != Some(&v) {
                    set_attrs(&label_id(i), &[("data-d", &format!("{} m", d.round() as i64)), ("data-next", if i == next { "1" } else { "0" })]);
                    if cache.1.len() != stops.0.len() {
                        cache.1 = vec![String::new(); stops.0.len()];
                    }
                    cache.1[i] = v;
                }
            }
        }
    }
    let (Ok((cam, cam_t)), Some(ground)) = (cams.single(), ground) else { return };
    let shown = layer.iter().all(|v| *v != Visibility::Hidden);
    if cache.0.len() != stops.0.len() {
        cache.0 = vec![String::new(); stops.0.len()];
    }
    let anchors: Vec<Option<Vec2>> = stops
        .0
        .iter()
        .enumerate()
        .map(|(i, s)| {
            // a pin being dragged follows the pointer on the ground
            let (se, sn) = match drag.as_ref().and_then(|d| d.0) { Some((k, e, n)) if k == i && !walk => (e, n), _ => (s.e, s.n) };
            // Plan: the pin sits on the stop itself; Walk: the plate floats above the stop
            let top = if walk { Vec3::new(s.e + crate::scene::layers::POLE_OFFSET_M, ground.0.height_at(s.e, s.n) + POLE_M + 1.0, -s.n) }
                      else { Vec3::new(se, ground.0.height_at(se, sn), -sn) };
            let in_front = (top - cam_t.translation()).dot(*cam_t.forward()) > 0.0;
            match cam.world_to_viewport(cam_t, top) {
                Ok(p) if shown && in_front && only.is_none_or(|k| k == i) => Some(p),
                _ => None,
            }
        })
        .collect();
    let sizes: Vec<Vec2> = stops.0.iter().enumerate().map(|(i, s)| label_box(label_text(i, &s.name).chars().count())).collect();
    for (i, p) in declutter(&anchors, &sizes).into_iter().enumerate() {
        // --lead: length of the leader line from a moved label back down to its flag
        let css = match (p, anchors[i]) {
            (Some(p), Some(a)) => format!("left:{}px;top:{}px;--lead:{}px", p.x.round() as i32, p.y.round() as i32, (a.y - p.y).round().max(0.0) as i32),
            _ => "display:none".to_string(),
        };
        if cache.0[i] != css {
            style(&label_id(i), &css);
            cache.0[i] = css;
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::math::Vec2;

    #[test]
    fn overlapping_labels_stack_and_distant_ones_stay() {
        let sz = Vec2::new(100.0, 18.0);
        let a = [Some(Vec2::new(200.0, 300.0)), Some(Vec2::new(205.0, 302.0)), Some(Vec2::new(600.0, 300.0)), None];
        let p = super::declutter(&a, &[sz; 4]);
        assert_eq!(p[0], a[0], "first label stays");
        let (p0, p1) = (p[0].unwrap(), p[1].unwrap());
        assert!((p0.y - p1.y).abs() >= 18.0, "second label moved clear of the first: {p0:?} {p1:?}");
        assert_eq!(p[2], a[2], "a distant label is not moved");
        assert_eq!(p[3], None, "hidden stays hidden");
    }

    #[test]
    fn pins_carry_the_number_and_an_escaped_short_name() {
        assert_eq!(super::pin_html(1, "Bococo: where ridges meet"), "<b class=\"pin-n\">2</b><span class=\"pin-name\">Bococo</span>");
        assert!(super::pin_html(0, "A<b>&").contains("A&lt;b&gt;&amp;"));
    }

    #[test]
    fn badges_follow_the_rover_check_and_its_direction() {
        use crate::ui::model::StopSummary;
        let s = |l: &str, a: f64, r: f64| StopSummary { ground: Some((l.into(), a, r)), ..Default::default() };
        assert_eq!(super::pin_badge(&s("agrees", 9.0, 9.5)), "agrees");
        assert_eq!(super::pin_badge(&s("disagrees", 22.8, 6.8)), "disagrees");
        assert_eq!(super::pin_badge(&s("disagrees", 11.5, 17.5)), "under");
        assert_eq!(super::pin_badge(&StopSummary::default()), "");
    }

    #[test]
    fn short_names_drop_the_description() {
        assert_eq!(super::short_name("Bococo: where ridges meet"), "Bococo");
        assert_eq!(super::short_name("Exit"), "Exit");
        assert_eq!(super::label_text(1, "Bococo: where ridges meet"), "2 Bococo");
    }
}
