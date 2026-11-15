//! Inspector (right): Brief, Plan, Stops, Conditions, Trust and About tabs; a stop's card replaces the tab while open.

use crate::ui::brief::BriefModel;
use crate::ui::model::{CardModel, Controls, LightModel, PanelModel, StopSummary};
use crate::plan::planner::{send, PanelEvent};
use crate::text::{self, fill, ui as u};

use super::*;

/// Station windows (start, end s) by walking-order stop index, the EVA end (s) and the start hour, from the simulation.
#[derive(Clone, Debug, PartialEq, Default)]
pub(super) struct Windows {
    by_stop: Vec<(usize, f32, f32)>,
    egress: Option<(f32, f32)>,
    end: f32,
    start_h: f64,
    sun: Vec<(f32, f32)>,
    /// every EVA block (kind, stop, start, end s, label) for the budget bar
    blocks: Vec<(&'static str, Option<usize>, f32, f32, String)>,
}

impl Windows {
    /// minutes after local midnight (Mars local true solar time) at EVA time t
    fn minutes(&self, t: f32) -> f64 {
        (self.start_h + t as f64 / route_core::light::MARS_HOUR_S) * 60.0
    }
    fn sun_at(&self, t: f32) -> Option<f64> {
        self.sun.iter().min_by(|a, b| (a.0 - t).abs().total_cmp(&(b.0 - t).abs())).map(|x| x.1 as f64)
    }
    fn of(&self, i: usize) -> Option<(f32, f32)> {
        if i == 0 {
            return self.egress;
        }
        self.by_stop.iter().find(|w| w.0 == i).map(|w| (w.1, w.2))
    }
    fn hm(&self, t: f32) -> String {
        clock(self.start_h, t as f64)
    }
}

fn windows_of(s: &crate::plan::simrun::SimModel) -> Windows {
    Windows {
        by_stop: s.blocks.iter().filter_map(|b| b.stop.map(|k| (k, b.t0, b.t1))).collect(),
        egress: s.blocks.iter().find(|b| b.kind == "egress").map(|b| (b.t0, b.t1)),
        end: s.blocks.last().map(|b| b.t1).unwrap_or(0.0),
        start_h: s.start_h,
        sun: s.sun.clone(),
        blocks: s.blocks.iter().map(|b| (b.kind, b.stop, b.t0, b.t1, b.label.clone())).collect(),
    }
}

/// The EVA as one bar: egress, walking legs, numbered stations, ingress
/// and the spare time to the EVA length, to scale; hover a segment for its times.
fn budget_bar(w: &Windows, eva_s: f64, go: bool, tip: String) -> impl IntoView {
    let total = (eva_s as f32).max(w.end).max(1.0);
    let pct = move |a: f32, b: f32| format!("{:.3}%", ((b - a) / total * 100.0).max(0.0));
    let spare = (eva_s as f32 - w.end).max(0.0);
    let segs: Vec<(String, String, String, String)> = w.blocks.iter().map(|(k, stop, a, b, label)| {
        let txt = match (*k, stop) { ("station", Some(i)) => (i + 1).to_string(), _ => String::new() };
        (format!("bb bb-{k}"), pct(*a, *b), txt,
         format!("{label} \u{b7} {}\u{2013}{} \u{b7} {} min", w.hm(*a), w.hm(*b), ((b - a) / 60.0).round() as i64))
    }).collect();
    let hours = (eva_s / 3600.0 * 10.0).round() / 10.0;
    view! {
        <div class=if go { "budget go" } else { "budget nogo" } title=tip>
            <div class="bb-row" role="img" aria-label=text::ops::BUDGET_ARIA>
                {segs.into_iter().map(|(cls, wd, txt, t)| view! { <span class=cls style:width=wd title=t>{txt}</span> }).collect_view()}
                {(spare > 1.0).then(|| view! {
                    <span class="bb bb-spare" style:width=pct(0.0, spare)
                        title=fill(text::ops::BUDGET_SPARE, &[("x", format!("{}", (spare / 60.0).round() as i64))])></span> })}
            </div>
            <div class="bb-scale"><span>{w.hm(0.0)}</span><span>{fill(text::ops::BUDGET_BACK, &[("end", w.hm(w.end))])}</span>
                <span>{fill(text::ops::BUDGET_EVA, &[("hours", format!("{hours}"))])}</span></div>
        </div>
    }
}

#[component]
pub(super) fn Inspector(panel: RwSignal<Option<PanelModel>>, light: RwSignal<LightModel>, card: RwSignal<CardModel>,
                        brief: RwSignal<BriefModel>, crew: RwSignal<crate::plan::crew::CrewModel>,
                        missions: RwSignal<Vec<crate::ui::mars::Mission>>,
                        sim: RwSignal<crate::plan::simrun::SimModel>,
                        exec: RwSignal<crate::ui::execlog::ExecLog>,
                        placed: RwSignal<crate::scene::place::PlacedModel>) -> impl IntoView {
    use text::ops as o;
    let tab = RwSignal::new(0usize);
    // saved plans (per viewer) and the station-plan editor's rows (None = not editing)
    let saved = RwSignal::new(load_plans());
    Effect::new(move |_| saved.with(|p| save_plans(p)));
    let editing = RwSignal::new(None::<Vec<crate::plan::edits::EditTask>>);
    // the EVA log is kept per viewer in this browser (a convenience; Download log is the record)
    Effect::new(move |_| exec.with(|l| save_log(l)));
    // planner priorities per stop id (UI setting; default: the drill pair P1, others P2) and "behind by" minutes
    let prio = RwSignal::new(std::collections::HashMap::<String, u8>::new());
    let behind = RwSignal::new(0u32);
    // the station windows change only when the plan or the EVA plan changes (the clock does not re-render the panel)
    let win = Memo::new(move |_| sim.with(windows_of));
    // first twilight / night inside the EVA (changes with the sol and start hour, not with the clock)
    let dark = Memo::new(move |_| sim.with(|s| s.dark_note.clone()));
    // a stop card opening (map label, table row, timeline block) shows the Station tab
    // only when a card is opened or moves to another stop (not when the same card is refreshed by a re-plan)
    let opened = Memo::new(move |_| card.with(|c| c.open.then_some(c.index)));
    Effect::new(move |_| {
        if opened.get().is_some() {
            tab.set(1);
        }
    });
    let tabs = [u::TAB_PLAN, o::TAB_STATION, text::exec::TAB, o::TAB_RISK, text::science::TAB, u::TAB_ABOUT];
    move || {
        let Some(m) = panel.get() else { return ().into_any() };
        let t = tab.get();
        let body = match t {
            0 => plan_overview(&m, crew, win.get(), prio, behind, saved, tab, light.get(), dark.get(), placed.get()).into_any(),
            1 => station_tab(card.get(), &m, win.get(), crew.with(|c| c.station_min), prio, editing).into_any(),
            2 => log_tab(&m, win.get(), exec, sim, prio, crew.with(|c| c.station_min)).into_any(),
            3 => view! { {safety(&m, &crew.get(), None, dark.get(), placed.get())} {risk_tab(&m, brief.get(), crew.get())} }.into_any(),
            4 => science_tab(&m).into_any(),
            _ => view! {
                {data_kinds()}
                {about_tab(m.clone(), brief.with(|b| b.trek_link.clone()))}
                {missions_block(missions.get())}
                <details class="more"><summary>{o::CONDITIONS}</summary>{sun_tab(light)}</details>
            }.into_any(),
        };
        view! {
            <nav class="tabs" role="tablist">
                {tabs.iter().enumerate().map(|(i, name)| view! {
                    <button type="button" role="tab" aria-selected=if i == t { "true" } else { "false" }
                        on:click=move |_| tab.set(i)>{*name}</button>
                }).collect_view()}
            </nav>
            <div class="tab-body">{body}</div>
        }.into_any()
    }
}

/// Priority of a station (planner setting; default: drill pair P1, others P2).
fn prio_of(prio: RwSignal<std::collections::HashMap<String, u8>>, s: &StopSummary) -> u8 {
    prio.with_untracked(|p| p.get(&if s.id.is_empty() { s.label.clone() } else { s.id.clone() }).copied())
        .unwrap_or(crate::ui::contingency::default_priority(s.pair))
}

/// Stations to drop when `behind` minutes late (contingency rule) and their names.
fn drops(m: &PanelModel, prio: RwSignal<std::collections::HashMap<String, u8>>, station_min: f64, behind: f64) -> (Vec<usize>, f64, Vec<String>) {
    use crate::ui::contingency::{drop_plan, Station};
    let st: Vec<Station> = m.stop_cards.iter().filter(|s| s.index > 0).map(|s| Station { index: s.index, priority: prio_of(prio, s), pair: s.pair }).collect();
    let (d, gave) = drop_plan(&st, behind, station_min);
    let names = d.iter().filter_map(|i| m.stop_cards.get(*i).map(|s| s.label.clone())).collect();
    (d, gave, names)
}

/// Cue cards for the stations (all, or one walking-order index).
fn cue_cards(m: &PanelModel, w: &Windows, prio: RwSignal<std::collections::HashMap<String, u8>>, station_min: f64, only: Option<usize>) -> String {
    use crate::ui::cuecard::{html, Card, MapCtx};
    let (_, gave, names) = drops(m, prio, station_min, 30.0);
    let abort = if names.is_empty() || gave < 30.0 { text::cue::ABORT_NONE.to_string() }
                else { fill(text::cue::ABORT, &[("x", names.join(", "))]) };
    let mut k = 0;
    let mut cards = Vec::new();
    for s in &m.stop_cards {
        let steps: Vec<_> = s.steps.iter().map(|(st, tx, bags)| (*st, tx.clone(), bags.iter().map(|b| { k += 1; (fill(text::exec::BAG, &[("k", k.to_string())]), b.clone()) }).collect())).collect();
        if s.index == 0 || only.is_some_and(|o| o != s.index) {
            continue;
        }
        let win = w.of(s.index);
        cards.push(Card {
            label: s.label.clone(), kind: s.kind.clone(), window: win.map(|(a, b)| (w.hm(a), w.hm(b))), question: if s.question_inference { format!("{} {}", s.question, text::cue::INFERENCE) } else { s.question.clone() }, steps,
            walk_back_min: s.walk_back_s.map(|x| x / 60.0), ground: s.ground.as_ref().map(|g| (g.1, g.2)),
            sun_deg: win.and_then(|(a, _)| w.sun_at(a)), latlon: s.latlon, photo: s.photo.clone(), e: s.e, n: s.n, abort: abort.clone(), yours: s.edited,
        });
    }
    let route: Vec<(f32, f32)> = m.samples.iter().map(|q| (q.e, q.n)).collect();
    html(&cards, &MapCtx { w_m: 553.0, h_m: 1078.0, ortho: "assets/ortho_0p5m.jpg", route: &route })
}

/// Length of a mean solar day on Mars in Earth seconds over 86 400 (Mars24 / Allison and McEwen 2000: 88 775.244 s):
/// one Mars local-solar-time minute in Earth minutes.
const MARS_MIN_PER_EARTH_MIN: f64 = 88_775.244 / 86_400.0;

/// Plan: one GO line, the station table (time window, priority, bags, map vs rover), the bags, "if we run late", and the
/// route rules and crew settings in a closed drawer.
fn plan_overview(m: &PanelModel, crew: RwSignal<crate::plan::crew::CrewModel>, w: Windows,
                 prio: RwSignal<std::collections::HashMap<String, u8>>, behind: RwSignal<u32>,
                 saved: RwSignal<Vec<crate::ui::plans::SavedPlan>>, tab: RwSignal<usize>, light: LightModel,
                 dark: Option<String>, placed: crate::scene::place::PlacedModel) -> impl IntoView {
    use crate::ui::contingency::{default_priority, drop_plan, Station};
    use text::ops as o;
    let c = crew.get();
    let n = m.stop_cards.len().saturating_sub(1);
    let key = |s: &StopSummary| if s.id.is_empty() { s.label.clone() } else { s.id.clone() };
    let pr = move |s: &StopSummary| prio.with(|p| p.get(&if s.id.is_empty() { s.label.clone() } else { s.id.clone() }).copied())
        .unwrap_or(default_priority(s.pair));
    let go = fill(o::GO_LINE, &[("verdict", c.verdict.clone()), ("hours", format!("{}", c.hours)), ("n", n.to_string()),
                                 ("x", format!("{}", c.station_min.round() as i64)), ("end", w.hm(w.end))]);
    // bags, numbered in walking order
    let mut bag_rows: Vec<(String, String, String)> = Vec::new();
    for s in &m.stop_cards {
        for b in &s.bags {
            bag_rows.push((format!("B{}", bag_rows.len() + 1), s.label.clone(), b.clone()));
        }
    }
    let n_bags = bag_rows.len();
    // contingency
    let stations: Vec<Station> = m.stop_cards.iter().filter(|s| s.index > 0).map(|s| Station { index: s.index, priority: pr(s), pair: s.pair }).collect();
    let late = behind.get();
    let (dropped, gave) = drop_plan(&stations, late as f64, c.station_min);
    let names: Vec<String> = dropped.iter().filter_map(|i| m.stop_cards.get(*i).map(|s| s.label.clone())).collect();
    let late_line = if late == 0 {
        o::LATE_NONE.to_string()
    } else if gave < late as f64 {
        fill(o::LATE_SHORT, &[("x", late.to_string())])
    } else {
        fill(o::LATE_DROP, &[("stops", names.join(", ")), ("x", format!("{}", gave.round() as i64))])
    };
    let can_apply = late > 0 && !dropped.is_empty() && gave >= late as f64 && dropped.len() < n;
    let to_drop = dropped.clone();
    // the windows are on the Mars clock (LTST); station minutes are Earth minutes
    let mars_clock = fill(o::MARS_CLOCK, &[("x", format!("{}", c.station_min.round() as i64)),
                                           ("y", format!("{:.1}", c.station_min / MARS_MIN_PER_EARTH_MIN))]);
    view! {
        {plans_bar(m, &w, c.go, saved, false)}
        {safety(m, &c, Some(tab), dark, placed)}
        {conditions(&light, c.hours)}
        {budget_bar(&w, c.hours * 3600.0, c.go, format!("{go} \u{b7} {}", c.lines.join(" \u{b7} ")))}
        {plan_delta(m)}
        <table class="stations">
            <thead><tr>
                <th scope="col">{o::COL_STATION}</th><th scope="col" title=mars_clock.clone()>{o::COL_WINDOW}</th>
                <th scope="col" title=o::PRIO_TIP>{o::COL_PRIO}</th><th scope="col">{o::COL_BAGS}</th><th scope="col">{o::COL_GROUND}</th>
            </tr></thead>
            <tbody>
                {m.stop_cards.iter().map(|s| {
                    let i = s.index;
                    let k = key(s);
                    let p = pr(s);
                    // a station the timeline could not schedule because a leg before it has no route
                    let blocked = !m.feasible && w.of(i).is_none() && !m.blocked_legs().is_empty();
                    let win = if blocked { o::NO_ROUTE_CELL.to_string() }
                              else { w.of(i).map(|(a, b)| format!("{}\u{2013}{}", w.hm(a), w.hm(b))).unwrap_or_else(|| o::MISSING.to_string()) };
                    let win_tip = blocked.then(|| fill(o::NO_ROUTE_TIP, &[("legs", m.blocked_label())])).unwrap_or_default();
                    let (gm, gt, gc) = match &s.ground {
                        Some((l, a, b)) => (match l.as_str() { "agrees" => o::GROUND_AGREES, "partial" => o::GROUND_PARTIAL, _ => o::GROUND_DISAGREES },
                                            fill(if s.map_understates() { o::GROUND_UNDER_TIP } else { o::GROUND_TIP }, &[("a", format!("{a:.1}")), ("b", format!("{b:.1}"))]),
                                            format!("g-{l}{}", if s.map_understates() { " g-under" } else { "" })),
                        None => (o::MISSING, String::new(), String::new()),
                    };
                    view! {
                        <tr class=if s.pair { "pick pair" } else { "pick" } on:click=move |_| crate::ui::card::send(crate::ui::card::CardEvent::Open(i))>
                            <td><button type="button" class="sn" on:click=move |_| crate::ui::card::send(crate::ui::card::CardEvent::Open(i))>{s.label.clone()}</button>
                                {s.pair.then(|| view! { " " <span class="pill pair-tag" title=o::PAIR_TIP>{o::PAIR}</span> })}</td>
                            <td class=if blocked { "num blocked" } else { "num" } title=win_tip>{win}</td>
                            <td>{(i > 0).then(|| view! {
                                <button type="button" class=format!("prio p{p}") title=o::PRIO_TIP
                                    on:click=move |ev| { ev.stop_propagation(); let k = k.clone(); prio.update(|m| { m.insert(k, if p >= 3 { 1 } else { p + 1 }); }) }>{format!("P{p}")}</button>
                            })}</td>
                            <td class="num">{if s.bags.is_empty() { String::new() } else { s.bags.len().to_string() }}</td>
                            <td class=format!("num ground {gc}") title=gt>{gm}</td>
                        </tr>
                    }
                }).collect_view()}
            </tbody>
        </table>
        {(n_bags > 0).then(|| view! {
            <h4 title=o::BAG_TIP>{fill(o::SAMPLES, &[("n", n_bags.to_string())])}</h4>
            <ol class="bags">{bag_rows.into_iter().map(|(b, st, what)| view! { <li><b>{b}</b>" "{what}<span class="fine">{format!(" \u{b7} {st}")}</span></li> }).collect_view()}</ol>
        })}
        <h4 title=o::LATE_RULE>{o::LATE}</h4>
        <div class="late">
            <div class="seg" role="group" aria-label=o::LATE>
                {[0u32, 15, 30, 60].into_iter().map(|x| view! {
                    <button type="button" aria-pressed=if x == late { "true" } else { "false" } on:click=move |_| behind.set(x)>
                        {if x == 0 { "0".to_string() } else { format!("+{x}") }}</button>
                }).collect_view()}
            </div>
            <p class="late-line">{late_line}</p>
            {can_apply.then(|| view! {
                <button type="button" class="primary" title=o::LATE_APPLY_TIP on:click=move |_| {
                    for &i in to_drop.iter().rev() { send(PanelEvent::Remove(i)); }
                    behind.set(0);
                }>{o::LATE_APPLY}</button>
            })}
        </div>
        <div class="ctl-row">
            <button type="button" title=text::cue::PRINT_TIP on:click={
                let (m2, w2) = (m.clone(), w.clone());
                let sm = c.station_min;
                move |_| { let _h = cue_cards(&m2, &w2, prio, sm, None); #[cfg(target_arch = "wasm32")] crate::ui::cuecard::print(&_h); }
            }>{text::cue::PRINT_ALL}</button>
        </div>
        {plans_bar(m, &w, c.go, saved, true)}
        {glossary(&m.vals)}
        <details class="more rules"><summary>{o::RULES}</summary>
            {plan_tab(m.controls.clone(), m.stops.clone())}
            {crew_block(crew)}
        </details>
    }
}

/// Station: the open stop card (science first), or how to pick one.
fn station_tab(c: CardModel, m: &PanelModel, w: Windows, station_min: f64, prio: RwSignal<std::collections::HashMap<String, u8>>,
               editing: RwSignal<Option<Vec<crate::plan::edits::EditTask>>>) -> impl IntoView {
    if !c.open {
        return view! {
            <p class="fine">{text::ops::PICK}</p>
            {stops_tab(m.stop_cards.clone())}
        }.into_any();
    }
    let s = m.stop_cards.get(c.index).cloned();
    let win = w.of(c.index).map(|(a, b)| format!("{}\u{2013}{}", w.hm(a), w.hm(b)));
    let at = if c.index == 0 { 0.0 } else { station_min };
    let i = c.index;
    let (m2, w2) = (m.clone(), w.clone());
    view! {
        {station_view(c, s.clone(), win, at)}
        {(i > 0).then(|| s.clone().map(|s| plan_editor(s, editing)))}
        {(i > 0).then(|| view! {
            <div class="ctl-row"><button type="button" title=text::cue::PRINT_TIP
                on:click=move |_| { let _h = cue_cards(&m2, &w2, prio, station_min, Some(i)); #[cfg(target_arch = "wasm32")] crate::ui::cuecard::print(&_h); }>
                {text::cue::PRINT_ONE}</button></div>
        })}
    }.into_any()
}

/// One plain-language line: ready or not, and what to watch first (fixed ranking, shown in the tooltip). On the Plan tab it
/// links to Risk.
fn safety(m: &PanelModel, c: &crate::plan::crew::CrewModel, tab: Option<RwSignal<usize>>, dark: Option<String>,
          placed: crate::scene::place::PlacedModel) -> impl IntoView {
    use text::safety as t;
    let total = m.samples.last().map(|q| q.d).unwrap_or(0.0);
    let hidden: f32 = m.samples.windows(2).filter(|x| !x[1].seen).map(|x| x[1].d - x[0].d).sum();
    let block = if !m.feasible { Some(if m.blocked_legs().is_empty() { t::NO_ROUTE.to_string() } else { fill(t::NO_ROUTE_LEGS, &[("legs", m.blocked_label())]) }) }
                else if c.available && !c.go { Some(t::NO_FIT.to_string()) }
                else if c.available && !c.rescue_pass { Some(format!("{} {}", t::NO_RESCUE, c.rescue)) }
                else { None };
    use crate::scene::select::{highlight, Highlight};
    // watch chips (fixed ranking, RULE_TIP): short words on the chip, the full sentence in its tooltip, a click draws the
    // problem on the map (scene/select.rs overlay)
    let mut chips: Vec<(String, String, Option<Highlight>)> = Vec::new();
    // (stop id, spot e, n, station number) for each steep target with a safer working spot
    let mut fix_spots: Option<Vec<(String, f32, f32, usize)>> = None;
    let under: Vec<_> = m.stop_cards.iter().filter(|s| s.map_understates()).collect();
    if !under.is_empty() {
        let names = under.iter().map(|s| s.label.clone()).collect::<Vec<_>>().join(", ");
        let nums = under.iter().map(|s| (s.index + 1).to_string()).collect::<Vec<_>>().join(", ");
        let b = under.iter().filter_map(|s| s.ground.as_ref().map(|g| g.2)).fold(f64::MIN, f64::max);
        chips.push((fill(t::CHIP_UNDER, &[("stops", nums)]), fill(t::UNDER, &[("names", names), ("b", format!("{b:.1}"))]),
                    Some(Highlight::Stops(under.iter().map(|s| s.index).collect()))));
        // its fix: where to stand and work beside each steep target (the target stays where it is)
        let spots: Vec<(String, f32, f32, usize)> = under.iter().filter_map(|s| s.work_spot.map(|w| (s.id.clone(), w.0, w.1, s.index + 1))).collect();
        if !spots.is_empty() {
            fix_spots = Some(spots);
        }
    }
    // the EVA or its reserve runs past sunset / before sunrise (plan/daylight.rs): twilight up to 2 h, then night
    if let Some(n) = dark {
        chips.push((n, text::sim::DARK_TIP.to_string(), None));
    }
    if hidden > 0.5 && total > 0.0 {
        let pct = format!("{}", (hidden / total * 100.0).round() as i64);
        chips.push((fill(t::CHIP_SIGHT, &[("pct", pct.clone())]), fill(t::SIGHT, &[("pct", pct), ("x", format!("{}", hidden.round() as i64))]),
                    Some(Highlight::Hidden)));
    }
    chips.push((t::CHIP_RIDGES.to_string(), t::RIDGES.to_string(), None));
    let ok = block.is_none();
    // the route rule lets corrected slopes exceed the limit only on ground Curiosity drove: the head says so in its tooltip
    let head_tip = if m.over_limit_cells > 0 && m.worst_deg > m.limit as f64 {
        fill(t::OK_OVER, &[("worst", format!("{:.1}", m.worst_deg)), ("limit", format!("{}", m.limit)), ("cells", m.over_limit_cells.to_string())])
    } else { t::OK.to_string() };
    let head = block.unwrap_or_else(|| t::OK_SHORT.to_string());
    let active = RwSignal::new(crate::scene::select::current_highlight());
    view! {
        <div class=if ok { "safety ok" } else { "safety no" } title=t::RULE_TIP>
            <p class="s-head" title=head_tip>{if ok { "\u{2713} " } else { "\u{2717} " }}{head}</p>
            // a blocked plan comes with its fix: move the stop the exit cannot be reached from to the nearest reachable ground
            {(!ok && !m.fixes.is_empty()).then(|| view! {
                <div class="watch-chips fixes">
                    {m.fixes.iter().map(|(i, label, e, n, d)| {
                        let (i, e, n) = (*i, *e, *n);
                        view! { <button type="button" class="watch-chip fix" title=text::drill::A_FIX_TIP
                            on:click=move |_| crate::plan::planner::send(crate::plan::planner::PanelEvent::Move(i, e, n))>
                            {fill(text::drill::A_FIX, &[("name", label.clone()), ("x", format!("{:.0}", d))])}</button> }
                    }).collect_view()}
                </div>
            })}
            <div class="watch-chips" role="group" aria-label=t::WATCH>
                <span class="watch-chips-k">{t::WATCH}</span>
                {chips.into_iter().map(|(label, tip, h)| match h {
                    Some(h) => {
                        let h2 = h.clone();
                        view! {
                            <button type="button" class="watch-chip" title=tip aria-pressed=move || if active.get().as_ref() == Some(&h2) { "true" } else { "false" }
                                on:click=move |_| { let next = if active.get_untracked().as_ref() == Some(&h) { None } else { Some(h.clone()) };
                                                    highlight(next.clone()); active.set(next); }>{label}</button>
                        }.into_any()
                    }
                    None => view! { <span class="watch-chip static" title=tip>{label}</span> }.into_any(),
                }).collect_view()}
                {fix_spots.map(|sp| {
                    use crate::scene::place::{send, PlaceAct};
                    let nums = sp.iter().map(|x| x.3.to_string()).collect::<Vec<_>>().join(", ");
                    let all_placed = sp.iter().all(|x| placed.work.contains(&x.0));
                    if all_placed {
                        let first = sp[0].0.clone();
                        view! { <button type="button" class="watch-chip fix done" title=text::place::WORK_PLACED_TIP
                            on:click=move |_| send(PlaceAct::SelectWork(first.clone()))>
                            {fill(text::place::WORK_PLACED, &[("stops", nums)])}</button> }.into_any()
                    } else {
                        let all: Vec<(String, f32, f32)> = sp.iter().map(|x| (x.0.clone(), x.1, x.2)).collect();
                        view! { <button type="button" class="watch-chip fix" title=text::drill::A_WORK_TIP
                            on:click=move |_| { active.set(None); send(PlaceAct::WorkAll(all.clone())); }>
                            {fill(text::drill::A_WORK, &[("stops", nums)])}</button> }.into_any()
                    }
                })}
                {match placed.relay {
                    Some((b, a, sees)) => {
                        let pct = |x: f32| format!("{}", (x * 100.0).round() as i64);
                        let label = if sees { fill(text::place::RELAY_PLACED, &[("a", pct(b)), ("b", pct(a))]) } else { text::place::RELAY_BLIND.to_string() };
                        Some(view! { <button type="button" class=if sees { "watch-chip fix done" } else { "watch-chip fix warn" } title=text::place::RELAY_PLACED_TIP
                            on:click=|_| crate::scene::place::send(crate::scene::place::PlaceAct::SelectRelay)>{label}</button> }.into_any())
                    }
                    None => (hidden > 0.5 && total > 0.0).then(|| view! {
                        <button type="button" class="watch-chip fix" title=text::drill::A_RELAY_TIP
                            on:click=|_| crate::scene::select::ask_relay()>{text::drill::A_RELAY}</button> }.into_any()),
                }}
                {tab.map(|tab| view! { <button type="button" class="linkish" on:click=move |_| tab.set(3)>{t::MORE}</button> })}
            </div>
        </div>
    }
}

/// Conditions for this EVA: REMS weather on the selected sol and the RAD dose over the EVA length (records, not a forecast).
fn conditions(l: &LightModel, hours: f64) -> impl IntoView {
    use text::cond as t;
    let mut chips: Vec<(String, String, String)> = l.wx_rows.iter().filter(|r| r.0 != text::light::RAD_LABEL)
        .map(|(k, v)| (k.clone(), v.clone(), String::new())).collect();
    if let Some((rate, n)) = l.rad_rate {
        chips.push((t::DOSE.to_string(), fill(t::DOSE_V, &[("x", format!("{}", (rate * hours).round() as i64)), ("h", format!("{hours}"))]),
                    fill(t::DOSE_TIP, &[("n", n.to_string()), ("rate", format!("{rate:.1}"))])));
    }
    let none = l.wx_rows.is_empty().then(|| fill(t::NONE, &[("sol", format!("{}", l.sol.round() as i64))]));
    view! {
        <details class="cond" title=t::TIP>
            <summary class="cond-h" title=l.date.clone()>{fill(t::TITLE_SOL, &[("sol", format!("{}", l.sol.round() as i64))])}</summary>
            <div class="cond-chips">
                {chips.into_iter().map(|(k, v, tip)| view! { <span class="cchip" title=tip><span class="k">{k}</span>" "<b>{v}</b></span> }).collect_view()}
            </div>
            {none.map(|x| view! { <p class="fine">{x}</p> })}
        </details>
    }
}

/// Which kinds of NASA data VISOR uses, from which instrument, where, and at what scale (About).
pub(super) fn data_kinds() -> impl IntoView {
    use text::cond as t;
    let rows = [(t::K_IMAGE, t::K_IMAGE_SRC, t::K_IMAGE_WHERE, t::K_IMAGE_SCALE), (t::K_TOPO, t::K_TOPO_SRC, t::K_TOPO_WHERE, t::K_TOPO_SCALE),
                (t::K_ROVER, t::K_ROVER_SRC, t::K_ROVER_WHERE, t::K_ROVER_SCALE), (t::K_CHEM, t::K_CHEM_SRC, t::K_CHEM_WHERE, t::K_CHEM_SCALE),
                (t::K_ATMO, t::K_ATMO_SRC, t::K_ATMO_WHERE, t::K_ATMO_SCALE), (t::K_THERMAL, t::K_THERMAL_SRC, t::K_THERMAL_WHERE, t::K_THERMAL_SCALE),
                (t::K_RAD, t::K_RAD_SRC, t::K_RAD_WHERE, t::K_RAD_SCALE), (t::K_SPEC, t::K_SPEC_SRC, t::K_SPEC_WHERE, t::K_SPEC_SCALE),
                (t::K_SUN, t::K_SUN_SRC, t::K_SUN_WHERE, t::K_SUN_SCALE)];
    view! {
        <h4>{t::KINDS}</h4>
        <table class="kinds">
            <thead><tr><th scope="col">{t::K_HEAD_KIND}</th><th scope="col">{t::K_HEAD_SRC}</th><th scope="col">{t::K_HEAD_WHERE}</th><th scope="col">{t::K_HEAD_SCALE}</th></tr></thead>
            <tbody>{rows.into_iter().map(|(a, b, c, d)| view! { <tr><th scope="row">{a}</th><td>{b}</td><td>{c}</td><td class="fine">{d}</td></tr> }).collect_view()}</tbody>
        </table>
        <p class="fine">{t::K_NOTE}</p>
    }
}

/// Science: chemistry near each station as a dot matrix (dot area and shade = wt%, the value printed), and the drill pair
/// (ridge top vs hollow) as paired bars with the difference. A station column opens / flies to the station.
fn science_tab(m: &PanelModel) -> impl IntoView {
    use crate::ui::science::{matrix, row_max, OXIDES};
    use text::science as t;
    let mx = matrix(&m.stop_cards);
    if mx.is_empty() {
        return view! { <p class="fine">{t::NONE}</p> }.into_any();
    }
    let rmax = row_max(&mx);
    let (lab, colw, rowh) = (46.0f32, 44.0f32, 42.0f32);
    let (w, h) = (lab + colw * mx.len() as f32 + 4.0, 26.0 + rowh * OXIDES.len() as f32 + 18.0);
    let cols = mx.clone();
    let pair: Vec<crate::ui::science::StationChem> = mx.iter().filter(|s| s.pair).cloned().collect();
    view! {
        <h4 title=t::TIP>{t::TITLE}</h4>
        <svg class="sci" viewBox=format!("0 0 {w} {h}") width=w.to_string() height=h.to_string() role="img" aria-label=t::TITLE>
            {OXIDES.iter().enumerate().map(|(k, ox)| view! {
                <text x="2" y=(26.0 + rowh * k as f32 + rowh / 2.0 + 4.0).to_string() class="sci-ox">{*ox}</text>
            }).collect_view()}
            {cols.into_iter().enumerate().map(|(c, s)| {
                let x = lab + colw * c as f32 + colw / 2.0;
                let i = s.index;
                view! {
                    <g class=if s.pair { "sci-col pair" } else { "sci-col" } on:click=move |_| crate::ui::card::send(crate::ui::card::CardEvent::Open(i))>
                        <title>{fill(t::COL_TIP, &[("name", s.label.clone()), ("n", s.n.to_string())])}</title>
                        <rect x=(x - colw / 2.0 + 2.0).to_string() y="2" width=(colw - 4.0).to_string() height=(h - 4.0).to_string() rx="4" class="sci-bg"/>
                        <circle cx=x.to_string() cy="13" r="9" class="sci-pin"/>
                        <text x=x.to_string() y="17" text-anchor="middle" class="sci-pin-n">{(i + 1).to_string()}</text>
                        {s.vals.iter().enumerate().map(|(k, v)| {
                            let cy = 26.0 + rowh * k as f32 + rowh / 2.0 - 3.0;
                            match v {
                                Some(v) => {
                                    let f = (*v / rmax[k]).clamp(0.0, 1.0) as f32;
                                    view! {
                                        <circle cx=x.to_string() cy=cy.to_string() r=(3.0 + 9.0 * f.sqrt()).to_string() class="sci-dot"
                                            fill-opacity=(0.35 + 0.65 * f).to_string()/>
                                        <text x=x.to_string() y=(cy + 23.0).to_string() text-anchor="middle" class="sci-v">{format!("{v:.1}")}</text>
                                    }.into_any()
                                }
                                None => view! { <text x=x.to_string() y=(cy + 4.0).to_string() text-anchor="middle" class="sci-v">"\u{2014}"</text> }.into_any(),
                            }
                        }).collect_view()}
                        <text x=x.to_string() y=(h - 4.0).to_string() text-anchor="middle" class="sci-n">{fill(t::N, &[("n", s.n.to_string())])}</text>
                    </g>
                }
            }).collect_view()}
        </svg>
        <p class="fine">{t::NOTE}</p>
        {(pair.len() == 2).then(|| {
            let (a, b) = (pair[0].clone(), pair[1].clone());
            let bw = 120.0f32;
            view! {
                <h4 title=t::PAIR_TIP>{fill(t::PAIR, &[("a", a.label.clone()), ("b", b.label.clone())])}</h4>
                <div class="sci-pair">
                    {OXIDES.iter().enumerate().map(|(k, ox)| {
                        let (va, vb) = (a.vals[k], b.vals[k]);
                        let mxv = va.unwrap_or(0.0).max(vb.unwrap_or(0.0)).max(1e-6);
                        let bar = |v: Option<f64>, cls: &'static str| view! {
                            <span class=format!("sp-bar {cls}") style:width=format!("{:.0}px", v.map(|v| (v / mxv) as f32 * bw).unwrap_or(0.0))></span> };
                        let d = match (va, vb) { (Some(x), Some(y)) => format!("{:+.1}", y - x), _ => "\u{2014}".into() };
                        view! {
                            <div class="sp-row">
                                <span class="sp-ox">{*ox}</span>
                                <span class="sp-bars">{bar(va, "a")}<b>{va.map(|v| format!("{v:.1}")).unwrap_or("\u{2014}".into())}</b>
                                    <br/>{bar(vb, "b")}<b>{vb.map(|v| format!("{v:.1}")).unwrap_or("\u{2014}".into())}</b></span>
                                <span class="sp-d" title=t::DIFF_TIP>{d}</span>
                            </div>
                        }
                    }).collect_view()}
                    <p class="fine"><span class="sp-key a"></span>{a.label.clone()}" "<span class="sp-key b"></span>{b.label.clone()}</p>
                </div>
            }
        })}
    }.into_any()
}

/// Plain meanings of the words the panel uses.
fn glossary(vals: &[(&'static str, String)]) -> impl IntoView {
    use text::safety as t;
    let items = [(t::G_EVA, t::G_EVA_V), (t::G_SOL, t::G_SOL_V), (t::G_EGRESS, t::G_EGRESS_V), (t::G_STATION, t::G_STATION_V),
                 (t::G_PRIO, t::G_PRIO_V), (t::G_PAIR, t::G_PAIR_V), (t::G_ROVER, t::G_ROVER_V), (t::G_CORRECTED, t::G_CORRECTED_V),
                 (t::G_VERIFIED, t::G_VERIFIED_V), (t::G_BACK, t::G_BACK_V), (t::G_RUNS, t::G_RUNS_V), (t::G_LTST, t::G_LTST_V)];
    view! {
        <details class="more glossary"><summary>{t::GLOSSARY}</summary>
            <dl>{items.into_iter().map(|(k, v)| view! { <dt>{k}</dt><dd>{fill(v, vals)}</dd> }).collect_view()}</dl>
        </details>
    }
}

/// The current plan's numbers for the comparison.
fn metrics_now(m: &PanelModel, w: &Windows, go: bool) -> crate::ui::plans::Metrics {
    crate::ui::plans::Metrics {
        walk_min: m.walk_s / 60.0, length_m: m.length_m, steepest_deg: m.worst_deg, back_min: m.back_s / 60.0,
        unseen_m: m.samples.windows(2).filter(|x| !x[1].seen).map(|x| (x[1].d - x[0].d) as f64).sum(),
        verified_pct: m.verified_pct as f64, stations: m.stop_cards.len().saturating_sub(1),
        bags: m.stop_cards.iter().map(|s| s.bags.len()).sum(), end: w.hm(w.end), go,
    }
}

/// The planner's own station plans, keyed as a reopened share link names its stops (custom stops renumber 1, 2, ... in
/// walking order when a link is decoded).
fn edits_for_link(m: &PanelModel) -> serde_json::Value {
    let mut k = 0;
    let mut out = std::collections::BTreeMap::new();
    for s in &m.stop_cards {
        let custom = s.id.starts_with("custom-");
        if custom {
            k += 1;
        }
        if s.edited {
            out.insert(if custom { format!("custom-{k}") } else { s.id.clone() }, s.raw.clone());
        }
    }
    crate::plan::edits::to_json(&out)
}

/// New plan / Done adding / Save plan, and the saved plans compared with the plan on screen.
fn plans_bar(m: &PanelModel, w: &Windows, go: bool, saved: RwSignal<Vec<crate::ui::plans::SavedPlan>>, compare: bool) -> impl IntoView {
    use crate::ui::plans::{rows, SavedPlan};
    use text::plans as t;
    let now = metrics_now(m, w, go);
    let picking = m.controls.picking;
    let (now2, edits) = (now.clone(), edits_for_link(m));
    let list = saved.get();
    let cols: Vec<&crate::ui::plans::Metrics> = std::iter::once(&now).chain(list.iter().rev().take(2).map(|p| &p.m)).collect();
    let r = rows(&cols);
    let heads: Vec<(Option<usize>, String)> = std::iter::once((None, t::CURRENT.to_string()))
        .chain(list.iter().enumerate().rev().take(2).map(|(i, p)| (Some(i), p.name.clone()))).collect();
    if compare {
        return view! {
            <h4>{fill(t::SAVED, &[("n", list.len().to_string())])}</h4>
            {if list.is_empty() {
                view! { <p class="fine">{t::NONE}</p> }.into_any()
            } else {
                view! {
                    <table class="cmp">
                        <thead><tr><th></th>{heads.into_iter().map(|(i, name)| view! {
                            <th scope="col"><span class="cmp-name">{name}</span>{i.map(|i| view! {
                                <span class="cmp-act">
                                    <button type="button" class="mini" on:click=move |_| {
                                        if let Some(p) = saved.with_untracked(|l| l.get(i).cloned()) {
                                            crate::ui::card::send(crate::ui::card::CardEvent::Close);
                                            crate::plan::edits::send(crate::plan::edits::EditEvent::ReplaceAll(crate::plan::edits::from_json(&p.edits)));
                                            send(PanelEvent::Apply(p.query));
                                        }
                                    }>{t::OPEN}</button>
                                    <button type="button" class="mini ghost" title=t::DELETE on:click=move |_| saved.update(|l| { if i < l.len() { l.remove(i); } })>"\u{d7}"</button>
                                </span>
                            })}</th>
                        }).collect_view()}</tr></thead>
                        <tbody>{r.into_iter().map(|(k, vals, best)| view! {
                            <tr><th scope="row">{k}</th>{vals.into_iter().enumerate().map(|(j, v)| view! {
                                <td class=if best == Some(j) { "num best" } else { "num" } title=if best == Some(j) { t::BEST_TIP } else { "" }>{v}</td>
                            }).collect_view()}</tr>
                        }).collect_view()}</tbody>
                    </table>
                }.into_any()
            }}
        }.into_any();
    }
    view! {
        <div class="ctl-row plans">
            {if picking {
                view! { <button type="button" class="primary" on:click=|_| send(PanelEvent::CancelPick)>{t::DONE}</button> }.into_any()
            } else {
                view! { <button type="button" title=t::NEW_TIP on:click=|_| { send(PanelEvent::NewPlan); }>{t::NEW}</button> }.into_any()
            }}
            {(!picking).then(|| view! { <button type="button" title=t::ADD_TIP on:click=|_| send(PanelEvent::StartPick)>{t::ADD}</button> })}
            <button type="button" title=t::SAVE_TIP on:click=move |_| {
                let q = web_sys::window().and_then(|w| w.location().search().ok()).unwrap_or_default();
                let name = fill(t::NAME, &[("k", (saved.with_untracked(|p| p.len()) + 1).to_string())]);
                saved.update(|p| p.push(SavedPlan { name, query: q.clone(), edits: edits.clone(), m: now2.clone() }));
            }>{t::SAVE}</button>
        </div>
        {(!m.controls.note.is_empty() && picking).then(|| view! { <p class="fine">{m.controls.note.clone()}</p> })}
    }.into_any()
}

/// Station-plan editor: write or change a station's plan (question / observe / image / sample / compare, bags), restore
/// VISOR's sourced suggestion, or start a new station from the nearest Curiosity stop's plan.
fn plan_editor(s: StopSummary, editing: RwSignal<Option<Vec<crate::plan::edits::EditTask>>>) -> impl IntoView {
    use crate::plan::edits::{send as esend, EditEvent, EditTask, STEPS};
    use text::edit as t;
    let id = s.id.clone();
    let has_suggestion = !s.id.starts_with("custom-");
    let raw = s.raw.clone();
    let near = s.near.clone();
    move || match editing.get() {
        None => {
            let raw = raw.clone();
            let near = near.clone();
            view! {
                {s.edited.then(|| view! { <p class="fine"><span class="badge" title=t::YOURS_TIP>{t::YOURS}</span></p> })}
                {(raw.is_empty()).then(|| view! { <p class="fine">{t::EMPTY}</p> })}
                <div class="ctl-row">
                    <button type="button" title=t::EDIT_TIP on:click={ let raw = raw.clone(); move |_| {
                        let mut r = raw.clone();
                        if r.is_empty() { r.push(EditTask { step: "question".into(), ..Default::default() }); }
                        editing.set(Some(r));
                    }}>{if raw.is_empty() { t::WRITE } else { t::EDIT }}</button>
                    {(raw.is_empty()).then(|| near.clone().map(|(name, d, tasks)| view! {
                        <button type="button" title=t::START_TIP on:click=move |_| editing.set(Some(tasks.clone()))>
                            {fill(t::START_FROM, &[("name", name), ("x", format!("{}", d.round() as i64))])}</button>
                    }))}
                </div>
            }.into_any()
        }
        Some(rows) => {
            let (id_s, id_r) = (id.clone(), id.clone());
            view! {
                <div class="editor">
                    {rows.into_iter().enumerate().map(|(k, r)| {
                        let step = r.step.clone();
                        view! {
                            <div class="ed-row">
                                <select on:change=move |ev| { let v = event_target_value(&ev); editing.update(|e| if let Some(e) = e { e[k].step = v }); }>
                                    {STEPS.iter().map(|x| view! { <option value=*x selected=step == *x>{crate::plan::simrun::step_label(x)}</option> }).collect_view()}
                                </select>
                                <textarea rows="2" placeholder=t::TEXT prop:value=r.text.clone()
                                    on:change=move |ev| { let v = event_target_value(&ev); editing.update(|e| if let Some(e) = e { e[k].text = v }); }></textarea>
                                {(r.step == "sample").then(move || view! {
                                    <input type="text" placeholder=t::BAGS prop:value=r.bags.join(", ")
                                        on:change=move |ev| { let v = event_target_value(&ev); editing.update(|e| if let Some(e) = e {
                                            e[k].bags = v.split(',').map(|b| b.trim().to_string()).filter(|b| !b.is_empty()).collect() }); }/>
                                })}
                                <button type="button" class="mini ghost" title=t::REMOVE on:click=move |_| editing.update(|e| if let Some(e) = e { if k < e.len() { e.remove(k); } })>"\u{d7}"</button>
                            </div>
                        }
                    }).collect_view()}
                    <div class="ctl-row">
                        <button type="button" on:click=move |_| editing.update(|e| if let Some(e) = e { e.push(EditTask { step: "observe".into(), ..Default::default() }) })>{t::ADD}</button>
                        <button type="button" class="primary" on:click=move |_| {
                            if let Some(e) = editing.get_untracked() { esend(EditEvent::Set(id_s.clone(), e)); }
                            editing.set(None);
                        }>{t::SAVE}</button>
                        <button type="button" class="ghost" on:click=move |_| editing.set(None)>{t::CANCEL}</button>
                        {has_suggestion.then(move || view! {
                            <button type="button" class="ghost" on:click=move |_| { esend(EditEvent::Clear(id_r.clone())); editing.set(None); }>{t::RESET}</button>
                        })}
                    </div>
                </div>
            }.into_any()
        }
    }
}

const PLANS_KEY: &str = "visor.plans.v1";

fn load_plans() -> Vec<crate::ui::plans::SavedPlan> {
    web_sys::window().and_then(|w| w.local_storage().ok().flatten()).and_then(|st| st.get_item(PLANS_KEY).ok().flatten())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()).map(|v| crate::ui::plans::from_json(&v)).unwrap_or_default()
}

fn save_plans(p: &[crate::ui::plans::SavedPlan]) {
    if let Some(st) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = st.set_item(PLANS_KEY, &crate::ui::plans::to_json(p).to_string());
    }
}

/// Log (execute mode): per station actual times (typed or "Now" from the EVA clock), task and bag check-offs, a note; how far
/// behind and what the run-late rule would drop; planned vs actual; CSV.
fn log_tab(m: &PanelModel, w: Windows, exec: RwSignal<crate::ui::execlog::ExecLog>, sim: RwSignal<crate::plan::simrun::SimModel>,
           prio: RwSignal<std::collections::HashMap<String, u8>>, station_min: f64) -> impl IntoView {
    use crate::ui::execlog::{behind_min, deltas, to_csv, Planned};
    use text::exec as x;
    let plan: Vec<Planned> = m.stop_cards.iter().filter(|s| s.index > 0)
        .filter_map(|s| w.of(s.index).map(|(a, b)| Planned { id: key_of(s), name: s.label.clone(), arrive: w.minutes(a), leave: w.minutes(b) })).collect();
    let behind = exec.with(|l| behind_min(&plan, l));
    let head = match behind {
        None => x::ON_PLAN.to_string(),
        Some(b) if b <= 0.5 => fill(x::AHEAD, &[("x", format!("{}", (-b).round() as i64))]),
        Some(b) => {
            let (_, gave, names) = drops(m, prio, station_min, b);
            if names.is_empty() || gave < b { fill(x::BEHIND, &[("x", format!("{}", b.round() as i64))]) }
            else { fill(x::BEHIND_DROP, &[("x", format!("{}", b.round() as i64)), ("stops", names.join(", "))]) }
        }
    };
    let dl = exec.with(|l| deltas(&plan, l));
    let totals: std::collections::BTreeMap<String, (usize, usize)> = m.stop_cards.iter()
        .map(|s| (key_of(s), (s.steps.iter().filter(|t| t.0 != text::sim::STEP_QUESTION).count(), s.bags.len()))).collect();
    let (plan2, totals2) = (plan.clone(), totals.clone());
    let now = move || sim.with_untracked(|s| s.clock.chars().take(5).collect::<String>());
    let mut k = 0usize;
    view! {
        <p class="fine">{x::INTRO}</p>
        <p class=if behind.is_some_and(|b| b > 0.5) { "go-line nogo" } else { "go-line go" }>{head}</p>
        {m.stop_cards.iter().filter(|s| s.index > 0).map(|s| {
            let id = key_of(s);
            let planned = w.of(s.index).map(|(a, b)| fill(x::PLANNED, &[("a", w.hm(a)), ("b", w.hm(b))])).unwrap_or_default();
            let field = move |id: String, which: u8| {
                let id2 = id.clone();
                view! {
                    <input type="text" class="t" inputmode="numeric" placeholder="hh:mm" size="5"
                        prop:value=move || exec.with(|l| l.0.get(&id).map(|e| if which == 0 { e.arrive.clone() } else { e.leave.clone() }).unwrap_or_default())
                        on:change=move |ev| { let v = event_target_value(&ev); exec.update(|l| { let e = l.0.entry(id2.clone()).or_default(); if which == 0 { e.arrive = v } else { e.leave = v } }); }/>
                }
            };
            let stamp = move |id: String, which: u8| view! {
                <button type="button" class="mini" title=x::CLOCK_TIP on:click=move |_| { let t = now(); exec.update(|l| { let e = l.0.entry(id.clone()).or_default(); if which == 0 { e.arrive = t } else { e.leave = t } }); }>{x::NOW}</button>
            };
            let tasks: Vec<(usize, &'static str, String)> = s.steps.iter().enumerate().filter(|t| t.1 .0 != text::sim::STEP_QUESTION).map(|(i, t)| (i, t.0, t.1.clone())).collect();
            let bags: Vec<(usize, String, String)> = s.bags.iter().enumerate().map(|(j, b)| { k += 1; (j, fill(x::BAG, &[("k", k.to_string())]), b.clone()) }).collect();
            let id_t = id.clone();
            let id_b = id.clone();
            let id_n = id.clone();
            let id_n2 = id.clone();
            view! {
                <section class="log-st">
                    <header><b>{s.label.clone()}</b><span class="fine">{planned}</span></header>
                    <div class="log-times">
                        <label>{x::ARRIVE}" "{field(id.clone(), 0)}{stamp(id.clone(), 0)}</label>
                        <label>{x::LEAVE}" "{field(id.clone(), 1)}{stamp(id.clone(), 1)}</label>
                    </div>
                    <ul class="checks">
                        {tasks.into_iter().map(|(i, st, tx)| { let id = id_t.clone(); let id2 = id.clone(); view! {
                            <li><label><input type="checkbox" prop:checked=move || exec.with(|l| l.0.get(&id).and_then(|e| e.tasks.get(&i).copied()).unwrap_or(false))
                                on:change=move |ev| { let on = event_target_checked(&ev); exec.update(|l| { l.0.entry(id2.clone()).or_default().tasks.insert(i, on); }); }/>
                                <span class="st">{st}</span>" "{tx}</label></li>
                        }}).collect_view()}
                        {bags.into_iter().map(|(j, b, lab)| { let id = id_b.clone(); let id2 = id.clone(); view! {
                            <li><label><input type="checkbox" prop:checked=move || exec.with(|l| l.0.get(&id).and_then(|e| e.bags.get(&j).copied()).unwrap_or(false))
                                on:change=move |ev| { let on = event_target_checked(&ev); exec.update(|l| { l.0.entry(id2.clone()).or_default().bags.insert(j, on); }); }/>
                                <b class="bagno">{b}</b>" "{lab}</label></li>
                        }}).collect_view()}
                    </ul>
                    <input type="text" class="note" placeholder=x::NOTE
                        prop:value=move || exec.with(|l| l.0.get(&id_n).map(|e| e.note.clone()).unwrap_or_default())
                        on:change=move |ev| { let v = event_target_value(&ev); exec.update(|l| { l.0.entry(id_n2.clone()).or_default().note = v; }); }/>
                </section>
            }
        }).collect_view()}
        <h4>{x::SUMMARY}</h4>
        <table class="legs">
            <thead><tr><th scope="col">{text::ops::COL_STATION}</th><th scope="col">{x::COL_ARR}</th><th scope="col">{x::COL_STAY}</th></tr></thead>
            <tbody>{dl.into_iter().map(|(n, a, d)| {
                let f = |v: Option<f64>| v.map(|v| fill(x::DELTA, &[("x", format!("{:+}", v.round() as i64))])).unwrap_or_else(|| "\u{2014}".into());
                view! { <tr><td>{n}</td><td class="num">{f(a)}</td><td class="num">{f(d)}</td></tr> }
            }).collect_view()}</tbody>
        </table>
        <div class="ctl-row share">
            <button type="button" on:click=move |_| {
                let _csv = exec.with_untracked(|l| to_csv(&plan2, l, &totals2));
                #[cfg(target_arch = "wasm32")] crate::plan::share::web::download("visor_eva_log.csv", "text/csv", &_csv);
            }>{x::CSV}</button>
            <button type="button" class="ghost" title=x::CLEAR_TIP on:click=move |_| exec.set(Default::default())>{x::CLEAR}</button>
        </div>
    }
}

fn key_of(s: &StopSummary) -> String {
    if s.id.is_empty() { s.label.clone() } else { s.id.clone() }
}

const LOG_KEY: &str = "visor.eva_log.v1";

/// Load the EVA log from this browser (empty if storage is blocked or the entry is bad).
pub(super) fn load_log() -> crate::ui::execlog::ExecLog {
    web_sys::window().and_then(|w| w.local_storage().ok().flatten()).and_then(|st| st.get_item(LOG_KEY).ok().flatten())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()).map(|v| crate::ui::execlog::from_json(&v)).unwrap_or_default()
}

fn save_log(l: &crate::ui::execlog::ExecLog) {
    if let Some(st) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = st.set_item(LOG_KEY, &crate::ui::execlog::to_json(l).to_string());
    }
}

/// Risk: only what needs attention, then "n checks OK", readiness and legs folded.
fn risk_tab(m: &PanelModel, b: BriefModel, c: crate::plan::crew::CrewModel) -> impl IntoView {
    use crate::ui::hud::minutes;
    use text::ops as o;
    let item = |s: &crate::ui::model::StopSummary| s.ground.as_ref().map(|(_, a, r)|
        fill(o::RISK_GROUND_ITEM, &[("name", s.label.clone()), ("a", format!("{a:.1}")), ("b", format!("{r:.1}"))]));
    let ground: Vec<String> = m.stop_cards.iter().filter(|s| s.map_overstates()).filter_map(item).collect();
    let under: Vec<String> = m.stop_cards.iter().filter(|s| s.map_understates()).filter_map(item).collect();

    let (watch, ok): (Vec<_>, Vec<_>) = b.findings.iter().cloned().partition(|f| f.level == "watch");
    let n_ok = ok.len() + usize::from(c.rescue_pass);
    let runs = runs_of(m);
    let unsettled = m.range_unsettled.then(|| (o::UNSETTLED, fill(o::UNSETTLED_TIP, &[("runs", runs.clone())])));
    let walk = m.time_range.map(|(lo, hi)| fill(o::RISK_WALK, &[("lo", minutes(lo as f32)), ("hi", minutes(hi as f32)), ("runs", runs.clone())]));
    let ready = b.readiness.clone();
    // line of sight to the exit along the route (panel::sight_line on each profile sample)
    let total = m.samples.last().map(|q| q.d).unwrap_or(0.0);
    let hidden: f32 = m.samples.windows(2).filter(|w| !w[1].seen).map(|w| w[1].d - w[0].d).sum();
    let sight = (hidden > 0.5 && total > 0.0).then(|| fill(o::RISK_SIGHT_V, &[("x", format!("{}", hidden.round() as i64)),
        ("total", format!("{}", total.round() as i64)), ("pct", format!("{}", (hidden / total * 100.0).round() as i64))]));
    view! {
        <div class="brief risk">
            {(!under.is_empty()).then(|| view! {
                <section class="finding lvl-watch">
                    <header><span class="dot" role="img" aria-label=text::brief::LVL_WATCH></span><h4 title=o::RISK_UNDER_TIP>{o::RISK_UNDER}</h4></header>
                    <ul class="plain">{under.into_iter().map(|g| view! { <li>{g}</li> }).collect_view()}</ul>
                    <p class="fine">{o::RISK_UNDER_TIP}</p>
                </section>
            })}
            {(!ground.is_empty()).then(|| view! {
                <section class="finding lvl-watch">
                    <header><span class="dot" role="img" aria-label=text::brief::LVL_WATCH></span><h4 title=o::RISK_GROUND_TIP>{o::RISK_GROUND}</h4></header>
                    <ul class="plain">{ground.into_iter().map(|g| view! { <li>{g}</li> }).collect_view()}</ul>
                </section>
            })}
            {sight.map(|x| view! {
                <section class="finding lvl-watch" title=o::RISK_SIGHT_TIP>
                    <header><span class="dot" role="img" aria-label=text::brief::LVL_WATCH></span><h4>{o::RISK_SIGHT}</h4></header>
                    <p class="verdict">{x}</p>
                </section>
            })}
            {(!c.rescue_pass && c.available).then(|| view! {
                <section class="finding lvl-watch"><header><span class="dot" role="img"></span><h4>{o::RISK_RESCUE}</h4></header>
                    <p class="verdict">{c.rescue.clone()}</p></section>
            })}
            {watch.into_iter().map(|f| view! {
                <section class="finding lvl-watch" title=format!("{} {}", f.why, f.changes_if)>
                    <header><span class="dot" role="img" aria-label=text::brief::LVL_WATCH></span><h4>{f.title}</h4>
                        {(!f.figure.is_empty()).then(|| view! { <span class="fig num">{f.figure.clone()}</span> })}</header>
                    <p class="verdict">{f.verdict.clone()}</p>
                </section>
            }).collect_view()}
            {walk.map(|x| view! { <p class="fine">{x}{unsettled.map(|(w, tip)| view! { <span title=tip>{w}</span> })}</p> })}
            <details class="more"><summary>{fill(o::RISK_OK, &[("n", n_ok.to_string())])}</summary>
                <ul class="oks">
                    {ok.into_iter().map(|f| view! {
                        <li title=format!("{} {}", f.why, f.changes_if)><b>{f.title}</b>{(!f.figure.is_empty()).then(|| view! { <span class="num">{format!(" {}", f.figure)}</span> })}
                            <span class="fine">{format!(" \u{b7} {}", f.verdict)}</span></li>
                    }).collect_view()}
                    {c.rescue_pass.then(|| view! { <li><b>{o::RISK_RESCUE}</b><span class="fine">{format!(" \u{b7} {}", c.rescue)}</span></li> })}
                </ul>
            </details>
            <details class="more"><summary>{o::RISK_READY}</summary>
                <div class="rrows">
                    {ready.into_iter().map(|r| view! {
                        <div class=format!("rrow-c r-{}", r.level) title=r.verdict.clone()>
                            <span class=format!("pill r-{}", r.level)>{ready_word(r.level)}</span>" "<span>{r.name()}</span>
                        </div>
                    }).collect_view()}
                </div>
            </details>
            <details class="more"><summary>{o::RISK_LEGS}</summary>{legs_table(m, b.weak_legs.clone())}</details>
        </div>
    }
}

/// The legs table (minutes, steepest corrected slope, share of runs on the same path); hover / click find it on the map.
fn legs_table(m: &PanelModel, weak: Vec<usize>) -> impl IntoView {
    let runs = runs_of(m);
    let n_legs = m.legs.len();
    let legs: Vec<_> = m.legs.iter().enumerate().map(|(i, r)| (i, r.clone(), m.leg_agree.get(i).copied().flatten(), leg_mid(m, i))).collect();
    view! {
        <table class="legs">
            <thead><tr>
                <th scope="col">{u::COL_LEG}</th><th scope="col">{u::COL_MIN}</th><th scope="col">{u::COL_SLOPE}</th>
                <th scope="col" title=fill(u::SAME_TIP, &[("runs", runs.clone())])>{u::COL_SAME}</th>
            </tr></thead>
            <tbody>
                {legs.into_iter().map(|(i, r, a, at)| view! {
                    <tr class=if weak.contains(&i) { "pick weak" } else { "pick" } title=r.label.clone()
                        on:mouseenter=pin_enter(at) on:mouseleave=pin_leave() on:click=pin_click(at)>
                        <td>{format!("{}\u{2192}{}", i + 1, if i + 1 == n_legs { 1 } else { i + 2 })}</td>
                        <td class="num">{r.minutes.clone()}</td><td class="num">{r.max_slope.clone()}</td>
                        <td class="num">{match a {
                            Some(x) => view! { <span class="agree"><i style:width=format!("{}%", (x * 100.0).round())></i>{format!("{}%", (x * 100.0).round() as i64)}</span> }.into_any(),
                            None => view! { "\u{2014}" }.into_any(),
                        }}</td>
                    </tr>
                }).collect_view()}
            </tbody>
        </table>
    }
}

/// Midpoint (east, north) of leg i on the loop profile, for "find on the map".
fn leg_mid(m: &PanelModel, i: usize) -> Option<(f32, f32)> {
    let a = *m.leg_starts.get(i)?;
    let b = m.leg_starts.get(i + 1).copied().unwrap_or(m.samples.len());
    m.samples.get((a + b) / 2).map(|q| (q.e, q.n))
}

/// Hover pins the point on the map, leaving clears it, click centres the map on it (the EVA timeline's convention).
fn pin_enter(at: Option<(f32, f32)>) -> impl Fn(web_sys::MouseEvent) + Clone + 'static {
    move |_| { if let (Some(p), Ok(mut g)) = (at, crate::ui::panel::HOVER.lock()) { *g = Some(p); } }
}
fn pin_leave() -> impl Fn(web_sys::MouseEvent) + Clone + 'static {
    move |_| { if let Ok(mut g) = crate::ui::panel::HOVER.lock() { *g = None; } }
}
fn pin_click(at: Option<(f32, f32)>) -> impl Fn(web_sys::MouseEvent) + Clone + 'static {
    move |_| { if let (Some(p), Ok(mut g)) = (at, crate::ui::panel::GOTO.lock()) { *g = Some(p); } }
}

/// Crew and margin (plan/crew.rs): EVA length, optional planned minutes per stop, station time, rescue check, roles.
pub(super) fn crew_block(crew: RwSignal<crate::plan::crew::CrewModel>) -> impl IntoView {
    use crate::plan::crew::{send, CrewEvent};
    use text::crew as t;
    move || {
        let c = crew.get();
        if !c.available {
            return ().into_any();
        }
        let planned = RwSignal::new(c.planned_min.map(|x| format!("{x}")).unwrap_or_default());
        view! {
            <div class="controls crew">
                <h4>{t::TITLE}</h4>
                <div class="ctl" role="group" aria-label=t::HOURS_LABEL title=t::HOURS_TIP>
                    <span>{t::HOURS_LABEL}</span>
                    <div class="seg">
                        {c.options.iter().map(|&h| view! {
                            <button type="button" aria-pressed=if (h - c.hours).abs() < 1e-9 { "true" } else { "false" }
                                on:click=move |_| send(CrewEvent::Hours(h))>{format!("{h} h")}</button>
                        }).collect_view()}
                    </div>
                </div>
                <label class="inline" title=t::PLANNED_TIP>
                    <span>{t::PLANNED_LABEL}</span>
                    <input type="number" min="0" step="5" prop:value=planned.get_untracked()
                        on:change=move |ev| { let v = event_target_value(&ev); planned.set(v.clone());
                            send(CrewEvent::Planned(v.trim().parse::<f64>().ok())) }/>
                </label>
                <p class=if c.go { "verdict go" } else { "verdict nogo" }>{c.verdict.clone()}</p>
                <ul class="plain fine">{c.lines.iter().map(|l| view! { <li>{l.clone()}</li> }).collect_view()}</ul>
                <p class=if c.rescue_pass { "fine" } else { "fine warn" }>{c.rescue.clone()}</p>
                <h4>{t::ROLES_TITLE} <span class="tag tag-analog">{c.roles_label.clone()}</span></h4>
                <ul class="plain fine"><li>{c.roles.0.clone()}</li><li>{c.roles.1.clone()}</li></ul>
                <p class="fine">{c.source_note.clone()}" "<a href=c.source_url.clone() target="_blank" rel="noopener">{t::SOURCE_LINK}</a></p>
            </div>
        }.into_any()
    }
}

/// The published plan's walk (s), loop (m), max walk-back (s) and steepest corrected slope (deg), remembered the first time
/// the panel shows it (a share link that opens another plan shows no change row until Reset).
static PUBLISHED: std::sync::Mutex<Option<(f64, f64, f64, f64)>> = std::sync::Mutex::new(None);

/// Change of the current plan against the published one: four numbers, signed; "same" when nothing moved.
fn plan_delta(m: &PanelModel) -> impl IntoView {
    use text::plan_panel as p;
    let Some(base) = PUBLISHED.lock().ok().and_then(|g| *g) else { return ().into_any() };
    if !m.feasible {
        return ().into_any();
    }
    let sg = |x: f64, dp: usize| if x.abs() < 0.5 * 10f64.powi(-(dp as i32)) { "\u{b1}0".to_string() } else { format!("{x:+.dp$}").replace('-', "\u{2212}") };
    let cells = [
        (p::D_WALK, sg((m.walk_s - base.0) / 60.0, 1), "min", m.walk_s - base.0),
        (p::D_LOOP, sg(m.length_m - base.1, 0), "m", m.length_m - base.1),
        (p::D_BACK, sg((m.back_s - base.2) / 60.0, 1), "min", m.back_s - base.2),
        (p::D_STEEP, sg(m.worst_deg - base.3, 1), "\u{b0}", m.worst_deg - base.3),
    ];
    let same = m.controls.is_default;
    if same {
        return ().into_any();
    }
    view! {
        <div class="delta" title=p::D_TIP>
            <h4>{if same { p::D_SAME } else { p::D_TITLE }}</h4>
            <div class="dcells">
                {cells.into_iter().map(|(k, v, unit, raw)| {
                    let cls = if same || raw.abs() < 1e-6 { "dc" } else if raw > 0.0 { "dc up" } else { "dc down" };
                    view! {
                    <div class=cls>
                        <span class="k">{k}</span><span class="v num">{v}<small>{unit}</small></span>
                    </div>
                    }
                }).collect_view()}
            </div>
        </div>
    }.into_any()
}

pub(super) fn plan_tab(c: Controls, stops: Vec<String>) -> impl IntoView {
    use text::plan_panel as p;
    let n = stops.len();
    let limit_txt = RwSignal::new(fill(p::LIMIT_LABEL, &[("slope_limit_deg", format!("{}", c.limit_deg))]));
    let bias = format!("{}", c.measured_bias);
    let correct_now = c.correct;
    let must = c.must.clone();
    let budget_txt = RwSignal::new(format!("{}", c.budget_min));
    view! {
        <div class="controls">
            <h4>{u::H_STOPS}</h4>
            <ol class="stoplist">
                {stops.into_iter().enumerate().map(|(i, name)| view! {
                    <li>
                        <button type="button" class="sn" title=text::stop_card::OPEN_HINT
                            on:click=move |_| crate::ui::card::send(crate::ui::card::CardEvent::Open(i))>{name}</button>
                        {(i > 0).then(|| view! {
                            <button type="button" class="mini star" title=p::MUST aria-label=p::MUST
                                aria-pressed=if must.get(i).copied().unwrap_or(false) { "true" } else { "false" }
                                on:click=move |_| send(PanelEvent::ToggleMust(i))>"\u{2605}"</button>
                            <button type="button" class="mini" title=p::MOVE_UP aria-label=p::MOVE_UP disabled={i < 2}
                                on:click=move |_| send(PanelEvent::MoveUp(i))>"\u{2191}"</button>
                            <button type="button" class="mini" title=p::MOVE_DOWN aria-label=p::MOVE_DOWN disabled={i + 1 >= n}
                                on:click=move |_| send(PanelEvent::MoveDown(i))>"\u{2193}"</button>
                            <button type="button" class="mini" title=p::REMOVE aria-label=p::REMOVE disabled={n <= 2}
                                on:click=move |_| send(PanelEvent::Remove(i))>"\u{2715}"</button>
                        })}
                    </li>
                }).collect_view()}
            </ol>
            {c.picking.then(|| view! { <p class="fine warn">{p::PICKING}</p> })}
            <div class="ctl-row">
                {if c.picking {
                    view! { <button type="button" on:click=move |_| send(PanelEvent::CancelPick)>{p::CANCEL_PICK}</button> }.into_any()
                } else {
                    view! { <button type="button" on:click=move |_| send(PanelEvent::StartPick)>{p::ADD_STOP}</button> }.into_any()
                }}
                <button type="button" on:click=move |_| send(PanelEvent::BestOrder)>{p::BEST_ORDER}</button>
                <button type="button" class="ghost" disabled=c.is_default on:click=move |_| send(PanelEvent::Reset)>{p::RESET}</button>
            </div>
            {(!c.note.is_empty()).then(|| view! { <p class="fine note">{c.note.clone()}</p> })}
            <h4>{u::H_RULES}</h4>
            <label class="ctl">
                <span>{move || limit_txt.get()}</span>
                <input type="range" min=c.limit_range.0.to_string() max=c.limit_range.1.to_string() step="1"
                    prop:value=c.limit_deg.to_string()
                    on:input=move |ev| limit_txt.set(fill(p::LIMIT_LABEL, &[("slope_limit_deg", event_target_value(&ev))]))
                    on:change=move |ev| { if let Ok(x) = event_target_value(&ev).parse::<f64>() { send(PanelEvent::Limit(x)) } }/>
            </label>
            <div class="ctl" role="group" aria-label=p::SPEED_LABEL>
                <span>{p::SPEED_LABEL}</span>
                <div class="seg">
                    {c.speeds.iter().map(|&v| {
                        let on_now = (v - c.speed).abs() < 1e-9;
                        view! { <button type="button" aria-pressed=if on_now { "true" } else { "false" }
                                    on:click=move |_| send(PanelEvent::Speed(v))>{format!("{v} {}", p::SPEED_UNIT)}</button> }
                    }).collect_view()}
                </div>
            </div>
            <label class="ctl check">
                <input type="checkbox" prop:checked=correct_now on:change=move |ev| send(PanelEvent::Correct(event_target_checked(&ev)))/>
                <span>{fill(p::CORRECT_LABEL, &[("rc_bias", bias)])}</span>
            </label>
            <h4>{u::H_BUDGET}</h4>
            <div class="inline">
                <input type="number" min="1" step="1" prop:value=budget_txt.get_untracked() aria-label=p::BUDGET_LABEL
                    on:input=move |ev| budget_txt.set(event_target_value(&ev))/>
                <span class="unit">"min"</span>
                <button type="button" on:click=move |_| { if let Ok(x) = budget_txt.get_untracked().parse::<f64>() { send(PanelEvent::FitBudget(x)) } }>
                    {p::FIT_BUDGET}</button>
            </div>
        </div>
    }
}

pub(super) fn stops_tab(list: Vec<StopSummary>) -> impl IntoView {
    view! {
        <ul class="stop-cards">
            {list.into_iter().map(|s| {
                let i = s.index;
                view! {
                    <li>
                        <button type="button" class="stop-row" on:click=move |_| crate::ui::card::send(crate::ui::card::CardEvent::Open(i))>
                            {match s.photo.clone() {
                                Some(src) => view! { <img src=src alt="" loading="lazy"/> }.into_any(),
                                None => view! { <span class="noimg"></span> }.into_any(),
                            }}
                            <span class="sr-text">
                                <b>{s.label.clone()}</b>
                                <small>{s.kind.clone()}</small>
                                <span class="sr-fact">{s.fact.clone()}</span>
                            </span>
                        </button>
                    </li>
                }
            }).collect_view()}
        </ul>
    }
}

/// A small compass: north up, the computed sun (filled) and the photo's own light (outline).
pub(super) fn sun_compass(sun_az: Option<f64>, photo_az: Option<f64>) -> impl IntoView {
    let at = |az: f64, r: f64| ((az.to_radians().sin() * r + 36.0), (36.0 - az.to_radians().cos() * r));
    view! {
        <svg class="sun-compass" width="72" height="72" viewBox="0 0 72 72" aria-hidden="true">
            <circle cx="36" cy="36" r="27" fill="none" stroke="var(--line-strong)"/>
            <text x="36" y="7" text-anchor="middle" class="sc-n">"N"</text>
            {photo_az.map(|a| { let (x, y) = at(a, 27.0); view! { <circle cx=x.to_string() cy=y.to_string() r="5" fill="none" stroke="var(--ink-2)" stroke-width="1.5"/> } })}
            {sun_az.map(|a| { let (x, y) = at(a, 27.0); let (x0, y0) = at(a, 12.0);
                view! { <line x1=x0.to_string() y1=y0.to_string() x2=x.to_string() y2=y.to_string() stroke="var(--route)" stroke-width="2"/>
                        <circle cx=x.to_string() cy=y.to_string() r="5" fill="var(--route)"/> } })}
        </svg>
    }
}

pub(super) fn sun_tab(light: RwSignal<LightModel>) -> impl IntoView {
    use crate::plan::sunlight::{send as light_send, LightEvent};
    use text::light as t;
    move || {
        let m = light.get();
        view! {
            <div class="light">
                {(!m.wx_rows.is_empty() || !m.wx_note.is_empty() || !m.rad_note.is_empty()).then(|| view! {
                    <h4>{m.wx_title.clone()}</h4>
                    <dl class="wx" title=m.wx_disclaimer.clone()>
                        {m.wx_rows.iter().map(|(k, v)| view! { <dt>{k.clone()}</dt><dd class="num">{v.clone()}</dd> }).collect_view()}
                    </dl>
                    {(!m.wx_note.is_empty()).then(|| view! { <p class="fine">{m.wx_note.clone()}</p> })}
                    {(!m.rad_note.is_empty()).then(|| view! { <p class="fine">{m.rad_note.clone()}</p> })}
                    {(!m.sw_text.is_empty()).then(|| view! {
                        <p class="fine">{m.sw_text.clone()}
                            {m.sw_events.iter().map(|(d, k, l)| view! { " " <a href=l.clone() target="_blank" rel="noopener">{format!("{k} {d}")}</a> }).collect_view()}
                        </p>
                    })}
                    <p class="fine" title=m.wx_disclaimer.clone()>{m.wx_source.clone()}" "
                        <a href=m.wx_url.clone() target="_blank" rel="noopener">{t::WX_SOURCE_LINK}</a></p>
                })}
                <p class="lead">{m.status.clone()}</p>
                <h4>{u::H_HOURS}</h4>
                <table class="hours">
                    <thead><tr>
                        <th scope="col">{t::COL_START}</th><th scope="col" title=t::COL_SUN>"Sun"</th><th scope="col">{t::COL_PASS}</th>
                        <th scope="col" title=t::COL_TOWARD>"Into sun"</th><th scope="col" title=t::COL_SHADOW>"Shade"</th>
                    </tr></thead>
                    <tbody>
                        {m.rows.iter().map(|r| {
                            let h = r.hour;
                            let cls = format!("{}{}{}", if r.passes { "pass" } else { "fail" }, if r.best { " best" } else { "" },
                                              if r.selected { " sel" } else { "" });
                            view! {
                                <tr class=cls>
                                    <td><button type="button" class="hour" aria-pressed=if r.selected { "true" } else { "false" }
                                        title=t::PICK_HOUR on:click=move |_| light_send(LightEvent::Hour(h))>{r.label.clone()}</button>
                                        {r.best.then(|| view! { " \u{2605}" })}</td>
                                    <td class="num">{r.sun.clone()}</td>
                                    <td>{if r.passes { "\u{2713}" } else { "\u{2013}" }}</td>
                                    <td class="num">{r.toward.clone()}</td>
                                    <td class="num">{r.shadow.clone()}</td>
                                </tr>
                            }
                        }).collect_view()}
                    </tbody>
                </table>
                <p class="fine">{m.intro.clone()}</p>
                <h4>{u::H_LIGHT}</h4>
                <div class="sun-row">
                    {sun_compass(m.sun_az, m.photo_az)}
                    <div>
                        {(!m.sun_text.is_empty()).then(|| view! { <p class="fine"><span class="sw sun"></span>{m.sun_text.clone()}</p> })}
                        {(!m.photo_text.is_empty()).then(|| view! { <p class="fine"><span class="sw photo"></span>{m.photo_text.clone()}</p> })}
                    </div>
                </div>
                {m.warn_south.then(|| view! { <p class="fine warn">{t::WARN_SOUTH}</p> })}
                <details class="more"><summary>{u::H_BASIS}</summary>
                    <p class="fine">{m.ties.clone()}</p>
                    {(!m.shadow_text.is_empty()).then(|| view! { <p class="fine">{m.shadow_text.clone()}</p> })}
                </details>
            </div>
        }
    }
}

fn ready_word(l: &str) -> &'static str {
    use text::readiness as r;
    match l { "ok" => r::LVL_OK, "limited" => r::LVL_LIMITED, "no" => r::LVL_NO, _ => r::LVL_CONTEXT }
}

pub(super) fn about_tab(m: PanelModel, trek_link: String) -> impl IntoView {
    use crate::plan::share::{act, ShareAction};
    let vals = m.vals.clone();
    view! {
        <p class="lead">{text::app::TAGLINE}</p>
        <h4>{u::H_BASIS}</h4>
        <ul class="basis">
            {text::assumptions_panel::ALL.iter().filter(|(k, _)| *k != "title").map(|(k, t)| {
                let tag = text::assumption_tags::ALL.iter().find(|(tk, _)| tk == k).map(|(_, v)| *v).unwrap_or("");
                view! { <li><span class=format!("tag tag-{}", tag.to_lowercase())>{tag}</span>{fill(t, &vals)}</li> }
            }).collect_view()}
        </ul>
        <h4>{u::H_SHARE}</h4>
        <p class="fine">{text::share::HINT}</p>
        <div class="ctl-row share">
            <button type="button" on:click=move |_| act(ShareAction::CopyLink)>{text::share::COPY_LINK}</button>
            <button type="button" on:click=move |_| act(ShareAction::Json)>{text::share::EXPORT_JSON}</button>
            <button type="button" on:click=move |_| act(ShareAction::Csv)>{text::share::EXPORT_CSV}</button>
            <button type="button" title=text::share::EXPORT_GEO_TIP on:click=move |_| act(ShareAction::GeoJson)>{text::share::EXPORT_GEO}</button>
        </div>
        {(!trek_link.is_empty()).then(|| view! {
            <p class="fine"><a href=trek_link.clone() target="_blank" rel="noopener noreferrer" title=u::TREK_OPEN_TIP>{u::TREK_OPEN}</a></p>
        })}
        <h4>{u::H_SOURCES}</h4>
        <ul class="plain fine">
            {text::credits_screen::ALL.iter().filter(|(k, _)| *k != "title").map(|(_, t)| view! { <li>{*t}</li> }).collect_view()}
        </ul>
        <p class="fine">{text::app::ADVISORY}</p>
        <p class="fine">{text::app::NOT_ENDORSED}</p>
        <span data-ready="1" hidden=true></span>
    }
}

// ================================================================= stop card (in the inspector) and photo view
