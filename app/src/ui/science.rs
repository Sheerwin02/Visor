//! Chemistry comparison for scientists (Science tab): per station, the mean of the ChemCam / APXS targets measured within
//! CHEM_R_M of it (the rows already on each station: scene::chem::near, placed at the rover position when measured), for
//! CaO, SiO2, FeOT and SO3 (wt%), with the number of targets; and the drill pair side by side (ridge top vs hollow).
//! Means of nearby targets: not a sample of the station's bedrock, and ChemCam spots are a few mm across.
use crate::ui::model::StopSummary;

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub const OXIDES: [&str; 4] = ["CaO", "SiO\u{2082}", "FeOT", "SO\u{2083}"];

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub struct StationChem {
    pub index: usize,
    pub label: String,
    pub pair: bool,
    /// mean wt% per oxide (OXIDES order); None when no nearby target reported it
    pub vals: [Option<f64>; 4],
    /// targets averaged
    pub n: usize,
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn matrix(cards: &[StopSummary]) -> Vec<StationChem> {
    cards.iter().filter(|s| !s.chem.is_empty()).map(|s| {
        let col = |k: usize| {
            let v: Vec<f64> = s.chem.iter().filter_map(|r| match k { 0 => r.4, 1 => r.5, 2 => r.6, _ => r.7 }).collect();
            (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
        };
        StationChem { index: s.index, label: s.label.clone(), pair: s.pair, vals: [col(0), col(1), col(2), col(3)], n: s.chem.len() }
    }).collect()
}

/// Largest value per oxide over the stations (for dot sizes); 1.0 when none.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn row_max(m: &[StationChem]) -> [f64; 4] {
    let mut out = [0.0f64; 4];
    for s in m {
        for (k, v) in s.vals.iter().enumerate() {
            if let Some(v) = v {
                out[k] = out[k].max(*v);
            }
        }
    }
    out.map(|x| if x > 0.0 { x } else { 1.0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn means_per_station_and_row_maxima() {
        let row = |cao: Option<f64>, so3: Option<f64>| ("ChemCam".to_string(), "t".to_string(), 1, 1.0f32, cao, Some(40.0), Some(18.0), so3);
        let a = StopSummary { index: 1, label: "2 A".into(), chem: vec![row(Some(4.0), None), row(Some(6.0), Some(2.0))], ..Default::default() };
        let b = StopSummary { index: 2, label: "3 B".into(), ..Default::default() };
        let m = matrix(&[a, b]);
        assert_eq!(m.len(), 1, "a station with no nearby target is left out");
        assert_eq!(m[0].vals[0], Some(5.0));
        assert_eq!(m[0].vals[3], Some(2.0), "SO3 from the one target that has it");
        assert_eq!(m[0].n, 2);
        assert_eq!(row_max(&m)[0], 6.0_f64.min(5.0).max(5.0));
    }

    #[test]
    fn real_bundle_drill_pair_has_chemistry() {
        let doc: serde_json::Value = serde_json::from_slice(&std::fs::read("../assets/chemistry.json").unwrap()).unwrap();
        let stops: serde_json::Value = serde_json::from_slice(&std::fs::read("../assets/stops.json").unwrap()).unwrap();
        let cards: Vec<StopSummary> = stops["stops"].as_array().unwrap().iter().enumerate().map(|(i, s)| {
            let (e, n) = (s["e_m"].as_f64().unwrap() as f32, s["n_m"].as_f64().unwrap() as f32);
            let (chem, _) = crate::scene::chem::near(&doc, e, n, crate::ui::views_consts::CHEM_R_M, 6);
            StopSummary { index: i, label: s["id"].as_str().unwrap().into(), pair: s.get("pair").is_some(), chem, ..Default::default() }
        }).collect();
        let m = matrix(&cards);
        let pair: Vec<&StationChem> = m.iter().filter(|s| s.pair).collect();
        assert_eq!(pair.len(), 2, "both drill sites have nearby targets: {m:?}");
        assert!(pair.iter().all(|s| s.vals[0].is_some()), "CaO at both");
    }
}
