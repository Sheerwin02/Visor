//! The left rail's layer switches: publishes which map layers are on (whether toggled by a key or by the rail, which
//! presses the same keys through controls::ui_press), so the checkboxes always show the map's real state.
use bevy::prelude::*;

use crate::scene::checks::Checks;
use crate::scene::layers::{RouteLayer, StopsLayer};
use crate::scene::mapview::PlanView;
use crate::ui::model::{LayersModel, UiBridge};
use crate::scene::rover_path::RoverPathLayer;
use crate::scene::trust::Overlays;
use crate::scene::walk::Mode;

#[allow(clippy::too_many_arguments)]
pub fn update(
    mode: Res<Mode>,
    view: Res<PlanView>,
    ov: Option<Res<Overlays>>,
    docs: Option<Res<crate::BundleDocs>>,
    bm: Option<Res<crate::scene::basemap::Basemaps>>,
    checks: Option<Res<Checks>>,
    route: Query<&Visibility, With<RouteLayer>>,
    stops: Query<&Visibility, (With<StopsLayer>, Without<RouteLayer>)>,
    path: Query<&Visibility, (With<RoverPathLayer>, Without<RouteLayer>, Without<StopsLayer>)>,
    chem: Query<&Visibility, (With<crate::scene::chem::ChemLayer>, Without<RoverPathLayer>, Without<RouteLayer>, Without<StopsLayer>)>,
    mut bridge: ResMut<UiBridge>,
) {
    let vis = |v: Option<&Visibility>| v.is_some_and(|v| *v != Visibility::Hidden);
    let m = if *mode == Mode::Walk { 1 } else { 0 };
    let show = ov.as_ref().map(|o| o.show[m]).unwrap_or_default();
    bridge.set_layers(LayersModel {
        walking: *mode == Mode::Walk,
        map_view: *view == PlanView::Map,
        route: vis(route.iter().next()),
        stops: vis(stops.iter().next()),
        path: vis(path.iter().next()),
        chem: vis(chem.iter().next()),
        has_chem: chem.iter().next().is_some(),
        back: ov.as_ref().is_some_and(|o| o.back_on && o.has_back),
        slope: show[0],
        checks: checks.as_ref().is_some_and(|c| c.shown),
        verified: show[1],
        agreement: show[2],
        shadows: show[3],
        has_shadows: ov.as_ref().is_some_and(|o| o.has_shadow),
        ready: show[4],
        has_ready: ov.as_ref().is_some_and(|o| o.has_ready),
        ready_tip: docs.as_ref().and_then(|d| d.readiness.as_ref()).map(|r| crate::text::fill(crate::text::ui::T_READY, &[
            ("a", format!("{}", r["thresholds_deg"]["agree"].as_f64().unwrap_or(f64::NAN))),
            ("b", format!("{}", r["thresholds_deg"]["partial"].as_f64().unwrap_or(f64::NAN))),
            ("outcome", r["verdict"].as_str().unwrap_or("").to_string())])).unwrap_or_default(),
        basemaps: bm.as_ref().map(|b| b.choices.iter().enumerate().map(|(i, c)| (c.title.clone(), c.detail.clone(), c.tip.clone(),
            i == b.selected, c.file.is_some() && b.pending.as_deref() == c.file.as_deref())).collect()).unwrap_or_default(),
        basemap_error: bm.as_ref().and_then(|b| b.error.clone()).unwrap_or_default(),
    });
}
