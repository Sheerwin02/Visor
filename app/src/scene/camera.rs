//! Overview orbit camera (Plan-style look around): left-drag rotates, right-drag or Shift+left-drag pans, wheel zooms.
//! Yaw is the compass heading of the view (0 = looking north, 90 = east); pitch is the angle below the horizon.
//! Kept small and self-written (instead of Bevy's anchor-based pan_orbit_camera) so it is predictable and unit-tested.
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;

use crate::scene::terrain::Grid;
use crate::scene::walk::Ground;

/// Lowest the overview camera may come to the ground below it (a visual comfort choice, not a measured value).
pub const CLEAR_M: f32 = 5.0;

#[derive(Component, Debug, Clone)]
pub struct Orbit {
    pub focus: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub radius: f32,
    pub min_radius: f32,
    pub max_radius: f32,
}

impl Orbit {
    /// Camera position: offset from the focus opposite to the viewing direction.
    pub fn eye(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        self.focus + self.radius * Vec3::new(-sy * cp, sp, cy * cp)
    }
    pub fn transform(&self) -> Transform {
        Transform::from_translation(self.eye()).looking_at(self.focus, Vec3::Y)
    }
}

/// Keep the orbit honest about the terrain: the focus (what zoom and rotate turn around) sits on the ground inside the map,
/// and the camera stays at least CLEAR_M above the ground under it (by raising the pitch if needed).
pub fn settle(o: &mut Orbit, g: &Grid) {
    o.focus.x = o.focus.x.clamp(0.0, g.w as f32 * g.cell);
    o.focus.z = o.focus.z.clamp(-(g.h as f32) * g.cell, 0.0);
    o.focus.y = g.height_at(o.focus.x, -o.focus.z);
    let max_pitch = 89f32.to_radians();
    for _ in 0..90 {
        let e = o.eye();
        if !g.contains(e.x, e.z) || e.y >= g.height_at(e.x, -e.z) + CLEAR_M || o.pitch >= max_pitch {
            break;
        }
        o.pitch = (o.pitch + 1f32.to_radians()).min(max_pitch);
    }
}

/// Distance at which a bounding sphere of radius `r` fits the view: vertical field of view `vfov` (rad), width/height
/// `aspect`. Reference for the tighter fit_box (used in tests).
#[cfg_attr(not(test), allow(dead_code))]
pub fn fit_distance(r: f32, vfov: f32, aspect: f32) -> f32 {
    let hfov = 2.0 * ((vfov / 2.0).tan() * aspect).atan();
    r / (vfov.min(hfov) / 2.0).sin()
}

/// Smallest orbit radius at which every corner of an axis-aligned box (min, max) projects inside `fill` (0..1) of the view,
/// for the given yaw, pitch, vertical FOV (rad) and aspect (width/height). Bisection on the radius.
pub fn fit_box(min: Vec3, max: Vec3, yaw: f32, pitch: f32, vfov: f32, aspect: f32, fill: f32) -> f32 {
    let focus = (min + max) / 2.0;
    let corners: Vec<Vec3> = (0..8)
        .map(|i| Vec3::new(if i & 1 == 0 { min.x } else { max.x }, if i & 2 == 0 { min.y } else { max.y }, if i & 4 == 0 { min.z } else { max.z }))
        .collect();
    let (tv, th) = ((vfov / 2.0).tan() * fill, (vfov / 2.0).tan() * aspect * fill);
    let fits = |radius: f32| {
        let o = Orbit { focus, yaw, pitch, radius, min_radius: 0.0, max_radius: f32::MAX };
        let view = o.transform().to_matrix().inverse();
        corners.iter().all(|c| {
            let p = view.transform_point3(*c);
            let depth = -p.z;
            depth > 0.0 && (p.x / depth).abs() <= th && (p.y / depth).abs() <= tv
        })
    };
    let (mut lo, mut hi) = (1.0f32, 1.0e6f32);
    for _ in 0..60 {
        let mid = (lo * hi).sqrt();
        if fits(mid) { hi = mid } else { lo = mid }
    }
    hi
}

/// A "nice" scale-bar length (1, 2 or 5 x 10^k metres) no longer than `max_m`.
pub fn nice_length(max_m: f32) -> f32 {
    // exact powers of ten (repeated multiplication drifts: 0.01 x 10^4 gives 99.99999, which would label "99 m")
    let mut best = 1.0;
    for e in -2..=6 {
        let k = 10f32.powi(e);
        for m in [1.0, 2.0, 5.0] {
            if m * k <= max_m {
                best = m * k;
            }
        }
    }
    best
}

pub fn orbit_controls(
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    ground: Option<Res<Ground>>,
    mut q: Query<(&mut Orbit, &mut Transform)>,
) {
    for (mut o, mut t) in &mut q {
        let d = motion.delta;
        let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
        if buttons.pressed(MouseButton::Right) || (buttons.pressed(MouseButton::Left) && shift) {
            // pan in the horizontal plane, scaled by distance
            let (sy, cy) = o.yaw.sin_cos();
            let forward = Vec3::new(sy, 0.0, -cy);
            let right = Vec3::new(cy, 0.0, sy);
            let k = o.radius * 0.0015;
            o.focus += (-d.x * right + d.y * forward) * k;
        } else if buttons.pressed(MouseButton::Left) {
            o.yaw += d.x * 0.005;
            o.pitch = (o.pitch + d.y * 0.005).clamp(10f32.to_radians(), 89f32.to_radians());
        }
        let wheel = match scroll.unit {
            MouseScrollUnit::Line => scroll.delta.y,
            MouseScrollUnit::Pixel => scroll.delta.y / 100.0,
        };
        if wheel != 0.0 {
            o.radius = (o.radius * (-wheel * 0.12).exp()).clamp(o.min_radius, o.max_radius);
        }
        if let Some(g) = ground.as_ref() {
            settle(&mut o, &g.0);
        }
        *t = o.transform();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaw_zero_looks_north_and_ninety_looks_east() {
        let mut o = Orbit { focus: Vec3::ZERO, yaw: 0.0, pitch: 45f32.to_radians(), radius: 10.0, min_radius: 1.0, max_radius: 100.0 };
        let fwd = o.transform().forward();
        assert!(fwd.z < -0.5 && fwd.x.abs() < 1e-5, "yaw 0 must look north (-z): {fwd:?}");
        o.yaw = 90f32.to_radians();
        let fwd = o.transform().forward();
        assert!(fwd.x > 0.5 && fwd.z.abs() < 1e-4, "yaw 90 must look east (+x): {fwd:?}");
        assert!((o.eye() - o.focus).length() - 10.0 < 1e-4);
    }

    fn hill() -> Grid {
        // 40 x 40 m, flat at 0 except a 30 m high block in the north-east quarter
        let mut b = Vec::new();
        for r in 0..40 {
            for c in 0..40 {
                let z: f32 = if r < 20 && c >= 20 { 30.0 } else { 0.0 };
                b.extend_from_slice(&z.to_le_bytes());
            }
        }
        Grid::from_bytes(&b, 40, 40, 1.0).unwrap()
    }

    #[test]
    fn focus_lands_on_the_ground_and_stays_on_the_map() {
        let g = hill();
        let mut o = Orbit { focus: Vec3::new(10.0, 58.0, -10.0), yaw: 0.0, pitch: 0.8, radius: 50.0, min_radius: 1.0, max_radius: 500.0 };
        settle(&mut o, &g);
        assert!(o.focus.y.abs() < 1e-4, "floating focus pulled down to the flat ground: {:?}", o.focus);
        o.focus = Vec3::new(30.0, 0.0, -30.0); // over the block
        settle(&mut o, &g);
        assert!((o.focus.y - 30.0).abs() < 1e-4, "buried focus lifted onto the block: {:?}", o.focus);
        o.focus = Vec3::new(500.0, 0.0, 20.0);
        settle(&mut o, &g);
        assert!(g.contains(o.focus.x, o.focus.z), "focus clamped to the map: {:?}", o.focus);
    }

    #[test]
    fn camera_is_lifted_out_of_the_ground() {
        let g = hill();
        // focus at the foot of the block, camera low and on the block's side: it would sit inside the block
        let mut o = Orbit { focus: Vec3::new(30.0, 0.0, -22.0), yaw: 180f32.to_radians(), pitch: 5f32.to_radians(), radius: 12.0, min_radius: 1.0, max_radius: 500.0 };
        let e = o.eye();
        assert!(e.y < g.height_at(e.x, -e.z), "test setup: the eye starts inside the block");
        settle(&mut o, &g);
        let e = o.eye();
        assert!(e.y >= g.height_at(e.x, -e.z) + CLEAR_M - 1e-3, "eye {e:?} must clear the ground by {CLEAR_M} m");
    }

    #[test]
    fn box_fit_is_tighter_than_sphere_and_keeps_all_corners_in_view() {
        let (min, max) = (Vec3::new(0.0, 0.0, -1078.0), Vec3::new(553.0, 60.0, 0.0)); // the VISOR grid's footprint
        let (yaw, pitch, vfov, aspect, fill) = (0.0, 55f32.to_radians(), 45f32.to_radians(), 16.0 / 9.0, 0.9);
        let r = fit_box(min, max, yaw, pitch, vfov, aspect, fill);
        let sphere = fit_distance(((max - min) / 2.0).length(), vfov, aspect);
        assert!(r < sphere, "box fit {r} should be tighter than the sphere fit {sphere}");
        // just inside at r, and a 5% closer camera would push a corner out of the fill area
        let o = Orbit { focus: (min + max) / 2.0, yaw, pitch, radius: r * 0.95, min_radius: 0.0, max_radius: f32::MAX };
        let view = o.transform().to_matrix().inverse();
        let tv = (vfov / 2.0).tan() * fill;
        let out = [Vec3::new(0.0, 0.0, 0.0), Vec3::new(553.0, 60.0, -1078.0), Vec3::new(0.0, 60.0, -1078.0), Vec3::new(553.0, 0.0, 0.0)]
            .iter()
            .any(|c| { let p = view.transform_point3(*c); (p.y / -p.z).abs() > tv || (p.x / -p.z).abs() > tv * aspect });
        assert!(out, "the fit should be tight");
    }

    #[test]
    fn whole_sphere_fits_and_scale_bar_is_nice() {
        // a 45 deg vertical FOV, square view: the sphere's angular radius must not exceed half the FOV
        let d = fit_distance(100.0, 45f32.to_radians(), 1.0);
        assert!(((100.0f32 / d).asin() - 22.5f32.to_radians()).abs() < 1e-5);
        // a narrow (portrait) window needs a larger distance than a wide one
        assert!(fit_distance(100.0, 0.8, 0.5) > fit_distance(100.0, 0.8, 2.0));
        assert_eq!(nice_length(149.0), 100.0);
        assert_eq!(nice_length(260.0), 200.0);
        assert_eq!(nice_length(7.0), 5.0);
    }
}
