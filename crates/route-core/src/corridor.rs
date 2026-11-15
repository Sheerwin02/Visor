//! Route-to-route comparison for the corridor rule (corridor rule, fixed in advance).
//! Buffer method (Goodchild and Hunter 1997, as in the leastcostpath R package): share of a route's length within a distance
//! of a reference route. Path Deviation Index (Jan et al., as in leastcostpath): area between two routes with the same ends,
//! divided by the straight-line distance between those ends.

type P = (f64, f64);

/// Points every `step` metres along a polyline, each standing for `step` metres of length (the last piece for its own length).
/// Returns (point, weight) pairs; the weights sum to the polyline length.
pub fn resample(poly: &[P], step: f64) -> Vec<(P, f64)> {
    let mut out = Vec::new();
    for w in poly.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        if len == 0.0 {
            continue;
        }
        let n = (len / step).ceil() as usize;
        let piece = len / n as f64;
        for k in 0..n {
            let t = (k as f64 + 0.5) / n as f64; // midpoint of each piece
            out.push(((a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1)), piece));
        }
    }
    out
}

fn seg_dist(p: P, a: P, b: P) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l2 = dx * dx + dy * dy;
    let t = if l2 == 0.0 { 0.0 } else { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / l2).clamp(0.0, 1.0) };
    ((p.0 - a.0 - t * dx).powi(2) + (p.1 - a.1 - t * dy).powi(2)).sqrt()
}

/// Shortest distance from a point to a polyline.
pub fn dist_to_polyline(p: P, poly: &[P]) -> f64 {
    if poly.len() == 1 {
        return seg_dist(p, poly[0], poly[0]);
    }
    poly.windows(2).map(|w| seg_dist(p, w[0], w[1])).fold(f64::INFINITY, f64::min)
}

/// Share of `route`'s length within each distance of `reference` (sampled every `step` m).
pub fn buffer_shares(route: &[P], reference: &[P], dists: &[f64], step: f64) -> Vec<f64> {
    let s = resample(route, step);
    let total: f64 = s.iter().map(|(_, w)| w).sum();
    if total == 0.0 {
        return vec![1.0; dists.len()]; // a zero-length route lies on its reference
    }
    let d: Vec<(f64, f64)> = s.iter().map(|&(p, w)| (dist_to_polyline(p, reference), w)).collect();
    dists.iter().map(|&r| d.iter().filter(|(x, _)| *x <= r).map(|(_, w)| w).sum::<f64>() / total).collect()
}

/// Area enclosed between two routes with the same ends: the closed ring a + reversed b, filled by the even-odd rule
/// (crossing routes give separate lobes, each counted once). Scanline integration, rows `dy` high.
pub fn enclosed_area(a: &[P], b: &[P], dy: f64) -> f64 {
    let mut ring: Vec<P> = a.to_vec();
    ring.extend(b.iter().rev());
    if ring.len() < 3 {
        return 0.0;
    }
    let (y_lo, y_hi) = ring.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), p| (l.min(p.1), h.max(p.1)));
    let rows = ((y_hi - y_lo) / dy).ceil() as usize;
    let n = ring.len();
    let mut area = 0.0;
    let mut xs = Vec::new();
    for r in 0..rows {
        let y = y_lo + (r as f64 + 0.5) * dy;
        xs.clear();
        for i in 0..n {
            let (p, q) = (ring[i], ring[(i + 1) % n]);
            if (p.1 <= y) != (q.1 <= y) {
                xs.push(p.0 + (y - p.1) / (q.1 - p.1) * (q.0 - p.0));
            }
        }
        xs.sort_by(|u, v| u.total_cmp(v));
        for pair in xs.chunks_exact(2) {
            area += (pair[1] - pair[0]) * dy;
        }
    }
    area
}

/// Path Deviation Index: (area between the routes / straight-line distance between the ends, in metres;
/// the same as a percentage of that distance). None when the ends coincide.
pub fn pdi(route: &[P], reference: &[P], dy: f64) -> Option<(f64, f64)> {
    let (s, e) = (*reference.first()?, *reference.last()?);
    let d = ((e.0 - s.0).powi(2) + (e.1 - s.1).powi(2)).sqrt();
    if d == 0.0 {
        return None;
    }
    let v = enclosed_area(route, reference, dy) / d;
    Some((v, v / d * 100.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_routes_agree_fully() {
        let a = vec![(0.5, 0.5), (1.5, 1.5), (5.5, 1.5)];
        assert_eq!(buffer_shares(&a, &a, &[0.0, 1.0], 0.25), vec![1.0, 1.0]);
        assert!(pdi(&a, &a, 0.25).unwrap().0.abs() < 1e-9);
    }

    #[test]
    fn detour_two_metres_off() {
        let a = vec![(0.0, 0.0), (10.0, 0.0)];
        let b = vec![(0.0, 0.0), (0.0, 2.0), (10.0, 2.0), (10.0, 0.0)]; // 14 m long, 2 m off for 10 m
        let s = buffer_shares(&b, &a, &[1.0, 3.0], 0.25);
        assert!((s[0] - 2.0 / 14.0).abs() < 1e-9, "{s:?}"); // only the lower halves of the two 2 m legs
        assert!((s[1] - 1.0).abs() < 1e-12);
        let (p, pct) = pdi(&b, &a, 0.25).unwrap();
        assert!((p - 2.0).abs() < 1e-9 && (pct - 20.0).abs() < 1e-9, "{p} {pct}");
    }

    #[test]
    fn crossing_routes_count_both_lobes() {
        let a = vec![(0.0, 0.0), (10.0, 0.0)];
        let b = vec![(0.0, 0.0), (2.5, 1.0), (7.5, -1.0), (10.0, 0.0)]; // crosses a at x = 5
        let area = enclosed_area(&b, &a, 0.01);
        assert!((area - 5.0).abs() < 0.02, "{area}"); // two triangles, each 5 m base x 1 m high / 2 = 2.5 m2
    }
}
