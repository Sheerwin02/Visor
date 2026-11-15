//! Station plans written by the planner. A stop's plan is VISOR's suggestion from stops.json (each task cites a sourced
//! fact) unless the planner has written their own here; a stop added on the map has only the planner's. Every reader of a
//! station plan (Station view, Walk station panel, cue cards, Log, GeoJSON) goes through `tasks_value`, so they all agree.
//! A written plan carries no citations and is labelled "your plan" wherever it is shown.
use std::collections::BTreeMap;
use std::sync::Mutex;

use bevy::prelude::*;
use serde_json::{json, Value};

/// One task as the planner writes it: step (question/observe/image/sample/compare), text, bag labels (sample tasks).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct EditTask {
    pub step: String,
    pub text: String,
    pub bags: Vec<String>,
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub const STEPS: [&str; 5] = ["question", "observe", "image", "sample", "compare"];

/// Planner-written plans by stop id.
#[derive(Resource, Clone, Debug, PartialEq, Default)]
pub struct StationEdits(pub BTreeMap<String, Vec<EditTask>>);

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub enum EditEvent {
    Set(String, Vec<EditTask>),
    /// back to VISOR's suggestion (or no plan for a stop added on the map)
    Clear(String),
    ReplaceAll(BTreeMap<String, Vec<EditTask>>),
}

pub static EVENTS: Mutex<Vec<EditEvent>> = Mutex::new(Vec::new());

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn send(e: EditEvent) {
    if let Ok(mut q) = EVENTS.lock() {
        q.push(e);
    }
}

/// The tasks of stop `id` in stops.json's schema: the planner's own (with `"user": true`, no refs) or stops.json's.
pub fn tasks_value(id: &str, doc: &Value, edits: &StationEdits) -> Value {
    if let Some(ts) = edits.0.get(id) {
        return Value::Array(ts.iter().filter(|t| !t.text.trim().is_empty()).map(|t| {
            let mut v = json!({"step": t.step, "text": t.text.trim(), "refs": [], "inference": false, "user": true});
            if t.step == "sample" {
                v["bags"] = json!(t.bags.iter().map(|b| b.trim()).filter(|b| !b.is_empty()).collect::<Vec<_>>());
            }
            v
        }).collect());
    }
    doc["stops"].as_array().and_then(|a| a.iter().find(|s| s["id"] == id)).map(|s| s["tasks"].clone())
        .filter(|t| t.is_array()).unwrap_or_else(|| json!([]))
}

/// A tasks array (stops.json schema) as editable tasks.
pub fn editable(tasks: &Value) -> Vec<EditTask> {
    tasks.as_array().into_iter().flatten().filter_map(|t| Some(EditTask {
        step: t["step"].as_str()?.to_string(),
        text: t["text"].as_str()?.to_string(),
        bags: t["bags"].as_array().map(|b| b.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default(),
    })).collect()
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn to_json(e: &BTreeMap<String, Vec<EditTask>>) -> Value {
    Value::Object(e.iter().map(|(k, ts)| (k.clone(), Value::Array(ts.iter().map(|t| json!({"step": t.step, "text": t.text, "bags": t.bags})).collect()))).collect())
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn from_json(v: &Value) -> BTreeMap<String, Vec<EditTask>> {
    v.as_object().into_iter().flatten().map(|(k, ts)| (k.clone(), editable(ts))).collect()
}

/// Apply edit events; when the plans change, refresh the station fields of the panel's stop list (the card follows
/// through card::update_card, the Walk panel through simrun).
pub fn update(mut edits: ResMut<StationEdits>, docs: Option<Res<crate::BundleDocs>>, mut bridge: ResMut<crate::ui::model::UiBridge>) {
    let ev: Vec<EditEvent> = EVENTS.lock().map(|mut q| q.drain(..).collect()).unwrap_or_default();
    if ev.is_empty() {
        return;
    }
    for e in ev {
        match e {
            EditEvent::Set(id, ts) => { edits.0.insert(id, ts); }
            EditEvent::Clear(id) => { edits.0.remove(&id); }
            EditEvent::ReplaceAll(m) => edits.0 = m,
        }
    }
    let (Some(docs), Some(mut pm)) = (docs, bridge.panel.clone()) else { return };
    for c in pm.stop_cards.iter_mut() {
        crate::ui::card::apply_tasks(c, &tasks_value(&c.id, &docs.stops_doc, &edits), &edits);
    }
    bridge.set_panel(pm);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planner_plans_replace_suggestions_and_round_trip() {
        let doc: Value = serde_json::from_slice(&std::fs::read("../assets/stops.json").unwrap()).unwrap();
        let mut e = StationEdits::default();
        let sugg = tasks_value("valle_de_la_luna", &doc, &e);
        assert!(sugg.as_array().unwrap().len() >= 3 && sugg[0]["refs"].as_array().is_some_and(|r| !r.is_empty()));
        assert_eq!(tasks_value("custom-1", &doc, &e), json!([]));
        e.0.insert("custom-1".into(), vec![
            EditTask { step: "question".into(), text: " Is the ridge cemented? ".into(), bags: vec![] },
            EditTask { step: "sample".into(), text: "Ridge crest".into(), bags: vec!["crest rock".into(), " ".into()] },
            EditTask { step: "observe".into(), text: "  ".into(), bags: vec![] },
        ]);
        let v = tasks_value("custom-1", &doc, &e);
        assert_eq!(v.as_array().unwrap().len(), 2, "blank tasks dropped");
        assert_eq!(v[0]["text"], "Is the ridge cemented?");
        assert_eq!(v[1]["bags"], json!(["crest rock"]));
        assert_eq!(v[1]["user"], true);
        assert_eq!(from_json(&to_json(&e.0)), e.0);
        assert_eq!(editable(&sugg).len(), sugg.as_array().unwrap().len());
    }
}
