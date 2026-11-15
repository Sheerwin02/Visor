//! Link between the planning panel's profile (panel.rs) and the 3D map: hovering the profile sets panel::HOVER and a ring
//! (#profile-pin) marks that point on the map; clicking sets panel::GOTO and the map (or the 3D view) centres there.
use bevy::prelude::*;

use crate::scene::camera::Orbit;
use crate::scene::mapview::{MapCam, PlanView};
use crate::ui::chrome::style;
use crate::ui::panel;
use crate::scene::walk::{Ground, Mode};

#[derive(Resource, Default)]
pub struct PinCache(pub String);

pub fn update_pin(
    cams: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    mode: Res<Mode>,
    ground: Option<Res<Ground>>,
    mut cache: ResMut<PinCache>,
) {
    let (Ok((cam, cam_t)), Some(ground)) = (cams.single(), ground) else { return };
    let css = match panel::HOVER.lock().ok().and_then(|g| *g) {
        Some((e, n)) if *mode == Mode::Plan => {
            let p = Vec3::new(e, ground.0.height_at(e, n), -n);
            match cam.world_to_viewport(cam_t, p) {
                Ok(v) => format!("left:{}px;top:{}px", v.x.round() as i32, v.y.round() as i32),
                Err(_) => "display:none".into(),
            }
        }
        _ => "display:none".into(),
    };
    if cache.0 != css {
        style("profile-pin", &css);
        cache.0 = css;
    }
}

pub fn goto(view: Res<PlanView>, ground: Option<Res<Ground>>, map: Option<ResMut<MapCam>>, mut orbits: Query<&mut Orbit>) {
    let Some((e, n)) = panel::GOTO.lock().ok().and_then(|mut g| g.take()) else { return };
    match *view {
        PlanView::Map => {
            if let Some(mut m) = map {
                m.e = e;
                m.n = n;
                m.user_moved = true;
            }
        }
        PlanView::Orbit => {
            if let Ok(mut o) = orbits.single_mut() {
                o.focus = Vec3::new(e, ground.as_ref().map(|g| g.0.height_at(e, n)).unwrap_or(o.focus.y), -n);
            }
        }
    }
}
