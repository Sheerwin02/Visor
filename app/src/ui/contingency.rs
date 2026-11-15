//! "If we run late": which stations to drop when the crew is behind schedule.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
//! Rule (VISOR's, shown in the UI): drop the lowest priority first (P3, then P2, then P1); among equal priority the later
//! station first (earlier ones are already done or close); the drill pair is never split (it drops as one group, with the
//! higher priority of its two members). Each dropped station gives back its station time; the change in walking time is
//! NOT estimated here: "Apply" removes the stations through the planner, which re-plans the route exactly.

/// A station as the contingency sees it: walking-order index, priority 1..=3, part of the drill pair.
#[derive(Clone, Debug, PartialEq)]
pub struct Station {
    pub index: usize,
    pub priority: u8,
    pub pair: bool,
}

/// Stations to drop (walking-order indices, ascending) and the station minutes they give back.
pub fn drop_plan(stations: &[Station], behind_min: f64, station_min: f64) -> (Vec<usize>, f64) {
    if behind_min <= 0.0 || station_min <= 0.0 {
        return (Vec::new(), 0.0);
    }
    // groups: the pair is one group (priority = the more important member, index = the later member)
    let mut groups: Vec<(u8, usize, Vec<usize>)> = stations.iter().filter(|s| !s.pair).map(|s| (s.priority, s.index, vec![s.index])).collect();
    let pair: Vec<&Station> = stations.iter().filter(|s| s.pair).collect();
    if !pair.is_empty() {
        groups.push((pair.iter().map(|s| s.priority).min().unwrap_or(1), pair.iter().map(|s| s.index).max().unwrap_or(0),
                     pair.iter().map(|s| s.index).collect()));
    }
    groups.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    let mut out = Vec::new();
    let mut got = 0.0;
    for (_, _, members) in groups {
        if got >= behind_min {
            break;
        }
        got += members.len() as f64 * station_min;
        out.extend(members);
    }
    out.sort_unstable();
    (out, got)
}

/// Default priority: the drill pair P1, every other station P2 (a planner setting, not data).
pub fn default_priority(pair: bool) -> u8 {
    if pair { 1 } else { 2 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(index: usize, priority: u8, pair: bool) -> Station {
        Station { index, priority, pair }
    }

    #[test]
    fn lowest_priority_and_later_stations_go_first_and_the_pair_stays_whole() {
        let st = [s(1, 2, false), s(2, 2, false), s(3, 3, false), s(4, 1, true), s(5, 1, true)];
        assert_eq!(drop_plan(&st, 0.0, 54.0), (vec![], 0.0));
        assert_eq!(drop_plan(&st, 15.0, 54.0), (vec![3], 54.0), "P3 first");
        assert_eq!(drop_plan(&st, 60.0, 54.0), (vec![2, 3], 108.0), "then the later P2");
        assert_eq!(drop_plan(&st, 200.0, 54.0), (vec![1, 2, 3, 4, 5], 270.0), "the pair drops last, both together");
        let p3pair = [s(1, 2, false), s(4, 3, true), s(5, 1, true)];
        assert_eq!(drop_plan(&p3pair, 10.0, 54.0), (vec![1], 54.0), "a pair takes its more important member's priority");
    }
}
