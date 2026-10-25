//! Everything that crosses the process boundary. Mirrors docs/PROTOCOL.md;
//! change both or neither.

use serde::{Deserialize, Serialize};
use shadow_graph::geo::{self, CITIES};
use shadow_graph::{LinkKind, RouterRole, Tier, Topology};

use crate::traffic::FlowKind;
use crate::world::RoutingMode;

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    FailLink { link: u32 },
    RestoreLink { link: u32 },
    FailRouter { router: u32 },
    RestoreRouter { router: u32 },
    FailAs {
        #[serde(rename = "as")]
        asys: u16,
    },
    RestoreAs {
        #[serde(rename = "as")]
        asys: u16,
    },
    LatencySpike { link: u32, ms: f64, secs: f64 },
    PacketLoss { link: u32, pct: f64, secs: f64 },
    Ddos { service: u16, bots: usize, mbps: f64, secs: f64 },
    FlashCrowd {
        service: u16,
        users: usize,
        mbps: f64,
        #[serde(rename = "rampSecs", default)]
        ramp_secs: f64,
        secs: f64,
    },
    ChaosMonkey {
        enabled: bool,
        #[serde(rename = "perMinute", default)]
        per_minute: Option<f64>,
    },
    SetRouting { mode: RoutingMode },
    HealAll,
    // The rest are about the run, not the network; the server's runner
    // handles them and the world never sees them.
    SetSpeed { speed: f64 },
    Pause,
    Resume,
    Reset {
        #[serde(default)]
        seed: Option<u64>,
    },
    RunScenario { name: String },
}

// -- topology -----------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopologyView {
    pub run_id: u32,
    pub seed: u64,
    pub summary: SummaryView,
    pub cities: Vec<CityView>,
    pub ases: Vec<AsView>,
    pub routers: Vec<RouterView>,
    pub links: Vec<LinkView>,
    pub services: Vec<ServiceView>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryView {
    pub nodes: usize,
    pub routers: usize,
    pub hosts: usize,
    pub servers: usize,
    pub links: usize,
    pub tier1: usize,
    pub tier2: usize,
    pub stubs: usize,
    pub services: usize,
}

#[derive(Serialize)]
pub struct CityView {
    pub code: &'static str,
    pub name: &'static str,
    pub lat: f64,
    pub lon: f64,
    pub region: &'static str,
}

#[derive(Serialize)]
pub struct AsView {
    pub id: u16,
    pub asn: u32,
    pub name: String,
    pub tier: &'static str,
    pub region: &'static str,
    pub home: &'static str,
    pub hosting: bool,
}

#[derive(Serialize)]
pub struct RouterView {
    pub id: u32,
    #[serde(rename = "as")]
    pub asys: u16,
    pub name: String,
    pub role: &'static str,
    pub city: &'static str,
    pub lat: f64,
    pub lon: f64,
    pub hosts: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkView {
    pub id: u32,
    pub a: u32,
    pub b: u32,
    pub kind: &'static str,
    pub capacity_gbps: f64,
    pub delay_ms: f64,
    pub km: f64,
}

#[derive(Serialize)]
pub struct ServiceView {
    pub id: u16,
    pub name: String,
    pub replicas: Vec<u32>,
    pub popularity: f64,
}

fn round(x: f64, places: i32) -> f64 {
    let p = 10f64.powi(places);
    (x * p).round() / p
}

pub fn tier_name(t: Tier) -> &'static str {
    match t {
        Tier::Tier1 => "tier1",
        Tier::Tier2 => "tier2",
        Tier::Stub => "stub",
    }
}

pub fn role_name(r: RouterRole) -> &'static str {
    match r {
        RouterRole::Core => "core",
        RouterRole::Border => "border",
        RouterRole::Access => "access",
        RouterRole::Datacenter => "datacenter",
    }
}

impl TopologyView {
    pub fn new(topo: &Topology, run_id: u32) -> Self {
        let s = topo.summary();
        TopologyView {
            run_id,
            seed: topo.config.seed,
            summary: SummaryView {
                nodes: s.nodes,
                routers: s.routers,
                hosts: s.hosts - s.servers,
                servers: s.servers,
                links: s.links,
                tier1: s.tier1,
                tier2: s.tier2,
                stubs: s.stubs,
                services: s.services,
            },
            cities: CITIES
                .iter()
                .map(|c| CityView {
                    code: c.code,
                    name: c.name,
                    lat: c.lat,
                    lon: c.lon,
                    region: c.region.code(),
                })
                .collect(),
            ases: topo
                .ases
                .iter()
                .map(|a| AsView {
                    id: a.id.0,
                    asn: a.asn,
                    name: a.name.clone(),
                    tier: tier_name(a.tier),
                    region: a.region.code(),
                    home: geo::city(a.home).code,
                    hosting: a.hosting,
                })
                .collect(),
            routers: topo
                .routers
                .iter()
                .map(|r| RouterView {
                    id: r.id.0,
                    asys: r.asys.0,
                    name: r.name.clone(),
                    role: role_name(r.role),
                    city: geo::city(r.city).code,
                    lat: round(r.lat, 3),
                    lon: round(r.lon, 3),
                    hosts: r.hosts.len() as u32,
                })
                .collect(),
            links: topo
                .links
                .iter()
                .map(|l| LinkView {
                    id: l.id.0,
                    a: l.a.0,
                    b: l.b.0,
                    kind: match l.kind {
                        LinkKind::Internal => "internal",
                        LinkKind::Peering => "peering",
                        LinkKind::Transit => "transit",
                    },
                    capacity_gbps: l.capacity_bps / 1e9,
                    delay_ms: round(l.delay_s * 1e3, 3),
                    km: l.km.round(),
                })
                .collect(),
            services: topo
                .services
                .iter()
                .map(|s| ServiceView {
                    id: s.id.0,
                    name: s.name.clone(),
                    replicas: s.replicas.iter().map(|r| r.0).collect(),
                    popularity: round(s.popularity, 4),
                })
                .collect(),
        }
    }
}

// -- frames ---------------------------------------------------------------------

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Frame {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub run_id: u32,
    pub seq: u64,
    pub t: f64,
    pub paused: bool,
    pub speed: f64,
    pub sim_rate: f64,
    pub routing: RoutingMode,
    pub chaos_monkey: bool,
    pub stats: Stats,
    pub links: LinkArrays,
    pub routers_down: Vec<u32>,
    pub flows: Vec<FlowView>,
    pub events: Vec<LogEvent>,
    pub incidents: Vec<IncidentView>,
    pub ai: AiView,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub offered_gbps: f64,
    pub delivered_gbps: f64,
    pub loss_pct: f64,
    pub latency_ms: Latency,
    pub flows: FlowCounts,
    pub links_down: usize,
    pub routers_down: usize,
    pub congested_links: usize,
    pub max_util: f64,
    pub events_per_sec: f64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Latency {
    pub mean: f64,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct FlowCounts {
    pub active: usize,
    pub unreachable: usize,
    pub looping: usize,
    pub blackholed: usize,
    pub crowd: usize,
    pub attack: usize,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkArrays {
    pub util: Vec<u16>,
    pub queue_ms: Vec<u16>,
    pub loss: Vec<u16>,
    pub state: Vec<u8>,
    pub risk: Vec<u16>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowView {
    pub id: u64,
    pub kind: FlowKind,
    pub path: Vec<u32>,
    pub latency_ms: f64,
    pub loss_pct: f64,
    pub mbps: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct LogEvent {
    pub t: f64,
    pub level: &'static str,
    pub kind: &'static str,
    pub text: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IncidentView {
    pub id: u32,
    pub cause: String,
    pub t0: f64,
    pub open: bool,
    pub igp_ms: Option<f64>,
    pub bgp_ms: Option<f64>,
    pub peak_loss_pct: f64,
    pub blackholed_flow_secs: f64,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiView {
    pub predictor: PredictorView,
    pub qrouting: QRoutingView,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PredictorView {
    pub auc: Option<f64>,
    pub baseline_auc: Option<f64>,
    pub precision_at10: Option<f64>,
    pub labelled: u64,
    pub positives: u64,
    pub top: Vec<RiskView>,
}

#[derive(Clone, Serialize)]
pub struct RiskView {
    pub link: u32,
    pub risk: f64,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QRoutingView {
    pub updates_per_sec: f64,
    pub mean_abs_td_ms: f64,
}

// -- history, trace, router -------------------------------------------------------

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct History {
    pub t: Vec<f64>,
    pub delivered_gbps: Vec<f64>,
    pub offered_gbps: Vec<f64>,
    pub loss_pct: Vec<f64>,
    pub mean_ms: Vec<f64>,
    pub p99_ms: Vec<f64>,
    pub active_flows: Vec<usize>,
    pub links_down: Vec<usize>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Trace {
    pub status: &'static str,
    pub hops: Vec<TraceHop>,
    pub as_path: Vec<u32>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceHop {
    pub router: u32,
    pub name: String,
    #[serde(rename = "as")]
    pub asys: u16,
    pub asn: u32,
    pub link: Option<u32>,
    pub cum_ms: f64,
    pub queue_ms: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouterInfo {
    pub id: u32,
    pub name: String,
    #[serde(rename = "as")]
    pub asys: u16,
    pub role: &'static str,
    pub down: bool,
    pub hosts: u32,
    pub neighbors: Vec<NeighborView>,
    pub fib: Vec<FibView>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NeighborView {
    pub router: u32,
    pub link: u32,
    pub up: bool,
    pub util: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FibView {
    pub target: String,
    pub link: Option<u32>,
    pub next_hop: Option<u32>,
    pub cost: Option<u32>,
}
