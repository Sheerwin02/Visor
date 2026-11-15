//! Drill-in card (scene/select.rs, ui/drill.rs): what was clicked on the map, a few measured numbers, and actions:
//! Look around (Curiosity's 360-degree Navcam view), Walk from here, Back to map. Sits over the map, bottom left.

use crate::ui::model::DrillModel;
use crate::text::{self};

use super::*;

#[component]
pub(super) fn DrillCard(drill: RwSignal<DrillModel>, chrome: RwSignal<ChromeModel>) -> impl IntoView {
    use crate::scene::select::{send, SelEvent};
    use text::drill as t;
    let pano: RwSignal<Option<(crate::ui::model::Pano, f64)>> = RwSignal::new(None);
    view! {
        {move || {
            let m = drill.get();
            if !m.open || chrome.get().walking {
                return ().into_any();
            }
            let p = m.pano.clone();
            view! {
                <section id="drill" aria-label=m.title.clone()>
                    <div class="dr-head">
                        <span class="dr-kind">{m.kind}</span>
                        <button type="button" class="mini" title=t::A_CLOSE aria-label=t::A_CLOSE on:click=move |_| send(SelEvent::Close)>"\u{00d7}"</button>
                    </div>
                    <h3>{m.title.clone()}</h3>
                    {m.photo.clone().map(|src| view! { <img class="dr-photo" src=src alt=m.title.clone()/> })}
                    <dl class="dr-rows">
                        {m.rows.iter().map(|(k, v)| view! { <dt>{*k}</dt><dd class="num">{v.clone()}</dd> }).collect_view()}
                    </dl>
                    {(m.profile.len() > 1).then(|| profile_chart(m.profile.clone(), m.caution, m.limit))}
                    {(!m.note.is_empty()).then(|| view! { <p class="dr-note">{m.note.clone()}</p> })}
                    <div class="dr-actions">
                        {p.map(|p| view! {
                            <button type="button" class="primary" title=text::sim::PANO_TIP on:click=move |_| pano.set(Some((p.clone(), 0.0)))>{t::A_LOOK}</button>
                        })}
                        {m.acts.iter().cloned().map(|(label, tip, act, primary)| view! {
                            <button type="button" class=if primary { "primary place-act" } else { "place-act" } title=tip
                                on:click=move |_| crate::scene::place::send(act.clone())>{label}</button>
                        }).collect_view()}
                        {m.can_walk.then(|| view! { <button type="button" on:click=move |_| send(SelEvent::WalkFrom)>{t::A_WALK}</button> })}
                        <button type="button" class="ghost" on:click=move |_| send(SelEvent::BackToMap)>{t::A_MAP}</button>
                    </div>
                </section>
            }.into_any()
        }}
        {move || pano.get().map(|(p, h0)| pano_view(p, h0, pano))}
    }
}

/// Map key: the symbols of the layers that are on, top left of the map; collapsible.
#[component]
pub(super) fn MapKey(layers: RwSignal<crate::ui::model::LayersModel>, chrome: RwSignal<ChromeModel>, mars: RwSignal<bool>) -> impl IntoView {
    use text::mapkey as t;
    move || {
        let l = layers.get();
        if chrome.get().walking || mars.get() {
            return ().into_any();
        }
        let row = |sw: &'static str, label: &'static str| view! { <li><i class=format!("mk {sw}") aria-hidden="true"></i>{label}</li> };
        view! {
            <details id="mapkey" open>
                <summary>{t::TITLE}</summary>
                <ul>
                    {(!l.walking).then(|| row("mk-contour", t::CONTOUR))}
                    {(!l.walking).then(|| row("mk-shape", t::SHAPE))}
                    {l.route.then(|| row("mk-route", t::ROUTE))}
                    {l.stops.then(|| view! { <li><i class="mk mk-pin" aria-hidden="true">"1"</i>{t::STATION}</li> })}
                    {l.stops.then(|| view! { <li><span class="mk-badges" aria-hidden="true">
                        <b style="background:#2f8f5b">"\u{2713}"</b><b style="background:#c98a1e;color:#111">"\u{2260}"</b><b style="background:#d6331f">"!"</b>
                        </span>{t::BADGES}</li> })}
                    {l.stops.then(|| view! { <li><i class="mk-pair" aria-hidden="true"></i>{t::PAIR}</li> })}
                    {l.path.then(|| row("mk-path", t::PATH))}
                    {l.back.then(|| row("mk-back", t::BACK))}
                    {l.checks.then(|| view! { <li><span class="mk-checks" aria-hidden="true">"\u{2713} ~ \u{2260}"</span>{t::CHECKS}</li> })}
                    {(l.chem && l.has_chem).then(|| row("mk-chem", t::CHEM))}
                    {l.slope.then(|| row("mk-slope", t::SLOPE))}
                    {l.agreement.then(|| row("mk-agree", t::AGREE))}
                </ul>
            </details>
        }.into_any()
    }
}

/// A leg's height profile: steep stretches hatched as on the map (corrected slope over the caution
/// and limit), the steepest point marked; hover marks the spot on the map (panel::HOVER), like the timeline.
fn profile_chart(p: Vec<crate::ui::panel::Sample>, caution: f32, limit: f32) -> impl IntoView {
    use crate::ui::panel::{band_runs, nearest, HOVER};
    let (w, h, pad) = (276.0f32, 86.0f32, 4.0f32);
    let (d0, d1) = (p[0].d, p.last().map(|q| q.d).unwrap_or(1.0));
    let (zlo, zhi) = p.iter().fold((f32::MAX, f32::MIN), |a, q| (a.0.min(q.z), a.1.max(q.z)));
    let zspan = (zhi - zlo).max(1.0);
    let x = move |d: f32| pad + (d - d0) / (d1 - d0).max(1.0) * (w - 2.0 * pad);
    let y = move |z: f32| h - 16.0 - (z - zlo) / zspan * (h - 30.0);
    let pts: Vec<String> = p.iter().map(|q| format!("{:.1},{:.1}", x(q.d), y(q.z))).collect();
    let area = format!("M{:.1},{:.1} L{} L{:.1},{:.1} Z", x(d0), h - 16.0, pts.join(" L"), x(d1), h - 16.0);
    let steep = p.iter().copied().max_by(|a, b| a.s.total_cmp(&b.s));
    let hover = RwSignal::new(None::<usize>);
    let pa = std::sync::Arc::new(p);
    let (pm, pt) = (pa.clone(), pa.clone());
    let bands = band_runs(&pa, caution, limit);
    view! {
        <svg class="dr-chart" viewBox=format!("0 0 {w} {h}") width=w.to_string() height=h.to_string() role="img"
            aria-label=crate::text::drill::PROFILE
            on:pointermove=move |ev: web_sys::PointerEvent| {
                use wasm_bindgen::JsCast;
                let Some(el) = ev.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) else { return };
                let r = el.get_bounding_client_rect();
                let f = ((ev.client_x() as f64 - r.left()) / r.width().max(1.0)) as f32;
                let i = nearest(&pm, d0 + f * (d1 - d0));
                hover.set(Some(i));
                if let Ok(mut g) = HOVER.lock() { *g = Some((pm[i].e, pm[i].n)); }
            }
            on:pointerleave=move |_| { hover.set(None); if let Ok(mut g) = HOVER.lock() { *g = None; } }>
            <defs>
                <pattern id="dr-h1" width="5" height="5" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
                    <line x1="0" y1="0" x2="0" y2="5" stroke="rgba(255,255,255,0.4)" stroke-width="1.2"/></pattern>
                <pattern id="dr-h2" width="5" height="5" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
                    <line x1="0" y1="0" x2="0" y2="5" stroke="rgba(255,255,255,0.45)" stroke-width="1.2"/>
                    <line x1="0" y1="0" x2="5" y2="0" stroke="rgba(255,255,255,0.45)" stroke-width="1.2"/></pattern>
            </defs>
            {bands.into_iter().map(|(b, a, c)| view! {
                <rect x=x(a).to_string() y="2" width=(x(c) - x(a)).max(1.0).to_string() height=(h - 18.0).to_string() fill=format!("url(#dr-h{b})")/>
            }).collect_view()}
            <path d=area class="dr-area"/>
            <path d=format!("M{}", pts.join(" L")) class="dr-line"/>
            {steep.map(|q| view! {
                <circle cx=x(q.d).to_string() cy=y(q.z).to_string() r="3.5" class="dr-steep"/>
                <text x=x(q.d).clamp(30.0, w - 30.0).to_string() y=(y(q.z) - 7.0).max(10.0).to_string() text-anchor="middle" class="dr-tick">
                    {format!("{:.0}\u{b0}", q.s)}</text>
            })}
            <text x=pad.to_string() y=(h - 3.0).to_string() class="dr-tick">"0 m"</text>
            <text x=(w - pad).to_string() y=(h - 3.0).to_string() text-anchor="end" class="dr-tick">{format!("{:.0} m", d1 - d0)}</text>
            <text x=(w / 2.0).to_string() y=(h - 3.0).to_string() text-anchor="middle" class="dr-tick">{format!("\u{2195} {:.1} m", zhi - zlo)}</text>
            {move || hover.get().map(|i| {
                let q = pt[i];
                view! {
                    <line x1=x(q.d).to_string() x2=x(q.d).to_string() y1="2" y2=(h - 16.0).to_string() class="dr-cross"/>
                    <text x=x(q.d).clamp(40.0, w - 40.0).to_string() y="12" text-anchor="middle" class="dr-tick dr-hv">
                        {format!("{:.0} m \u{b7} {:.0}\u{b0}{}", q.d - d0, q.s, if q.seen { "" } else { " \u{b7} unseen" })}</text>
                }
            })}
        </svg>
    }
}
