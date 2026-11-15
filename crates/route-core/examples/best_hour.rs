//! Best-hour rule on the real bundle (best-hour rule fixed before any hour was computed).
//! Reports the full hour table for the default sol (4718) and the best start hour for every study sol (4534-4865), for the
//! published plan. Writes data/interim/best_hour/best_hour_result.json (local).
//!   cargo run --release -p route-core --example best_hour
use std::path::PathBuf;

use route_core::light::{best, box3, hours, timed_route, Heights};
use route_core::sun::planetographic;
use serde_json::{json, Value};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn j(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(root().join("assets").join(name)).unwrap()).unwrap()
}

fn row(r: &route_core::light::HourResult) -> Value {
    let r1 = |x: f64| (x * 10.0).round() / 10.0;
    json!({ "start_ltst": r.start_ltst, "passes": r.passes, "start_elev": r1(r.start_elev), "start_az": r1(r.start_az),
            "min_elev": r1(r.min_elev), "max_elev": r1(r.max_elev), "toward_low_sun_m": r1(r.toward_low_sun_m),
            "away_m": r1(r.away_m), "shadow_m": r1(r.shadow_m), "margin": r1(r.margin) })
}

fn main() {
    let man = j("manifest.json");
    let (w, h) = (man["grid"]["width"].as_u64().unwrap() as usize, man["grid"]["height"].as_u64().unwrap() as usize);
    let cell = man["grid"]["cell_m"].as_f64().unwrap();
    let z: Vec<f32> = std::fs::read(root().join("assets/heightmap.bin")).unwrap().chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    let heights = Heights::new(box3(&z, w, h), w, h, cell);
    let route = j("route.json");
    let poly: Vec<(f64, f64)> = route["polyline_en"].as_array().unwrap().iter().map(|p| (p[0].as_f64().unwrap(), p[1].as_f64().unwrap())).collect();
    let legs: Vec<(usize, usize, f64)> = route["legs"].as_array().unwrap().iter()
        .map(|l| (l["poly_start"].as_u64().unwrap() as usize, l["poly_end"].as_u64().unwrap() as usize, l["time_s"].as_f64().unwrap())).collect();
    let timed = timed_route(&poly, &legs);
    let exit = &j("stops.json")["stops"][0];
    let (west, lat_pg) = (360.0 - exit["lon"].as_f64().unwrap(), planetographic(exit["lat"].as_f64().unwrap()));
    eprintln!("site {west:.3} W, {lat_pg:.4} planetographic; walk {:.0} s over {} route points", timed.last().unwrap().2, timed.len());

    let t0 = std::time::Instant::now();
    let default = hours(&timed, &heights, 4718.0, west, lat_pg);
    let b = best(&default);
    let t_hours = t0.elapsed().as_secs_f64();
    let t1 = std::time::Instant::now();
    let r = &default[(16 - route_core::light::FIRST_HOUR) as usize];
    let mask = heights.shadow_mask(r.start_elev as f32, r.start_az as f32);
    let t_mask = t1.elapsed().as_secs_f64();
    let share = mask.iter().filter(|&&m| m == 255).count() as f64 / mask.len() as f64;

    let mut per_sol = Vec::new();
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for sol in 4534..=4865 {
        let rs = hours(&timed, &heights, sol as f64, west, lat_pg);
        let key = match best(&rs) {
            Ok(i) => format!("{:02}:00", rs[i].start_ltst as u32),
            Err(_) => "none".to_string(),
        };
        *counts.entry(key.clone()).or_default() += 1;
        per_sol.push(json!({ "sol": sol, "best": key, "passing": rs.iter().filter(|x| x.passes).count() }));
    }
    let out = json!({
        "rule": "doc: Best-hour rule (approved)",
        "site": { "west_lon": west, "lat_planetographic": lat_pg },
        "walk_s": timed.last().unwrap().2,
        "default_sol": 4718,
        "default_table": default.iter().map(row).collect::<Vec<_>>(),
        "default_best": match b { Ok(i) => json!(default[i].start_ltst), Err(n) => json!({ "none": true, "nearest": n.map(|i| default[i].start_ltst) }) },
        "best_start_counts_over_study_sols": counts,
        "per_sol": per_sol,
        "timing_native_s": { "hour_table": t_hours, "full_shadow_mask": t_mask },
        "shadow_share_of_grid_at_16h_sol_4718": share,
    });
    let dir = root().join("data/interim/best_hour");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("best_hour_result.json"), serde_json::to_string_pretty(&out).unwrap()).unwrap();
    println!("{}", serde_json::to_string_pretty(&json!({ "default_table": out["default_table"], "default_best": out["default_best"],
        "counts": out["best_start_counts_over_study_sols"], "timing": out["timing_native_s"], "shadow_share_16h": share })).unwrap());
}
