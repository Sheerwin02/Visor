//! Point readout (JMARS "Investigate"-style: the values of every layer under the cursor). In Plan mode (map or 3D)
//! the ground point under the mouse shows: position (lat/lon), elevation, map and corrected slope at 3 m, the data-readiness
//! class there (readiness.bin), the nearest rover-checked stop (rover check label and distance) and the straight-line
//! distance to the exit. Hidden in Walk, over the side panels, and off the map. Values come from the bundle grids only.
use bevy::prelude::*;

use crate::scene::camera::Orbit;
use crate::scene::walk::{Ground, Mode, SlopeMap, Stops};

/// Bundle data the readout needs besides the grids.
#[derive(Resource, Default)]
pub struct InspectData {
    /// readiness class per cell (row 0 north) and its legend (code -> words)
    pub ready: Option<Vec<u8>>,
    pub classes: Vec<(u8, String)>,
    /// rover-checked stops: east, north, sol, label
    pub rover: Vec<(f32, f32, i64, String)>,
    pub z_offset: f32,
    pub grid: serde_json::Value,
    pub bias: f32,
    /// chemistry.json (ChemCam + APXS along the walk)
    pub chem: Option<serde_json::Value>,
}

/// The lines of the readout for a ground point (e, n east/north m from the grid's south-west corner).
pub fn lines(d: &InspectData, ground: &crate::scene::terrain::Grid, slope: &crate::scene::terrain::Grid, exit: Option<(f32, f32)>,
             e: f32, n: f32) -> Vec<(&'static str, String)> {
    use crate::text::inspect as t;
    let mut out = Vec::new();
    if let Some((la, lo)) = crate::scene::basemap::latlon(&d.grid, e as f64, n as f64) {
        out.push((t::POS, format!("{la:.5}\u{b0}, {lo:.5}\u{b0}E")));
    }
    out.push((t::ELEV, format!("{:.1} m", ground.height_at(e, n) + d.z_offset)));
    let s = slope.height_at(e, n);
    out.push((t::SLOPE, format!("{:.1}\u{b0} map \u{b7} {:.1}\u{b0} corrected", s, (s - d.bias).max(0.0))));
    if let Some(b) = d.ready.as_ref() {
        let (col, row) = ((e / ground.cell).floor() as i64, (ground.h as f32 - n / ground.cell).floor() as i64);
        if col >= 0 && row >= 0 && (col as usize) < ground.w && (row as usize) < ground.h {
            let code = b[row as usize * ground.w + col as usize];
            if let Some((_, w)) = d.classes.iter().find(|c| c.0 == code) {
                out.push((t::READY, w.clone()));
            }
        }
    }
    if let Some((re, rn, sol, lab)) = d.rover.iter().min_by(|a, b| ((a.0 - e).powi(2) + (a.1 - n).powi(2)).total_cmp(&((b.0 - e).powi(2) + (b.1 - n).powi(2)))) {
        let dist = ((re - e).powi(2) + (rn - n).powi(2)).sqrt();
        out.push((t::ROVER, crate::text::fill(t::ROVER_V, &[("x", format!("{dist:.0}")), ("sol", sol.to_string()), ("label", lab.clone())])));
    }
    if let Some((rows, _)) = d.chem.as_ref().map(|c| crate::scene::chem::near(c, e, n, 15.0, 1)) {
        if let Some(r) = rows.first() {
            out.push((crate::text::chem::READOUT, crate::text::fill(crate::text::chem::READOUT_V, &[("name", r.1.clone()), ("ins", r.0.clone()),
                ("sol", r.2.to_string()), ("cao", r.4.map(|x| format!("{x:.1}")).unwrap_or_else(|| "?".into()))])));
        }
    }
    if let Some((xe, xn)) = exit {
        out.push((t::EXIT, format!("{:.0} m", ((xe - e).powi(2) + (xn - n).powi(2)).sqrt())));
    }
    out
}

const ID: &str = "inspect";

pub fn create_element() {
    #[cfg(target_arch = "wasm32")]
    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
        if doc.get_element_by_id(ID).is_none() {
            if let (Ok(el), Some(body)) = (doc.create_element("div"), doc.body()) {
                el.set_id(ID);
                let _ = el.set_attribute("style", "display:none");
                let _ = el.set_attribute("role", "status");
                let _ = body.append_child(&el);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn update(
    windows: Query<&Window>,
    cams: Query<(&Camera, &GlobalTransform), With<Orbit>>,
    mode: Res<Mode>,
    ground: Option<Res<Ground>>,
    slope: Option<Res<SlopeMap>>,
    stops: Option<Res<Stops>>,
    data: Option<Res<InspectData>>,
    hover: Option<Res<crate::scene::select::Hover>>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut last: Local<String>,
    mut last_in: Local<u64>,
) {
    // the readout depends only on the pointer, the camera, the mode, the hover and the buttons: skip the terrain ray march and
    // the HTML when none of them changed since the last frame (the usual idle case)
    {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        if let Ok(win) = windows.single() {
            win.cursor_position().map(|c| (c.x.to_bits(), c.y.to_bits())).hash(&mut h);
            (win.width().to_bits(), win.height().to_bits()).hash(&mut h);
        }
        if let Ok((_, ct)) = cams.single() {
            let (t, r) = (ct.translation(), ct.rotation());
            [t.x, t.y, t.z, r.x, r.y, r.z, r.w].map(f32::to_bits).hash(&mut h);
        }
        (*mode == Mode::Walk, *mode == Mode::Plan).hash(&mut h);
        hover.as_ref().map(|x| x.label.as_str()).hash(&mut h);
        (buttons.pressed(MouseButton::Left), buttons.pressed(MouseButton::Right),
         keys.pressed(KeyCode::AltLeft) || keys.pressed(KeyCode::AltRight)).hash(&mut h);
        (ground.is_some(), data.is_some(), stops.as_ref().map(|s| s.0.len())).hash(&mut h);
        let k = h.finish();
        if *last_in == k && !last.is_empty() {
            return;
        }
        *last_in = k;
    }
    let mut css = "display:none".to_string();
    let mut html = String::new();
    let walk = *mode == Mode::Walk;
    if let (true, Ok(win), Ok((cam, ct)), Some(g), Some(sl), Some(d)) = (*mode == Mode::Plan || walk, windows.single(), cams.single(), ground, slope, data) {
        if let Some(c) = win.cursor_position() {
            use crate::scene::mapview::{BAR_PX, RAIL_L_PX, RAIL_R_PX, TIMELINE_PX};
            // Walk has no side panels: the whole view between the bar and the timeline (not while dragging to look around)
            let inside = if walk { c.y > BAR_PX && c.y < win.height() - TIMELINE_PX && !buttons.pressed(MouseButton::Left) && !buttons.pressed(MouseButton::Right)
                                   && (keys.pressed(KeyCode::AltLeft) || keys.pressed(KeyCode::AltRight)) }
                         else { c.x > RAIL_L_PX && c.x < win.width() - RAIL_R_PX && c.y > BAR_PX && c.y < win.height() - TIMELINE_PX };
            if let (true, Ok(ray)) = (inside, cam.viewport_to_world(ct, c)) {
                if let Some(t) = g.0.ray_hit(ray.origin, *ray.direction, 1.0e5) {
                    let p = ray.origin + *ray.direction * t;
                    let (e, n) = (p.x, -p.z);
                    if let Some(h) = hover.as_ref().filter(|h| !h.label.is_empty()) {
                        let esc = h.label.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
                        html.push_str(&format!("<div class=\"hv\"><b>{esc}</b><span>{}</span></div>", crate::text::drill::HOVER_CLICK));
                    }
                    let exit = stops.as_ref().and_then(|s| s.0.first().map(|x| (x.e, x.n)));
                    for (k, v) in lines(&d, &g.0, &sl.0, exit, e, n) {
                        html.push_str(&format!("<div><span>{k}</span><b>{v}</b></div>"));
                    }
                    let left = if c.x + 280.0 > win.width() - RAIL_R_PX { c.x - 270.0 } else { c.x + 16.0 };
                    css = format!("left:{}px;top:{}px", left.round() as i32, (c.y + 16.0).round() as i32);
                }
            }
        }
    }
    let key = format!("{css}|{html}");
    if *last != key {
        crate::ui::chrome::style(ID, &css);
        #[cfg(target_arch = "wasm32")]
        if let Some(el) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.get_element_by_id(ID)) {
            el.set_inner_html(&html);
        }
        *last = key;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readout_lists_position_elevation_slope_rover_and_exit() {
        let m: serde_json::Value = serde_json::from_slice(&std::fs::read("../assets/manifest.json").unwrap()).unwrap();
        let g = crate::scene::terrain::Grid::from_bytes(&std::fs::read("../assets/heightmap.bin").unwrap(), 553, 1078, 1.0).unwrap();
        let s = crate::scene::terrain::Grid::from_bytes(&std::fs::read("../assets/slope_3m.bin").unwrap(), 553, 1078, 1.0).unwrap();
        let d = InspectData { chem: None, grid: m["grid"].clone(), z_offset: -3668.0, bias: 3.4, rover: vec![(100.0, 100.0, 4600, "agrees".into())],
                              classes: vec![(0, "source unknown".into())], ready: Some(vec![0u8; 553 * 1078]) };
        let l = lines(&d, &g, &s, Some((100.0, 110.0)), 100.0, 100.0);
        let keys: Vec<_> = l.iter().map(|x| x.0).collect();
        assert_eq!(keys.len(), 6, "{l:?}");
        assert!(l[1].1.ends_with(" m") && l[1].1.starts_with('-'), "elevation below datum: {}", l[1].1);
        assert!(l.iter().any(|x| x.1.contains("sol 4600")));
        assert_eq!(l.last().unwrap().1, "10 m");
    }
}
