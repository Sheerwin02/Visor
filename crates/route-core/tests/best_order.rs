//! On the real bundle: the exact best visiting order is never slower than the published order, and planning the loop in
//! that order gives the time the tour search predicted. Prints both, for the record. Skipped when assets/ is absent.
use std::path::PathBuf;

use route_core::{plan_loop, times_between, CostGrid, Rules, Tours};
use serde_json::Value;

fn assets() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets")
}

fn json(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(assets().join(name)).unwrap()).unwrap()
}

#[test]
fn best_order_is_never_slower_than_the_published_one() {
    if !assets().join("route.json").exists() {
        eprintln!("assets/ not found: skipped");
        return;
    }
    let man = json("manifest.json");
    let (w, h) = (man["grid"]["width"].as_u64().unwrap() as usize, man["grid"]["height"].as_u64().unwrap() as usize);
    let route = json("route.json");
    let a = &route["assumptions"];
    let f = |k: &str| a[k].as_f64().unwrap();
    let rules = Rules { flat_mps: f("speed_flat_mps"), k: f("tobler_k"), bias_deg: f("bias_deg"), limit_deg: f("slope_limit_deg"),
                        verified_r_m: f("verified_radius_m") };
    let slope: Vec<f32> = std::fs::read(assets().join("slope_3m.bin")).unwrap().chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    let rover: Vec<(f64, f64)> = json("rover_path.json")["points"].as_array().unwrap().iter()
        .map(|p| (p["e_m"].as_f64().unwrap(), p["n_m"].as_f64().unwrap())).collect();
    let stops_v = json("stops.json");
    let st = stops_v["stops"].as_array().unwrap();
    let stops: Vec<(f64, f64)> = st.iter().map(|s| (s["e_m"].as_f64().unwrap(), s["n_m"].as_f64().unwrap())).collect();
    let g = CostGrid::new(&slope, w, h, 1.0, &rules, &rover);
    let t0 = std::time::Instant::now();
    let times = times_between(&g, &stops);
    let tours = Tours::new(&times).unwrap();
    let (best, order) = tours.best_all().unwrap();
    let ms = t0.elapsed().as_secs_f64() * 1e3;
    let published: f64 = (0..stops.len()).map(|i| times[i][(i + 1) % stops.len()]).sum();
    let names: Vec<&str> = order.iter().map(|&i| st[i]["id"].as_str().unwrap()).collect();
    eprintln!("published order {published:.0} s; best order {best:.0} s ({:?}); pairwise times + search {ms:.0} ms native", names);
    assert!(best <= published + 1e-6);
    let reordered: Vec<(f64, f64)> = order.iter().map(|&i| stops[i]).collect();
    let p = plan_loop(&g, &reordered, rules.limit_deg);
    assert!((p.total_time_s - best).abs() < 1e-6, "plan in the best order takes {} s, search said {best}", p.total_time_s);
}
