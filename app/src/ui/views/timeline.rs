//! EVA timeline (bottom): the stops and the terrain profile along the route (distance), and the whole EVA on a time axis
//! (egress, walking legs, stations, ingress, the emergency reserve; the sun's height over it). Hover finds, click centres
//! (route lanes) or opens the station's card / seeks the simulation (EVA lane).
use wasm_bindgen::JsCast;

use crate::ui::brief::BriefModel;
use crate::ui::model::{LightModel, PanelModel};
use crate::ui::panel::{Sample, HOVER};
use crate::text::{fill, ui as u};

use super::*;

#[component]
pub(super) fn Timeline(panel: RwSignal<Option<PanelModel>>, light: RwSignal<LightModel>, brief: RwSignal<BriefModel>,
                       sim: RwSignal<crate::plan::simrun::SimModel>, drill: RwSignal<crate::ui::model::DrillModel>) -> impl IntoView {
    let hover = RwSignal::new(None::<usize>);
    // the chart only rebuilds when the plan or light change; the playhead and transport follow the clock separately
    let active = Memo::new(move |_| sim.with(|s| s.active));
    let eva = Memo::new(move |_| sim.with(|s| EvaLane { blocks: s.blocks.clone(), eva_s: s.eva_s, reserve_s: s.reserve_s, sun: s.sun.clone(),
                                                        start_h: s.start_h, dark: s.dark.clone(), dark_note: s.dark_note.clone() }));
    let chart = move || {
        let Some(m) = panel.get() else { return ().into_any() };
        let l = light.get();
        let weak = brief.with(|b| b.weak_legs.clone());
        timeline_view(m, l, weak, hover, active.get(), eva.get(), sim, drill).into_any()
    };
    view! { {move || active.get().then(|| transport(sim))} {chart} {move || playhead(sim)} }
}

/// Transport (Walk): start, previous stop, play/pause, next stop, speed, clock, CSV.
fn transport(sim: RwSignal<crate::plan::simrun::SimModel>) -> impl IntoView {
    use crate::plan::simrun::{send, SimEvent};
    use crate::text::sim as t;
    view! {
        <div class="tl-transport" role="toolbar" aria-label=t::PLAY>
            <button type="button" class="tb" title=t::START aria-label=t::START on:click=|_| send(SimEvent::Start)>"\u{23EE}"</button>
            <button type="button" class="tb" title=t::PREV aria-label=t::PREV on:click=|_| send(SimEvent::Prev)>"\u{23EA}"</button>
            <button type="button" class="tb play" title=move || if sim.with(|s| s.playing) { t::PAUSE } else { t::PLAY }
                aria-label=t::PLAY on:click=|_| send(SimEvent::Toggle)>{move || if sim.with(|s| s.playing) { "\u{23F8}" } else { "\u{25B6}" }}</button>
            <button type="button" class="tb" title=t::NEXT aria-label=t::NEXT on:click=|_| send(SimEvent::Next)>"\u{23E9}"</button>
            <div class="seg tb-speed" role="group" aria-label=t::SPEED title=t::SPEED>
                {move || sim.with(|s| s.speeds.clone()).into_iter().map(|v| view! {
                    <button type="button" aria-pressed=move || if (sim.with(|s| s.speed) - v).abs() < 1e-3 { "true" } else { "false" }
                        on:click=move |_| send(SimEvent::Speed(v))>{format!("{v}\u{00d7}")}</button>
                }).collect_view()}
            </div>
            <span class="tb-clock" title=t::LBL_CLOCK>{move || sim.with(|s| s.clock.clone())}</span>
            <span class="tb-elapsed" title=t::LBL_ELAPSED>{move || sim.with(|s| s.elapsed.clone())}</span>
            <span class="tb-note" title=move || sim.with(|s| if s.manual { t::MANUAL.to_string() } else { s.dwell_note.clone() })>
                {move || sim.with(|s| if s.manual { t::MANUAL_SHORT } else { "\u{24d8}" })}</span>
            <button type="button" class="tb csv" title=t::CSV aria-label=t::CSV on:click=|_| send(SimEvent::Csv)>"CSV"</button>
        </div>
    }
}

/// The playhead across the lanes (Walk): x from the distance walked, like the chart.
fn playhead(sim: RwSignal<crate::plan::simrun::SimModel>) -> impl IntoView {
    let s = sim.get();
    if !s.active {
        return ().into_any();
    }
    // the EVA clock, as the lanes (one time axis)
    let tmax = (s.eva_s + s.reserve_s).max(s.blocks.last().map(|b| b.t1).unwrap_or(0.0)).max(1.0);
    let win_w = web_sys::window().and_then(|w| w.inner_width().ok()).and_then(|v| v.as_f64()).unwrap_or(1280.0) as f32;
    let w = (win_w - 2.0).max(600.0);
    let (lab, mr) = (92.0f32, 20.0f32);
    let x = lab + s.t / tmax * (w - lab - mr);
    view! { <div class="tl-playhead" style:left=format!("{x}px")></div> }.into_any()
}

pub(super) fn clock(start_h: f64, secs: f64) -> String {
    let h = start_h + secs / route_core::light::MARS_HOUR_S;
    format!("{:02}:{:02}", h.floor() as i64, ((h.fract()) * 60.0).floor() as i64)
}

/// The whole EVA for the time lane (from the simulation model).
#[derive(Clone, Debug, PartialEq, Default)]
pub(super) struct EvaLane {
    blocks: Vec<crate::plan::simrun::Block>,
    eva_s: f32,
    reserve_s: f32,
    sun: Vec<(f32, f32)>,
    start_h: f64,
    /// twilight / night stretches (t0, t1 s, 1 twilight | 2 night) and the first one as a note
    dark: Vec<(f32, f32, u8)>,
    dark_note: Option<String>,
}

/// Time of each profile sample on the EVA clock: inside leg i the walk block (t0, t1) is spread over the leg's distance;
/// without an EVA simulation, distance stands in for time. Stations show as flat stretches between legs.
pub(super) fn sample_times(samples: &[Sample], leg_starts: &[usize], walk_ts: &[(f32, f32)]) -> Vec<f32> {
    let n = samples.len();
    let dmax = samples.last().map(|q| q.d).unwrap_or(1.0).max(1.0);
    if walk_ts.is_empty() || leg_starts.is_empty() {
        return samples.iter().map(|q| q.d / dmax).collect();
    }
    let mut out = vec![0.0f32; n];
    for (i, &k0) in leg_starts.iter().enumerate() {
        let k1 = leg_starts.get(i + 1).copied().unwrap_or(n - 1).min(n - 1);
        let (t0, t1) = walk_ts.get(i).copied().unwrap_or((walk_ts.last().map(|w| w.1).unwrap_or(0.0), walk_ts.last().map(|w| w.1).unwrap_or(0.0)));
        let (d0, d1) = (samples[k0.min(n - 1)].d, samples[k1].d);
        for k in k0.min(n - 1)..=k1 {
            let f = if d1 > d0 { (samples[k].d - d0) / (d1 - d0) } else { 0.0 };
            out[k] = t0 + f * (t1 - t0);
        }
    }
    out
}

/// Leg index of profile sample k.
fn leg_of(leg_starts: &[usize], k: usize) -> usize {
    leg_starts.iter().rposition(|&s| s <= k).unwrap_or(0)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn timeline_view(m: PanelModel, l: LightModel, weak: Vec<usize>, hover: RwSignal<Option<usize>>, walk: bool, eva: EvaLane,
                            sim: RwSignal<crate::plan::simrun::SimModel>, drill: RwSignal<crate::ui::model::DrillModel>) -> impl IntoView {
    use crate::scene::select::{send as sel_send, Pick, SelEvent};
    let samples: Vec<Sample> = m.samples.clone();
    if samples.is_empty() {
        return ().into_any();
    }
    let win_w = web_sys::window().and_then(|w| w.inner_width().ok()).and_then(|v| v.as_f64()).unwrap_or(1280.0) as f32;
    let w = (win_w - 2.0).max(600.0);
    let (lab, mr) = (92.0f32, 20.0f32);
    // two lanes on ONE clock (a stops lane on distance did not line up with the EVA lane on time)
    let lanes = [(16.0f32, 70.0f32), (78.0, 134.0)]; // terrain, EVA (top, bottom)
    let h = 140.0f32;
    let walk_ts: Vec<(f32, f32)> = eva.blocks.iter().filter(|b| b.kind == "walk").map(|b| (b.t0, b.t1)).collect();
    let t_end = eva.blocks.last().map(|b| b.t1).unwrap_or(0.0);
    let have_clock = !walk_ts.is_empty();
    let tmax = if have_clock { (eva.eva_s + eva.reserve_s).max(t_end).max(1.0) } else { 1.0 };
    let xt = move |t: f32| lab + t / tmax * (w - lab - mr);
    let ts = std::sync::Arc::new(sample_times(&samples, &m.leg_starts, &walk_ts));
    let zmax = samples.iter().map(|q| q.z).fold(1.0f32, f32::max);
    let yz = move |z: f32| lanes[0].1 - z / zmax * (lanes[0].1 - lanes[0].0 - 6.0);
    let ys = move |e: f64| lanes[1].1 - 16.0 - (e.clamp(0.0, 90.0) / 90.0) as f32 * (lanes[1].1 - lanes[1].0 - 18.0);
    let hour_s = route_core::light::MARS_HOUR_S as f32;
    let pts: Vec<String> = samples.iter().zip(ts.iter()).map(|(q, &t)| format!("{:.1},{:.1}", xt(t), yz(q.z))).collect();
    let t_last = ts.last().copied().unwrap_or(0.0);
    let area = format!("M{},{} L{} L{},{} Z", xt(ts[0]), lanes[0].1, pts.join(" L"), xt(t_last), lanes[0].1);
    // per-leg time ranges on the clock
    let leg_t: Vec<(f32, f32)> = (0..m.leg_starts.len()).map(|i| {
        // the leg's own walking time (not the station visit after it)
        if let Some(&wt) = walk_ts.get(i) {
            return wt;
        }
        let k0 = m.leg_starts[i].min(samples.len() - 1);
        let k1 = m.leg_starts.get(i + 1).copied().unwrap_or(samples.len() - 1).min(samples.len() - 1);
        (ts[k0], ts[k1])
    }).collect();
    let sun_line = (!eva.sun.is_empty() && have_clock).then(|| {
        let p: Vec<String> = eva.sun.iter().filter(|q| q.0 <= tmax).map(|&(t, e)| format!("{:.1},{:.1}", xt(t), ys(e as f64))).collect();
        format!("M{}", p.join(" L"))
    });
    let ticks: Vec<(f32, String)> = if have_clock {
        (0..=((tmax / hour_s).floor() as i64)).map(|k| {
            let t = k as f32 * hour_s;
            let hh = eva.start_h + k as f64;
            (xt(t), format!("{:02}:{:02}", (hh.floor() as i64).rem_euclid(24), ((hh.fract() * 60.0).round() as i64) % 60))
        }).collect()
    } else { Vec::new() };
    // samples -> time runs, for the hatching and out-of-sight strips
    // hatching and out-of-sight strips per walking step only (never across a station visit, which is flat time)
    let (mut bands, mut unseen): (Vec<(u8, f32, f32)>, Vec<(f32, f32)>) = (Vec::new(), Vec::new());
    for k in 0..samples.len().saturating_sub(1) {
        if leg_of(&m.leg_starts, k) != leg_of(&m.leg_starts, k + 1) && m.leg_starts.contains(&(k + 1)) {
            continue;
        }
        let (t0, t1) = (ts[k], ts[k + 1]);
        let b = crate::scene::trust::band(samples[k + 1].s, m.caution, m.limit);
        if b > 0 {
            match bands.last_mut() { Some(r) if r.0 == b && (r.2 - t0).abs() < 1e-3 => r.2 = t1, _ => bands.push((b, t0, t1)) }
        }
        if !samples[k + 1].seen {
            match unseen.last_mut() { Some(r) if (r.1 - t0).abs() < 1e-3 => r.1 = t1, _ => unseen.push((t0, t1)) }
        }
    }
    let eva_blocks = eva.blocks.clone();
    let (eva_s, reserve_s) = (eva.eva_s, eva.reserve_s);
    let stop_pos: Vec<(f32, f32)> = m.stop_cards.iter().map(|s| (s.e, s.n)).collect();
    let samples = std::sync::Arc::new(samples);
    // pointer -> nearest sample by time
    let at = {
        let ts = ts.clone();
        move |ev: &web_sys::MouseEvent| -> Option<usize> {
            let el = ev.current_target()?.dyn_into::<web_sys::Element>().ok()?;
            let r = el.get_bounding_client_rect();
            let t = ((ev.client_x() as f64 - r.left()) / r.width().max(1.0)) as f32 * (xt(tmax) - lab) / (w - lab - mr) * tmax;
            ts.iter().enumerate().min_by(|a, b| (a.1 - t).abs().total_cmp(&(b.1 - t).abs())).map(|x| x.0)
        }
    };
    let at2 = at.clone();
    let (s_move, s_click, s_tip) = (samples.clone(), samples.clone(), samples.clone());
    let leg_starts = m.leg_starts.clone();
    let lane_names = [u::LANE_TERRAIN, crate::text::sim::LANE_EVA];
    let ts_tip = ts.clone();
    view! {
        <div class="tl-head">
            <span class="tl-title">{u::TL_TITLE}</span>
            {(!m.feasible && !m.blocked_legs().is_empty()).then(|| view! {
                <span class="tl-blocked">{fill(u::TL_BLOCKED, &[("legs", m.blocked_label())])}</span> })}
            // the EVA runs past sunset or starts before sunrise (plan/daylight.rs)
            {have_clock.then(|| eva.dark_note.clone()).flatten().map(|n| view! {
                <span class="tl-dark-note" title=crate::text::sim::DARK_TIP>{format!("\u{263e} {n}")}</span> })}
            <span class="tl-hint">{u::TL_HINT}</span>
        </div>
        <svg class="tl" viewBox=format!("0 0 {w} {h}") width=w.to_string() height=h.to_string() role="img" aria-label=u::TL_TITLE>
            <defs>
                <pattern id="tl-h1" width="5" height="5" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
                    <line x1="0" y1="0" x2="0" y2="5" stroke="rgba(255,255,255,0.45)" stroke-width="1.2"/></pattern>
                <pattern id="tl-h2" width="5" height="5" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
                    <line x1="0" y1="0" x2="0" y2="5" stroke="rgba(255,255,255,0.45)" stroke-width="1.2"/>
                    <line x1="0" y1="0" x2="5" y2="0" stroke="rgba(255,255,255,0.45)" stroke-width="1.2"/></pattern>
            </defs>
            {lanes.iter().zip(lane_names).map(|(&(t0, t1), name)| view! {
                <rect x=lab.to_string() y=t0.to_string() width=(w - lab - mr).to_string() height=(t1 - t0).to_string() class="lane"/>
                <text x="12" y=((t0 + t1) / 2.0 + 4.0).to_string() class="lane-name">{name}</text>
            }).collect_view()}
            // terrain lane: steep stretches hatched (same meaning as the map), out-of-sight strip, the height profile; legs the
            // uncertainty runs disagree on are outlined dashed; the selected leg is lit
            {bands.into_iter().map(|(b, t0, t1)| view! {
                <rect x=xt(t0).to_string() y=lanes[0].0.to_string() width=(xt(t1) - xt(t0)).max(1.0).to_string() height=(lanes[0].1 - lanes[0].0).to_string()
                    fill=format!("url(#tl-h{b})")/>
            }).collect_view()}
            {unseen.into_iter().map(|(t0, t1)| view! {
                <rect x=xt(t0).to_string() y=(lanes[0].1 - 4.0).to_string() width=(xt(t1) - xt(t0)).max(1.0).to_string() height="4" class="unseen">
                    <title>{crate::text::ops::TL_SIGHT}</title></rect>
            }).collect_view()}
            {weak.iter().filter_map(|&i| leg_t.get(i)).map(|&(t0, t1)| view! {
                <rect x=(xt(t0) + 1.0).to_string() y=(lanes[0].0 + 1.0).to_string() width=(xt(t1) - xt(t0) - 2.0).max(1.0).to_string()
                    height=(lanes[0].1 - lanes[0].0 - 2.0).to_string() rx="3" class="leg-weak">
                    <title>{fill(crate::text::brief::TL_WEAK, &[("pct", format!("{}", (crate::ui::brief::WEAK_LEG_SHARE * 100.0).round() as i64))])}</title>
                </rect>
            }).collect_view()}
            {
                let leg_t = leg_t.clone();
                move || drill.with(|d| d.leg).and_then(|i| leg_t.get(i).copied()).map(|(t0, t1)| view! {
                    <rect x=xt(t0).to_string() y=lanes[0].0.to_string() width=(xt(t1) - xt(t0)).max(2.0).to_string()
                        height=(lanes[0].1 - lanes[0].0).to_string() class="tl-sel"/>
                })
            }
            <path d=area class="area"/>
            <path d=format!("M{}", pts.join(" L")) class="line"/>
            <text x=(lab + 4.0).to_string() y=(lanes[0].0 + 10.0).to_string() class="tick">{format!("{:.0} m", zmax)}</text>
            // EVA lane: the sun's walking band and height, then egress / legs / stations / ingress, reserve hatched
            <rect x=lab.to_string() y=ys(l.band.1).to_string() width=(w - lab - mr).to_string() height=(ys(l.band.0) - ys(l.band.1)).to_string() class="band"/>
            // twilight and night on the EVA clock (plan/daylight.rs), behind the blocks
            {have_clock.then(|| eva.dark.iter().filter(|r| r.0 < tmax).map(|&(t0, t1, k)| view! {
                <rect x=xt(t0).to_string() y=lanes[1].0.to_string() width=(xt(t1.min(tmax)) - xt(t0)).max(1.0).to_string()
                    height=(lanes[1].1 - lanes[1].0 - 14.0).to_string() class=if k == 2 { "tl-dark night" } else { "tl-dark twilight" }>
                    <title>{if k == 2 { crate::text::sim::LANE_NIGHT } else { crate::text::sim::LANE_TWILIGHT }}</title>
                </rect>
            }).collect_view())}
            {sun_line.map(|d| view! { <path d=d class="sun-line"/> })}
            {(reserve_s > 0.0 && have_clock).then(|| view! {
                <rect x=xt(eva_s).to_string() y=(lanes[1].0 + 4.0).to_string() width=(xt(eva_s + reserve_s) - xt(eva_s)).max(1.0).to_string()
                    height=(lanes[1].1 - lanes[1].0 - 22.0).to_string() fill="url(#tl-h1)" class="eva-reserve"><title>{crate::text::sim::RESERVE}</title></rect>
                <line x1=xt(eva_s).to_string() x2=xt(eva_s).to_string() y1=lanes[1].0.to_string() y2=(lanes[1].1 - 16.0).to_string() class="eva-end"/>
            })}
            {eva_blocks.into_iter().enumerate().map(|(_, b)| {
                let (x0, x1) = (xt(b.t0), xt(b.t1));
                let bt0 = b.t0;
                let (y0, hh) = if b.kind == "walk" { (lanes[1].0 + 14.0, 6.0) } else { (lanes[1].0 + 6.0, 22.0) };
                let stop = b.stop;
                let label = b.label.clone();
                let tip = format!("{} \u{b7} {}\u{2013}{}", b.label, clock(eva.start_h, b.t0 as f64), clock(eva.start_h, b.t1 as f64));
                let pos = stop.and_then(|k| stop_pos.get(k).copied());
                let kind = b.kind;
                view! {
                    <g class=move || format!("eva-b eva-{kind}{}", if stop.is_some() && drill.with(|d| d.station) == stop { " sel" } else { "" })
                        on:pointerenter=move |_| if let (Some(p), Ok(mut g)) = (pos, HOVER.lock()) { *g = Some(p); }
                        on:pointerleave=move |_| if let Ok(mut g) = HOVER.lock() { *g = None; }
                        on:click=move |_| {
                            if walk { crate::plan::simrun::send(crate::plan::simrun::SimEvent::SeekTime(bt0 + 0.5)); }
                            else if let Some(k) = stop { crate::ui::card::send(crate::ui::card::CardEvent::Open(k)); }
                        }>
                        <title>{tip}</title>
                        <rect x=(x0 + 0.5).to_string() y=y0.to_string() width=(x1 - x0 - 1.0).max(1.0).to_string() height=hh.to_string() rx="3"/>
                        {(kind != "walk" && x1 - x0 > 7.0 * label.chars().count().min(14) as f32 + 6.0).then(|| view! {
                            <text x=((x0 + x1) / 2.0).to_string() y=(y0 + 15.0).to_string() text-anchor="middle" class="eva-lbl">{label.clone()}</text>
                        })}
                    </g>
                }
            }).collect_view()}
            {ticks.into_iter().map(|(tx, lbl)| view! {
                <text x=tx.to_string() y=(lanes[1].1 - 3.0).to_string() class="tick" text-anchor="middle">{lbl}</text>
            }).collect_view()}
            {move || {
                let (on, t) = sim.with(|s| (s.active, s.t));
                on.then(|| view! { <line x1=xt(t).to_string() x2=xt(t).to_string() y1=lanes[0].0.to_string() y2=(lanes[1].1 - 14.0).to_string() class="eva-now"/> })
            }}
            // hover and click on the terrain lane: find on the map / select the leg
            {move || hover.get().map(|i| view! {
                <line x1=xt(ts_tip[i]).to_string() x2=xt(ts_tip[i]).to_string() y1=lanes[0].0.to_string() y2=lanes[0].1.to_string() class="cross"/>
            })}
            <rect x=lab.to_string() y=lanes[0].0.to_string() width=(w - lab - mr).to_string() height=(lanes[0].1 - lanes[0].0).to_string()
                fill="transparent" class="hit"
                on:pointermove=move |ev| {
                    if let Some(i) = at(&ev) {
                        hover.set(Some(i));
                        if let Ok(mut g) = HOVER.lock() { *g = Some((s_move[i].e, s_move[i].n)); }
                    }
                }
                on:pointerleave=move |_| {
                    hover.set(None);
                    if let Ok(mut g) = HOVER.lock() { *g = None; }
                }
                on:click=move |ev| {
                    if let Some(i) = at2(&ev) {
                        let q = s_click[i];
                        if walk {
                            crate::plan::simrun::send(crate::plan::simrun::SimEvent::SeekDist(q.d));
                        } else {
                            sel_send(SelEvent::Pick(Pick::Leg(leg_of(&leg_starts, i)), (q.e, q.n)));
                        }
                    }
                }/>
        </svg>
        {move || hover.get().map(|i| {
            let q = s_tip[i];
            let seen = if q.seen { "" } else { " \u{b7} exit out of sight" };
            let left = (xt(ts[i]) - 90.0).clamp(0.0, w - 180.0);
            view! {
                <div class="tl-tip" style:left=format!("{left}px")>
                    {format!("{}{} m \u{b7} slope {}\u{b0}{}", if have_clock { format!("{} \u{b7} ", clock(eva.start_h, ts[i] as f64)) } else { String::new() },
                             q.d.round() as i64, q.s.round() as i64, seen)}
                </div>
            }
        })}
    }.into_any()
}
