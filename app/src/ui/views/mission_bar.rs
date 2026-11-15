//! Mission bar (top): sol, start hour, verdict, key numbers, share link, mode button; and the mode chip.

use crate::scene::controls::press_mode_button;
use crate::ui::model::{ChromeModel, LightModel, PanelModel};
use crate::text::{self, fill, ui as u};

use super::*;

#[component]
pub(super) fn ModeName(chrome: RwSignal<ChromeModel>) -> impl IntoView {
    use text::modes;
    move || {
        let c = chrome.get();
        if c.walking { modes::WALK } else if c.in_map { modes::MAP } else { modes::ORBIT }
    }
}

pub(super) fn stat(label: &'static str, value: String, unit: &'static str, sub: Option<String>, tip: String) -> impl IntoView {
    view! {
        <div class="mb-stat" title=tip>
            <div class="mb-stat-l">{label}</div>
            <div class="mb-stat-v">{value}<small>{unit}</small></div>
            {sub.map(|s| view! { <div class="mb-stat-s">{s}</div> })}
        </div>
    }
}

#[component]
pub(super) fn MissionBar(panel: RwSignal<Option<PanelModel>>, light: RwSignal<LightModel>, chrome: RwSignal<ChromeModel>) -> impl IntoView {
    use crate::ui::hud::minutes;
    use crate::plan::sunlight::{send as light_send, LightEvent};
    // only what the sol field and the start list show: other light updates (sun text, shadow mask) must not rebuild the
    // <select>, because a rebuild closes an open dropdown
    let when_data = Memo::new(move |_| {
        let l = light.get();
        let mut l2 = LightModel { sol: l.sol, sol_range: l.sol_range, earth_date: l.earth_date.clone(), band: l.band, ..Default::default() };
        l2.rows = l.rows.iter().map(|r| crate::ui::model::LightRow { hour: r.hour, label: r.label.clone(), passes: r.passes, best: r.best,
                                                                     selected: r.selected, ..Default::default() }).collect();
        l2
    });
    let when = move || {
        let l = when_data.get();
        if l.earth_date.is_empty() {
            return ().into_any();
        }
        let tip_sol = fill(u::SOL_TIP, &[("sol_first", format!("{}", l.sol_range.0 as i64)), ("sol_last", format!("{}", l.sol_range.1 as i64))]);
        let tip_start = fill(u::START_TIP, &[("lo", format!("{}", l.band.0)), ("hi", format!("{}", l.band.1))]);
        view! {
            <div class="mb-when">
                <label class="mb-field" title=tip_sol>
                    <span>{u::SOL}</span>
                    <input class="sol-in" type="number" min=l.sol_range.0.to_string() max=l.sol_range.1.to_string() step="1"
                        prop:value=format!("{}", l.sol as i64)
                        on:change=move |ev| { if let Ok(x) = event_target_value(&ev).parse::<f64>() { light_send(LightEvent::Sol(x)) } }/>
                    <em title=u::EARTH_DATE_TIP>{fill(u::EARTH_DATE, &[("date", l.earth_date.clone())])}</em>
                </label>
                <label class="mb-field" title=tip_start>
                    <span>{u::START}</span>
                    <select class="start-in" on:change=move |ev| { if let Ok(h) = event_target_value(&ev).parse::<f64>() { light_send(LightEvent::Hour(h)) } }>
                        {l.rows.iter().map(|r| view! {
                            <option value=format!("{}", r.hour) selected=r.selected>
                                {format!("{} {}", r.label, if r.best { "\u{2605}" } else if r.passes { "\u{2713}" } else { "" })}
                            </option>
                        }).collect_view()}
                    </select>
                </label>
            </div>
        }.into_any()
    };
    let summary = move || {
        let Some(m) = panel.get() else { return ().into_any() };
        let runs = runs_of(&m);
        let range = m.time_range.map(|(lo, hi)| fill(u::ST_RANGE, &[("lo", minutes(lo as f32)), ("hi", minutes(hi as f32)), ("runs", runs.clone())]));
        // how sure: the run count and whether the percentiles settled go into the tooltip, not the bar
        let walk_tip = format!("{} {}{}", fill(text::plan_panel::TILE_TIME_NOTE, &m.vals), fill(u::ST_RANGE_TIP, &[("runs", runs.clone())]),
                               if m.range_unsettled { format!(" {}", fill(text::ops::UNSETTLED_TIP, &[("runs", runs.clone())])) } else { String::new() });
        view! {
            <div class=if m.feasible { "mb-verdict ok" } else { "mb-verdict no" } title=u::VERDICT_TIP>
                {if m.feasible { "\u{2713} " } else { "\u{2717} " }}{if m.feasible { u::WALKABLE } else { u::BLOCKED }}
            </div>
            <div class=if m.feasible { "mb-stats" } else { "mb-stats stale" }
                title=(!m.feasible).then(|| fill(u::STATS_PARTIAL_TIP, &[("legs", m.blocked_label())])).unwrap_or_default()>
                {(!m.feasible).then(|| view! { <div class="mb-partial">{u::STATS_PARTIAL}</div> })}
                {stat(u::ST_WALK, minutes(m.walk_s as f32), " min", range, walk_tip)}
                {stat(u::ST_LOOP, format!("{}", m.length_m.round() as i64), " m", None, String::new())}
                {stat(u::ST_BACK, minutes(m.back_s as f32), " min", None, fill(text::plan_panel::TILE_BACK_NOTE, &[("stop_name", m.back_stop.clone())]))}
                {stat(u::ST_STEEP, format!("{}", m.worst_deg.round() as i64), "\u{b0}", None, fill(text::plan_panel::TILE_SLOPE_NOTE, &[("leg", m.worst_leg.clone())]))}
            </div>
        }.into_any()
    };
    view! {
        <div class="mb-brand"><span class="brand">{text::app::TITLE}</span><span class="site">{u::SITE}</span></div>
        {when}
        {summary}
        <div class="mb-actions">
            <button type="button" class="ghost" title=text::share::HINT
                on:click=move |_| crate::plan::share::act(crate::plan::share::ShareAction::CopyLink)>{u::SHARE}</button>
            <button id="mode-btn" type="button" class="primary"
                disabled=move || !chrome.get().walking && panel.with(|p| p.as_ref().is_some_and(|m| !m.feasible))
                title=move || if !chrome.get().walking && panel.with(|p| p.as_ref().is_some_and(|m| !m.feasible)) { u::WALK_BLOCKED_TIP } else { "" }
                on:click=move |_| {
                press_mode_button();
                if let Some(c) = host("visor") { let _ = c.focus(); }
            }>{move || if chrome.get().walking { u::TO_PLAN } else { u::TO_WALK }}</button>
        </div>
    }
}

// ================================================================= layer rail
