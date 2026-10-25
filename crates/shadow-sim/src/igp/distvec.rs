//! Distance-vector routing, modelled on RIP.
//!
//! Hop count metric, infinity at 16, split horizon with poisoned reverse,
//! triggered updates plus periodic full updates. It is here as the control
//! group: it converges to fewest-hops rather than lowest-latency paths, it
//! needs the periodic updates to find alternatives after a failure, and with
//! the right failure it counts to infinity. All of that is visible in the
//! incident timings.

use std::sync::Arc;

use shadow_graph::algo::NONE;
use shadow_graph::{LinkId, RouterId};

use super::{Fib, View};
use crate::event::{Event, EventQueue};
use crate::time::{MS, Nanos};

pub const INFINITY: u8 = 16;

pub struct DistanceVector {
    metric: Vec<Box<[u8]>>,
    send_pending: Vec<bool>,
    periodic_armed: Vec<bool>,
    pub fib: Fib,
    pub updates_sent: u64,
    /// Set whenever any FIB entry moves; the world clears it.
    pub changed: bool,
}

impl DistanceVector {
    pub fn new(v: &View) -> Self {
        let n = v.topo.routers.len();
        let fib = Fib::new(v.topo, v.layout);
        DistanceVector {
            metric: fib.next.iter().map(|row| vec![INFINITY; row.len()].into()).collect(),
            send_pending: vec![false; n],
            periodic_armed: vec![false; n],
            fib,
            updates_sent: 0,
            changed: false,
        }
    }

    fn set(&mut self, r: RouterId, t: usize, metric: u8, via: u32) {
        let row = &mut self.metric[r.idx()];
        if row[t] == metric && self.fib.next[r.idx()][t] == via {
            return;
        }
        row[t] = metric;
        self.fib.next[r.idx()][t] = via;
        self.fib.cost[r.idx()][t] = if metric >= INFINITY { u32::MAX } else { metric as u32 };
        self.changed = true;
    }

    /// Fresh start: we know ourselves and our directly attached exits.
    pub fn boot(&mut self, r: RouterId, v: &View, q: &mut EventQueue) {
        if !v.alive[r.idx()] {
            return;
        }
        let lay = v.layout.of(v.topo, r);
        for t in 0..lay.targets() {
            self.set(r, t, INFINITY, NONE);
        }
        self.set(r, v.layout.router_target(r), 0, NONE);
        self.refresh_exits(r, v);
        self.trigger(r, v, q);
        if !self.periodic_armed[r.idx()] {
            self.periodic_armed[r.idx()] = true;
            // Stagger periodic updates by router id so 1700 routers do not
            // all fire on the same nanosecond.
            let offset = (r.0 as Nanos * 7919 * MS) % v.timers.dv_periodic;
            q.after(v.timers.dv_periodic + offset, Event::DvPeriodic { router: r });
        }
    }

    /// Directly connected exits always win with metric 1.
    fn refresh_exits(&mut self, r: RouterId, v: &View) {
        let my_as = v.topo.as_of(r);
        let lay = v.layout.of(v.topo, r);
        let mut direct: Vec<(usize, LinkId)> = v
            .live_links(r)
            .filter(|&(_, n)| v.topo.as_of(n) != my_as)
            .filter_map(|(l, n)| Some((lay.exit_target(v.topo.as_of(n))?, l)))
            .collect();
        direct.sort_unstable();
        direct.dedup_by_key(|e| e.0);
        for (t, l) in direct {
            self.set(r, t, 1, l.0);
        }
    }

    fn vector_for(&self, r: RouterId, towards: LinkId) -> Arc<[u8]> {
        let next = &self.fib.next[r.idx()];
        self.metric[r.idx()]
            .iter()
            .zip(next.iter())
            .map(|(&m, &via)| if via == towards.0 { INFINITY } else { m })
            .collect()
    }

    fn trigger(&mut self, r: RouterId, v: &View, q: &mut EventQueue) {
        if !self.send_pending[r.idx()] {
            self.send_pending[r.idx()] = true;
            q.after(v.timers.dv_triggered, Event::DvSend { router: r });
        }
    }

    pub fn send(&mut self, r: RouterId, v: &View, q: &mut EventQueue) {
        self.send_pending[r.idx()] = false;
        if !v.alive[r.idx()] {
            return;
        }
        let my_as = v.topo.as_of(r);
        let neighbours: Vec<(LinkId, RouterId)> = v
            .live_links(r)
            .filter(|&(_, n)| v.topo.as_of(n) == my_as)
            .collect();
        for (l, n) in neighbours {
            let vector = self.vector_for(r, l);
            let delay = crate::time::from_secs(v.links.propagation(l)) + MS;
            q.after(delay, Event::DvRecv { to: n, via: l, vector });
            self.updates_sent += 1;
        }
    }

    pub fn periodic(&mut self, r: RouterId, v: &View, q: &mut EventQueue) {
        if !v.alive[r.idx()] {
            self.periodic_armed[r.idx()] = false;
            return;
        }
        self.send(r, v, q);
        q.after(v.timers.dv_periodic, Event::DvPeriodic { router: r });
    }

    pub fn receive(&mut self, to: RouterId, via: LinkId, vector: &[u8], v: &View, q: &mut EventQueue) {
        if !v.alive[to.idx()] || !v.links.seen_up_from(via, to) {
            return;
        }
        let me = v.layout.router_target(to);
        let mut moved = false;
        for (t, &theirs) in vector.iter().enumerate() {
            if t == me {
                continue;
            }
            let m = theirs.saturating_add(1).min(INFINITY);
            let (cur_m, cur_via) = (self.metric[to.idx()][t], self.fib.next[to.idx()][t]);
            if cur_via == via.0 {
                // News from our current next hop is believed, good or bad.
                if m != cur_m {
                    let via = if m >= INFINITY { NONE } else { via.0 };
                    self.set(to, t, m, via);
                    moved = true;
                }
            } else if m < cur_m {
                self.set(to, t, m, via.0);
                moved = true;
            }
        }
        if moved {
            self.trigger(to, v, q);
        }
    }

    pub fn link_down(&mut self, r: RouterId, l: LinkId, v: &View, q: &mut EventQueue) {
        if !v.alive[r.idx()] {
            return;
        }
        let row: Vec<usize> = self.fib.next[r.idx()]
            .iter()
            .enumerate()
            .filter(|&(_, &via)| via == l.0)
            .map(|(t, _)| t)
            .collect();
        for t in row {
            self.set(r, t, INFINITY, NONE);
        }
        self.refresh_exits(r, v);
        self.trigger(r, v, q);
    }

    pub fn link_up(&mut self, r: RouterId, _l: LinkId, v: &View, q: &mut EventQueue) {
        if !v.alive[r.idx()] {
            return;
        }
        self.refresh_exits(r, v);
        self.trigger(r, v, q);
    }

    pub fn router_down(&mut self, r: RouterId) {
        let n = self.metric[r.idx()].len();
        for t in 0..n {
            self.set(r, t, INFINITY, NONE);
        }
        self.send_pending[r.idx()] = false;
    }
}
