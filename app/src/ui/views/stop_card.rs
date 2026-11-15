//! Stop card (in the inspector) and the photo lightbox.

use crate::ui::model::{CardModel, FactRow};
use crate::text::{self, fill, ui as u};

use super::*;

pub(super) fn fact_item(f: FactRow) -> impl IntoView {
    view! {
        <li>
            <p class="fact-text">{f.text}{f.inference.then(|| view! { " " <span class="badge">{text::stop_card::INFERENCE}</span> })}</p>
            <p class="fact-src">
                <a href=f.url target="_blank" rel="noopener noreferrer">{f.title}</a>
                <span class="src-meta">{f.meta}</span>
            </p>
        </li>
    }
}

/// Station view (Station tab): the science question first, four numbers, the tasks, then the photo and sources folded.
pub(super) fn station_view(m: CardModel, st: Option<crate::ui::model::StopSummary>, window: Option<String>, station_min: f64) -> impl IntoView {
    use crate::ui::card::{send, CardEvent};
    use text::{ops as o, sim as sm, stop_card as t};
    let question = m.plan.iter().find(|p| p.0 == sm::STEP_QUESTION).cloned();
    let tasks: Vec<_> = m.plan.iter().filter(|p| p.0 != sm::STEP_QUESTION).cloned().collect();
    let back = m.stats.first().map(|x| format!("{} {}", x.value, x.unit)).unwrap_or_else(|| o::MISSING.to_string());
    let ground = st.as_ref().and_then(|x| x.ground.clone());
    let pair = st.as_ref().is_some_and(|x| x.pair);
    let under = st.as_ref().is_some_and(|x| x.map_understates());
    let under_line = under.then(|| ground.as_ref().map(|g| fill(o::GROUND_UNDER_TIP, &[("a", format!("{:.1}", g.1)), ("b", format!("{:.1}", g.2))]))).flatten();
    let n_src = m.facts.len();
    view! {
        <article class="card station" aria-labelledby="card-title">
            <div class="card-top">
                <button type="button" class="ghost" on:click=move |_| send(CardEvent::Close)>{u::BACK}</button>
                <span class="card-nav">
                    <button type="button" class="mini" title=t::PREV aria-label=t::PREV on:click=move |_| send(CardEvent::Prev)>"\u{2190}"</button>
                    <button type="button" class="mini" title=t::NEXT aria-label=t::NEXT on:click=move |_| send(CardEvent::Next)>"\u{2192}"</button>
                </span>
            </div>
            <div class="kicker">{m.kicker.clone()}{pair.then(|| view! { " " <span class="pill pair-tag" title=o::PAIR_TIP>{o::PAIR}</span> })}</div>
            <h2 id="card-title">{m.title.clone()}</h2>
            {question.map(|q| view! {
                <p class="question">{q.1}{q.3.then(|| view! { " " <span class="badge">{sm::INFERENCE}</span> })}<span class="refs">{format!(" {}", q.2)}</span></p>
            })}
            {m.photo.clone().map(|p| view! {
                <figure class=if p.borrowed { "photo-main borrowed" } else { "photo-main" }>
                    <button type="button" class="photo-open" title=t::OPEN_PHOTO on:click=move |_| send(CardEvent::Photo(true))>
                        <img src=p.src.clone() alt=p.alt.clone()/>
                        {p.az.map(|a| view! {
                            <span class="view-arrow" role="img" aria-label=p.view.clone() title=t::PHOTO_VIEW_TIP>
                                <span class="n-tag">"N"</span><i style:transform=format!("rotate({a:.0}deg)")>"\u{2191}"</i>
                            </span>
                        })}
                    </button>
                    <figcaption>
                        {(!p.view.is_empty()).then(|| view! { <b title=t::PHOTO_VIEW_TIP>{p.view.clone()}</b><br/> })}
                        <span class="fine">{p.caption.clone()}</span>
                    </figcaption>
                </figure>
            })}
            <div class="nums">
                <div class="n"><span class="k">{o::NUM_WINDOW}</span><span class="v num">{window.unwrap_or_else(|| o::MISSING.to_string())}</span></div>
                <div class="n"><span class="k">{o::NUM_STATION}</span><span class="v num">{format!("{} min", station_min.round() as i64)}</span></div>
                <div class="n"><span class="k">{o::NUM_BACK}</span><span class="v num">{back}</span></div>
                <div class=format!("n {}{}", ground.as_ref().map(|g| format!("g-{}", g.0)).unwrap_or_default(), if under { " g-under" } else { "" })
                    title=ground.as_ref().map(|g| fill(if under { o::GROUND_UNDER_TIP } else { o::GROUND_TIP },
                                                       &[("a", format!("{:.1}", g.1)), ("b", format!("{:.1}", g.2))])).unwrap_or_default()>
                    <span class="k">{o::NUM_GROUND}</span>
                    <span class="v num">{ground.as_ref().map(|g| format!("{:.1} / {:.1}\u{b0}", g.1, g.2)).unwrap_or_else(|| o::MISSING.to_string())}</span></div>
            </div>
            {under_line.map(|x| view! { <p class="warn-line">{x}</p> })}
            {(!tasks.is_empty()).then(|| view! {
                <dl class="plan" title=sm::PLAN_NOTE>
                    {tasks.into_iter().map(|(step, text, refs, inf)| view! {
                        <dt>{step}</dt>
                        <dd>{text}{inf.then(|| view! { " " <span class="badge">{sm::INFERENCE}</span> })}<span class="refs">{format!(" {refs}")}</span></dd>
                    }).collect_view()}
                </dl>
                <p class="fine tag-line">{if m.edited { text::edit::YOURS_TIP } else { sm::PLAN_NOTE }}</p>
            })}
            {m.custom.then(|| view! { <p class="card-note"><strong>{t::CUSTOM_TITLE}</strong>" "{m.custom_note.clone()}</p> })}
            {st.as_ref().map(|x| chem_table(x))}
            {(n_src > 0).then(|| view! {
                <details class="more"><summary>{fill(o::SOURCES, &[("n", n_src.to_string())])}</summary>
                    <ol class="facts">{m.facts.clone().into_iter().map(fact_item).collect_view()}</ol>
                    {(!m.position_note.is_empty()).then(|| view! { <p class="fine">{m.position_note.clone()}</p> })}
                    {(!m.trek_link.is_empty()).then(|| view! {
                        <p class="fine"><a href=m.trek_link.clone() target="_blank" rel="noopener noreferrer" title=u::TREK_OPEN_TIP>{u::TREK_OPEN}</a></p>
                    })}
                </details>
            })}
        </article>
    }
}

/// "Measured nearby": Curiosity's ChemCam / APXS results around the station (wt% oxides).
fn chem_table(s: &crate::ui::model::StopSummary) -> impl IntoView {
    use text::chem as c;
    let r = format!("{}", crate::ui::views_consts::CHEM_R_M);
    let f = |v: Option<f64>| v.map(|x| format!("{x:.1}")).unwrap_or_else(|| "\u{2014}".into());
    let rows = s.chem.clone();
    let more = s.chem_total.saturating_sub(rows.len());
    view! {
        <h4 title=fill(c::TIP, &[("r", r.clone())])>{c::TITLE}</h4>
        {if rows.is_empty() {
            view! { <p class="fine">{fill(c::NONE, &[("r", r.clone())])}</p> }.into_any()
        } else {
            view! {
                <table class="chem">
                    <thead><tr><th scope="col">{c::COL_TARGET}</th><th scope="col">{c::COL_SOL}</th>
                        <th scope="col" title=c::COL_DIST_TIP>{c::COL_DIST}</th><th scope="col">"CaO"</th>
                        <th scope="col">"SiO\u{2082}"</th><th scope="col" title=c::COL_FE_TIP>"FeOT"</th><th scope="col">"SO\u{2083}"</th></tr></thead>
                    <tbody>{rows.into_iter().map(|(ins, name, sol, d, cao, si, fe, so3)| view! {
                        <tr><th scope="row">{name}<small>{format!(" {ins}")}</small></th><td class="num">{sol}</td>
                            <td class="num">{format!("{d:.0} m")}</td>
                            <td class="num">{f(cao)}</td><td class="num">{f(si)}</td><td class="num">{f(fe)}</td><td class="num">{f(so3)}</td></tr>
                    }).collect_view()}</tbody>
                </table>
                {(more > 0).then(|| view! { <p class="fine">{fill(c::MORE, &[("n", more.to_string()), ("r", r.clone())])}</p> })}
            }.into_any()
        }}
    }
}

#[component]
pub(super) fn Lightbox(card: RwSignal<CardModel>) -> impl IntoView {
    use crate::ui::card::{send, CardEvent};
    move || {
        let m = card.get();
        (m.open && m.lightbox).then(|| m.photo.clone()).flatten().map(|p| view! {
            <div class="lightbox" role="dialog" aria-modal="true" aria-label=p.alt.clone() on:click=move |_| send(CardEvent::Photo(false))>
                <figure>
                    <img src=p.src.clone() alt=p.alt.clone()/>
                    <figcaption>{p.caption.clone()}</figcaption>
                </figure>
                <button type="button" class="icon-btn lb-close" title=text::stop_card::CLOSE aria-label=text::stop_card::CLOSE>"\u{2715}"</button>
            </div>
        })
    }
}

// ================================================================= EVA timeline
