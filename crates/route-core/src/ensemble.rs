//! Building blocks for the route uncertainty ensemble (route uncertainty rule, fixed in advance).
//! Deterministic: every random draw comes from `Rng` seeded per run, so results do not depend on thread count.

/// SplitMix64: small, fast, reproducible. Not for cryptography.
#[derive(Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        let mut r = Rng(seed);
        r.next_u64();
        r
    }
    /// A generator for one run of one ensemble, independent of the order runs are computed in.
    pub fn for_run(seed: u64, ensemble: u64, run: u64) -> Rng {
        Rng::new(seed ^ ensemble.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ run.wrapping_mul(0xbf58_476d_1ce4_e5b9))
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    /// Uniform in [0, 1).
    pub fn uniform(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.uniform()
    }
    /// Uniform integer in 0..n.
    pub fn below(&mut self, n: usize) -> usize {
        ((self.uniform() * n as f64) as usize).min(n - 1)
    }
    /// Standard normal (Box-Muller).
    pub fn normal(&mut self) -> f64 {
        let u1 = 1.0 - self.uniform(); // (0, 1]
        let u2 = self.uniform();
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }
}

/// Gaussian white noise smoothed with a Gaussian kernel of sigma = l/2 (correlation falls to 1/e at distance l), scaled
/// to unit variance. Generated on a padded grid and convolved fully, so the edges have the same statistics as the middle.
pub fn smooth_field(w: usize, h: usize, cell: f64, l_m: f64, rng: &mut Rng) -> Vec<f64> {
    let sigma = l_m / 2.0 / cell;
    let r = (3.0 * sigma).ceil() as usize;
    let mut k: Vec<f64> = (0..=2 * r).map(|i| (-((i as f64 - r as f64).powi(2)) / (2.0 * sigma * sigma)).exp()).collect();
    let s: f64 = k.iter().sum();
    k.iter_mut().for_each(|x| *x /= s);
    let var: f64 = k.iter().map(|x| x * x).sum::<f64>().powi(2); // variance of separable smoothing of unit white noise
    let (pw, ph) = (w + 2 * r, h + 2 * r);
    let noise: Vec<f64> = (0..pw * ph).map(|_| rng.normal()).collect();
    // rows: (ph x pw) -> (ph x w)
    let mut a = vec![0.0; ph * w];
    for y in 0..ph {
        for x in 0..w {
            a[y * w + x] = (0..=2 * r).map(|i| k[i] * noise[y * pw + x + i]).sum();
        }
    }
    // columns: (ph x w) -> (h x w)
    let sd = var.sqrt();
    let mut out = vec![0.0; h * w];
    for y in 0..h {
        for x in 0..w {
            out[y * w + x] = (0..=2 * r).map(|i| k[i] * a[(y + i) * w + x]).sum::<f64>() / sd;
        }
    }
    out
}

/// Standard normal CDF (Abramowitz and Stegun 7.1.26 erf, |error| < 1.5e-7).
pub fn phi(z: f64) -> f64 {
    let x = z.abs() / std::f64::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let y = 1.0 - (((((1.061_405_429 * t - 1.453_152_027) * t) + 1.421_413_741) * t - 0.284_496_736) * t + 0.254_829_592) * t * (-x * x).exp();
    if z >= 0.0 { 0.5 * (1.0 + y) } else { 0.5 * (1.0 - y) }
}

/// Quantile q of sorted values, linear interpolation between order statistics (numpy's default).
pub fn quantile_sorted(xs: &[f64], q: f64) -> f64 {
    assert!(!xs.is_empty());
    let pos = q.clamp(0.0, 1.0) * (xs.len() - 1) as f64;
    let (i, f) = (pos.floor() as usize, pos - pos.floor());
    if i + 1 < xs.len() { xs[i] + f * (xs[i + 1] - xs[i]) } else { xs[i] }
}

pub fn quantile(xs: &[f64], q: f64) -> f64 {
    let mut s = xs.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    quantile_sorted(&s, q)
}

pub fn median(xs: &[f64]) -> f64 {
    quantile(xs, 0.5)
}

/// Map unit-normal values onto an empirical distribution (sorted), through the normal CDF.
pub fn map_to_empirical(z: &mut [f64], sorted: &[f64]) {
    for v in z.iter_mut() {
        *v = quantile_sorted(sorted, phi(*v));
    }
}

/// Median of one moving-block bootstrap resample (blocks of `block` consecutive values, concatenated, cut to len).
pub fn block_bootstrap_median(d: &[f64], block: usize, rng: &mut Rng) -> f64 {
    let n = d.len();
    let block = block.min(n);
    let mut x = Vec::with_capacity(n + block);
    while x.len() < n {
        let s = rng.below(n - block + 1);
        x.extend_from_slice(&d[s..s + block]);
    }
    x.truncate(n);
    median(&x)
}

fn ranks(x: &[f64]) -> Vec<f64> {
    let mut idx: Vec<usize> = (0..x.len()).collect();
    idx.sort_by(|&a, &b| x[a].total_cmp(&x[b]));
    let mut r = vec![0.0; x.len()];
    let mut i = 0;
    while i < idx.len() {
        let mut j = i;
        while j + 1 < idx.len() && x[idx[j + 1]] == x[idx[i]] {
            j += 1;
        }
        let avg = (i + j) as f64 / 2.0 + 1.0; // ties share the mean rank
        for &k in &idx[i..=j] {
            r[k] = avg;
        }
        i = j + 1;
    }
    r
}

/// Spearman rank correlation (Pearson correlation of mean ranks).
pub fn spearman(x: &[f64], y: &[f64]) -> f64 {
    let (a, b) = (ranks(x), ranks(y));
    let n = a.len() as f64;
    let (ma, mb) = (a.iter().sum::<f64>() / n, b.iter().sum::<f64>() / n);
    let cov: f64 = a.iter().zip(&b).map(|(p, q)| (p - ma) * (q - mb)).sum();
    let va: f64 = a.iter().map(|p| (p - ma).powi(2)).sum();
    let vb: f64 = b.iter().map(|q| (q - mb).powi(2)).sum();
    cov / (va * vb).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_is_reproducible_and_uniform() {
        let (mut a, mut b) = (Rng::for_run(1, 2, 3), Rng::for_run(1, 2, 3));
        assert_eq!(a.next_u64(), b.next_u64());
        assert_ne!(Rng::for_run(1, 2, 3).next_u64(), Rng::for_run(1, 2, 4).next_u64());
        let mut r = Rng::new(7);
        let m = (0..100_000).map(|_| r.uniform()).sum::<f64>() / 1e5;
        assert!((m - 0.5).abs() < 0.01, "{m}");
        let z: Vec<f64> = (0..100_000).map(|_| r.normal()).collect();
        let v = z.iter().map(|x| x * x).sum::<f64>() / 1e5;
        assert!((v - 1.0).abs() < 0.02, "{v}");
    }

    #[test]
    fn field_has_unit_variance_and_the_stated_correlation_length() {
        // averaged over 8 fields: one 200 x 200 field with l = 10 has only a few hundred independent patches
        let (w, h, l) = (200, 200, 10.0);
        let lag = l as usize;
        let (mut vs, mut rhos) = (0.0, 0.0);
        for seed in 0..8 {
            let f = smooth_field(w, h, 1.0, l, &mut Rng::new(seed));
            let n = f.len() as f64;
            let m = f.iter().sum::<f64>() / n;
            let v = f.iter().map(|x| (x - m).powi(2)).sum::<f64>() / n;
            let (mut c, mut k) = (0.0, 0.0);
            for y in 0..h {
                for x in 0..w - lag {
                    c += (f[y * w + x] - m) * (f[y * w + x + lag] - m);
                    k += 1.0;
                }
            }
            vs += v / 8.0;
            rhos += c / k / v / 8.0;
        }
        assert!((vs - 1.0).abs() < 0.08, "variance {vs}");
        assert!((rhos - (-1.0f64).exp()).abs() < 0.05, "correlation at l: {rhos}");
    }

    #[test]
    fn helpers_match_known_values() {
        assert!((phi(0.0) - 0.5).abs() < 1e-7 && (phi(1.96) - 0.975).abs() < 1e-3);
        assert_eq!(quantile(&[1.0, 2.0, 3.0, 4.0], 0.5), 2.5);
        assert_eq!(quantile_sorted(&[0.0, 10.0], 0.25), 2.5);
        let mut z = vec![-10.0, 0.0, 10.0];
        map_to_empirical(&mut z, &[1.0, 2.0, 3.0]);
        assert!((z[0] - 1.0).abs() < 1e-6 && (z[1] - 2.0).abs() < 1e-6 && (z[2] - 3.0).abs() < 1e-6, "{z:?}");
        assert!((spearman(&[1.0, 2.0, 3.0], &[10.0, 20.0, 30.0]) - 1.0).abs() < 1e-12);
        assert!((spearman(&[1.0, 2.0, 3.0], &[3.0, 2.0, 1.0]) + 1.0).abs() < 1e-12);
        let mut r = Rng::new(3);
        let d: Vec<f64> = (0..85).map(|i| i as f64).collect();
        let m = block_bootstrap_median(&d, 8, &mut r);
        assert!((0.0..=84.0).contains(&m));
    }
}
