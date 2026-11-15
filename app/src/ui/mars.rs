//! Mars view model: the whole planet down to the boxwork patch.
//! overview.json (tools/overview.py) gives the NASA landing sites (Trek bookmarks), Curiosity's whole PLACES traverse, the
//! study patch and the basemap tile services, which are streamed (equirectangular, 2^(L+1) x 2^L tiles at level L).
//! missions.json lists which NASA mission's data VISOR uses where. Pure functions here; the view is views/marsview.rs.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
use serde_json::Value;

pub const MARS_R_M: f64 = 3_396_190.0; // Mars 2000 sphere (the tile services' and the DEM's own)

#[derive(Clone, Debug, PartialEq)]
pub struct Base {
    pub id: String,
    pub title: String,
    pub detail: String,
    pub url: String,
    pub tile_px: f64,
    pub max_level: u32,
    pub credit: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Site {
    pub title: String,
    pub mission: String,
    pub lat: f64,
    pub lon: f64,
    pub bbox: [f64; 4],
    pub position: String,
    pub description: String,
    pub image: Option<String>,
    pub source: String,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Overview {
    pub bases: Vec<Base>,
    pub sites: Vec<Site>,
    /// (lat, lon, sol)
    pub traverse: Vec<(f64, f64, i64)>,
    pub sol_first: i64,
    pub sol_last: i64,
    pub traverse_credit: String,
    /// west, south, east, north (deg)
    pub study: [f64; 4],
    pub study_title: String,
    pub credit_sites: String,
}

fn bbox(v: &Value) -> Option<[f64; 4]> {
    let a = v.as_array()?;
    Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?, a.get(2)?.as_f64()?, a.get(3)?.as_f64()?])
}

pub fn parse(v: &Value) -> Option<Overview> {
    let s = |x: &Value| x.as_str().unwrap_or_default().to_string();
    let bases = v["bases"].as_array()?.iter().filter_map(|b| Some(Base {
        id: s(&b["id"]), title: s(&b["title"]), detail: s(&b["detail"]), url: b["url"].as_str()?.to_string(),
        tile_px: b["tile_px"].as_f64()?, max_level: b["max_level"].as_u64()? as u32, credit: s(&b["credit"]),
    })).collect::<Vec<_>>();
    let sites = v["sites"].as_array()?.iter().filter_map(|x| Some(Site {
        title: s(&x["title"]), mission: s(&x["mission"]), lat: x["lat"].as_f64()?, lon: x["lon"].as_f64()?, bbox: bbox(&x["bbox"])?,
        position: s(&x["position"]), description: s(&x["description"]), image: x["image"].as_str().map(str::to_string), source: s(&x["source"]),
    })).collect();
    let t = &v["traverse"];
    let traverse = t["points"].as_array()?.iter()
        .filter_map(|p| Some((p.get(0)?.as_f64()?, p.get(1)?.as_f64()?, p.get(2)?.as_i64()?))).collect();
    if bases.is_empty() {
        return None;
    }
    Some(Overview { bases, sites, traverse, sol_first: t["sol_first"].as_i64().unwrap_or(0), sol_last: t["sol_last"].as_i64().unwrap_or(0),
                    traverse_credit: s(&t["credit"]), study: bbox(&v["study"]["bbox"])?, study_title: s(&v["study"]["title"]),
                    credit_sites: s(&v["credit_sites"]) })
}

#[derive(Clone, Debug, PartialEq)]
pub struct Use {
    pub instrument: String,
    pub what: String,
    pub files: Vec<String>,
    pub live: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mission {
    pub name: String,
    pub short: String,
    pub uses: Vec<Use>,
}

pub fn missions(v: &Value) -> Vec<Mission> {
    let s = |x: &Value| x.as_str().unwrap_or_default().to_string();
    v["missions"].as_array().map(|a| a.iter().map(|m| Mission {
        name: s(&m["mission"]), short: s(&m["short"]),
        uses: m["uses"].as_array().map(|u| u.iter().map(|x| Use {
            instrument: s(&x["instrument"]), what: s(&x["for"]),
            files: x["files"].as_array().map(|f| f.iter().filter_map(|y| y.as_str().map(str::to_string)).collect()).unwrap_or_default(),
            live: x["live"].as_str().map(str::to_string),
        }).collect()).unwrap_or_default(),
    }).collect()).unwrap_or_default()
}

/// The map window: centre (deg), degrees per screen pixel, size in CSS px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub lon: f64,
    pub lat: f64,
    pub dpp: f64,
    pub w: f64,
    pub h: f64,
}

pub fn deg_per_px(level: u32, tile_px: f64) -> f64 {
    360.0 / (tile_px * 2f64.powi(level as i32 + 1))
}

/// Coarsest level whose pixels are no larger than the screen's (so tiles are shown at scale <= 1), capped at max_level.
pub fn level_for(dpp: f64, b: &Base) -> u32 {
    (0..=b.max_level).find(|&l| deg_per_px(l, b.tile_px) <= dpp * 1.0001).unwrap_or(b.max_level)
}

/// Finest and coarsest zoom (deg per px): the whole planet's width fills the window; down to 2 screen px per CTX native pixel.
pub fn dpp_range(w: f64) -> (f64, f64) {
    (deg_per_px(13, 512.0) / 2.0, 360.0 / w.max(100.0))
}

pub fn clamp(v: View) -> View {
    let (lo, hi) = dpp_range(v.w);
    let dpp = v.dpp.clamp(lo, hi);
    let half_h = v.h / 2.0 * dpp;
    let lat = if half_h >= 90.0 { 0.0 } else { v.lat.clamp(-90.0 + half_h, 90.0 - half_h) };
    View { dpp, lat, lon: (v.lon + 180.0).rem_euclid(360.0) - 180.0, ..v }
}

/// Basemap index for "Auto": Viking colour while it still has detail at this zoom, then CTX.
pub fn auto_base(o: &Overview, dpp: f64) -> usize {
    let viking = o.bases.iter().position(|b| b.id == "viking");
    let ctx = o.bases.iter().position(|b| b.id == "ctx");
    match (viking, ctx) {
        (Some(v), Some(c)) => if dpp >= deg_per_px(o.bases[v].max_level, o.bases[v].tile_px) * 0.999 { v } else { c },
        (Some(v), None) => v,
        (None, Some(c)) => c,
        _ => 0,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tile {
    pub url: String,
    pub left: f64,
    pub top: f64,
    pub size: f64,
}

/// The tiles covering the window (at most 400).
pub fn tiles(b: &Base, v: &View) -> Vec<Tile> {
    let l = level_for(v.dpp, b);
    let td = deg_per_px(l, b.tile_px) * b.tile_px; // degrees per tile
    let (ncol, nrow) = (2i64.pow(l + 1), 2i64.pow(l));
    let lon_left = v.lon - v.w / 2.0 * v.dpp;
    let lat_top = v.lat + v.h / 2.0 * v.dpp;
    let c0 = ((lon_left + 180.0) / td).floor() as i64;
    let c1 = ((lon_left + v.w * v.dpp + 180.0) / td).floor() as i64;
    let r0 = ((90.0 - lat_top) / td).floor().max(0.0) as i64;
    let r1 = (((90.0 - (lat_top - v.h * v.dpp)) / td).floor() as i64).min(nrow - 1);
    let mut out = Vec::new();
    if (c1 - c0 + 1) * (r1 - r0 + 1) > 400 {
        return out;
    }
    for r in r0..=r1 {
        for c in c0..=c1 {
            let cw = c.rem_euclid(ncol);
            out.push(Tile {
                url: b.url.replace("{z}", &l.to_string()).replace("{row}", &r.to_string()).replace("{col}", &cw.to_string()),
                left: (c as f64 * td - 180.0 - lon_left) / v.dpp,
                top: (lat_top - (90.0 - r as f64 * td)) / v.dpp,
                size: td / v.dpp,
            });
        }
    }
    out
}

/// Screen position (CSS px from the window's top-left) of a point; longitude taken nearest the centre.
pub fn to_screen(v: &View, lat: f64, lon: f64) -> (f64, f64) {
    let dl = (lon - v.lon + 180.0).rem_euclid(360.0) - 180.0;
    (v.w / 2.0 + dl / v.dpp, v.h / 2.0 - (lat - v.lat) / v.dpp)
}

pub fn from_screen(v: &View, x: f64, y: f64) -> (f64, f64) {
    (v.lat - (y - v.h / 2.0) * v.dpp, v.lon + (x - v.w / 2.0) * v.dpp)
}

/// Zoom by `f` (> 1 = in) keeping the point under (x, y) fixed.
pub fn zoom_at(v: &View, f: f64, x: f64, y: f64) -> View {
    let (la, lo) = from_screen(v, x, y);
    let dpp = clamp(View { dpp: v.dpp / f, ..*v }).dpp;
    clamp(View { dpp, lat: la + (y - v.h / 2.0) * dpp, lon: lo - (x - v.w / 2.0) * dpp, ..*v })
}

/// Fit a west/south/east/north box with `pad` px around it.
pub fn fit(b: [f64; 4], w: f64, h: f64, pad: f64) -> View {
    let dpp = ((b[2] - b[0]) / (w - 2.0 * pad).max(10.0)).max((b[3] - b[1]) / (h - 2.0 * pad).max(10.0));
    clamp(View { lon: (b[0] + b[2]) / 2.0, lat: (b[1] + b[3]) / 2.0, dpp, w, h })
}

/// Bounding box of the traverse.
pub fn traverse_box(o: &Overview) -> Option<[f64; 4]> {
    if o.traverse.is_empty() {
        return None;
    }
    let f = |g: fn(&(f64, f64, i64)) -> f64, min: bool| o.traverse.iter().map(g).fold(if min { f64::INFINITY } else { f64::NEG_INFINITY },
                                                                                    |a, b| if min { a.min(b) } else { a.max(b) });
    Some([f(|p| p.1, true), f(|p| p.0, true), f(|p| p.1, false), f(|p| p.0, false)])
}

/// Label baselines (relative to each marker, default 4 px) so that labels (x + 10, y + dy, width w, height `line`) do not
/// overlap: labels are placed top to bottom and pushed down a line at a time (at most 4 lines).
pub fn declutter(pts: &[(f64, f64, f64)], line: f64) -> Vec<f64> {
    let mut order: Vec<usize> = (0..pts.len()).collect();
    order.sort_by(|&a, &b| pts[a].1.total_cmp(&pts[b].1));
    let mut placed: Vec<(f64, f64, f64, f64)> = Vec::new(); // x0, y0, x1, y1
    let mut out = vec![4.0; pts.len()];
    for i in order {
        let (x, y, w) = pts[i];
        let mut dy = 4.0;
        for _ in 0..4 {
            let b = (x + 10.0, y + dy - line + 3.0, x + 10.0 + w, y + dy + 3.0);
            if placed.iter().any(|p| b.0 < p.2 && p.0 < b.2 && b.1 < p.3 && p.1 < b.3) {
                dy += line;
            } else {
                break;
            }
        }
        placed.push((x + 10.0, y + dy - line + 3.0, x + 10.0 + w, y + dy + 3.0));
        out[i] = dy;
    }
    out
}

/// Scale bar along north-south (equirectangular: constant): length in px (<= max_px) and its label.
pub fn scale_bar(dpp: f64, max_px: f64) -> (f64, String) {
    let m_per_px = dpp.to_radians() * MARS_R_M;
    let target = m_per_px * max_px;
    let p = 10f64.powf(target.log10().floor());
    let nice = [5.0, 2.0, 1.0].iter().map(|k| k * p).find(|&x| x <= target).unwrap_or(p);
    let label = if nice >= 1000.0 { format!("{} km", nice / 1000.0) } else { format!("{nice} m") };
    (nice / m_per_px, label)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn real() -> Overview {
        parse(&serde_json::from_slice(&std::fs::read("../assets/overview.json").unwrap()).unwrap()).unwrap()
    }

    #[test]
    fn overview_parses_the_real_bundle() {
        let o = real();
        assert_eq!(o.sites.len(), 9);
        assert!(o.traverse.len() > 1000 && o.sol_first == 0, "{} {}", o.traverse.len(), o.sol_first);
        let cur = o.sites.iter().find(|s| s.title.starts_with("Curiosity")).unwrap();
        assert!((cur.lat - o.traverse[0].0).abs() < 0.01 && (cur.lon - o.traverse[0].1).abs() < 0.01, "landing = first traverse point");
        assert!(o.study[0] < o.study[2] && o.study[1] < o.study[3]);
        let m = missions(&serde_json::from_slice(&std::fs::read("../assets/missions.json").unwrap()).unwrap());
        assert_eq!(m.len(), 5);
        assert!(m.iter().all(|x| !x.uses.is_empty()));
    }

    #[test]
    fn declutter_pushes_overlapping_labels_down() {
        let d = declutter(&[(100.0, 100.0, 80.0), (110.0, 102.0, 80.0), (400.0, 100.0, 80.0)], 15.0);
        assert_eq!(d[0], 4.0);
        assert!(d[1] > 4.0, "{d:?}");
        assert_eq!(d[2], 4.0);
    }

    #[test]
    fn tiles_cover_the_window_and_points_round_trip() {
        let o = real();
        let ctx = o.bases.iter().find(|b| b.id == "ctx").unwrap();
        assert!((deg_per_px(0, 512.0) - 0.3515625).abs() < 1e-12, "the CTX service's own level-0 resolution");
        let v = fit(o.study, 800.0, 600.0, 40.0);
        let t = tiles(ctx, &v);
        assert!(!t.is_empty() && t.len() <= 400);
        let (minl, mint) = t.iter().fold((f64::MAX, f64::MAX), |a, x| (a.0.min(x.left), a.1.min(x.top)));
        let (maxr, maxb) = t.iter().fold((f64::MIN, f64::MIN), |a, x| (a.0.max(x.left + x.size), a.1.max(x.top + x.size)));
        assert!(minl <= 0.0 && mint <= 0.0 && maxr >= 800.0 && maxb >= 600.0, "tiles cover the window");
        let (x, y) = to_screen(&v, o.study[1], o.study[0]);
        let (la, lo) = from_screen(&v, x, y);
        assert!((la - o.study[1]).abs() < 1e-9 && (lo - o.study[0]).abs() < 1e-9);
        let z = zoom_at(&v, 2.0, 100.0, 120.0);
        let (la2, lo2) = from_screen(&z, 100.0, 120.0);
        let (la1, lo1) = from_screen(&v, 100.0, 120.0);
        assert!((la1 - la2).abs() < 1e-9 && (lo1 - lo2).abs() < 1e-9, "zoom keeps the cursor point");
        assert_eq!(o.bases[auto_base(&o, v.dpp)].id, "ctx");
        assert_eq!(o.bases[auto_base(&o, dpp_range(800.0).1)].id, "viking");
        let whole = clamp(View { lon: 137.4, lat: 0.0, dpp: 1.0, w: 800.0, h: 600.0 });
        assert!((whole.dpp - 0.45).abs() < 1e-9, "whole planet fills the width");
        let (px, label) = scale_bar(v.dpp, 120.0);
        assert!(px > 0.0 && px <= 120.0 && label.ends_with('m'), "{px} {label}");
    }
}
