//! Walk mode: first-person view at eye height on the terrain. WASD/arrows walk at the route's assumed pace (route.json
//! assumptions: flat speed, Tobler-shaped slope penalty, measured DEM bias), so walking here takes the time the route says.
//! Shift = FAST_FACTOR (labelled as not a walking speed), left-drag looks around, N faces the next stop, Tab switches Walk / Plan.
//! Map axes: e = metres east, n = metres north; world x = e, z = -n. Headings are compass degrees (0 north, 90 east).
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;

use leafwing_input_manager::prelude::ActionState;

use crate::scene::controls::{mode_button_clicked, tapped, Act};
use crate::scene::terrain::Grid;

/// Eye height above the ground (the Client build spec's Walk camera).
pub const EYE_M: f32 = 1.7;
/// Shift multiplier, for getting around quickly; shown on screen as "not a walking speed".
pub const FAST_FACTOR: f32 = 5.0;
/// Distance at which a stop counts as reached.
pub const ARRIVE_M: f32 = 3.0;

#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Plan,
    Walk,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stop {
    pub id: String,
    pub name: String,
    pub e: f32,
    pub n: f32,
}

#[derive(Resource, Default)]
pub struct Stops(pub Vec<Stop>);

/// Terrain heights shared with the walk systems.
#[derive(Resource)]
pub struct Ground(pub Grid);

/// Map slope at 3 m scale (slope_3m.bin, degrees; same grid layout as the heightmap), the slope the route was planned on.
#[derive(Resource)]
pub struct SlopeMap(pub Grid);

/// Walking pace from route.json "assumptions": speed = flat x exp(-k tan(max(slope - bias, 0))), as in tools/route.py.
/// All three numbers come from the bundle; none is typed in the client. ASSUMPTION (Tobler shape, Earth walking, relative only).
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct Pace {
    pub flat_mps: f32,
    pub k: f32,
    pub bias_deg: f32,
}

impl Pace {
    pub fn from_route(route: &serde_json::Value) -> Option<Pace> {
        let a = &route["assumptions"];
        Some(Pace {
            flat_mps: a["speed_flat_mps"].as_f64()? as f32,
            k: a["tobler_k"].as_f64()? as f32,
            bias_deg: a["bias_deg"].as_f64()? as f32,
        })
    }

    /// Walking speed (m/s) on ground whose map slope is `slope_deg`.
    pub fn speed(&self, slope_deg: f32) -> f32 {
        let corrected = (slope_deg - self.bias_deg).max(0.0);
        self.flat_mps * (-self.k * corrected.to_radians().tan()).exp()
    }
}

#[derive(Resource, Debug, Clone)]
pub struct Walker {
    pub e: f32,
    pub n: f32,
    /// compass heading of the view, radians (0 = north, pi/2 = east)
    pub heading: f32,
    /// look up (+) or down (-), radians
    pub pitch: f32,
    /// index of the next stop to reach, in loop order (wraps back to the exit)
    pub next: usize,
}

/// Compass bearing in degrees [0, 360) from (e0, n0) to (e1, n1).
pub fn bearing_deg(e0: f32, n0: f32, e1: f32, n1: f32) -> f32 {
    let b = (e1 - e0).atan2(n1 - n0).to_degrees();
    if b < 0.0 { b + 360.0 } else { b }
}

/// World eye position and the view's forward direction for a walker on the ground.
pub fn eye(w: &Walker, g: &Grid) -> (Vec3, Vec3) {
    let p = Vec3::new(w.e, g.height_at(w.e, w.n) + EYE_M, -w.n);
    let (sh, ch) = w.heading.sin_cos();
    let (sp, cp) = w.pitch.sin_cos();
    (p, Vec3::new(sh * cp, sp, -ch * cp))
}

/// Move by `forward` and `right` metres relative to the heading, staying inside the grid (1 m margin).
pub fn step(w: &mut Walker, g: &Grid, forward: f32, right: f32) {
    let (sh, ch) = w.heading.sin_cos();
    let de = forward * sh + right * ch;
    let dn = forward * ch - right * sh;
    let (we, wn) = (g.w as f32 * g.cell, g.h as f32 * g.cell);
    w.e = (w.e + de).clamp(1.0, we - 1.0);
    w.n = (w.n + dn).clamp(1.0, wn - 1.0);
}

/// Advance `next` when the walker is within ARRIVE_M of it. Returns true when a stop was just reached.
pub fn check_arrival(w: &mut Walker, stops: &[Stop]) -> bool {
    if stops.is_empty() {
        return false;
    }
    let s = &stops[w.next % stops.len()];
    if ((s.e - w.e).powi(2) + (s.n - w.n).powi(2)).sqrt() <= ARRIVE_M {
        w.next = (w.next + 1) % stops.len();
        return true;
    }
    false
}

/// Tab or M (controls::input_map), or a click on the on-screen mode button.
pub fn switch_mode(keys: Res<ButtonInput<KeyCode>>, mut mode: ResMut<Mode>) {
    let pressed = tapped(&keys, Act::SwitchMode);
    let clicked = mode_button_clicked();
    if pressed || clicked {
        *mode = if *mode == Mode::Plan { Mode::Walk } else { Mode::Plan };
    }
}

/// Starting mode: `?mode=walk` in the page URL opens in Walk mode (used by the headless smoke test and for sharing links).
pub fn initial_mode() -> Mode {
    #[cfg(target_arch = "wasm32")]
    if let Some(q) = web_sys::window().and_then(|w| w.location().search().ok()) {
        if q.contains("mode=walk") {
            return Mode::Walk;
        }
    }
    Mode::Plan
}

/// Haze distances for Walk mode. A VISUAL choice so the crop edge reads as distance, not a measured Martian haze.
pub const WALK_FOG_M: (f32, f32) = (250.0, 1400.0);

/// Plan-mode haze, set when the scene is framed.
#[derive(Resource, Clone, Copy)]
pub struct PlanFog(pub f32, pub f32);

/// Swap the haze distances when the mode changes (the Plan values would put the haze about 1 km away from a walker).
pub fn mode_fog(mode: Res<Mode>, plan: Option<Res<PlanFog>>, mut fogs: Query<&mut bevy::pbr::DistanceFog>) {
    let Some(plan) = plan else { return };
    if !mode.is_changed() && !plan.is_changed() {
        return;
    }
    let (start, end) = if *mode == Mode::Walk { WALK_FOG_M } else { (plan.0, plan.1) };
    for mut f in &mut fogs {
        f.falloff = bevy::pbr::FogFalloff::Linear { start, end };
    }
}

/// True while the load-time shader warm-up runs: Walk-only visuals (lit terrain, shadows, sky, AR overlays, suits) are shown
/// then so their GPU programs compile behind the loading screen, not on the first switch to Walk (a 0.3 s stall after the
/// heavy variants were replaced, measured).
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq)]
pub struct WalkWarm(pub bool);

pub fn in_walk(mode: Res<Mode>) -> bool {
    *mode == Mode::Walk
}


#[allow(clippy::too_many_arguments)]
/// Click-to-walk target in Walk (east, north) and the steepest corrected slope on the straight way there (deg): set by
/// `walk_click`, walked by `walk_controls`, drawn by select::shade, cleared on arrival or by a movement key.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq)]
pub struct WalkTarget(pub Option<(f32, f32, f32)>);

/// Steepest corrected slope (deg) along the straight line from a to b, sampled every half metre.
pub fn steepest_on_line(slope: &Grid, bias: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
    let n = (len / 0.5).ceil().max(1.0) as usize;
    (0..=n).map(|k| {
        let f = k as f32 / n as f32;
        (slope.height_at(a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f) - bias).max(0.0)
    }).fold(0.0f32, f32::max)
}

/// Walk: a click on the ground (press and release without dragging, not on the panels) sets the walk target.
#[allow(clippy::too_many_arguments)]
pub fn walk_click(
    windows: Query<&Window>,
    buttons: Res<ButtonInput<MouseButton>>,
    mode: Res<Mode>,
    cams: Query<(&Camera, &GlobalTransform), With<crate::scene::camera::Orbit>>,
    ground: Option<Res<Ground>>,
    slope: Option<Res<SlopeMap>>,
    pace: Option<Res<Pace>>,
    walker: Option<Res<Walker>>,
    mut target: ResMut<WalkTarget>,
    mut press: Local<Option<Vec2>>,
) {
    let (Ok(win), Ok((cam, ct)), Some(g), Some(sl), Some(pace), Some(w)) = (windows.single(), cams.single(), ground, slope, pace, walker) else { return };
    if *mode != Mode::Walk {
        *press = None;
        return;
    }
    let cursor = win.cursor_position();
    if buttons.just_pressed(MouseButton::Left) {
        *press = cursor;
    }
    if !buttons.just_released(MouseButton::Left) {
        return;
    }
    let (Some(p0), Some(p1)) = (press.take(), cursor) else { return };
    let (bar, tl) = (crate::scene::mapview::BAR_PX, crate::scene::mapview::TIMELINE_PX);
    if p0.distance(p1) >= 4.0 || p1.y < bar || p1.y > win.height() - tl {
        return;
    }
    let Ok(ray) = cam.viewport_to_world(ct, p1) else { return };
    let Some(t) = g.0.ray_hit(ray.origin, *ray.direction, 2000.0) else { return };
    let p = ray.origin + *ray.direction * t;
    let (e, n) = (p.x, -p.z);
    target.0 = Some((e, n, steepest_on_line(&sl.0, pace.bias_deg, (w.e, w.n), (e, n))));
}

pub fn walk_controls(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    acts: Query<&ActionState<Act>>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    ground: Option<Res<Ground>>,
    slope: Option<Res<SlopeMap>>,
    pace: Option<Res<Pace>>,
    stops: Res<Stops>,
    walker: Option<ResMut<Walker>>,
    mut cams: Query<&mut Transform, With<Camera3d>>,
    view: Option<Res<crate::scene::astronaut::View>>,
    mut target: ResMut<WalkTarget>,
) {
    let (Some(ground), Some(mut w), Some(pace), Ok(act)) = (ground, walker, pace, acts.single()) else { return };
    let g = &ground.0;
    if buttons.pressed(MouseButton::Left) {
        w.heading += motion.delta.x * 0.004;
        w.pitch = (w.pitch - motion.delta.y * 0.004).clamp(-80f32.to_radians(), 80f32.to_radians());
    }
    let mut f = 0.0f32;
    let mut r = 0.0f32;
    if act.pressed(&Act::Forward) { f += 1.0; }
    if act.pressed(&Act::Back) { f -= 1.0; }
    if act.pressed(&Act::Right) { r += 1.0; }
    if act.pressed(&Act::Left) { r -= 1.0; }
    // click-to-walk: head for the target at the planned pace; a movement key takes over (and clears it)
    if f != 0.0 || r != 0.0 {
        target.0 = None;
    } else if let Some((te, tn, _)) = target.0 {
        let d = ((te - w.e).powi(2) + (tn - w.n).powi(2)).sqrt();
        if d < 0.4 {
            target.0 = None;
        } else {
            w.heading = bearing_deg(w.e, w.n, te, tn).to_radians();
            let slope_deg = slope.as_ref().map(|s| s.0.height_at(w.e, w.n)).unwrap_or(0.0);
            let fast = act.pressed(&Act::Fast);
            let v = (pace.speed(slope_deg) * if fast { FAST_FACTOR } else { 1.0 } * time.delta_secs()).min(d);
            step(&mut w, g, v, 0.0);
        }
    }
    if f != 0.0 || r != 0.0 {
        let fast = act.pressed(&Act::Fast);
        let slope_deg = slope.as_ref().map(|s| s.0.height_at(w.e, w.n)).unwrap_or(0.0);
        let v = pace.speed(slope_deg) * if fast { FAST_FACTOR } else { 1.0 } * time.delta_secs();
        let norm = (f * f + r * r).sqrt();
        step(&mut w, g, f / norm * v, r / norm * v);
    }
    if tapped(&keys, Act::FaceNext) && !stops.0.is_empty() {
        let s = &stops.0[w.next % stops.0.len()];
        w.heading = bearing_deg(w.e, w.n, s.e, s.n).to_radians();
        w.pitch = 0.0;
    }
    if check_arrival(&mut w, &stops.0) {
        info!("reached a stop; next is {}", stops.0[w.next].name);
    }
    if view.as_deref().is_some_and(|v| *v == crate::scene::astronaut::View::Third) {
        return; // astronaut.rs places the chase camera
    }
    let (p, fwd) = eye(&w, g);
    for mut t in &mut cams {
        *t = Transform::from_translation(p).looking_to(fwd, Vec3::Y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(w: usize, h: usize, z: f32) -> Grid {
        let mut b = Vec::new();
        for _ in 0..w * h {
            b.extend_from_slice(&z.to_le_bytes());
        }
        Grid::from_bytes(&b, w, h, 1.0).unwrap()
    }

    #[test]
    fn steepest_on_a_line_reads_the_slope_grid() {
        let mut b = Vec::new();
        for i in 0..100 {
            let v: f32 = if i % 10 == 5 { 30.0 } else { 5.0 };
            b.extend_from_slice(&v.to_le_bytes());
        }
        let g = Grid::from_bytes(&b, 10, 10, 1.0).unwrap();
        assert!((steepest_on_line(&g, 3.4, (0.5, 5.0), (9.5, 5.0)) - 26.6).abs() < 0.01, "crosses the 30 deg column");
        assert!(steepest_on_line(&g, 3.4, (0.5, 0.5), (3.5, 0.5)) < 2.0);
    }

    #[test]
    fn pace_matches_the_route_formula() {
        let p = Pace { flat_mps: 0.8, k: 3.5, bias_deg: 3.4 };
        assert_eq!(p.speed(0.0), 0.8, "below the bias the corrected slope is 0: flat speed");
        assert_eq!(p.speed(3.4), 0.8);
        let want = 0.8 * (-3.5 * (20f32).to_radians().tan()).exp(); // tools/route.py at corrected 20 deg
        assert!((p.speed(23.4) - want).abs() < 1e-6);
        assert!(p.speed(30.0) < p.speed(15.0), "steeper is slower");
    }

    /// Acceptance test 25: bearing to the next flag within 1 degree.
    #[test]
    fn bearing_is_compass_correct_within_one_degree() {
        let cases = [((0.0, 0.0), (0.0, 10.0), 0.0), ((0.0, 0.0), (10.0, 0.0), 90.0), ((0.0, 0.0), (0.0, -10.0), 180.0),
                     ((0.0, 0.0), (-10.0, 0.0), 270.0), ((5.0, 5.0), (15.0, 15.0), 45.0), ((0.0, 0.0), (-3.0, 3.0 * 3f32.sqrt()), 330.0)];
        for ((e0, n0), (e1, n1), want) in cases {
            let got = bearing_deg(e0, n0, e1, n1);
            let diff = ((got - want + 540.0) % 360.0 - 180.0).abs();
            assert!(diff < 1.0, "bearing to ({e1},{n1}): got {got}, want {want}");
        }
    }

    #[test]
    fn eye_is_exactly_eye_height_above_the_ground_and_looks_along_heading() {
        let g = flat(20, 20, 7.0);
        let w = Walker { e: 10.0, n: 10.0, heading: 90f32.to_radians(), pitch: 0.0, next: 0 };
        let (p, f) = eye(&w, &g);
        assert!((p.y - (7.0 + EYE_M)).abs() < 1e-5);
        assert!(f.x > 0.99 && f.z.abs() < 1e-5, "heading 90 must look east: {f:?}");
    }

    #[test]
    fn walking_follows_heading_and_stays_on_the_map() {
        let g = flat(20, 20, 0.0);
        let mut w = Walker { e: 10.0, n: 10.0, heading: 0.0, pitch: 0.0, next: 0 };
        step(&mut w, &g, 2.0, 0.0);
        assert!((w.n - 12.0).abs() < 1e-5 && (w.e - 10.0).abs() < 1e-5, "forward at heading 0 goes north");
        step(&mut w, &g, 0.0, 3.0);
        assert!((w.e - 13.0).abs() < 1e-5, "right at heading 0 goes east");
        step(&mut w, &g, 1000.0, 0.0);
        assert!(w.n <= 19.0, "clamped inside the map");
    }

    #[test]
    fn arrival_advances_to_the_next_stop_and_wraps() {
        let stops = vec![
            Stop { id: "a".into(), name: "A".into(), e: 0.0, n: 0.0 },
            Stop { id: "b".into(), name: "B".into(), e: 50.0, n: 0.0 },
        ];
        let mut w = Walker { e: 40.0, n: 0.0, heading: 0.0, pitch: 0.0, next: 1 };
        assert!(!check_arrival(&mut w, &stops));
        w.e = 48.0;
        assert!(check_arrival(&mut w, &stops) && w.next == 0, "within 3 m of B, then back to A");
    }
}
