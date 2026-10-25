//! Graph algorithms, written from scratch on purpose.
//!
//! The protocol models in `shadow-sim` call these on graphs that are rebuilt
//! constantly (a router's view of its LSDB changes every time an LSA lands),
//! so the interface is a tiny trait rather than a graph library's type
//! zoo. Anything that can enumerate out-edges can be routed over.

mod bellman_ford;
mod betweenness;
mod dijkstra;

pub use bellman_ford::{BellmanFord, NegativeCycle, bellman_ford};
pub use betweenness::edge_betweenness;
pub use dijkstra::{ShortestPaths, dijkstra, dijkstra_with};

/// Sentinel for "no node / no edge" in the dense result arrays.
pub const NONE: u32 = u32::MAX;
/// Distance to a node we never reached.
pub const UNREACHABLE: u64 = u64::MAX;

pub trait Graph {
    fn node_count(&self) -> usize;

    /// Calls `f(to, weight, edge_id)` for every out-edge of `u`. Edge ids are
    /// opaque to the algorithms; they are handed back in results so callers
    /// can map a hop to a physical link.
    fn for_each_edge<F: FnMut(usize, u64, u32)>(&self, u: usize, f: F);
}

/// Compressed sparse row adjacency. The workhorse representation: one
/// allocation for all edges, cache friendly, cheap to rebuild.
#[derive(Clone, Debug, Default)]
pub struct Csr {
    offsets: Vec<u32>,
    to: Vec<u32>,
    weight: Vec<u64>,
    id: Vec<u32>,
}

impl Csr {
    /// Builds from `(from, to, weight, id)` arcs. Undirected graphs should
    /// pass both directions (see [`Csr::undirected`]).
    pub fn from_arcs(n: usize, arcs: impl IntoIterator<Item = (usize, usize, u64, u32)>) -> Self {
        let arcs: Vec<_> = arcs.into_iter().collect();
        let mut degree = vec![0u32; n + 1];
        for &(u, v, _, _) in &arcs {
            assert!(u < n && v < n, "arc {u}->{v} out of range for {n} nodes");
            degree[u + 1] += 1;
        }
        for i in 0..n {
            degree[i + 1] += degree[i];
        }
        let offsets = degree;
        let m = arcs.len();
        let mut cursor = offsets.clone();
        let (mut to, mut weight, mut id) = (vec![0; m], vec![0; m], vec![0; m]);
        for (u, v, w, e) in arcs {
            let slot = cursor[u] as usize;
            cursor[u] += 1;
            to[slot] = v as u32;
            weight[slot] = w;
            id[slot] = e;
        }
        Csr { offsets, to, weight, id }
    }

    pub fn undirected(n: usize, edges: impl IntoIterator<Item = (usize, usize, u64, u32)>) -> Self {
        Self::from_arcs(
            n,
            edges
                .into_iter()
                .flat_map(|(u, v, w, e)| [(u, v, w, e), (v, u, w, e)]),
        )
    }

    pub fn arc_count(&self) -> usize {
        self.to.len()
    }

    pub fn degree(&self, u: usize) -> usize {
        (self.offsets[u + 1] - self.offsets[u]) as usize
    }
}

impl Graph for Csr {
    fn node_count(&self) -> usize {
        self.offsets.len() - 1
    }

    #[inline]
    fn for_each_edge<F: FnMut(usize, u64, u32)>(&self, u: usize, mut f: F) {
        let (a, b) = (self.offsets[u] as usize, self.offsets[u + 1] as usize);
        for i in a..b {
            f(self.to[i] as usize, self.weight[i], self.id[i]);
        }
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use super::Csr;
    use crate::rng::Rng;

    /// Connected random graph: a random spanning tree plus `extra` chords.
    pub fn random_connected(rng: &mut Rng, n: usize, extra: usize, max_w: u64) -> Csr {
        let mut edges = Vec::new();
        let mut id = 0u32;
        for v in 1..n {
            let u = rng.below(v);
            edges.push((u, v, 1 + rng.below(max_w as usize) as u64, id));
            id += 1;
        }
        for _ in 0..extra {
            let (u, v) = (rng.below(n), rng.below(n));
            if u != v {
                edges.push((u, v, 1 + rng.below(max_w as usize) as u64, id));
                id += 1;
            }
        }
        Csr::undirected(n, edges)
    }
}
