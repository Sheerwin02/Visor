//! Route line (key 5) and stop flags (key 6), per ui_text layers."5" and "6".
//! Flags follow the HUD rule "vertical flag markers": a pole with a panel at each stop, plus a tall thin beacon line so a
//! flag can be found from far away. Meaning is not carried by colour alone: stops are also named in the HUD and stop cards.
use bevy::asset::RenderAssetUsages;
use bevy::mesh::PrimitiveTopology;
use bevy::prelude::*;

use crate::scene::controls::{tapped, Act};
use crate::scene::rover_path::draped_line;
use crate::scene::terrain::Grid;
use crate::scene::walk::Stop;

pub const POLE_M: f32 = 4.0;
pub const BEACON_M: f32 = 40.0;
/// The pole stands this far east of the stop point, so a walker standing on the stop is not inside it (m).
pub const POLE_OFFSET_M: f32 = 1.2;

#[derive(Component)]
pub struct RouteLayer;

#[derive(Component)]
pub struct StopsLayer;

/// Drawn only in Plan mode (the thin route line) / only in Walk mode (the ground ribbon, chevrons, arrival rings).
#[derive(Component)]
pub struct PlanOnly;
#[derive(Component)]
pub struct WalkOnly;

/// Walk-mode AR stop marker: a light beam (height, width) and the working circle. AR colour (linear RGB). VISUAL choices.
pub const BEAM_M: f32 = 30.0;
pub const BEAM_W: f32 = 0.08;
pub const AR: [f32; 3] = [0.25, 0.85, 1.0];

/// A vertical beam: two crossed quads, bright at the ground and fading out at the top (vertex alpha).
pub fn beam_mesh(h: f32, w: f32) -> Mesh {
    let (mut pos, mut col, mut idx) = (Vec::new(), Vec::new(), Vec::<u32>::new());
    for (dx, dz) in [(w / 2.0, 0.0), (0.0, w / 2.0)] {
        let b = pos.len() as u32;
        for (x, z, y, a) in [(-dx, -dz, 0.0, 0.5), (dx, dz, 0.0, 0.5), (dx, dz, h, 0.0), (-dx, -dz, h, 0.0)] {
            pos.push([x, y, z]);
            col.push([AR[0], AR[1], AR[2], a]);
        }
        idx.extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);
    }
    tri_mesh(pos, Some(col), idx)
}

/// Walk-mode route ribbon: width on the ground, lift above it (m), chevron spacing (m). VISUAL choices.
pub const RIBBON_W: f32 = 0.6;
pub const RIBBON_LIFT: f32 = 0.06;
pub const CHEVRON_EVERY_M: f32 = 4.0;

/// Points every `step` metres along a polyline (ends kept).
pub fn resample(poly: &[(f32, f32)], step: f32) -> Vec<(f32, f32)> {
    let mut out = Vec::new();
    for w in poly.windows(2) {
        let ((e0, n0), (e1, n1)) = (w[0], w[1]);
        let len = ((e1 - e0).powi(2) + (n1 - n0).powi(2)).sqrt();
        let k = (len / step).ceil().max(1.0) as usize;
        for i in 0..k {
            let t = i as f32 / k as f32;
            out.push((e0 + (e1 - e0) * t, n0 + (n1 - n0) * t));
        }
    }
    if let Some(&l) = poly.last() {
        out.push(l);
    }
    out
}

/// A flat strip of width `w` along the route, draped on the terrain `lift` above it, with direction chevrons every
/// CHEVRON_EVERY_M. Returns (positions, colours, indices) for a TriangleList; route colour for the strip, dark chevrons.
pub fn ribbon(grid: &Grid, poly: &[(f32, f32)], w: f32, lift: f32, strip: [f32; 4], chevron: [f32; 4]) -> (Vec<[f32; 3]>, Vec<[f32; 4]>, Vec<u32>) {
    let p = resample(poly, 0.5);
    let (mut pos, mut col, mut idx) = (Vec::new(), Vec::new(), Vec::new());
    if p.len() < 2 {
        return (pos, col, idx);
    }
    let at = |e: f32, n: f32, up: f32| [e, grid.height_at(e, n) + up, -n];
    let dir = |i: usize| {
        let (a, b) = (p[i.saturating_sub(1)], p[(i + 1).min(p.len() - 1)]);
        let (de, dn) = (b.0 - a.0, b.1 - a.1);
        let l = (de * de + dn * dn).sqrt().max(1e-6);
        (de / l, dn / l)
    };
    for (i, &(e, n)) in p.iter().enumerate() {
        let (de, dn) = dir(i);
        let (le, ln) = (-dn * w / 2.0, de * w / 2.0); // left of travel
        pos.push(at(e + le, n + ln, lift));
        pos.push(at(e - le, n - ln, lift));
        col.push(strip);
        col.push(strip);
        if i > 0 {
            let b = (2 * i) as u32;
            idx.extend_from_slice(&[b - 2, b - 1, b, b, b - 1, b + 1]);
        }
    }
    // chevrons: a "V" pointing along the travel direction, slightly above the strip
    let mut s_along = 0.0f32;
    for i in 1..p.len() {
        s_along += ((p[i].0 - p[i - 1].0).powi(2) + (p[i].1 - p[i - 1].1).powi(2)).sqrt();
        if s_along < CHEVRON_EVERY_M {
            continue;
        }
        s_along = 0.0;
        let (e, n) = p[i];
        let (de, dn) = dir(i);
        let (le, ln) = (-dn, de);
        let (h, a, t) = (w * 0.38, 0.3f32, 0.12f32); // half-width, arm length back, thickness
        let tip = (e + de * 0.15, n + dn * 0.15);
        for side in [-1.0f32, 1.0] {
            let back = (tip.0 - de * a + le * h * side, tip.1 - dn * a + ln * h * side);
            let q = [tip, (tip.0 - de * t, tip.1 - dn * t), (back.0 - de * t, back.1 - dn * t), back];
            let b = pos.len() as u32;
            for (qe, qn) in q {
                pos.push(at(qe, qn, lift + 0.01));
                col.push(chevron);
            }
            idx.extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);
        }
    }
    (pos, col, idx)
}

/// A flat ring on the terrain around (e, n), inner and outer radius in metres.
pub fn ring(grid: &Grid, e: f32, n: f32, r_in: f32, r_out: f32, lift: f32) -> (Vec<[f32; 3]>, Vec<u32>) {
    let segs = 64u32;
    let (mut pos, mut idx) = (Vec::new(), Vec::new());
    for k in 0..=segs {
        let a = k as f32 / segs as f32 * std::f32::consts::TAU;
        for r in [r_in, r_out] {
            let (pe, pn) = (e + r * a.cos(), n + r * a.sin());
            pos.push([pe, grid.height_at(pe, pn) + lift, -pn]);
        }
        if k > 0 {
            let b = 2 * k;
            idx.extend_from_slice(&[b - 2, b, b - 1, b - 1, b, b + 1]);
        }
    }
    (pos, idx)
}

fn tri_mesh(pos: Vec<[f32; 3]>, col: Option<Vec<[f32; 4]>>, idx: Vec<u32>) -> Mesh {
    let n = pos.len();
    let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; n]);
    if let Some(c) = col {
        m.insert_attribute(Mesh::ATTRIBUTE_COLOR, c);
    }
    m.insert_indices(bevy::mesh::Indices::U32(idx));
    m
}

/// Show Plan-only parts in Plan and Walk-only parts in Walk (the layer switches still hide both through their parent).
pub fn mode_parts(mode: Res<crate::scene::walk::Mode>, warm: Res<crate::scene::walk::WalkWarm>,
                  mut plan: Query<&mut Visibility, (With<PlanOnly>, Without<WalkOnly>)>,
                  mut walk: Query<&mut Visibility, (With<WalkOnly>, Without<PlanOnly>)>) {
    let w = *mode == crate::scene::walk::Mode::Walk || warm.0;
    let (pv, wv) = if w { (Visibility::Hidden, Visibility::Inherited) } else { (Visibility::Inherited, Visibility::Hidden) };
    for mut v in &mut plan {
        if *v != pv {
            *v = pv;
        }
    }
    for mut v in &mut walk {
        if *v != wv {
            *v = wv;
        }
    }
}


/// Chaikin corner cutting, for drawing only: the planner's route runs cell to cell on a 1 m grid, which reads as a
/// staircase on screen. Ends are kept; the smoothed line stays within half a cell of the planned one.
pub fn chaikin(poly: &[(f32, f32)], iterations: usize) -> Vec<(f32, f32)> {
    let mut p = poly.to_vec();
    for _ in 0..iterations {
        if p.len() < 3 {
            return p;
        }
        let mut q = Vec::with_capacity(p.len() * 2);
        q.push(p[0]);
        for w in p.windows(2) {
            let (a, b) = (w[0], w[1]);
            q.push((0.75 * a.0 + 0.25 * b.0, 0.75 * a.1 + 0.25 * b.1));
            q.push((0.25 * a.0 + 0.75 * b.0, 0.25 * a.1 + 0.75 * b.1));
        }
        q.push(*p.last().unwrap());
        p = q;
    }
    p
}

pub fn spawn_route(commands: &mut Commands, meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>, grid: &Grid, poly: &[(f32, f32)]) -> Entity {
    let mut line = Mesh::new(PrimitiveTopology::LineList, RenderAssetUsages::RENDER_WORLD);
    line.insert_attribute(Mesh::ATTRIBUTE_POSITION, draped_line(grid, &chaikin(poly, 2), 1.0, 0.0, 0.25));
    let mat = materials.add(StandardMaterial { base_color: crate::ui::palette::ROUTE.color(), unlit: true, ..default() });
    let rc = crate::ui::palette::ROUTE.color().to_linear();
    let (pos, col, idx) = ribbon(grid, &chaikin(poly, 2), RIBBON_W, RIBBON_LIFT, [rc.red, rc.green, rc.blue, 0.45], [1.0, 0.97, 0.9, 0.85]);
    let rib_mat = crate::scene::armat::handle();
    let rib = meshes.add(tri_mesh(pos, Some(col), idx));
    // the route is painted on the ground by the terrain shader in both modes (coloured by steepness, select.rs paint_route);
    // this entity stays as the layer switch (key 5). The line and ribbon meshes are no longer spawned.
    let _ = (line, mat, rib, rib_mat);
    commands.spawn((RouteLayer, Transform::default(), Visibility::Visible, Name::new("layer: route"))).id()
}

/// Vertical beacon segments (a LineList) above each stop, from the ground to BEACON_M.
pub fn beacon_lines(grid: &Grid, stops: &[Stop]) -> Vec<[f32; 3]> {
    stops
        .iter()
        .flat_map(|s| {
            let e = s.e + POLE_OFFSET_M;
            let y = grid.height_at(e, s.n);
            [[e, y, -s.n], [e, y + BEACON_M, -s.n]]
        })
        .collect()
}

pub fn spawn_flags(commands: &mut Commands, meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>, grid: &Grid, stops: &[Stop]) -> Entity {
    let mat = materials.add(StandardMaterial { base_color: crate::ui::palette::FLAG.color(), unlit: true, cull_mode: None, ..default() });
    let pole = meshes.add(Cylinder::new(0.08, POLE_M));
    let (ring_mat, disc_mat, ar_mat) = (crate::scene::armat::handle(), crate::scene::armat::handle(), crate::scene::armat::beam_handle());
    let beam = meshes.add(beam_mesh(BEAM_M, BEAM_W));
    let panel = meshes.add(Cuboid::new(1.2, 0.8, 0.04));
    let mut beacon = Mesh::new(PrimitiveTopology::LineList, RenderAssetUsages::RENDER_WORLD);
    beacon.insert_attribute(Mesh::ATTRIBUTE_POSITION, beacon_lines(grid, stops));
    commands
        .spawn((StopsLayer, Transform::default(), Visibility::Visible, Name::new("layer: stops")))
        .with_children(|p| {
            // Plan: no 3D flags, poles or beacon lines (they read as placeholders); numbered HTML pins on the
            // ground mark the stops (labels.rs). Walk keeps the AR beam, disc and arrival ring below.
            let _ = (&mat, &pole, &panel, beacon);
            for s in stops {
                // Walk: AR beam on the stop point itself, and a faint disc inside the working circle
                let foot = Vec3::new(s.e, grid.height_at(s.e, s.n), -s.n);
                p.spawn((Mesh3d(beam.clone()), MeshMaterial3d(ar_mat.clone()), Transform::from_translation(foot), WalkOnly, Visibility::Hidden,
                         bevy::light::NotShadowCaster, Name::new(format!("AR beam: {}", s.name))));
                let (dp, di) = ring(grid, s.e, s.n, 0.0, crate::scene::walk::ARRIVE_M, RIBBON_LIFT - 0.01);
                let dc = crate::scene::armat::colours(dp.len(), [AR[0], AR[1], AR[2], 0.10]);
                p.spawn((Mesh3d(meshes.add(tri_mesh(dp, Some(dc), di))), MeshMaterial3d(disc_mat.clone()), WalkOnly, Visibility::Hidden,
                         bevy::light::NotShadowCaster));
                // arrival ring (Walk): the stop counts as reached inside walk::ARRIVE_M
                let (rp, ri) = ring(grid, s.e, s.n, crate::scene::walk::ARRIVE_M - 0.12, crate::scene::walk::ARRIVE_M, RIBBON_LIFT);
                let rc = crate::scene::armat::colours(rp.len(), [AR[0], AR[1], AR[2], 0.9]);
                p.spawn((Mesh3d(meshes.add(tri_mesh(rp, Some(rc), ri))), MeshMaterial3d(ring_mat.clone()), WalkOnly, Visibility::Hidden,
                         bevy::light::NotShadowCaster));
            }
        })
        .id()
}

pub fn toggle(keys: Res<ButtonInput<KeyCode>>, mut route: Query<&mut Visibility, (With<RouteLayer>, Without<StopsLayer>)>, mut stops: Query<&mut Visibility, (With<StopsLayer>, Without<RouteLayer>)>) {
    let flip = |v: &mut Visibility| *v = if *v == Visibility::Hidden { Visibility::Visible } else { Visibility::Hidden };
    if tapped(&keys, Act::LayerRoute) {
        for mut v in &mut route { flip(&mut v); }
    }
    if tapped(&keys, Act::LayerStops) {
        for mut v in &mut stops { flip(&mut v); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chaikin_keeps_the_ends_and_stays_close() {
        let p = vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (2.0, 1.0)];
        let q = chaikin(&p, 2);
        assert_eq!(q.first(), p.first());
        assert_eq!(q.last(), p.last());
        assert!(q.len() > p.len());
        for (x, y) in q {
            let d = p.windows(2).map(|w| {
                let (a, b) = (w[0], w[1]);
                let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                let t = (((x - a.0) * dx + (y - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
                ((x - a.0 - t * dx).powi(2) + (y - a.1 - t * dy).powi(2)).sqrt()
            }).fold(f32::MAX, f32::min);
            assert!(d <= 0.5, "{d}");
        }
    }

    #[test]
    fn beacons_stand_on_the_ground_at_each_stop() {
        let mut b = Vec::new();
        for r in 0..10 {
            for c in 0..10 {
                b.extend_from_slice(&((r + c) as f32).to_le_bytes());
            }
        }
        let g = Grid::from_bytes(&b, 10, 10, 1.0).unwrap();
        let stops = vec![Stop { id: "a".into(), name: "A".into(), e: 3.5, n: 4.5 }, Stop { id: "b".into(), name: "B".into(), e: 7.5, n: 1.5 }];
        let v = beacon_lines(&g, &stops);
        assert_eq!(v.len(), 4);
        for (i, s) in stops.iter().enumerate() {
            let (foot, top) = (v[2 * i], v[2 * i + 1]);
            // beside the stop (POLE_OFFSET_M east), on the ground there
            let e = s.e + POLE_OFFSET_M;
            assert!((foot[1] - g.height_at(e, s.n)).abs() < 1e-5 && (top[1] - foot[1] - BEACON_M).abs() < 1e-4);
            assert_eq!((foot[0], foot[2]), (e, -s.n));
        }
    }
}

#[cfg(test)]
mod walk_parts_tests {
    use super::*;

    fn flat(w: usize, h: usize) -> Grid {
        let b: Vec<u8> = (0..w * h).flat_map(|_| 2.0f32.to_le_bytes()).collect();
        Grid::from_bytes(&b, w, h, 1.0).unwrap()
    }

    #[test]
    fn ribbon_has_the_set_width_and_lies_just_above_the_ground() {
        let g = flat(40, 40);
        let (pos, col, idx) = ribbon(&g, &[(5.0, 5.0), (25.0, 5.0)], 0.6, 0.06, [1.0; 4], [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(pos.len(), col.len());
        assert!(idx.len() % 3 == 0 && idx.iter().all(|&i| (i as usize) < pos.len()));
        // first cross-section: two points 0.6 m apart across the travel direction (north-south for an eastward route)
        let (a, b) = (Vec3::from(pos[0]), Vec3::from(pos[1]));
        assert!(((a - b).length() - 0.6).abs() < 1e-4 && (a.x - b.x).abs() < 1e-4, "{a:?} {b:?}");
        assert!(pos.iter().all(|p| (p[1] - 2.06).abs() < 0.02), "draped 6 cm above (chevrons 1 cm higher)");
        let chevrons = col.iter().filter(|c| c[0] == 0.0).count(); // the test passes black chevrons
        assert_eq!(chevrons, 8 * 5, "20 m route: chevrons every 4 m (5), 2 arms x 4 vertices each");
    }

    #[test]
    fn ring_radii_are_kept() {
        let g = flat(20, 20);
        let (pos, idx) = ring(&g, 10.0, 10.0, 2.88, 3.0, 0.06);
        assert!(idx.len() % 3 == 0);
        for (k, p) in pos.iter().enumerate() {
            let r = ((p[0] - 10.0).powi(2) + (-p[2] - 10.0).powi(2)).sqrt();
            let want = if k % 2 == 0 { 2.88 } else { 3.0 };
            assert!((r - want).abs() < 1e-3, "{r} vs {want}");
        }
    }
}
