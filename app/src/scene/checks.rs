//! Rover-check marks (key 3): at each of the 85 localized Curiosity stops, a shape saying how well the map's tilt matched the
//! rover's measured tilt there (rover_check.json, rule fixed before the run): check mark = agrees, tilde = partial,
//! cross = disagrees. Shapes carry the meaning, not colour. Hover a mark for the numbers. Plan mode only.
use bevy::prelude::*;

use crate::scene::controls::{tapped, Act};
use crate::ui::chrome::{fill, style};
use crate::scene::walk::{Ground, Mode};

#[derive(Clone, Debug, PartialEq)]
pub struct Check {
    pub e: f32,
    pub n: f32,
    pub label: String,
    pub sol: i64,
    pub rover_tilt: f32,
    pub dem_tilt: f32,
    /// PLACES pose (rover_check.json, augmented by build_bundle rehash): heading (N 0, E 90), pitch, roll (deg)
    pub pose: Option<(f32, f32, f32)>,
}

#[derive(Resource, Default)]
pub struct Checks {
    pub items: Vec<Check>,
    pub shown: bool,
    pub cache: Vec<String>,
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn glyph(label: &str) -> &'static str {
    match label {
        "agrees" => "\u{2713}",    // check mark
        "partial" => "~",
        "disagrees" => "\u{2717}", // cross
        _ => "?",
    }
}

pub fn parse(v: &serde_json::Value) -> Vec<Check> {
    v["stops"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|s| {
                    Some(Check {
                        e: s["e_m"].as_f64()? as f32,
                        n: s["n_m"].as_f64()? as f32,
                        label: s["label"].as_str()?.to_string(),
                        sol: s["sol"].as_i64()?,
                        rover_tilt: s["rover_tilt"].as_f64()? as f32,
                        dem_tilt: s["dem_tilt"].as_f64()? as f32,
                        pose: (|| Some((s["yaw_deg"].as_f64()? as f32, s["pitch_deg"].as_f64()? as f32, s["roll_deg"].as_f64()? as f32)))(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn mark_id(i: usize) -> String {
    format!("rc-{i}")
}

/// Create one mark element per check (web only), with its hover text.
pub fn create_marks(checks: &[Check]) {
    #[cfg(target_arch = "wasm32")]
    {
        let Some(doc) = web_sys::window().and_then(|w| w.document()) else { return };
        let Some(body) = doc.body() else { return };
        let tip = crate::text::legend::ROVER_CHECK_TIP;
        for (i, c) in checks.iter().enumerate() {
            if let Ok(el) = doc.create_element("div") {
                el.set_id(&mark_id(i));
                el.set_class_name(&format!("rc-mark rc-{}", c.label));
                el.set_text_content(Some(glyph(&c.label)));
                let t = fill(&tip, &[("sol", c.sol.to_string()), ("rover_tilt_deg", format!("{:.1}", c.rover_tilt)),
                                     ("dem_tilt_deg", format!("{:.1}", c.dem_tilt)), ("label", c.label.clone())]);
                let _ = el.set_attribute("title", &t);
                let _ = el.set_attribute("aria-label", &t);
                let _ = el.set_attribute("style", "display:none");
                // a click selects this stop (drill-in card + fly-in, scene/select.rs)
                use wasm_bindgen::JsCast;
                let (e, n) = (c.e, c.n);
                let cb = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move ||
                    crate::scene::select::send(crate::scene::select::SelEvent::Pick(crate::scene::select::Pick::Rover(i), (e, n))));
                let _ = el.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref());
                cb.forget();
                let _ = body.append_child(&el);
            }
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (checks, fill);
}

pub fn toggle(keys: Res<ButtonInput<KeyCode>>, mut checks: ResMut<Checks>) {
    if tapped(&keys, Act::LayerChecks) {
        checks.shown = !checks.shown;
    }
}

pub fn update_marks(cams: Query<(&Camera, &GlobalTransform), With<Camera3d>>, mode: Res<Mode>, ground: Option<Res<Ground>>, mut checks: ResMut<Checks>) {
    let (Ok((cam, cam_t)), Some(ground)) = (cams.single(), ground) else { return };
    let show = checks.shown && *mode == Mode::Plan;
    let n = checks.items.len();
    if checks.cache.len() != n {
        checks.cache = vec![String::new(); n];
    }
    for i in 0..n {
        let c = &checks.items[i];
        let p = Vec3::new(c.e, ground.0.height_at(c.e, c.n), -c.n);
        let in_front = (p - cam_t.translation()).dot(*cam_t.forward()) > 0.0;
        let css = match cam.world_to_viewport(cam_t, p) {
            Ok(v) if show && in_front => format!("left:{}px;top:{}px", v.x.round() as i32, v.y.round() as i32),
            _ => "display:none".to_string(),
        };
        if checks.cache[i] != css {
            style(&mark_id(i), &css);
            checks.cache[i] = css;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_label_has_a_distinct_shape() {
        let g: Vec<&str> = ["agrees", "partial", "disagrees"].iter().map(|l| glyph(l)).collect();
        assert!(g[0] != g[1] && g[1] != g[2] && g[0] != g[2]);
    }

    #[test]
    fn parses_rover_check_entries() {
        let v: serde_json::Value = serde_json::from_str(r#"{"stops":[{"sol":4534,"rover_tilt":8.982,"dem_tilt":9.211,"label":"agrees","e_m":493.4,"n_m":1018.2}]}"#).unwrap();
        let c = parse(&v);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].label, "agrees");
        assert_eq!(c[0].sol, 4534);
    }
}
