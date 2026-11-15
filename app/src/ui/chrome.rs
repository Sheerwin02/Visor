//! Light HTML overlay over the canvas, written directly: status box, north arrow, scale bar, page mode attributes. The mode
//! chip and mode button are Leptos components (ui/views) fed with a ChromeModel from here. On native builds these calls only
//! log, so the app and its tests run without a browser.
use bevy::prelude::*;

use crate::scene::camera::{nice_length, Orbit};
use crate::scene::mapview::{MapCam, PlanView};
use crate::ui::model::{ChromeModel, UiBridge};
pub use crate::text::fill;
use crate::scene::walk::{Ground, Mode, Pace, Walker};

/// Set an element's text (web) or log it (native).
pub fn text(id: &str, value: &str) {
    #[cfg(target_arch = "wasm32")]
    if let Some(el) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.get_element_by_id(id)) {
        el.set_text_content(Some(value));
    }
    #[cfg(not(target_arch = "wasm32"))]
    bevy::log::info!("[{id}] {value}");
}

/// Set an element's inline style (web only).
pub fn style(id: &str, value: &str) {
    #[cfg(target_arch = "wasm32")]
    if let Some(el) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.get_element_by_id(id)) {
        let _ = el.set_attribute("style", value);
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (id, value);
}

/// Set an attribute (web only). Used for the walker's position, which the headless smoke test reads.
pub fn attr(id: &str, name: &str, value: &str) {
    #[cfg(target_arch = "wasm32")]
    if let Some(el) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.get_element_by_id(id)) {
        let _ = el.set_attribute(name, value);
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (id, name, value);
}

pub fn status(msg: &str) {
    text("status", msg);
    style("status", "");
}

pub fn hide_status() {
    style("status", "display:none");
}

/// Last values written to the DOM, so the overlay is only touched when something changes.
#[derive(Resource, Default)]
pub struct OverlayCache {
    pub north_deg: i32,
    pub bar: (i32, i32),
    pub mode: Option<Mode>,
    pub walker: String,
    /// K toggles the full key list on the mode chip (short by default, so it does not run off narrow screens)
    pub keys_shown: bool,
    pub keys_drawn: Option<bool>,
    pub view: Option<PlanView>,
}

pub fn toggle_keys(keys: Res<ButtonInput<KeyCode>>, mut cache: ResMut<OverlayCache>) {
    if crate::scene::controls::tapped(&keys, crate::scene::controls::Act::ToggleKeys) {
        cache.keys_shown = !cache.keys_shown;
    }
}

const BAR_MAX_PX: f32 = 140.0;

/// North arrow rotation and scale bar length, updated from the orbit camera.
pub fn update_overlay(
    windows: Query<&Window>,
    cams: Query<(&Orbit, &Projection, &Transform)>,
    ground: Option<Res<Ground>>,
    mode: Res<Mode>,
    walker: Option<Res<Walker>>,
    pace: Option<Res<Pace>>,
    mut bridge: ResMut<UiBridge>,
    view: Res<PlanView>,
    map: Option<Res<MapCam>>,
    mut cache: ResMut<OverlayCache>,
    mut last_cam: Local<Option<(Transform, f32, u32)>>,
) {
    let (Ok(win), Ok((o, proj, cam))) = (windows.single(), cams.single()) else { return };
    let walking = *mode == Mode::Walk;
    // mode chip and scale-bar visibility change only when the mode changes
    let in_map = !walking && *view == PlanView::Map;
    if cache.mode != Some(*mode) || cache.keys_drawn != Some(cache.keys_shown) || cache.view != Some(*view) {
        cache.view = Some(*view);
        attr("page", "data-mode", if walking { "walk" } else { "plan" });
        attr("page", "data-view", if in_map { "map" } else { "orbit" });
        if cache.mode != Some(*mode) {
            cache.bar = (0, 0);
        }
        cache.mode = Some(*mode);
        cache.keys_drawn = Some(cache.keys_shown);
        style("scale", if walking { "display:none" } else { "" });
    }
    bridge.set_chrome(ChromeModel {
        walking,
        in_map,
        keys_shown: cache.keys_shown,
        speed: pace.as_ref().map(|p| format!("{}", p.flat_mps)).unwrap_or_default(),
    });
    // north arrow: rotate by minus the view heading so it keeps pointing north on screen
    let heading = if walking { walker.as_ref().map(|w| w.heading).unwrap_or(0.0) } else if in_map { 0.0 } else { o.yaw };
    let north = (-heading.to_degrees()).round() as i32;
    if north != cache.north_deg {
        cache.north_deg = north;
        style("north", &format!("transform: rotate({north}deg)"));
    }
    if walking {
        // e, n (m) and heading (deg) on the mode chip as data-walker, for the headless key test; written only when it changes
        if let Some(w) = walker.as_ref() {
            let s = format!("{:.1},{:.1},{:.1}", w.e, w.n, w.heading.to_degrees().rem_euclid(360.0));
            if s != cache.walker {
                attr("mode", "data-walker", &s);
                cache.walker = s;
            }
        }
        return;
    }
    // scale bar: metres per CSS pixel across the screen at the ground point under the screen centre (true there only;
    // a perspective view changes scale with distance). Hidden when the centre of the screen shows sky.
    let (m_per_px, exact) = match proj {
        // orthographic map: the same scale everywhere
        Projection::Orthographic(_) => match map.as_ref() {
            Some(m) => (m.m_per_px(win.height()), true),
            None => return,
        },
        Projection::Perspective(p) => {
            // the terrain ray march only when the camera, its field of view or the window height changed
            let now = (*cam, p.fov, win.height().to_bits());
            if *last_cam == Some(now) && cache.bar != (0, 0) {
                return;
            }
            *last_cam = Some(now);
            let hit = ground.as_ref().and_then(|g| g.0.ray_hit(cam.translation, *cam.forward(), p.far));
            let Some(dist) = hit else {
                if cache.bar != (-1, -1) {
                    cache.bar = (-1, -1);
                    style("scale", "display:none");
                }
                return;
            };
            (2.0 * dist * (p.fov / 2.0).tan() / win.height().max(1.0), false)
        }
        _ => return,
    };
    if cache.bar == (-1, -1) {
        style("scale", "");
    }
    let len = nice_length(BAR_MAX_PX * m_per_px);
    let px = (len / m_per_px).round() as i32;
    let key = (len.round() as i32, px);
    if key != cache.bar {
        cache.bar = key;
        style("scale-bar", &format!("width:{px}px"));
        let tpl = if exact { crate::text::scale::EXACT } else { crate::text::scale::CENTRE };
        text("scale-label", &fill(tpl, &[("metres", format!("{}", len as i32))]));
    }
}

