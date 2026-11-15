//! Real 3D ground at the Nevado Sajama drill site in Walk mode: Curiosity's Navcam stereo of the drill workspace (sol 4718,
//! PDS XYZ product NLB_816331121XYZ_F1200082NCAM00309M1, ~2.3 x 3.2 m), 50 000 points placed on the map by
//! tools/navcam_check.py (assets/navcam_4718_points.bin: e_m, n_m, height above z_offset_m, slope_deg). The one spot where
//! VISOR has centimetre-scale relief of the boxwork: the 1 m map smooths the ridge top, these points do not.
//! Placement: the check measured the map a median `median_dem_minus_navcam_m` above the Navcam heights (navcam_check.json);
//! the cloud is raised by that measured median so it sits on the map surface (a vertical shift only, stated on screen).
//! Drawn as small horizontal squares in the AR material (position + colour), shaded by height above the patch's best-fit
//! plane (least squares): lighter = higher than the local slope, so the centimetre relief shows, not the workspace's tilt.
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use serde_json::Value;

use crate::scene::walk::{Mode, Walker};

pub const FILE: &str = "navcam_4718_points.bin";
/// Side of each point's square (m): the points are ~1.2 cm apart (50 000 over ~7.4 m2). VISUAL.
pub const QUAD_M: f32 = 0.02;
/// Shown in Walk within this distance (m) of the cloud's centre. VISUAL.
pub const SHOW_M: f32 = 25.0;

#[derive(Component)]
pub struct NavcamPoints {
    pub centre: (f32, f32),
}

/// Points as (e, n, height, slope); a truncated or empty file gives none.
pub fn parse(bytes: &[u8]) -> Vec<[f32; 4]> {
    bytes.chunks_exact(16).map(|c| {
        let f = |i: usize| f32::from_le_bytes([c[i], c[i + 1], c[i + 2], c[i + 3]]);
        [f(0), f(4), f(8), f(12)]
    }).filter(|p| p.iter().all(|x| x.is_finite())).collect()
}

/// The measured vertical offset (m) between the map and the Navcam heights at this site; 0 without the check.
pub fn lift_m(check: &Value) -> f32 {
    check["height_check"]["median_dem_minus_navcam_m"].as_f64().unwrap_or(0.0) as f32
}

/// Height shading: dark brown at the cloud's 2nd percentile height, pale tan at its 98th (robust to stray points).
pub fn shade(h: f32, lo: f32, hi: f32) -> [f32; 4] {
    let t = if hi > lo { ((h - lo) / (hi - lo)).clamp(0.0, 1.0) } else { 0.5 };
    // ramp chosen in sRGB (dark brown to pale tan), handed to the shader as linear vertex colour
    let dark = [0.24, 0.15, 0.10];
    let pale = [0.86, 0.74, 0.58];
    let lin = |x: f32| x.powf(2.2);
    [lin(dark[0] + t * (pale[0] - dark[0])), lin(dark[1] + t * (pale[1] - dark[1])), lin(dark[2] + t * (pale[2] - dark[2])), 1.0]
}

/// Least-squares plane h = a e + b n + c through the points (coordinates taken about their mean for conditioning);
/// returns each point's height above the plane.
pub fn detrend(points: &[[f32; 4]]) -> Vec<f32> {
    let n = points.len() as f64;
    let (me, mn, mh) = points.iter().fold((0.0, 0.0, 0.0), |a, p| (a.0 + p[0] as f64 / n, a.1 + p[1] as f64 / n, a.2 + p[2] as f64 / n));
    let (mut see, mut snn, mut sen, mut seh, mut snh) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for p in points {
        let (e, nn, h) = (p[0] as f64 - me, p[1] as f64 - mn, p[2] as f64 - mh);
        see += e * e; snn += nn * nn; sen += e * nn; seh += e * h; snh += nn * h;
    }
    let det = see * snn - sen * sen;
    let (a, b) = if det.abs() > 1e-12 { ((seh * snn - snh * sen) / det, (snh * see - seh * sen) / det) } else { (0.0, 0.0) };
    points.iter().map(|p| ((p[2] as f64 - mh) - a * (p[0] as f64 - me) - b * (p[1] as f64 - mn)) as f32).collect()
}

/// One square per point, world axes x = e, y = height + lift, z = -n; and the cloud's centre (e, n).
pub fn mesh(points: &[[f32; 4]], lift: f32) -> Option<(Mesh, (f32, f32))> {
    if points.is_empty() {
        return None;
    }
    let rel = detrend(points);
    let mut hs = rel.clone();
    hs.sort_by(|a, b| a.total_cmp(b));
    let q = |f: f32| hs[((hs.len() - 1) as f32 * f).round() as usize];
    let (lo, hi) = (q(0.02), q(0.98));
    let r = QUAD_M / 2.0;
    let (mut pos, mut col, mut idx) = (Vec::with_capacity(points.len() * 4), Vec::with_capacity(points.len() * 4), Vec::with_capacity(points.len() * 6));
    let (mut se, mut sn) = (0.0f64, 0.0f64);
    for (p, &dh) in points.iter().zip(&rel) {
        let (x, y, z) = (p[0], p[2] + lift, -p[1]);
        let base = pos.len() as u32;
        pos.extend_from_slice(&[[x - r, y, z - r], [x + r, y, z - r], [x + r, y, z + r], [x - r, y, z + r]]);
        let c = shade(dh, lo, hi);
        col.extend_from_slice(&[c, c, c, c]);
        // both windings, so the squares show from above whatever the culling
        idx.extend_from_slice(&[base, base + 2, base + 1, base, base + 3, base + 2]);
        se += p[0] as f64;
        sn += p[1] as f64;
    }
    let n = points.len() as f64;
    let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    m.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
    m.insert_indices(Indices::U32(idx));
    Some((m, ((se / n) as f32, (sn / n) as f32)))
}

pub fn spawn(commands: &mut Commands, meshes: &mut Assets<Mesh>, bytes: &[u8], check: Option<&Value>) {
    let pts = parse(bytes);
    let lift = check.map(lift_m).unwrap_or(0.0);
    if let Some((m, centre)) = mesh(&pts, lift) {
        info!("navcam 3D: {} points, raised {lift} m (measured median map - Navcam)", pts.len());
        commands.spawn((Mesh3d(meshes.add(m)), MeshMaterial3d(crate::scene::armat::handle()), Transform::default(), Visibility::Hidden,
                        NavcamPoints { centre }, bevy::light::NotShadowCaster, Name::new("Navcam 3D, sol 4718")));
    }
}

pub fn update(mode: Res<Mode>, walker: Option<Res<Walker>>, mut q: Query<(&NavcamPoints, &mut Visibility)>) {
    let Some(w) = walker else { return };
    for (p, mut vis) in &mut q {
        let near = ((p.centre.0 - w.e).powi(2) + (p.centre.1 - w.n).powi(2)).sqrt() < SHOW_M;
        let want = if *mode == Mode::Walk && near { Visibility::Visible } else { Visibility::Hidden };
        if *vis != want {
            *vis = want;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundle_cloud_parses_and_sits_at_the_drill_site() {
        let b = std::fs::read("../assets/navcam_4718_points.bin").unwrap();
        let pts = parse(&b);
        assert_eq!(pts.len(), 50_000);
        let check: Value = serde_json::from_slice(&std::fs::read("../assets/navcam_check.json").unwrap()).unwrap();
        let lift = lift_m(&check);
        assert!(lift > 0.0 && lift < 1.0, "measured offset {lift}");
        let (_, c) = mesh(&pts, lift).unwrap();
        let s: Value = serde_json::from_slice(&std::fs::read("../assets/stops.json").unwrap()).unwrap();
        let nev = s["stops"].as_array().unwrap().iter().find(|x| x["id"] == "nevado_sajama").unwrap();
        let d = ((c.0 - nev["e_m"].as_f64().unwrap() as f32).powi(2) + (c.1 - nev["n_m"].as_f64().unwrap() as f32).powi(2)).sqrt();
        assert!(d < 5.0, "cloud centre {d:.1} m from the Nevado Sajama stop");
    }

    #[test]
    fn detrend_removes_a_tilt() {
        let pts: Vec<[f32; 4]> = (0..100).map(|i| { let (e, n) = ((i % 10) as f32, (i / 10) as f32); [e, n, 0.3 * e - 0.2 * n + 5.0, 0.0] }).collect();
        assert!(detrend(&pts).iter().all(|r| r.abs() < 1e-4));
        let mut bump = pts.clone();
        bump[55][2] += 0.05;
        let r = detrend(&bump);
        assert!(r[55] > 0.04 && r.iter().enumerate().filter(|(i, _)| *i != 55).all(|(_, x)| x.abs() < 0.01));
    }

    #[test]
    fn shading_and_bad_input() {
        assert!((shade(0.0, 0.0, 1.0)[0] - 0.24f32.powf(2.2)).abs() < 1e-6);
        assert!((shade(1.0, 0.0, 1.0)[0] - 0.86f32.powf(2.2)).abs() < 1e-6);
        assert_eq!(shade(5.0, 1.0, 1.0)[3], 1.0);
        assert!(parse(&[0u8; 15]).is_empty());
        assert!(mesh(&[], 0.0).is_none());
    }
}
