//! Day, twilight and night, from the Mars24 sun (route_core::sun) for the chosen sol.
//!
//! Twilight = the sun below the horizon but within TWILIGHT_H hours of sunrise or sunset. Source: NASA Science, "What Do
//! Sunrises and Sunsets Look Like on Mars?": "the twilight glow remains visible, but increasingly fainter, for up to two
//! hours before sunrise or after sunset" (attributed there to high-altitude dust). Sunrise and sunset come from the computed
//! sun (checked against Mars Trek's sun service, assets/sun_check.json). The light levels and sky colours derived from the
//! phase (`Daylight` fields) are VISUAL choices for the Walk scene, not a measured or modelled Martian sky.

/// Twilight length either side of sunrise / sunset (Mars hours). NASA Science: "up to two hours".
pub const TWILIGHT_H: f64 = 2.0;
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub const TWILIGHT_SOURCE: &str = "https://science.nasa.gov/solar-system/planets/mars/what-does-a-sunrise-sunset-look-like-on-mars/";
/// Sampling step of the elevation series (Mars hours): 5 minutes; crossings are interpolated between samples.
pub const STEP_H: f64 = 5.0 / 60.0;
/// Night light floor (share of daylight ambient). VISUAL: kept above black so the ground stays readable; no starlight or
/// moonlight model.
pub const NIGHT_FLOOR: f32 = 0.08;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Day,
    Twilight,
    Night,
}

/// Light state for the Walk scene at one moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Daylight {
    pub phase: Phase,
    /// hours to the nearest sunrise / sunset while the sun is down (0 in daylight, capped at TWILIGHT_H)
    pub from_horizon_h: f64,
    /// direct sunlight factor 0..1 (fades as the sun reaches the horizon). VISUAL.
    pub direct: f32,
    /// sky / ambient brightness 0..1 (NIGHT_FLOOR at night). VISUAL.
    pub sky: f32,
    /// how far the sky colour has moved from the butterscotch day toward the blue-grey twilight 0..1. VISUAL.
    pub blue: f32,
    /// star visibility 0..1. VISUAL (illustrative stars, not a star catalogue).
    pub stars: f32,
}

impl Daylight {
    pub const DAY: Daylight = Daylight { phase: Phase::Day, from_horizon_h: 0.0, direct: 1.0, sky: 1.0, blue: 0.0, stars: 0.0 };
}

fn smooth(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Light state from the sun's elevation (deg) and the hours to the nearest horizon crossing (when the sun is down).
pub fn light(el_deg: f64, from_horizon_h: f64) -> Daylight {
    if el_deg > 0.0 {
        // day: direct light fades in over the first degrees, the sky brightens up to a 20 deg sun; a low sun gives a
        // slight blue cast (Curiosity's blue sunsets)
        let low = 1.0 - smooth(0.0, 12.0, el_deg);
        return Daylight { phase: Phase::Day, from_horizon_h: 0.0, direct: smooth(-0.5, 5.0, el_deg) as f32,
                          sky: (0.55 + 0.45 * smooth(0.0, 20.0, el_deg)) as f32, blue: (0.35 * low) as f32, stars: 0.0 };
    }
    let dt = from_horizon_h.clamp(0.0, TWILIGHT_H);
    let fade = 1.0 - dt / TWILIGHT_H; // 1 at the horizon crossing, 0 at the end of twilight
    let phase = if from_horizon_h <= TWILIGHT_H { Phase::Twilight } else { Phase::Night };
    // "increasingly fainter": brightness falls with the square of the fade
    let sky = (0.55 * fade * fade).max(NIGHT_FLOOR as f64) as f32;
    Daylight { phase, from_horizon_h: dt, direct: 0.0, sky, blue: (0.35 + 0.65 * smooth(0.0, 0.3, 1.0 - fade)) as f32,
               stars: smooth(0.4, 0.95, 1.0 - fade) as f32 }
}

/// Sun elevations every STEP_H from h0 (LTST hours; may run past 24), and the horizon crossings found in them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Series {
    pub h0: f64,
    pub el: Vec<f64>,
    pub crossings: Vec<f64>,
}

impl Series {
    /// Sample `el_of(hour)` from h0 to h1 (inclusive, STEP_H apart).
    pub fn new(h0: f64, h1: f64, el_of: impl Fn(f64) -> f64) -> Series {
        let n = (((h1 - h0) / STEP_H).ceil() as usize).max(1) + 1;
        let el: Vec<f64> = (0..n).map(|k| el_of(h0 + k as f64 * STEP_H)).collect();
        let crossings = el.windows(2).enumerate().filter(|(_, w)| (w[0] > 0.0) != (w[1] > 0.0)).map(|(k, w)| {
            h0 + (k as f64 + w[0] / (w[0] - w[1])) * STEP_H
        }).collect();
        Series { h0, el, crossings }
    }

    pub fn el_at(&self, h: f64) -> f64 {
        let x = ((h - self.h0) / STEP_H).clamp(0.0, (self.el.len() - 1) as f64);
        let k = (x.floor() as usize).min(self.el.len().saturating_sub(2));
        let f = x - k as f64;
        match (self.el.get(k), self.el.get(k + 1)) {
            (Some(a), Some(b)) => a + (b - a) * f,
            (Some(a), None) => *a,
            _ => f64::NAN,
        }
    }

    /// Hours from h to the nearest horizon crossing (infinite when there is none in the series).
    pub fn from_horizon_h(&self, h: f64) -> f64 {
        self.crossings.iter().map(|c| (c - h).abs()).fold(f64::INFINITY, f64::min)
    }

    /// Sunrise and sunset hours (0..24) in this series: a crossing with the sun up just after it is a sunrise.
    pub fn rise_set(&self) -> (Option<f64>, Option<f64>) {
        let up_after = |c: f64| self.el_at(c + STEP_H * 0.5) > 0.0;
        (self.crossings.iter().copied().find(|c| up_after(*c)).map(|c| c.rem_euclid(24.0)),
         self.crossings.iter().copied().find(|c| !up_after(*c)).map(|c| c.rem_euclid(24.0)))
    }

    /// Light state at hour h, given the sun elevation at h (the exact value, not the interpolated one).
    pub fn at(&self, h: f64, el_deg: f64) -> Daylight {
        light(el_deg, if el_deg > 0.0 { 0.0 } else { self.from_horizon_h(h) })
    }

    /// Runs of twilight and night between hours h0 and h1: (start h, end h, phase), on the sampling grid.
    pub fn dark_runs(&self, h0: f64, h1: f64) -> Vec<(f64, f64, Phase)> {
        let mut out: Vec<(f64, f64, Phase)> = Vec::new();
        let mut h = h0;
        while h < h1 - 1e-9 {
            let h2 = (h + STEP_H).min(h1);
            let mid = 0.5 * (h + h2);
            let p = self.at(mid, self.el_at(mid)).phase;
            if p != Phase::Day {
                match out.last_mut() { Some(r) if r.2 == p && (r.1 - h).abs() < 1e-9 => r.1 = h2, _ => out.push((h, h2, p)) }
            }
            h = h2;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // a toy sun: rises at 6, sets at 18, 15 deg per hour
    fn toy(h: f64) -> f64 { 90.0 - 15.0 * (h - 12.0).abs() }

    #[test]
    fn phases_follow_the_two_hour_twilight_rule() {
        let s = Series::new(0.0, 30.0, toy);
        assert_eq!(s.crossings.len(), 2, "{:?}", s.crossings);
        assert!((s.crossings[0] - 6.0).abs() < 1e-6 && (s.crossings[1] - 18.0).abs() < 1e-6);
        let (rise, set) = s.rise_set();
        assert!((rise.unwrap() - 6.0).abs() < 1e-6 && (set.unwrap() - 18.0).abs() < 1e-6);
        assert_eq!(s.at(12.0, toy(12.0)).phase, Phase::Day);
        assert_eq!(s.at(19.0, toy(19.0)).phase, Phase::Twilight);
        assert_eq!(s.at(19.99, toy(19.99)).phase, Phase::Twilight);
        assert_eq!(s.at(20.5, toy(20.5)).phase, Phase::Night);
        assert_eq!(s.at(4.5, toy(4.5)).phase, Phase::Twilight, "before sunrise too");
        assert_eq!(s.at(3.0, toy(3.0)).phase, Phase::Night);
    }

    #[test]
    fn light_dims_monotonically_into_the_night() {
        let s = Series::new(10.0, 24.0, toy);
        let mut last = f32::INFINITY;
        for k in 0..=60 {
            let h = 14.0 + k as f64 * 0.1;
            let d = s.at(h, toy(h));
            assert!(d.sky <= last + 1e-6, "sky brightened at {h}");
            last = d.sky;
            assert!((0.0..=1.0).contains(&d.direct) && (0.0..=1.0).contains(&d.stars) && (0.0..=1.0).contains(&d.blue));
        }
        let night = s.at(23.0, toy(23.0));
        assert_eq!(night.direct, 0.0);
        assert!((night.sky - NIGHT_FLOOR).abs() < 1e-6 && night.stars > 0.99);
        assert_eq!(s.at(12.0, toy(12.0)).stars, 0.0);
    }

    #[test]
    fn dark_runs_cover_the_evening() {
        let s = Series::new(10.0, 24.0, toy);
        let r = s.dark_runs(14.0, 22.0);
        assert_eq!(r.len(), 2, "{r:?}");
        assert_eq!(r[0].2, Phase::Twilight);
        assert!((r[0].0 - 18.0).abs() <= STEP_H && (r[0].1 - 20.0).abs() <= STEP_H);
        assert_eq!(r[1].2, Phase::Night);
        assert!((r[1].1 - 22.0).abs() < 1e-9);
        assert!(s.dark_runs(9.0, 17.0).is_empty());
    }

    #[test]
    fn real_sun_at_gale_has_one_sunrise_and_sunset_per_sol() {
        // Gale, sol 4711 (Nevado Sajama drill sol); site as the planner uses it: west longitude, planetographic latitude
        let (west, lat) = (360.0 - 137.38, route_core::sun::planetographic(-4.80));
        let s = Series::new(0.0, 24.0, |h| route_core::sun::sun_at_sol(4711.0, h, west, lat).elevation);
        assert_eq!(s.crossings.len(), 2, "{:?}", s.crossings);
        assert!(s.crossings[0] > 5.0 && s.crossings[0] < 7.0 && s.crossings[1] > 17.0 && s.crossings[1] < 19.0, "{:?}", s.crossings);
        // the EVA's usual 08:00 to 14:00 is daylight
        assert!(s.dark_runs(8.0, 14.0).is_empty());
    }

    #[test]
    fn sunrise_and_sunset_match_the_rems_feed_on_the_mission_clock() {
        // the REMS feed (assets/conditions.json) gives sunrise / sunset on Curiosity's clock = local MEAN solar time;
        // ours are local TRUE solar time: LMST = LTST - EOT. Compare every sol with both times, to the minute the feed rounds to.
        let path = format!("{}/../assets/conditions.json", env!("CARGO_MANIFEST_DIR"));
        let doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let (west, lat) = (360.0 - 137.38, route_core::sun::planetographic(-4.80));
        let hm = |s: &str| -> Option<f64> { let (h, m) = s.split_once(':')?; Some(h.parse::<f64>().ok()? + m.parse::<f64>().ok()? / 60.0) };
        let (mut n, mut worst) = (0, 0.0f64);
        for r in doc["sols"].as_array().unwrap() {
            let (Some(sol), Some(fr), Some(fs)) = (r["sol"].as_f64(), r["sunrise"].as_str().and_then(hm), r["sunset"].as_str().and_then(hm)) else { continue };
            let s = Series::new(0.0, 24.0, |h| route_core::sun::sun_at_sol(sol, h, west, lat).elevation);
            let (rise, set) = s.rise_set();
            let eot_h = route_core::sun::sun_at_sol(sol, 12.0, west, lat).eot / 15.0;
            for (ours, feed) in [(rise.unwrap(), fr), (set.unwrap(), fs)] {
                worst = worst.max(((ours - eot_h) - feed).abs() * 60.0);
            }
            n += 1;
        }
        eprintln!("sunrise/sunset vs REMS feed (LMST): worst {worst:.2} min over {n} sols");
        assert!(n > 300, "{n} sols");
        // a few minutes: the feed's own definition of sunrise (disc edge, refraction) is not stated
        assert!(worst < 5.0, "worst sunrise / sunset difference {worst:.1} min over {n} sols");
    }
}
