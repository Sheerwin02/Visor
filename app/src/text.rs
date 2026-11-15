//! Every word the app shows, compiled in from text/en.json by build.rs (typed constants: `text::hud::LBL_DISTANCE`).
//! Numbers are never typed here: {placeholders} are filled with `fill` from the bundle or the live plan.
#![allow(dead_code)]
include!(concat!(env!("OUT_DIR"), "/text.rs"));

/// Fill a template such as "Loading {file} ({done} of {total})".
pub fn fill(template: &str, pairs: &[(&str, String)]) -> String {
    let mut s = template.to_string();
    for (k, v) in pairs {
        s = s.replace(&format!("{{{k}}}"), v);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_text_is_compiled_in_and_fills() {
        assert_eq!(hud::LBL_DISTANCE, "Distance");
        assert_eq!(fill(loading::FILE, &[("file", "heightmap.bin".into()), ("done", "2".into()), ("total", "5".into())]), "Loading heightmap.bin (2 of 5)");
        assert!(assumptions_panel::ALL.iter().any(|(k, _)| *k == "title"));
    }

    /// The page shows some text before the app runs (index.html); it must say exactly what text/en.json says.
    #[test]
    fn index_html_matches_the_text_file() {
        let html = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/index.html")).unwrap();
        for t in [loading::START, loading::NOSCRIPT] {
            assert!(html.contains(t), "index.html does not show: {t}");
        }
    }
}
