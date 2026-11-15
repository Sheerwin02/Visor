//! Curiosity in Walk mode: the NASA Science 3D model (assets/curiosity.bin, baked by
//! tools/curiosity.py) parked where the rover actually stood at each stop: the PLACES position, heading (yaw), pitch and roll
//! for the stop's sol/site/drive (rover_check.json, augmented from PLACES by build_bundle rehash). One rover per stop, each
//! tagged with its sol: a record of the past ("parked here on sol N"), not a live position, and the tag says so.
//! Within GHOST_M of the rover it turns into a see-through AR ghost so the crew can walk onto the spot it drilled.
//! The model faces +Z (arm end), Y up, origin at the centre of the wheel contacts.
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use serde_json::Value;

use crate::scene::terrain::Grid;
use crate::scene::walk::{Ground, Mode, Walker};

/// Closer than this (m, walker to rover origin) the rover is drawn as a see-through ghost. VISUAL choice.
pub const GHOST_M: f32 = 6.0;
/// Closer than this (m) the walker stands inside the rover's 3.0 x 2.8 m footprint (half diagonal ~2.05 m): no rover is
/// drawn, so the first-person camera is not inside the model. VISUAL choice.
pub const INSIDE_M: f32 = 2.1;

/// Which rover to draw for one pose: (solid, ghost). Only the rover at the stop being walked to, or one within GHOST_M
/// (the stop just reached), is drawn: one rover per stop on screen at once read as a fleet. Inside the footprint: none.
pub fn shown(pose_stop: usize, next_stop: usize, dist_m: f32) -> (bool, bool) {
    if dist_m < INSIDE_M {
        return (false, false);
    }
    let close = dist_m < GHOST_M;
    let relevant = pose_stop == next_stop || close;
    (relevant && !close, relevant && close)
}

/// Tag height above the rover's origin (m): above the mast head (2.2 m in the model).
const TAG_UP_M: f32 = 2.7;

#[derive(Clone, Debug, PartialEq)]
pub struct Pose {
    pub stop: usize,
    pub sol: i64,
    pub e: f32,
    pub n: f32,
    pub yaw_deg: f32,
    pub pitch_deg: f32,
    pub roll_deg: f32,
    /// the stop's position is inferred (stops.json position_note starts with INFERRED)
    pub inferred: bool,
}

#[derive(Resource, Default)]
pub struct Poses(pub Vec<Pose>);

/// The rover shown in Plan 3D at the selected Curiosity stop (scene/select.rs), at its PLACES pose.
#[derive(Component)]
pub struct SelRover;

#[derive(Component)]
pub struct Rover {
    /// index into Poses
    pub pose: usize,
    pub ghost: bool,
}

pub fn parse(meta: &Value, bin: &[u8]) -> Option<Mesh> {
    let nv = meta["vertices"].as_u64()? as usize;
    let ni = meta["indices"].as_u64()? as usize;
    let at = |k: &str| meta[k].as_u64().map(|x| x as usize);
    let f = |off: usize, n: usize| -> Option<Vec<f32>> {
        Some(bin.get(off..off + 4 * n)?.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
    };
    let pos = f(at("pos_offset")?, nv * 3)?;
    let nrm = f(at("nrm_offset")?, nv * 3)?;
    let co = at("col_offset")?;
    let col = bin.get(co..co + 4 * nv)?;
    let io = at("idx_offset")?;
    let idx: Vec<u32> = bin.get(io..io + 4 * ni)?.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    if idx.iter().any(|&i| i as usize >= nv) {
        return None;
    }
    let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos.chunks_exact(3).map(|v| [v[0], v[1], v[2]]).collect::<Vec<_>>());
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm.chunks_exact(3).map(|v| [v[0], v[1], v[2]]).collect::<Vec<_>>());
    m.insert_attribute(Mesh::ATTRIBUTE_COLOR, col.chunks_exact(4).map(|c| {
        let l = Color::srgba_u8(c[0], c[1], c[2], 255).to_linear();
        [l.red, l.green, l.blue, 1.0]
    }).collect::<Vec<_>>());
    m.insert_indices(Indices::U32(idx));
    Some(m)
}

/// The rover's recorded pose at each stop: the rover-check stop with the same sol, site and drive.
pub fn poses(stops_doc: &Value, rover_check: &Value) -> Vec<Pose> {
    let rc = rover_check["stops"].as_array().cloned().unwrap_or_default();
    stops_doc["stops"].as_array().map(|ss| ss.iter().enumerate().filter_map(|(i, s)| {
        let r = rc.iter().find(|r| r["sol"] == s["sol"] && r["site"] == s["site"] && r["drive"] == s["drive"])?;
        Some(Pose {
            stop: i,
            sol: r["sol"].as_i64()?,
            e: r["e_m"].as_f64()? as f32,
            n: r["n_m"].as_f64()? as f32,
            yaw_deg: r["yaw_deg"].as_f64()? as f32,
            pitch_deg: r["pitch_deg"].as_f64()? as f32,
            roll_deg: r["roll_deg"].as_f64()? as f32,
            inferred: s["position_note"].as_str().is_some_and(|x| x.starts_with("INFERRED")),
        })
    }).collect()).unwrap_or_default()
}

/// Model rotation for a PLACES pose: yaw = heading (N 0, E 90), pitch + = nose up, roll + = left side up (PLACES SIS).
/// Yaw about up, then pitch about the rover's lateral axis, then roll about its forward axis. Model: front +Z, left +X.
pub fn rotation(yaw_deg: f32, pitch_deg: f32, roll_deg: f32) -> Quat {
    Quat::from_rotation_y(crate::scene::astronaut::yaw_for_heading(yaw_deg.to_radians()))
        * Quat::from_rotation_x(-pitch_deg.to_radians())
        * Quat::from_rotation_z(roll_deg.to_radians())
}

fn tag_id(i: usize) -> String {
    format!("rover-tag-{i}")
}

/// Spawn one rover per recorded pose, twice each (solid, and an AR ghost for close range), hidden until Walk, at its pose.
pub fn spawn(commands: &mut Commands, meshes: &mut Assets<Mesh>, ar: &mut Assets<crate::scene::armat::ArMaterial>, mesh: Mesh,
             poses: &[Pose], g: &Grid) {
    // the ghost uses the AR material, whose meshes carry position + colour only (the same vertex layout as every AR overlay;
    // with normals added it did not draw, checked)
    let mut gm = mesh.clone();
    gm.remove_attribute(Mesh::ATTRIBUTE_NORMAL);
    let gh = meshes.add(gm);
    let h = meshes.add(mesh);
    let solid = crate::scene::armat::SUIT_MAT.get().cloned().unwrap_or_default();
    let ghost = ar.add(crate::scene::armat::ArMaterial { tint: Vec4::new(0.75, 0.95, 1.0, 0.45), fade: Vec4::ZERO });
    // Plan: one more copy for the selected stop, placed by select::show_rover
    commands.spawn((Mesh3d(h.clone()), MeshMaterial3d(solid.clone()), Transform::default(), Visibility::Hidden, SelRover,
                    Name::new("Curiosity at the selected stop")));
    for (i, p) in poses.iter().enumerate() {
        let t = Transform::from_xyz(p.e, g.height_at(p.e, p.n), -p.n).with_rotation(rotation(p.yaw_deg, p.pitch_deg, p.roll_deg));
        commands.spawn((Mesh3d(h.clone()), MeshMaterial3d(solid.clone()), t, Visibility::Hidden, Rover { pose: i, ghost: false },
                        Name::new(format!("Curiosity, sol {}", p.sol))));
        commands.spawn((Mesh3d(gh.clone()), MeshMaterial3d(ghost.clone()), t, Visibility::Hidden, Rover { pose: i, ghost: true },
                        bevy::light::NotShadowCaster, Name::new(format!("Curiosity (ghost), sol {}", p.sol))));
        #[cfg(target_arch = "wasm32")]
        if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
            if doc.get_element_by_id(&tag_id(i)).is_none() {
                if let (Ok(el), Some(body)) = (doc.create_element("div"), doc.body()) {
                    el.set_id(&tag_id(i));
                    el.set_class_name("rover-tag");
                    el.set_text_content(Some(&crate::text::fill(if p.inferred { crate::text::rover::TAG_INFERRED } else { crate::text::rover::TAG },
                                                                &[("sol", p.sol.to_string())])));
                    let _ = el.set_attribute("title", crate::text::rover::TIP);
                    let _ = el.set_attribute("style", "display:none");
                    let _ = body.append_child(&el);
                }
            }
        }
    }
}

/// Line of sight from `from` to `to` over the terrain (the tag of a rover behind a ridge is hidden): the straight line may
/// not pass below the ground at any of 32 samples (0.3 m tolerance).
pub fn visible(g: &Grid, from: Vec3, to: Vec3) -> bool {
    (1..32).all(|k| {
        let p = from.lerp(to, k as f32 / 32.0);
        p.y >= g.height_at(p.x, -p.z) - 0.3
    })
}

/// Tags further than this (m) from the walker are not shown (the stop plates already say where the stops are). VISUAL.
const TAG_RANGE_M: f32 = 30.0;

#[allow(clippy::too_many_arguments)]
pub fn update(
    mode: Res<Mode>,
    poses: Res<Poses>,
    ground: Option<Res<Ground>>,
    walker: Option<Res<Walker>>,
    stops: Res<crate::scene::walk::Stops>,
    warm: Res<crate::scene::walk::WalkWarm>,
    mut rovers: Query<(&Rover, &mut Visibility)>,
    cams: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    mut last: Local<Vec<String>>,
) {
    let (Some(ground), Some(w)) = (ground, walker) else { return };
    let g: &Grid = &ground.0;
    let walk = *mode == Mode::Walk;
    let dist: Vec<f32> = poses.0.iter().map(|p| ((p.e - w.e).powi(2) + (p.n - w.n).powi(2)).sqrt()).collect();
    let next = if stops.0.is_empty() { 0 } else { w.next % stops.0.len() };
    let on: Vec<(bool, bool)> = poses.0.iter().zip(&dist).map(|(p, &d)| shown(p.stop, next, d)).collect();
    for (r, mut vis) in &mut rovers {
        let (solid, ghost) = on.get(r.pose).copied().unwrap_or((false, false));
        let show = warm.0 || (walk && if r.ghost { ghost } else { solid });
        let want = if show { Visibility::Visible } else { Visibility::Hidden };
        if *vis != want {
            *vis = want;
        }
    }
    if last.len() != poses.0.len() {
        *last = vec![String::new(); poses.0.len()];
    }
    let cam = cams.single().ok();
    for (i, p) in poses.0.iter().enumerate() {
        let mut css = "display:none".to_string();
        if let (true, Some((cam, ct))) = (walk && dist[i] < TAG_RANGE_M && (on[i].0 || on[i].1), cam) {
            let top = Vec3::new(p.e, g.height_at(p.e, p.n) + TAG_UP_M, -p.n);
            if (top - ct.translation()).dot(*ct.forward()) > 0.0 && visible(g, ct.translation(), top) {
                if let Ok(s) = cam.world_to_viewport(ct, top) {
                    css = format!("left:{}px;top:{}px", s.x.round() as i32, s.y.round() as i32);
                }
            }
        }
        if last[i] != css {
            crate::ui::chrome::style(&tag_id(i), &css);
            last[i] = css;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_rover_at_a_time_and_none_around_the_walker() {
        assert_eq!(shown(5, 5, 80.0), (true, false), "the stop being walked to: solid");
        assert_eq!(shown(3, 5, 80.0), (false, false), "other stops: hidden");
        assert_eq!(shown(4, 5, 4.0), (false, true), "the stop just reached, close: ghost");
        assert_eq!(shown(5, 5, 1.0), (false, false), "inside the footprint: nothing");
    }

    fn read(name: &str) -> Value {
        serde_json::from_slice(&std::fs::read(format!("../assets/{name}")).unwrap()).unwrap()
    }

    #[test]
    fn every_stop_has_its_recorded_pose() {
        let p = poses(&read("stops.json"), &read("rover_check.json"));
        let stops = read("stops.json");
        assert_eq!(p.len(), stops["stops"].as_array().unwrap().len(), "{p:?}");
        let s0 = &stops["stops"][0];
        assert!((p[0].e as f64 - s0["e_m"].as_f64().unwrap()).abs() < 0.01, "pose = the stop's own PLACES position");
        assert!(p.iter().any(|x| x.inferred), "Valle de la Luna is inferred");
    }

    #[test]
    fn pose_rotation_follows_places_conventions() {
        let f = rotation(90.0, 0.0, 0.0) * Vec3::Z; // heading east: front points +x (east)
        assert!((f - Vec3::X).length() < 1e-4, "{f:?}");
        let up = rotation(0.0, 10.0, 0.0) * Vec3::Z; // nose up
        assert!(up.y > 0.17, "{up:?}");
        let left = rotation(0.0, 0.0, 10.0) * Vec3::X; // left side up
        assert!(left.y > 0.17, "{left:?}");
    }

    #[test]
    fn parses_the_real_bundle_model() {
        let m = parse(&read("curiosity.json"), &std::fs::read("../assets/curiosity.bin").unwrap()).expect("rover mesh");
        assert!(m.count_vertices() > 10_000);
    }
}
