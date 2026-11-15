//! The HTML interface as Leptos components (web only), laid out like NASA planning tools (Worldview, JMARS, xGDS/Playbook):
//! a mission bar on top (when, verdict, key numbers), map layers on the left, an inspector on the right (plan, stops, sun,
//! trust, about; a stop's card opens in it), and the EVA timeline along the bottom (stops, terrain and sun on one axis).
//! Components render the models in model.rs (published by Bevy through UiBridge) and send actions back through the
//! planner/card/light event queues, the profile hover/goto statics, the mode-button flag and controls::ui_press.
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::ui::brief::BriefModel;
use crate::ui::model::{CardModel, ChromeModel, LayersModel, LightModel, PanelModel, TopBarModel};

mod mission_bar;
mod layer_rail;
mod inspector;
mod stop_card;
mod timeline;
mod walk_bar;
mod marsview;
mod drill_card;
use drill_card::*;
use mission_bar::*;
use layer_rail::*;
use inspector::*;
use stop_card::*;
use timeline::*;
use walk_bar::*;
use marsview::*;

/// Signal handles kept by Bevy (UiBridge) to publish new models.
#[derive(Clone, Copy)]
pub struct Signals {
    panel: RwSignal<Option<PanelModel>>,
    top: RwSignal<TopBarModel>,
    chrome: RwSignal<ChromeModel>,
    card: RwSignal<CardModel>,
    light: RwSignal<LightModel>,
    layers: RwSignal<LayersModel>,
    brief: RwSignal<BriefModel>,
    crew: RwSignal<crate::plan::crew::CrewModel>,
    sim: RwSignal<crate::plan::simrun::SimModel>,
    /// Mars view open; overview.json and missions.json (fetched by the UI itself at start)
    mars: RwSignal<bool>,
    overview: RwSignal<Option<std::sync::Arc<crate::ui::mars::Overview>>>,
    missions: RwSignal<Vec<crate::ui::mars::Mission>>,
    drill: RwSignal<crate::ui::model::DrillModel>,
    placed: RwSignal<crate::scene::place::PlacedModel>,
}

impl Signals {
    pub fn set_panel(&self, m: PanelModel) {
        self.panel.set(Some(m));
    }
    pub fn set_top(&self, m: TopBarModel) {
        self.top.set(m);
    }
    pub fn set_chrome(&self, m: ChromeModel) {
        self.chrome.set(m);
    }
    pub fn set_card(&self, m: CardModel) {
        self.card.set(m);
    }
    pub fn set_light(&self, m: LightModel) {
        self.light.set(m);
    }
    pub fn set_sim(&self, m: crate::plan::simrun::SimModel) {
        self.sim.set(m);
    }
    pub fn set_crew(&self, m: crate::plan::crew::CrewModel) {
        self.crew.set(m);
    }
    pub fn set_brief(&self, m: BriefModel) {
        self.brief.set(m);
    }
    pub fn set_placed(&self, m: crate::scene::place::PlacedModel) {
        self.placed.set(m);
    }
    pub fn set_drill(&self, m: crate::ui::model::DrillModel) {
        self.drill.set(m);
    }
    pub fn set_layers(&self, m: LayersModel) {
        self.layers.set(m);
    }
}

fn host(id: &str) -> Option<web_sys::HtmlElement> {
    web_sys::window()?.document()?.get_element_by_id(id)?.dyn_into::<web_sys::HtmlElement>().ok()
}

/// Mount the components into their host elements (index.html) and return the signals.
pub fn mount() -> Option<Signals> {
    let s = Signals { panel: RwSignal::new(None), top: RwSignal::new(TopBarModel::default()), chrome: RwSignal::new(ChromeModel::default()),
                      card: RwSignal::new(CardModel::default()), light: RwSignal::new(LightModel::default()),
                      layers: RwSignal::new(LayersModel::default()), brief: RwSignal::new(BriefModel::default()),
                      crew: RwSignal::new(Default::default()), sim: RwSignal::new(Default::default()),
                      mars: RwSignal::new(false), overview: RwSignal::new(None), missions: RwSignal::new(Vec::new()),
                      drill: RwSignal::new(Default::default()), placed: RwSignal::new(Default::default()) };
    // ?view=mars opens the Mars view at start
    if web_sys::window().and_then(|w| w.location().search().ok()).is_some_and(|q| q.contains("view=mars")) {
        s.mars.set(true);
    }
    wasm_bindgen_futures::spawn_local(async move {
        if let Ok(b) = crate::bundle::fetch_bytes("assets/overview.json").await {
            if let Some(o) = serde_json::from_slice(&b).ok().and_then(|v| crate::ui::mars::parse(&v)) {
                s.overview.set(Some(std::sync::Arc::new(o)));
            }
        }
        if let Ok(b) = crate::bundle::fetch_bytes("assets/missions.json").await {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&b) {
                s.missions.set(crate::ui::mars::missions(&v));
            }
        }
    });
    leptos::mount::mount_to(host("mission-bar")?, move || view! { <MissionBar panel=s.panel light=s.light chrome=s.chrome/> }).forget();
    leptos::mount::mount_to(host("mode")?, move || view! { <ModeName chrome=s.chrome/> }).forget();
    leptos::mount::mount_to(host("layers")?, move || view! { <LayerRail layers=s.layers panel=s.panel mars=s.mars/> }).forget();
    let exec = RwSignal::new(load_log());
    leptos::mount::mount_to(host("inspector")?, move || view! { <Inspector panel=s.panel light=s.light card=s.card brief=s.brief crew=s.crew missions=s.missions sim=s.sim exec=exec placed=s.placed/> }).forget();
    leptos::mount::mount_to(host("timeline")?, move || view! { <Timeline panel=s.panel light=s.light brief=s.brief sim=s.sim drill=s.drill/> }).forget();
    leptos::mount::mount_to(host("mars-host")?, move || view! { <MarsView open=s.mars ov=s.overview/> }).forget();
    leptos::mount::mount_to(host("hud-host")?, move || view! { <TopBar top=s.top sim=s.sim exec=exec/> }).forget();
    leptos::mount::mount_to(host("overlay-host")?, move || view! { <Lightbox card=s.card/> }).forget();
    leptos::mount::mount_to(host("overlay-host")?, move || view! { <DrillCard drill=s.drill chrome=s.chrome/> }).forget();
    leptos::mount::mount_to(host("overlay-host")?, move || view! { <MapKey layers=s.layers chrome=s.chrome mars=s.mars/> }).forget();
    // Esc for the card works wherever the focus is (a focused button keeps key events from the canvas)
    let esc = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(|e: web_sys::KeyboardEvent| {
        if e.key() == "Escape" {
            crate::ui::card::send(crate::ui::card::CardEvent::Escape);
        }
    });
    let _ = web_sys::window()?.add_event_listener_with_callback("keydown", esc.as_ref().unchecked_ref());
    esc.forget();
    Some(s)
}

pub(super) fn val(vals: &[(&'static str, String)], k: &str) -> String {
    vals.iter().find(|(x, _)| *x == k).map(|(_, v)| v.clone()).unwrap_or_default()
}

pub(super) fn runs_of(m: &PanelModel) -> String {
    m.runs.clone()
}

// ================================================================= mission bar
