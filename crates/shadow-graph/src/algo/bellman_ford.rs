//! Bellman-Ford over an explicit arc list, with signed weights.
//!
//! The distance-vector protocol in `shadow-sim` is the distributed version of
//! this loop; having the centralised one around lets the tests check that the
//! protocol converges to the same answer.

use super::NONE;

#[derive(Clone, Debug)]
pub struct BellmanFord {
    pub source: usize,
    /// `None` for unreachable nodes.
    pub dist: Vec<Option<i64>>,
    pub parent: Vec<u32>,
    /// How many full relaxation passes it took to settle. Handy to show how
    /// the number of rounds tracks the hop diameter, which is the whole
    /// reason RIP converges slowly.
    pub rounds: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NegativeCycle {
    /// The nodes on one negative cycle, in order.
    pub cycle: Vec<usize>,
}

impl std::fmt::Display for NegativeCycle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "negative cycle through {:?}", self.cycle)
    }
}

impl std::error::Error for NegativeCycle {}

/// `arcs` are directed `(from, to, weight)`.
pub fn bellman_ford(n: usize, arcs: &[(usize, usize, i64)], source: usize) -> Result<BellmanFord, NegativeCycle> {
    let mut dist: Vec<Option<i64>> = vec![None; n];
    let mut parent = vec![NONE; n];
    dist[source] = Some(0);

    let mut rounds = 0;
    let mut last_relaxed = None;
    for _ in 0..n {
        last_relaxed = None;
        for &(u, v, w) in arcs {
            let Some(du) = dist[u] else { continue };
            let nd = du + w;
            if dist[v].is_none_or(|dv| nd < dv) {
                dist[v] = Some(nd);
                parent[v] = u as u32;
                last_relaxed = Some(v);
            }
        }
        rounds += 1;
        if last_relaxed.is_none() {
            break;
        }
    }

    // Still relaxing after n passes means a negative cycle is reachable.
    // Walking parents n times from the last relaxed node is guaranteed to
    // land us on the cycle itself (not just on a tail leading into it).
    if let Some(mut v) = last_relaxed {
        for _ in 0..n {
            v = parent[v] as usize;
        }
        let start = v;
        let mut cycle = vec![start];
        let mut u = parent[start] as usize;
        while u != start {
            cycle.push(u);
            u = parent[u] as usize;
        }
        cycle.reverse();
        return Err(NegativeCycle { cycle });
    }

    Ok(BellmanFord { source, dist, parent, rounds })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::algo::testing::random_connected;
    use crate::algo::{Graph, dijkstra};
    use crate::rng::Rng;

    #[test]
    fn matches_dijkstra_on_nonnegative_graphs() {
        let mut rng = Rng::new(17);
        for _ in 0..15 {
            let n = 150;
            let g = random_connected(&mut rng, n, 300, 100);
            let mut arcs = Vec::new();
            for u in 0..n {
                g.for_each_edge(u, |v, w, _| arcs.push((u, v, w as i64)));
            }
            let src = rng.below(n);
            let bf = bellman_ford(n, &arcs, src).unwrap();
            let dj = dijkstra(&g, src);
            for v in 0..n {
                assert_eq!(bf.dist[v], Some(dj.dist[v] as i64), "node {v}");
            }
        }
    }

    #[test]
    fn handles_negative_edges() {
        // 0 -> 1 (4), 0 -> 2 (5), 2 -> 1 (-3)
        let bf = bellman_ford(3, &[(0, 1, 4), (0, 2, 5), (2, 1, -3)], 0).unwrap();
        assert_eq!(bf.dist, vec![Some(0), Some(2), Some(5)]);
        assert_eq!(bf.parent[1], 2);
    }

    #[test]
    fn finds_the_negative_cycle() {
        // 0 -> 1 -> 2 -> 3 -> 1 with the loop summing to -1.
        let arcs = [(0, 1, 1), (1, 2, 1), (2, 3, 1), (3, 1, -3)];
        let err = bellman_ford(4, &arcs, 0).unwrap_err();
        let mut c = err.cycle.clone();
        c.sort_unstable();
        assert_eq!(c, vec![1, 2, 3]);
    }

    #[test]
    fn unreachable_stays_none() {
        let bf = bellman_ford(3, &[(0, 1, 1)], 0).unwrap();
        assert_eq!(bf.dist[2], None);
    }

    #[test]
    fn rounds_track_hop_count() {
        // A line, with arcs listed backwards so each pass only advances one
        // hop: the worst case for Bellman-Ford.
        let n = 10;
        let arcs: Vec<_> = (0..n - 1).rev().map(|i| (i, i + 1, 1)).collect();
        let bf = bellman_ford(n, &arcs, 0).unwrap();
        assert_eq!(bf.rounds, n); // n-1 passes to settle, one more to notice
    }
}
