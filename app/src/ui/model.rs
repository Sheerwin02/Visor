//! What the HTML interface shows, as plain data (built by Bevy systems, tested natively) and the bridge that publishes it to
//! the Leptos components in ui/views (web only). The UI never computes plan numbers itself; it renders these models.
use bevy::prelude::*;

use crate::ui::panel::Sample;

/// One row of the leg table, already formatted.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct LegRow {
    pub label: String,
    pub metres: String,
    pub minutes: String,
    pub max_slope: String,
    pub verified: String,
    /// share of uncertainty runs that keep this leg within 3 m (corridor rule); empty when not applicable
    pub corridor: String,
    pub corridor_tip: String,
    /// the planner found a route for this leg under the rules
    pub possible: bool,
}

/// Current planner settings, for the panel's controls (planner.rs owns them).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Controls {
    pub limit_deg: f64,
    pub limit_range: (f64, f64),
    pub speed: f64,
    pub speeds: Vec<f64>,
    pub correct: bool,
    pub measured_bias: f64,
    pub picking: bool,
    pub is_default: bool,
    /// published plan: loop length (m), walking time (s)
    pub published: (f64, f64),
    /// must-visit flag per stop (walking order)
    pub must: Vec<bool>,
    pub budget_min: f64,
    /// result of the last automatic action (best order, budget fit)
    pub note: String,
    /// robustness check across slope limits, one line each
    pub robust: Vec<String>,
}

/// The planning panel.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PanelModel {
    pub feasible: bool,
    pub legs_n: usize,
    pub walk_s: f64,
    pub length_m: f64,
    pub back_s: f64,
    pub back_stop: String,
    pub worst_deg: f64,
    pub worst_leg: String,
    pub over_limit_cells: i64,
    pub verified_pct: i64,
    /// values for the text templates (speed, limit, bias, measured results), already formatted
    pub vals: Vec<(&'static str, String)>,
    pub samples: Vec<Sample>,
    pub leg_starts: Vec<usize>,
    pub caution: f32,
    pub limit: f32,
    pub legs: Vec<LegRow>,
    /// numbered stop labels, in walking order ("2 Bococo")
    pub stops: Vec<String>,
    pub controls: Controls,
    /// rover check per-stop counts: agrees, partial, disagrees
    pub rover_counts: (String, String, String),
    /// uncertainty range of the published plan (route ensemble), already worded; empty without an ensemble
    pub range_note: String,
    /// walking time p10 and p90 (s) of the published plan's uncertainty runs (None for other plans)
    pub time_range: Option<(f64, f64)>,
    /// the shown uncertainty run had not settled (route_ensemble.json "converged": false)
    pub range_unsettled: bool,
    /// blocked plan: stops the exit cannot be reached from, and the nearest reachable spot (index, label, east, north, m)
    pub fixes: Vec<(usize, String, f32, f32, f32)>,
    /// per leg: walking time (s) and share of uncertainty runs keeping it within 3 m (published plan only)
    pub leg_secs: Vec<f64>,
    pub leg_agree: Vec<Option<f64>>,
    /// one line per stop for the inspector's Stops tab
    pub stop_cards: Vec<StopSummary>,
    /// number of uncertainty runs in the shown ensemble (empty without one)
    pub runs: String,
}

/// A stop in the Stops list: number, name, kind, photo, first fact.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct StopSummary {
    pub index: usize,
    pub label: String,
    pub kind: String,
    pub photo: Option<String>,
    pub fact: String,
    /// stops.json id ("" for a stop added on the map)
    pub id: String,
    /// the science question (first "question" task), bag labels (sample tasks), number of tasks
    pub question: String,
    /// the question links facts in a way NASA does not state
    pub question_inference: bool,
    pub bags: Vec<String>,
    pub tasks: usize,
    /// part of the drill pair (stops.json "pair")
    pub pair: bool,
    /// rover check at this stop: label (agrees/partial/disagrees), map tilt, rover tilt (deg)
    pub ground: Option<(String, f64, f64)>,
    /// station tasks in order: step label, text, bag labels (sample tasks)
    pub steps: Vec<(&'static str, String, Vec<String>)>,
    /// position (east, north m), lat / east lon (deg), walk-back to the exit on the live plan (s)
    pub e: f32,
    pub n: f32,
    pub latlon: Option<(f64, f64)>,
    pub walk_back_s: Option<f64>,
    /// the planner wrote this station's plan (no citations); its tasks as editable rows
    pub edited: bool,
    pub raw: Vec<crate::plan::edits::EditTask>,
    /// nearest Curiosity stop with a station plan: name, distance (m), its tasks (a starting point for a new station)
    pub near: Option<(String, f32, Vec<crate::plan::edits::EditTask>)>,
    /// Curiosity measured steeper ground than the map here: the gentlest map ground within reach (east, north, m, corrected
    /// slope deg; plan/relay.rs work_spot)
    pub work_spot: Option<(f32, f32, f32, f32)>,
    /// ChemCam / APXS targets measured within CHEM_R_M of the station (scene::chem::near rows) and how many in all
    #[allow(clippy::type_complexity)]
    pub chem: Vec<(String, String, i64, f32, Option<f64>, Option<f64>, Option<f64>, Option<f64>)>,
    pub chem_total: usize,
}

impl PanelModel {
    /// Legs (walking order) with no route under the rules.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn blocked_legs(&self) -> Vec<usize> {
        self.legs.iter().enumerate().filter(|(_, l)| !l.possible).map(|(i, _)| i).collect()
    }

    /// Blocked legs as "3→4, 6→1" (stop numbers in walking order).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn blocked_label(&self) -> String {
        let n = self.legs.len();
        self.blocked_legs().iter().map(|&i| format!("{}\u{2192}{}", i + 1, if i + 1 == n { 1 } else { i + 2 })).collect::<Vec<_>>().join(", ")
    }
}

#[cfg(test)]
mod blocked_tests {
    use super::*;

    #[test]
    fn blocked_legs_are_listed_and_labelled() {
        let leg = |ok: bool| LegRow { possible: ok, ..Default::default() };
        let m = PanelModel { legs: vec![leg(true), leg(true), leg(false), leg(true)], ..Default::default() };
        assert_eq!(m.blocked_legs(), [2]);
        assert_eq!(m.blocked_label(), "3\u{2192}4");
        let last = PanelModel { legs: vec![leg(true), leg(false)], ..Default::default() };
        assert_eq!(last.blocked_label(), "2\u{2192}1");
        assert!(PanelModel::default().blocked_legs().is_empty());
    }
}

/// Map slope above the rover's tilt by more than this (deg) is listed in Risk as "map overstates" (the ground is gentler).
pub const OVERSTATE_DEG: f64 = 5.0;

impl StopSummary {
    /// Curiosity measured steeper ground than the map here: the rover check disagrees (more than 3 deg off, the per-stop
    /// rule) and the rover's tilt is the larger. The hazard direction: the map correction would make it look gentler still.
    pub fn map_understates(&self) -> bool {
        matches!(&self.ground, Some((l, a, r)) if l == "disagrees" && r > a)
    }

    /// The map is much steeper than the rover measured (the ground is gentler than the map).
    pub fn map_overstates(&self) -> bool {
        matches!(&self.ground, Some((l, a, r)) if l == "disagrees" && a - r > OVERSTATE_DEG)
    }
}

#[cfg(test)]
mod slope_direction_tests {
    use super::*;

    fn at(l: &str, map: f64, rover: f64) -> StopSummary {
        StopSummary { ground: Some((l.into(), map, rover)), ..Default::default() }
    }

    #[test]
    fn both_directions_of_a_disagreeing_stop_are_told_apart() {
        // the published stations' rover checks (stops.json): Rio Frio, ridge top, Valle de la Luna, Nevado Sajama
        assert!(at("disagrees", 11.47, 17.533).map_understates() && !at("disagrees", 11.47, 17.533).map_overstates());
        assert!(at("disagrees", 7.501, 11.928).map_understates());
        assert!(at("disagrees", 22.797, 6.807).map_overstates() && !at("disagrees", 22.797, 6.807).map_understates());
        assert!(at("disagrees", 20.018, 11.088).map_overstates());
        // agreeing or partial stops are neither; no rover check is neither
        assert!(!at("agrees", 9.0, 10.0).map_understates() && !at("partial", 9.0, 11.5).map_understates());
        assert!(!StopSummary::default().map_understates() && !StopSummary::default().map_overstates());
    }

    #[test]
    fn real_bundle_flags_rio_frio_and_the_ridge_top() {
        let s: serde_json::Value = serde_json::from_slice(&std::fs::read("../assets/stops.json").unwrap()).unwrap();
        let under: Vec<&str> = s["stops"].as_array().unwrap().iter().filter(|x| {
            let rc = &x["rover_check"];
            at(rc["label"].as_str().unwrap(), rc["dem_tilt"].as_f64().unwrap(), rc["rover_tilt"].as_f64().unwrap()).map_understates()
        }).map(|x| x["id"].as_str().unwrap()).collect();
        assert_eq!(under, ["rio_frio", "transition"]);
    }
}

/// Which map layers are on (left rail), and the view.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct LayersModel {
    pub walking: bool,
    pub map_view: bool,
    pub route: bool,
    pub stops: bool,
    pub path: bool,
    pub slope: bool,
    pub checks: bool,
    pub verified: bool,
    pub agreement: bool,
    pub shadows: bool,
    pub has_shadows: bool,
    pub ready: bool,
    pub has_ready: bool,
    pub chem: bool,
    pub has_chem: bool,
    /// walk-back time lines (key =)
    pub back: bool,
    /// readiness verdict and thresholds for the rail's tooltip (from readiness.json)
    pub ready_tip: String,
    /// basemap choices: title, detail (source), tooltip, selected, loading; and an error line
    pub basemaps: Vec<(String, String, String, bool, bool)>,
    pub basemap_error: String,
}

/// Walk mode's top bar.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct TopBarModel {
    pub visible: bool,
    pub next_label: String,
    pub stop_k: usize,
    pub stop_n: usize,
    pub dist: String,
    pub time: String,
    pub slope: String,
    pub slope_map: String,
    /// line of sight from the walker: 2 = the exit, 1 = the suggested relay (not the exit), 0 = neither (terrain only)
    pub comms: u8,
    /// standing on rover-verified ground (badge on the slope tile; the sentence is its tooltip)
    pub verified: bool,
    /// click-to-walk target or binocular range, worded (empty = none)
    pub aim: String,
    /// the walk target's way goes over the slope limit
    pub aim_warn: bool,
    /// Curiosity parked within ROVER_TILT_R_M: its measured tilt, distance to it (m), sol (rounded, worded)
    pub rover_tilt: Option<(String, String, String)>,
    /// None = no way back under the rules
    pub back: Option<String>,
    /// compass strip offset and next-stop marker position (CSS px), off-view side (-1, 0, 1), bearing and relative angle
    pub strip_px: i32,
    pub marker_px: i32,
    pub marker_side: i8,
    pub bearing: i32,
    pub rel: i32,
    pub note: String,
    pub speed: String,
    pub bias: String,
    /// minimap: walker and next stop as fractions of the patch (x east 0..1, y south 0..1), heading (deg), third person on
    pub map_me: (f32, f32),
    pub map_next: (f32, f32),
    pub heading: i32,
    pub third: bool,
}

/// Mode chip, mode button and the key list.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ChromeModel {
    pub walking: bool,
    pub in_map: bool,
    pub keys_shown: bool,
    pub speed: String,
}

/// One sourced fact on a stop card.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct FactRow {
    pub text: String,
    /// source page title, and "publisher · date · byline"
    pub title: String,
    pub meta: String,
    pub url: String,
    /// our pairing of a NASA statement with this stop (labelled on the card)
    pub inference: bool,
}

/// The rover's own photo from a stop (approach view).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Photo {
    pub src: String,
    pub alt: String,
    pub caption: String,
    pub raw_url: String,
    /// which way the camera looked (stops.json photo.view): compass azimuth (deg) and the worded line; None when unknown
    pub az: Option<f64>,
    pub view: String,
    /// the photo belongs to the nearest Curiosity stop, not to this (map-added) station
    pub borrowed: bool,
}

/// Drill-in card for the object selected on the map (scene/select.rs, ui/drill.rs).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct DrillModel {
    pub open: bool,
    pub kind: &'static str,
    pub title: String,
    pub rows: Vec<(&'static str, String)>,
    pub note: String,
    pub photo: Option<String>,
    pub pano: Option<Pano>,
    pub can_walk: bool,
    pub at: (f32, f32),
    /// the selected station or leg (walking order), for the timeline's highlight
    pub station: Option<usize>,
    pub leg: Option<usize>,
    /// a selected leg's profile (panel samples) and the slope rule's caution / limit (corrected deg)
    pub profile: Vec<crate::ui::panel::Sample>,
    pub caution: f32,
    pub limit: f32,
    /// extra buttons: label, tooltip, action, primary (placed relay / working spots: scene/place.rs)
    pub acts: Vec<(String, &'static str, crate::scene::place::PlaceAct, bool)>,
}

/// Curiosity's Navcam 360-degree mosaic at a stop (panos.json): cylindrical, azimuth clockwise from north starting at
/// `start_az_deg` on the left edge, elevation `el_max_deg` at the top edge down to `el_min_deg`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Pano {
    pub src: String,
    pub start_az_deg: f64,
    pub el_max_deg: f64,
    pub el_min_deg: f64,
    pub sol: i64,
    pub product: String,
    pub url: String,
}

impl Pano {
    pub fn from_doc(doc: &serde_json::Value, stop_id: &str) -> Option<Pano> {
        let p = doc["panos"].as_array()?.iter().find(|p| p["stop"] == stop_id)?;
        Some(Pano { src: format!("assets/{}", p["file"].as_str()?), start_az_deg: p["start_az_deg"].as_f64()?,
                    el_max_deg: p["el_max_deg"].as_f64()?, el_min_deg: p["el_min_deg"].as_f64()?, sol: p["sol"].as_i64()?,
                    product: p["product"].as_str()?.to_string(), url: p["url"].as_str().unwrap_or("").to_string() })
    }

    /// Image left edge relative to the view's left edge, in degrees of azimuth: the view centred on `heading_deg` and
    /// `fov_deg` wide starts at heading - fov/2; returns how far left (deg, 0..360) the image must move.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn shift_deg(&self, heading_deg: f64, fov_deg: f64) -> f64 {
        (heading_deg - fov_deg / 2.0 - self.start_az_deg).rem_euclid(360.0)
    }
}

#[cfg(test)]
mod pano_tests {
    use super::*;

    #[test]
    fn panorama_shift_and_the_bundle_entries() {
        let p = Pano { start_az_deg: 0.0, ..Default::default() };
        assert_eq!(p.shift_deg(45.0, 90.0), 0.0, "facing NE with 90 deg: the view starts at north");
        assert_eq!(p.shift_deg(0.0, 90.0), 315.0, "facing north: starts at 315");
        let d: serde_json::Value = serde_json::from_slice(&std::fs::read("../assets/panos.json").unwrap()).unwrap();
        let n = Pano::from_doc(&d, "nevado_sajama").unwrap();
        assert!(n.el_min_deg < 0.0 && n.el_max_deg > 0.0 && n.sol == 4711 && n.src.ends_with(".jpg"));
        assert!(Pano::from_doc(&d, "custom-1").is_none());
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Stat {
    pub label: String,
    pub value: String,
    pub unit: &'static str,
    pub note: String,
}

/// The stop card (card.rs builds it).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct CardModel {
    /// "Open in NASA Mars Trek" link centred on this stop (empty without trek_layers.json)
    pub trek_link: String,
    pub open: bool,
    pub lightbox: bool,
    pub index: usize,
    pub kicker: String,
    pub title: String,
    pub subtitle: String,
    pub photo: Option<Photo>,
    pub stats: Vec<Stat>,
    pub facts: Vec<FactRow>,
    /// a stop added on the map (no NASA observation)
    pub custom: bool,
    pub custom_note: String,
    pub position_tag: String,
    pub position_note: String,
    /// boxwork context facts shared by every card
    pub about: Vec<FactRow>,
    /// station plan (stops.json tasks): step label, text, the facts it cites ("[1]" = this card's fact 1; other stops by name),
    /// inference
    pub plan: Vec<(&'static str, String, String, bool)>,
    /// the plan was written by the planner (no citations)
    pub edited: bool,
}

/// One start hour in the "When to walk" table.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct LightRow {
    pub hour: f64,
    pub label: String,
    /// sun at the start: "height / azimuth" in degrees
    pub sun: String,
    pub passes: bool,
    pub toward: String,
    pub shadow: String,
    pub best: bool,
    pub selected: bool,
}

/// The "When to walk" panel section (sunlight.rs builds it).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct LightModel {
    pub sol: f64,
    pub sol_range: (f64, f64),
    pub date: String,
    /// "13 Nov 2025" (UTC date of local noon on the sol)
    pub earth_date: String,
    pub intro: String,
    pub ties: String,
    pub rows: Vec<LightRow>,
    pub status: String,
    pub shadow_text: String,
    pub sun_text: String,
    pub sun_az: Option<f64>,
    pub photo_text: String,
    pub photo_az: Option<f64>,
    pub warn_south: bool,
    /// sun height along the walk at the chosen start: (metres from the start, degrees)
    pub curve: Vec<(f64, f64)>,
    pub band: (f64, f64),
    pub hour_label: String,
    /// REMS daily weather for the sol (conditions.json): title, (label, value) rows, note when absent, source line, feed URL
    pub wx_title: String,
    pub wx_rows: Vec<(String, String)>,
    pub wx_note: String,
    pub wx_source: String,
    pub wx_url: String,
    pub wx_disclaimer: String,
    /// RAD context line for the sol (radiation.json), and the DONKI space-weather line with its events (date, type, link)
    pub rad_note: String,
    /// RAD silicon detector B: median of the stop sols' median dose rates (uGy/h) and the number of valid sols
    pub rad_rate: Option<(f64, usize)>,
    pub sw_text: String,
    pub sw_events: Vec<(String, String, String)>,
}

/// Publishes models to the web UI (no-op on native builds, where the last models are kept for tests).
#[derive(Resource, Default)]
pub struct UiBridge {
    #[cfg(target_arch = "wasm32")]
    pub signals: Option<crate::ui::views::Signals>,
    pub panel: Option<PanelModel>,
    pub top: Option<TopBarModel>,
    pub chrome: Option<ChromeModel>,
    pub card: Option<CardModel>,
    pub light: Option<LightModel>,
    pub layers: Option<LayersModel>,
    pub brief: Option<crate::ui::brief::BriefModel>,
    pub crew: Option<crate::plan::crew::CrewModel>,
    pub sim: Option<crate::plan::simrun::SimModel>,
    pub drill: Option<DrillModel>,
    pub placed: Option<crate::scene::place::PlacedModel>,
}

impl UiBridge {
    pub fn set_drill(&mut self, m: DrillModel) {
        if self.drill.as_ref() == Some(&m) {
            return;
        }
        #[cfg(target_arch = "wasm32")]
        if let Some(s) = self.signals.as_ref() {
            s.set_drill(m.clone());
        }
        self.drill = Some(m);
    }
    pub fn set_placed(&mut self, m: crate::scene::place::PlacedModel) {
        if self.placed.as_ref() == Some(&m) {
            return;
        }
        #[cfg(target_arch = "wasm32")]
        if let Some(s) = self.signals.as_ref() {
            s.set_placed(m.clone());
        }
        self.placed = Some(m);
    }
    pub fn set_panel(&mut self, m: PanelModel) {
        #[cfg(target_arch = "wasm32")]
        if let Some(s) = self.signals.as_ref() {
            s.set_panel(m.clone());
        }
        self.panel = Some(m);
    }
    pub fn set_top(&mut self, m: TopBarModel) {
        if self.top.as_ref() == Some(&m) {
            return;
        }
        #[cfg(target_arch = "wasm32")]
        if let Some(s) = self.signals.as_ref() {
            s.set_top(m.clone());
        }
        self.top = Some(m);
    }
    pub fn set_card(&mut self, m: CardModel) {
        if self.card.as_ref() == Some(&m) {
            return;
        }
        #[cfg(target_arch = "wasm32")]
        if let Some(s) = self.signals.as_ref() {
            s.set_card(m.clone());
        }
        self.card = Some(m);
    }
    pub fn set_layers(&mut self, m: LayersModel) {
        if self.layers.as_ref() == Some(&m) {
            return;
        }
        #[cfg(target_arch = "wasm32")]
        if let Some(s) = self.signals.as_ref() {
            s.set_layers(m.clone());
        }
        self.layers = Some(m);
    }
    pub fn set_light(&mut self, m: LightModel) {
        if self.light.as_ref() == Some(&m) {
            return;
        }
        #[cfg(target_arch = "wasm32")]
        if let Some(s) = self.signals.as_ref() {
            s.set_light(m.clone());
        }
        self.light = Some(m);
    }
    pub fn set_brief(&mut self, m: crate::ui::brief::BriefModel) {
        if self.brief.as_ref() == Some(&m) {
            return;
        }
        #[cfg(target_arch = "wasm32")]
        if let Some(s) = self.signals.as_ref() {
            s.set_brief(m.clone());
        }
        self.brief = Some(m);
    }
    pub fn set_sim(&mut self, m: crate::plan::simrun::SimModel) {
        if self.sim.as_ref() == Some(&m) {
            return;
        }
        #[cfg(target_arch = "wasm32")]
        if let Some(s) = self.signals.as_ref() {
            s.set_sim(m.clone());
        }
        self.sim = Some(m);
    }
    pub fn set_crew(&mut self, m: crate::plan::crew::CrewModel) {
        if self.crew.as_ref() == Some(&m) {
            return;
        }
        #[cfg(target_arch = "wasm32")]
        if let Some(s) = self.signals.as_ref() {
            s.set_crew(m.clone());
        }
        self.crew = Some(m);
    }
    pub fn set_chrome(&mut self, m: ChromeModel) {
        if self.chrome.as_ref() == Some(&m) {
            return;
        }
        #[cfg(target_arch = "wasm32")]
        if let Some(s) = self.signals.as_ref() {
            s.set_chrome(m.clone());
        }
        self.chrome = Some(m);
    }
}

/// Mount the web UI once (web only) and keep its signals in the bridge.
pub fn mount_ui(mut bridge: ResMut<UiBridge>) {
    #[cfg(target_arch = "wasm32")]
    if bridge.signals.is_none() {
        bridge.signals = crate::ui::views::mount();
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = &mut bridge;
}
