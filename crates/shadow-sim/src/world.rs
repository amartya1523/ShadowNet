//! The world: owns every piece of state and is the only thing that touches
//! the event queue's output. One thread, no locks.

use std::collections::VecDeque;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use shadow_graph::geo::{self, Region};
use shadow_graph::{AsId, LinkId, Relationship, RouterId, RouterRole, ServiceId, Tier, Topology, TopologyConfig};

use crate::bgp::Bgp;
use crate::brain::predictor::Predictor;
use crate::brain::qrouting::QRouting;
use crate::chaos::Monkey;
use crate::event::{Event, EventQueue};
use crate::forward::Forwarder;
use crate::igp::distvec::DistanceVector;
use crate::igp::linkstate::LinkStateIgp;
use crate::igp::{Fib, Layout, View};
use crate::links::{LinkTable, cause};
use crate::scenario::{Action, Scenario};
use crate::time::{MS, Nanos, SEC, Timers, from_secs, to_ms, to_secs};
use crate::traffic::{FlowKind, PathStatus, Traffic, TrafficConfig};
use crate::wire::{self, Command, LogEvent};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoutingMode {
    Ospf,
    Dv,
    Qrouting,
}

impl RoutingMode {
    pub fn label(self) -> &'static str {
        match self {
            RoutingMode::Ospf => "link-state (OSPF)",
            RoutingMode::Dv => "distance-vector (RIP)",
            RoutingMode::Qrouting => "Q-routing",
        }
    }
}

#[derive(Clone, Debug)]
pub struct SimConfig {
    pub topology: TopologyConfig,
    pub traffic: TrafficConfig,
    pub timers: Timers,
    pub routing: RoutingMode,
}

impl Default for SimConfig {
    fn default() -> Self {
        SimConfig {
            topology: TopologyConfig::default(),
            traffic: TrafficConfig::default(),
            timers: Timers::default(),
            routing: RoutingMode::Ospf,
        }
    }
}

impl SimConfig {
    pub fn small(seed: u64) -> Self {
        SimConfig {
            topology: TopologyConfig::small(seed),
            traffic: TrafficConfig {
                concurrent: 1500,
                ..TrafficConfig::default()
            },
            ..SimConfig::default()
        }
    }
}

/// A burst of control-plane activity caused by a topology change, tracked
/// from the physical event until the network has been quiet for a while.
#[derive(Clone, Debug)]
struct Incident {
    id: u32,
    cause: String,
    causes: u32,
    t0: Nanos,
    open: bool,
    igp_last: Option<Nanos>,
    bgp_last: Option<Nanos>,
    last_activity: Nanos,
    peak_loss: f64,
    blackholed_flow_secs: f64,
}

const INCIDENT_QUIET: Nanos = 2 * SEC;
const HISTORY_LEN: usize = 600;
const FLOW_SAMPLE: usize = 260;

#[inline]
fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Builds a [`View`] out of disjoint field borrows, so protocol calls can
/// take `&mut self.q` alongside it.
macro_rules! view {
    ($w:expr) => {
        View {
            topo: &$w.topo,
            layout: &$w.layout,
            links: &$w.links,
            alive: &$w.alive,
            timers: &$w.timers,
        }
    };
}

pub struct World {
    pub topo: Arc<Topology>,
    layout: Layout,
    pub links: LinkTable,
    alive: Vec<bool>,
    timers: Timers,
    q: EventQueue,
    ls: LinkStateIgp,
    dv: DistanceVector,
    bgp: Bgp,
    qr: QRouting,
    mode: RoutingMode,
    pub traffic: Traffic,
    monkey: Monkey,
    predictor: Predictor,
    /// Bumped whenever forwarding might have changed. Flows recompute their
    /// path lazily when they notice.
    route_gen: u64,
    epoch: Nanos,
    next_sample: Nanos,
    last_control_change: Nanos,
    log: Vec<LogEvent>,
    incidents: VecDeque<Incident>,
    next_incident: u32,
    history: VecDeque<(f64, wire::Stats)>,
    scenario: Option<Scenario>,
    scenario_run: u32,
    td_ms: f64,
    q_updates_rate: f64,
    rate_mark: (Nanos, u64, u64),
    events_per_sec: f64,
    tick_loss: f64,
}

impl World {
    pub fn new(cfg: SimConfig) -> World {
        let topo = Arc::new(Topology::generate(&cfg.topology));
        Self::with_topology(topo, cfg)
    }

    pub fn with_topology(topo: Arc<Topology>, cfg: SimConfig) -> World {
        let layout = Layout::new(&topo);
        let links = LinkTable::new(&topo);
        let alive = vec![true; topo.routers.len()];
        let timers = cfg.timers.clone();
        let (ls, dv, qr) = {
            let v = View {
                topo: &topo,
                layout: &layout,
                links: &links,
                alive: &alive,
                timers: &timers,
            };
            (LinkStateIgp::new(&v), DistanceVector::new(&v), QRouting::new(&v))
        };
        let seed = topo.config.seed;
        let mut w = World {
            bgp: Bgp::new(&topo, timers.bgp_processing),
            traffic: Traffic::new(&topo, cfg.traffic.clone(), seed ^ 0x7AFF_1C),
            monkey: Monkey::new(&topo, seed ^ 0xC4A0_5),
            predictor: Predictor::new(&topo),
            topo,
            layout,
            links,
            alive,
            timers,
            q: EventQueue::default(),
            ls,
            dv,
            qr,
            mode: cfg.routing,
            route_gen: 0,
            epoch: 0,
            next_sample: 0,
            last_control_change: 0,
            log: Vec::new(),
            incidents: VecDeque::new(),
            next_incident: 1,
            history: VecDeque::new(),
            scenario: None,
            scenario_run: 0,
            td_ms: 0.0,
            q_updates_rate: 0.0,
            rate_mark: (0, 0, 0),
            events_per_sec: 0.0,
            tick_loss: 0.0,
        };
        w.boot();
        w
    }

    /// Bring every protocol up and let the control plane converge before any
    /// traffic exists. Real networks are never cold-started all at once,
    /// but nobody wants to watch that part.
    fn boot(&mut self) {
        let mrai = self.timers.bgp_mrai;
        for i in 0..self.topo.routers.len() {
            let r = RouterId::from_idx(i);
            let v = view!(self);
            self.ls.originate(r, &v, &mut self.q);
            self.dv.boot(r, &v, &mut self.q);
        }
        for a in self.topo.ases.iter() {
            for &(b, _) in &a.neighbors {
                if a.id < b {
                    self.bgp.set_session(a.id, b, true, mrai, &mut self.q);
                }
            }
        }
        for i in 0..self.topo.ases.len() {
            self.bgp.originate(AsId::from_idx(i), mrai, &mut self.q);
        }

        // BGP is what takes longest. Run until it has been quiet for a few
        // MRAI intervals; the periodic DV timers mean the queue itself never
        // empties.
        let limit = 120 * SEC;
        while self.q.now() < limit {
            let horizon = self.q.now() + 100 * MS;
            self.run_until(horizon);
            if self.q.now() > 5 * SEC && self.q.now() - self.last_control_change > 4 * mrai {
                break;
            }
        }
        self.route_gen += 1;
        self.epoch = self.q.now();
        self.next_sample = self.epoch + SEC;
        self.rate_mark = (self.epoch, self.q.processed, self.qr.updates);
        self.traffic.prefill(&self.topo, &self.alive, self.epoch);
        self.q.at(self.epoch, Event::Tick);
        let boot_s = to_secs(self.epoch);
        self.info(
            "boot",
            format!(
                "{} nodes, {} routers, {} ASes up; control plane converged in {:.1}s simulated",
                self.topo.node_count(),
                self.topo.routers.len(),
                self.topo.ases.len(),
                boot_s
            ),
        );
    }

    // -- clock --------------------------------------------------------------

    /// Seconds since traffic started.
    pub fn t(&self) -> f64 {
        to_secs(self.q.now().saturating_sub(self.epoch))
    }

    pub fn now(&self) -> Nanos {
        self.q.now()
    }

    pub fn mode(&self) -> RoutingMode {
        self.mode
    }

    pub fn monkey_enabled(&self) -> bool {
        self.monkey.enabled
    }

    pub fn events_processed(&self) -> u64 {
        self.q.processed
    }

    pub fn advance(&mut self, dt: Nanos) {
        let until = self.q.now() + dt;
        self.run_until(until);
    }

    pub fn run_until(&mut self, until: Nanos) {
        while let Some(ev) = self.q.pop_until(until) {
            self.handle(ev);
        }
        self.q.advance_to(until);
    }

    fn handle(&mut self, ev: Event) {
        let mrai = self.timers.bgp_mrai;
        match ev {
            Event::Tick => {
                self.tick();
                self.q.after(self.timers.tick, Event::Tick);
            }
            Event::Detect { link, end, up } => self.on_detect(link, end, up),
            Event::Lsa { to, via, lsa } => {
                let v = view!(self);
                self.ls.receive(to, via, lsa, &v, &mut self.q);
            }
            Event::Spf { router } => {
                let v = view!(self);
                self.ls.run_spf(router, &v, &mut self.q);
            }
            Event::FibInstall { router, next, cost } => {
                if self.alive[router.idx()] && self.ls.install(router, next, cost) {
                    self.last_control_change = self.q.now();
                    if self.mode == RoutingMode::Ospf {
                        self.route_gen += 1;
                        self.mark_incident(true);
                    }
                }
            }
            Event::DvSend { router } => {
                let v = view!(self);
                self.dv.send(router, &v, &mut self.q);
            }
            Event::DvPeriodic { router } => {
                let v = view!(self);
                self.dv.periodic(router, &v, &mut self.q);
            }
            Event::DvRecv { to, via, vector } => {
                let v = view!(self);
                self.dv.receive(to, via, &vector, &v, &mut self.q);
            }
            Event::Bgp { to, from, epoch, update } => {
                self.bgp.receive(to, from, epoch, &update, mrai, &mut self.q);
            }
            Event::Mrai { asys, neighbor } => self.bgp.mrai_expired(asys, neighbor, mrai, &mut self.q),
            Event::Clear { link, token } => {
                let st = &mut self.links.state[link.idx()];
                if st.impairment_token == token {
                    st.extra_delay_s = 0.0;
                    st.extra_loss = 0.0;
                    let name = self.link_name(link);
                    self.info("impairment_end", format!("{name} back to normal"));
                }
            }
            Event::Repair { link, token } => {
                if self.links.state[link.idx()].repair_token == token {
                    self.set_cut(link, cause::MONKEY, false, true, "repaired");
                }
            }
            Event::ScenarioStep { run, step } => {
                if run == self.scenario_run {
                    self.scenario_step(step);
                }
            }
        }

        if self.dv.changed {
            self.dv.changed = false;
            if self.mode == RoutingMode::Dv {
                self.route_gen += 1;
                self.mark_incident(true);
            }
        }
        if self.bgp.changed {
            self.bgp.changed = false;
            self.route_gen += 1;
            self.last_control_change = self.q.now();
            self.mark_incident(false);
        }
    }

    // -- topology changes -----------------------------------------------------

    fn set_cut(&mut self, l: LinkId, bit: u8, on: bool, planned: bool, why: &str) {
        let st = &mut self.links.state[l.idx()];
        if on {
            st.cut |= bit;
        } else {
            st.cut &= !bit;
        }
        self.refresh_link(l, planned, why);
    }

    /// Recompute a link's physical state from its causes and its ends, and
    /// if it changed, schedule each end to notice.
    fn refresh_link(&mut self, l: LinkId, planned: bool, why: &str) {
        let link = self.topo.link(l);
        let (a, b) = (link.a, link.b);
        let st = &mut self.links.state[l.idx()];
        let up = st.cut == 0 && self.alive[a.idx()] && self.alive[b.idx()];
        if up == st.up {
            return;
        }
        st.up = up;
        self.route_gen += 1;
        let delay = if up { self.timers.detect_up } else { self.timers.detect_down };
        for end in 0..2u8 {
            self.q.after(delay, Event::Detect { link: l, end, up });
        }
        let name = self.link_name(l);
        let t = self.t();
        if up {
            self.info("link_up", format!("{name} up ({why})"));
        } else {
            self.predictor.record_failure(l, t, planned);
            self.monkey.note_failure(l);
            self.push_log("warn", "link_down", format!("{name} down ({why})"));
            self.open_incident(format!("{name} down"));
        }
    }

    fn on_detect(&mut self, l: LinkId, end: u8, up: bool) {
        let st = &self.links.state[l.idx()];
        // The link flipped again before this end noticed the first change.
        if st.up != up || st.seen_up[end as usize] == up {
            return;
        }
        let link = self.topo.link(l);
        let r = if end == 0 { link.a } else { link.b };
        if !self.alive[r.idx()] {
            return;
        }
        self.links.state[l.idx()].seen_up[end as usize] = up;
        self.route_gen += 1;
        if let Some(inc) = self.incidents.front_mut().filter(|i| i.open) {
            inc.last_activity = self.q.now();
        }

        let n = link.other(r);
        let (ra, na) = (self.topo.as_of(r), self.topo.as_of(n));
        let v = view!(self);
        if ra == na && up {
            self.ls.adjacency_up(r, l, &v, &mut self.q);
        } else {
            self.ls.originate(r, &v, &mut self.q);
        }
        if up {
            self.dv.link_up(r, l, &v, &mut self.q);
        } else {
            self.dv.link_down(r, l, &v, &mut self.q);
        }
        if ra != na {
            self.refresh_session(ra, na);
        }
    }

    fn refresh_session(&mut self, a: AsId, b: AsId) {
        let should = self.topo.links_between(a, b).iter().any(|&l| {
            let link = self.topo.link(l);
            let st = &self.links.state[l.idx()];
            st.seen_up[0] && st.seen_up[1] && self.alive[link.a.idx()] && self.alive[link.b.idx()]
        });
        if should != self.bgp.session_up(a, b) {
            let (na, nb) = (self.topo.asys(a).asn, self.topo.asys(b).asn);
            let state = if should { "established" } else { "down" };
            self.push_log(
                if should { "info" } else { "warn" },
                "bgp_session",
                format!("BGP AS{na} <-> AS{nb} {state}"),
            );
            self.bgp.set_session(a, b, should, self.timers.bgp_mrai, &mut self.q);
        }
    }

    fn set_router(&mut self, r: RouterId, up: bool) {
        if self.alive[r.idx()] == up {
            return;
        }
        self.alive[r.idx()] = up;
        let name = self.topo.router(r).name.clone();
        if !up {
            self.ls.router_down(r);
            self.dv.router_down(r);
            // The dead box stops believing in anything immediately; its
            // neighbours find out the usual way.
            for &(l, _) in self.topo.neighbors(r) {
                let end = self.links.end_of(l, r) as usize;
                self.links.state[l.idx()].seen_up[end] = false;
            }
            self.push_log("crit", "router_down", format!("{name} crashed"));
        } else {
            self.info("router_up", format!("{name} booting"));
        }
        let links: Vec<LinkId> = self.topo.neighbors(r).iter().map(|&(l, _)| l).collect();
        for l in links {
            self.refresh_link(l, true, if up { "router recovered" } else { "router crashed" });
        }
        if up {
            let v = view!(self);
            self.dv.boot(r, &v, &mut self.q);
            self.ls.originate(r, &v, &mut self.q);
        }
        self.route_gen += 1;
    }

    // -- data plane ------------------------------------------------------------

    fn tick(&mut self) {
        let now = self.q.now();
        let dt = to_secs(self.timers.tick);
        self.traffic.step(&self.topo, &self.alive, now, dt);

        // Q-routing learns all the time, whichever protocol is forwarding,
        // so it is warm the moment you switch to it.
        {
            let v = view!(self);
            self.qr.step(&v, false);
        }
        if self.mode == RoutingMode::Qrouting {
            self.route_gen += 1;
        }

        let fwd = Forwarder {
            topo: &self.topo,
            layout: &self.layout,
            links: &self.links,
            alive: &self.alive,
            bgp: &self.bgp,
            fib: match self.mode {
                RoutingMode::Dv => &self.dv.fib,
                _ => &self.ls.fib,
            },
            qr: &self.qr,
            mode: self.mode,
        };
        let route_gen = self.route_gen;
        for f in &mut self.traffic.flows {
            if f.route_gen != route_gen {
                f.status = fwd.walk(f.src, f.dst, mix(f.id), &mut f.hops);
                f.route_gen = route_gen;
            }
        }

        self.links.clear_offered();
        for f in &self.traffic.flows {
            for &h in &f.hops {
                self.links.offer(h, f.demand_bps);
            }
        }
        self.links.integrate(dt);

        let (mut offered, mut delivered, mut broken) = (0.0, 0.0, 0usize);
        for f in &mut self.traffic.flows {
            offered += f.demand_bps;
            if f.status == PathStatus::Ok {
                let mut lat = 0.0;
                let mut survive = 1.0;
                for &h in &f.hops {
                    lat += self.links.hop_delay(h);
                    survive *= 1.0 - self.links.queue(h).loss;
                }
                f.latency_s = lat;
                f.loss = 1.0 - survive;
                f.delivered_bps = f.demand_bps * survive;
            } else {
                f.latency_s = 0.0;
                f.loss = 1.0;
                f.delivered_bps = 0.0;
                broken += 1;
            }
            delivered += f.delivered_bps;
        }
        self.tick_loss = if offered > 0.0 { 1.0 - delivered / offered } else { 0.0 };
        if let Some(inc) = self.incidents.front_mut().filter(|i| i.open) {
            inc.peak_loss = inc.peak_loss.max(self.tick_loss);
            inc.blackholed_flow_secs += broken as f64 * dt;
        }

        for l in self.monkey.step(&self.topo, &self.links, dt) {
            self.set_cut(l, cause::MONKEY, true, false, "failure");
            let st = &mut self.links.state[l.idx()];
            st.repair_token = st.repair_token.wrapping_add(1);
            let token = st.repair_token;
            let after = from_secs(self.monkey.repair_time_s());
            self.q.after(after, Event::Repair { link: l, token });
        }
        self.predictor.observe(&self.links, dt);

        if now >= self.next_sample {
            self.next_sample += SEC;
            let t = self.t();
            self.predictor.sample(&self.links, t);
            self.close_quiet_incidents();
            self.update_rates();
            let stats = self.stats();
            self.history.push_back((t, stats));
            if self.history.len() > HISTORY_LEN {
                self.history.pop_front();
            }
        }
    }

    fn active_fib(&self) -> &Fib {
        match self.mode {
            RoutingMode::Dv => &self.dv.fib,
            _ => &self.ls.fib,
        }
    }

    fn update_rates(&mut self) {
        let now = self.q.now();
        let (t0, e0, u0) = self.rate_mark;
        let dt = to_secs(now - t0).max(1e-9);
        self.events_per_sec = (self.q.processed - e0) as f64 / dt;
        self.q_updates_rate = (self.qr.updates - u0) as f64 / dt;
        self.td_ms = self.qr.take_td() * 1e3;
        self.rate_mark = (now, self.q.processed, self.qr.updates);
    }

    // -- incidents and log --------------------------------------------------------

    fn open_incident(&mut self, cause: String) {
        let now = self.q.now();
        if let Some(inc) = self.incidents.front_mut().filter(|i| i.open) {
            inc.causes += 1;
            inc.last_activity = now;
            return;
        }
        self.incidents.push_front(Incident {
            id: self.next_incident,
            cause,
            causes: 1,
            t0: now,
            open: true,
            igp_last: None,
            bgp_last: None,
            last_activity: now,
            peak_loss: self.tick_loss,
            blackholed_flow_secs: 0.0,
        });
        self.next_incident += 1;
        self.incidents.truncate(8);
    }

    fn mark_incident(&mut self, igp: bool) {
        let now = self.q.now();
        if let Some(inc) = self.incidents.front_mut().filter(|i| i.open) {
            if igp {
                inc.igp_last = Some(now);
            } else {
                inc.bgp_last = Some(now);
            }
            inc.last_activity = now;
        }
    }

    fn close_quiet_incidents(&mut self) {
        let now = self.q.now();
        let mut closed = Vec::new();
        for inc in self.incidents.iter_mut().filter(|i| i.open) {
            if now - inc.last_activity >= INCIDENT_QUIET {
                inc.open = false;
                closed.push(inc.clone());
            }
        }
        for inc in closed {
            let fmt = |x: Option<Nanos>| match x {
                Some(t) => format!("{:.0} ms", to_ms(t - inc.t0)),
                None => "no change".into(),
            };
            self.info(
                "converged",
                format!(
                    "incident #{} settled: IGP {}, BGP {}, peak loss {:.1}%",
                    inc.id,
                    fmt(inc.igp_last),
                    fmt(inc.bgp_last),
                    inc.peak_loss * 100.0
                ),
            );
        }
    }

    fn push_log(&mut self, level: &'static str, kind: &'static str, text: String) {
        let t = (self.t() * 1000.0).round() / 1000.0;
        self.log.push(LogEvent { t, level, kind, text });
        if self.log.len() > 2000 {
            self.log.drain(..1000);
        }
    }

    fn info(&mut self, kind: &'static str, text: String) {
        self.push_log("info", kind, text);
    }

    pub fn drain_log(&mut self) -> Vec<LogEvent> {
        std::mem::take(&mut self.log)
    }

    fn link_name(&self, l: LinkId) -> String {
        let link = self.topo.link(l);
        format!("{} <-> {}", self.topo.router(link.a).name, self.topo.router(link.b).name)
    }

    // -- commands ----------------------------------------------------------------

    pub fn apply(&mut self, cmd: Command) -> Result<(), String> {
        let n_links = self.topo.links.len() as u32;
        let n_routers = self.topo.routers.len() as u32;
        let n_ases = self.topo.ases.len() as u16;
        let n_services = self.topo.services.len() as u16;
        let check = |ok: bool, what: &str| if ok { Ok(()) } else { Err(format!("no such {what}")) };
        match cmd {
            Command::FailLink { link } => {
                check(link < n_links, "link")?;
                self.set_cut(LinkId(link), cause::MANUAL, true, true, "cut by operator");
            }
            Command::RestoreLink { link } => {
                check(link < n_links, "link")?;
                let l = LinkId(link);
                self.links.state[l.idx()].cut &= !cause::MONKEY;
                self.set_cut(l, cause::MANUAL, false, true, "restored by operator");
            }
            Command::FailRouter { router } => {
                check(router < n_routers, "router")?;
                self.set_router(RouterId(router), false);
            }
            Command::RestoreRouter { router } => {
                check(router < n_routers, "router")?;
                self.set_router(RouterId(router), true);
            }
            Command::FailAs { asys } => {
                check(asys < n_ases, "AS")?;
                self.set_as(AsId(asys), false);
            }
            Command::RestoreAs { asys } => {
                check(asys < n_ases, "AS")?;
                self.set_as(AsId(asys), true);
            }
            Command::LatencySpike { link, ms, secs } => {
                check(link < n_links, "link")?;
                self.impair(LinkId(link), Some(ms / 1e3), None, secs);
            }
            Command::PacketLoss { link, pct, secs } => {
                check(link < n_links, "link")?;
                self.impair(LinkId(link), None, Some((pct / 100.0).clamp(0.0, 1.0)), secs);
            }
            Command::Ddos { service, bots, mbps, secs } => {
                check(service < n_services, "service")?;
                self.campaign(FlowKind::Attack, ServiceId(service), bots, mbps, 2.0, secs);
            }
            Command::FlashCrowd {
                service,
                users,
                mbps,
                ramp_secs,
                secs,
            } => {
                check(service < n_services, "service")?;
                self.campaign(FlowKind::Crowd, ServiceId(service), users, mbps, ramp_secs, secs);
            }
            Command::ChaosMonkey { enabled, per_minute } => {
                self.monkey.enabled = enabled;
                if let Some(p) = per_minute {
                    self.monkey.per_minute = p.max(0.0);
                }
                let msg = if enabled {
                    format!("chaos monkey on, ~{:.0} failures/min at rest", self.monkey.per_minute)
                } else {
                    "chaos monkey off".to_string()
                };
                self.push_log("warn", "chaos", msg);
            }
            Command::SetRouting { mode } => {
                if mode != self.mode {
                    self.mode = mode;
                    self.route_gen += 1;
                    self.info("routing", format!("forwarding now uses {}", mode.label()));
                }
            }
            Command::HealAll => self.heal_all(),
            other => return Err(format!("{other:?} is handled by the runner")),
        }
        Ok(())
    }

    fn set_as(&mut self, a: AsId, up: bool) {
        let name = self.topo.asys(a).name.clone();
        let asn = self.topo.asys(a).asn;
        if up {
            self.info("as_up", format!("AS{asn} {name} recovering"));
        } else {
            self.push_log("crit", "as_down", format!("AS{asn} {name} is down"));
        }
        for r in self.topo.asys(a).routers.clone() {
            self.set_router(r, up);
        }
    }

    fn impair(&mut self, l: LinkId, delay: Option<f64>, loss: Option<f64>, secs: f64) {
        let st = &mut self.links.state[l.idx()];
        if let Some(d) = delay {
            st.extra_delay_s = d;
        }
        if let Some(p) = loss {
            st.extra_loss = p;
        }
        st.impairment_token = st.impairment_token.wrapping_add(1);
        let token = st.impairment_token;
        self.q.after(from_secs(secs.max(0.1)), Event::Clear { link: l, token });
        let name = self.link_name(l);
        let what = match (delay, loss) {
            (Some(d), _) => format!("+{:.0} ms latency", d * 1e3),
            (_, Some(p)) => format!("{:.1}% packet loss", p * 100.0),
            _ => unreachable!(),
        };
        self.push_log("warn", "impairment", format!("{name}: {what} for {secs:.0}s"));
    }

    fn campaign(&mut self, kind: FlowKind, svc: ServiceId, count: usize, mbps: f64, ramp: f64, secs: f64) {
        let now = self.q.now();
        self.traffic
            .start_campaign(kind, svc, count, mbps * 1e6, now, from_secs(ramp.max(0.0)), from_secs(secs));
        let name = self.topo.services[svc.idx()].name.clone();
        let (level, text) = match kind {
            FlowKind::Crowd => ("warn", format!("flash crowd: {count} users hitting {name} at {mbps} Mbps each")),
            _ => ("crit", format!("DDoS: {count} bots flooding {name} at {mbps} Mbps each")),
        };
        self.push_log(level, if kind == FlowKind::Crowd { "flash_crowd" } else { "ddos" }, text);
    }

    pub fn heal_all(&mut self) {
        for i in 0..self.topo.routers.len() {
            self.set_router(RouterId::from_idx(i), true);
        }
        for i in 0..self.topo.links.len() {
            let l = LinkId::from_idx(i);
            let st = &mut self.links.state[i];
            st.extra_delay_s = 0.0;
            st.extra_loss = 0.0;
            st.impairment_token = st.impairment_token.wrapping_add(1);
            st.repair_token = st.repair_token.wrapping_add(1);
            if st.cut != 0 {
                st.cut = 0;
                self.refresh_link(l, true, "healed");
            }
        }
        self.monkey.enabled = false;
        self.traffic.stop_campaigns(self.q.now());
        self.info("heal", "all faults cleared".to_string());
    }

    // -- scenarios -------------------------------------------------------------

    pub fn start_scenario(&mut self, s: Scenario) {
        self.scenario_run += 1;
        let run = self.scenario_run;
        if let Some(mode) = s.routing {
            let _ = self.apply(Command::SetRouting { mode });
        }
        for (i, step) in s.steps.iter().enumerate() {
            self.q.after(from_secs(step.at.max(0.0)), Event::ScenarioStep { run, step: i });
        }
        self.push_log("info", "scenario", format!("scenario: {}", s.title));
        self.scenario = Some(s);
    }

    fn resolve_service(&self, sel: &str) -> Option<ServiceId> {
        let svcs = &self.topo.services;
        let i = match sel {
            "@top" => (!svcs.is_empty()).then_some(0)?,
            "@tail" => svcs.len().checked_sub(1)?,
            "@single" => svcs.iter().position(|s| s.replicas.len() == 1)?,
            name => svcs.iter().position(|s| s.name == name)?,
        };
        Some(ServiceId::from_idx(i))
    }

    fn region_links(&self, from: &str, to: &str) -> Vec<LinkId> {
        let region = |code: &str| Region::ALL.into_iter().find(|r| r.code() == code);
        let (Some(f), Some(t)) = (region(from), region(to)) else {
            return Vec::new();
        };
        let reg = |r: RouterId| geo::city(self.topo.router(r).city).region;
        let mut v: Vec<LinkId> = self
            .topo
            .links
            .iter()
            .filter(|l| {
                let (x, y) = (reg(l.a), reg(l.b));
                (x == f && y == t) || (x == t && y == f)
            })
            .map(|l| l.id)
            .collect();
        v.sort_by(|a, b| self.topo.link(*b).capacity_bps.total_cmp(&self.topo.link(*a).capacity_bps));
        v
    }

    fn busiest_links(&self) -> Vec<LinkId> {
        let mut v: Vec<(f64, LinkId)> = self
            .links
            .state
            .iter()
            .enumerate()
            .filter(|(_, s)| s.up)
            .map(|(i, s)| (s.dir[0].util.max(s.dir[1].util) * self.topo.links[i].capacity_bps, LinkId::from_idx(i)))
            .collect();
        v.sort_by(|a, b| b.0.total_cmp(&a.0));
        v.into_iter().map(|x| x.1).collect()
    }

    fn scenario_step(&mut self, i: usize) {
        let Some(step) = self.scenario.as_ref().and_then(|s| s.steps.get(i)).cloned() else {
            return;
        };
        let missing_service = |w: &mut World, sel: &str| w.push_log("warn", "scenario", format!("no service matches {sel}"));
        match step.action {
            Action::Note { text } => self.push_log("info", "note", text),
            Action::CutBetweenRegions { from, to, max } => {
                let links = self.region_links(&from, &to);
                let n = max.unwrap_or(links.len()).min(links.len());
                self.push_log("crit", "scenario", format!("cutting {n} links between {from} and {to}"));
                for l in links.into_iter().take(n) {
                    self.set_cut(l, cause::MANUAL, true, true, "cable cut");
                }
            }
            Action::CutBusiest { count } => {
                for l in self.busiest_links().into_iter().take(count) {
                    self.set_cut(l, cause::MANUAL, true, true, "cut (busiest)");
                }
            }
            Action::CrashBusiestRouter => {
                let mut load = vec![0.0f64; self.topo.routers.len()];
                for (i, s) in self.links.state.iter().enumerate() {
                    let l = &self.topo.links[i];
                    let carried = (s.dir[0].util + s.dir[1].util) * l.capacity_bps;
                    load[l.a.idx()] += carried;
                    load[l.b.idx()] += carried;
                }
                let r = (0..load.len())
                    .filter(|&i| self.alive[i] && self.topo.routers[i].role == RouterRole::Core)
                    .max_by(|&a, &b| load[a].total_cmp(&load[b]));
                if let Some(r) = r {
                    self.set_router(RouterId::from_idx(r), false);
                }
            }
            Action::FailHostingAs { service } => match self.resolve_service(&service) {
                Some(s) => {
                    let a = self.topo.as_of(self.topo.services[s.idx()].replicas[0]);
                    self.set_as(a, false);
                }
                None => missing_service(self, &service),
            },
            Action::FailBiggestTier1 => {
                let biggest = self
                    .topo
                    .ases
                    .iter()
                    .filter(|a| a.tier == Tier::Tier1)
                    .max_by_key(|a| a.neighbors.iter().filter(|n| n.1 == Relationship::Customer).count())
                    .map(|a| a.id);
                if let Some(a) = biggest {
                    self.set_as(a, false);
                }
            }
            Action::FlashCrowd {
                service,
                users,
                mbps,
                ramp,
                secs,
            } => match self.resolve_service(&service) {
                Some(s) => self.campaign(FlowKind::Crowd, s, users, mbps, ramp, secs),
                None => missing_service(self, &service),
            },
            Action::Ddos {
                service,
                bots,
                mbps,
                secs,
            } => match self.resolve_service(&service) {
                Some(s) => self.campaign(FlowKind::Attack, s, bots, mbps, 2.0, secs),
                None => missing_service(self, &service),
            },
            Action::LatencySpikeBetweenRegions { from, to, ms, secs } => {
                for l in self.region_links(&from, &to) {
                    self.impair(l, Some(ms / 1e3), None, secs);
                }
            }
            Action::SetRouting { mode } => {
                let _ = self.apply(Command::SetRouting { mode });
            }
            Action::ChaosMonkey { enabled, per_minute } => {
                let _ = self.apply(Command::ChaosMonkey { enabled, per_minute });
            }
            Action::HealAll => self.heal_all(),
        }
    }

    // -- observation ----------------------------------------------------------------

    pub fn stats(&self) -> wire::Stats {
        let mut s = wire::Stats::default();
        let mut lat: Vec<(f64, f64)> = Vec::with_capacity(self.traffic.flows.len());
        let (mut offered, mut delivered) = (0.0, 0.0);
        for f in &self.traffic.flows {
            offered += f.demand_bps;
            delivered += f.delivered_bps;
            match f.status {
                PathStatus::Ok => lat.push((f.latency_s, f.delivered_bps)),
                PathStatus::Unreachable => s.flows.unreachable += 1,
                PathStatus::Loop => s.flows.looping += 1,
                PathStatus::Blackhole => s.flows.blackholed += 1,
            }
            match f.kind {
                FlowKind::Crowd => s.flows.crowd += 1,
                FlowKind::Attack => s.flows.attack += 1,
                FlowKind::Background => {}
            }
        }
        s.flows.active = self.traffic.flows.len();
        s.offered_gbps = offered / 1e9;
        s.delivered_gbps = delivered / 1e9;
        s.loss_pct = if offered > 0.0 { (1.0 - delivered / offered) * 100.0 } else { 0.0 };

        // Percentiles weighted by delivered bytes: "what latency does the
        // average bit see", which is what users feel.
        lat.sort_by(|a, b| a.0.total_cmp(&b.0));
        let total: f64 = lat.iter().map(|x| x.1).sum();
        if total > 0.0 {
            let pick = |q: f64| {
                let mut acc = 0.0;
                for &(l, w) in &lat {
                    acc += w;
                    if acc >= q * total {
                        return l * 1e3;
                    }
                }
                lat.last().unwrap().0 * 1e3
            };
            s.latency_ms = wire::Latency {
                mean: lat.iter().map(|x| x.0 * x.1).sum::<f64>() / total * 1e3,
                p50: pick(0.5),
                p95: pick(0.95),
                p99: pick(0.99),
            };
        }
        for st in &self.links.state {
            if !st.up {
                s.links_down += 1;
                continue;
            }
            let u = st.dir[0].util.max(st.dir[1].util);
            s.max_util = s.max_util.max(u);
            if u >= 0.9 {
                s.congested_links += 1;
            }
        }
        s.routers_down = self.alive.iter().filter(|a| !**a).count();
        s.events_per_sec = self.events_per_sec;
        s
    }

    pub fn link_arrays(&self) -> wire::LinkArrays {
        let n = self.links.state.len();
        let mut a = wire::LinkArrays {
            util: Vec::with_capacity(n),
            queue_ms: Vec::with_capacity(n),
            loss: Vec::with_capacity(n),
            state: Vec::with_capacity(n),
            risk: Vec::with_capacity(n),
        };
        for (i, st) in self.links.state.iter().enumerate() {
            let cap = self.topo.links[i].capacity_bps;
            let offered = st.dir[0].offered_bps.max(st.dir[1].offered_bps);
            let (util, q, loss) = if st.up {
                (
                    (offered / cap * 1000.0).round().min(2000.0) as u16,
                    (st.dir[0].queue_s.max(st.dir[1].queue_s) * 1e4).round() as u16,
                    (st.dir[0].loss.max(st.dir[1].loss) * 1000.0).round() as u16,
                )
            } else {
                (0, 0, 0)
            };
            let state = if !st.up {
                if st.seen_up[0] || st.seen_up[1] { 3 } else { 1 }
            } else if st.extra_delay_s > 0.0 || st.extra_loss > 0.0 {
                2
            } else {
                0
            };
            a.util.push(util);
            a.queue_ms.push(q);
            a.loss.push(loss);
            a.state.push(state);
            a.risk.push((self.predictor.risk[i] * 1000.0).round() as u16);
        }
        a
    }

    pub fn routers_down(&self) -> Vec<u32> {
        self.alive
            .iter()
            .enumerate()
            .filter(|(_, a)| !**a)
            .map(|(i, _)| i as u32)
            .collect()
    }

    /// A stable sample of flows for the animation. Membership is decided by
    /// a hash of the flow id, so a flow that is sampled stays sampled for
    /// its whole life and its dot does not blink in and out.
    pub fn flow_sample(&self) -> Vec<wire::FlowView> {
        let count = |k: FlowKind| self.traffic.flows.iter().filter(|f| f.kind == k).count();
        let mut out = Vec::new();
        // Campaign flows get a quarter each when present; background gets
        // whatever is left, so a quiet network still looks busy.
        for kind in [FlowKind::Attack, FlowKind::Crowd, FlowKind::Background] {
            let q = match kind {
                FlowKind::Background => FLOW_SAMPLE - out.len(),
                _ => FLOW_SAMPLE / 4,
            };
            let n = count(kind);
            if n == 0 {
                continue;
            }
            let stride = (n / q).max(1) as u64;
            for f in self.traffic.flows.iter().filter(|f| f.kind == kind) {
                if mix(f.id) % stride != 0 || f.hops.is_empty() {
                    continue;
                }
                let mut path = Vec::with_capacity(f.hops.len() + 1);
                path.push(f.src.0);
                path.extend(f.hops.iter().map(|&h| self.links.queue_target(h).0));
                out.push(wire::FlowView {
                    id: f.id,
                    kind: f.kind,
                    path,
                    latency_ms: (f.latency_s * 1e4).round() / 10.0,
                    loss_pct: (f.loss * 1e3).round() / 10.0,
                    mbps: (f.demand_bps / 1e5).round() / 10.0,
                });
            }
        }
        out
    }

    pub fn incidents(&self) -> Vec<wire::IncidentView> {
        self.incidents
            .iter()
            .map(|i| wire::IncidentView {
                id: i.id,
                cause: if i.causes > 1 {
                    format!("{} (+{} more)", i.cause, i.causes - 1)
                } else {
                    i.cause.clone()
                },
                t0: (to_secs(i.t0.saturating_sub(self.epoch)) * 1000.0).round() / 1000.0,
                open: i.open,
                igp_ms: i.igp_last.map(|t| to_ms(t - i.t0).round()),
                bgp_ms: i.bgp_last.map(|t| to_ms(t - i.t0).round()),
                peak_loss_pct: (i.peak_loss * 1000.0).round() / 10.0,
                blackholed_flow_secs: (i.blackholed_flow_secs * 10.0).round() / 10.0,
            })
            .collect()
    }

    pub fn ai(&self) -> wire::AiView {
        let p = &self.predictor;
        wire::AiView {
            predictor: wire::PredictorView {
                auc: p.auc,
                baseline_auc: p.baseline_auc,
                precision_at10: p.precision_at_10(),
                labelled: p.labelled,
                positives: p.positives,
                top: p
                    .top(10)
                    .into_iter()
                    .filter(|(_, r)| *r > 0.0)
                    .map(|(l, r)| wire::RiskView {
                        link: l.0,
                        risk: (r as f64 * 1000.0).round() / 1000.0,
                    })
                    .collect(),
            },
            qrouting: wire::QRoutingView {
                updates_per_sec: self.q_updates_rate.round(),
                mean_abs_td_ms: (self.td_ms * 1000.0).round() / 1000.0,
            },
        }
    }

    pub fn predictor(&self) -> &Predictor {
        &self.predictor
    }

    pub fn history(&self) -> wire::History {
        let mut h = wire::History::default();
        for (t, s) in &self.history {
            h.t.push(*t);
            h.delivered_gbps.push(s.delivered_gbps);
            h.offered_gbps.push(s.offered_gbps);
            h.loss_pct.push(s.loss_pct);
            h.mean_ms.push(s.latency_ms.mean);
            h.p99_ms.push(s.latency_ms.p99);
            h.active_flows.push(s.flows.active);
            h.links_down.push(s.links_down);
        }
        h
    }

    pub fn trace(&self, src: RouterId, dst: RouterId) -> wire::Trace {
        let fwd = Forwarder {
            topo: &self.topo,
            layout: &self.layout,
            links: &self.links,
            alive: &self.alive,
            bgp: &self.bgp,
            fib: self.active_fib(),
            qr: &self.qr,
            mode: self.mode,
        };
        let mut hops = Vec::new();
        let status = fwd.walk(src, dst, 0, &mut hops);
        let mut out = Vec::with_capacity(hops.len() + 1);
        let mut cum = 0.0;
        let mut as_path: Vec<u32> = Vec::new();
        let mut push = |r: RouterId, link: Option<LinkId>, cum: f64, queue: f64, out: &mut Vec<wire::TraceHop>| {
            let router = self.topo.router(r);
            let asn = self.topo.asys(router.asys).asn;
            if as_path.last() != Some(&asn) {
                as_path.push(asn);
            }
            out.push(wire::TraceHop {
                router: r.0,
                name: router.name.clone(),
                asys: router.asys.0,
                asn,
                link: link.map(|l| l.0),
                cum_ms: (cum * 1e4).round() / 10.0,
                queue_ms: (queue * 1e4).round() / 10.0,
            });
        };
        push(src, None, 0.0, 0.0, &mut out);
        for &h in &hops {
            cum += self.links.hop_delay(h);
            push(
                self.links.queue_target(h),
                Some(LinkId(h / 2)),
                cum,
                self.links.queue(h).queue_s,
                &mut out,
            );
        }
        wire::Trace {
            status: match status {
                PathStatus::Ok => "ok",
                PathStatus::Unreachable => "unreachable",
                PathStatus::Loop => "loop",
                PathStatus::Blackhole => "blackhole",
            },
            hops: out,
            as_path,
        }
    }

    pub fn router_info(&self, r: RouterId) -> wire::RouterInfo {
        let router = self.topo.router(r);
        let lay = self.layout.of(&self.topo, r);
        let fib = self.active_fib();
        let neighbors = self
            .topo
            .neighbors(r)
            .iter()
            .map(|&(l, n)| {
                let q = self.links.queue_index(l, r);
                wire::NeighborView {
                    router: n.0,
                    link: l.0,
                    up: self.links.up(l),
                    util: (self.links.queue(q).util * 1000.0).round() / 1000.0,
                }
            })
            .collect();
        let fib_rows = (0..lay.targets())
            .filter(|&t| t != self.layout.router_target(r))
            .take(48)
            .map(|t| {
                let target = match lay.exit_as(t) {
                    Some(b) => format!("exit AS{}", self.topo.asys(b).asn),
                    None => self.topo.router(lay.routers[t]).name.clone(),
                };
                let link = match self.mode {
                    RoutingMode::Qrouting => self.qr.choose(r, t, 0),
                    _ => fib.lookup(r, t),
                };
                let cost = match self.mode {
                    RoutingMode::Qrouting => {
                        let v = self.qr.value(r, t);
                        v.is_finite().then(|| (v * 1e4).round() as u32)
                    }
                    _ => Some(fib.cost[r.idx()][t]).filter(|&c| c != u32::MAX),
                };
                wire::FibView {
                    target,
                    link: link.map(|l| l.0),
                    next_hop: link.map(|l| self.links.other_end(l, r).0),
                    cost,
                }
            })
            .collect();
        wire::RouterInfo {
            id: r.0,
            name: router.name.clone(),
            asys: router.asys.0,
            role: wire::role_name(router.role),
            down: !self.alive[r.idx()],
            hosts: router.hosts.len() as u32,
            neighbors,
            fib: fib_rows,
        }
    }

    /// Counters for benchmarks and the CLI.
    pub fn counters(&self) -> Counters {
        Counters {
            events: self.q.processed,
            pending: self.q.len(),
            spf_runs: self.ls.spf_runs,
            lsa_deliveries: self.ls.lsa_deliveries,
            dv_updates: self.dv.updates_sent,
            bgp_updates: self.bgp.updates_sent,
            bgp_best_changes: self.bgp.best_changes,
            q_updates: self.qr.updates,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Counters {
    pub events: u64,
    pub pending: usize,
    pub spf_runs: u64,
    pub lsa_deliveries: u64,
    pub dv_updates: u64,
    pub bgp_updates: u64,
    pub bgp_best_changes: u64,
    pub q_updates: u64,
}
