use std::cmp::Reverse;
use std::collections::BinaryHeap;

use super::{Graph, NONE, UNREACHABLE};

/// A shortest-path tree rooted at `source`.
///
/// Besides the usual distance and parent arrays we keep `first_edge`: the
/// edge leaving `source` on the way to each node. That is precisely a
/// router's FIB row, and computing it during the search is free, whereas
/// walking parents back afterwards is O(n * depth).
#[derive(Clone, Debug)]
pub struct ShortestPaths {
    pub source: usize,
    pub dist: Vec<u64>,
    pub parent: Vec<u32>,
    pub parent_edge: Vec<u32>,
    pub first_edge: Vec<u32>,
    /// Nodes in the order they were settled. Brandes needs this, and it is
    /// handy for debugging.
    pub order: Vec<u32>,
}

impl ShortestPaths {
    pub fn reachable(&self, v: usize) -> bool {
        self.dist[v] != UNREACHABLE
    }

    /// Node sequence from the source to `target`, inclusive at both ends.
    pub fn path(&self, target: usize) -> Option<Vec<usize>> {
        if !self.reachable(target) {
            return None;
        }
        let mut out = vec![target];
        let mut v = target;
        while v != self.source {
            v = self.parent[v] as usize;
            out.push(v);
        }
        out.reverse();
        Some(out)
    }

    pub fn first_hop(&self, target: usize) -> Option<u32> {
        match self.first_edge[target] {
            NONE => None,
            e => Some(e),
        }
    }
}

/// Plain Dijkstra from `source` over every node.
pub fn dijkstra<G: Graph>(g: &G, source: usize) -> ShortestPaths {
    dijkstra_with(g, source, |_| true)
}

/// Dijkstra that only relaxes through nodes for which `transit` returns
/// true. Nodes that fail the predicate can still be *reached*, they just do
/// not forward. Used for things like "a router that is overloaded advertises
/// itself as a stub" without rebuilding the graph.
pub fn dijkstra_with<G: Graph>(g: &G, source: usize, transit: impl Fn(usize) -> bool) -> ShortestPaths {
    let n = g.node_count();
    let mut dist = vec![UNREACHABLE; n];
    let mut parent = vec![NONE; n];
    let mut parent_edge = vec![NONE; n];
    let mut first_edge = vec![NONE; n];
    let mut order = Vec::with_capacity(n);
    let mut done = vec![false; n];

    // (distance, node). Ties on distance break towards the lower node id,
    // which makes route selection deterministic between runs.
    let mut heap = BinaryHeap::with_capacity(n);
    dist[source] = 0;
    heap.push(Reverse((0u64, source as u32)));

    while let Some(Reverse((d, u))) = heap.pop() {
        let u = u as usize;
        if done[u] {
            continue;
        }
        done[u] = true;
        order.push(u as u32);
        if u != source && !transit(u) {
            continue;
        }
        g.for_each_edge(u, |v, w, e| {
            let nd = d.saturating_add(w);
            // Strictly-better only: the first equal-cost parent found wins,
            // and since the heap settles in (dist, id) order that is the one
            // with the smaller id.
            if nd < dist[v] {
                dist[v] = nd;
                parent[v] = u as u32;
                parent_edge[v] = e;
                first_edge[v] = if u == source { e } else { first_edge[u] };
                heap.push(Reverse((nd, v as u32)));
            }
        });
    }

    ShortestPaths {
        source,
        dist,
        parent,
        parent_edge,
        first_edge,
        order,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::algo::Csr;
    use crate::algo::testing::random_connected;
    use crate::rng::Rng;

    fn diamond() -> Csr {
        //    1
        //  /   \
        // 0     3 -- 4
        //  \   /
        //    2
        Csr::undirected(
            5,
            [(0, 1, 1, 0), (1, 3, 5, 1), (0, 2, 2, 2), (2, 3, 1, 3), (3, 4, 1, 4)],
        )
    }

    #[test]
    fn picks_cheaper_branch() {
        let sp = dijkstra(&diamond(), 0);
        assert_eq!(sp.dist, vec![0, 1, 2, 3, 4]);
        assert_eq!(sp.path(4).unwrap(), vec![0, 2, 3, 4]);
        assert_eq!(sp.first_hop(4), Some(2));
        assert_eq!(sp.first_hop(1), Some(0));
        assert_eq!(sp.first_hop(0), None);
    }

    #[test]
    fn unreachable_nodes() {
        let g = Csr::undirected(4, [(0, 1, 1, 0), (2, 3, 1, 1)]);
        let sp = dijkstra(&g, 0);
        assert!(!sp.reachable(2));
        assert_eq!(sp.path(3), None);
        assert_eq!(sp.first_hop(3), None);
    }

    #[test]
    fn transit_predicate_blocks_forwarding_not_reachability() {
        // 0 - 1 - 2, with 1 refusing transit.
        let g = Csr::undirected(3, [(0, 1, 1, 0), (1, 2, 1, 1)]);
        let sp = dijkstra_with(&g, 0, |v| v != 1);
        assert!(sp.reachable(1));
        assert!(!sp.reachable(2));
    }

    #[test]
    fn first_edge_agrees_with_parent_walk() {
        let mut rng = Rng::new(5);
        for _ in 0..20 {
            let g = random_connected(&mut rng, 200, 400, 50);
            let src = rng.below(200);
            let sp = dijkstra(&g, src);
            for t in 0..200 {
                if t == src {
                    continue;
                }
                let path = sp.path(t).unwrap();
                // parent_edge of the second node on the path is the edge
                // leaving the source.
                assert_eq!(sp.first_edge[t], sp.parent_edge[path[1]]);
            }
        }
    }

    #[test]
    fn deterministic_tie_break() {
        // Two equal-cost routes 0->1->3 and 0->2->3.
        let g = Csr::undirected(4, [(0, 1, 1, 0), (0, 2, 1, 1), (1, 3, 1, 2), (2, 3, 1, 3)]);
        for _ in 0..10 {
            assert_eq!(dijkstra(&g, 0).path(3).unwrap(), vec![0, 1, 3]);
        }
    }
}
