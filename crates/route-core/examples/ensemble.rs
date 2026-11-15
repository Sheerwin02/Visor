//! Route uncertainty ensemble on the real bundle (rule fixed before any run).
//! Run after tools/build_bundle.py build, then hash the outputs into the manifest:
//!   cargo run --release -p route-core --example ensemble
//!   python tools/build_bundle.py rehash && python tools/validate_bundle.py
//! Reads assets/ (route.json, stops.json, rover_path.json, slope_3m.bin, manifest.json) and
//! data/interim/rover_check/rover_check_stops.csv (d = DEM tilt - rover tilt per stop, in stop order).
//! Writes assets/route_ensemble.json and assets/route_usage.bin (u8 per cell, row 0 north: round(255 x share of runs)).
//! The rule is not changed after a result; a new question gets a new rule.
use std::collections::HashSet;
use std::path::PathBuf;

use route_core::corridor::{buffer_shares, pdi};
use route_core::ensemble::{block_bootstrap_median, map_to_empirical, median, quantile, smooth_field, spearman, Rng};
use route_core::{plan_loop, CostGrid, Rules};
use serde_json::{json, Value};

const SEED: u64 = 20261112;
const BLOCK: usize = 8;
const SPEED: (f64, f64) = (0.4, 0.8);
const K: (f64, f64) = (2.5, 7.5);
const LIMIT: (f64, f64) = (15.0, 25.0);
const LS_M: [f64; 2] = [3.0, 10.0];
const START: usize = 200;
const STEP: usize = 50;
const MAX_RUNS: usize = 1000;
const FIELD_ONLY_RUNS: usize = 200;

struct Inputs {
    w: usize,
    h: usize,
    cell: f64,
    slope: Vec<f32>,
    rover: Vec<(f64, f64)>,
    stops: Vec<(f64, f64)>,
    stop_ids: Vec<String>,
    published: Rules,
    d: Vec<f64>,
    resid_sorted: Vec<f64>,
}

#[derive(Clone)]
struct Run {
    feasible: bool,
    time_s: f64,
    length_m: f64,
    steepest: f64,
    walk_back: Vec<Option<f64>>,
    scalars: [f64; 4], // bias, speed, k, limit
    cells: Vec<u32>,
    /// each leg's route (None = impossible) and the whole loop, in e/n metres (for the corridor rule)
    legs_poly: Vec<Option<Vec<(f64, f64)>>>,
    loop_poly: Vec<(f64, f64)>,
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn json_file(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(root().join("assets").join(name)).expect(name)).expect(name)
}

fn load() -> (Inputs, Value) {
    let man = json_file("manifest.json");
    let (w, h, cell) = (man["grid"]["width"].as_u64().unwrap() as usize, man["grid"]["height"].as_u64().unwrap() as usize,
                        man["grid"]["cell_m"].as_f64().unwrap());
    let route = json_file("route.json");
    let a = &route["assumptions"];
    let f = |k: &str| a[k].as_f64().unwrap();
    let published = Rules { flat_mps: f("speed_flat_mps"), k: f("tobler_k"), bias_deg: f("bias_deg"), limit_deg: f("slope_limit_deg"),
                            verified_r_m: f("verified_radius_m") };
    let slope: Vec<f32> = std::fs::read(root().join("assets/slope_3m.bin")).unwrap().chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    let rover = json_file("rover_path.json")["points"].as_array().unwrap().iter()
        .map(|p| (p["e_m"].as_f64().unwrap(), p["n_m"].as_f64().unwrap())).collect();
    let sj = json_file("stops.json");
    let st = sj["stops"].as_array().unwrap();
    let stops = st.iter().map(|s| (s["e_m"].as_f64().unwrap(), s["n_m"].as_f64().unwrap())).collect();
    let stop_ids = st.iter().map(|s| s["id"].as_str().unwrap().to_string()).collect();
    let csv = std::fs::read_to_string(root().join("data/interim/rover_check/rover_check_stops.csv")).expect("rover_check_stops.csv");
    let mut lines = csv.lines();
    let head: Vec<&str> = lines.next().unwrap().split(',').collect();
    let col = |n: &str| head.iter().position(|h| *h == n).unwrap();
    let (cd, cr) = (col("dem_tilt"), col("rover_tilt"));
    let d: Vec<f64> = lines.filter(|l| !l.trim().is_empty()).map(|l| {
        let v: Vec<&str> = l.split(',').collect();
        v[cd].parse::<f64>().unwrap() - v[cr].parse::<f64>().unwrap()
    }).collect();
    let md = median(&d);
    let mut resid_sorted: Vec<f64> = d.iter().map(|x| x - md).collect();
    resid_sorted.sort_by(|a, b| a.total_cmp(b));
    (Inputs { w, h, cell, slope, rover, stops, stop_ids, published, d, resid_sorted }, route)
}

/// One run. `field_l`: correlation length of the error field (None = no field); `scalars`: vary the once-per-run inputs.
fn run_one(inp: &Inputs, ens: u64, i: u64, field_l: Option<f64>, scalars: bool) -> Run {
    let mut rng = Rng::for_run(SEED, ens, i);
    let mut r = inp.published.clone();
    if scalars {
        r.bias_deg = block_bootstrap_median(&inp.d, BLOCK, &mut rng);
        r.flat_mps = rng.range(SPEED.0, SPEED.1);
        r.k = rng.range(K.0, K.1);
        r.limit_deg = rng.range(LIMIT.0, LIMIT.1);
    }
    let field = field_l.map(|l| {
        let mut f = smooth_field(inp.w, inp.h, inp.cell, l, &mut rng);
        map_to_empirical(&mut f, &inp.resid_sorted);
        f
    });
    let g = CostGrid::with_error(&inp.slope, inp.w, inp.h, inp.cell, &r, &inp.rover, field.as_deref());
    let p = plan_loop(&g, &inp.stops, r.limit_deg);
    let cells: HashSet<u32> = p.polyline_en.iter().filter_map(|&(e, n)| g.index(e, n)).map(|k| k as u32).collect();
    Run {
        feasible: p.all_legs_possible,
        time_s: p.total_time_s,
        length_m: p.total_length_m,
        steepest: p.legs.iter().filter(|l| l.possible).map(|l| l.max_corrected_slope_deg).fold(0.0, f64::max),
        walk_back: p.walk_back_stops.clone(),
        scalars: [r.bias_deg, r.flat_mps, r.k, r.limit_deg],
        cells: cells.into_iter().collect(),
        legs_poly: p.legs.iter().map(|l| l.possible.then(|| p.polyline_en[l.poly_start..=l.poly_end].to_vec())).collect(),
        loop_poly: p.polyline_en.clone(),
    }
}

/// Runs lo..hi in parallel; results in run order.
fn batch(inp: &Inputs, ens: u64, lo: usize, hi: usize, field_l: Option<f64>, scalars: bool) -> Vec<Run> {
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(16);
    let idx: Vec<usize> = (lo..hi).collect();
    let chunk = idx.len().div_ceil(threads).max(1);
    let mut out: Vec<(usize, Run)> = std::thread::scope(|s| {
        let hs: Vec<_> = idx.chunks(chunk).map(|c| s.spawn(move || c.iter().map(|&i| (i, run_one(inp, ens, i as u64, field_l, scalars))).collect::<Vec<_>>())).collect();
        hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    out.sort_by_key(|(i, _)| *i);
    out.into_iter().map(|(_, r)| r).collect()
}

fn pct(xs: &[f64]) -> Value {
    if xs.is_empty() {
        return Value::Null;
    }
    let r = |x: f64| (x * 10.0).round() / 10.0;
    json!({ "p10": r(quantile(xs, 0.1)), "p50": r(quantile(xs, 0.5)), "p90": r(quantile(xs, 0.9)) })
}

fn usage(runs: &[Run], n_cells: usize) -> Vec<u32> {
    let mut u = vec![0u32; n_cells];
    for r in runs {
        for &c in &r.cells {
            u[c as usize] += 1;
        }
    }
    u
}

/// Convergence numbers at the current run count: time p10/p50/p90 (possible runs) and the share of ever-used cells at >= 80%.
fn check(runs: &[Run], n_cells: usize) -> (f64, f64, f64, f64) {
    let t: Vec<f64> = runs.iter().filter(|r| r.feasible).map(|r| r.time_s).collect();
    if t.is_empty() {
        return (f64::NAN, f64::NAN, f64::NAN, f64::NAN); // never stable: runs continue to the maximum
    }
    let u = usage(runs, n_cells);
    let used = u.iter().filter(|&&c| c > 0).count() as f64;
    let hi = u.iter().filter(|&&c| c as f64 >= 0.8 * runs.len() as f64).count() as f64;
    (quantile(&t, 0.1), quantile(&t, 0.5), quantile(&t, 0.9), hi / used)
}

fn summary(inp: &Inputs, runs: &[Run], l: Option<f64>, converged: Option<bool>, conv: Vec<Value>) -> Value {
    let n_cells = inp.w * inp.h;
    let ok: Vec<&Run> = runs.iter().filter(|r| r.feasible).collect();
    let t: Vec<f64> = ok.iter().map(|r| r.time_s).collect();
    let u = usage(runs, n_cells);
    let n = runs.len() as f64;
    let class = |lo: f64, hi: f64| u.iter().filter(|&&c| c > 0 && (c as f64) / n >= lo && (c as f64) / n < hi).count();
    let wb: Vec<Value> = inp.stop_ids.iter().enumerate().map(|(k, id)| {
        let v: Vec<f64> = ok.iter().filter_map(|r| r.walk_back[k]).collect();
        let p = |q: f64| if v.is_empty() { Value::Null } else { json!((quantile(&v, q) * 10.0).round() / 10.0) };
        json!({ "id": id, "p50": p(0.5), "p90": p(0.9), "runs": v.len() })
    }).collect();
    let sp = |j: usize| {
        let x: Vec<f64> = ok.iter().map(|r| r.scalars[j]).collect();
        if x.iter().all(|v| *v == x[0]) { Value::Null } else { json!((spearman(&x, &t) * 1000.0).round() / 1000.0) }
    };
    json!({
        "correlation_length_m": l, "runs": runs.len(), "converged": converged, "convergence": conv,
        "impossible_runs": runs.len() - ok.len(),
        "time_s": pct(&t), "length_m": pct(&ok.iter().map(|r| r.length_m).collect::<Vec<_>>()),
        "steepest_corrected_deg": pct(&ok.iter().map(|r| r.steepest).collect::<Vec<_>>()),
        "walk_back_s": wb,
        "spearman_time_vs": { "bias": sp(0), "speed": sp(1), "k": sp(2), "limit": sp(3) },
        "usage_cells": { "at_least_80pct": class(0.8, 1.01), "20_to_80pct": class(0.2, 0.8), "under_20pct": class(0.0, 0.2) },
    })
}

const BUFFERS_M: [f64; 3] = [1.0, 3.0, 5.0];
const STEP_M: f64 = 0.25;
const AGREE_SHARE: f64 = 0.9; // share of length within 3 m (the corridor rule's choice)

fn r3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

/// Corridor rule (fixed in advance): regenerate the stored runs, check them, compare each with the published route.
fn corridor(inp: &Inputs, route: &Value) {
    let path = root().join("assets/route_ensemble.json");
    let mut out: Value = serde_json::from_slice(&std::fs::read(&path).expect("run the ensemble first")).unwrap();
    let pubpoly: Vec<(f64, f64)> = route["polyline_en"].as_array().unwrap().iter()
        .map(|p| (p[0].as_f64().unwrap(), p[1].as_f64().unwrap())).collect();
    let publegs: Vec<Vec<(f64, f64)>> = route["legs"].as_array().unwrap().iter()
        .map(|l| pubpoly[l["poly_start"].as_u64().unwrap() as usize..=l["poly_end"].as_u64().unwrap() as usize].to_vec()).collect();
    let names: Vec<String> = (0..publegs.len()).map(|k| format!("{} to {}", inp.stop_ids[k], inp.stop_ids[(k + 1) % inp.stop_ids.len()])).collect();
    let mut results = Vec::new();
    for (e, ens) in out["ensembles"].as_array().unwrap().clone().iter().enumerate() {
        let n = ens["runs"].as_u64().unwrap() as usize;
        let l = ens["correlation_length_m"].as_f64().unwrap();
        let runs = batch(inp, e as u64 + 1, 0, n, Some(l), true);
        let t: Vec<f64> = runs.iter().filter(|r| r.feasible).map(|r| r.time_s).collect();
        let same = pct(&t) == ens["time_s"];
        eprintln!("l = {l} m: regenerated {n} runs; time percentiles match the stored ones: {same}");
        if !same {
            eprintln!("regenerated runs differ from the stored ensemble: nothing else runs (corridor rule item 1)");
            std::process::exit(1);
        }
        let summarise = |shares: &[Vec<f64>], pdis: &[(f64, f64)], impossible: usize| {
            let col = |j: usize| shares.iter().map(|s| s[j]).collect::<Vec<f64>>();
            let p3 = |xs: &[f64]| if xs.is_empty() { Value::Null } else {
                json!({ "p10": r3(quantile(xs, 0.1)), "p50": r3(quantile(xs, 0.5)), "p90": r3(quantile(xs, 0.9)) }) };
            json!({
                "runs": shares.len() + impossible, "impossible": impossible,
                "agree_runs": shares.iter().filter(|s| s[1] >= AGREE_SHARE).count(),
                "share_within_1m": p3(&col(0)), "share_within_3m": p3(&col(1)), "share_within_5m": p3(&col(2)),
                "pdi_m": p3(&pdis.iter().map(|p| p.0).collect::<Vec<_>>()),
                "pdi_pct": p3(&pdis.iter().map(|p| p.1).collect::<Vec<_>>()),
            })
        };
        let mut legs = Vec::new();
        for (k, pl) in publegs.iter().enumerate() {
            let mut shares = Vec::new();
            let mut pdis = Vec::new();
            let mut impossible = 0;
            for r in &runs {
                match &r.legs_poly[k] {
                    Some(rl) => {
                        shares.push(buffer_shares(rl, pl, &BUFFERS_M, STEP_M));
                        if let Some(p) = pdi(rl, pl, STEP_M) {
                            pdis.push(p);
                        }
                    }
                    None => impossible += 1,
                }
            }
            let mut v = summarise(&shares, &pdis, impossible);
            v["leg"] = json!(names[k]);
            legs.push(v);
        }
        let ok: Vec<&Run> = runs.iter().filter(|r| r.feasible).collect();
        let loop_shares: Vec<Vec<f64>> = ok.iter().map(|r| buffer_shares(&r.loop_poly, &pubpoly, &BUFFERS_M, STEP_M)).collect();
        let mut lp = summarise(&loop_shares, &[], runs.len() - ok.len());
        lp.as_object_mut().unwrap().remove("pdi_m");
        lp.as_object_mut().unwrap().remove("pdi_pct");
        results.push(json!({ "correlation_length_m": l, "loop": lp, "legs": legs }));
    }
    out["corridor"] = json!({ "rule": "doc: Corridor rule (approved)", "buffers_m": BUFFERS_M, "sample_step_m": STEP_M,
                              "agree": "share of length within 3 m >= 0.9", "ensembles": results });
    std::fs::write(&path, serde_json::to_string_pretty(&out).unwrap()).unwrap();
    println!("{}", serde_json::to_string_pretty(&out["corridor"]).unwrap());
}

fn main() {
    let t0 = std::time::Instant::now();
    let (inp, route) = load();
    if std::env::args().any(|a| a == "--corridor") {
        corridor(&inp, &route);
        eprintln!("corridor done in {:.1} s", t0.elapsed().as_secs_f64());
        return;
    }
    let n_cells = inp.w * inp.h;
    eprintln!("inputs: {} x {} grid, {} rover stops in the error model, {} stops", inp.w, inp.h, inp.d.len(), inp.stops.len());

    // 4. zero-noise check: published inputs, no field -> the published plan
    let z = run_one(&inp, 0, 0, None, false);
    let pub_time = route["total_time_s"].as_f64().unwrap();
    let g0 = CostGrid::new(&inp.slope, inp.w, inp.h, inp.cell, &inp.published, &inp.rover);
    let pub_cells: HashSet<u32> = route["polyline_en"].as_array().unwrap().iter()
        .filter_map(|p| g0.index(p[0].as_f64().unwrap(), p[1].as_f64().unwrap())).map(|k| k as u32).collect();
    let z_cells: HashSet<u32> = z.cells.iter().copied().collect();
    let zero_ok = z.feasible && (z.time_s - pub_time).abs() <= 1.0 && z_cells == pub_cells;
    eprintln!("zero-noise check: time {:.1} s vs published {pub_time} s; route cells match: {}; {}", z.time_s, z_cells == pub_cells,
              if zero_ok { "OK" } else { "FAILED" });
    let mut out = json!({
        "rule": "doc: Route uncertainty rule (approved)", "seed": SEED,
        "inputs": { "published_total_time_s": pub_time, "bias_deg": inp.published.bias_deg, "rover_check_stops": inp.d.len(),
                    "speed_mps": SPEED, "k": K, "limit_deg": LIMIT, "bias_block": BLOCK, "correlation_lengths_m": LS_M },
        "zero_noise": { "ok": zero_ok, "time_s": (z.time_s * 10.0).round() / 10.0, "route_cells_match": z_cells == pub_cells },
    });
    if !zero_ok {
        std::fs::write(root().join("assets/route_ensemble.json"), serde_json::to_string_pretty(&out).unwrap()).unwrap();
        eprintln!("zero-noise check failed: nothing else runs (rule item 4)");
        std::process::exit(1);
    }

    // 5. two ensembles with the stopping rule
    let mut ens = Vec::new();
    let mut all_runs = Vec::new();
    for (e, &l) in LS_M.iter().enumerate() {
        let mut runs = batch(&inp, e as u64 + 1, 0, START, Some(l), true);
        let mut prev = check(&runs, n_cells);
        let mut conv = vec![json!({ "runs": runs.len(), "p10": prev.0, "p50": prev.1, "p90": prev.2, "share_ge80": prev.3 })];
        let mut converged = false;
        while runs.len() < MAX_RUNS {
            let more = batch(&inp, e as u64 + 1, runs.len(), runs.len() + STEP, Some(l), true);
            runs.extend(more);
            let c = check(&runs, n_cells);
            conv.push(json!({ "runs": runs.len(), "p10": c.0, "p50": c.1, "p90": c.2, "share_ge80": c.3 }));
            let stable = (c.0 - prev.0).abs() < 1.0 && (c.1 - prev.1).abs() < 1.0 && (c.2 - prev.2).abs() < 1.0 && (c.3 - prev.3).abs() < 0.01;
            prev = c;
            if stable {
                converged = true;
                break;
            }
        }
        eprintln!("l = {l} m: {} runs, converged {converged} ({:.0} s so far)", runs.len(), t0.elapsed().as_secs_f64());
        ens.push(summary(&inp, &runs, Some(l), Some(converged), conv));
        all_runs.push(runs);
    }
    // 7. the app shows the ensemble with the wider p10-p90 time range
    let width = |v: &Value| v["time_s"]["p90"].as_f64().unwrap_or(0.0) - v["time_s"]["p10"].as_f64().unwrap_or(0.0);
    let shown = if width(&ens[1]) > width(&ens[0]) { 1 } else { 0 };
    // 6. field only (scalars at published values), at the shown ensemble's correlation length
    let fo = batch(&inp, 9, 0, FIELD_ONLY_RUNS, Some(LS_M[shown]), false);
    let fo_s = summary(&inp, &fo, Some(LS_M[shown]), None, vec![]);
    let u = usage(&all_runs[shown], n_cells);
    let n = all_runs[shown].len() as f64;
    let bytes: Vec<u8> = u.iter().map(|&c| ((c as f64 / n) * 255.0).round() as u8).collect();
    std::fs::write(root().join("assets/route_usage.bin"), &bytes).unwrap();
    out["ensembles"] = json!(ens);
    out["shown_correlation_length_m"] = json!(LS_M[shown]);
    out["field_only"] = fo_s;
    out["usage_file"] = json!({ "name": "route_usage.bin", "encoding": "uint8 per cell, row 0 north: round(255 x share of runs whose route crosses the cell)",
                                "runs": all_runs[shown].len(), "correlation_length_m": LS_M[shown] });
    out["runtime_s"] = json!((t0.elapsed().as_secs_f64() * 10.0).round() / 10.0);
    std::fs::write(root().join("assets/route_ensemble.json"), serde_json::to_string_pretty(&out).unwrap()).unwrap();
    println!("{}", serde_json::to_string_pretty(&out).unwrap());
}
