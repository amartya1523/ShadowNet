//! Edge betweenness, Brandes (2001) generalised to weighted graphs.
//!
//! Used as a static feature for the failure predictor ("how much of the
//! network's shortest-path structure runs over this link") and by the
//! scenario selectors ("cut the most central link").

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use super::{Graph, UNREACHABLE};

/// Returns a score per edge id, `edge_count` long. For undirected graphs
/// stored as two arcs, each unordered pair is counted from both ends, which
/// is the usual convention; divide by two if you need the other one.
///
/// `sources` limits the computation to a subset of source nodes (pivots),
/// which gives an unbiased estimate at a fraction of the cost. Pass `None`
/// for the exact value.
pub fn edge_betweenness<G: Graph>(g: &G, edge_count: usize, sources: Option<&[usize]>) -> Vec<f64> {
    let n = g.node_count();
    let mut score = vec![0.0; edge_count];

    // Reused across sources to keep allocation out of the loop.
    let mut dist = vec![UNREACHABLE; n];
    let mut sigma = vec![0.0f64; n];
    let mut delta = vec![0.0f64; n];
    let mut preds: Vec<Vec<(u32, u32)>> = vec![Vec::new(); n];
    let mut stack = Vec::with_capacity(n);
    let mut heap = BinaryHeap::new();

    let all: Vec<usize>;
    let sources = match sources {
        Some(s) => s,
        None => {
            all = (0..n).collect();
            &all
        }
    };
    let scale = n as f64 / sources.len().max(1) as f64;

    for &s in sources {
        dist.fill(UNREACHABLE);
        sigma.fill(0.0);
        delta.fill(0.0);
        for p in &mut preds {
            p.clear();
        }
        stack.clear();

        dist[s] = 0;
        sigma[s] = 1.0;
        heap.push(Reverse((0u64, s as u32)));
        while let Some(Reverse((d, u))) = heap.pop() {
            let u = u as usize;
            if d > dist[u] {
                continue;
            }
            stack.push(u);
            g.for_each_edge(u, |v, w, e| {
                let nd = d + w;
                if nd < dist[v] {
                    dist[v] = nd;
                    sigma[v] = 0.0;
                    preds[v].clear();
                    heap.push(Reverse((nd, v as u32)));
                }
                if nd == dist[v] {
                    sigma[v] += sigma[u];
                    preds[v].push((u as u32, e));
                }
            });
        }

        // Weights are strictly positive and we only push on strict
        // improvement, so every node lands on the stack exactly once and the
        // stack is in non-decreasing distance order. Unwinding it is the
        // dependency accumulation from the paper.
        while let Some(w) = stack.pop() {
            for &(v, e) in &preds[w] {
                let v = v as usize;
                let c = sigma[v] / sigma[w] * (1.0 + delta[w]);
                score[e as usize] += c * scale;
                delta[v] += c;
            }
        }
    }
    score
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::algo::Csr;

    #[test]
    fn line_graph_middle_edge_is_busiest() {
        // 0 - 1 - 2 - 3. Pairs crossing edge (1,2): {0,1}x{2,3} = 4, counted
        // in both directions = 8.
        let g = Csr::undirected(4, [(0, 1, 1, 0), (1, 2, 1, 1), (2, 3, 1, 2)]);
        let b = edge_betweenness(&g, 3, None);
        assert_eq!(b, vec![6.0, 8.0, 6.0]);
    }

    #[test]
    fn equal_cost_paths_split_credit() {
        // Square 0-1-3, 0-2-3. Pair (0,3) splits over both sides.
        let g = Csr::undirected(4, [(0, 1, 1, 0), (1, 3, 1, 1), (0, 2, 1, 2), (2, 3, 1, 3)]);
        let b = edge_betweenness(&g, 4, None);
        for x in &b {
            assert!((x - b[0]).abs() < 1e-9, "{b:?}");
        }
    }

    #[test]
    fn bridge_carries_everything() {
        // Two triangles joined by a bridge (edge 3).
        let g = Csr::undirected(
            6,
            [(0, 1, 1, 0), (1, 2, 1, 1), (2, 0, 1, 2), (2, 3, 1, 3), (3, 4, 1, 4), (4, 5, 1, 5), (5, 3, 1, 6)],
        );
        let b = edge_betweenness(&g, 7, None);
        // 3 x 3 pairs cross the bridge, both directions.
        assert!((b[3] - 18.0).abs() < 1e-9);
        assert!(b.iter().enumerate().all(|(i, &x)| i == 3 || x < b[3]));
    }
}
