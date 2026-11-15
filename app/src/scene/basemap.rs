//! Basemap selector (NASA Mars Trek as a second imagery source). The terrain's photo can be switched between our
//! USGS 25 cm photo (the measurement map: default) and NASA Mars Trek layers composited over the patch by tools/trek.py
//! (assets/trek_layers.json + trek_<label>.jpg): view only, each labelled with its resolution. Images are fetched the first
//! time they are chosen (optional files; the app works without them).
use bevy::asset::RenderAssetUsages;
use bevy::image::{CompressedImageFormats, ImageSampler, ImageType};
use bevy::prelude::*;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;

use crate::text::{fill, ui as u};

/// A basemap must cover the patch with at least this many pixels across to be offered on the terrain (VISUAL choice).
pub const MIN_PATCH_PX: f64 = 100.0;

/// One basemap choice.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    /// None = the USGS photo already on the terrain
    pub file: Option<String>,
    pub title: String,
    pub detail: String,
    pub tip: String,
}

/// The choices: USGS first, then every Trek layer that has a basemap image.
pub fn choices(trek: Option<&Value>) -> Vec<Choice> {
    let mut v = vec![Choice { file: None, title: u::L_PHOTO.to_string(), detail: u::S_MRO.to_string(), tip: u::T_PHOTO.to_string() }];
    if let Some(layers) = trek.and_then(|t| t["layers"].as_array()) {
        for l in layers {
            let (Some(file), Some(short), Some(m)) = (l["file"].as_str(), l["short"].as_str(), l["m_per_px"].as_f64()) else { continue };
            // walk-scale basemaps only: a layer that covers the patch with fewer than MIN_PATCH_PX pixels across is a grey smear
            // here (CTX 41 m/px, THEMIS 81 m/px); those stay in the Trust tab and, for THEMIS, in the Mars view
            if l["patch_px"][0].as_f64().unwrap_or(0.0) < MIN_PATCH_PX {
                continue;
            }
            let m_s = if m >= 10.0 { format!("{}", m.round() as i64) } else { format!("{m:.2}") };
            v.push(Choice {
                file: Some(file.to_string()),
                title: short.to_string(),
                detail: fill(u::S_TREK, &[("x", m_s.clone())]),
                tip: fill(u::T_TREK, &[("label", l["title"].as_str().unwrap_or(short).to_string()), ("x", m_s)]),
            });
        }
    }
    v
}

#[derive(Resource)]
pub struct Basemaps {
    pub usgs: Handle<Image>,
    pub choices: Vec<Choice>,
    pub selected: usize,
    pub loaded: HashMap<String, Handle<Image>>,
    pub pending: Option<String>,
    pub error: Option<String>,
}

impl Basemaps {
    pub fn new(usgs: Handle<Image>, trek: Option<&Value>) -> Self {
        Basemaps { usgs, choices: choices(trek), selected: 0, loaded: HashMap::new(), pending: None, error: None }
    }
}

/// Basemap images that arrived (main.rs `receive` routes trek_*.jpg here).
pub static ARRIVED: Mutex<Vec<(String, Result<Vec<u8>, String>)>> = Mutex::new(Vec::new());
pub static EVENTS: Mutex<Vec<usize>> = Mutex::new(Vec::new());

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn select(i: usize) {
    if let Ok(mut q) = EVENTS.lock() {
        q.push(i);
    }
}

pub fn is_basemap_file(name: &str) -> bool {
    name.starts_with("trek_") && name.ends_with(".jpg")
}

/// Apply a choice: request its image if needed, swap the terrain photo when it is there.
pub fn update(
    inbox: Res<crate::bundle::Inbox>,
    bm: Option<ResMut<Basemaps>>,
    overlays: Option<Res<crate::scene::trust::Overlays>>,
    mut mats: ResMut<Assets<crate::scene::trust::TerrainMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let (Some(mut bm), Some(ov)) = (bm, overlays) else { return };
    let mut changed = false;
    for i in EVENTS.lock().map(|mut q| q.drain(..).collect::<Vec<_>>()).unwrap_or_default() {
        if i < bm.choices.len() && i != bm.selected {
            bm.selected = i;
            bm.error = None;
            changed = true;
            if let Some(f) = bm.choices[i].file.clone() {
                if !bm.loaded.contains_key(&f) && bm.pending.as_deref() != Some(f.as_str()) {
                    bm.pending = Some(f.clone());
                    inbox.request(&f);
                }
            }
        }
    }
    for (name, r) in ARRIVED.lock().map(|mut q| q.drain(..).collect::<Vec<_>>()).unwrap_or_default() {
        if bm.pending.as_deref() == Some(name.as_str()) {
            bm.pending = None;
        }
        match r.and_then(|b| Image::from_buffer(&b, ImageType::Extension("jpg"), CompressedImageFormats::NONE, true, ImageSampler::linear(),
                                                   RenderAssetUsages::RENDER_WORLD).map_err(|e| e.to_string())) {
            Ok(img) => {
                let h = images.add(crate::scene::mipmap::with_mips(img));
                bm.loaded.insert(name, h);
                changed = true;
            }
            Err(e) => {
                warn!("basemap {name}: {e}");
                bm.error = Some(fill(u::TREK_FAILED, &[("file", name)]));
                bm.selected = 0;
                changed = true;
            }
        }
    }
    if !changed {
        return;
    }
    let want = match bm.choices.get(bm.selected).and_then(|c| c.file.as_ref()) {
        None => Some(bm.usgs.clone()),
        Some(f) => bm.loaded.get(f).cloned(),
    };
    if let Some(h) = want {
        if let Some(mut m) = mats.get_mut(&ov.material) {
            if m.base.base_color_texture.as_ref() != Some(&h) {
                m.base.base_color_texture = Some(h);
            }
        }
    }
}

/// Planetocentric lat / east lon (deg) of a point on the grid (east and north metres from its south-west corner).
pub fn latlon(grid: &Value, e: f64, n: f64) -> Option<(f64, f64)> {
    let r = 3_396_190.0f64; // Mars 2000 sphere, the grid's own projection (manifest grid.projection)
    let (xw, yn, h, cell) = (grid["x_west_m"].as_f64()?, grid["y_north_m"].as_f64()?, grid["height"].as_f64()?, grid["cell_m"].as_f64()?);
    Some((((yn - h * cell + n) / r).to_degrees(), ((xw + e) / r).to_degrees()))
}

/// "Open in NASA Mars Trek" link (trek_layers.json deep_link: the format Trek's own address bar uses), centred on (lat, lon)
/// with a box of +-half_deg, or on the whole patch when None.
pub fn trek_link(trek: &Value, centre: Option<(f64, f64)>, half_deg: f64) -> Option<String> {
    let t = trek["deep_link"].as_str()?;
    let z = trek["deep_link_zoom"].as_i64().unwrap_or(16);
    let (lat, lon, w, s, e, n) = match centre {
        Some((la, lo)) => (la, lo, lo - half_deg, la - half_deg, lo + half_deg, la + half_deg),
        None => {
            let b = trek["patch_bbox_deg"].as_array()?;
            let v: Vec<f64> = b.iter().filter_map(|x| x.as_f64()).collect();
            if v.len() != 4 {
                return None;
            }
            ((v[1] + v[3]) / 2.0, (v[0] + v[2]) / 2.0, v[0], v[1], v[2], v[3])
        }
    };
    let f = |x: f64| format!("{x:.6}");
    Some(t.replace("{lon}", &f(lon)).replace("{lat}", &f(lat)).replace("{z}", &z.to_string())
          .replace("{w}", &f(w)).replace("{s}", &f(s)).replace("{e}", &f(e)).replace("{n}", &f(n)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trek_link_centres_on_the_patch_and_on_a_stop() {
        let t: Value = serde_json::from_slice(&std::fs::read("../assets/trek_layers.json").unwrap()).unwrap();
        let l = trek_link(&t, None, 0.0).unwrap();
        assert!(l.starts_with("https://trek.nasa.gov/mars/#") && l.contains("&x=137.38") && l.contains("&y=-4.80") && !l.contains('{'), "{l}");
        let s = trek_link(&t, Some((-4.81, 137.381)), 0.001).unwrap();
        assert!(s.contains("&x=137.381000") && s.contains("&y=-4.810000") && s.contains("137.380000%2C-4.811000"), "{s}");
        let m: Value = serde_json::from_slice(&std::fs::read("../assets/manifest.json").unwrap()).unwrap();
        let (la, lo) = latlon(&m["grid"], 0.0, 0.0).unwrap();
        let b = t["patch_bbox_deg"].as_array().unwrap();
        assert!((lo - b[0].as_f64().unwrap()).abs() < 1e-5 && (la - b[1].as_f64().unwrap()).abs() < 1e-5, "south-west corner");
    }

    #[test]
    fn choices_list_usgs_first_then_trek_basemaps_with_resolution() {
        let t: Value = serde_json::from_slice(&std::fs::read("../assets/trek_layers.json").unwrap()).unwrap();
        let c = choices(Some(&t));
        assert!(c[0].file.is_none());
        let with_file = t["layers"].as_array().unwrap().iter()
            .filter(|l| l["file"].is_string() && l["patch_px"][0].as_f64().unwrap_or(0.0) >= MIN_PATCH_PX).count();
        assert_eq!(c.len(), 1 + with_file);
        assert!(c[1..].iter().all(|x| x.detail.contains("m/px") && x.file.as_deref().is_some_and(is_basemap_file)), "{c:#?}");
        assert_eq!(choices(None).len(), 1);
    }
}
