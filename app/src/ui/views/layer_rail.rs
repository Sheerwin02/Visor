//! Layer rail (left): map layer switches with their keys and sources, and the Map/3D view switch.

use crate::scene::controls::{ui_press, Act};
use crate::ui::model::{LayersModel, PanelModel};
use crate::text::{fill, ui as u};

use super::*;

#[component]
pub(super) fn LayerRail(layers: RwSignal<LayersModel>, panel: RwSignal<Option<PanelModel>>, mars: RwSignal<bool>) -> impl IntoView {
    let row = move |on: bool, act: Option<Act>, label: &'static str, src: String, key: &'static str, tip: String, swatch: AnyView| view! {
        <button type="button" class="layer" role="switch" aria-checked=if on { "true" } else { "false" } title=format!("{src}. {tip}")
            disabled=act.is_none() on:click=move |_| { if let Some(a) = act { ui_press(a) } }>
            <span class="lsw">{swatch}</span>
            <span class="lname">{label}<small>{src.clone()}</small></span>
            <kbd>{key}</kbd>
        </button>
    };
    move || {
        let l = layers.get();
        let vals = panel.get().map(|m| m.vals).unwrap_or_default();
        let runs = panel.get().map(|m| runs_of(&m)).unwrap_or_default();
        let v = |k: &str| val(&vals, k);
        let sw = |cls: &'static str| view! { <i class=cls></i> }.into_any();
        view! {
            <div class="rail-title">{u::LAYERS}</div>
            <div class="lgroup">{u::G_BASE}</div>
            {if l.basemaps.is_empty() {
                row(true, None, u::L_PHOTO, u::S_MRO.into(), "", String::new(), sw("sw-photo")).into_any()
            } else {
                view! {
                    <div class="basemaps" role="radiogroup" aria-label=u::G_BASE>
                        {l.basemaps.iter().enumerate().map(|(i, (title, detail, tip, on, loading))| view! {
                            <button type="button" class="layer basemap" role="radio" aria-checked=if *on { "true" } else { "false" } title=format!("{detail}. {tip}")
                                on:click=move |_| crate::scene::basemap::select(i)>
                                <span class="lsw"><i class=if i == 0 { "sw-photo" } else { "sw-trek" }></i></span>
                                <span class="lname">{title.clone()}<small>{if *loading { format!("{detail} \u{2026}") } else { detail.clone() }}</small></span>
                            </button>
                        }).collect_view()}
                        {(!l.basemap_error.is_empty()).then(|| view! { <p class="fine warn">{l.basemap_error.clone()}</p> })}
                    </div>
                }.into_any()
            }}
            <div class="lgroup">{u::G_PLAN}</div>
            {row(l.route, Some(Act::LayerRoute), u::L_ROUTE, String::new(), "5", String::new(), sw("sw-route"))}
            {row(l.stops, Some(Act::LayerStops), u::L_STOPS, String::new(), "6", String::new(), sw("sw-flag"))}
            {row(l.path, Some(Act::LayerRover), u::L_PATH, u::S_MSL.into(), "7", String::new(), sw("sw-rover"))}
            {row(l.back, Some(Act::LayerBack), u::L_BACK, u::S_PLAN.into(), "=", u::T_BACK.into(), sw("sw-back"))}
            <div class="lgroup">{u::G_TERRAIN}</div>
            {row(l.slope, Some(Act::LayerSlope), u::L_SLOPE, u::S_DEM.into(), "2",
                 fill(u::T_SLOPE, &[("caution_deg", v("caution_deg")), ("slope_limit_deg", v("slope_limit_deg"))]), sw("sw-hatch"))}
            // specialist layers folded (the rail was crowded); open by itself when one of them is on
            <details class="more-layers" open=l.checks || l.verified || l.chem || l.agreement || l.ready || l.shadows>
            <summary>{u::MORE_LAYERS}</summary>
            <div class="lgroup">{u::G_ROVER}</div>
            {row(l.checks, Some(Act::LayerChecks), u::L_CHECKS, u::S_MSL.into(), "3", u::T_CHECKS.into(), sw("sw-check"))}
            {row(l.verified, Some(Act::LayerVerified), u::L_VERIFIED, u::S_MSL.into(), "4",
                 fill(u::T_VERIFIED, &[("verified_radius_m", v("verified_radius_m"))]), sw("sw-dots"))}
            {l.has_chem.then(|| row(l.chem, Some(Act::LayerChem), u::L_CHEM, u::S_CHEM.into(), "-", u::T_CHEM.into(), sw("sw-chem")))}
            <div class="lgroup">{u::G_TRUST}</div>
            {row(l.agreement, Some(Act::LayerDensity), u::L_AGREE, fill(u::S_RUNS, &[("runs", runs.clone())]), "8",
                 fill(u::T_AGREE, &[("runs", runs)]), sw("sw-density"))}
            {l.has_ready.then(|| row(l.ready, Some(Act::LayerReady), u::L_READY, u::S_READY.into(), "0", l.ready_tip.clone(), sw("sw-ready")))}
            <div class="lgroup">{u::G_SUN}</div>
            {row(l.shadows, Some(Act::LayerShadow), u::L_SHADOW, u::S_SUN.into(), "9", u::T_SHADOW.into(), sw("sw-shadow"))}
            </details>
            <div class="rail-title view-title">{u::VIEW}</div>
            <div class="seg view-seg">
                <button type="button" title=crate::text::mars::T_VIEW_MARS aria-pressed=move || if mars.get() { "true" } else { "false" }
                    on:click=move |_| mars.set(true)>{crate::text::mars::VIEW_MARS}</button>
                <button type="button" aria-pressed=move || if !mars.get() && l.map_view { "true" } else { "false" }
                    on:click=move |_| { mars.set(false); if !l.map_view { ui_press(Act::ToggleView) } }>{u::VIEW_MAP}</button>
                <button type="button" aria-pressed=move || if !mars.get() && !l.map_view { "true" } else { "false" }
                    on:click=move |_| { mars.set(false); if l.map_view { ui_press(Act::ToggleView) } }>{u::VIEW_3D}</button>
            </div>
            <div class="data-used"><div class="rail-title">{u::DATA_USED}</div><p>{u::DATA_LIST}</p></div>
        }
    }
}

// ================================================================= inspector
