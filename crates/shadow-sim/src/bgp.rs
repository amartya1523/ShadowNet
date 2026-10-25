//! Inter-domain routing: a path-vector protocol with Gao-Rexford policies.
//!
//! One speaker per AS, one prefix per AS. Sessions ride on the physical
//! interconnects: a session is up while at least one link between the two
//! ASes is up as seen from both ends.
//!
//! The decision process is the part of BGP that matters for traffic:
//!
//! 1. local preference by business relationship: customer > peer > provider
//!    (customers pay us, providers charge us);
//! 2. shortest AS path;
//! 3. lowest neighbour ASN, as a deterministic tie-break.
//!
//! Export follows the valley-free rule: routes from customers (and our own
//! prefix) go to everybody; routes from peers and providers go to customers
//! only. Under that rule BGP is guaranteed to converge, which is a nice
//! thing to be able to rely on in a simulator.

use std::sync::Arc;

use shadow_graph::{AsId, Relationship, Topology};

use crate::event::{Event, EventQueue};
use crate::time::{Nanos, from_secs};

pub type AsPath = Arc<[AsId]>;

/// A batch of announcements and withdrawals for one neighbour.
#[derive(Debug, Default)]
pub struct Update {
    pub routes: Vec<(AsId, Option<AsPath>)>,
}

#[derive(Clone, Debug)]
pub struct Best {
    /// Neighbour index we learned it from, or `None` for our own prefix.
    pub from: Option<u16>,
    /// AS path as we would advertise it *minus* ourselves. Empty for our own
    /// prefix; first element is the next-hop AS otherwise.
    pub path: AsPath,
}

impl Best {
    pub fn next_as(&self) -> Option<AsId> {
        self.path.first().copied()
    }
}

struct Speaker {
    asys: AsId,
    neighbors: Vec<(AsId, Relationship)>,
    session_up: Vec<bool>,
    /// Bumped on every session transition so updates that were in flight
    /// when a session flapped are recognised as belonging to a dead session.
    epoch: Vec<u32>,
    rib_in: Vec<Vec<Option<AsPath>>>,
    best: Vec<Option<Best>>,
    adj_out: Vec<Vec<Option<AsPath>>>,
    dirty: Vec<Vec<bool>>,
    dirty_any: Vec<bool>,
    mrai_armed: Vec<bool>,
}

pub struct Bgp {
    speakers: Vec<Speaker>,
    /// Neighbour index of B inside A's table, per (A, B). Dense because the
    /// AS count is small (hundreds).
    slot: Vec<Vec<u16>>,
    asn: Vec<u32>,
    delay: Vec<Vec<Nanos>>,
    pub updates_sent: u64,
    pub best_changes: u64,
    /// Set whenever any best path changes; the world clears it.
    pub changed: bool,
}

fn preference(rel: Relationship) -> u8 {
    match rel {
        Relationship::Customer => 3,
        Relationship::Peer => 2,
        Relationship::Provider => 1,
    }
}

impl Bgp {
    pub fn new(topo: &Topology, processing: Nanos) -> Self {
        let n = topo.ases.len();
        let mut slot = vec![vec![u16::MAX; n]; n];
        let mut delay = Vec::with_capacity(n);
        let speakers = topo
            .ases
            .iter()
            .map(|a| {
                let mut neighbors = a.neighbors.clone();
                neighbors.sort_by_key(|&(b, _)| b);
                let mut d = Vec::with_capacity(neighbors.len());
                for (i, &(b, _)) in neighbors.iter().enumerate() {
                    slot[a.id.idx()][b.idx()] = i as u16;
                    // Updates travel over the shortest interconnect.
                    let prop = topo
                        .links_between(a.id, b)
                        .iter()
                        .map(|&l| topo.link(l).delay_s)
                        .fold(f64::INFINITY, f64::min);
                    d.push(processing + from_secs(prop));
                }
                delay.push(d);
                let k = neighbors.len();
                Speaker {
                    asys: a.id,
                    neighbors,
                    session_up: vec![false; k],
                    epoch: vec![0; k],
                    rib_in: vec![vec![None; n]; k],
                    best: vec![None; n],
                    adj_out: vec![vec![None; n]; k],
                    dirty: vec![vec![false; n]; k],
                    dirty_any: vec![false; k],
                    mrai_armed: vec![false; k],
                }
            })
            .collect();
        Bgp {
            speakers,
            slot,
            asn: topo.ases.iter().map(|a| a.asn).collect(),
            delay,
            updates_sent: 0,
            best_changes: 0,
            changed: false,
        }
    }

    #[inline]
    pub fn best(&self, at: AsId, prefix: AsId) -> Option<&Best> {
        self.speakers[at.idx()].best[prefix.idx()].as_ref()
    }

    pub fn session_up(&self, a: AsId, b: AsId) -> bool {
        match self.slot[a.idx()][b.idx()] {
            u16::MAX => false,
            i => self.speakers[a.idx()].session_up[i as usize],
        }
    }

    /// Originate our own prefix. Called once per AS at boot.
    pub fn originate(&mut self, a: AsId, mrai: Nanos, q: &mut EventQueue) {
        let s = &mut self.speakers[a.idx()];
        s.best[a.idx()] = Some(Best {
            from: None,
            path: Arc::from([]),
        });
        self.changed = true;
        self.mark_all_neighbors(a, a);
        self.flush_all(a, mrai, q);
    }

    pub fn set_session(&mut self, a: AsId, b: AsId, up: bool, mrai: Nanos, q: &mut EventQueue) {
        for (x, y) in [(a, b), (b, a)] {
            let i = self.slot[x.idx()][y.idx()];
            if i == u16::MAX {
                continue;
            }
            let i = i as usize;
            let s = &mut self.speakers[x.idx()];
            if s.session_up[i] == up {
                continue;
            }
            s.session_up[i] = up;
            s.epoch[i] = s.epoch[i].wrapping_add(1);
            s.adj_out[i].iter_mut().for_each(|p| *p = None);
            s.mrai_armed[i] = false;
            if up {
                // Fresh session: advertise everything we have.
                let n = s.best.len();
                for p in 0..n {
                    s.dirty[i][p] = s.best[p].is_some();
                }
                s.dirty_any[i] = true;
                self.flush(x, i, mrai, q);
            } else {
                // Everything learned over the session is gone.
                let learned: Vec<usize> = (0..s.rib_in[i].len()).filter(|&p| s.rib_in[i][p].is_some()).collect();
                for &p in &learned {
                    s.rib_in[i][p] = None;
                }
                s.dirty[i].iter_mut().for_each(|d| *d = false);
                s.dirty_any[i] = false;
                for p in learned {
                    self.decide(x, AsId::from_idx(p));
                }
                self.flush_all(x, mrai, q);
            }
        }
    }

    pub fn receive(&mut self, to: AsId, from: AsId, epoch: u32, update: &Update, mrai: Nanos, q: &mut EventQueue) {
        let i = self.slot[to.idx()][from.idx()];
        if i == u16::MAX {
            return;
        }
        let i = i as usize;
        {
            let s = &self.speakers[to.idx()];
            // `epoch` is the sender's epoch for this session; both sides bump
            // together on every transition, so a mismatch means the update
            // left before the session last changed state.
            if !s.session_up[i] || s.epoch[i] != epoch {
                return;
            }
        }
        for (prefix, path) in &update.routes {
            let path = path.as_ref().filter(|p| !p.contains(&to)).cloned();
            self.speakers[to.idx()].rib_in[i][prefix.idx()] = path;
            self.decide(to, *prefix);
        }
        self.flush_all(to, mrai, q);
    }

    pub fn mrai_expired(&mut self, a: AsId, neighbor: u16, mrai: Nanos, q: &mut EventQueue) {
        let i = neighbor as usize;
        self.speakers[a.idx()].mrai_armed[i] = false;
        if self.speakers[a.idx()].dirty_any[i] {
            self.flush(a, i, mrai, q);
        }
    }

    /// Run the decision process for one prefix. Marks the prefix dirty
    /// towards every neighbour if the winner changed.
    fn decide(&mut self, a: AsId, prefix: AsId) {
        if a == prefix {
            return;
        }
        let s = &self.speakers[a.idx()];
        let mut winner: Option<(u8, usize, u32, usize)> = None;
        for (i, &(nbr, rel)) in s.neighbors.iter().enumerate() {
            if !s.session_up[i] {
                continue;
            }
            let Some(path) = &s.rib_in[i][prefix.idx()] else { continue };
            // Higher preference, then shorter path, then lower ASN. Pack it
            // so a single comparison does the whole tie-break chain.
            let key = (preference(rel), usize::MAX - path.len(), u32::MAX - self.asn[nbr.idx()], i);
            if winner.is_none_or(|w| (key.0, key.1, key.2) > (w.0, w.1, w.2)) {
                winner = Some(key);
            }
        }
        let new = winner.map(|(_, _, _, i)| Best {
            from: Some(i as u16),
            path: s.rib_in[i][prefix.idx()].clone().unwrap(),
        });
        let old = &s.best[prefix.idx()];
        let same = match (old, &new) {
            (None, None) => true,
            (Some(o), Some(n)) => o.from == n.from && o.path == n.path,
            _ => false,
        };
        if same {
            return;
        }
        self.speakers[a.idx()].best[prefix.idx()] = new;
        self.best_changes += 1;
        self.changed = true;
        self.mark_all_neighbors(a, prefix);
    }

    fn mark_all_neighbors(&mut self, a: AsId, prefix: AsId) {
        let s = &mut self.speakers[a.idx()];
        for i in 0..s.neighbors.len() {
            if s.session_up[i] {
                s.dirty[i][prefix.idx()] = true;
                s.dirty_any[i] = true;
            }
        }
    }

    /// What `a` would announce to neighbour `i` for `prefix` right now.
    fn export(&self, a: AsId, i: usize, prefix: AsId) -> Option<AsPath> {
        let s = &self.speakers[a.idx()];
        let best = s.best[prefix.idx()].as_ref()?;
        let to_rel = s.neighbors[i].1;
        match best.from {
            None => {}
            Some(from) if from as usize == i => return None,
            Some(from) => {
                let learned_from = s.neighbors[from as usize].1;
                if learned_from != Relationship::Customer && to_rel != Relationship::Customer {
                    return None;
                }
            }
        }
        let mut path = Vec::with_capacity(best.path.len() + 1);
        path.push(s.asys);
        path.extend_from_slice(&best.path);
        Some(path.into())
    }

    fn flush_all(&mut self, a: AsId, mrai: Nanos, q: &mut EventQueue) {
        for i in 0..self.speakers[a.idx()].neighbors.len() {
            if self.speakers[a.idx()].dirty_any[i] {
                self.flush(a, i, mrai, q);
            }
        }
    }

    /// Send everything dirty towards neighbour `i`, unless the MRAI timer is
    /// still running, in which case it will be picked up when it fires.
    fn flush(&mut self, a: AsId, i: usize, mrai: Nanos, q: &mut EventQueue) {
        let s = &self.speakers[a.idx()];
        if s.mrai_armed[i] || !s.session_up[i] {
            return;
        }
        let mut update = Update::default();
        for p in 0..s.dirty[i].len() {
            if !s.dirty[i][p] {
                continue;
            }
            let prefix = AsId::from_idx(p);
            let out = self.export(a, i, prefix);
            if out != s.adj_out[i][p] {
                update.routes.push((prefix, out));
            }
        }
        let s = &mut self.speakers[a.idx()];
        s.dirty[i].iter_mut().for_each(|d| *d = false);
        s.dirty_any[i] = false;
        if update.routes.is_empty() {
            return;
        }
        for (prefix, path) in &update.routes {
            s.adj_out[i][prefix.idx()] = path.clone();
        }
        let (to, _) = s.neighbors[i];
        let epoch = s.epoch[i];
        s.mrai_armed[i] = true;
        self.updates_sent += 1;
        q.after(
            self.delay[a.idx()][i],
            Event::Bgp {
                to,
                from: a,
                epoch,
                update: Arc::new(update),
            },
        );
        q.after(mrai, Event::Mrai { asys: a, neighbor: i as u16 });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::SEC;
    use shadow_graph::{Tier, TopologyConfig};

    fn converge(topo: &Topology) -> (Bgp, EventQueue) {
        let mut bgp = Bgp::new(topo, 5_000_000);
        let mut q = EventQueue::default();
        let mrai = SEC;
        for a in &topo.ases {
            for &(b, _) in &a.neighbors {
                if a.id < b {
                    bgp.set_session(a.id, b, true, mrai, &mut q);
                }
            }
        }
        for a in &topo.ases {
            bgp.originate(a.id, mrai, &mut q);
        }
        while let Some(ev) = q.pop_until(u64::MAX) {
            match ev {
                Event::Bgp { to, from, epoch, update } => bgp.receive(to, from, epoch, &update, mrai, &mut q),
                Event::Mrai { asys, neighbor } => bgp.mrai_expired(asys, neighbor, mrai, &mut q),
                _ => unreachable!(),
            }
        }
        (bgp, q)
    }

    #[test]
    fn full_reachability_after_convergence() {
        let topo = Topology::generate(&TopologyConfig::small(1));
        let (bgp, _) = converge(&topo);
        for a in &topo.ases {
            for p in &topo.ases {
                assert!(bgp.best(a.id, p.id).is_some(), "{} cannot reach {}", a.name, p.name);
            }
        }
    }

    #[test]
    fn paths_are_valley_free() {
        let topo = Topology::generate(&TopologyConfig::small(2));
        let (bgp, _) = converge(&topo);
        for a in &topo.ases {
            for p in &topo.ases {
                let best = bgp.best(a.id, p.id).unwrap();
                let mut hops = vec![a.id];
                hops.extend(best.path.iter().copied());
                // Once the path has gone across a peer link or down to a
                // customer, it may only keep going down.
                let mut descending = false;
                for w in hops.windows(2) {
                    let rel = topo.relationship(w[0], w[1]).expect("path uses a non-adjacent AS pair");
                    match rel {
                        Relationship::Provider => assert!(!descending, "valley in {hops:?}"),
                        Relationship::Peer => {
                            assert!(!descending, "two peer hops or peer after descent in {hops:?}");
                            descending = true;
                        }
                        Relationship::Customer => descending = true,
                    }
                }
                assert_eq!(*hops.last().unwrap(), p.id);
            }
        }
    }

    #[test]
    fn customer_routes_beat_shorter_provider_routes() {
        let topo = Topology::generate(&TopologyConfig::small(3));
        let (bgp, _) = converge(&topo);
        // Every tier-1 must reach its direct customers directly.
        for a in topo.ases.iter().filter(|a| a.tier == Tier::Tier1) {
            for &(c, rel) in &a.neighbors {
                if rel == Relationship::Customer {
                    assert_eq!(&*bgp.best(a.id, c).unwrap().path, &[c]);
                }
            }
        }
    }

    #[test]
    fn session_loss_withdraws_and_reroutes() {
        let topo = Topology::generate(&TopologyConfig::small(4));
        let (mut bgp, mut q) = converge(&topo);
        // Pick a multihomed stub and drop one of its sessions.
        let stub = topo
            .ases
            .iter()
            .find(|a| a.tier == Tier::Stub && a.neighbors.len() >= 2)
            .expect("no multihomed stub in test topology");
        let (lost, _) = stub.neighbors[0];
        bgp.set_session(stub.id, lost, false, SEC, &mut q);
        while let Some(ev) = q.pop_until(u64::MAX) {
            match ev {
                Event::Bgp { to, from, epoch, update } => bgp.receive(to, from, epoch, &update, SEC, &mut q),
                Event::Mrai { asys, neighbor } => bgp.mrai_expired(asys, neighbor, SEC, &mut q),
                _ => unreachable!(),
            }
        }
        for a in &topo.ases {
            let best = bgp.best(a.id, stub.id).expect("stub became unreachable");
            if a.id != stub.id {
                // Nobody may still route into the stub over the dead session.
                let mut hops = vec![a.id];
                hops.extend(best.path.iter().copied());
                assert!(
                    !hops.windows(2).any(|w| w == [lost, stub.id]),
                    "{} still uses the dead session: {hops:?}",
                    a.name
                );
            }
        }
    }
}
