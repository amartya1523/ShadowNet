//! Who is talking to whom, and how hard.
//!
//! Flows are open-loop rate demands. Most of the background is users
//! pulling from services (the direction that matters is server to user, so
//! that is the direction we model), the rest is user to user. On top of that
//! sit *campaigns*: a flash crowd on one service, or a botnet aimed at one.

use std::collections::HashMap;
use std::ops::Range;

use serde::{Deserialize, Serialize};
use shadow_graph::geo::haversine_km;
use shadow_graph::rng::WeightedIndex;
use shadow_graph::{Rng, RouterId, ServiceId, Topology};

use crate::time::{Nanos, from_secs};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FlowKind {
    #[serde(rename = "bg")]
    Background,
    #[serde(rename = "crowd")]
    Crowd,
    #[serde(rename = "attack")]
    Attack,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathStatus {
    Ok,
    /// Some router on the way had no route.
    Unreachable,
    /// TTL ran out: a forwarding loop.
    Loop,
    /// Forwarded onto a link that is down but not yet known to be down.
    Blackhole,
}

#[derive(Clone, Debug)]
pub struct Flow {
    pub id: u64,
    pub kind: FlowKind,
    pub src: RouterId,
    pub dst: RouterId,
    pub demand_bps: f64,
    pub ends_at: Nanos,
    /// Directional queue indices traversed, in order.
    pub hops: Vec<u32>,
    pub status: PathStatus,
    /// Routing generation `hops` was computed under.
    pub route_gen: u64,
    pub latency_s: f64,
    pub loss: f64,
    pub delivered_bps: f64,
    /// When an endpoint of this flow died. Clients give up after a grace
    /// period instead of retrying a dead box forever.
    pub dead_since: Option<Nanos>,
}

/// How long a client keeps trying a dead endpoint before giving up.
const GIVE_UP: Nanos = 3 * crate::time::SEC;

#[derive(Clone, Debug)]
pub struct TrafficConfig {
    /// Background flows alive at any moment, on average.
    pub concurrent: usize,
    pub mean_duration_s: f64,
    pub rate_median_bps: f64,
    pub rate_sigma: f64,
    /// Share of background flows that are user <- service.
    pub service_share: f64,
}

impl Default for TrafficConfig {
    fn default() -> Self {
        TrafficConfig {
            concurrent: 30_000,
            mean_duration_s: 30.0,
            rate_median_bps: 8.0e6,
            rate_sigma: 0.9,
            service_share: 0.75,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Campaign {
    pub id: u32,
    pub kind: FlowKind,
    pub service: ServiceId,
    pub count: usize,
    pub rate_bps: f64,
    pub start: Nanos,
    pub ramp: Nanos,
    pub end: Nanos,
    pub spawned: usize,
}

pub struct Traffic {
    pub cfg: TrafficConfig,
    pub flows: Vec<Flow>,
    pub campaigns: Vec<Campaign>,
    next_id: u64,
    next_campaign: u32,
    rng: Rng,
    users: Range<u32>,
    popularity: WeightedIndex,
    carry: f64,
    /// Nearest replica per (service, user router), filled lazily. Users of
    /// one access router all resolve to the same place, like GeoDNS.
    nearest: HashMap<(u16, u32), RouterId>,
}

impl Traffic {
    pub fn new(topo: &Topology, cfg: TrafficConfig, seed: u64) -> Self {
        let popularity = if topo.services.is_empty() {
            WeightedIndex::new([1.0])
        } else {
            WeightedIndex::new(topo.services.iter().map(|s| s.popularity))
        };
        Traffic {
            cfg,
            flows: Vec::new(),
            campaigns: Vec::new(),
            next_id: 1,
            next_campaign: 1,
            rng: Rng::new(seed),
            users: 0..topo.server_hosts.start,
            popularity,
            carry: 0.0,
            nearest: HashMap::new(),
        }
    }

    fn random_user(&mut self, topo: &Topology) -> (RouterId, f64) {
        let h = self.users.start + self.rng.below(self.users.len()) as u32;
        (topo.host_router[h as usize], topo.host_rate_bps[h as usize] as f64)
    }

    /// Closest live replica, the way a health-checked GSLB answers. If every
    /// replica is down we still hand out the nearest one; the flow will fail,
    /// which is the honest outcome.
    fn replica_for(&mut self, topo: &Topology, alive: &[bool], svc: ServiceId, user: RouterId) -> RouterId {
        if let Some(&r) = self.nearest.get(&(svc.0, user.0)) {
            if alive[r.idx()] {
                return r;
            }
        }
        let u = topo.router(user);
        let dist = |r: RouterId| {
            let rr = topo.router(r);
            haversine_km(u.lat, u.lon, rr.lat, rr.lon)
        };
        let replicas = &topo.services[svc.idx()].replicas;
        let best = replicas
            .iter()
            .copied()
            .filter(|r| alive[r.idx()])
            .min_by(|&a, &b| dist(a).total_cmp(&dist(b)))
            .or_else(|| replicas.iter().copied().min_by(|&a, &b| dist(a).total_cmp(&dist(b))))
            .expect("service without replicas");
        self.nearest.insert((svc.0, user.0), best);
        best
    }

    fn push(&mut self, kind: FlowKind, src: RouterId, dst: RouterId, demand_bps: f64, ends_at: Nanos) {
        let id = self.next_id;
        self.next_id += 1;
        self.flows.push(Flow {
            id,
            kind,
            src,
            dst,
            demand_bps,
            ends_at,
            hops: Vec::new(),
            status: PathStatus::Ok,
            route_gen: u64::MAX,
            latency_s: 0.0,
            loss: 0.0,
            delivered_bps: 0.0,
            dead_since: None,
        });
    }

    fn background(&mut self, topo: &Topology, alive: &[bool], now: Nanos, duration_s: f64) {
        let (user, user_rate) = self.random_user(topo);
        let demand = self.rng.lognormal(self.cfg.rate_median_bps, self.cfg.rate_sigma);
        let ends = now + from_secs(duration_s);
        if !topo.services.is_empty() && self.rng.chance(self.cfg.service_share) {
            let svc = ServiceId::from_idx(self.popularity.sample(&mut self.rng));
            let server = self.replica_for(topo, alive, svc, user);
            self.push(FlowKind::Background, server, user, demand.min(user_rate), ends);
        } else {
            let (peer, peer_rate) = self.random_user(topo);
            self.push(FlowKind::Background, peer, user, demand.min(user_rate).min(peer_rate), ends);
        }
    }

    /// Start in steady state rather than with an empty network. Remaining
    /// lifetimes of a population of exponential flows are themselves
    /// exponential, so this is exact.
    pub fn prefill(&mut self, topo: &Topology, alive: &[bool], now: Nanos) {
        for _ in 0..self.cfg.concurrent {
            let d = self.rng.exp(self.cfg.mean_duration_s);
            self.background(topo, alive, now, d);
        }
    }

    pub fn start_campaign(
        &mut self,
        kind: FlowKind,
        service: ServiceId,
        count: usize,
        rate_bps: f64,
        start: Nanos,
        ramp: Nanos,
        duration: Nanos,
    ) -> u32 {
        let id = self.next_campaign;
        self.next_campaign += 1;
        self.campaigns.push(Campaign {
            id,
            kind,
            service,
            count,
            rate_bps,
            start,
            ramp,
            end: start + duration,
            spawned: 0,
        });
        id
    }

    /// Drop finished flows and create new ones. Returns how many arrived.
    pub fn step(&mut self, topo: &Topology, alive: &[bool], now: Nanos, dt: f64) -> usize {
        for f in &mut self.flows {
            let dead = !alive[f.src.idx()] || !alive[f.dst.idx()];
            f.dead_since = match (dead, f.dead_since) {
                (false, _) => None,
                (true, None) => Some(now),
                (true, since) => since,
            };
        }
        self.flows
            .retain(|f| f.ends_at > now && f.dead_since.is_none_or(|t| now - t < GIVE_UP));

        let rate = self.cfg.concurrent as f64 / self.cfg.mean_duration_s;
        // The arrival *count* per step is deterministic (fractional part
        // carried over); the randomness that matters is in who, how fast and
        // for how long.
        let expected = rate * dt + self.carry;
        let n = expected.floor() as usize;
        self.carry = expected - n as f64;
        for _ in 0..n {
            let d = self.rng.exp(self.cfg.mean_duration_s);
            self.background(topo, alive, now, d);
        }

        let mut spawn = Vec::new();
        for c in &mut self.campaigns {
            if now < c.start || now >= c.end || c.spawned >= c.count {
                continue;
            }
            let frac = if c.ramp == 0 {
                1.0
            } else {
                ((now - c.start) as f64 / c.ramp as f64).min(1.0)
            };
            let target = (c.count as f64 * frac).round() as usize;
            if target > c.spawned {
                spawn.push((c.kind, c.service, target - c.spawned, c.rate_bps, c.end));
                c.spawned = target;
            }
        }
        for (kind, svc, k, rate_bps, end) in spawn {
            for _ in 0..k {
                let (user, user_rate) = self.random_user(topo);
                let replica = self.replica_for(topo, alive, svc, user);
                // Not every member of a crowd leaves at the same instant.
                let jitter = from_secs(self.rng.range_f64(0.0, 3.0));
                let demand = rate_bps.min(user_rate);
                match kind {
                    FlowKind::Crowd => self.push(kind, replica, user, demand, end + jitter),
                    FlowKind::Attack => self.push(kind, user, replica, demand, end + jitter),
                    FlowKind::Background => unreachable!(),
                }
            }
        }
        self.campaigns.retain(|c| now < c.end);
        n
    }

    pub fn stop_campaigns(&mut self, now: Nanos) {
        self.campaigns.clear();
        for f in &mut self.flows {
            if f.kind != FlowKind::Background {
                f.ends_at = f.ends_at.min(now + 1);
            }
        }
    }
}
