//! prep: read a window of a (Geo)TIFF DEM without loading the whole file, and run the looks-flat measurement.
//!   prep info    DEM.tif
//!   prep measure DEM.tif ROW0 ROW1 COL0 COL1 [--pad 100] [--steps 256] [--max-dist 200] [--out results.csv]
//!   prep locate  DEM.tif LAT LON [--radius 3396190] [--lat-ts 0] [--lon0 0]   (equirectangular; CHECK the projection printed by `info`)
//!   prep slope   DEM.tif ROW0 ROW1 COL0 COL1   (gate step 2: share of ground >= 15 and >= 25 deg, native resolution, no smoothing)
//! STATUS: tested on a small synthetic tiled GeoTIFF only. NOT tested on the 3.6 GB USGS mosaic: sample type, compression,
//! nodata and pixel-scale units must be checked from the `info` output first. Pixel scale in degrees triggers a warning.
use std::{env, fs::File, io::{BufReader, Write}};
use terrain_core::{looks_flat_multi, Grid, MultiShare};
use tiff::{decoder::{Decoder, DecodingResult}, tags::Tag};

type Dec = Decoder<BufReader<File>>;

fn open(path: &str) -> Dec {
    Decoder::new(BufReader::new(File::open(path).expect("cannot open file"))).expect("not a readable TIFF")
}

fn pixel_scale(d: &mut Dec) -> Option<f64> {
    d.get_tag_f64_vec(Tag::ModelPixelScaleTag).ok().and_then(|v| v.first().copied())
}
fn nodata(d: &mut Dec) -> Option<f32> {
    d.get_tag_ascii_string(Tag::GdalNodata).ok().and_then(|s| s.trim().parse::<f32>().ok())
}

fn tiepoint(d: &mut Dec) -> Option<Vec<f64>> { d.get_tag_f64_vec(Tag::ModelTiepointTag).ok() }
fn geo_ascii(d: &mut Dec) -> Option<String> { d.get_tag_ascii_string(Tag::GeoAsciiParamsTag).ok() }
fn geo_doubles(d: &mut Dec) -> Option<Vec<f64>> { d.get_tag_f64_vec(Tag::GeoDoubleParamsTag).ok() }

/// Equirectangular forward projection, then pixel = tiepoint + offset / scale. Returns fractional (row, col).
/// ASSUMES the DEM is equirectangular with the given sphere radius, standard parallel and central meridian;
/// the defaults (3396190 m, 0, 0) reproduce PLACES easting/northing, NOT verified against the DEM's own GeoKeys.
fn latlon_to_pixel(lat: f64, lon: f64, radius: f64, lat_ts: f64, lon0: f64, tie: &[f64], scale: &[f64]) -> (f64, f64) {
    let x = radius * lat_ts.to_radians().cos() * (lon - lon0).to_radians();
    let y = radius * lat.to_radians();
    let col = tie[0] + (x - tie[3]) / scale[0];
    let row = tie[1] + (tie[4] - y) / scale[1];
    (row, col)
}

pub struct SlopeStats { pub n: usize, pub n_excluded: usize, pub ge15: usize, pub ge25: usize, pub p50: f32, pub p90: f32, pub p99: f32, pub max: f32 }

/// Slope stats over rows [r0,r1) x cols [c0,c1) of the full image, reading a 1-pixel border so inner gradients are central differences.
/// Pixels whose own or 4-neighbour value is nodata/NaN are EXCLUDED (not filled).
/// Gaussian smoothing with standard deviation `sigma_px` pixels, separable, normalized over valid (non-`bad`) pixels only,
/// so nodata does not leak into the result. sigma 0 returns the input unchanged.
fn gaussian_smooth(z: &[f32], bad: &[bool], w: usize, h: usize, sigma_px: f32) -> Vec<f32> {
    if sigma_px <= 0.0 { return z.to_vec(); }
    let rad = (3.0 * sigma_px).ceil() as i64;
    let k: Vec<f32> = (-rad..=rad).map(|i| (-(i * i) as f32 / (2.0 * sigma_px * sigma_px)).exp()).collect();
    let pass = |src: &[f32], wt: &[f32], horiz: bool| -> (Vec<f32>, Vec<f32>) {
        let (mut out, mut ow) = (vec![0f32; w * h], vec![0f32; w * h]);
        for r in 0..h {
            for c in 0..w {
                let (mut s, mut sw) = (0f32, 0f32);
                for (j, kv) in k.iter().enumerate() {
                    let o = j as i64 - rad;
                    let (cc, rr) = if horiz { (c as i64 + o, r as i64) } else { (c as i64, r as i64 + o) };
                    if cc < 0 || rr < 0 || cc >= w as i64 || rr >= h as i64 { continue; }
                    let i = rr as usize * w + cc as usize;
                    s += kv * src[i] * wt[i];
                    sw += kv * wt[i];
                }
                out[r * w + c] = if sw > 0.0 { s / sw } else { f32::NAN };
                ow[r * w + c] = sw;
            }
        }
        (out, ow)
    };
    let wt: Vec<f32> = bad.iter().map(|b| if *b { 0.0 } else { 1.0 }).collect();
    let zc: Vec<f32> = z.iter().zip(bad).map(|(v, b)| if *b { 0.0 } else { *v }).collect();
    let (h1, _) = pass(&zc, &wt, true);
    let wt1: Vec<f32> = h1.iter().map(|v| if v.is_nan() { 0.0 } else { 1.0 }).collect();
    let h1c: Vec<f32> = h1.iter().map(|v| if v.is_nan() { 0.0 } else { *v }).collect();
    pass(&h1c, &wt1, false).0
}

/// Slope stats with optional Gaussian smoothing of `sigma_m` metres before the gradient (0 = native resolution).
#[cfg(test)]
fn slope_stats(d: &mut Dec, r0: usize, r1: usize, c0: usize, c1: usize) -> SlopeStats { slope_stats_smoothed(d, r0, r1, c0, c1, 0.0) }

fn slope_stats_smoothed(d: &mut Dec, r0: usize, r1: usize, c0: usize, c1: usize, sigma_m: f32) -> SlopeStats {
    let (w, h) = d.dimensions().unwrap();
    let cell = pixel_scale(d).unwrap_or(1.0) as f32;
    let sigma_px = sigma_m / cell;
    let pad = 1 + (3.0 * sigma_px).ceil() as usize;
    let (pr0, pr1, pc0, pc1) = (r0.saturating_sub(pad), (r1 + pad).min(h as usize), c0.saturating_sub(pad), (c1 + pad).min(w as usize));
    let z = read_window(d, pr0, pr1, pc0, pc1);
    let nd = nodata(d);
    let (gw, gh) = (pc1 - pc0, pr1 - pr0);
    let bad: Vec<bool> = z.iter().map(|v| v.is_nan() || Some(*v) == nd).collect();
    let z = gaussian_smooth(&z, &bad, gw, gh, sigma_px);
    let g = Grid::new(gw, gh, cell, z);
    let mut s: Vec<f32> = Vec::with_capacity((r1 - r0) * (c1 - c0));
    let mut n_excluded = 0;
    for r in (r0 - pr0)..(r1 - pr0) {
        for c in (c0 - pc0)..(c1 - pc0) {
            let nb = |dc: i64, dr: i64| {
                let (cc, rr) = (c as i64 + dc, r as i64 + dr);
                cc >= 0 && rr >= 0 && (cc as usize) < gw && (rr as usize) < gh && bad[rr as usize * gw + cc as usize]
            };
            if nb(0, 0) || nb(-1, 0) || nb(1, 0) || nb(0, -1) || nb(0, 1) { n_excluded += 1; continue; }
            s.push(g.slope_deg(c, r));
        }
    }
    let n = s.len();
    let ge15 = s.iter().filter(|v| **v >= 15.0).count();
    let ge25 = s.iter().filter(|v| **v >= 25.0).count();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let q = |p: f32| if n == 0 { f32::NAN } else { s[(p * (n - 1) as f32).round() as usize] };
    SlopeStats { n, n_excluded, ge15, ge25, p50: q(0.5), p90: q(0.9), p99: q(0.99), max: if n == 0 { f32::NAN } else { s[n - 1] } }
}

fn to_f32(r: DecodingResult) -> Vec<f32> {
    match r {
        DecodingResult::F32(v) => v,
        DecodingResult::F64(v) => v.into_iter().map(|x| x as f32).collect(),
        DecodingResult::I16(v) => v.into_iter().map(|x| x as f32).collect(),
        DecodingResult::U16(v) => v.into_iter().map(|x| x as f32).collect(),
        DecodingResult::I32(v) => v.into_iter().map(|x| x as f32).collect(),
        DecodingResult::U32(v) => v.into_iter().map(|x| x as f32).collect(),
        _ => panic!("unsupported sample type: check `prep info`"),
    }
}

/// Read rows [r0, r1) x cols [c0, c1) by decoding only the chunks (strips or tiles) that overlap.
fn read_window(d: &mut Dec, r0: usize, r1: usize, c0: usize, c1: usize) -> Vec<f32> {
    let (w, h) = d.dimensions().unwrap();
    let (w, h) = (w as usize, h as usize);
    assert!(r1 <= h && c1 <= w && r0 < r1 && c0 < c1, "window outside the image ({w} x {h})");
    let (cw, ch) = d.chunk_dimensions();
    let (cw, ch) = (cw as usize, ch as usize);
    let across = (w + cw - 1) / cw;
    let mut out = vec![f32::NAN; (r1 - r0) * (c1 - c0)];
    for cr in (r0 / ch)..=((r1 - 1) / ch) {
        for cc in (c0 / cw)..=((c1 - 1) / cw) {
            let idx = (cr * across + cc) as u32;
            let (dw, dh) = d.chunk_data_dimensions(idx);
            let (dw, dh) = (dw as usize, dh as usize);
            let data = match d.read_chunk(idx) {
                Ok(v) => to_f32(v),
                Err(e) => { eprintln!("ERROR reading chunk {idx}: {e}\nIf this is an unsupported compression (zstd, JPEG-XL, LERC), convert once with GDAL: gdal_translate -co COMPRESS=DEFLATE -co TILED=YES -co BIGTIFF=YES in.tif out.tif"); std::process::exit(2); }
            };
            // chunks are stored with the full chunk width in tiled files; strips use the image width
            let stride = if data.len() >= cw * dh && cw >= dw { data.len() / dh } else { dw };
            for ry in 0..dh {
                let gy = cr * ch + ry;
                if gy < r0 || gy >= r1 { continue; }
                for rx in 0..dw {
                    let gx = cc * cw + rx;
                    if gx < c0 || gx >= c1 { continue; }
                    out[(gy - r0) * (c1 - c0) + (gx - c0)] = data[ry * stride + rx];
                }
            }
        }
    }
    out
}

// Sun azimuths (deg from north, clockwise) a Gale sun can have at each elevation over all seasons, morning range sampled at 5 points
// and mirrored for the afternoon. GENERATED by tools/misleading_slope.py realistic_az() (obliquity 25.19 deg from the NASA Mars fact sheet;
// azimuth formula is standard spherical astronomy, not from the Mars24 notes). Regenerate if the latitude or obliquity changes.
const REALISTIC_AZ: [(f32, [f32; 10]); 4] = [
    (15.0, [62.3, 75.4, 88.6, 101.7, 114.8, 297.7, 284.6, 271.4, 258.3, 245.2]),
    (30.0, [57.2, 72.0, 86.8, 101.6, 116.4, 302.8, 288.0, 273.2, 258.4, 243.6]),
    (50.0, [40.1, 61.2, 82.2, 103.3, 124.4, 319.9, 298.8, 277.8, 256.7, 235.6]),
    (60.0, [1.4, 34.9, 68.3, 101.7, 135.1, 358.6, 325.1, 291.7, 258.3, 224.9]),
];
fn azimuths(el: f32, all: bool) -> Vec<f32> {
    if all { return (0..8).map(|k| k as f32 * 45.0).collect(); }
    REALISTIC_AZ.iter().find(|(e, _)| *e == el).map(|(_, v)| v.to_vec()).expect("elevation not in table")
}

fn arg<T: std::str::FromStr>(a: &[String], name: &str, default: T) -> T {
    a.iter().position(|s| s == name).and_then(|i| a.get(i + 1)).and_then(|s| s.parse().ok()).unwrap_or(default)
}

fn main() {
    let a: Vec<String> = env::args().collect();
    match a.get(1).map(|s| s.as_str()) {
        Some("info") => {
            let mut d = open(&a[2]);
            let (w, h) = d.dimensions().unwrap();
            println!("size {w} x {h}");
            println!("color type {:?}", d.colortype().unwrap());
            println!("chunk type {:?}, chunk dimensions {:?}", d.get_chunk_type(), d.chunk_dimensions());
            println!("pixel scale {:?}  nodata {:?}", pixel_scale(&mut d), nodata(&mut d));
            println!("model tiepoint {:?}", tiepoint(&mut d));
            println!("geo ascii params {:?}", geo_ascii(&mut d));
            println!("geo double params {:?}", geo_doubles(&mut d));
        }
        Some("locate") => {
            let mut d = open(&a[2]);
            let lat: f64 = a[3].parse().expect("LAT");
            let lon: f64 = a[4].parse().expect("LON");
            let radius: f64 = arg(&a, "--radius", 3396190.0);
            let lat_ts: f64 = arg(&a, "--lat-ts", 0.0);
            let lon0: f64 = arg(&a, "--lon0", 0.0);
            let tie = tiepoint(&mut d).expect("no ModelTiepoint tag");
            let scale = d.get_tag_f64_vec(Tag::ModelPixelScaleTag).expect("no ModelPixelScale tag");
            let (row, col) = latlon_to_pixel(lat, lon, radius, lat_ts, lon0, &tie, &scale);
            println!("ASSUMED projection: equirectangular, radius {radius} m, standard parallel {lat_ts}, central meridian {lon0}: check against `prep info`");
            println!("row {row:.2} col {col:.2}");
            let (w, h) = d.dimensions().unwrap();
            if row >= 0.0 && col >= 0.0 && (row as u32) < h && (col as u32) < w {
                let (r, c) = (row as usize, col as usize);
                println!("DEM value at that pixel {:?} (compare with the PLACES elevation of the same stop)", read_window(&mut d, r, r + 1, c, c + 1)[0]);
            } else {
                println!("outside the image ({w} x {h})");
            }
        }
        Some("dump") => {
            // dump DEM.tif LAT_N LON_W LAT_S LON_E out.f32: raw window for inspection (u32 width, u32 height, f32 cell, then f32 row-major; nodata -> NaN)
            let mut d = open(&a[2]);
            let f = |i: usize| a[i].parse::<f64>().expect("LAT_N LON_W LAT_S LON_E must be numbers");
            let tie = tiepoint(&mut d).expect("no ModelTiepoint tag");
            let scale = d.get_tag_f64_vec(Tag::ModelPixelScaleTag).expect("no ModelPixelScale tag");
            let (ra, ca) = latlon_to_pixel(f(3), f(4), 3396190.0, 0.0, 0.0, &tie, &scale);
            let (rb, cb) = latlon_to_pixel(f(5), f(6), 3396190.0, 0.0, 0.0, &tie, &scale);
            let (w, h) = d.dimensions().unwrap();
            let cl = |v: f64, hi: u32| v.max(0.0).min(hi as f64) as usize;
            let (r0, r1, c0, c1) = (cl(ra.min(rb).floor(), h), cl(ra.max(rb).ceil(), h), cl(ca.min(cb).floor(), w), cl(ca.max(cb).ceil(), w));
            let nd = nodata(&mut d);
            let cell = pixel_scale(&mut d).unwrap_or(1.0) as f32;
            let z: Vec<f32> = read_window(&mut d, r0, r1, c0, c1).into_iter().map(|v| if Some(v) == nd { f32::NAN } else { v }).collect();
            let mut out = File::create(&a[7]).expect("cannot create output");
            out.write_all(&((c1 - c0) as u32).to_le_bytes()).unwrap();
            out.write_all(&((r1 - r0) as u32).to_le_bytes()).unwrap();
            out.write_all(&cell.to_le_bytes()).unwrap();
            for v in &z { out.write_all(&v.to_le_bytes()).unwrap(); }
            println!("dumped rows {r0}..{r1} cols {c0}..{c1} ({} x {}) to {}", c1 - c0, r1 - r0, a[7]);
        }
        Some("slope") | Some("slopebox") => {
            let mut d = open(&a[2]);
            let (r0, r1, c0, c1) = if a[1] == "slope" {
                let p = |i: usize| a[i].parse::<usize>().expect("ROW0 ROW1 COL0 COL1 must be integers");
                (p(3), p(4), p(5), p(6))
            } else {
                // slopebox DEM.tif LAT_N LON_W LAT_S LON_E: box corners -> pixels, rounded outward
                let f = |i: usize| a[i].parse::<f64>().expect("LAT_N LON_W LAT_S LON_E must be numbers");
                let radius: f64 = arg(&a, "--radius", 3396190.0);
                let lat_ts: f64 = arg(&a, "--lat-ts", 0.0);
                let lon0: f64 = arg(&a, "--lon0", 0.0);
                let tie = tiepoint(&mut d).expect("no ModelTiepoint tag");
                let scale = d.get_tag_f64_vec(Tag::ModelPixelScaleTag).expect("no ModelPixelScale tag");
                let (ra, ca) = latlon_to_pixel(f(3), f(4), radius, lat_ts, lon0, &tie, &scale);
                let (rb, cb) = latlon_to_pixel(f(5), f(6), radius, lat_ts, lon0, &tie, &scale);
                let (w, h) = d.dimensions().unwrap();
                let cl = |v: f64, hi: u32| v.max(0.0).min(hi as f64) as usize;
                (cl(ra.min(rb).floor(), h), cl(ra.max(rb).ceil(), h), cl(ca.min(cb).floor(), w), cl(ca.max(cb).ceil(), w))
            };
            let cell = pixel_scale(&mut d).unwrap_or_else(|| { eprintln!("no pixel scale tag: assuming 1 m"); 1.0 });
            if cell < 0.01 { eprintln!("WARNING: pixel scale {cell} looks like degrees, not metres: reproject first"); }
            let label: String = arg(&a, "--label", "window".to_string());
            let smooth: String = arg(&a, "--smooth", "0,1,2,3".to_string());
            println!("{label}: rows {r0}..{r1} cols {c0}..{c1}, cell {cell} m; Gaussian smoothing sigma (m) {smooth}; nodata-adjacent pixels excluded");
            for s in smooth.split(',') {
                let sigma: f32 = s.trim().parse().expect("--smooth takes comma-separated metres, e.g. 0,1,2,3");
                let st = slope_stats_smoothed(&mut d, r0, r1, c0, c1, sigma);
                let pct = |k: usize| 100.0 * k as f64 / st.n.max(1) as f64;
                println!("  sigma {sigma} m: used {} excl {}  >=15 deg {:.3}%  >=25 deg {:.3}%  p50 {:.2} p90 {:.2} p99 {:.2} max {:.2}",
                    st.n, st.n_excluded, pct(st.ge15), pct(st.ge25), st.p50, st.p90, st.p99, st.max);
                // machine-readable line for tools/gate_patch.py
                println!("RESULT,{label},{sigma},{},{},{},{},{:.4},{:.4},{:.4},{:.4}", st.n, st.n_excluded, st.ge15, st.ge25, st.p50, st.p90, st.p99, st.max);
            }
        }
        Some("measure") => {
            let mut d = open(&a[2]);
            let p = |i: usize| a[i].parse::<usize>().expect("ROW0 ROW1 COL0 COL1 must be integers");
            let (r0, r1, c0, c1) = (p(3), p(4), p(5), p(6));
            let pad: usize = arg(&a, "--pad", 100);
            let steps: u32 = arg(&a, "--steps", 256);
            let max_dist: f32 = arg(&a, "--max-dist", 200.0);
            let all_az = a.iter().any(|x| x == "--all-azimuths");
            let out_path: String = arg(&a, "--out", "prep_results.csv".to_string());
            let cell = pixel_scale(&mut d).unwrap_or_else(|| { eprintln!("no pixel scale tag: assuming 1 m"); 1.0 }) as f32;
            if cell < 0.01 { eprintln!("WARNING: pixel scale {cell} looks like degrees, not metres: reproject first"); }
            let (w, h) = d.dimensions().unwrap();
            let (pr0, pr1) = (r0.saturating_sub(pad), (r1 + pad).min(h as usize));
            let (pc0, pc1) = (c0.saturating_sub(pad), (c1 + pad).min(w as usize));
            let mut z = read_window(&mut d, pr0, pr1, pc0, pc1);
            let nd = nodata(&mut d);
            let bad = z.iter().filter(|v| v.is_nan() || Some(**v) == nd).count();
            if bad > 0 {
                eprintln!("WARNING: {bad} nodata/NaN pixels ({:.2}%) replaced by the window mean: check seams and gaps", 100.0 * bad as f64 / z.len() as f64);
                let good: Vec<f32> = z.iter().copied().filter(|v| !v.is_nan() && Some(*v) != nd).collect();
                let mean = good.iter().sum::<f32>() / good.len().max(1) as f32;
                for v in z.iter_mut() { if v.is_nan() || Some(*v) == nd { *v = mean; } }
            }
            let g = Grid::new(pc1 - pc0, pr1 - pr0, cell, z);
            let mask = (r0 - pr0, r1 - pr0, c0 - pc0, c1 - pc0);
            println!("window {}x{} px, cell {cell} m, padded read {}x{}", r1 - r0, c1 - c0, g.h, g.w);
            let mut f = File::create(&out_path).unwrap();
            writeln!(f, "band_deg,sun_el,tol,shadows,n_steep,n_shadowed,share_min,share_mean,share_max").unwrap();
            let bands = [15.0f32, 25.0]; let tols = [0.05f32, 0.10, 0.20]; let els = [15.0f32, 30.0, 50.0, 60.0];
            for &el in &els {
                let azs = azimuths(el, all_az);
                // per azimuth: one pass for all (band, tol) pairs
                let per_az: Vec<Vec<MultiShare>> = azs.iter().map(|&az| looks_flat_multi(&g, mask, &bands, &tols, el, az, steps, max_dist)).collect();
                for (bi, &band) in bands.iter().enumerate() { for (ti, &tol) in tols.iter().enumerate() {
                    for &with_shadows in &[false, true] {
                        let idx = bi * tols.len() + ti;
                        let v: Vec<f32> = per_az.iter().filter(|m| m[idx].n_steep > 0).map(|m| {
                            (if with_shadows { m[idx].looks_flat_lit } else { m[idx].looks_flat_no_shadow }) as f32 / m[idx].n_steep as f32 }).collect();
                        if v.is_empty() { continue; }
                        let (mn, mx) = (v.iter().cloned().fold(f32::MAX, f32::min), v.iter().cloned().fold(f32::MIN, f32::max));
                        let mean = v.iter().sum::<f32>() / v.len() as f32;
                        let n_steep = per_az[0][idx].n_steep;
                        let ns = per_az.iter().map(|m| m[idx].n_shadowed).sum::<usize>() / per_az.len();
                        let tag = if with_shadows { "on" } else { "off" };
                        writeln!(f, "{band},{el},{tol},{tag},{n_steep},{ns},{mn:.4},{mean:.4},{mx:.4}").unwrap();
                        println!("tol {tol:.2} band>={band:2} el {el:2} shadows {:3}  steep {n_steep:8}  looks-flat share min {mn:.2} mean {mean:.2} max {mx:.2}", tag);
                    }
                }}
            }
            println!("wrote {out_path}");
        }
        _ => eprintln!("usage: prep info DEM.tif | prep locate DEM.tif LAT LON [--radius R] [--lat-ts D] [--lon0 D] | prep slope DEM.tif ROW0 ROW1 COL0 COL1 | prep measure DEM.tif ROW0 ROW1 COL0 COL1 [--pad N] [--steps N] [--max-dist M] [--out f.csv]"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiff::encoder::{colortype::Gray32Float, TiffEncoder};

    /// Synthetic GeoTIFF: a plane rising eastward at `slope_deg` on the left half, flat on the right half,
    /// one nodata pixel at (row 5, col 5), 1 m cells, pixel (0,0) at x=1000, y=5000.
    fn write_fixture(path: &std::path::Path, w: u32, h: u32, slope_deg: f32) {
        let t = slope_deg.to_radians().tan();
        let mut z = vec![0f32; (w * h) as usize];
        for r in 0..h {
            for c in 0..w {
                z[(r * w + c) as usize] = if c < w / 2 { c as f32 * t } else { (w / 2) as f32 * t };
            }
        }
        z[(5 * w + 5) as usize] = -9999.0;
        let f = File::create(path).unwrap();
        let mut enc = TiffEncoder::new(f).unwrap();
        let mut img = enc.new_image::<Gray32Float>(w, h).unwrap();
        img.encoder().write_tag(Tag::ModelPixelScaleTag, &[1.0f64, 1.0, 0.0][..]).unwrap();
        img.encoder().write_tag(Tag::ModelTiepointTag, &[0.0f64, 0.0, 0.0, 1000.0, 5000.0, 0.0][..]).unwrap();
        img.encoder().write_tag(Tag::GdalNodata, "-9999").unwrap();
        img.write_data(&z).unwrap();
    }

    #[test]
    fn slope_share_on_half_plane() {
        let p = std::env::temp_dir().join("prep_slope_fixture.tif");
        write_fixture(&p, 40, 20, 20.0);
        let mut d = open(p.to_str().unwrap());
        // window on the sloped half only: every valid pixel is 20 deg
        let st = slope_stats(&mut d, 2, 18, 2, 18);
        assert_eq!(st.n_excluded, 5, "nodata pixel and its 4 neighbours are excluded");
        assert_eq!(st.n, 16 * 16 - 5);
        assert_eq!(st.ge15, st.n);
        assert_eq!(st.ge25, 0);
        assert!((st.p50 - 20.0).abs() < 0.01, "p50 {}", st.p50);
        // flat half: nothing steep
        let st = slope_stats(&mut d, 2, 18, 24, 38);
        assert_eq!(st.ge15, 0);
        assert!(st.max < 0.01);
        // the tiepoint is read back
        assert_eq!(tiepoint(&mut d).unwrap()[3], 1000.0);
    }

    /// Flat ground plus white height noise: smoothing must remove most noise-made steep pixels,
    /// while a real 20 deg plane keeps its slope after smoothing.
    #[test]
    fn smoothing_separates_noise_from_real_slope() {
        let (w, h) = (80usize, 80usize);
        let mut seed: u64 = 12345;
        let mut noise = || { // Box-Muller from an LCG, deterministic
            let mut u = || { seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407); ((seed >> 11) as f64 / (1u64 << 53) as f64).max(1e-12) };
            let (a, b) = (u(), u());
            ((-2.0 * a.ln()).sqrt() * (2.0 * std::f64::consts::PI * b).cos()) as f32
        };
        let t = 20f32.to_radians().tan();
        let mut z = vec![0f32; w * h];
        for r in 0..h { for c in 0..w { z[r * w + c] = 0.3 * noise() + if c < w / 2 { c as f32 * t } else { (w / 2) as f32 * t }; } }
        let p = std::env::temp_dir().join("prep_noise_fixture.tif");
        let f = File::create(&p).unwrap();
        let mut enc = TiffEncoder::new(f).unwrap();
        let mut img = enc.new_image::<Gray32Float>(w as u32, h as u32).unwrap();
        img.encoder().write_tag(Tag::ModelPixelScaleTag, &[1.0f64, 1.0, 0.0][..]).unwrap();
        img.write_data(&z).unwrap();
        let mut d = open(p.to_str().unwrap());
        let flat0 = slope_stats_smoothed(&mut d, 10, 70, 50, 75, 0.0);
        let flat2 = slope_stats_smoothed(&mut d, 10, 70, 50, 75, 2.0);
        let plane2 = slope_stats_smoothed(&mut d, 10, 70, 10, 30, 2.0);
        let share = |s: &SlopeStats| s.ge15 as f32 / s.n as f32;
        // white noise sigma 0.3 m at 1 m spacing: expected ~45% of flat pixels >= 15 deg (Rayleigh, sigma_g = 0.3/sqrt(2))
        assert!(share(&flat0) > 0.3, "noise-only steep share at sigma 0: {}", share(&flat0));
        assert!(share(&flat2) < 0.01, "noise-only steep share at sigma 2 m: {}", share(&flat2));
        assert!((plane2.p50 - 20.0).abs() < 1.0, "20 deg plane after 2 m smoothing: p50 {}", plane2.p50);
    }

    #[test]
    fn locate_inverts_projection() {
        let tie = [0.0, 0.0, 0.0, 1000.0, 5000.0, 0.0];
        let scale = [1.0, 1.0, 0.0];
        let r = 3396190.0f64;
        // a point 12 m east and 7 m south of pixel (0,0)
        let lon = (1000.0 + 12.0) / r;
        let lat = (5000.0 - 7.0) / r;
        let (row, col) = latlon_to_pixel(lat.to_degrees(), lon.to_degrees(), r, 0.0, 0.0, &tie, &scale);
        assert!((row - 7.0).abs() < 1e-6 && (col - 12.0).abs() < 1e-6, "{row} {col}");
    }

    #[test]
    fn places_easting_matches_default_projection() {
        // PLACES row for sol 4534 (localized_pos.csv): lat -4.797617633, lon 137.386436072, northing -284377.334, easting 8143539.442
        let r = 3396190.0f64;
        let x = r * 137.386436072f64.to_radians();
        let y = r * (-4.797617633f64).to_radians();
        assert!((x - 8143539.442).abs() < 1.0 && (y + 284377.334).abs() < 1.0, "{x} {y}");
    }
}
