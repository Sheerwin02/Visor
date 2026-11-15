//! Where a plan's time goes on the real bundle (native, release): legs vs the full walk-back field.
use route_core::{plan_loop, CostGrid, Rules};
use std::time::Instant;
fn main() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let j = |n: &str| -> serde_json::Value { serde_json::from_slice(&std::fs::read(root.join(n)).unwrap()).unwrap() };
    let (man, route, stops, rp) = (j("manifest.json"), j("route.json"), j("stops.json"), j("rover_path.json"));
    let (w, h) = (man["grid"]["width"].as_u64().unwrap() as usize, man["grid"]["height"].as_u64().unwrap() as usize);
    let a = &route["assumptions"];
    let f = |k: &str| a[k].as_f64().unwrap();
    let r = Rules { flat_mps: f("speed_flat_mps"), k: f("tobler_k"), bias_deg: f("bias_deg"), limit_deg: f("slope_limit_deg"), verified_r_m: f("verified_radius_m") };
    let slope: Vec<f32> = std::fs::read(root.join("slope_3m.bin")).unwrap().chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    let rover: Vec<(f64, f64)> = rp["points"].as_array().unwrap().iter().map(|p| (p["e_m"].as_f64().unwrap(), p["n_m"].as_f64().unwrap())).collect();
    let st: Vec<(f64, f64)> = stops["stops"].as_array().unwrap().iter().map(|s| (s["e_m"].as_f64().unwrap(), s["n_m"].as_f64().unwrap())).collect();
    for _ in 0..3 {
        let t0 = Instant::now();
        let g = CostGrid::new(&slope, w, h, 1.0, &r, &rover);
        let t1 = Instant::now();
        let _p = plan_loop(&g, &st, r.limit_deg);
        let t2 = Instant::now();
        let exit = g.index(st[0].0, st[0].1).unwrap();
        let _ = g.dijkstra(exit, None);
        let t3 = Instant::now();
        println!("grid {:.0} ms | plan_loop {:.0} ms | of which full walk-back field ~{:.0} ms", (t1 - t0).as_secs_f64() * 1e3,
                 (t2 - t1).as_secs_f64() * 1e3, (t3 - t2).as_secs_f64() * 1e3);
    }
}
