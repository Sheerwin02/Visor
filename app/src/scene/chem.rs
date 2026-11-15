//! Chemistry layer (Plan mode): Curiosity's ChemCam and APXS results along the walk (chemistry.json, tools/chem.py), one
//! dot per target at the rover's position when it measured (a target can be a few metres from its dot). Colour = CaO wt%
//! in five steps (the calcium-sulfate nodules and veins of the boxwork are calcium-rich); APXS dots are larger, so the
//! instrument is not told by colour alone. Off by default; the rail and key - toggle it.
use bevy::prelude::*;

use crate::scene::controls::{tapped, Act};
use crate::scene::rover_path::RoverDot;
use crate::scene::terrain::Grid;

#[derive(Component)]
pub struct ChemLayer;

/// CaO class limits (wt%) and colours, light to dark (VISUAL choice; the legend is in the rail tooltip).
pub const CAO_STEPS: [f64; 4] = [3.0, 6.0, 10.0, 20.0];
pub const CAO_COLOURS: [(u8, u8, u8); 5] = [(255, 245, 184), (255, 209, 102), (244, 160, 60), (224, 99, 45), (184, 36, 42)];

pub fn cao_class(cao: f64) -> usize {
    CAO_STEPS.iter().take_while(|&&s| cao >= s).count()
}

/// One record as the layer needs it: east, north, CaO (wt%), APXS.
pub fn points(doc: &serde_json::Value) -> Vec<(f32, f32, f64, bool)> {
    doc["records"].as_array().into_iter().flatten().filter_map(|r| Some((
        r["e_m"].as_f64()? as f32, r["n_m"].as_f64()? as f32, r["oxides"]["CaO"].as_f64()?, r["instrument"] == "APXS",
    ))).collect()
}

pub fn spawn(commands: &mut Commands, meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>, grid: &Grid,
             pts: &[(f32, f32, f64, bool)]) {
    let mats: Vec<Handle<StandardMaterial>> = CAO_COLOURS.iter().map(|&(r, g, b)| materials.add(StandardMaterial {
        base_color: Color::srgb_u8(r, g, b), unlit: true, ..default() })).collect();
    let (dot, big) = (meshes.add(Sphere::new(1.0)), meshes.add(Sphere::new(1.8)));
    commands.spawn((ChemLayer, Transform::default(), Visibility::Hidden, Name::new("layer: chemistry"))).with_children(|p| {
        for &(e, n, cao, apxs) in pts {
            let at = Transform::from_xyz(e, grid.height_at(e, n) + 1.2, -n);
            p.spawn((RoverDot, Mesh3d(if apxs { big.clone() } else { dot.clone() }), MeshMaterial3d(mats[cao_class(cao)].clone()), at,
                     crate::scene::layers::PlanOnly, Visibility::Inherited));
        }
    });
}

pub fn toggle(keys: Res<ButtonInput<KeyCode>>, mut q: Query<&mut Visibility, With<ChemLayer>>) {
    if tapped(&keys, Act::LayerChem) {
        for mut v in &mut q {
            *v = if *v == Visibility::Hidden { Visibility::Visible } else { Visibility::Hidden };
        }
    }
}

/// Targets measured near a point: within `r` m of (e, n), APXS first, then by distance; at most `max`. Each row: instrument,
/// target, sol, distance (m), CaO, SiO2, FeO (APXS FeO or ChemCam FeOT), SO3 (APXS only).
#[allow(clippy::type_complexity)]
pub fn near(doc: &serde_json::Value, e: f32, n: f32, r: f32, max: usize)
            -> (Vec<(String, String, i64, f32, Option<f64>, Option<f64>, Option<f64>, Option<f64>)>, usize) {
    let mut v: Vec<_> = doc["records"].as_array().into_iter().flatten().filter_map(|x| {
        let d = ((x["e_m"].as_f64()? as f32 - e).powi(2) + (x["n_m"].as_f64()? as f32 - n).powi(2)).sqrt();
        (d <= r).then(|| {
            let o = &x["oxides"];
            (x["instrument"].as_str().unwrap_or("").to_string(), x["target"].as_str().unwrap_or("").replace('_', " "), x["sol"].as_i64().unwrap_or(0), d,
             o["CaO"].as_f64(), o["SiO2"].as_f64(), o["FeO"].as_f64().or(o["FeOT"].as_f64()), o["SO3"].as_f64())
        })
    }).collect();
    v.sort_by(|a, b| (a.0 != "APXS").cmp(&(b.0 != "APXS")).then(a.3.total_cmp(&b.3)));
    let total = v.len();
    v.truncate(max);
    (v, total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_and_nearby_targets_from_the_bundle() {
        assert_eq!((cao_class(1.0), cao_class(3.0), cao_class(9.9), cao_class(35.0)), (0, 1, 2, 4));
        let d: serde_json::Value = serde_json::from_slice(&std::fs::read("../assets/chemistry.json").unwrap()).unwrap();
        assert!(points(&d).len() > 100);
        let s: serde_json::Value = serde_json::from_slice(&std::fs::read("../assets/stops.json").unwrap()).unwrap();
        let rio = s["stops"].as_array().unwrap().iter().find(|x| x["id"] == "rio_frio").unwrap();
        let (rows, total) = near(&d, rio["e_m"].as_f64().unwrap() as f32, rio["n_m"].as_f64().unwrap() as f32, 8.0, 6);
        assert!(total >= 1 && rows.iter().any(|r| r.1 == "El Tapado"), "{rows:?}");
    }
}
