//! Third-person view in Walk mode: two suited astronauts (EV1 = you, EV2 = your buddy) built from the NASA 3D Resources
//! "Astronaut" model. Format 2 (the earlier part split tore the arms): tools/astronaut.py gives ONE connected mesh with
//! smooth joint weights (torso, shoulders, hips), skinned on the GPU here (Bevy SkinnedMesh, as Bevy's custom_skinned_mesh
//! example), so a lowered or swinging arm bends the suit instead of tearing it. The walk is procedural (the model has no
//! animation): arms lowered from the T-pose and swung, legs swung at the hips, in step with the distance walked.
//! EV1 wears red leg stripes and EV2 a plain suit, as NASA marks the two crew on ISS spacewalks ("EV1 ... is wearing a suit
//! with red stripes", NASA space station blog, e.g. Mar 15 2022). EV2 walks the planned route BUDDY_GAP_M behind EV1 (never
//! beside it: off the route is unchecked ground). Key C switches first / third person.
use bevy::asset::RenderAssetUsages;
use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

use crate::scene::controls::{tapped, Act};
use crate::scene::terrain::Grid;
use crate::scene::walk::{Ground, Mode, Walker};

/// Arms come down this far from the model's T-pose (deg) and swing this much each way; legs swing LEG_SWING each way.
/// VISUAL choices.
pub const ARM_DOWN_DEG: f32 = 72.0;
pub const ARM_SWING_DEG: f32 = 18.0;
pub const LEG_SWING_DEG: f32 = 20.0;
/// One full gait cycle (two steps) per this many metres walked. VISUAL choice.
pub const CYCLE_M: f32 = 1.4;
/// EV2 walks this far behind EV1 along the planned route (m). VISUAL choice; NASA gives no separation rule we could read.
pub const BUDDY_GAP_M: f32 = 5.0;
/// Chase camera: distance behind, to the right of, and height above the walker; look-at height (m). Behind-right and high
/// enough that EV1 and EV2 (BUDDY_GAP_M behind on the route) are both in view. VISUAL.
pub const CHASE_BACK_M: f32 = 12.0;
pub const CHASE_SIDE_M: f32 = 3.0;
pub const CHASE_UP_M: f32 = 5.0;
pub const LOOK_AT_M: f32 = 0.8;
/// EV1's red leg stripes: heights above the soles (m) on leg vertices. VISUAL placement; the colour rule is NASA's (see top).
pub const STRIPES_M: [(f32, f32); 2] = [(0.50, 0.56), (0.22, 0.27)];
const STRIPE_RGB: [f32; 3] = [0.62, 0.05, 0.04];

#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    #[default]
    First,
    Third,
}

#[derive(Component)]
pub struct Suit {
    pub ev: u8,
    /// distance walked (m), drives the gait
    pub walked: f32,
    pub last: Option<(f32, f32)>,
}

#[derive(Component, Clone, Copy)]
pub enum Limb {
    ArmL,
    ArmR,
    LegL,
    LegR,
    Torso,
}

/// EV1's track (every TRACK_STEP_M), for EV2 to follow.
#[derive(Resource, Default)]
pub struct Track(pub Vec<(f32, f32)>);
const TRACK_STEP_M: f32 = 0.25;

/// Parsed astronaut.json + astronaut.bin (format 2): the mesh, per-vertex height and leg weight (for EV1's stripes), joint
/// pivots (m, from the soles) in the order torso, arm_l, arm_r, leg_l, leg_r.
pub struct SuitModel {
    pub mesh: Mesh,
    pub height: Vec<f32>,
    pub leg_w: Vec<f32>,
    pub colours: Vec<[f32; 4]>,
    pub pivots: Vec<Vec3>,
}

/// A joint of a suit's skeleton.
#[derive(Component, Clone, Copy)]
pub struct Joint(pub Limb);

const LIMBS: [Limb; 5] = [Limb::Torso, Limb::ArmL, Limb::ArmR, Limb::LegL, Limb::LegR];

fn f32s(b: &[u8], off: usize, n: usize) -> Option<Vec<f32>> {
    let s = b.get(off..off + 4 * n)?;
    Some(s.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
}

pub fn parse(meta: &serde_json::Value, bin: &[u8]) -> Option<SuitModel> {
    if meta["format"].as_u64() != Some(2) {
        return None;
    }
    let nv = meta["vertices"].as_u64()? as usize;
    let ni = meta["indices"].as_u64()? as usize;
    let at = |k: &str| meta[k].as_u64().map(|x| x as usize);
    let pos = f32s(bin, at("pos_offset")?, nv * 3)?;
    let nrm = f32s(bin, at("nrm_offset")?, nv * 3)?;
    let co = at("col_offset")?;
    let col = bin.get(co..co + 4 * nv)?;
    let jo = at("joints_offset")?;
    let joints: Vec<[u16; 4]> = bin.get(jo..jo + 8 * nv)?.chunks_exact(8)
        .map(|c| [u16::from_le_bytes([c[0], c[1]]), u16::from_le_bytes([c[2], c[3]]), u16::from_le_bytes([c[4], c[5]]), u16::from_le_bytes([c[6], c[7]])])
        .collect();
    let w = f32s(bin, at("weights_offset")?, nv * 4)?;
    let io = at("idx_offset")?;
    let idx: Vec<u32> = bin.get(io..io + 4 * ni)?.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    if idx.iter().any(|&i| i as usize >= nv) || joints.iter().flatten().any(|&j| j >= 5) {
        return None;
    }
    let pivots: Vec<Vec3> = meta["joints"].as_array()?.iter().filter_map(|j| {
        let p = j["pivot_m"].as_array()?;
        Some(Vec3::new(p[0].as_f64()? as f32, p[1].as_f64()? as f32, p[2].as_f64()? as f32))
    }).collect();
    if pivots.len() != 5 {
        return None;
    }
    let colours: Vec<[f32; 4]> = col.chunks_exact(4).map(|c| {
        let l = Color::srgba_u8(c[0], c[1], c[2], 255).to_linear();
        [l.red, l.green, l.blue, 1.0]
    }).collect();
    let weights: Vec<[f32; 4]> = w.chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect();
    let leg_w: Vec<f32> = joints.iter().zip(&weights).map(|(j, w)| (0..4).filter(|&k| j[k] == 3 || j[k] == 4).map(|k| w[k]).sum()).collect();
    let height: Vec<f32> = pos.chunks_exact(3).map(|v| v[1]).collect();
    let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos.chunks_exact(3).map(|v| [v[0], v[1], v[2]]).collect::<Vec<_>>());
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm.chunks_exact(3).map(|v| [v[0], v[1], v[2]]).collect::<Vec<_>>());
    m.insert_attribute(Mesh::ATTRIBUTE_COLOR, colours.clone());
    m.insert_attribute(Mesh::ATTRIBUTE_JOINT_INDEX, bevy::mesh::VertexAttributeValues::Uint16x4(joints));
    m.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, weights);
    m.insert_indices(Indices::U32(idx));
    Some(SuitModel { mesh: m, height, leg_w, colours, pivots })
}

/// Vertex colours with EV1's red leg stripes.
pub fn striped(m: &SuitModel) -> Vec<[f32; 4]> {
    m.colours.iter().zip(m.height.iter().zip(&m.leg_w)).map(|(c, (&y, &lw))| {
        if lw > 0.6 && STRIPES_M.iter().any(|&(a, b)| y >= a && y <= b) {
            [STRIPE_RGB[0], STRIPE_RGB[1], STRIPE_RGB[2], 1.0]
        } else {
            *c
        }
    }).collect()
}

/// Spawn EV1 and EV2 (hidden until Walk + third person / Walk): a root per suit, five joint entities under it, and the
/// skinned mesh (whose own transform is ignored by skinning).
pub fn spawn(commands: &mut Commands, meshes: &mut Assets<Mesh>, binds: &mut Assets<SkinnedMeshInverseBindposes>, model: SuitModel,
             start: (f32, f32), g: &Grid) {
    let mat = crate::scene::armat::SUIT_MAT.get().cloned().unwrap_or_default();
    let ibp = binds.add(model.pivots.iter().map(|p| Mat4::from_translation(-*p)).collect::<Vec<_>>());
    let mut ev1 = model.mesh.clone();
    ev1.insert_attribute(Mesh::ATTRIBUTE_COLOR, striped(&model));
    let ev2 = model.mesh.clone();
    let handles = [meshes.add(ev1), meshes.add(ev2)];
    for ev in [1u8, 2u8] {
        let (e, n) = (start.0 - if ev == 2 { 1.5 } else { 0.0 }, start.1);
        let root = commands
            .spawn((Suit { ev, walked: 0.0, last: None }, Transform::from_xyz(e, g.height_at(e, n), -n), Visibility::Hidden,
                    Name::new(format!("EV{ev}"))))
            .id();
        let joints: Vec<Entity> = LIMBS.iter().zip(&model.pivots).map(|(l, p)| {
            commands.spawn((Joint(*l), Transform::from_translation(*p), ChildOf(root))).id()
        }).collect();
        commands.spawn((Mesh3d(handles[ev as usize - 1].clone()), MeshMaterial3d(mat.clone()),
                        SkinnedMesh { inverse_bindposes: ibp.clone(), joints }, bevy::camera::visibility::NoFrustumCulling,
                        Transform::default(), ChildOf(root), Name::new(format!("EV{ev} suit"))));
    }
}

/// Model yaw for a compass heading (radians): the model faces +Z, world north is -Z.
pub fn yaw_for_heading(heading: f32) -> f32 {
    std::f32::consts::PI - heading
}

/// Limb rotation for a gait phase (radians). Left arm is the model's +X side.
pub fn limb_rotation(limb: Limb, phase: f32) -> Quat {
    let s = phase.sin();
    match limb {
        Limb::ArmL => Quat::from_rotation_x(-s * ARM_SWING_DEG.to_radians()) * Quat::from_rotation_z(-ARM_DOWN_DEG.to_radians()),
        Limb::ArmR => Quat::from_rotation_x(s * ARM_SWING_DEG.to_radians()) * Quat::from_rotation_z(ARM_DOWN_DEG.to_radians()),
        Limb::LegL => Quat::from_rotation_x(s * LEG_SWING_DEG.to_radians()),
        Limb::LegR => Quat::from_rotation_x(-s * LEG_SWING_DEG.to_radians()),
        Limb::Torso => Quat::IDENTITY,
    }
}

/// Point and heading on a track `back` metres behind its end (None if the track is shorter).
pub fn behind(track: &[(f32, f32)], back: f32) -> Option<((f32, f32), f32)> {
    let mut left = back;
    for i in (1..track.len()).rev() {
        let (a, b) = (track[i - 1], track[i]);
        let l = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        if l >= left && l > 0.0 {
            let t = 1.0 - left / l;
            let p = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
            return Some((p, (b.0 - a.0).atan2(b.1 - a.1)));
        }
        left -= l;
    }
    None
}

/// C toggles the view (Walk only).
pub fn toggle_view(keys: Res<ButtonInput<KeyCode>>, mode: Res<Mode>, mut view: ResMut<View>) {
    if *mode == Mode::Walk && tapped(&keys, Act::Camera) {
        *view = if *view == View::First { View::Third } else { View::First };
    }
}

/// Place EV1 at the walker and EV2 on EV1's track, animate both, show them per mode/view, and (third person) set the camera.
#[allow(clippy::too_many_arguments)]
pub fn update(
    mode: Res<Mode>,
    view: Res<View>,
    ground: Option<Res<Ground>>,
    walker: Option<Res<Walker>>,
    mut track: ResMut<Track>,
    mut suits: Query<(&mut Suit, &mut Transform, &mut Visibility, &Children), Without<Camera3d>>,
    mut limbs: Query<(&Joint, &mut Transform), (Without<Suit>, Without<Camera3d>)>,
    mut cams: Query<&mut Transform, (With<Camera3d>, Without<Suit>, Without<Joint>)>,
    run: Option<Res<crate::plan::simrun::SimRun>>,
    warm: Res<crate::scene::walk::WalkWarm>,
) {
    // while the simulation drives the walk, both suits are exact functions of its clock (scrubbing included)
    let simmed = run.as_ref().filter(|r| !r.manual).and_then(|r| r.sim.as_ref().map(|s| (s, r.t)));
    // EV2 = the same planned route, BUDDY_GAP_M of walking behind EV1 (exact: the sim's own time-at-distance)
    let ev2_t = simmed.map(|(sm, t)| sm.time_at_dist((sm.state(t).dist - BUDDY_GAP_M).max(0.0)));
    let (Some(ground), Some(w)) = (ground, walker) else { return };
    let g = &ground.0;
    let walk = *mode == Mode::Walk;
    // EV1's track
    if track.0.last().is_none_or(|l| ((l.0 - w.e).powi(2) + (l.1 - w.n).powi(2)).sqrt() >= TRACK_STEP_M) {
        track.0.push((w.e, w.n));
        if track.0.len() > 4000 {
            track.0.drain(..1000);
        }
    }
    for (mut s, mut t, mut vis, kids) in &mut suits {
        let from_sim = simmed.map(|(sm, t)| sm.state(if s.ev == 1 { t } else { ev2_t.unwrap_or(t) }));
        let ((e, n), heading) = if let Some(st) = from_sim.as_ref() {
            ((st.e, st.n), st.heading)
        } else if s.ev == 1 {
            ((w.e, w.n), w.heading)
        } else {
            match behind(&track.0, BUDDY_GAP_M) {
                Some(p) => p,
                None => match s.last {
                    Some(l) => (l, yaw_for_heading_inverse(t.rotation)),
                    None => ((w.e - 1.5, w.n), w.heading),
                },
            }
        };
        if let Some(st) = from_sim.as_ref() {
            s.walked = st.dist;
        } else if let Some(l) = s.last {
            s.walked += ((e - l.0).powi(2) + (n - l.1).powi(2)).sqrt();
        }
        s.last = Some((e, n));
        let phase = s.walked / CYCLE_M * std::f32::consts::TAU;
        let bob = 0.02 * (2.0 * phase).sin().abs();
        t.translation = Vec3::new(e, g.height_at(e, n) + bob, -n);
        t.rotation = Quat::from_rotation_y(yaw_for_heading(heading));
        let show = warm.0 || (walk && (s.ev == 2 || *view == View::Third));
        let want = if show { Visibility::Visible } else { Visibility::Hidden };
        if *vis != want {
            *vis = want;
        }
        for &k in kids {
            if let Ok((j, mut lt)) = limbs.get_mut(k) {
                lt.rotation = limb_rotation(j.0, phase);
            }
        }
    }
    if walk && *view == View::Third {
        let (sh, ch) = w.heading.sin_cos();
        let fwd = Vec3::new(sh, 0.0, -ch);
        let right = Vec3::new(ch, 0.0, sh);
        let me = Vec3::new(w.e, g.height_at(w.e, w.n), -w.n);
        // pitch tilts the camera up/down around the walker (drag up = look from higher)
        let up = CHASE_UP_M - w.pitch.sin() * CHASE_BACK_M;
        let mut cam = me - fwd * CHASE_BACK_M + right * CHASE_SIDE_M + Vec3::Y * up.max(0.6);
        let floor = g.height_at(cam.x, -cam.z) + 0.6;
        cam.y = cam.y.max(floor);
        for mut ct in &mut cams {
            *ct = Transform::from_translation(cam).looking_at(me + Vec3::Y * LOOK_AT_M, Vec3::Y);
        }
    }
}

fn yaw_for_heading_inverse(q: Quat) -> f32 {
    let (y, _, _) = q.to_euler(EulerRot::YXZ);
    std::f32::consts::PI - y
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_faces_the_walk_heading() {
        for h in [0.0f32, 90.0, 180.0, 270.0, 33.0] {
            let q = Quat::from_rotation_y(yaw_for_heading(h.to_radians()));
            let f = q * Vec3::Z; // the model's front
            let want = Vec3::new(h.to_radians().sin(), 0.0, -h.to_radians().cos());
            assert!((f - want).length() < 1e-4, "heading {h}: {f:?} vs {want:?}");
        }
    }

    #[test]
    fn arms_come_down_from_the_t_pose() {
        let tip_l = limb_rotation(Limb::ArmL, 0.0) * Vec3::X; // left arm points +X in the T-pose
        let tip_r = limb_rotation(Limb::ArmR, 0.0) * Vec3::NEG_X;
        assert!(tip_l.y < -0.9 && tip_r.y < -0.9, "{tip_l:?} {tip_r:?}");
        // legs swing in opposition
        let l = limb_rotation(Limb::LegL, 1.0) * Vec3::NEG_Y;
        let r = limb_rotation(Limb::LegR, 1.0) * Vec3::NEG_Y;
        assert!(l.z * r.z < 0.0, "legs swing opposite: {l:?} {r:?}");
    }

    #[test]
    fn buddy_point_is_the_gap_behind_along_the_track() {
        let track: Vec<(f32, f32)> = (0..=40).map(|i| (i as f32 * 0.25, 0.0)).collect(); // 10 m east
        let ((e, n), h) = behind(&track, 5.0).unwrap();
        assert!((e - 5.0).abs() < 1e-4 && n.abs() < 1e-6, "{e} {n}");
        assert!((h - std::f32::consts::FRAC_PI_2).abs() < 1e-4, "heading east");
        assert!(behind(&track, 50.0).is_none());
    }

    #[test]
    fn parses_the_real_bundle_model() {
        let meta: serde_json::Value = serde_json::from_slice(&std::fs::read("../assets/astronaut.json").unwrap()).unwrap();
        let bin = std::fs::read("../assets/astronaut.bin").unwrap();
        let m = parse(&meta, &bin).expect("skinned suit");
        assert_eq!(m.pivots.len(), 5);
        assert!(m.mesh.attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT).is_some() && m.mesh.attribute(Mesh::ATTRIBUTE_JOINT_INDEX).is_some());
        assert!(parse(&meta, &bin[..1000]).is_none(), "a short file is refused");
        // EV1's stripes: some leg vertices turn red, no torso or arm vertex does
        let st = striped(&m);
        let red: Vec<usize> = (0..st.len()).filter(|&i| st[i] != m.colours[i]).collect();
        assert!(red.len() > 20, "{}", red.len());
        assert!(red.iter().all(|&i| m.leg_w[i] > 0.6 && m.height[i] < 0.6));
    }
}
