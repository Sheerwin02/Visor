//! "Curiosity's path" layer: a dot at each localized rover position and a dashed line draped on the terrain between them
//! (straight lines between localized stops; the rover's actual drive paths are not in PLACES localized_pos).
//! Toggled with key 7 (ui_text layers."7").
use bevy::asset::RenderAssetUsages;
use bevy::mesh::PrimitiveTopology;
use bevy::prelude::*;

use crate::scene::controls::{tapped, Act};
use crate::scene::terrain::Grid;

const LIFT_M: f32 = 0.4; // keep the line just above the ground
const DASH_M: f32 = 3.0;
const GAP_M: f32 = 2.0;
const STEP_M: f32 = 0.5;

#[derive(Component)]
pub struct RoverPathLayer;

/// A rover-position dot; its size follows the camera distance so it stays a marker, not a boulder, up close.
#[derive(Component)]
pub struct RoverDot;

/// On-screen dot radius in CSS px.
pub const DOT_PX: f32 = 3.0;

/// Dot radius (m) where one screen pixel covers `m_per_px` metres: a constant on-screen size, never below 0.1 m.
pub fn dot_radius(m_per_px: f32) -> f32 {
    (DOT_PX * m_per_px).max(0.1)
}

pub fn scale_dots(
    windows: Query<&Window>,
    cams: Query<(&GlobalTransform, &Projection), With<Camera3d>>,
    mut dots: Query<&mut Transform, With<RoverDot>>,
) {
    let (Ok(win), Ok((cam, proj))) = (windows.single(), cams.single()) else { return };
    let h = win.height().max(1.0);
    let eye = cam.translation();
    for mut t in &mut dots {
        let m_per_px = match proj {
            Projection::Perspective(p) => 2.0 * t.translation.distance(eye) * (p.fov / 2.0).tan() / h,
            Projection::Orthographic(o) => o.area.height() / h,
            _ => continue,
        };
        let r = dot_radius(m_per_px);
        if (t.scale.x - r).abs() > 0.02 * r {
            t.scale = Vec3::splat(r);
        }
    }
}

/// Dashed polyline as a LineList, sampling the terrain every STEP_M along each segment between consecutive points (e, n).
pub fn dashed_line(grid: &Grid, pts: &[(f32, f32)]) -> Vec<[f32; 3]> {
    draped_line(grid, pts, DASH_M, GAP_M, LIFT_M)
}

/// Polyline draped on the terrain as a LineList (`gap_m` = 0 gives a solid line), `lift_m` above the ground.
pub fn draped_line(grid: &Grid, pts: &[(f32, f32)], dash_m: f32, gap_m: f32, lift_m: f32) -> Vec<[f32; 3]> {
    let mut out = Vec::new();
    let period = dash_m + gap_m;
    let mut s_total = 0.0f32; // distance along the whole path, so dashes continue across stops
    for w in pts.windows(2) {
        let ((e0, n0), (e1, n1)) = (w[0], w[1]);
        let len = ((e1 - e0).powi(2) + (n1 - n0).powi(2)).sqrt();
        let steps = (len / STEP_M).ceil().max(1.0) as usize;
        for i in 0..steps {
            let (t0, t1) = (i as f32 / steps as f32, (i + 1) as f32 / steps as f32);
            let mid = s_total + len * (t0 + t1) / 2.0;
            if gap_m > 0.0 && mid % period > dash_m {
                continue; // in a gap
            }
            for t in [t0, t1] {
                let (e, n) = (e0 + (e1 - e0) * t, n0 + (n1 - n0) * t);
                out.push([e, grid.height_at(e, n) + lift_m, -n]);
            }
        }
        s_total += len;
    }
    out
}

pub fn spawn(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    grid: &Grid,
    pts: &[(f32, f32)],
) -> usize {
    let colour = crate::ui::palette::ROVER.color();
    let line_mat = materials.add(StandardMaterial { base_color: colour, unlit: true, ..default() });
    let dot_mat = materials.add(StandardMaterial { base_color: colour, unlit: true, ..default() });
    let mut line = Mesh::new(PrimitiveTopology::LineList, RenderAssetUsages::RENDER_WORLD);
    line.insert_attribute(Mesh::ATTRIBUTE_POSITION, dashed_line(grid, pts));
    let dot = meshes.add(Sphere::new(1.0));
    commands
        .spawn((RoverPathLayer, Transform::default(), Visibility::Visible, Name::new("layer: Curiosity's path")))
        .with_children(|p| {
            p.spawn((Mesh3d(meshes.add(line)), MeshMaterial3d(line_mat), crate::scene::layers::PlanOnly, Visibility::Inherited));
            for &(e, n) in pts {
                // centre 1 m up, as before; a shrunken dot floats a little above the ground, which keeps it visible
                p.spawn((RoverDot, Mesh3d(dot.clone()), MeshMaterial3d(dot_mat.clone()),
                         Transform::from_xyz(e, grid.height_at(e, n) + 1.0, -n), crate::scene::layers::PlanOnly, Visibility::Inherited));
            }
        });
    pts.len()
}

/// One rover footprint for Walk mode: PLACES position, heading (deg clockwise from north) and the rover-check result.
#[derive(Clone, Debug, PartialEq)]
pub struct Footprint {
    pub e: f32,
    pub n: f32,
    pub yaw_deg: f32,
    /// "agrees" | "partial" | "disagrees" (rover check, per stop)
    pub label: String,
}

/// Line pattern per rover-check result (dash on, gap, m): solid / dashed / dotted, so the meaning is not carried by colour alone.
pub fn pattern(label: &str) -> (f32, f32) {
    match label {
        "agrees" => (1.0, 0.0),
        "partial" => (0.45, 0.25),
        _ => (0.12, 0.22),
    }
}

/// Footprint outline (and a heading tick from the centre to the front edge) as thin draped strips, TriangleList.
/// `along` x `across` metres, centred on (e, n), rotated to the heading.
pub fn footprint_mesh(grid: &Grid, f: &Footprint, along: f32, across: f32) -> (Vec<[f32; 3]>, Vec<u32>) {
    let y = f.yaw_deg.to_radians();
    let fwd = (y.sin(), y.cos());
    let rgt = (y.cos(), -y.sin());
    let c = |a: f32, r: f32| (f.e + fwd.0 * a + rgt.0 * r, f.n + fwd.1 * a + rgt.1 * r);
    let (ha, hr) = (along / 2.0, across / 2.0);
    let corners = [c(ha, -hr), c(ha, hr), c(-ha, hr), c(-ha, -hr), c(ha, -hr)];
    let (on, off) = pattern(&f.label);
    let (mut pos, mut idx) = (Vec::new(), Vec::new());
    let w = 0.07f32;
    let mut strip = |p0: (f32, f32), p1: (f32, f32), on: f32, off: f32| {
        let (dx, dy) = (p1.0 - p0.0, p1.1 - p0.1);
        let len = (dx * dx + dy * dy).sqrt().max(1e-6);
        let (ux, uy) = (dx / len, dy / len);
        let (nx, ny) = (-uy * w / 2.0, ux * w / 2.0);
        let mut s0 = 0.0f32;
        while s0 < len {
            let s1 = (s0 + on).min(len);
            let b = pos.len() as u32;
            for (s, side) in [(s0, 1.0f32), (s0, -1.0), (s1, 1.0), (s1, -1.0)] {
                let (e, n) = (p0.0 + ux * s + nx * side, p0.1 + uy * s + ny * side);
                pos.push([e, grid.height_at(e, n) + 0.05, -n]);
            }
            idx.extend_from_slice(&[b, b + 1, b + 2, b + 2, b + 1, b + 3]);
            s0 = s1 + off;
            if off == 0.0 {
                break;
            }
        }
    };
    for k in 0..4 {
        strip(corners[k], corners[k + 1], on, off);
    }
    strip(c(0.0, 0.0), c(ha, 0.0), 1e6, 0.0); // heading tick
    (pos, idx)
}

/// Rover footprints (Walk only), children of a Curiosity's-path layer entity (key 7 toggles them with the path).
pub fn spawn_footprints(commands: &mut Commands, meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>, grid: &Grid,
                        prints: &[Footprint], along: f32, across: f32) {
    let col = |l: &str| match l {
        "agrees" => [0.25, 0.95, 0.75, 0.9],
        "partial" => [1.0, 0.75, 0.2, 0.9],
        _ => [1.0, 0.35, 0.55, 0.9],
    };
    let _ = &materials;
    let mat = crate::scene::armat::handle();
    commands
        .spawn((RoverPathLayer, Transform::default(), Visibility::Visible, Name::new("layer: Curiosity's footprints")))
        .with_children(|p| {
            for f in prints {
                let (pos, idx) = footprint_mesh(grid, f, along, across);
                let n = pos.len();
                let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
                m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
                m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; n]);
                m.insert_attribute(Mesh::ATTRIBUTE_COLOR, crate::scene::armat::colours(n, col(&f.label)));
                m.insert_indices(bevy::mesh::Indices::U32(idx));
                p.spawn((Mesh3d(meshes.add(m)), MeshMaterial3d(mat.clone()), crate::scene::layers::WalkOnly, Visibility::Hidden,
                         bevy::light::NotShadowCaster));
            }
        });
}

pub fn toggle(keys: Res<ButtonInput<KeyCode>>, mut q: Query<&mut Visibility, With<RoverPathLayer>>) {
    if tapped(&keys, Act::LayerRover) {
        for mut v in &mut q {
            *v = if *v == Visibility::Hidden { Visibility::Visible } else { Visibility::Hidden };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dots_keep_a_constant_screen_size() {
        assert_eq!(dot_radius(0.001), 0.1, "floor up close");
        assert!((dot_radius(2.0) - 2.0 * DOT_PX).abs() < 1e-6, "DOT_PX pixels at 2 m per pixel");
    }

    #[test]
    fn dashes_follow_the_ground_and_leave_gaps() {
        let (w, h) = (40usize, 10usize);
        let mut bytes = Vec::new();
        for _r in 0..h {
            for c in 0..w {
                bytes.extend_from_slice(&((c as f32) * 0.5).to_le_bytes()); // slope rising east
            }
        }
        let g = Grid::from_bytes(&bytes, w, h, 1.0).unwrap();
        let v = dashed_line(&g, &[(2.5, 5.0), (32.5, 5.0)]); // a 30 m segment
        assert!(!v.is_empty() && v.len() % 2 == 0);
        for p in &v {
            assert!((p[1] - (g.height_at(p[0], -p[2]) + LIFT_M)).abs() < 1e-4, "vertex off the ground: {p:?}");
        }
        let drawn: f32 = v.chunks(2).map(|s| (s[1][0] - s[0][0]).abs()).sum();
        let expected = 30.0 * DASH_M / (DASH_M + GAP_M);
        assert!((drawn - expected).abs() < 2.0 * STEP_M, "drawn {drawn} vs expected about {expected}");
    }
}

#[cfg(test)]
mod footprint_tests {
    use super::*;

    fn flat() -> Grid {
        let b: Vec<u8> = (0..40 * 40).flat_map(|_| 1.0f32.to_le_bytes()).collect();
        Grid::from_bytes(&b, 40, 40, 1.0).unwrap()
    }

    #[test]
    fn footprint_is_the_rover_check_box_turned_to_the_heading() {
        let g = flat();
        let f = Footprint { e: 20.0, n: 20.0, yaw_deg: 90.0, label: "agrees".into() };
        let (pos, idx) = footprint_mesh(&g, &f, 3.0, 2.8);
        assert!(idx.len() % 3 == 0 && !pos.is_empty());
        // heading east: the box spans 3.0 m east-west and 2.8 m north-south (plus the 7 cm line width)
        let (emin, emax) = pos.iter().fold((f32::MAX, f32::MIN), |a, p| (a.0.min(p[0]), a.1.max(p[0])));
        let (nmin, nmax) = pos.iter().fold((f32::MAX, f32::MIN), |a, p| (a.0.min(-p[2]), a.1.max(-p[2])));
        assert!(((emax - emin) - 3.07).abs() < 0.01 && ((nmax - nmin) - 2.87).abs() < 0.01, "{} {}", emax - emin, nmax - nmin);
        assert!(pos.iter().all(|p| (p[1] - 1.05).abs() < 1e-4));
    }

    #[test]
    fn patterns_differ_by_result() {
        let g = flat();
        let n = |l: &str| footprint_mesh(&g, &Footprint { e: 20.0, n: 20.0, yaw_deg: 0.0, label: l.into() }, 3.0, 2.8).1.len();
        assert!(n("agrees") < n("partial") && n("partial") < n("disagrees"), "solid < dashed < dotted pieces");
    }
}
