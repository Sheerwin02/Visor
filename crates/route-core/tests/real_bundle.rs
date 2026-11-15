//! The Rust planner must reproduce the Python reference (tools/route.py, written into assets/ by build_bundle) on the real
//! bundle: same leg times (route.json rounds to 1 s), same lengths (up to equal-time ties), same walk-back time in every
//! cell (walk_back_s.bin rounds to 1 s) and the same reachable cells. Skipped when assets/ is absent.
use std::path::PathBuf;

use route_core::{plan_loop, CostGrid, Rules};
use serde_json::Value;

fn assets() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets")
}

fn json(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(assets().join(name)).unwrap()).unwrap()
}

#[test]
fn reproduces_the_python_route_on_the_real_bundle() {
    if !assets().join("route.json").exists() {
        eprintln!("assets/ not found: skipped");
        return;
    }
    let man = json("manifest.json");
    let (w, h, cell) = (man["grid"]["width"].as_u64().unwrap() as usize, man["grid"]["height"].as_u64().unwrap() as usize,
                        man["grid"]["cell_m"].as_f64().unwrap());
    let route = json("route.json");
    let a = &route["assumptions"];
    let f = |k: &str| a[k].as_f64().unwrap();
    let rules = Rules { flat_mps: f("speed_flat_mps"), k: f("tobler_k"), bias_deg: f("bias_deg"), limit_deg: f("slope_limit_deg"),
                        verified_r_m: f("verified_radius_m") };
    let slope: Vec<f32> = std::fs::read(assets().join("slope_3m.bin")).unwrap().chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    let rover: Vec<(f64, f64)> = json("rover_path.json")["points"].as_array().unwrap().iter()
        .map(|p| (p["e_m"].as_f64().unwrap(), p["n_m"].as_f64().unwrap())).collect();
    let stops: Vec<(f64, f64)> = json("stops.json")["stops"].as_array().unwrap().iter()
        .map(|s| (s["e_m"].as_f64().unwrap(), s["n_m"].as_f64().unwrap())).collect();

    let t0 = std::time::Instant::now();
    let g = CostGrid::new(&slope, w, h, cell, &rules, &rover);
    let plan = plan_loop(&g, &stops, rules.limit_deg);
    eprintln!("native plan: {:.0} ms", t0.elapsed().as_secs_f64() * 1e3);

    assert_eq!(g.blocked.iter().filter(|&&b| b).count() as u64, route["blocked_cells"].as_u64().unwrap(), "blocked cells");
    assert_eq!(g.verified.iter().filter(|&&b| b).count() as u64, route["verified_cells"].as_u64().unwrap(), "verified cells");
    assert_eq!(plan.all_legs_possible, route["all_legs_possible"].as_bool().unwrap());
    for (i, (l, r)) in plan.legs.iter().zip(route["legs"].as_array().unwrap()).enumerate() {
        let rt = r["time_s"].as_f64().unwrap();
        assert!((l.time_s - rt).abs() <= 0.5 + 1e-9, "leg {i}: time {} vs Python {rt}", l.time_s);
        let rl = r["length_m"].as_f64().unwrap();
        assert!((l.length_m - rl).abs() <= 1.0, "leg {i}: length {} vs Python {rl} (ties may differ slightly)", l.length_m);
        let rs = r["max_corrected_slope_deg"].as_f64().unwrap();
        assert!((l.max_corrected_slope_deg - rs).abs() <= 0.5, "leg {i}: max slope {} vs Python {rs}", l.max_corrected_slope_deg);
    }
    // walk-back time in every cell: same reachability, within the 1 s rounding of walk_back_s.bin
    let wb: Vec<u16> = std::fs::read(assets().join("walk_back_s.bin")).unwrap().chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let no_way = man["files"]["walk_back_s.bin"]["no_way_back"].as_u64().unwrap() as u16;
    let mut worst = 0.0f64;
    for (i, (&py, &rs)) in wb.iter().zip(&plan.walk_back).enumerate() {
        assert_eq!(py == no_way, !rs.is_finite(), "cell {i}: reachability differs (Python {py}, Rust {rs})");
        if rs.is_finite() {
            worst = worst.max((rs - py as f64).abs());
        }
    }
    // walk_back_s.bin holds whole seconds (numpy rounds half to even), and tools/route.py computes speeds and edge costs in
    // float32 (numpy 2 keeps float32 when a float32 array meets a Python float; checked) while this crate uses f64:
    // a cell near x.5 s can differ by 0.5 s plus ~1e-5 s. 0.501 s allows that and nothing more.
    assert!(worst <= 0.501, "walk-back differs by up to {worst} s");
}
