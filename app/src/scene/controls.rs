//! Key bindings in one place: actions are named once and bound to keys here, so the rest of the app asks
//! "was SwitchMode pressed?" rather than naming keys. Two paths, chosen per action:
//! - HELD actions (walking, Shift) use leafwing-input-manager, which samples "is the key down?" once per frame.
//! - TAP actions (switch mode, face next stop, layer toggles) use Bevy's key events (ButtonInput::just_pressed), which
//!   keep a press even when it is released within the same frame. Leafwing's per-frame sampling lost repeated taps at
//!   about 1 frame per second in headless Chrome; a slow phone could lose quick taps the same way.
//! Mouse look and orbit stay on Bevy's raw mouse input.
//! The on-screen mode button (index.html #mode-btn) is a second way to switch mode that needs no keyboard.
use std::sync::atomic::{AtomicBool, Ordering};

use bevy::prelude::*;
use leafwing_input_manager::prelude::*;

#[derive(Actionlike, PartialEq, Eq, Clone, Copy, Hash, Debug, Reflect)]
pub enum Act {
    SwitchMode,
    Forward,
    Back,
    Left,
    Right,
    Fast,
    FaceNext,
    LayerRoute,
    LayerStops,
    LayerRover,
    HideHud,
    ToggleKeys,
    ToggleView,
    LayerSlope,
    LayerChecks,
    LayerVerified,
    LayerDensity,
    LayerShadow,
    LayerChem,
    LayerReady,
    /// walk-back time lines to the exit (Plan)
    LayerBack,
    Camera,
}

/// Tap bindings, read from key events. Tab and M both switch mode: M is a fallback that no browser treats as focus navigation.
pub const TAPS: &[(Act, KeyCode)] = &[
    (Act::SwitchMode, KeyCode::Tab),
    (Act::SwitchMode, KeyCode::KeyM),
    (Act::FaceNext, KeyCode::KeyN),
    (Act::LayerRoute, KeyCode::Digit5),
    (Act::LayerStops, KeyCode::Digit6),
    (Act::LayerRover, KeyCode::Digit7),
    (Act::HideHud, KeyCode::KeyH),
    (Act::ToggleKeys, KeyCode::KeyK),
    (Act::ToggleView, KeyCode::KeyV),
    (Act::LayerSlope, KeyCode::Digit2),
    (Act::LayerChecks, KeyCode::Digit3),
    (Act::LayerVerified, KeyCode::Digit4),
    (Act::LayerDensity, KeyCode::Digit8),
    (Act::LayerShadow, KeyCode::Digit9),
    (Act::LayerChem, KeyCode::Minus),
    (Act::LayerReady, KeyCode::Digit0),
    (Act::LayerBack, KeyCode::Equal),
    (Act::Camera, KeyCode::KeyC),
];

/// True in the frame a key bound to tap action `act` went down (kept even if it was released in the same frame).
pub fn tapped(keys: &ButtonInput<KeyCode>, act: Act) -> bool {
    TAPS.iter().any(|&(a, k)| a == act && keys.just_pressed(k))
}

/// Held bindings (leafwing).
pub fn input_map() -> InputMap<Act> {
    InputMap::default()
        .with(Act::Forward, KeyCode::KeyW)
        .with(Act::Forward, KeyCode::ArrowUp)
        .with(Act::Back, KeyCode::KeyS)
        .with(Act::Back, KeyCode::ArrowDown)
        .with(Act::Left, KeyCode::KeyA)
        .with(Act::Left, KeyCode::ArrowLeft)
        .with(Act::Right, KeyCode::KeyD)
        .with(Act::Right, KeyCode::ArrowRight)
        .with(Act::Fast, KeyCode::ShiftLeft)
        .with(Act::Fast, KeyCode::ShiftRight)
}

pub fn spawn_controls(mut commands: Commands) {
    commands.spawn((input_map(), Name::new("controls")));
}

/// Set by the mode button's click handler (web), read once per frame by `mode_button_clicked`.
static MODE_BUTTON: AtomicBool = AtomicBool::new(false);

/// Keys pressed by the web UI (layer switches, view switch): injected into Bevy's keyboard input for one frame, so the
/// layer systems read them exactly like a key press (one code path for keyboard and mouse).
static UI_KEYS: std::sync::Mutex<Vec<KeyCode>> = std::sync::Mutex::new(Vec::new());

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))] // called by the web UI
pub fn ui_press(act: Act) {
    if let Some(&(_, k)) = TAPS.iter().find(|(a, _)| *a == act) {
        if let Ok(mut q) = UI_KEYS.lock() {
            q.push(k);
        }
    }
}

/// PreUpdate, after Bevy's input systems: release last frame's injected keys, press the new ones.
pub fn inject_ui_keys(mut keys: ResMut<ButtonInput<KeyCode>>, mut held: Local<Vec<KeyCode>>) {
    for k in held.drain(..) {
        keys.release(k);
    }
    for k in UI_KEYS.lock().map(|mut q| q.drain(..).collect::<Vec<_>>()).unwrap_or_default() {
        keys.press(k);
        held.push(k);
    }
}

/// True once per click of the on-screen mode button.
pub fn mode_button_clicked() -> bool {
    MODE_BUTTON.swap(false, Ordering::Relaxed)
}

/// Called by the mode button (ui/views/mission_bar.rs); `switch_mode` reads it on the next frame.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn press_mode_button() {
    MODE_BUTTON.store(true, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::walk::{switch_mode, Mode};
    use bevy::input::InputPlugin;

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, InputPlugin, InputManagerPlugin::<Act>::default()))
            .init_resource::<Mode>()
            .add_systems(Startup, spawn_controls)
            .add_systems(Update, switch_mode);
        app.update();
        app
    }

    fn tap(app: &mut App, key: KeyCode) {
        key.press(app.world_mut());
        app.update();
        key.release(app.world_mut());
        app.update();
    }

    #[test]
    fn tab_and_m_both_switch_mode_and_other_keys_do_not() {
        let mut app = app();
        assert_eq!(*app.world().resource::<Mode>(), Mode::Plan);
        tap(&mut app, KeyCode::KeyM);
        assert_eq!(*app.world().resource::<Mode>(), Mode::Walk, "M switches to Walk");
        tap(&mut app, KeyCode::Tab);
        assert_eq!(*app.world().resource::<Mode>(), Mode::Plan, "Tab switches back to Plan");
        tap(&mut app, KeyCode::KeyW);
        assert_eq!(*app.world().resource::<Mode>(), Mode::Plan, "W does not switch mode");
    }

    #[test]
    fn holding_the_key_switches_once() {
        let mut app = app();
        KeyCode::KeyM.press(app.world_mut());
        for _ in 0..5 {
            app.update();
        }
        assert_eq!(*app.world().resource::<Mode>(), Mode::Walk, "one switch per press, not per frame");
    }

    #[test]
    fn every_action_has_exactly_one_kind_of_binding() {
        let map = input_map();
        for a in [Act::SwitchMode, Act::Forward, Act::Back, Act::Left, Act::Right, Act::Fast, Act::FaceNext,
                  Act::LayerRoute, Act::LayerStops, Act::LayerRover, Act::HideHud, Act::ToggleKeys, Act::ToggleView,
                  Act::LayerSlope, Act::LayerChecks, Act::LayerVerified, Act::LayerDensity, Act::LayerShadow, Act::LayerReady, Act::LayerChem, Act::LayerBack, Act::Camera] {
            let held = map.get(&a).is_some_and(|b| !b.is_empty());
            let tap = TAPS.iter().any(|&(t, _)| t == a);
            assert!(held != tap, "{a:?}: held {held}, tap {tap} (needs exactly one)");
        }
    }

    /// The failure seen in headless Chrome: a press and release inside one frame must still count.
    #[test]
    fn a_tap_inside_one_frame_still_switches() {
        let mut app = app();
        KeyCode::KeyM.press(app.world_mut());
        KeyCode::KeyM.release(app.world_mut());
        app.update();
        assert_eq!(*app.world().resource::<Mode>(), Mode::Walk);
    }

    /// Release and press again inside one frame (two quick taps): the second tap must count.
    #[test]
    fn repeated_taps_across_one_frame_each_count() {
        let mut app = app();
        KeyCode::KeyM.press(app.world_mut());
        app.update();
        KeyCode::KeyM.release(app.world_mut());
        KeyCode::KeyM.press(app.world_mut());
        app.update();
        assert_eq!(*app.world().resource::<Mode>(), Mode::Plan, "two taps: Walk then back to Plan");
    }
}
