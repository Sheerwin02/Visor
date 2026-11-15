//! The HTML interface: models the Bevy side publishes (model.rs via UiBridge), the systems that build them (panel, card,
//! hud, chrome, pin, layer switches), the colour palette shared with style.css, and the Leptos views (web only).
pub mod brief;
pub mod card;
pub mod chrome;
pub mod contingency;
pub mod drill;
pub mod science;
pub mod execlog;
pub mod cuecard;
pub mod plans;
pub mod views_consts;
pub mod hud;
pub mod mars;
pub mod layerpanel;
pub mod model;
pub mod palette;
pub mod panel;
pub mod pin;
pub mod readiness;
#[cfg(target_arch = "wasm32")]
pub mod views;
