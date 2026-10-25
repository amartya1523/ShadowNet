//! Per-link runtime state: is it up, does each end *know* whether it is up,
//! and the two fluid queues (one per direction).

use shadow_graph::{LinkId, RouterId, Topology};

/// Why a link is down. A link can be down for several reasons at once (cut
/// by an operator *and* one end crashed), and only comes back when all of
/// them are gone.
pub mod cause {
    pub const MANUAL: u8 = 1 << 0;
    pub const MONKEY: u8 = 1 << 1;
}

/// Seconds of line-rate buffering per output queue. 50 ms is on the generous
/// side of what router vendors ship; it is what makes bufferbloat visible.
pub const BUFFER_S: f64 = 0.050;

#[derive(Clone, Debug, Default)]
pub struct Direction {
    /// Sum of flow demand entering this queue this tick.
    pub offered_bps: f64,
    pub backlog_bits: f64,
    pub queue_s: f64,
    /// Fraction dropped (buffer overflow plus injected loss).
    pub loss: f64,
    /// Delivered / capacity, 0..=1.
    pub util: f64,
}

#[derive(Clone, Debug)]
pub struct LinkState {
    pub cut: u8,
    /// Physical state, derived from `cut` and the liveness of both ends.
    pub up: bool,
    /// What each end believes (index 0 = `a`, 1 = `b`). Lags `up` by the
    /// detection delay. The gap between the two is where black holes live.
    pub seen_up: [bool; 2],
    pub extra_delay_s: f64,
    pub extra_loss: f64,
    /// Bumped whenever an injected impairment is replaced, so a stale
    /// `Clear` event can tell it is stale.
    pub impairment_token: u32,
    pub repair_token: u32,
    pub dir: [Direction; 2],
}

pub struct LinkTable {
    pub state: Vec<LinkState>,
    capacity: Vec<f64>,
    delay: Vec<f64>,
    ends: Vec<(RouterId, RouterId)>,
}

impl LinkTable {
    pub fn new(topo: &Topology) -> Self {
        LinkTable {
            state: topo
                .links
                .iter()
                .map(|_| LinkState {
                    cut: 0,
                    up: true,
                    seen_up: [true, true],
                    extra_delay_s: 0.0,
                    extra_loss: 0.0,
                    impairment_token: 0,
                    repair_token: 0,
                    dir: Default::default(),
                })
                .collect(),
            capacity: topo.links.iter().map(|l| l.capacity_bps).collect(),
            delay: topo.links.iter().map(|l| l.delay_s).collect(),
            ends: topo.links.iter().map(|l| (l.a, l.b)).collect(),
        }
    }

    #[inline]
    pub fn up(&self, l: LinkId) -> bool {
        self.state[l.idx()].up
    }

    /// Which end of `l` is `r`: 0 for `a`, 1 for `b`.
    #[inline]
    pub fn end_of(&self, l: LinkId, r: RouterId) -> u8 {
        if self.ends[l.idx()].0 == r { 0 } else { 1 }
    }

    #[inline]
    pub fn seen_up_from(&self, l: LinkId, r: RouterId) -> bool {
        self.state[l.idx()].seen_up[self.end_of(l, r) as usize]
    }

    /// Index of the directional queue for traffic leaving `from` over `l`.
    #[inline]
    pub fn queue_index(&self, l: LinkId, from: RouterId) -> u32 {
        l.0 * 2 + self.end_of(l, from) as u32
    }

    #[inline]
    pub fn queue(&self, q: u32) -> &Direction {
        &self.state[(q / 2) as usize].dir[(q % 2) as usize]
    }

    pub fn other_end(&self, l: LinkId, r: RouterId) -> RouterId {
        let (a, b) = self.ends[l.idx()];
        if a == r { b } else { a }
    }

    /// Router at the far end of directional queue `q`.
    #[inline]
    pub fn queue_target(&self, q: u32) -> RouterId {
        let (a, b) = self.ends[(q / 2) as usize];
        if q % 2 == 0 { b } else { a }
    }

    /// Current one-way delay through queue `q`: propagation, queueing and
    /// whatever has been injected.
    #[inline]
    pub fn hop_delay(&self, q: u32) -> f64 {
        let l = (q / 2) as usize;
        self.delay[l] + self.state[l].extra_delay_s + self.state[l].dir[(q % 2) as usize].queue_s
    }

    pub fn capacity(&self, l: LinkId) -> f64 {
        self.capacity[l.idx()]
    }

    pub fn propagation(&self, l: LinkId) -> f64 {
        self.delay[l.idx()]
    }

    pub fn clear_offered(&mut self) {
        for s in &mut self.state {
            s.dir[0].offered_bps = 0.0;
            s.dir[1].offered_bps = 0.0;
        }
    }

    #[inline]
    pub fn offer(&mut self, q: u32, bps: f64) {
        self.state[(q / 2) as usize].dir[(q % 2) as usize].offered_bps += bps;
    }

    /// Integrates every queue over `dt` seconds.
    ///
    /// Fluid model: the backlog grows at (offered - capacity) and is clamped
    /// to [0, buffer]. Anything that would overflow the buffer is dropped.
    /// Queueing delay is the time to drain the current backlog.
    pub fn integrate(&mut self, dt: f64) {
        for (i, s) in self.state.iter_mut().enumerate() {
            let cap = self.capacity[i];
            let buffer = cap * BUFFER_S;
            for d in &mut s.dir {
                if !s.up {
                    d.backlog_bits = 0.0;
                    d.queue_s = 0.0;
                    d.util = 0.0;
                    d.loss = if d.offered_bps > 0.0 { 1.0 } else { 0.0 };
                    continue;
                }
                let excess = (d.offered_bps - cap) * dt;
                let unclamped = d.backlog_bits + excess;
                let overflow = (unclamped - buffer).max(0.0);
                d.backlog_bits = unclamped.clamp(0.0, buffer);
                d.queue_s = d.backlog_bits / cap;
                let arrived = d.offered_bps * dt;
                let drop = if arrived > 0.0 { overflow / arrived } else { 0.0 };
                d.loss = 1.0 - (1.0 - drop) * (1.0 - s.extra_loss);
                d.util = (d.offered_bps * (1.0 - drop)).min(cap) / cap;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shadow_graph::TopologyConfig;

    fn table() -> (Topology, LinkTable) {
        let t = Topology::generate(&TopologyConfig::small(1));
        let lt = LinkTable::new(&t);
        (t, lt)
    }

    #[test]
    fn underloaded_queue_stays_empty() {
        let (t, mut lt) = table();
        let cap = t.links[0].capacity_bps;
        for _ in 0..100 {
            lt.clear_offered();
            lt.offer(0, cap * 0.5);
            lt.integrate(0.05);
        }
        let d = lt.queue(0);
        assert_eq!(d.backlog_bits, 0.0);
        assert!((d.util - 0.5).abs() < 1e-9);
        assert_eq!(d.loss, 0.0);
    }

    #[test]
    fn overload_fills_buffer_then_drops() {
        let (t, mut lt) = table();
        let cap = t.links[0].capacity_bps;
        lt.clear_offered();
        lt.offer(0, cap * 1.5);
        lt.integrate(0.05);
        // Half a capacity of excess for 50 ms is exactly 25 ms of backlog,
        // still inside the 50 ms buffer: no loss yet.
        assert!((lt.queue(0).queue_s - 0.025).abs() < 1e-9);
        assert_eq!(lt.queue(0).loss, 0.0);
        for _ in 0..10 {
            lt.clear_offered();
            lt.offer(0, cap * 1.5);
            lt.integrate(0.05);
        }
        let d = lt.queue(0);
        assert!((d.queue_s - BUFFER_S).abs() < 1e-9);
        // Steady state: a third of what arrives is dropped.
        assert!((d.loss - 1.0 / 3.0).abs() < 1e-6, "{}", d.loss);
        assert!((d.util - 1.0).abs() < 1e-9);
    }

    #[test]
    fn queue_drains_when_load_drops() {
        let (t, mut lt) = table();
        let cap = t.links[0].capacity_bps;
        for _ in 0..20 {
            lt.clear_offered();
            lt.offer(0, cap * 2.0);
            lt.integrate(0.05);
        }
        for _ in 0..3 {
            lt.clear_offered();
            lt.offer(0, cap * 0.5);
            lt.integrate(0.05);
        }
        // Draining at half capacity for 150 ms clears 75 ms worth: empty.
        assert_eq!(lt.queue(0).backlog_bits, 0.0);
    }

    #[test]
    fn directions_are_independent() {
        let (t, lt) = table();
        let l = &t.links[0];
        assert_eq!(lt.queue_index(l.id, l.a), 0);
        assert_eq!(lt.queue_index(l.id, l.b), 1);
        assert_eq!(lt.queue_target(0), l.b);
        assert_eq!(lt.queue_target(1), l.a);
    }
}
