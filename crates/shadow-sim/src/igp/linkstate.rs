//! Link-state routing, modelled on OSPF.
//!
//! What is real: LSAs with sequence numbers, reliable flooding hop by hop
//! with real link delays, LSDB synchronisation when an adjacency comes up,
//! the two-way connectivity check, SPF throttling, a delay between computing
//! a route and having it in hardware, and the fact that every router
//! computes its FIB from *its own* LSDB at *its own* pace. That last part is
//! what produces micro-loops during convergence.
//!
//! What is not: areas, DR election, LSA aging and refresh, acks and
//! retransmission (links that are up deliver everything).

use std::sync::Arc;

use shadow_graph::algo::{Csr, NONE, UNREACHABLE, dijkstra};
use shadow_graph::{AsId, LinkId, RouterId};

use super::{Fib, View};
use crate::event::{Event, EventQueue};
use crate::time::Nanos;

#[derive(Debug)]
pub struct Lsa {
    pub origin: RouterId,
    pub seq: u32,
    /// Live adjacencies inside the AS: (neighbour, link, cost).
    pub adj: Vec<(RouterId, LinkId, u32)>,
    /// Live links to other ASes: (neighbour AS, link, cost). The equivalent
    /// of an ASBR advertising itself.
    pub exits: Vec<(AsId, LinkId, u32)>,
}

pub struct LinkStateIgp {
    lsdb: Vec<Vec<Option<Arc<Lsa>>>>,
    own_seq: Vec<u32>,
    spf_pending: Vec<bool>,
    last_spf: Vec<Option<Nanos>>,
    pub fib: Fib,
    pub spf_runs: u64,
    pub lsa_deliveries: u64,
}

impl LinkStateIgp {
    pub fn new(v: &View) -> Self {
        let n = v.topo.routers.len();
        LinkStateIgp {
            lsdb: v.topo.routers.iter().map(|r| vec![None; v.layout.per_as[r.asys.idx()].routers.len()]).collect(),
            own_seq: vec![0; n],
            spf_pending: vec![false; n],
            last_spf: vec![None; n],
            fib: Fib::new(v.topo, v.layout),
            spf_runs: 0,
            lsa_deliveries: 0,
        }
    }

    pub fn lsdb_size(&self, r: RouterId) -> usize {
        self.lsdb[r.idx()].iter().flatten().count()
    }

    fn build_lsa(&mut self, r: RouterId, v: &View) -> Arc<Lsa> {
        let my_as = v.topo.as_of(r);
        let mut adj = Vec::new();
        let mut exits = Vec::new();
        for (l, n) in v.live_links(r) {
            let cost = v.topo.link(l).cost;
            let n_as = v.topo.as_of(n);
            if n_as == my_as {
                adj.push((n, l, cost));
            } else {
                exits.push((n_as, l, cost));
            }
        }
        self.own_seq[r.idx()] += 1;
        Arc::new(Lsa { origin: r, seq: self.own_seq[r.idx()], adj, exits })
    }

    /// Re-describe `r`'s links and flood the result. Called whenever one of
    /// its adjacencies changes.
    pub fn originate(&mut self, r: RouterId, v: &View, q: &mut EventQueue) {
        if !v.alive[r.idx()] {
            return;
        }
        let lsa = self.build_lsa(r, v);
        let slot = v.layout.local[r.idx()] as usize;
        self.lsdb[r.idx()][slot] = Some(lsa.clone());
        self.flood(r, None, &lsa, v, q);
        self.schedule_spf(r, v, q);
    }

    fn flood(&mut self, r: RouterId, except: Option<LinkId>, lsa: &Arc<Lsa>, v: &View, q: &mut EventQueue) {
        let my_as = v.topo.as_of(r);
        for (l, n) in v.live_links(r) {
            if Some(l) == except || v.topo.as_of(n) != my_as {
                continue;
            }
            let delay = crate::time::from_secs(v.links.propagation(l)) + v.timers.lsa_processing;
            q.after(delay, Event::Lsa { to: n, via: l, lsa: lsa.clone() });
        }
    }

    pub fn receive(&mut self, to: RouterId, via: LinkId, lsa: Arc<Lsa>, v: &View, q: &mut EventQueue) {
        // A dead router hears nothing; a link that has not come up from our
        // side yet has no adjacency to deliver on.
        if !v.alive[to.idx()] || !v.links.seen_up_from(via, to) {
            return;
        }
        self.lsa_deliveries += 1;

        if lsa.origin == to {
            // Our own LSA coming back with a sequence number we do not
            // remember issuing: we restarted and the network still has the
            // old incarnation. Jump past it and re-originate.
            if lsa.seq >= self.own_seq[to.idx()] {
                self.own_seq[to.idx()] = lsa.seq;
                self.originate(to, v, q);
            }
            return;
        }

        let slot = v.layout.local[lsa.origin.idx()] as usize;
        let stored = &mut self.lsdb[to.idx()][slot];
        if stored.as_ref().is_some_and(|s| s.seq >= lsa.seq) {
            return;
        }
        *stored = Some(lsa.clone());
        self.flood(to, Some(via), &lsa, v, q);
        self.schedule_spf(to, v, q);
    }

    /// A new adjacency: describe ourselves again and bring the neighbour's
    /// database up to date with ours (OSPF's database exchange, minus the
    /// handshake).
    pub fn adjacency_up(&mut self, r: RouterId, l: LinkId, v: &View, q: &mut EventQueue) {
        if !v.alive[r.idx()] {
            return;
        }
        self.originate(r, v, q);
        let n = v.links.other_end(l, r);
        if v.topo.as_of(n) != v.topo.as_of(r) {
            return;
        }
        let delay = crate::time::from_secs(v.links.propagation(l)) + v.timers.lsa_processing;
        for lsa in self.lsdb[r.idx()].iter().flatten() {
            q.after(delay, Event::Lsa { to: n, via: l, lsa: lsa.clone() });
        }
    }

    fn schedule_spf(&mut self, r: RouterId, v: &View, q: &mut EventQueue) {
        if self.spf_pending[r.idx()] {
            return;
        }
        self.spf_pending[r.idx()] = true;
        let now = q.now();
        let earliest = now + v.timers.spf_initial;
        let at = match self.last_spf[r.idx()] {
            Some(last) => earliest.max(last + v.timers.spf_hold),
            None => earliest,
        };
        q.at(at, Event::Spf { router: r });
    }

    /// Dijkstra over `r`'s own LSDB, then schedule the result to land in the
    /// FIB a little later.
    pub fn run_spf(&mut self, r: RouterId, v: &View, q: &mut EventQueue) {
        self.spf_pending[r.idx()] = false;
        if !v.alive[r.idx()] {
            return;
        }
        self.last_spf[r.idx()] = Some(q.now());
        self.spf_runs += 1;

        let lay = v.layout.of(v.topo, r);
        let n = lay.routers.len();
        let db = &self.lsdb[r.idx()];

        // An adjacency only counts if both ends report it (OSPF's two-way
        // check). Otherwise a router that died keeps being routed through
        // until its stale LSA ages out.
        let reports = |u: usize, peer: RouterId, l: LinkId| {
            db[u].as_ref().is_some_and(|lsa| lsa.adj.iter().any(|&(x, y, _)| x == peer && y == l))
        };
        let mut arcs = Vec::new();
        for (u, lsa) in db.iter().enumerate() {
            let Some(lsa) = lsa else { continue };
            for &(nbr, l, cost) in &lsa.adj {
                let w = v.layout.local[nbr.idx()] as usize;
                if reports(w, lay.routers[u], l) {
                    arcs.push((u, w, cost as u64, l.0));
                }
            }
        }
        let g = Csr::from_arcs(n, arcs);
        let me = v.layout.local[r.idx()] as usize;
        let sp = dijkstra(&g, me);

        let mut next = vec![NONE; lay.targets()].into_boxed_slice();
        let mut cost = vec![u32::MAX; lay.targets()].into_boxed_slice();
        for t in 0..n {
            if sp.reachable(t) {
                next[t] = sp.first_edge[t];
                cost[t] = sp.dist[t].min(u32::MAX as u64 - 1) as u32;
            }
        }
        // Hot-potato egress: for each neighbouring AS, the cheapest
        // (distance to border + exit link cost), ties to the lower link id.
        for (u, lsa) in db.iter().enumerate() {
            let Some(lsa) = lsa else { continue };
            if sp.dist[u] == UNREACHABLE {
                continue;
            }
            for &(to_as, l, c) in &lsa.exits {
                let Some(t) = lay.exit_target(to_as) else { continue };
                let total = (sp.dist[u] + c as u64).min(u32::MAX as u64 - 1) as u32;
                let hop = if u == me { l.0 } else { sp.first_edge[u] };
                if total < cost[t] || (total == cost[t] && hop < next[t]) {
                    cost[t] = total;
                    next[t] = hop;
                }
            }
        }
        q.after(v.timers.fib_install, Event::FibInstall { router: r, next, cost });
    }

    /// Returns true if the forwarding behaviour actually changed.
    pub fn install(&mut self, r: RouterId, next: Box<[u32]>, cost: Box<[u32]>) -> bool {
        let changed = self.fib.next[r.idx()] != next;
        self.fib.next[r.idx()] = next;
        self.fib.cost[r.idx()] = cost;
        changed
    }

    /// A crash loses all protocol state. The neighbours notice the dead
    /// adjacencies through link detection, not through anything we send.
    pub fn router_down(&mut self, r: RouterId) {
        self.lsdb[r.idx()].fill(None);
        self.fib.clear_router(r);
        // Sequence numbers do not survive a reboot either; the fight-back
        // path in `receive` handles the consequences.
        self.own_seq[r.idx()] = 0;
        self.spf_pending[r.idx()] = false;
    }
}
