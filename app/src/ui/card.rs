//! Stop cards and the approach view: what Curiosity found at each stop, with its sources, the rover's own
//! photo from that spot, and the stop's walk-back and slope numbers. Built from stops.json (every fact quoted and linked,
//! checked against the live pages), the current plan and the uncertainty ensemble; nothing here is typed in code.
//! Opened from a stop label on the map, a stop name in the panel, or Enter in Walk mode; Esc closes.
use std::sync::Mutex;

use bevy::prelude::*;
use serde_json::Value;

use crate::ui::model::{CardModel, FactRow, Photo, Stat, UiBridge};
use crate::text::{fill, stop_card as t};
use crate::scene::walk::{Mode, Stop, Stops, Walker};

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))] // sent by the web UI
pub enum CardEvent {
    /// open the card of the stop at this index in walking order
    Open(usize),
    Close,
    Prev,
    Next,
    /// show or hide the full-size approach photo
    Photo(bool),
    /// Esc: close the full-size photo if open, else the card. Sent by a page-level key listener (ui/views), because after a
    /// click the focus is on an HTML button and the key never reaches the canvas.
    Escape,
}

pub static EVENTS: Mutex<Vec<CardEvent>> = Mutex::new(Vec::new());

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn send(e: CardEvent) {
    if let Ok(mut q) = EVENTS.lock() {
        q.push(e);
    }
}

#[derive(Resource, Default)]
pub struct CardState {
    pub open: Option<usize>,
    pub lightbox: bool,
}

/// "Valle de la Luna: hollow drill site" -> ("Valle de la Luna", "Hollow drill site").
pub fn split_name(name: &str) -> (String, String) {
    match name.split_once(':') {
        Some((a, b)) => {
            let b = b.trim();
            let mut c = b.chars();
            let sub = c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default();
            (a.trim().to_string(), sub)
        }
        None => (name.trim().to_string(), String::new()),
    }
}

/// "2025-11-18" -> "18 Nov 2025" (the date as printed on the source page).
pub fn nice_date(iso: &str) -> String {
    const M: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let p: Vec<&str> = iso.split('-').collect();
    match (p.first(), p.get(1).and_then(|m| m.parse::<usize>().ok()), p.get(2).and_then(|d| d.parse::<u32>().ok())) {
        (Some(y), Some(m), Some(d)) if (1..=12).contains(&m) => format!("{d} {} {y}", M[m - 1]),
        _ => iso.to_string(),
    }
}

/// Publisher, date and byline for a source line: "NASA Science · 18 Nov 2025 · Michelle Minitti, Framework".
pub fn source_meta(url: &str, published: &str, byline: &str) -> String {
    let site = if url.contains("jpl.nasa.gov") {
        "NASA JPL"
    } else if url.contains("science.nasa.gov") {
        "NASA Science"
    } else {
        url.split('/').nth(2).unwrap_or("")
    };
    [site.to_string(), nice_date(published), byline.to_string()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" \u{00b7} ")
}

/// Position notes that start with "NOTIONAL:" or "INFERRED:" get a tag; the rest of the note is returned as written.
pub fn position_tag(note: &str) -> (String, String) {
    for (key, tag) in [("NOTIONAL:", t::NOTIONAL), ("INFERRED:", t::INFERRED)] {
        if let Some(rest) = note.strip_prefix(key) {
            return (tag.to_string(), capitalise(rest.trim()));
        }
    }
    (String::new(), capitalise(note.trim()))
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

fn fact_rows(facts: &Value) -> Vec<FactRow> {
    facts.as_array().into_iter().flatten().map(|f| {
        let s = |k: &str| f[k].as_str().unwrap_or("").to_string();
        FactRow { text: s("text"), title: s("source_title"), meta: source_meta(&s("source"), &s("published"), &s("byline")),
                  url: s("source"), inference: f["inference"].as_bool() == Some(true) }
    }).collect()
}

fn one(x: f64) -> String {
    format!("{:.1}", x)
}

/// Everything the card shows for stop `i` of the current walking order.
/// `doc` = stops.json; `plan_stops` = the current plan's stops with walk_back_s; `ens` = route_ensemble.json (used only for
/// the published plan, `is_default`); `bias` = the rover correction in use (deg).
/// The station plan of a stops.json stop: (step label, text, cited facts, inference). A fact of this stop is cited by its
/// number in the card's list ("[2]"); a fact of another stop by that stop's short name and number.
pub fn plan_rows(me: &str, tasks: &Value, doc: &Value) -> Vec<(&'static str, String, String, bool)> {
    let short = |id: &str| doc["stops"].as_array().and_then(|a| a.iter().find(|x| x["id"] == id))
        .and_then(|x| x["name"].as_str()).map(|n| split_name(n).0).unwrap_or_else(|| id.to_string());
    tasks.as_array().map(|ts| ts.iter().filter_map(|x| {
        let refs: Vec<String> = x["refs"].as_array().into_iter().flatten().filter_map(|r| {
            let (id, k) = (r[0].as_str()?, r[1].as_u64()? + 1);
            Some(if id == me { format!("[{k}]") } else { format!("{} [{k}]", short(id)) })
        }).collect();
        Some((crate::plan::simrun::step_label(x["step"].as_str()?), x["text"].as_str()?.to_string(), refs.join(", "),
              x["inference"].as_bool().unwrap_or(false)))
    }).collect()).unwrap_or_default()
}

pub fn build(i: usize, stops: &[Stop], doc: &Value, plan_stops: &[Value], ens: Option<&Value>, is_default: bool, bias: f64,
             edits: &crate::plan::edits::StationEdits) -> CardModel {
    use crate::ui::hud::minutes;
    let n = stops.len();
    let st = &stops[i];
    let d = doc["stops"].as_array().and_then(|a| a.iter().find(|x| x["id"].as_str() == Some(st.id.as_str())));
    let (title, subtitle) = split_name(&st.name);
    let kind_key = d.and_then(|x| x["kind"].as_str()).unwrap_or("");
    let kind = crate::text::stop_kinds::ALL.iter().find(|(k, _)| *k == kind_key).map(|(_, v)| v.to_string()).unwrap_or_default();
    let mut kicker = fill(t::KICKER, &[("stop_k", (i + 1).to_string()), ("stop_n", n.to_string())]);
    if !kind.is_empty() {
        kicker = format!("{kicker} \u{00b7} {kind}");
    }
    let mut stats = Vec::new();
    // walk back to the exit: the current plan, plus the ensemble range for the published plan
    let wb = plan_stops.iter().find(|x| x["id"].as_str() == Some(st.id.as_str())).map(|x| x["walk_back_s"].as_f64());
    let range = ens.filter(|_| is_default).and_then(|e| {
        let shown = e["shown_correlation_length_m"].as_f64();
        let en = e["ensembles"].as_array()?.iter().find(|x| x["correlation_length_m"].as_f64() == shown)?;
        let w = en["walk_back_s"].as_array()?.iter().find(|x| x["id"].as_str() == Some(st.id.as_str()))?;
        Some(fill(t::WALK_BACK_RANGE, &[("runs", en["runs"].to_string()), ("t_p50", minutes(w["p50"].as_f64()? as f32)),
                                        ("t_p90", minutes(w["p90"].as_f64()? as f32))]))
    });
    stats.push(match wb.flatten() {
        Some(s) if s <= 0.0 => Stat { label: t::WALK_BACK_LABEL.into(), value: "0".into(), unit: t::UNIT_MIN.into(), note: t::WALK_BACK_EXIT.into() },
        Some(s) => Stat { label: t::WALK_BACK_LABEL.into(), value: minutes(s as f32), unit: t::UNIT_MIN.into(), note: range.unwrap_or_default() },
        None => Stat { label: t::WALK_BACK_LABEL.into(), value: "\u{2014}".into(), unit: "", note: t::WALK_BACK_NONE.into() },
    });
    let mut facts = Vec::new();
    let mut position = (String::new(), String::new());
    let mut photo = None;
    let mut custom_note = String::new();
    match d {
        Some(d) => {
            if let Some(s) = d["slope_3m_deg"].as_f64() {
                stats.push(Stat { label: t::SLOPE_LABEL.into(), value: one(s), unit: crate::text::hud::UNIT_DEG.into(),
                                  note: fill(t::SLOPE_NOTE, &[("x", one((s - bias).max(0.0)))]) });
            }
            let rc = &d["rover_check"];
            if let (Some(lab), Some(dt), Some(rt)) = (rc["label"].as_str(), rc["dem_tilt"].as_f64(), rc["rover_tilt"].as_f64()) {
                let word = match lab { "agrees" => t::CHECK_AGREES, "partial" => t::CHECK_PARTIAL, _ => t::CHECK_DISAGREES };
                stats.push(Stat { label: t::ROVER_CHECK_LABEL.into(), value: word.into(), unit: "",
                                  note: fill(t::CHECK_NOTE, &[("dem_tilt_deg", one(dt)), ("rover_tilt_deg", one(rt)),
                                                              ("sol", d["sol"].to_string())]) });
            }
            facts = fact_rows(&d["facts"]);
            position = position_tag(d["position_note"].as_str().unwrap_or(""));
            photo = photo_of(&d["photo"], &title, false);
        }
        None => {
            // a stop added on the map: no NASA observation; point to the nearest Curiosity stop
            let near = doc["stops"].as_array().into_iter().flatten().filter_map(|x| {
                let (e, nn) = (x["e_m"].as_f64()?, x["n_m"].as_f64()?);
                Some((((e - st.e as f64).powi(2) + (nn - st.n as f64).powi(2)).sqrt(), x["name"].as_str()?.to_string(), x))
            }).min_by(|a, b| a.0.total_cmp(&b.0));
            if let Some((dist, name, x)) = near {
                let (short, dm) = (split_name(&name).0, format!("{}", dist.round() as i64));
                custom_note = fill(t::CUSTOM_NOTE, &[("stop_name", short.clone()), ("distance_m", dm.clone())]);
                // the nearest Curiosity photo, labelled as taken elsewhere
                photo = photo_of(&x["photo"], &short, true).map(|mut p| {
                    p.caption = format!("{} {}", fill(t::PHOTO_NEAR, &[("stop_name", short), ("distance_m", dm)]), p.caption);
                    p
                });
            }
        }
    }
    let plan = plan_rows(&st.id, &crate::plan::edits::tasks_value(&st.id, doc, edits), doc);
    let edited = edits.0.contains_key(&st.id);
    CardModel {
        edited,
        trek_link: String::new(),
        open: true,
        lightbox: false,
        index: i,
        kicker,
        title,
        subtitle,
        photo,
        stats,
        facts,
        custom: d.is_none(),
        custom_note,
        position_tag: position.0,
        position_note: position.1,
        about: fact_rows(&doc["context_facts"]),
        plan,
    }
}

/// One line per stop (in walking order) for the inspector's Stops list: kind, photo, first sourced fact.
pub fn summaries(stops: &[Stop], doc: &Value, edits: &crate::plan::edits::StationEdits) -> Vec<crate::ui::model::StopSummary> {
    stops.iter().enumerate().map(|(i, st)| {
        let d = doc["stops"].as_array().and_then(|a| a.iter().find(|x| x["id"].as_str() == Some(st.id.as_str())));
        let kind_key = d.and_then(|x| x["kind"].as_str()).unwrap_or("");
        let near = doc["stops"].as_array().into_iter().flatten()
            .filter(|x| x["id"].as_str() != Some(st.id.as_str()) && x["tasks"].as_array().is_some_and(|t| !t.is_empty()))
            .filter_map(|x| Some((((x["e_m"].as_f64()? as f32 - st.e).powi(2) + (x["n_m"].as_f64()? as f32 - st.n).powi(2)).sqrt(), x)))
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(dist, x)| (split_name(x["name"].as_str().unwrap_or("")).0, dist, crate::plan::edits::editable(&x["tasks"])));
        let mut c = crate::ui::model::StopSummary {
            index: i,
            label: crate::scene::labels::label_text(i, &st.name),
            kind: crate::text::stop_kinds::ALL.iter().find(|(k, _)| *k == kind_key).map(|(_, v)| v.to_string()).unwrap_or_default(),
            photo: d.and_then(|x| x["photo"]["file"].as_str()).map(|f| format!("assets/{f}")),
            fact: d.and_then(|x| x["facts"][0]["text"].as_str()).unwrap_or("").to_string(),
            id: st.id.clone(),
            pair: d.is_some_and(|x| x["pair"].is_object()),
            e: st.e,
            n: st.n,
            near: if d.is_none() { near } else { None },
            ground: d.and_then(|x| {
                let rc = &x["rover_check"];
                Some((rc["label"].as_str()?.to_string(), rc["dem_tilt"].as_f64()?, rc["rover_tilt"].as_f64()?))
            }),
            ..Default::default()
        };
        apply_tasks(&mut c, &crate::plan::edits::tasks_value(&st.id, doc, edits), edits);
        c
    }).collect()
}

/// The station-plan fields of a stop summary from its tasks (stops.json schema).
pub fn apply_tasks(c: &mut crate::ui::model::StopSummary, tasks: &Value, edits: &crate::plan::edits::StationEdits) {
    let ts = tasks.as_array().cloned().unwrap_or_default();
    let q = ts.iter().find(|t| t["step"] == "question");
    c.question = q.and_then(|t| t["text"].as_str()).unwrap_or("").to_string();
    c.question_inference = q.and_then(|t| t["inference"].as_bool()).unwrap_or(false);
    c.bags = ts.iter().filter_map(|t| t["bags"].as_array()).flatten().filter_map(|b| b.as_str().map(str::to_string)).collect();
    c.tasks = ts.len();
    c.steps = ts.iter().filter_map(|t| Some((crate::plan::simrun::step_label(t["step"].as_str()?), t["text"].as_str()?.to_string(),
        t["bags"].as_array().map(|b| b.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default()))).collect();
    c.edited = edits.0.contains_key(&c.id);
    c.raw = crate::plan::edits::editable(tasks);
}

/// Apply card events and keys, and publish the card model.
#[allow(clippy::too_many_arguments)]
pub fn update_card(
    keys: Res<ButtonInput<KeyCode>>,
    mode: Res<Mode>,
    mut st: ResMut<CardState>,
    stops: Option<Res<Stops>>,
    docs: Option<Res<crate::BundleDocs>>,
    last: Option<Res<crate::plan::share::LastPlan>>,
    inputs: Option<Res<crate::plan::planner::PlanInputs>>,
    settings: Option<Res<crate::plan::planner::PlanSettings>>,
    walker: Option<Res<Walker>>,
    edits: Res<crate::plan::edits::StationEdits>,
    mut bridge: ResMut<UiBridge>,
) {
    let (Some(stops), Some(docs)) = (stops, docs) else { return };
    let n = stops.0.len();
    let mut events: Vec<CardEvent> = EVENTS.lock().map(|mut q| q.drain(..).collect()).unwrap_or_default();
    if keys.just_pressed(KeyCode::Enter) && *mode == Mode::Walk && n > 0 {
        // the stop the walker stands at (within 10 m), else the next one
        let at = walker.as_ref().and_then(|w| stops.0.iter().position(|s| (s.e - w.e).hypot(s.n - w.n) <= 10.0))
            .or_else(|| walker.as_ref().map(|w| w.next % n));
        if let Some(k) = at {
            events.push(if st.open == Some(k) { CardEvent::Close } else { CardEvent::Open(k) });
        }
    }
    let changed = !events.is_empty() || stops.is_changed() || edits.is_changed() || last.as_ref().is_some_and(|l| l.is_changed());
    for e in events {
        match e {
            CardEvent::Open(k) if k < n => {
                st.open = Some(k);
                st.lightbox = false;
            }
            CardEvent::Open(_) => {}
            CardEvent::Close => {
                st.open = None;
                st.lightbox = false;
            }
            CardEvent::Prev => st.open = st.open.map(|k| (k + n - 1) % n.max(1)),
            CardEvent::Next => st.open = st.open.map(|k| (k + 1) % n.max(1)),
            CardEvent::Photo(on) => st.lightbox = on && st.open.is_some(),
            CardEvent::Escape if st.lightbox => st.lightbox = false,
            CardEvent::Escape => st.open = None,
        }
    }
    if st.open.is_some_and(|k| k >= n) {
        st.open = None;
    }
    if !changed {
        return;
    }
    let model = match st.open {
        None => CardModel::default(),
        Some(k) => {
            let is_default = matches!((inputs.as_ref(), settings.as_ref()), (Some(i), Some(s)) if s.same_plan(&i.defaults()));
            let bias = settings.as_ref().zip(inputs.as_ref()).map(|(s, i)| i.rules(s).bias_deg).unwrap_or(0.0);
            let plan_stops = last.as_ref().map(|l| l.stops.clone()).unwrap_or_default();
            let mut m = build(k, &stops.0, &docs.stops_doc, &plan_stops, docs.ensemble.as_ref(), is_default, bias, &edits);
            let s = &stops.0[k];
            m.trek_link = docs.trek.as_ref().and_then(|t| crate::scene::basemap::latlon(&docs.manifest["grid"], s.e as f64, s.n as f64)
                .and_then(|c| crate::scene::basemap::trek_link(t, Some(c), 0.0006))).unwrap_or_default();
            m.lightbox = st.lightbox;
            m
        }
    };
    bridge.set_card(model);
}

/// A station photo from stops.json "photo": file, caption, and the view direction line when photo.view is present.
pub fn photo_of(p: &Value, stop_name: &str, borrowed: bool) -> Option<Photo> {
    let f = p["file"].as_str()?;
    let v = &p["view"];
    let az = v["az_deg"].as_f64();
    let view = match (az, v["el_deg"].as_f64(), v["compass"].as_str()) {
        (Some(a), Some(e), Some(c)) => {
            let line = fill(t::PHOTO_VIEW, &[("compass", c.to_string()), ("az", format!("{}", a.round() as i64)),
                                             ("el", format!("{}", (-e).round() as i64))]);
            // steeply down: the frame is mostly the ground beside the rover, not the view ahead
            if e <= PHOTO_DOWN_DEG { format!("{line}{}", t::PHOTO_VIEW_DOWN) } else { line }
        }
        _ => String::new(),
    };
    Some(Photo {
        src: format!("assets/{f}"),
        alt: fill(t::PHOTO_ALT, &[("stop_name", stop_name.to_string())]),
        caption: fill(t::PHOTO_CAPTION, &[("sol", p["sol"].to_string()), ("credit", p["credit"].as_str().unwrap_or("").to_string())]),
        raw_url: p["url"].as_str().unwrap_or("").to_string(),
        az,
        view,
        borrowed,
    })
}

/// Camera elevation (deg) at or below which a photo is said to show mostly the ground beside the rover.
const PHOTO_DOWN_DEG: f64 = -30.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn photos_carry_their_view_direction() {
        let doc: Value = serde_json::from_slice(&std::fs::read("../assets/stops.json").unwrap()).unwrap();
        let by = |id: &str| doc["stops"].as_array().unwrap().iter().find(|s| s["id"] == id).unwrap()["photo"].clone();
        let rio = photo_of(&by("rio_frio"), "R\u{ed}o Fr\u{ed}o", false).unwrap();
        assert_eq!(rio.az.map(|a| a.round()), Some(238.0));
        assert!(rio.view.contains("WSW") && rio.view.ends_with(t::PHOTO_VIEW_DOWN), "{}", rio.view);
        let nev = photo_of(&by("nevado_sajama"), "Nevado Sajama", false).unwrap();
        assert!(nev.view.contains("NE") && !nev.view.ends_with(t::PHOTO_VIEW_DOWN), "{}", nev.view);
        assert!(photo_of(&serde_json::json!({"file": "x.jpg"}), "x", true).is_some_and(|p| p.view.is_empty() && p.az.is_none() && p.borrowed));
    }

    #[test]
    fn names_dates_and_tags() {
        assert_eq!(split_name("Valle de la Luna: hollow drill site"), ("Valle de la Luna".into(), "Hollow drill site".into()));
        assert_eq!(split_name("Ridge top to hollow edge"), ("Ridge top to hollow edge".into(), String::new()));
        assert_eq!(nice_date("2025-11-18"), "18 Nov 2025");
        assert_eq!(nice_date("bad"), "bad");
        assert_eq!(source_meta("https://www.jpl.nasa.gov/news/x/", "2026-02-23", ""), "NASA JPL \u{00b7} 23 Feb 2026");
        assert_eq!(source_meta("https://science.nasa.gov/blog/x", "2025-08-14", "Lucy Lim"), "NASA Science \u{00b7} 14 Aug 2025 \u{00b7} Lucy Lim");
        assert_eq!(position_tag("NOTIONAL: chosen by the plan").0, t::NOTIONAL);
        assert_eq!(position_tag("INFERRED: last fix").1, "Last fix");
        assert_eq!(position_tag("rover position on sol 4625").1, "Rover position on sol 4625");
        assert_eq!(position_tag("rover position on sol 4625").0, "");
    }

    #[test]
    fn station_plans_cite_facts_by_number_or_by_stop() {
        let doc: Value = serde_json::from_slice(&std::fs::read("../assets/stops.json").unwrap()).unwrap();
        let valle = doc["stops"].as_array().unwrap().iter().find(|s| s["id"] == "valle_de_la_luna").unwrap();
        let rows = plan_rows("valle_de_la_luna", &valle["tasks"], &doc);
        assert!(rows.len() >= 3, "{rows:?}");
        assert!(rows.iter().all(|r| !r.2.is_empty()), "every task cites a fact");
        assert!(rows.iter().any(|r| r.2.contains("Nevado Sajama [")), "a fact of another stop is cited by its name");
        assert!(rows.iter().any(|r| r.3), "the carbonate/clay link is marked inference");
        let exit = doc["stops"].as_array().unwrap().iter().find(|s| s["id"] == "exit").unwrap();
        assert!(plan_rows("exit", &exit["tasks"], &doc).is_empty());
    }
}