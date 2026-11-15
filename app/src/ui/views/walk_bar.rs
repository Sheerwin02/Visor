//! Walk-mode top bar (HUD).

use crate::ui::model::TopBarModel;
use crate::text::{self, fill};

use super::*;

#[component]
pub(super) fn TopBar(top: RwSignal<TopBarModel>, sim: RwSignal<crate::plan::simrun::SimModel>,
                     exec: RwSignal<crate::ui::execlog::ExecLog>) -> impl IntoView {
    use crate::ui::hud::{COMPASS_HALF_DEG, COMPASS_PX_PER_DEG};
    use text::hud as t;
    // Curiosity's 360-degree view at the station, opened from the station panel: (panorama, heading when opened)
    let pano: RwSignal<Option<(crate::ui::model::Pano, f64)>> = RwSignal::new(None);
    // the station panel opens to its plan on demand (it covered a quarter of the view at every station)
    let sp_open = RwSignal::new(false);
    // compass ticks: every 5 deg over -180..540, longer every 15, labelled every 45
    let ticks: Vec<(f32, i32)> = (-36i32..=108).map(|i| ((i * 5 + 180) as f32 * COMPASS_PX_PER_DEG, (i * 5).rem_euclid(360))).collect();
    let label = |h: i32| match h {
        0 => text::compass::N.to_string(),
        90 => text::compass::E.to_string(),
        180 => text::compass::S.to_string(),
        270 => text::compass::W.to_string(),
        _ => h.to_string(),
    };
    let width = 2.0 * COMPASS_HALF_DEG * COMPASS_PX_PER_DEG;
    let m = move || top.get();
    let tip = move |key: &'static str| {
        move || {
            let m = top.get();
            let (rt, rd, rs) = m.rover_tilt.clone().unwrap_or_default();
            fill(key, &[("stop_name", m.next_label.clone()), ("speed_flat_mps", m.speed.clone()), ("rc_bias", m.bias.clone()),
                        ("slope_map_deg", m.slope_map.clone()), ("slope_corrected_deg", m.slope.clone()), ("rover_tilt_deg", rt),
                        ("distance_m", rd), ("sol", rs)])
        }
    };
    view! {
        // first person: a helmet-visor frame (cosmetic)
        <div id="visor-frame" aria-hidden="true" style:display=move || if m().visible && !m().third { "" } else { "none" }></div>
        <div id="hud" role="region" aria-label="Walk top bar" style:display=move || if m().visible { "" } else { "none" }>
            // one header line: where to, how far, how long; the ridge caveat as an icon
            <div id="hud-head">
                <span id="hud-next">{move || match sim.with(|s| s.station.clone()).filter(|st| !st.egress) {
                    // at a station: say where the crew is; the next stop comes after it
                    Some(st) => fill(t::AT_STATION, &[("stop_name", st.name.clone()), ("label", m().next_label)]),
                    None => format!("\u{2192} {}", m().next_label),
                }}</span>
                <span id="hud-dist-time">{move || format!("{} {} \u{b7} {} {}", m().dist, t::UNIT_M, m().time, t::UNIT_MIN)}</span>
                <span id="hud-ridge" role="img" aria-label=t::RIDGE_STRIP title=t::RIDGE_TIP>"\u{26a0}"</span>
                // day / twilight / night from the simulation clock (plan/daylight.rs); the tooltip gives sunrise and sunset
                {move || {
                    let (label, class, tip) = sim.with(|s| s.daylight.clone());
                    (!label.is_empty()).then(|| view! { <a id="hud-day" class=format!("day-{class}") href=crate::plan::daylight::TWILIGHT_SOURCE target="_blank"
                                rel="noopener" title=tip>{label}</a> })
                }}
            </div>
            <div id="compass" role="img"
                aria-label=move || fill(t::DIRECTION_ARIA, &[("stop_name", m().next_label), ("bearing_deg", m().bearing.to_string())])
                style:width=format!("{width}px")>
                <div id="compass-strip" style:transform=move || format!("translateX({}px)", m().strip_px)>
                    {ticks.into_iter().map(|(x, h)| {
                        let class = if h % 45 == 0 { "ct ct-l" } else if h % 15 == 0 { "ct ct-m" } else { "ct" };
                        view! {
                            <div class=class style:left=format!("{x}px")></div>
                            {(h % 45 == 0).then(|| view! { <div class="cl" style:left=format!("{x}px")>{label(h)}</div> })}
                        }
                    }).collect_view()}
                </div>
                <div id="compass-caret"></div>
                <div id="compass-target" style:left=move || format!("{}px", m().marker_px) data-rel=move || m().rel.to_string()>
                    <span id="compass-target-deg">{move || {
                        let m = m();
                        match m.marker_side {
                            -1 => format!("{} {}", text::compass::BEHIND_LEFT, m.bearing),
                            1 => format!("{} {}", m.bearing, text::compass::BEHIND_RIGHT),
                            _ => m.bearing.to_string(),
                        }
                    }}</span>
                </div>
            </div>
            <div id="hud-tiles">
                // distance and time are in the header, the clock and EVA time in the timeline (kept hidden for the tests' ids)
                <span id="hud-dist" hidden>{move || m().dist}</span><span id="hud-time" hidden>{move || m().time}</span>
                <div class=move || if m().rover_tilt.is_some() { "ht rover" } else { "ht" } id="ht-slope"
                    title=move || if m().rover_tilt.is_some() { tip(t::TIP_ROVER_TILT)() } else { tip(t::TIP_SLOPE)() }>
                    <div class="ht-l">{move || if m().rover_tilt.is_some() { t::LBL_ROVER_TILT } else { t::LBL_SLOPE }}
                        {move || m().verified.then(|| view! { <span class="ht-badge" title=t::ROVER_VERIFIED>{t::ROVER_GROUND}</span> })}</div>
                    <div class="ht-v"><span id="hud-slope">{move || m().rover_tilt.map(|r| r.0).unwrap_or_else(|| m().slope)}</span><small>{t::UNIT_DEG}</small>
                        {move || m().rover_tilt.is_some().then(|| view! { <small class="ht-map">{fill(t::MAP_SMALL, &[("x", m().slope)])}</small> })}</div>
                </div>
                <div class=move || match m().comms { 2 => "ht comms ok", 1 => "ht comms relay", _ => "ht comms lost" } id="ht-comms" title=t::TIP_COMMS>
                    <div class="ht-l">{t::LBL_COMMS}</div>
                    <div class="ht-v"><span id="hud-comms">{move || match m().comms { 2 => t::COMMS_EXIT, 1 => t::COMMS_RELAY, _ => t::COMMS_NONE }}</span></div>
                </div>
                <div class="ht" id="ht-back" title=tip(t::TIP_BACK)>
                    <div class="ht-l">{t::LBL_BACK}</div>
                    <div class="ht-v"><span id="hud-back">{move || m().back.unwrap_or_else(|| t::NO_WAY_SHORT.to_string())}</span>
                        <small>{move || if m().back.is_some() { t::UNIT_MIN } else { "" }}</small></div>
                </div>
            </div>
            <div id="hud-note">{move || m().note}</div>
            // one line under the header: a simulation alert while recent, else the walk target / binocular range
            {move || match (sim.with(|s| s.alert.clone()), m().aim) {
                (Some(a), _) => view! { <div id="hud-alert" role="alert">{a}</div> }.into_any(),
                (None, aim) if !aim.is_empty() => view! { <div id="hud-aim" class=if m().aim_warn { "warn" } else { "" }>{aim}</div> }.into_any(),
                _ => ().into_any(),
            }}
            // the station the crew is at now: its plan (stops.json tasks) and the time left there (EVA simulation)
            {move || sim.with(|s| s.station.clone()).map(|st| view! {
                <div id="station-panel" role="status" aria-live="polite" class=move || if sp_open.get() { "open" } else { "closed" }>
                    <button type="button" class="sp-head" title=text::sim::SP_TOGGLE on:click=move |_| sp_open.update(|o| *o = !*o)>
                        {st.photo.clone().map(|p| view! { <img class="sp-thumb" src=p.src.clone() alt=""/> })}
                        <span class="sp-name">{if st.egress { text::sim::BLOCK_EGRESS.to_string() } else { st.name.clone() }}</span>
                        <span class="sp-left">{st.left.clone()}</span>
                        <span class="sp-caret" aria-hidden="true">{move || if sp_open.get() { "\u{25b4}" } else { "\u{25be}" }}</span></button>
                    {let pz = st.pano.clone(); move || (!sp_open.get()).then(|| pz.clone().map(|p| view! {
                        <button type="button" class="pano-open slim" title=text::sim::PANO_TIP
                            on:click=move |_| pano.set(Some((p.clone(), top.get_untracked().heading as f64)))>{text::sim::PANO_OPEN}</button>
                    }))}
                    {if !sp_open.get() { ().into_any() } else if st.egress {
                        view! { <p class="sp-note">{text::sim::EGRESS_NOTE}</p> }.into_any()
                    } else {
                        view! {
                            {st.photo.clone().map(|p| view! {
                                <figure class="sp-photo">
                                    <a href=p.src.clone() target="_blank" rel="noopener noreferrer" title=text::stop_card::OPEN_PHOTO>
                                        <img src=p.src.clone() alt=p.alt.clone()/>
                                        {p.az.map(|a| view! {
                                            <span class="view-arrow" role="img" aria-label=p.view.clone() title=text::stop_card::PHOTO_VIEW_TIP>
                                                <span class="n-tag">"N"</span><i style:transform=format!("rotate({a:.0}deg)")>"\u{2191}"</i>
                                            </span>
                                        })}
                                    </a>
                                    <figcaption><b>{text::stop_card::PHOTO_WALK}</b>" "{p.view.clone()}<br/><span class="sp-note">{p.caption.clone()}</span></figcaption>
                                </figure>
                            })}
                            {st.pano.clone().map(|p| view! {
                                <button type="button" class="pano-open" title=text::sim::PANO_TIP
                                    on:click=move |_| pano.set(Some((p.clone(), top.get_untracked().heading as f64)))>{text::sim::PANO_OPEN}</button>
                            })}
                            {st.navcam3d.then(|| view! { <p class="sp-3d" title=text::sim::NAVCAM3D_TIP>{text::sim::NAVCAM3D}</p> })}
                            <dl class="sp-plan">
                                {st.tasks.iter().enumerate().map(|(i, (step, txt, inf))| {
                                    // check-offs go to the same EVA log as the Log tab (task index in stops.json order)
                                    let q = *step == text::sim::STEP_QUESTION;
                                    let (id, id2) = (st.id.clone(), st.id.clone());
                                    view! {
                                        <dt>{(!q).then(|| view! {
                                            <input type="checkbox" aria-label=*step prop:checked=move || exec.with(|l| l.0.get(&id).and_then(|e| e.tasks.get(&i).copied()).unwrap_or(false))
                                                on:change=move |ev| { let on = event_target_checked(&ev); exec.update(|l| { l.0.entry(id2.clone()).or_default().tasks.insert(i, on); }); }/>
                                        })}{*step}</dt>
                                        <dd>{txt.clone()}{inf.then(|| view! { " " <span class="badge">{text::sim::INFERENCE}</span> })}</dd>
                                    }
                                }).collect_view()}
                            </dl>
                            <div class="sp-log">
                                {[(0u8, text::exec::ARRIVE), (1u8, text::exec::LEAVE)].into_iter().map(|(which, lab)| {
                                    let id = st.id.clone();
                                    view! { <button type="button" class="mini" title=text::exec::CLOCK_TIP on:click=move |_| {
                                        let t = sim.with_untracked(|s| s.clock.chars().take(5).collect::<String>());
                                        exec.update(|l| { let e = l.0.entry(id.clone()).or_default(); if which == 0 { e.arrive = t } else { e.leave = t } });
                                    }>{format!("{lab} {}", text::exec::NOW.to_lowercase())}</button> }
                                }).collect_view()}
                                <span class="sp-stamps">{
                                    let id = st.id.clone();
                                    move || exec.with(|l| l.0.get(&id).map(|e| format!("{} {} \u{b7} {} {}", text::exec::ARRIVE, e.arrive, text::exec::LEAVE, e.leave)).unwrap_or_default())
                                }</span>
                            </div>
                            <p class="sp-note">{if st.yours { text::edit::YOURS_TIP } else { text::sim::PLAN_NOTE }}</p>
                        }.into_any()
                    }}
                </div>
            })}
            {move || pano.get().map(|(p, h0)| pano_view(p, h0, pano))}
            <div id="eva-log" role="log" aria-label=text::sim::LOG>
                <div class="el-title">{text::sim::LOG}</div>
                {move || {
                    let s = sim.get();
                    let cur = s.current.unwrap_or(0);
                    let lo = cur.saturating_sub(3);
                    s.log.iter().enumerate().skip(lo).take(7).map(|(i, r)| {
                        let tt = r.t;
                        let cls = format!("{}{}", if i == cur { "el-row cur" } else if i < cur { "el-row past" } else { "el-row" },
                                          if r.alert { " alert" } else { "" });
                        view! { <button type="button" class=cls on:click=move |_| crate::plan::simrun::send(crate::plan::simrun::SimEvent::SeekTime(tt))>
                            <span class="el-t">{r.clock.clone()}</span><span class="el-x">{r.text.clone()}</span></button> }
                    }).collect_view()
                }}
            </div>
            <div id="minimap" title=t::MINIMAP>
                <div id="mm-next" style:left=move || format!("{:.2}%", m().map_next.0 * 100.0)
                    style:top=move || format!("{:.2}%", m().map_next.1 * 100.0)></div>
                <div id="mm-me" style:left=move || format!("{:.2}%", m().map_me.0 * 100.0)
                    style:top=move || format!("{:.2}%", m().map_me.1 * 100.0)
                    style:transform=move || format!("translate(-50%, -50%) rotate({}deg)", m().heading)></div>
            </div>
            <div id="hud-view" title=move || if m().third { t::VIEW_THIRD } else { t::VIEW_FIRST }>{move || if m().third { t::VIEW_THIRD_SHORT } else { t::VIEW_FIRST_SHORT }}</div>
        </div>
    }
}

/// Horizontal field of view of the panorama viewer (deg). VISUAL: about what a person sees ahead without turning.
const PANO_FOV_DEG: f64 = 90.0;

/// Curiosity's Navcam 360-degree mosaic as a full-window view, turned to `h0` (the walker's heading); drag or the arrow
/// buttons turn it. Cylindrical image shown with CSS only: no new GPU material (no shader compile when it opens).
pub(super) fn pano_view(p: crate::ui::model::Pano, h0: f64, open: RwSignal<Option<(crate::ui::model::Pano, f64)>>) -> impl IntoView {
    use text::sim as t;
    let yaw = RwSignal::new(h0);
    let drag = RwSignal::new(false);
    let vw_per_deg = 100.0 / PANO_FOV_DEG;
    let (p2, p3) = (p.clone(), p.clone());
    let style = move || {
        let shift = p2.shift_deg(yaw.get(), PANO_FOV_DEG);
        // horizon (elevation 0) at the middle of the viewer, which sits between the mission bar and the timeline
        format!("background-image:url('{}');background-size:{:.3}vw auto;background-position:{:.3}vw calc((100vh - var(--bar-h) - var(--tl-h)) / 2 - {:.3}vw)",
                p2.src, 360.0 * vw_per_deg, -shift * vw_per_deg, p2.el_max_deg * vw_per_deg)
    };
    let turn = move |d: f64| yaw.update(|y| *y = (*y + d).rem_euclid(360.0));
    let width = || web_sys::window().and_then(|w| w.inner_width().ok()).and_then(|v| v.as_f64()).unwrap_or(1280.0);
    view! {
        <div id="pano" role="dialog" aria-modal="true" aria-label=fill(t::PANO_TITLE, &[("sol", p.sol.to_string())]) style=style
            on:pointerdown=move |_| drag.set(true) on:pointerup=move |_| drag.set(false) on:pointerleave=move |_| drag.set(false)
            on:pointermove=move |ev: web_sys::PointerEvent| if drag.get_untracked() { turn(-(ev.movement_x() as f64) * PANO_FOV_DEG / width()) }>
            <div class="pano-bar" on:pointerdown=|ev: web_sys::PointerEvent| ev.stop_propagation()>
                <b>{fill(t::PANO_TITLE, &[("sol", p.sol.to_string())])}</b>
                <span class="pano-facing">{move || fill(t::PANO_FACING, &[("x", format!("{:03}", yaw.get().round() as i64 % 360))])}</span>
                <button type="button" class="mini" aria-label=t::PANO_LEFT on:click=move |_| turn(-30.0)>"\u{25c0}"</button>
                <button type="button" class="mini" aria-label=t::PANO_RIGHT on:click=move |_| turn(30.0)>"\u{25b6}"</button>
                <button type="button" on:click=move |_| open.set(None)>{t::PANO_CLOSE}</button>
            </div>
            <div class="pano-centre" aria-hidden="true"></div>
            <p class="pano-note" on:pointerdown=|ev: web_sys::PointerEvent| ev.stop_propagation()>{t::PANO_NOTE}" "
                <a href=p3.url.clone() target="_blank" rel="noopener noreferrer">{fill(t::PANO_CREDIT, &[("name", p3.product.clone())])}</a></p>
        </div>
    }
}
