//! Mars view (rail View: Mars): a streamed planet map from the whole of Mars down to the boxwork patch. NASA landing sites
//! (Mars Trek bookmarks), Curiosity's whole traverse (PLACES), this walk's outline; Viking colour / MOLA relief / CTX
//! basemaps. Drag to pan, wheel to zoom about the cursor, ladder buttons fly to Mars, Gale, the traverse and this walk.
//! Tile maths and projection are in ui/mars.rs (tested natively).
use std::sync::Arc;

use crate::scene::mapview::{BAR_PX, RAIL_L_PX, RAIL_R_PX, TIMELINE_PX};
use crate::text::{fill, mars as t};
use crate::ui::mars::{self, Overview, View};

use super::*;

fn window_size() -> (f64, f64) {
    let w = web_sys::window();
    let iw = w.as_ref().and_then(|w| w.inner_width().ok()).and_then(|v| v.as_f64()).unwrap_or(1280.0);
    let ih = w.as_ref().and_then(|w| w.inner_height().ok()).and_then(|v| v.as_f64()).unwrap_or(800.0);
    ((iw - (RAIL_L_PX + RAIL_R_PX) as f64).max(200.0), (ih - (BAR_PX + TIMELINE_PX) as f64).max(200.0))
}

/// Whole planet, centred on 0 deg east (every landing site in view).
fn whole(_o: &Overview, w: f64, h: f64) -> View {
    mars::clamp(View { lon: 0.0, lat: 0.0, dpp: f64::MAX, w, h })
}

#[component]
pub(super) fn MarsView(open: RwSignal<bool>, ov: RwSignal<Option<Arc<Overview>>>) -> impl IntoView {
    let view = RwSignal::new(None::<View>);
    let drag = RwSignal::new((0.0f64, 0.0f64));
    let start = StoredValue::new(None::<(f64, f64)>);
    let moved = StoredValue::new(false);
    let base = RwSignal::new(None::<usize>); // None = Auto
    let sel = RwSignal::new(None::<usize>);
    // first open (or resize): fit the planet
    Effect::new(move |_| {
        if open.get() {
            if let Some(o) = ov.get() {
                let (w, h) = window_size();
                view.update(|v| match v {
                    Some(x) if (x.w - w).abs() < 1.0 && (x.h - h).abs() < 1.0 => {}
                    Some(x) => *x = mars::clamp(View { w, h, ..*x }),
                    None => *v = Some(whole(&o, w, h)),
                });
            }
        }
    });
    // fit between the button rows (top) and the legend (bottom)
    let fly = move |b: [f64; 4]| {
        let (w, h) = window_size();
        let (top, bottom) = (100.0, 70.0);
        let inner = (h - top - bottom).max(100.0);
        let f = mars::fit(b, w, inner, 32.0);
        let yc = top + inner / 2.0;
        view.set(Some(mars::clamp(View { h, lat: f.lat + (yc - h / 2.0) * f.dpp, ..f })));
    };
    move || {
        if !open.get() {
            return ().into_any();
        }
        let Some(o) = ov.get() else { return view! { <section class="mars"><p class="mars-wait">"\u{2026}"</p></section> }.into_any() };
        let Some(v) = view.get() else { return ().into_any() };
        let bi = base.get().unwrap_or_else(|| mars::auto_base(&o, v.dpp)).min(o.bases.len() - 1);
        let b = o.bases[bi].clone();
        let tiles = mars::tiles(&b, &v);
        let (dx, dy) = drag.get();
        let tf = format!("translate({dx}px, {dy}px)");
        // overlays
        let route: String = {
            let mut s = String::new();
            for (i, p) in o.traverse.iter().enumerate() {
                let (x, y) = mars::to_screen(&v, p.0, p.1);
                s.push_str(&format!("{}{x:.1},{y:.1} ", if i == 0 { "M" } else { "L" }));
            }
            s
        };
        let (sx0, sy0) = mars::to_screen(&v, o.study[3], o.study[0]);
        let (sx1, sy1) = mars::to_screen(&v, o.study[1], o.study[2]);
        let (sw, sh) = ((sx1 - sx0).max(6.0), (sy1 - sy0).max(6.0));
        let (scx, scy) = (sx0 + (sx1 - sx0) / 2.0, sy0 + (sy1 - sy0) / 2.0);
        let (bar_px, bar_label) = mars::scale_bar(v.dpp, 120.0);
        let o2 = o.clone();
        let tr_box = mars::traverse_box(&o);
        let gale = o.sites.iter().find(|s| s.title.starts_with("Curiosity")).map(|s| s.bbox);
        let study = o.study;
        let selected = sel.get().and_then(|i| o.sites.get(i).cloned());
        let label_y = mars::declutter(&o.sites.iter().map(|s| {
            let (x, y) = mars::to_screen(&v, s.lat, s.lon);
            (x, y, 7.0 * s.mission.chars().count() as f64)
        }).collect::<Vec<_>>(), 15.0);
        let trek_link = |b: [f64; 4], lat: f64, lon: f64| format!(
            "https://trek.nasa.gov/mars/#v=0.1&x={lon:.6}&y={lat:.6}&z=10&p=urn%3Aogc%3Adef%3Acrs%3AEPSG%3A%3A104905&d=&locale=&b=mars&e={:.6}%2C{:.6}%2C{:.6}%2C{:.6}&sfz=&w=",
            b[0], b[1], b[2], b[3]);
        view! {
            <section class="mars" aria-label=t::TITLE>
                <div class="mars-map"
                    on:pointerdown=move |e: web_sys::PointerEvent| {
                        start.set_value(Some((e.client_x() as f64, e.client_y() as f64)));
                        moved.set_value(false);
                        if let Some(el) = e.current_target().and_then(|x| x.dyn_into::<web_sys::Element>().ok()) {
                            let _ = el.set_pointer_capture(e.pointer_id());
                        }
                    }
                    on:pointermove=move |e: web_sys::PointerEvent| {
                        if let Some((x0, y0)) = start.get_value() {
                            let d = (e.client_x() as f64 - x0, e.client_y() as f64 - y0);
                            if d.0.abs() + d.1.abs() > 3.0 { moved.set_value(true); }
                            if moved.get_value() { drag.set(d); }
                        }
                    }
                    on:pointerup=move |_| {
                        let (dx, dy) = drag.get_untracked();
                        start.set_value(None);
                        if dx != 0.0 || dy != 0.0 {
                            view.update(|v| if let Some(x) = v { *x = mars::clamp(View { lon: x.lon - dx * x.dpp, lat: x.lat + dy * x.dpp, ..*x }) });
                            drag.set((0.0, 0.0));
                        }
                    }
                    on:wheel=move |e: web_sys::WheelEvent| {
                        e.prevent_default();
                        let r = e.current_target().and_then(|x| x.dyn_into::<web_sys::Element>().ok()).map(|el| el.get_bounding_client_rect());
                        let (x, y) = r.map(|r| (e.client_x() as f64 - r.left(), e.client_y() as f64 - r.top())).unwrap_or((0.0, 0.0));
                        let f = if e.delta_y() < 0.0 { 1.25 } else { 0.8 };
                        view.update(|v| if let Some(c) = v { *c = mars::zoom_at(c, f, x, y) });
                    }>
                    <div class="mars-tiles" style:transform=tf.clone()>
                        {tiles.into_iter().map(|tl| view! {
                            <img src=tl.url alt="" draggable="false" style:left=format!("{:.2}px", tl.left) style:top=format!("{:.2}px", tl.top)
                                style:width=format!("{:.2}px", tl.size + 0.5) style:height=format!("{:.2}px", tl.size + 0.5)
                                on:error=move |e| {
                                    // one retry (a busy tile server), then hide the tile
                                    if let Some(el) = e.target().and_then(|x| x.dyn_into::<web_sys::HtmlImageElement>().ok()) {
                                        let src = el.src();
                                        if src.contains("retry=1") { let _ = web_sys::HtmlElement::style(&el).set_property("visibility", "hidden"); }
                                        else { el.set_src(&format!("{src}{}retry=1", if src.contains('?') { "&" } else { "?" })); }
                                    }
                                }/>
                        }).collect_view()}
                    </div>
                    <svg class="mars-ov" width=v.w.to_string() height=v.h.to_string() style:transform=tf>
                        {(sw > 24.0).then(|| view! {
                            <image href="assets/ortho_0p5m.jpg" x=sx0.to_string() y=sy0.to_string() width=sw.to_string() height=sh.to_string()
                                preserveAspectRatio="none" class="mars-photo"><title>{t::T_PHOTO}</title></image>
                        })}
                        <path d=route class="mars-route"/>
                        <rect x=sx0.to_string() y=sy0.to_string() width=sw.to_string() height=sh.to_string() class="mars-study"/>
                        {(sw < 14.0).then(|| view! { <circle cx=scx.to_string() cy=scy.to_string() r="9" class="mars-study-ring"/> })}
                        {o.sites.iter().enumerate().map(|(i, s)| {
                            let (x, y) = mars::to_screen(&v, s.lat, s.lon);
                            let ly = label_y[i];
                            let on = sel.get() == Some(i);
                            view! {
                                <g class=if on { "mars-site on" } else { "mars-site" } transform=format!("translate({x:.1},{y:.1})")
                                    on:pointerdown=move |e: web_sys::PointerEvent| e.stop_propagation()
                                    on:click=move |_| sel.set(if sel.get_untracked() == Some(i) { None } else { Some(i) })>
                                    <title>{format!("{} \u{b7} {}", s.mission, s.title)}</title>
                                    <path d="M0,-7 L6,0 L0,7 L-6,0 Z"/>
                                    {(ly != 4.0).then(|| view! { <line x1="6" y1="0" x2="10" y2=(ly - 4.0).to_string() class="mars-leader"/> })}
                                    <text x="10" y=ly.to_string()>{s.mission.clone()}</text>
                                </g>
                            }
                        }).collect_view()}
                    </svg>
                </div>
                <div class="mars-ladder" role="group" aria-label=t::TITLE>
                    <button type="button" on:click=move |_| { let (w, h) = window_size(); view.set(Some(whole(&o2, w, h))); }>{t::GO_MARS}</button>
                    {gale.map(|g| view! { <button type="button" title=t::T_GO_GALE on:click=move |_| fly(g)>{t::GO_GALE}</button> })}
                    {tr_box.map(|b| view! { <button type="button" title=fill(t::T_GO_TRAVERSE, &[("a", o.sol_first.to_string()), ("b", o.sol_last.to_string())])
                        on:click=move |_| fly(b)>{t::GO_TRAVERSE}</button> })}
                    <button type="button" title=t::T_GO_WALK on:click=move |_| fly(study)>{t::GO_WALK}</button>
                    <button type="button" class="primary" on:click=move |_| open.set(false)>{t::OPEN_PLANNER}</button>
                </div>
                <div class="mars-bases seg" role="group">
                    <button type="button" title=t::T_BASE_AUTO aria-pressed=if base.get().is_none() { "true" } else { "false" }
                        on:click=move |_| base.set(None)>{t::BASE_AUTO}</button>
                    {o.bases.iter().enumerate().map(|(i, bb)| view! {
                        <button type="button" title=format!("{} \u{b7} {}", bb.detail, bb.credit) aria-pressed=if base.get() == Some(i) { "true" } else { "false" }
                            on:click=move |_| base.set(Some(i))>{bb.title.clone()}</button>
                    }).collect_view()}
                </div>
                <div class="mars-zoom">
                    <button type="button" aria-label=t::ZOOM_IN title=t::ZOOM_IN
                        on:click=move |_| view.update(|v| if let Some(c) = v { *c = mars::zoom_at(c, 2.0, c.w / 2.0, c.h / 2.0) })>"+"</button>
                    <button type="button" aria-label=t::ZOOM_OUT title=t::ZOOM_OUT
                        on:click=move |_| view.update(|v| if let Some(c) = v { *c = mars::zoom_at(c, 0.5, c.w / 2.0, c.h / 2.0) })>"\u{2212}"</button>
                </div>
                <div class="mars-legend">
                    <span><i class="lg-site"></i>{t::LEG_SITE}</span>
                    <span><i class="lg-route"></i>{fill(t::LEG_ROUTE, &[("a", o.sol_first.to_string()), ("b", o.sol_last.to_string())])}</span>
                    <span><i class="lg-walk"></i>{t::LEG_WALK}</span>
                    <span class="mars-scale" title=t::SCALE_NS><i style:width=format!("{bar_px:.0}px")></i>{bar_label}</span>
                    <span class="fine">{t::HELP}</span>
                    <span class="mars-credit" title=t::TILES_NOTE>{format!("{} \u{b7} {} \u{b7} {}", b.credit, o.credit_sites, o.traverse_credit)}</span>
                </div>
                {selected.map(|s| view! {
                    <aside class="mars-card">
                        <button type="button" class="ghost close" aria-label=t::CLOSE on:click=move |_| sel.set(None)>"\u{2715}"</button>
                        <h3>{s.mission.clone()}</h3>
                        <p class="fine">{s.title.clone()}</p>
                        {s.image.clone().map(|src| view! { <img src=src alt="" loading="lazy"/> })}
                        <p class="desc">{s.description.clone()}</p>
                        <p class="fine">{fill(t::POSITION, &[("x", s.position.clone())])}" "{s.source.clone()}</p>
                        <p class="fine"><a href=trek_link(s.bbox, s.lat, s.lon) target="_blank" rel="noopener noreferrer">{t::OPEN_TREK}</a></p>
                        {s.title.starts_with("Curiosity").then(|| view! {
                            <div class="ctl-row">
                                <button type="button" on:click=move |_| fly(study)>{t::GO_WALK}</button>
                                <button type="button" class="primary" on:click=move |_| open.set(false)>{t::OPEN_PLANNER}</button>
                            </div>
                        })}
                    </aside>
                })}
            </section>
        }.into_any()
    }
}

/// "Data from N NASA missions": each mission with what VISOR uses (About tab).
pub(super) fn missions_block(ms: Vec<mars::Mission>) -> impl IntoView {
    (!ms.is_empty()).then(|| view! {
        <h4 title=t::MISSIONS_TIP>{fill(t::MISSIONS, &[("n", ms.len().to_string())])}</h4>
        <ul class="missions">
            {ms.into_iter().map(|m| view! {
                <li>
                    <b>{m.short.clone()}</b><span class="fine">{m.name.clone()}</span>
                    <ul class="plain">
                        {m.uses.into_iter().map(|u| view! {
                            <li title=u.files.join(", ")><span class="inst">{u.instrument}</span>" \u{2014} "{u.what}
                                {u.live.map(|l| view! { " " <span class="tag" title=l>{t::LIVE}</span> })}</li>
                        }).collect_view()}
                    </ul>
                </li>
            }).collect_view()}
        </ul>
    })
}
