//! Map symbol colours, defined once: the 3D lines and flags (Bevy colours), the legend swatches (SVG) and the HTML labels
//! (CSS variables --route, --flag, --rover on <body id="page">) all read them from here. Meaning never rests on colour
//! alone: each symbol also differs in shape or line style (legend).
use bevy::color::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

/// planned route (solid line)
pub const ROUTE: Rgb = Rgb(255, 217, 51);
/// stop flags, labels, profile stop marks
pub const FLAG: Rgb = Rgb(255, 115, 38);
/// Curiosity's path (dashed line and dots)
pub const ROVER: Rgb = Rgb(64, 217, 255);
/// route agreement shading (uncertainty runs; opacity carries the share, the legend says so)
pub const DENSITY: Rgb = Rgb(255, 77, 196);

impl Rgb {
    pub fn color(self) -> Color {
        Color::srgb_u8(self.0, self.1, self.2)
    }
    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }
}

/// CSS custom properties for the HTML overlay, so labels and the panel use the same colours as the 3D scene.
pub fn css_vars() -> String {
    format!("--route:{};--flag:{};--rover:{};--density:{}", ROUTE.hex(), FLAG.hex(), ROVER.hex(), DENSITY.hex())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_matches_the_bytes() {
        assert_eq!(FLAG.hex(), "#ff7326");
        assert_eq!(css_vars(), "--route:#ffd933;--flag:#ff7326;--rover:#40d9ff;--density:#ff4dc4");
    }

    #[test]
    fn page_and_stylesheet_take_symbol_colours_from_css_variables() {
        for f in ["index.html", "style.css"] {
            let s = std::fs::read_to_string(format!("{}/{f}", env!("CARGO_MANIFEST_DIR"))).unwrap().to_lowercase();
            for hex in [ROUTE.hex(), FLAG.hex(), ROVER.hex()] {
                assert!(!s.contains(&hex), "{hex} typed in {f}: use var(--route/--flag/--rover) instead");
            }
        }
    }

    /// The panel width is used by Rust (map framing, pick area) and by the stylesheet; they must agree.
    #[test]
    fn stylesheet_panel_width_matches_the_map_framing() {
        let css = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/style.css")).unwrap();
        for (var, px) in [("--rail-l", crate::scene::mapview::RAIL_L_PX), ("--rail-r", crate::scene::mapview::RAIL_R_PX), ("--bar-h", crate::scene::mapview::BAR_PX),
                          ("--tl-h", crate::scene::mapview::TIMELINE_PX)] {
            assert!(css.contains(&format!("{var}: {px}px")), "style.css {var} must equal mapview ({px}px)");
        }
    }
}
