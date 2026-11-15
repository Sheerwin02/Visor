//! Walk-mode top bar (HTML #hud, index.html): next stop, distance and walking time along the planned route, direction arrow,
//! slope under your feet (map slope minus the measured bias), walk-back time to the exit from where you stand, and a note on
//! rover-verified ground. Every number comes from the bundle; every word from ui_text.json "hud". H hides the bar.
//! HUD rules (after a lunar HUD study, arXiv 2402.15692): top of the view, lower third clear, no meaning by colour alone, advise, not command.
use bevy::prelude::*;

use crate::scene::controls::{tapped, Act};
use crate::ui::model::{TopBarModel, UiBridge};
use crate::ui::chrome::{fill, text};
use crate::scene::terrain::Grid;
use crate::scene::walk::{bearing_deg, Mode, Pace, SlopeMap, Stops, Walker};

/// The planned loop (route.json): polyline in (e, n) metres and, per leg, its first and last index in the polyline.
#[derive(Resource, Default, Clone)]
pub struct RouteData {
    pub poly: Vec<(f32, f32)>,
    /// (poly_start, poly_end) for leg i = stop i to stop i + 1 (the last leg returns to the exit)
    pub legs: Vec<(usize, usize)>,
}

/// Time back to the exit from every cell (s), from the live plan (planner.rs); infinity = no way back under the rules.
#[derive(Resource)]
pub struct WalkBack {
    pub w: usize,
    pub h: usize,
    pub cell: f32,
    pub s: Vec<f32>,
}

impl WalkBack {
    /// Seconds back to the exit from map position (e, n); None = no way back under the rules (or off the map).
    pub fn at(&self, e: f32, n: f32) -> Option<f32> {
        let c = (e / self.cell).floor();
        let r = (self.h as f32 - n / self.cell).floor();
        if c < 0.0 || r < 0.0 || c >= self.w as f32 || r >= self.h as f32 {
            return None;
        }
        let v = self.s[r as usize * self.w + c as usize];
        v.is_finite().then_some(v)
    }
}

/// Localized rover positions (e, n) and the radius counted as rover-verified ground (route.json assumptions).
#[derive(Resource, Default)]
pub struct RoverGround {
    pub pts: Vec<(f32, f32)>,
    pub radius_m: f32,
}

#[derive(Resource, Default)]
pub struct HudState {
    pub hidden: bool,
    /// stop name and the time (s) it was reached, for a short notice
    pub reached: Option<(String, f32)>,
    /// the stop being walked to, to notice an arrival
    pub last_next: Option<usize>,
}

const REACHED_SHOW_S: f32 = 5.0;

/// Time (s) to walk from polyline point i to i + 1, exactly as tools/route.py costs a graph edge:
/// step length x the mean of 1/speed at the two cell centres.
pub fn edge_time(a: (f32, f32), b: (f32, f32), slope: &Grid, pace: &Pace) -> f32 {
    let step = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
    let inv = |p: (f32, f32)| 1.0 / pace.speed(slope.height_at(p.0, p.1));
    step * 0.5 * (inv(a) + inv(b))
}

/// Distance (m) and time (s) from the walker at (e, n) to the end of leg `leg` along the route: to the nearest route point
/// of that leg in a straight line, then along the route. Also returns how far (m) the walker is from that route point.
pub fn remaining(route: &RouteData, leg: usize, e: f32, n: f32, slope: &Grid, pace: &Pace) -> Option<(f32, f32, f32)> {
    let &(s, t) = route.legs.get(leg)?;
    let pts = route.poly.get(s..=t)?;
    let (k, off) = pts
        .iter()
        .enumerate()
        .map(|(i, p)| (i, ((p.0 - e).powi(2) + (p.1 - n).powi(2)).sqrt()))
        .min_by(|a, b| a.1.total_cmp(&b.1))?;
    let mut dist = off;
    let mut time = off / pace.speed(slope.height_at(e, n));
    for w in pts[k..].windows(2) {
        dist += ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt();
        time += edge_time(w[0], w[1], slope, pace);
    }
    Some((dist, time, off))
}

/// Arrow rotation on screen (degrees, clockwise, 0 = straight ahead) for a target at compass `bearing` when facing `heading`.
pub fn arrow_deg(bearing: f32, heading: f32) -> f32 {
    let d = (bearing - heading).rem_euclid(360.0);
    if d > 180.0 { d - 360.0 } else { d }
}

/// Minutes for display: "0" only at zero, one decimal under 10 min (never shown as 0.0), whole minutes above.
pub fn minutes(s: f32) -> String {
    let m = s / 60.0;
    if s <= 0.0 {
        "0".to_string()
    } else if m < 10.0 {
        format!("{:.1}", m.max(0.1))
    } else {
        format!("{}", m.round() as i32)
    }
}

pub fn toggle_hud(keys: Res<ButtonInput<KeyCode>>, mode: Res<Mode>, mut st: ResMut<HudState>) {
    if *mode == Mode::Walk && tapped(&keys, Act::HideHud) {
        st.hidden = !st.hidden;
        text("hud-hidden", if st.hidden { crate::text::hud::HIDDEN } else { "" });
    }
    if mode.is_changed() && *mode != Mode::Walk {
        text("hud-hidden", "");
    }
}

/// Compass strip geometry (index.html #compass): CSS px per degree and the visible half-width in degrees.
pub const COMPASS_PX_PER_DEG: f32 = 3.0;
pub const COMPASS_HALF_DEG: f32 = 60.0;

/// Strip offset (CSS px) that puts compass heading `heading_deg` under the centre mark. The strip covers -180..540 deg,
/// so any heading has a full window either side.
pub fn compass_offset(heading_deg: f32) -> f32 {
    let h = heading_deg.rem_euclid(360.0);
    COMPASS_HALF_DEG * COMPASS_PX_PER_DEG - (h + 180.0) * COMPASS_PX_PER_DEG
}

/// Where the next-stop marker sits on the compass: (left in CSS px from the window's left edge, off-screen side).
/// side: 0 = in view, -1 = off to the left, +1 = off to the right (marker pinned to that edge).
pub fn compass_marker(rel_deg: f32) -> (f32, i8) {
    let w = 2.0 * COMPASS_HALF_DEG * COMPASS_PX_PER_DEG;
    if rel_deg < -COMPASS_HALF_DEG {
        (0.0, -1)
    } else if rel_deg > COMPASS_HALF_DEG {
        (w, 1)
    } else {
        (w / 2.0 + rel_deg * COMPASS_PX_PER_DEG, 0)
    }
}

#[allow(clippy::too_many_arguments)]
/// Position as fractions of the patch for the minimap (x east 0..1, y from the north edge 0..1).
pub fn map_frac(g: &crate::scene::terrain::Grid, e: f32, n: f32) -> (f32, f32) {
    let (we, hn) = (g.w as f32 * g.cell, g.h as f32 * g.cell);
    ((e / we).clamp(0.0, 1.0), (1.0 - n / hn).clamp(0.0, 1.0))
}

/// How close (m) the walker must be to a rover-checked PLACES position for the HUD to show Curiosity's measured tilt instead of
/// the map's slope: the Walk arrival distance, so it shows at every station. The tilt is the rover's whole 3.0 x 2.8 m
/// footprint, so it describes the spot where it parked, which the tooltip names with the distance.
pub const ROVER_TILT_R_M: f32 = 3.0;

/// The rover-checked stop nearest to (e, n) within ROVER_TILT_R_M, and its distance (m).
pub fn nearest_tilt(items: &[crate::scene::checks::Check], e: f32, n: f32) -> Option<(&crate::scene::checks::Check, f32)> {
    items.iter().map(|c| (c, ((c.e - e).powi(2) + (c.n - n).powi(2)).sqrt()))
        .filter(|(_, d)| *d <= ROVER_TILT_R_M).min_by(|a, b| a.1.total_cmp(&b.1))
}

/// Binoculars (Walk): while the right mouse button is held the view narrows BINOC_ZOOM times and the HUD shows the range
/// to the ground at the centre of the view.
#[derive(Resource, Default)]
pub struct Binoculars {
    pub range_m: Option<f32>,
}

/// Binocular zoom factor (field of view divided by this). VISUAL.
pub const BINOC_ZOOM: f32 = 5.0;

pub fn binoculars(
    mode: Res<crate::scene::walk::Mode>,
    buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    persp: Option<Res<crate::scene::mapview::PerspectiveSettings>>,
    ground: Option<Res<crate::scene::walk::Ground>>,
    mut cams: Query<(&Camera, &GlobalTransform, &mut Projection), With<Camera3d>>,
    mut b: ResMut<Binoculars>,
) {
    let Some(persp) = persp else { return };
    let on = *mode == crate::scene::walk::Mode::Walk && buttons.pressed(MouseButton::Right);
    for (cam, ct, mut p) in &mut cams {
        if let Projection::Perspective(pp) = &mut *p {
            let want = if on { persp.0.fov / BINOC_ZOOM } else { persp.0.fov };
            if (pp.fov - want).abs() > 1e-4 {
                pp.fov = want;
            }
            b.range_m = if on {
                windows.single().ok().and_then(|w| cam.viewport_to_world(ct, Vec2::new(w.width() / 2.0, w.height() / 2.0)).ok())
                    .zip(ground.as_ref()).and_then(|(ray, g)| g.0.ray_hit(ray.origin, *ray.direction, 5000.0))
            } else { None };
        }
    }
}

pub fn update_hud(
    time: Res<Time>,
    mode: Res<Mode>,
    walker: Option<Res<Walker>>,
    stops: Res<Stops>,
    route: Option<Res<RouteData>>,
    slope: Option<Res<SlopeMap>>,
    pace: Option<Res<Pace>>,
    back: Option<Res<WalkBack>>,
    rover: Option<Res<RoverGround>>,
    checks: Option<Res<crate::scene::checks::Checks>>,
    mut st: ResMut<HudState>,
    mut bridge: ResMut<UiBridge>,
    view: Option<Res<crate::scene::astronaut::View>>,
    ground: Option<Res<crate::scene::walk::Ground>>,
    (target, binoc, limit): (Option<Res<crate::scene::walk::WalkTarget>>, Option<Res<Binoculars>>, Option<Res<crate::scene::trust::Overlays>>),
) {
    let visible = *mode == Mode::Walk && !st.hidden;
    let (Some(w), Some(route), Some(slope), Some(pace), Some(back), Some(rover)) = (walker, route, slope, pace, back, rover) else {
        return;
    };
    if stops.0.is_empty() || *mode != Mode::Walk {
        // already hidden (every Plan frame): nothing to copy or compare
        if bridge.top.as_ref().is_some_and(|t| !t.visible) {
            return;
        }
        let hidden = TopBarModel { visible: false, ..bridge.top.clone().unwrap_or_default() };
        bridge.set_top(hidden);
        return;
    }
    let now = time.elapsed_secs();
    let n_stops = stops.0.len();
    let k = w.next % n_stops;
    let next = &stops.0[k];
    // walker.next is the stop being walked to; leg (next - 1) ends there (leg n-1 returns to the exit, index 0)
    let leg = (w.next + n_stops - 1) % n_stops;
    if st.last_next.is_some_and(|prev| prev != k) {
        st.reached = Some((crate::scene::labels::label_text(leg, &stops.0[leg].name), now));
    }
    st.last_next = Some(k);
    let (dist, time_s) = remaining(&route, leg, w.e, w.n, &slope.0, &pace).map(|(d, t, _)| (d, t)).unwrap_or((f32::NAN, f32::NAN));
    // slope under your feet: map slope at 3 m scale, corrected by the bias in use (floored at 0)
    let map_slope = slope.0.height_at(w.e, w.n);
    let corrected = (map_slope - pace.bias_deg).max(0.0);
    // compass: strip under the centre mark, marker at the next stop's bearing (pinned to an edge when out of view)
    let heading = w.heading.to_degrees();
    let b = bearing_deg(w.e, w.n, next.e, next.n);
    let rel = arrow_deg(b, heading);
    let (left, side) = compass_marker(rel);
    let rover_tilt = checks.as_ref().and_then(|c| nearest_tilt(&c.items, w.e, w.n))
        .map(|(c, d)| (format!("{:.0}", c.rover_tilt), format!("{d:.0}"), c.sol.to_string()));
    let on_verified = rover.pts.iter().any(|p| (p.0 - w.e).powi(2) + (p.1 - w.n).powi(2) <= rover.radius_m * rover.radius_m);
    let note = match &st.reached {
        Some((name, t0)) if now - t0 < REACHED_SHOW_S => fill(crate::text::hud::REACHED, &[("stop_name", name.clone())]),
        // rover-verified ground is a badge on the slope tile now (a full sentence on screen all the time was too much)
        _ => String::new(),
    };
    bridge.set_top(TopBarModel {
        visible,
        next_label: crate::scene::labels::label_text(k, &next.name),
        stop_k: k + 1,
        stop_n: n_stops,
        dist: if dist.is_finite() { format!("{}", dist.round() as i32) } else { "\u{2014}".into() },
        time: if time_s.is_finite() { minutes(time_s) } else { "\u{2014}".into() },
        slope: format!("{}", corrected.round() as i32),
        slope_map: format!("{}", map_slope.round() as i32),
        rover_tilt,
        verified: on_verified,
        aim: match (binoc.as_ref().and_then(|b| b.range_m), target.as_ref().and_then(|t| t.0)) {
            (Some(r), _) => fill(crate::text::hud::BINOC_RANGE, &[("x", format!("{}", r.round() as i64))]),
            (None, Some((te, tn, s))) => fill(crate::text::hud::GOTO, &[("x", format!("{}", ((te - w.e).powi(2) + (tn - w.n).powi(2)).sqrt().round() as i64)),
                                                                   ("a", format!("{}", s.round() as i64))]),
            _ => String::new(),
        },
        aim_warn: binoc.as_ref().and_then(|b| b.range_m).is_none()
            && target.as_ref().and_then(|t| t.0).zip(limit.as_ref()).is_some_and(|(t, l)| t.2 > l.limit),
        comms: match (ground.as_ref(), stops.0.first()) {
            (Some(g), Some(x)) if crate::ui::panel::sight_line(&g.0, x.e, x.n, w.e, w.n) => 2,
            (Some(g), _) if crate::scene::select::RELAY_AT.lock().ok().and_then(|r| *r)
                .is_some_and(|(re, rn)| crate::ui::panel::sight_line(&g.0, re, rn, w.e, w.n)) => 1,
            (Some(_), _) => 0,
            _ => 2,
        },
        back: back.at(w.e, w.n).map(minutes),
        strip_px: compass_offset(heading).round() as i32,
        marker_px: left.round() as i32,
        marker_side: side,
        bearing: b.round() as i32 % 360,
        rel: rel.round() as i32,
        note,
        speed: format!("{}", pace.flat_mps),
        bias: format!("{}", pace.bias_deg),
        map_me: map_frac(&slope.0, w.e, w.n),
        map_next: map_frac(&slope.0, next.e, next.n),
        heading: heading.round() as i32,
        third: view.is_some_and(|v| *v == crate::scene::astronaut::View::Third),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rover_tilt_replaces_the_map_slope_only_near_a_rover_check() {
        use crate::scene::checks::Check;
        let c = |e: f32, n: f32, t: f32| Check { e, n, label: "disagrees".into(), sol: 4687, rover_tilt: t, dem_tilt: 22.8, pose: None };
        let items = [c(0.0, 0.0, 6.8), c(10.0, 0.0, 11.1)];
        assert_eq!(nearest_tilt(&items, 1.0, 1.0).map(|(x, _)| x.rover_tilt), Some(6.8));
        assert_eq!(nearest_tilt(&items, 9.0, 0.0).map(|(x, d)| (x.rover_tilt, d)), Some((11.1, 1.0)));
        assert!(nearest_tilt(&items, 5.0, 0.0).is_none(), "5 m from both: the map slope stays");
        // the real bundle: standing on the Valle de la Luna drill site gives Curiosity's 6.8 deg, not the map's
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read("../assets/rover_check.json").unwrap()).unwrap();
        let s: serde_json::Value = serde_json::from_slice(&std::fs::read("../assets/stops.json").unwrap()).unwrap();
        let valle = s["stops"].as_array().unwrap().iter().find(|x| x["id"] == "valle_de_la_luna").unwrap();
        let all = crate::scene::checks::parse(&v);
        let (x, _) = nearest_tilt(&all, valle["e_m"].as_f64().unwrap() as f32, valle["n_m"].as_f64().unwrap() as f32).unwrap();
        assert!((x.rover_tilt - 6.807).abs() < 0.01 && x.sol == 4687, "{x:?}");
    }

    #[test]
    fn arrow_points_relative_to_the_view() {
        assert_eq!(arrow_deg(90.0, 90.0), 0.0, "facing the target: straight ahead");
        assert_eq!(arrow_deg(90.0, 0.0), 90.0, "target east while facing north: right");
        assert_eq!(arrow_deg(0.0, 90.0), -90.0, "target north while facing east: left");
        assert_eq!(arrow_deg(10.0, 350.0), 20.0, "wraps through north");
        assert_eq!(arrow_deg(180.0, 0.0), 180.0, "behind");
    }

    #[test]
    fn compass_centres_the_heading_and_pins_the_marker() {
        // heading h sits under the centre: tick for angle a is at (a + 180) * px, so offset + (h + 180) * px = half-width
        for h in [0.0f32, 90.0, 359.0] {
            let centre = compass_offset(h) + (h + 180.0) * COMPASS_PX_PER_DEG;
            assert!((centre - COMPASS_HALF_DEG * COMPASS_PX_PER_DEG).abs() < 1e-3);
        }
        assert_eq!(compass_offset(360.0), compass_offset(0.0), "wraps");
        let mid = COMPASS_HALF_DEG * COMPASS_PX_PER_DEG;
        assert_eq!(compass_marker(0.0), (mid, 0), "ahead: centre");
        assert_eq!(compass_marker(30.0), (mid + 30.0 * COMPASS_PX_PER_DEG, 0), "right of centre");
        assert_eq!(compass_marker(-170.0).1, -1, "behind on the left: pinned left");
        assert_eq!(compass_marker(100.0), (2.0 * mid, 1), "off to the right: pinned right");
    }

    #[test]
    fn minutes_are_readable() {
        assert_eq!(minutes(59.0), "1.0");
        assert_eq!(minutes(326.0), "5.4");
        assert_eq!(minutes(1413.0), "24");
        assert_eq!(minutes(0.0), "0", "at the exit");
        assert_eq!(minutes(2.0), "0.1", "close to the exit is never shown as 0.0");
    }

    #[test]
    fn walk_back_reads_the_right_cell_and_flags_no_way() {
        // 3 x 2 grid, row 0 north: [[1, 2, 3], [4, 5, NO]]
        let wb = WalkBack { w: 3, h: 2, cell: 1.0, s: vec![1.0, 2.0, 3.0, 4.0, 5.0, f32::INFINITY] };
        assert_eq!(wb.at(0.5, 1.5), Some(1.0), "north-west cell");
        assert_eq!(wb.at(1.5, 0.5), Some(5.0), "south-middle cell");
        assert_eq!(wb.at(2.5, 0.5), None, "no way back");
        assert_eq!(wb.at(-1.0, 0.5), None, "off the map");
    }

    #[test]
    fn remaining_on_flat_ground_is_distance_over_flat_speed() {
        let mut b = Vec::new();
        for _ in 0..30 * 30 {
            b.extend_from_slice(&0.0f32.to_le_bytes());
        }
        let g = Grid::from_bytes(&b, 30, 30, 1.0).unwrap();
        let pace = Pace { flat_mps: 0.8, k: 3.5, bias_deg: 3.4 };
        let route = RouteData { poly: (0..21).map(|i| (5.5 + i as f32, 10.5)).collect(), legs: vec![(0, 20)] };
        let (d, t, off) = remaining(&route, 0, 5.5, 13.5, &g, &pace).unwrap();
        assert!((off - 3.0).abs() < 1e-4 && (d - 23.0).abs() < 1e-3, "3 m to the route, then 20 m along it: {d}");
        assert!((t - 23.0 / 0.8).abs() < 1e-2, "{t}");
    }
}
