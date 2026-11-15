//! EVA simulation clock (automatic walk with play and rewind). The whole walk is a pure function of the
//! simulation time t (seconds from leaving the exit): positions, heading, which stop is next, and the event list, so play,
//! rewind and scrubbing are exact and deterministic (no state carried from frame to frame).
//! - Legs take the planner's own leg times; inside a leg, time is spread by the route's pace model (slower on steeper
//!   corrected slope), normalised so each leg still sums to the planner's time.
//! - Time at each station = the planner's typed minutes per stop, else the station time NASA's numbers leave (crew.rs:
//!   (EVA length - egress/ingress - walking p90) split evenly over the stations). The whole EVA is simulated
//!   (a full 6-hour EVA): egress at the exit first, the stations, ingress at the exit last. NASA gives egress/ingress as
//!   one hour together; splitting it half out, half back is a VISOR ASSUMPTION (crew.rs EGRESS_SHARE).
//! - EV2 walks the same route astronaut::BUDDY_GAP_M behind EV1 (time_at_dist; a VISUAL choice).
use bevy::prelude::*;

#[derive(Clone, Debug, PartialEq)]
pub enum Seg {
    /// walking leg `leg` over route points i0..=i1 (indices into Sim::pts), from t0 to t1
    Walk { leg: usize, i0: usize, i1: usize, t0: f32, t1: f32 },
    /// standing at stop `stop` (walking order) from t0 to t1
    Dwell { stop: usize, t0: f32, t1: f32 },
}

#[derive(Clone, Debug, PartialEq)]
pub enum EventKind {
    Depart(usize),
    Arrive(usize),
    /// steepest corrected slope on a leg (deg), with the leg index
    Steepest(usize, f32),
    /// back at the exit after the last leg
    End,
    /// egress starts (t = 0) / ingress starts (after End) / the EVA is over
    Egress,
    Ingress,
    Done,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub t: f32,
    pub kind: EventKind,
    pub e: f32,
    pub n: f32,
}

/// Where the walk is at time t.
#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub e: f32,
    pub n: f32,
    /// compass heading, radians (0 north, pi/2 east)
    pub heading: f32,
    /// distance walked along the route (m)
    pub dist: f32,
    /// next stop to reach (walking order index); at a stop: that stop
    pub next: usize,
    /// Some(stop) while standing at a stop
    pub at_stop: Option<usize>,
    pub done: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sim {
    /// route points: east, north, time reached (s, walking only, before dwells are inserted), cumulative distance (m)
    pub pts: Vec<(f32, f32, f32, f32)>,
    pub segs: Vec<Seg>,
    pub events: Vec<Event>,
    pub total: f32,
    pub dwell_s: f32,
    pub egress_s: f32,
    pub ingress_s: f32,
    pub n_stops: usize,
}

/// Build the simulation. `poly` and `legs` are the live plan (share::LastPlan: polyline east/north and per leg
/// (first, last polyline index, walking s)); `speed_at(e, n)` is the walking speed there (m/s, pace model);
/// `slope_at(e, n)` the corrected slope (deg, for the steepest-point events).
pub fn build(poly: &[(f64, f64)], legs: &[(usize, usize, f64)], speed_at: impl Fn(f32, f32) -> f32, slope_at: impl Fn(f32, f32) -> f32,
             dwell_s: f32, egress_s: f32, ingress_s: f32) -> Sim {
    let mut sim = Sim { dwell_s: dwell_s.max(0.0), egress_s: egress_s.max(0.0), ingress_s: ingress_s.max(0.0), n_stops: legs.len(),
                        ..Default::default() };
    let mut t = 0.0f32; // with dwells
    if sim.egress_s > 0.0 && !legs.is_empty() {
        sim.segs.push(Seg::Dwell { stop: 0, t0: 0.0, t1: sim.egress_s });
        let p0 = poly.get(legs[0].0).copied().unwrap_or_default();
        sim.events.push(Event { t: 0.0, kind: EventKind::Egress, e: p0.0 as f32, n: p0.1 as f32 });
        t = sim.egress_s;
    }
    let mut dist = 0.0f32;
    for (k, &(a, b, secs)) in legs.iter().enumerate() {
        if b <= a || b >= poly.len() {
            continue;
        }
        let p = |i: usize| (poly[i].0 as f32, poly[i].1 as f32);
        // weights: segment length / speed at its midpoint, normalised to the planner's leg time
        let w: Vec<(f32, f32)> = (a..b).map(|i| {
            let (p0, p1) = (p(i), p(i + 1));
            let l = ((p1.0 - p0.0).powi(2) + (p1.1 - p0.1).powi(2)).sqrt();
            let v = speed_at((p0.0 + p1.0) / 2.0, (p0.1 + p1.1) / 2.0).max(1e-3);
            (l, l / v)
        }).collect();
        let wsum: f32 = w.iter().map(|x| x.1).sum();
        let scale = if wsum > 0.0 { secs as f32 / wsum } else { 0.0 };
        // every leg starts with its own point at the current clock (after any dwell), so a leg never interpolates across a stop
        sim.pts.push((p(a).0, p(a).1, t, dist));
        let i0 = sim.pts.len() - 1;
        sim.events.push(Event { t, kind: EventKind::Depart(k), e: p(a).0, n: p(a).1 });
        let t_leg0 = t;
        let (mut worst, mut worst_at) = (-1.0f32, (t, p(a)));
        for (j, (l, wt)) in w.iter().enumerate() {
            let q = p(a + j + 1);
            t += wt * scale;
            dist += l;
            sim.pts.push((q.0, q.1, t, dist));
            let s = slope_at(q.0, q.1);
            if s > worst {
                worst = s;
                worst_at = (t, q);
            }
        }
        sim.segs.push(Seg::Walk { leg: k, i0, i1: sim.pts.len() - 1, t0: t_leg0, t1: t });
        sim.events.push(Event { t: worst_at.0, kind: EventKind::Steepest(k, worst), e: worst_at.1 .0, n: worst_at.1 .1 });
        let arrive = (k + 1) % legs.len().max(1);
        let last = k + 1 == legs.len();
        sim.events.push(Event { t, kind: if last { EventKind::End } else { EventKind::Arrive(arrive) }, e: p(b).0, n: p(b).1 });
        if !last && sim.dwell_s > 0.0 {
            sim.segs.push(Seg::Dwell { stop: arrive, t0: t, t1: t + sim.dwell_s });
            t += sim.dwell_s;
            // the points after a dwell are reached later: later legs start from the shifted clock
        }
        if last && sim.ingress_s > 0.0 {
            sim.segs.push(Seg::Dwell { stop: 0, t0: t, t1: t + sim.ingress_s });
            sim.events.push(Event { t, kind: EventKind::Ingress, e: p(b).0, n: p(b).1 });
            t += sim.ingress_s;
            sim.events.push(Event { t, kind: EventKind::Done, e: p(b).0, n: p(b).1 });
        }
    }
    // stable sort: events at the same time keep their order (End before Ingress)
    sim.events.sort_by(|x, y| x.t.total_cmp(&y.t));
    sim.total = t;
    sim
}

impl Sim {
    fn point_heading(&self, i: usize) -> f32 {
        let n = self.pts.len();
        if n < 2 {
            return 0.0;
        }
        // the next point at a different position (legs repeat their start point)
        let a = self.pts[i.min(n - 1)];
        let b = self.pts[i.min(n - 1)..].iter().find(|q| (q.0, q.1) != (a.0, a.1)).copied();
        match b {
            Some(b) => (b.0 - a.0).atan2(b.1 - a.1),
            None => {
                let pa = self.pts[..i.min(n - 1)].iter().rev().find(|q| (q.0, q.1) != (a.0, a.1)).copied().unwrap_or(a);
                (a.0 - pa.0).atan2(a.1 - pa.1)
            }
        }
    }

    /// The walk at time t (clamped to 0..total).
    pub fn state(&self, t: f32) -> State {
        let finished = t >= self.total && self.total > 0.0;
        let t = t.clamp(0.0, self.total);
        let mut heading = self.point_heading(0);
        for s in &self.segs {
            match *s {
                Seg::Walk { leg, i0, i1, t0, t1 } if t <= t1 && !finished => {
                    let mut i = i0;
                    while i < i1 && self.pts[i + 1].2 < t {
                        i += 1;
                    }
                    let (a, b) = (self.pts[i], self.pts[(i + 1).min(i1)]);
                    let f = if b.2 > a.2 { ((t - a.2) / (b.2 - a.2)).clamp(0.0, 1.0) } else { 0.0 };
                    let _ = t0;
                    return State { e: a.0 + (b.0 - a.0) * f, n: a.1 + (b.1 - a.1) * f, heading: self.point_heading(i),
                                   dist: a.3 + (b.3 - a.3) * f, next: (leg + 1) % self.n_stops.max(1), at_stop: None, done: false };
                }
                Seg::Walk { i1, .. } => heading = self.point_heading(i1.saturating_sub(1)),
                Seg::Dwell { stop, t0, t1 } if t <= t1 && !finished => {
                    let p = self.stop_point(stop);
                    // egress (the exit before any walking) is at the start of the route, not its end
                    let dist = if stop == 0 && t0 == 0.0 { 0.0 } else { p.2 };
                    return State { e: p.0, n: p.1, heading, dist, next: if dist == 0.0 && stop == 0 { 1 % self.n_stops.max(1) } else { stop },
                                   at_stop: Some(stop), done: false };
                }
                Seg::Dwell { .. } => {}
            }
        }
        let last = self.pts.last().copied().unwrap_or_default();
        State { e: last.0, n: last.1, heading, dist: last.3, next: 0, at_stop: None, done: true }
    }

    /// Where stop k is on the route (east, north, distance along the route).
    fn stop_point(&self, k: usize) -> (f32, f32, f32) {
        for s in &self.segs {
            if let Seg::Walk { leg, i1, .. } = *s {
                if (leg + 1) % self.n_stops.max(1) == k {
                    let p = self.pts[i1];
                    return (p.0, p.1, p.3);
                }
            }
        }
        self.pts.first().map(|p| (p.0, p.1, p.3)).unwrap_or_default()
    }

    /// Times segments start, as stop boundaries for prev/next: departures from each stop.
    pub fn stop_times(&self) -> Vec<f32> {
        let mut v: Vec<f32> = self.events.iter().filter(|e| matches!(e.kind, EventKind::Depart(_) | EventKind::End | EventKind::Done)).map(|e| e.t).collect();
        v.dedup();
        v
    }

    /// The first time the walk reaches `dist` metres along the route (for scrubbing on the distance-based timeline).
    pub fn time_at_dist(&self, dist: f32) -> f32 {
        for s in &self.segs {
            if let Seg::Walk { i0, i1, .. } = *s {
                for i in i0..i1 {
                    let (a, b) = (self.pts[i], self.pts[i + 1]);
                    if dist <= b.3 {
                        let f = if b.3 > a.3 { ((dist - a.3) / (b.3 - a.3)).clamp(0.0, 1.0) } else { 0.0 };
                        return a.2 + (b.2 - a.2) * f;
                    }
                }
            }
        }
        self.total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// a square loop 0,0 -> 10,0 -> 10,10 -> 0,0 with three legs
    fn square() -> (Vec<(f64, f64)>, Vec<(usize, usize, f64)>) {
        let poly = vec![(0.0, 0.0), (5.0, 0.0), (10.0, 0.0), (10.0, 5.0), (10.0, 10.0), (5.0, 5.0), (0.0, 0.0)];
        (poly, vec![(0, 2, 20.0), (2, 4, 40.0), (4, 6, 30.0)])
    }

    #[test]
    fn leg_times_match_the_planner_and_dwells_are_added() {
        let (p, l) = square();
        let s = build(&p, &l, |_, _| 1.0, |_, _| 0.0, 0.0, 0.0, 0.0);
        assert!((s.total - 90.0).abs() < 1e-4, "{}", s.total);
        let d = build(&p, &l, |_, _| 1.0, |_, _| 0.0, 60.0, 0.0, 0.0);
        assert!((d.total - 210.0).abs() < 1e-3, "two intermediate stops x 60 s: {}", d.total);
        // at t = 25 s the walk stands at stop 1 (arrived at 20 s, dwell until 80 s)
        let st = d.state(25.0);
        assert_eq!(st.at_stop, Some(1));
        assert!((st.e - 10.0).abs() < 1e-4 && st.n.abs() < 1e-4);
        // after the dwell the second leg runs 80..120 s: half way at 100 s is (10, 5)
        let m = d.state(100.0);
        assert!((m.e - 10.0).abs() < 1e-3 && (m.n - 5.0).abs() < 1e-3, "{m:?}");
        assert_eq!(m.next, 2);
        assert!(d.state(1e9).done);
    }

    #[test]
    fn slower_ground_takes_more_of_the_leg_time() {
        let (p, l) = square();
        // first half of leg 0 (e < 5) is twice as slow
        let s = build(&p, &l, |e, _| if e < 5.0 { 0.5 } else { 1.0 }, |_, _| 0.0, 0.0, 0.0, 0.0);
        let mid = s.pts.iter().find(|q| q.0 == 5.0 && q.1 == 0.0).unwrap();
        assert!((mid.2 - 20.0 * 2.0 / 3.0).abs() < 1e-3, "slow half takes 2/3 of 20 s: {}", mid.2);
        assert!((s.total - 90.0).abs() < 1e-3, "leg totals unchanged");
    }

    #[test]
    fn heading_follows_travel_and_scrub_by_distance_is_consistent() {
        let (p, l) = square();
        let s = build(&p, &l, |_, _| 1.0, |_, _| 0.0, 0.0, 0.0, 0.0);
        let st = s.state(5.0);
        assert!((st.heading - std::f32::consts::FRAC_PI_2).abs() < 1e-4, "east first: {}", st.heading);
        for t in [3.0f32, 27.0, 55.0, 80.0] {
            let d = s.state(t).dist;
            assert!((s.time_at_dist(d) - t).abs() < 1e-2, "t {t}: dist {d} -> {}", s.time_at_dist(d));
        }
    }

    #[test]
    fn events_are_ordered_and_name_the_steepest_point() {
        let (p, l) = square();
        let s = build(&p, &l, |_, _| 1.0, |e, n| if (e - 10.0).abs() < 1e-3 && (n - 5.0).abs() < 1e-3 { 12.0 } else { 3.0 }, 30.0, 0.0, 0.0);
        assert!(s.events.windows(2).all(|w| w[0].t <= w[1].t));
        let st = s.events.iter().find(|e| matches!(e.kind, EventKind::Steepest(1, _))).unwrap();
        assert!(matches!(st.kind, EventKind::Steepest(1, x) if (x - 12.0).abs() < 1e-6) && (st.e - 10.0).abs() < 1e-4);
        assert!(matches!(s.events.last().unwrap().kind, EventKind::End));
        assert_eq!(s.stop_times().len(), 4, "3 departures + end");
    }

    #[test]
    fn egress_and_ingress_bracket_the_walk_at_the_exit() {
        let (p, l) = square();
        let s = build(&p, &l, |_, _| 1.0, |_, _| 0.0, 60.0, 1800.0, 1800.0);
        assert!((s.total - (1800.0 + 90.0 + 120.0 + 1800.0)).abs() < 1e-2, "{}", s.total);
        let a = s.state(100.0); // during egress: at the exit, not moving
        assert_eq!(a.at_stop, Some(0));
        assert!(a.e.abs() < 1e-4 && a.n.abs() < 1e-4 && a.dist == 0.0, "{a:?}");
        let w = s.state(1810.0); // 10 s into leg 0
        assert!(w.at_stop.is_none() && (w.e - 10.0 * 10.0 / 20.0).abs() < 1e-3, "{w:?}");
        let i = s.state(s.total - 100.0); // ingress
        assert_eq!(i.at_stop, Some(0));
        assert!(i.e.abs() < 1e-4 && i.n.abs() < 1e-4);
        let kinds: Vec<_> = s.events.iter().map(|e| e.kind.clone()).collect();
        assert_eq!(kinds.first(), Some(&EventKind::Egress));
        assert_eq!(kinds.last(), Some(&EventKind::Done));
        let ei = kinds.iter().position(|k| *k == EventKind::End).unwrap();
        assert_eq!(kinds[ei + 1], EventKind::Ingress, "End then Ingress at the same time");
        assert!((s.time_at_dist(0.0) - 1800.0).abs() < 1e-2, "walking starts after egress");
    }
}
