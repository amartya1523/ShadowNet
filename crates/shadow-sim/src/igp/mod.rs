//! Interior routing. Two classic protocols plus the shared bookkeeping.
//!
//! Every router in an AS routes towards a fixed set of *targets*: each router
//! of its own AS, and one "exit" per neighbouring AS. Exits are what BGP
//! hands us: "to reach that prefix, leave towards AS 64530". How you get to
//! the right border router is the IGP's business. Indexing targets densely
//! means a FIB row is just a slice.

pub mod distvec;
pub mod linkstate;

use shadow_graph::algo::NONE;
use shadow_graph::{AsId, LinkId, RouterId, Topology};

use crate::links::LinkTable;
use crate::time::Timers;

pub struct AsLayout {
    pub routers: Vec<RouterId>,
    /// Neighbouring ASes, sorted, one exit target each.
    pub exits: Vec<AsId>,
    exit_slot: Vec<u16>,
}

impl AsLayout {
    #[inline]
    pub fn targets(&self) -> usize {
        self.routers.len() + self.exits.len()
    }

    #[inline]
    pub fn exit_target(&self, to: AsId) -> Option<usize> {
        match self.exit_slot[to.idx()] {
            u16::MAX => None,
            s => Some(self.routers.len() + s as usize),
        }
    }

    /// The neighbour AS an exit target stands for.
    pub fn exit_as(&self, target: usize) -> Option<AsId> {
        target.checked_sub(self.routers.len()).map(|i| self.exits[i])
    }
}

pub struct Layout {
    /// Index of each router inside its own AS.
    pub local: Vec<u16>,
    pub per_as: Vec<AsLayout>,
}

impl Layout {
    pub fn new(topo: &Topology) -> Self {
        let mut local = vec![0u16; topo.routers.len()];
        let per_as = topo
            .ases
            .iter()
            .map(|a| {
                for (i, r) in a.routers.iter().enumerate() {
                    local[r.idx()] = i as u16;
                }
                let mut exits: Vec<AsId> = a.neighbors.iter().map(|&(n, _)| n).collect();
                exits.sort_unstable();
                let mut exit_slot = vec![u16::MAX; topo.ases.len()];
                for (i, e) in exits.iter().enumerate() {
                    exit_slot[e.idx()] = i as u16;
                }
                AsLayout { routers: a.routers.clone(), exits, exit_slot }
            })
            .collect();
        Layout { local, per_as }
    }

    #[inline]
    pub fn of(&self, topo: &Topology, r: RouterId) -> &AsLayout {
        &self.per_as[topo.as_of(r).idx()]
    }

    /// Target index of `dst` (a router) as seen from inside its own AS.
    #[inline]
    pub fn router_target(&self, dst: RouterId) -> usize {
        self.local[dst.idx()] as usize
    }
}

/// Read-only view of the world handed to protocol code, so the protocols
/// cannot reach into each other's state.
pub struct View<'a> {
    pub topo: &'a Topology,
    pub layout: &'a Layout,
    pub links: &'a LinkTable,
    pub alive: &'a [bool],
    pub timers: &'a Timers,
}

impl View<'_> {
    /// Links of `r` that `r` currently believes are up, split into internal
    /// adjacencies and exits.
    pub fn live_links(&self, r: RouterId) -> impl Iterator<Item = (LinkId, RouterId)> + '_ {
        self.topo.neighbors(r).iter().copied().filter(move |&(l, _)| self.links.seen_up_from(l, r))
    }
}

/// A forwarding table per router: for each target, the outgoing link (or
/// `NONE`) and the protocol's cost to get there.
pub struct Fib {
    pub next: Vec<Box<[u32]>>,
    pub cost: Vec<Box<[u32]>>,
}

impl Fib {
    pub fn new(topo: &Topology, layout: &Layout) -> Self {
        let rows = |r: &shadow_graph::Router| layout.per_as[r.asys.idx()].targets();
        Fib {
            next: topo.routers.iter().map(|r| vec![NONE; rows(r)].into()).collect(),
            cost: topo.routers.iter().map(|r| vec![u32::MAX; rows(r)].into()).collect(),
        }
    }

    #[inline]
    pub fn lookup(&self, r: RouterId, target: usize) -> Option<LinkId> {
        match self.next[r.idx()][target] {
            NONE => None,
            l => Some(LinkId(l)),
        }
    }

    pub fn clear_router(&mut self, r: RouterId) {
        self.next[r.idx()].fill(NONE);
        self.cost[r.idx()].fill(u32::MAX);
    }
}
