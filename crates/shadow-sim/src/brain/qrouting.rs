//! Q-routing: Boyan & Littman, "Packet Routing in Dynamically Changing
//! Networks: A Reinforcement Learning Approach" (NIPS 1993).
//!
//! Each router `x` learns `Q_x(t, a)`: how long it expects delivery to
//! target `t` to take if it hands the traffic to action `a` (an internal
//! neighbour, or an exit link to another AS). The update is the paper's,
//! with the reward being the delay we actually measure on the link right
//! now, queueing included:
//!
//! ```text
//!     Q_x(t, a) <- (1 - alpha) Q_x(t, a) + alpha (d(x, a) + V_y(t))
//!     V_y(t)     = min_b Q_y(t, b)
//! ```
//!
//! We run the "full echo" variant (every action is updated each step, not
//! just the one taken), because in a fluid model there is no packet to carry
//! the feedback, and full echo is the variant that recovers fastest from a
//! topology change anyway.
//!
//! Where this departs from the paper is in how the values are *used*. Pure
//! argmin forwarding herds all traffic onto whichever path looked best last
//! step, and the paper's own plots show the oscillation that follows. So:
//!
//! * flows pick among near-best actions with a soft-min, keyed by a hash of
//!   the flow, so load spreads proportionally and a given flow only moves
//!   when the values move enough to push it across a boundary;
//! * only *feasible* actions are eligible: the neighbour's own estimate must
//!   be strictly below ours (EIGRP's feasibility condition), which rules out
//!   forwarding loops as long as the estimates are consistent.

use shadow_graph::{AsId, LinkId, RouterId};

use crate::igp::View;

/// Expected cost of a lost packet: roughly one retransmission timeout's
/// worth of extra delay. Without this term the agent happily routes into a
/// link that drops 30% of everything because its queue looks short.
const LOSS_PENALTY_S: f32 = 0.2;
/// Per-hop forwarding cost, so equal-latency paths prefer fewer hops.
const HOP_S: f32 = 50e-6;

#[derive(Clone, Copy, Debug)]
struct Action {
    link: LinkId,
    queue: u32,
    to: RouterId,
    /// Some(B) if this action leaves the AS towards B.
    exit: Option<AsId>,
}

struct RouterQ {
    actions: Vec<Action>,
    /// targets x actions, row-major by target.
    q: Vec<f32>,
    v: Vec<f32>,
}

pub struct QRouting {
    routers: Vec<RouterQ>,
    /// Exit target slot for each action that leaves the AS, precomputed.
    exit_target: Vec<Vec<Option<usize>>>,
    pub alpha: f32,
    pub updates: u64,
    td_sum: f64,
    td_n: u64,
}

#[inline]
fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// What it costs right now to hand traffic to `a`, as `x` sees it.
fn action_delay(v: &View, x: RouterId, a: &Action, static_only: bool) -> f32 {
    if !v.links.seen_up_from(a.link, x) {
        return f32::INFINITY;
    }
    if static_only {
        return v.links.propagation(a.link) as f32 + HOP_S;
    }
    let dq = v.links.queue(a.queue);
    let st = &v.links.state[a.link.idx()];
    v.links.hop_delay(a.queue) as f32 + (dq.loss.max(st.extra_loss) as f32) * LOSS_PENALTY_S + HOP_S
}

impl QRouting {
    pub fn new(v: &View) -> Self {
        let mut routers = Vec::with_capacity(v.topo.routers.len());
        let mut exit_target = Vec::with_capacity(v.topo.routers.len());
        for r in &v.topo.routers {
            let lay = &v.layout.per_as[r.asys.idx()];
            let actions: Vec<Action> = v
                .topo
                .neighbors(r.id)
                .iter()
                .map(|&(l, n)| {
                    let n_as = v.topo.as_of(n);
                    Action {
                        link: l,
                        queue: v.links.queue_index(l, r.id),
                        to: n,
                        exit: (n_as != r.asys).then_some(n_as),
                    }
                })
                .collect();
            exit_target.push(actions.iter().map(|a| a.exit.and_then(|b| lay.exit_target(b))).collect());
            let t = lay.targets();
            routers.push(RouterQ {
                q: vec![f32::INFINITY; t * actions.len()],
                v: vec![f32::INFINITY; t],
                actions,
            });
        }
        let mut qr = QRouting {
            routers,
            exit_target,
            alpha: 0.25,
            updates: 0,
            td_sum: 0.0,
            td_n: 0,
        };
        qr.warm_start(v);
        qr
    }

    /// Seed the tables with propagation delay only, iterated to a fixed
    /// point. Equivalent to starting from the IGP's answer, so switching a
    /// running network over to Q-routing does not begin with a blind phase.
    fn warm_start(&mut self, v: &View) {
        let alpha = self.alpha;
        self.alpha = 1.0;
        for _ in 0..64 {
            self.step(v, true);
        }
        self.alpha = alpha;
        self.td_sum = 0.0;
        self.td_n = 0;
        self.updates = 0;
    }

    /// One synchronous learning step over every router.
    pub fn step(&mut self, v: &View, static_only: bool) {
        let alpha = self.alpha;
        // Jacobi style: every router reads its neighbours' V from the
        // previous step, so the result does not depend on router order.
        let snapshot: Vec<Vec<f32>> = self.routers.iter().map(|r| r.v.clone()).collect();
        let mut td_sum = 0.0f64;
        let mut td_n = 0u64;

        for (xi, rq) in self.routers.iter_mut().enumerate() {
            let x = RouterId::from_idx(xi);
            if !v.alive[xi] {
                rq.q.fill(f32::INFINITY);
                rq.v.fill(f32::INFINITY);
                continue;
            }
            let n_act = rq.actions.len();
            let n_t = rq.v.len();
            let me = v.layout.router_target(x);
            for (ai, a) in rq.actions.iter().enumerate() {
                let d = action_delay(v, x, a, static_only);
                let exit_t = self.exit_target[xi][ai];
                for t in 0..n_t {
                    if t == me {
                        continue;
                    }
                    let target = match a.exit {
                        Some(_) if exit_t == Some(t) => d,
                        Some(_) => f32::INFINITY,
                        None => {
                            let vy = snapshot[a.to.idx()][t];
                            d + vy
                        }
                    };
                    let cell = &mut rq.q[t * n_act + ai];
                    let old = *cell;
                    *cell = if !target.is_finite() {
                        f32::INFINITY
                    } else if !old.is_finite() {
                        target
                    } else {
                        let new = old + alpha * (target - old);
                        td_sum += (target - old).abs() as f64;
                        td_n += 1;
                        new
                    };
                }
            }
            for t in 0..n_t {
                rq.v[t] = if t == me {
                    0.0
                } else {
                    rq.q[t * n_act..(t + 1) * n_act]
                        .iter()
                        .copied()
                        .fold(f32::INFINITY, f32::min)
                };
            }
            self.updates += (n_act * n_t) as u64;
        }
        self.td_sum += td_sum;
        self.td_n += td_n;
    }

    /// Mean absolute temporal-difference error since the last call, in
    /// seconds. Falls towards zero as the tables settle.
    pub fn take_td(&mut self) -> f64 {
        let r = if self.td_n == 0 { 0.0 } else { self.td_sum / self.td_n as f64 };
        self.td_sum = 0.0;
        self.td_n = 0;
        r
    }

    pub fn value(&self, x: RouterId, t: usize) -> f32 {
        self.routers[x.idx()].v[t]
    }

    /// Pick the outgoing link for a flow at router `x` heading to target `t`.
    pub fn choose(&self, x: RouterId, t: usize, flow_hash: u64) -> Option<LinkId> {
        let rq = &self.routers[x.idx()];
        let n_act = rq.actions.len();
        let vx = rq.v[t];
        if !vx.is_finite() || n_act == 0 {
            return None;
        }
        let row = &rq.q[t * n_act..(t + 1) * n_act];
        let best = vx;
        let slack = best * 0.25 + 0.002;
        let tau = best * 0.08 + 0.0005;

        let mut weights = [0f32; 32];
        let mut idx = [0usize; 32];
        let mut k = 0;
        let mut total = 0.0;
        for (ai, &qv) in row.iter().enumerate() {
            if !qv.is_finite() || qv > best + slack || k == weights.len() {
                continue;
            }
            let a = &rq.actions[ai];
            let feasible = match a.exit {
                Some(_) => true,
                None => self.routers[a.to.idx()].v[t] < vx,
            };
            if !feasible {
                continue;
            }
            let w = (-(qv - best) / tau).exp();
            weights[k] = w;
            idx[k] = ai;
            total += w;
            k += 1;
        }
        if k == 0 {
            // Nothing feasible (estimates mid-update). Fall back to argmin;
            // if that loops, the TTL will say so.
            let ai = row
                .iter()
                .enumerate()
                .min_by(|a, b| a.1.total_cmp(b.1))
                .map(|(i, _)| i)?;
            return Some(rq.actions[ai].link);
        }
        let u = (mix(flow_hash ^ (x.0 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)) >> 40) as f32
            / (1u64 << 24) as f32
            * total;
        let mut acc = 0.0;
        for j in 0..k {
            acc += weights[j];
            if u < acc {
                return Some(rq.actions[idx[j]].link);
            }
        }
        Some(rq.actions[idx[k - 1]].link)
    }
}
