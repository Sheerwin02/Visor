//! Crew cue cards: one printable page per station, after the Apollo cuff checklists (a station sketch map plus the task
//! page with clock times). Built as an HTML string from the plan on screen and printed from a body-level container
//! (#cue-print; print CSS hides the app). Map crop = our 0.5 m HiRISE photo (ortho_0p5m.jpg) around the station with the
//! planned route, north up, a 10 m scale bar.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
use crate::text::{cue as t, fill};

/// Side of the map crop (m) and its size on the page (px).
pub const CROP_M: f32 = 80.0;
pub const CROP_PX: f32 = 280.0;

/// One card's content (already resolved from the plan).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Card {
    pub label: String,
    pub kind: String,
    pub window: Option<(String, String)>,
    pub question: String,
    /// step label, text, bag numbers ("B2") with labels
    pub steps: Vec<(&'static str, String, Vec<(String, String)>)>,
    pub walk_back_min: Option<f64>,
    pub ground: Option<(f64, f64)>,
    pub sun_deg: Option<f64>,
    pub latlon: Option<(f64, f64)>,
    pub photo: Option<String>,
    pub e: f32,
    pub n: f32,
    pub abort: String,
    /// the station plan was written by the planner (no citations)
    pub yours: bool,
}

/// Grid size (m) for the map crop and the route (east, north m).
pub struct MapCtx<'a> {
    pub w_m: f32,
    pub h_m: f32,
    pub ortho: &'a str,
    pub route: &'a [(f32, f32)],
}

pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn crop(c: &Card, m: &MapCtx) -> String {
    let k = CROP_PX / CROP_M; // px per m
    let (e0, n1) = (c.e - CROP_M / 2.0, c.n + CROP_M / 2.0); // west edge, north edge of the crop
    let bg = format!("background-image:url('{}');background-size:{:.1}px {:.1}px;background-position:{:.1}px {:.1}px",
                     esc(m.ortho), m.w_m * k, m.h_m * k, -e0 * k, -(m.h_m - n1) * k);
    let pts: Vec<String> = m.route.iter().map(|&(e, n)| format!("{:.1},{:.1}", (e - e0) * k, (n1 - n) * k)).collect();
    let mid = CROP_PX / 2.0;
    format!(concat!("<div class=\"cc-map\" style=\"width:{w}px;height:{w}px;{bg}\">",
                    "<svg width=\"{w}\" height=\"{w}\" viewBox=\"0 0 {w} {w}\">",
                    "<polyline points=\"{pts}\" fill=\"none\" stroke=\"#ffd23f\" stroke-width=\"3\" stroke-linejoin=\"round\"/>",
                    "<circle cx=\"{mid}\" cy=\"{mid}\" r=\"9\" fill=\"none\" stroke=\"#fff\" stroke-width=\"3\"/>",
                    "<circle cx=\"{mid}\" cy=\"{mid}\" r=\"9\" fill=\"none\" stroke=\"#e4572e\" stroke-width=\"1.5\"/>",
                    "<g transform=\"translate({nx},22)\"><path d=\"M0,-14 L7,6 L0,1 L-7,6 Z\" fill=\"#fff\" stroke=\"#000\"/>",
                    "<text y=\"18\" text-anchor=\"middle\" font-size=\"11\" fill=\"#fff\" stroke=\"#000\" stroke-width=\"0.4\">N</text></g>",
                    "<rect x=\"10\" y=\"{sy}\" width=\"{sb}\" height=\"5\" fill=\"#fff\" stroke=\"#000\"/>",
                    "<text x=\"10\" y=\"{ty}\" font-size=\"11\" fill=\"#fff\" stroke=\"#000\" stroke-width=\"0.4\">{scale}</text>",
                    "</svg></div>"),
            w = CROP_PX, bg = bg, pts = pts.join(" "), mid = mid, nx = CROP_PX - 20.0, sy = CROP_PX - 16.0, sb = 10.0 * k,
            ty = CROP_PX - 20.0, scale = t::SCALE)
}

/// The cards as one HTML fragment (one `section.cc` per page).
pub fn html(cards: &[Card], m: &MapCtx) -> String {
    let mut out = String::new();
    for c in cards {
        let mut facts = Vec::new();
        if let Some((a, b)) = &c.window {
            facts.push(fill(t::WINDOW, &[("a", a.clone()), ("b", b.clone())]));
        }
        if let Some(x) = c.walk_back_min {
            facts.push(fill(t::BACK, &[("x", format!("{x:.1}"))]));
        }
        if let Some((a, b)) = c.ground {
            facts.push(fill(t::GROUND, &[("a", format!("{a:.1}")), ("b", format!("{b:.1}"))]));
        }
        if let Some(s) = c.sun_deg {
            facts.push(fill(t::SUN, &[("x", format!("{}", s.round() as i64))]));
        }
        if let Some((la, lo)) = c.latlon {
            facts.push(fill(t::POS, &[("lat", format!("{la:.5}")), ("lon", format!("{lo:.5}"))]));
        }
        let steps: String = c.steps.iter().filter(|s| s.0 != crate::text::sim::STEP_QUESTION).map(|(st, tx, bags)| {
            let b: String = bags.iter().map(|(k, l)| format!(" <b class=\"cc-bag\">{}</b> {}", esc(k), esc(l))).collect();
            format!("<li><span class=\"cc-box\"></span><span class=\"cc-step\">{}</span> {}{}</li>", esc(st), esc(tx), b)
        }).collect();
        out += &format!(concat!("<section class=\"cc\"><header><span class=\"cc-app\">{app}</span><span class=\"cc-kind\">{kind}</span></header>",
                                "<h1>{label}</h1><p class=\"cc-q\">{q}</p><div class=\"cc-row\">{map}<div class=\"cc-side\">",
                                "<ul class=\"cc-facts\">{facts}</ul>{photo}</div></div><ol class=\"cc-steps\">{steps}</ol>",
                                "<p class=\"cc-abort\">{abort}</p><footer>{foot}</footer></section>"),
                        app = t::TITLE, kind = esc(&c.kind), label = esc(&c.label), q = esc(&c.question), map = crop(c, m),
                        facts = facts.iter().map(|f| format!("<li>{}</li>", esc(f))).collect::<String>(),
                        photo = c.photo.as_ref().map(|p| format!("<img class=\"cc-photo\" src=\"{}\" alt=\"\">", esc(p))).unwrap_or_default(),
                        steps = steps, abort = esc(&c.abort),
                        foot = if c.yours { format!("{} {}", crate::text::edit::YOURS_TIP, t::FOOT) } else { t::FOOT.to_string() });
    }
    out
}

/// Put the cards in #cue-print (created on first use) and open the print dialog.
#[cfg(target_arch = "wasm32")]
pub fn print(html: &str) {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else { return };
    let el = doc.get_element_by_id("cue-print").or_else(|| {
        let e = doc.create_element("div").ok()?;
        e.set_id("cue-print");
        doc.body()?.append_child(&e).ok()?;
        Some(e)
    });
    if let Some(el) = el {
        el.set_inner_html(html);
        if let Some(w) = web_sys::window() {
            let _ = w.print();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_card_has_its_times_tasks_bags_crop_and_abort_cue() {
        let c = Card { label: "5 Valle de la Luna".into(), kind: "Hollow".into(), window: Some(("11:17".into(), "12:10".into())),
                       question: "How does the <hollow> differ?".into(),
                       steps: vec![(crate::text::sim::STEP_SAMPLE, "Bedrock beside the drill hole.".into(), vec![("B4".into(), "hollow bedrock".into())])],
                       walk_back_min: Some(7.2), ground: Some((22.8, 6.8)), sun_deg: Some(41.6), latlon: Some((-4.80784, 137.38104)),
                       photo: Some("assets/photo.jpg".into()), e: 300.0, n: 200.0, abort: "If 30 min behind: drop 4".into(), yours: false };
        let route = [(260.0, 200.0), (300.0, 200.0), (300.0, 240.0)];
        let h = html(&[c], &MapCtx { w_m: 553.0, h_m: 1078.0, ortho: "assets/ortho_0p5m.jpg", route: &route });
        assert!(h.contains("11:17") && h.contains("12:10") && h.contains("7.2 min") && h.contains("22.8") && h.contains("B4"));
        assert!(h.contains("&lt;hollow&gt;"), "escaped");
        assert!(h.contains("If 30 min behind"));
        // the station is at the crop centre: route point (300, 200) maps to (140, 140)
        assert!(h.contains("140.0,140.0"), "{h}");
        // background offset: west edge 260 m -> -910 px; north edge 240 m -> (1078 - 240) * 3.5 = 2933 px down
        assert!(h.contains("background-position:-910.0px -2933.0px"), "{h}");
        assert_eq!(h.matches("<section class=\"cc\">").count(), 1);
    }
}
