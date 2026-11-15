//! Plan mode's default view: a north-up, top-down orthographic MAP (the planner's working view; metres per pixel are the same
//! everywhere, so the scale bar is exact). V switches to the 3D orbit view and back. Left-drag pans, the wheel zooms about
//! the pointer. The left side of the window is taken by the planning panel (index.html #plan-panel), so framing leaves it clear.
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;

use crate::scene::camera::Orbit;
use crate::scene::controls::{tapped, Act};
use crate::scene::walk::{Ground, Mode};

/// The planner's frame around the map, in CSS px (style.css --rail-l, --rail-r, --bar-h, --tl-h; tested equal):
/// layer rail on the left, inspector on the right, mission bar on top, EVA timeline at the bottom.
pub const RAIL_L_PX: f32 = 232.0;
pub const RAIL_R_PX: f32 = 360.0;
pub const BAR_PX: f32 = 56.0;
pub const TIMELINE_PX: f32 = 176.0;
const MARGIN_PX: f32 = 32.0;
/// Extra room above the loop for stop labels (they sit above their flags).
const LABELS_PX: f32 = 36.0;

/// Is a window point inside the free map area (not under the bar, rails or timeline)?
pub fn in_map_area(p: Vec2, win: Vec2) -> bool {
    p.x > RAIL_L_PX && p.x < win.x - RAIL_R_PX && p.y > BAR_PX && p.y < win.y - TIMELINE_PX
}
/// The map camera hangs this far above the highest ground (orthographic: height does not change the scale).
const CAM_ABOVE_TOP_M: f32 = 400.0;

#[derive(Resource, Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum PlanView {
    #[default]
    Map,
    Orbit,
}

/// Map view state: centre of the window in map metres (e east, n north) and the map height shown by the window.
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct MapCam {
    pub e: f32,
    pub n: f32,
    pub height_m: f32,
    pub min_height_m: f32,
    pub max_height_m: f32,
    /// the box the map was framed on, and whether the user has moved the map since (re-framed on resize until then)
    pub fit_box: ((f32, f32), (f32, f32)),
    pub user_moved: bool,
}

impl MapCam {
    pub fn m_per_px(&self, win_h: f32) -> f32 {
        self.height_m / win_h.max(1.0)
    }

    /// Map position (e, n) under window pixel (x, y) (y down), for a window of `win` CSS px.
    pub fn to_map(&self, px: Vec2, win: Vec2) -> (f32, f32) {
        let k = self.m_per_px(win.y);
        (self.e + (px.x - win.x / 2.0) * k, self.n - (px.y - win.y / 2.0) * k)
    }

    /// Zoom by `factor` (< 1 zooms in) keeping the map point under `px` fixed on screen.
    pub fn zoom_about(&mut self, factor: f32, px: Vec2, win: Vec2) {
        let (e0, n0) = self.to_map(px, win);
        self.height_m = (self.height_m * factor).clamp(self.min_height_m, self.max_height_m);
        let (e1, n1) = self.to_map(px, win);
        self.e += e0 - e1;
        self.n += n0 - n1;
    }

    pub fn transform(&self, top_m: f32) -> Transform {
        // looking straight down with north (-z) at the top of the screen
        Transform::from_xyz(self.e, top_m + CAM_ABOVE_TOP_M, -self.n).looking_to(Vec3::NEG_Y, Vec3::NEG_Z)
    }
}

/// Frame the map box (e0, n0)-(e1, n1) in the free map area of a `win` (CSS px) window (inside the four edges).
pub fn fit(min: (f32, f32), max: (f32, f32), win: Vec2, grid_extent_m: f32) -> MapCam {
    let (bw, bh) = ((max.0 - min.0).max(1.0), (max.1 - min.1).max(1.0));
    let (top, bottom) = (BAR_PX + LABELS_PX, TIMELINE_PX);
    let usable = Vec2::new((win.x - RAIL_L_PX - RAIL_R_PX - 2.0 * MARGIN_PX).max(100.0), (win.y - top - bottom - 2.0 * MARGIN_PX).max(100.0));
    let k = (bw / usable.x).max(bh / usable.y); // metres per px
    let height_m = k * win.y;
    // the box centre sits at the centre of the free area: (RAIL_L - RAIL_R) / 2 right of the window centre and
    // (top - bottom) / 2 below it (screen y grows downward, map n upward)
    let e = (min.0 + max.0) / 2.0 - (RAIL_L_PX - RAIL_R_PX) / 2.0 * k;
    let n = (min.1 + max.1) / 2.0 + (top - bottom) / 2.0 * k;
    MapCam { e, n, height_m, min_height_m: 20.0, max_height_m: grid_extent_m * 1.6, fit_box: (min, max), user_moved: false }
}

pub fn ortho(height_m: f32) -> Projection {
    Projection::Orthographic(OrthographicProjection {
        scaling_mode: bevy::camera::ScalingMode::FixedVertical { viewport_height: height_m },
        near: 0.1,
        far: 5000.0,
        ..OrthographicProjection::default_3d()
    })
}

/// Perspective settings shared by Walk mode and the 3D orbit view.
#[derive(Resource, Clone)]
pub struct PerspectiveSettings(pub PerspectiveProjection);

pub fn in_map(mode: Res<Mode>, view: Res<PlanView>) -> bool {
    *mode == Mode::Plan && *view == PlanView::Map
}

pub fn in_orbit(mode: Res<Mode>, view: Res<PlanView>) -> bool {
    *mode == Mode::Plan && *view == PlanView::Orbit
}

/// V switches map and 3D in Plan mode; the 3D view turns around the map's centre and vice versa.
pub fn toggle_view(
    keys: Res<ButtonInput<KeyCode>>,
    mode: Res<Mode>,
    mut view: ResMut<PlanView>,
    map: Option<ResMut<MapCam>>,
    ground: Option<Res<Ground>>,
    mut orbits: Query<&mut Orbit>,
) {
    if *mode != Mode::Plan || !tapped(&keys, Act::ToggleView) {
        return;
    }
    let (Some(mut map), Ok(mut o)) = (map, orbits.single_mut()) else { return };
    *view = match *view {
        PlanView::Map => {
            o.focus = Vec3::new(map.e, ground.as_ref().map(|g| g.0.height_at(map.e, map.n)).unwrap_or(o.focus.y), -map.n);
            PlanView::Orbit
        }
        PlanView::Orbit => {
            map.e = o.focus.x;
            map.n = -o.focus.z;
            PlanView::Map
        }
    };
}

/// Put the right projection on the camera for the current mode and view (and keep the map's zoom in step).
pub fn apply_projection(
    mode: Res<Mode>,
    view: Res<PlanView>,
    map: Option<Res<MapCam>>,
    persp: Option<Res<PerspectiveSettings>>,
    mut cams: Query<&mut Projection, With<Camera3d>>,
) {
    let (Some(map), Some(persp)) = (map, persp) else { return };
    if !(mode.is_changed() || view.is_changed() || map.is_changed()) {
        return;
    }
    let want_map = *mode == Mode::Plan && *view == PlanView::Map;
    for mut p in &mut cams {
        match (&mut *p, want_map) {
            (Projection::Orthographic(o), true) => {
                o.scaling_mode = bevy::camera::ScalingMode::FixedVertical { viewport_height: map.height_m };
            }
            (_, true) => *p = ortho(map.height_m),
            (Projection::Perspective(_), false) => {}
            (_, false) => *p = Projection::Perspective(persp.0.clone()),
        }
    }
}

pub fn map_controls(
    windows: Query<&Window>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    ground: Option<Res<Ground>>,
    map: Option<ResMut<MapCam>>,
    mut cams: Query<&mut Transform, With<Camera3d>>,
) {
    let (Ok(win), Some(mut map), Some(ground)) = (windows.single(), map, ground) else { return };
    let size = Vec2::new(win.width(), win.height());
    let k = map.m_per_px(size.y);
    if (buttons.pressed(MouseButton::Left) || buttons.pressed(MouseButton::Right)) && motion.delta != Vec2::ZERO {
        map.e -= motion.delta.x * k;
        map.n += motion.delta.y * k;
        map.user_moved = true;
    } else if !map.user_moved {
        // the browser sizes the canvas after start-up (and on resize): keep the planned loop framed until the user moves
        let (lo, hi) = map.fit_box;
        let refit = fit(lo, hi, size, map.max_height_m / 1.6);
        if (refit.height_m - map.height_m).abs() > 0.01 || (refit.e - map.e).abs() > 0.01 {
            *map = refit;
        }
    }
    let wheel = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y,
        MouseScrollUnit::Pixel => scroll.delta.y / 100.0,
    };
    if wheel != 0.0 {
        let at = win.cursor_position().unwrap_or(size / 2.0);
        map.zoom_about((-wheel * 0.15).exp(), at, size);
        map.user_moved = true;
    }
    // keep the map centre over the terrain
    let (w, h) = (ground.0.w as f32 * ground.0.cell, ground.0.h as f32 * ground.0.cell);
    let (e, n) = (map.e.clamp(0.0, w), map.n.clamp(0.0, h));
    if (e, n) != (map.e, map.n) {
        map.e = e;
        map.n = n;
    }
    let t = map.transform(ground.0.top);
    for mut c in &mut cams {
        if *c != t {
            *c = t;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_keeps_the_box_in_the_free_map_area() {
        let win = Vec2::new(1440.0, 900.0);
        let m = fit((100.0, 400.0), (300.0, 600.0), win, 1078.0);
        let k = m.m_per_px(win.y);
        // box corners in window px: x = (e - m.e) / k + w/2, y = (m.n - n) / k + h/2
        let px = |e: f32, n: f32| Vec2::new((e - m.e) / k + win.x / 2.0, (m.n - n) / k + win.y / 2.0);
        for (e, n) in [(100.0, 400.0), (300.0, 600.0)] {
            let p = px(e, n);
            assert!(in_map_area(p, win), "{p:?}");
            assert!(p.x >= RAIL_L_PX + MARGIN_PX - 0.5 && p.x <= win.x - RAIL_R_PX - MARGIN_PX + 0.5, "x {p:?}");
            assert!(p.y >= BAR_PX + LABELS_PX + MARGIN_PX - 0.5 && p.y <= win.y - TIMELINE_PX - MARGIN_PX + 0.5, "y {p:?}");
        }
    }

    #[test]
    fn north_is_up_and_east_is_right_on_screen() {
        let m = MapCam { e: 100.0, n: 100.0, height_m: 80.0, min_height_m: 1.0, max_height_m: 1e4, fit_box: ((0.0, 0.0), (1.0, 1.0)), user_moved: false };
        let win = Vec2::new(800.0, 800.0);
        let (e, n) = m.to_map(Vec2::new(800.0, 0.0), win); // top-right pixel
        assert!(e > 100.0 && n > 100.0, "top-right is north-east: {e} {n}");
        let t = m.transform(50.0);
        assert!(t.forward().y < -0.999, "looks straight down");
        assert!(t.up().z < -0.999, "screen up is north (-z)");
        assert!(t.right().x > 0.999, "screen right is east (+x)");
    }

    #[test]
    fn zoom_keeps_the_point_under_the_pointer() {
        let mut m = MapCam { e: 100.0, n: 100.0, height_m: 400.0, min_height_m: 20.0, max_height_m: 2000.0, fit_box: ((0.0, 0.0), (1.0, 1.0)), user_moved: false };
        let (win, at) = (Vec2::new(1000.0, 800.0), Vec2::new(900.0, 100.0));
        let before = m.to_map(at, win);
        m.zoom_about(0.5, at, win);
        let after = m.to_map(at, win);
        assert!((before.0 - after.0).abs() < 1e-3 && (before.1 - after.1).abs() < 1e-3);
        assert_eq!(m.height_m, 200.0);
        m.zoom_about(0.001, at, win);
        assert_eq!(m.height_m, 20.0, "zoom limited");
    }
}
