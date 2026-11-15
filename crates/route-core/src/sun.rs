//! Mars sun position: the Mars24 algorithm as printed on the NASA GISS page "Mars24 Sunclock - Algorithm and Worked
//! Examples" (updated 2025-01-07), steps A-6 to D-6. A line-for-line port of tools/mars_sun.py (the Python reference),
//! tested here against the same GISS worked examples and HiRISE label, and against the Python output on sample sols.
//! Conventions: longitude in degrees WEST, latitude PLANETOGRAPHIC, azimuth clockwise from north.
//! MSL sols: sol N starts when MSD + 137.42/360 = 49269 + N (see tools/mars_sun.py for the derivation and its assumption).

const PBS_TERMS: [(f64, f64, f64); 7] = [
    (0.0071, 2.2353, 49.409), (0.0057, 2.7543, 168.173), (0.0039, 1.1177, 191.837), (0.0037, 15.7866, 21.736),
    (0.0021, 2.1354, 15.704), (0.0020, 2.4694, 95.528), (0.0018, 32.8493, 49.095),
];
pub const MSL_SOL0_K: f64 = 49269.0;
pub const MSL_CLOCK_E: f64 = 137.42;
const MARS_A_KM: f64 = 3396.19;
const MARS_B_KM: f64 = 3376.20;

fn s(x: f64) -> f64 {
    x.to_radians().sin()
}

fn c(x: f64) -> f64 {
    x.to_radians().cos()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sun {
    pub ls: f64,
    pub eot: f64,
    pub mst: f64,
    pub ltst: f64,
    pub lam_s: f64,
    pub dec: f64,
    pub elevation: f64,
    pub azimuth: f64,
    pub r: f64,
}

/// TT minus UTC (s) for the dates VISOR uses (2006 onward; no leap second after 2016-12-31 per the GISS page).
pub fn tt_minus_utc(jd_ut: f64) -> f64 {
    let steps = [(2453736.5, 33.0), (2454832.5, 34.0), (2456109.5, 35.0), (2457204.5, 36.0), (2457754.5, 37.0)];
    let tai = steps.iter().filter(|(jd, _)| jd_ut >= *jd).map(|(_, v)| *v).fold(32.0, f64::max);
    tai + 32.184
}

/// The sun at a site, from the TT Julian date.
pub fn sun(jd_tt: f64, west_lon: f64, lat_pg: f64) -> Sun {
    let dt = jd_tt - 2451545.0;
    let m = 19.3871 + 0.52402073 * dt;
    let a_fms = 270.3871 + 0.524038496 * dt;
    let pbs: f64 = PBS_TERMS.iter().map(|&(a, tau, phi)| a * c(0.985626 * dt / tau + phi)).sum();
    let nu_m = (10.691 + 3.0e-7 * dt) * s(m) + 0.623 * s(2.0 * m) + 0.050 * s(3.0 * m) + 0.005 * s(4.0 * m) + 0.0005 * s(5.0 * m) + pbs;
    let ls = (a_fms + nu_m).rem_euclid(360.0);
    let eot = 2.861 * s(2.0 * ls) - 0.071 * s(4.0 * ls) + 0.002 * s(6.0 * ls) - nu_m;
    let msd = (jd_tt - 2451549.5) / 1.0274912517 + 44796.0 - 0.0009626;
    let mst = (24.0 * msd).rem_euclid(24.0);
    let r = 1.52367934 * (1.00436 - 0.09309 * c(m) - 0.004336 * c(2.0 * m) - 0.00031 * c(3.0 * m) - 0.00003 * c(4.0 * m));
    let lmst = (mst - west_lon / 15.0).rem_euclid(24.0);
    let ltst = (lmst + eot / 15.0).rem_euclid(24.0);
    let lam_s = (15.0 * mst + eot + 180.0).rem_euclid(360.0);
    let dec = (0.42565 * s(ls)).asin().to_degrees() + 0.25 * s(ls);
    let h = west_lon - lam_s;
    let z = (s(dec) * s(lat_pg) + c(dec) * c(lat_pg) * c(h)).clamp(-1.0, 1.0).acos().to_degrees();
    let az = s(h).atan2(c(lat_pg) * dec.to_radians().tan() - s(lat_pg) * c(h)).to_degrees().rem_euclid(360.0);
    Sun { ls, eot, mst, ltst, lam_s, dec, elevation: 90.0 - z, azimuth: az, r }
}

pub fn sun_utc(jd_ut: f64, west_lon: f64, lat_pg: f64) -> Sun {
    sun(jd_ut + tt_minus_utc(jd_ut) / 86400.0, west_lon, lat_pg)
}

pub fn jd_tt_from_msd(msd: f64) -> f64 {
    (msd - 44796.0 + 0.0009626) * 1.0274912517 + 2451549.5
}

/// The sun on MSL sol `sol` when the site's local true solar time is `ltst_h` (hours, 0..24).
pub fn sun_at_sol(sol: f64, ltst_h: f64, west_lon: f64, lat_pg: f64) -> Sun {
    let msd0 = MSL_SOL0_K + sol - MSL_CLOCK_E / 360.0;
    let site_e = 360.0 - west_lon;
    let at = |lmst: f64| msd0 + (lmst - (site_e - MSL_CLOCK_E) / 15.0) / 24.0;
    let mut target = ltst_h;
    for _ in 0..3 {
        let x = sun(jd_tt_from_msd(at(target)), west_lon, lat_pg);
        target = ltst_h - x.eot / 15.0;
    }
    sun(jd_tt_from_msd(at(target)), west_lon, lat_pg)
}

/// Gregorian UTC calendar date (year, month, day) of a Julian date (UT).
pub fn utc_date(jd_ut: f64) -> (i64, u32, u32) {
    let z = (jd_ut + 0.5).floor() as i64;
    let a = if z < 2299161 { z } else { let al = ((z as f64 - 1867216.25) / 36524.25).floor() as i64; z + 1 + al - al / 4 };
    let b = a + 1524;
    let c = ((b as f64 - 122.1) / 365.25).floor() as i64;
    let d = (365.25 * c as f64).floor() as i64;
    let e = ((b - d) as f64 / 30.6001).floor() as i64;
    let day = (b - d - (30.6001 * e as f64).floor() as i64) as u32;
    let month = if e < 14 { e - 1 } else { e - 13 } as u32;
    let year = if month > 2 { c - 4716 } else { c - 4715 };
    (year, month, day)
}

/// UTC date of local true noon on MSL sol `sol` at the site, and Ls then (for labels).
pub fn sol_noon(sol: f64, west_lon: f64, lat_pg: f64) -> ((i64, u32, u32), f64) {
    let site_e = 360.0 - west_lon;
    let mut target = 12.0;
    let msd0 = MSL_SOL0_K + sol - MSL_CLOCK_E / 360.0;
    let at = |lmst: f64| msd0 + (lmst - (site_e - MSL_CLOCK_E) / 15.0) / 24.0;
    for _ in 0..3 {
        target = 12.0 - sun(jd_tt_from_msd(at(target)), west_lon, lat_pg).eot / 15.0;
    }
    let jd_tt = jd_tt_from_msd(at(target));
    let jd_ut = jd_tt - tt_minus_utc(jd_tt) / 86400.0;
    (utc_date(jd_ut), sun(jd_tt, west_lon, lat_pg).ls)
}

/// Planetocentric to planetographic latitude (IAU 2000 Mars ellipsoid).
pub fn planetographic(lat_pc: f64) -> f64 {
    ((MARS_A_KM / MARS_B_KM).powi(2) * lat_pc.to_radians().tan()).atan().to_degrees()
}

/// Unit vector toward the sun in VISOR's map frame: x = east, y = north, z = up.
pub fn direction(elevation: f64, azimuth: f64) -> [f64; 3] {
    [c(elevation) * s(azimuth), c(elevation) * c(azimuth), s(elevation)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64, what: &str) {
        assert!((a - b).abs() <= tol, "{what}: {a} vs {b} (tol {tol})");
    }

    #[test]
    fn giss_example_1() {
        let e = sun_utc(2451549.5, 0.0, 0.0);
        close(e.ls, 277.18758, 1e-4, "Ls");
        close(e.eot, -5.18774, 1e-4, "EOT");
        close(e.mst, 23.99425, 1e-4, "MST");
        close(e.ltst, 23.64840, 1e-4, "LTST");
        close(e.lam_s, 174.72600, 1e-4, "subsolar longitude");
        close(e.r, 1.39358, 1e-4, "R");
        close(e.azimuth, 191.03905, 1e-4, "A");
        close(e.dec, -25.22825, 1e-3, "declination");
        close(90.0 - e.elevation, 154.26182, 1e-3, "Z");
    }

    #[test]
    fn giss_example_2_spirit() {
        let jd = 2440587.5 + 1073137591000.0 / 8.64e7;
        let e = sun_utc(jd, 184.702, -14.640);
        close(e.ls, 327.32416, 1e-4, "Ls");
        close(e.eot, -12.77553, 1e-4, "EOT");
        close(e.lam_s, 4.70500, 1e-4, "subsolar longitude");
        close(e.r, 1.47767, 1e-4, "R");
        close(e.azimuth, 179.99383, 1e-4, "A");
        close(e.ltst.min(24.0 - e.ltst), 0.0, 1e-3, "LTST midnight");
        close(e.dec, -13.42065, 1e-3, "declination");
        close(90.0 - e.elevation, 151.93895, 1e-3, "Z");
    }

    #[test]
    fn hirise_label_of_the_patch_strip() {
        // PSP_009149_1750: START_TIME 2008-07-09T09:24:54, Ls 96.272867, local time 15.48044, incidence 58.808048,
        // derived sun azimuth 303.1; centre -4.799 N, 137.412 E. JD of 2008-07-09 09:24:54 UTC = 2454656.89229...
        let jd = 2454656.5 + (9.0 + 24.0 / 60.0 + 54.0 / 3600.0) / 24.0;
        let h = sun_utc(jd, 360.0 - 137.412, planetographic(-4.799));
        close(h.ls, 96.272867, 0.01, "Ls");
        close(h.ltst, 15.48044, 0.02, "local time");
        close(h.elevation, 90.0 - 58.808048, 0.5, "elevation");
        close(h.azimuth, 303.1, 1.0, "azimuth");
    }

    #[test]
    fn matches_the_python_reference_on_sols() {
        // python tools/mars_sun.py table 4718 (site 222.620 W, -4.8623 planetographic)
        for (h, el, az) in [(7.0, 14.58, 84.8), (9.0, 44.22, 79.9), (12.0, 81.39, 360.0), (16.0, 29.45, 277.0)] {
            let x = sun_at_sol(4718.0, h, 222.620, -4.8623);
            close(x.ltst, h, 1e-4, "LTST round trip");
            close(x.elevation, el, 0.01, "elevation vs Python");
            let d = (x.azimuth - az).rem_euclid(360.0);
            close(d.min(360.0 - d), 0.0, 0.1, "azimuth vs Python");
        }
        close(planetographic(-4.814633), -4.87, 0.01, "planetographic latitude");
        assert_eq!(utc_date(2451549.5), (2000, 1, 6));
        // sol 4718 begins 2025-11-13 06:43 UTC (tools/mars_sun.py / research notes): local noon is later that day
        let ((y, m, d), ls) = sol_noon(4718.0, 222.62, -4.8623);
        assert_eq!((y, m, d), (2025, 11, 13));
        close(ls, 171.25, 0.01, "Ls at noon, sol 4718 (Python table)");
        let d = direction(90.0, 0.0);
        close(d[2], 1.0, 1e-12, "zenith");
    }
}
