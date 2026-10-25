//! The forwarding walk: what happens to a packet, hop by hop, given the
//! routing state *as it is right now in each router it visits*.
//!
//! This is deliberately not "look up the shortest path". During convergence
//! two routers can disagree, and then the walk bounces between them until
//! the TTL runs out, exactly as real traffic does.

use shadow_graph::{RouterId, Topology};

use crate::bgp::Bgp;
use crate::brain::qrouting::QRouting;
use crate::igp::{Fib, Layout};
use crate::links::LinkTable;
use crate::traffic::PathStatus;
use crate::world::RoutingMode;

pub const MAX_HOPS: usize = 64;

pub struct Forwarder<'a> {
    pub topo: &'a Topology,
    pub layout: &'a Layout,
    pub links: &'a LinkTable,
    pub alive: &'a [bool],
    pub bgp: &'a Bgp,
    pub fib: &'a Fib,
    pub qr: &'a QRouting,
    pub mode: RoutingMode,
}

impl Forwarder<'_> {
    /// Fills `out` with directional queue indices and reports how the walk
    /// ended. On failure `out` still holds the hops taken before it: that
    /// traffic used real capacity before it died.
    pub fn walk(&self, src: RouterId, dst: RouterId, flow_hash: u64, out: &mut Vec<u32>) -> PathStatus {
        out.clear();
        if !self.alive[src.idx()] {
            return PathStatus::Blackhole;
        }
        let dst_as = self.topo.as_of(dst);
        let mut r = src;
        for _ in 0..MAX_HOPS {
            if r == dst {
                return PathStatus::Ok;
            }
            let here = self.topo.as_of(r);
            let lay = &self.layout.per_as[here.idx()];
            let target = if here == dst_as {
                self.layout.router_target(dst)
            } else {
                let Some(next_as) = self.bgp.best(here, dst_as).and_then(|b| b.next_as()) else {
                    return PathStatus::Unreachable;
                };
                match lay.exit_target(next_as) {
                    Some(t) => t,
                    None => return PathStatus::Unreachable,
                }
            };
            let link = match self.mode {
                RoutingMode::Qrouting => self.qr.choose(r, target, flow_hash),
                _ => self.fib.lookup(r, target),
            };
            let Some(link) = link else {
                return PathStatus::Unreachable;
            };
            out.push(self.links.queue_index(link, r));
            if !self.links.up(link) {
                return PathStatus::Blackhole;
            }
            r = self.links.other_end(link, r);
        }
        PathStatus::Loop
    }
}
