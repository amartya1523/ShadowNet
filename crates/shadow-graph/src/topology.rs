//! Internet-shaped topology generation.
//!
//! The shape we are after, from the top down:
//!
//! * a handful of **tier-1** backbones with POPs on every continent, peering
//!   with each other in a full mesh at shared exchange cities;
//! * **tier-2** regional carriers that buy transit from two or three tier-1s
//!   and peer with some of their neighbours;
//! * many **stub** networks (eyeball ISPs and hosting) that buy transit from
//!   one or two upstreams and carry the hosts.
//!
//! Everything is drawn from one seed. The same config always produces the
//! same network, byte for byte.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use crate::algo::Csr;
use crate::geo::{self, CITIES, City, Region};
use crate::ids::{AsId, CityId, HostId, LinkId, RouterId, ServiceId};
use crate::rng::{Rng, WeightedIndex};

#[derive(Clone, Debug)]
pub struct TopologyConfig {
    pub seed: u64,
    /// Total node budget: routers + hosts + servers. Whatever the routers
    /// do not use goes to hosts.
    pub nodes: usize,
    pub tier1: usize,
    pub tier2: usize,
    pub stubs: usize,
    /// How many of the stubs are hosting networks (datacenters).
    pub hosting: usize,
    pub services: usize,
    pub servers_per_replica: usize,
}

impl Default for TopologyConfig {
    fn default() -> Self {
        TopologyConfig {
            seed: 0x5ead_0e7,
            nodes: 100_000,
            tier1: 6,
            tier2: 36,
            stubs: 240,
            hosting: 14,
            services: 16,
            servers_per_replica: 48,
        }
    }
}

impl TopologyConfig {
    /// Something that builds in a few milliseconds, for tests.
    pub fn small(seed: u64) -> Self {
        TopologyConfig {
            seed,
            nodes: 3_000,
            tier1: 3,
            tier2: 8,
            stubs: 30,
            hosting: 4,
            services: 5,
            servers_per_replica: 8,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Tier {
    Tier1,
    Tier2,
    Stub,
}

/// What the *neighbour* is to us. If B is our `Provider`, we pay B.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Relationship {
    Customer,
    Peer,
    Provider,
}

impl Relationship {
    pub fn inverse(self) -> Self {
        match self {
            Relationship::Customer => Relationship::Provider,
            Relationship::Provider => Relationship::Customer,
            Relationship::Peer => Relationship::Peer,
        }
    }
}

#[derive(Clone, Debug)]
pub struct AutonomousSystem {
    pub id: AsId,
    /// Drawn from the private-use range (64512-65534) so we never print a
    /// real operator's number next to made-up behaviour.
    pub asn: u32,
    pub name: String,
    pub tier: Tier,
    pub region: Region,
    pub home: CityId,
    pub routers: Vec<RouterId>,
    pub neighbors: Vec<(AsId, Relationship)>,
    pub hosting: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RouterRole {
    Core,
    Border,
    Access,
    Datacenter,
}

#[derive(Clone, Debug)]
pub struct Router {
    pub id: RouterId,
    pub asys: AsId,
    pub city: CityId,
    pub role: RouterRole,
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    /// Contiguous block of host ids attached to this router.
    pub hosts: Range<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LinkKind {
    /// Inside one AS.
    Internal,
    /// Settlement-free interconnect between two ASes.
    Peering,
    /// Customer-provider interconnect.
    Transit,
}

#[derive(Clone, Debug)]
pub struct Link {
    pub id: LinkId,
    pub a: RouterId,
    pub b: RouterId,
    pub kind: LinkKind,
    pub capacity_bps: f64,
    /// One-way propagation delay.
    pub delay_s: f64,
    pub km: f64,
    /// IGP metric. Latency based, in units of 100 us, minimum 1: what most
    /// operators converge on once they stop using reference bandwidth.
    pub cost: u32,
    /// Hidden failure-proneness. Only the chaos engine reads this.
    pub fragility: f64,
}

impl Link {
    pub fn other(&self, r: RouterId) -> RouterId {
        if r == self.a {
            self.b
        } else {
            debug_assert_eq!(r, self.b);
            self.a
        }
    }

    pub fn is_inter_as(&self) -> bool {
        self.kind != LinkKind::Internal
    }
}

#[derive(Clone, Debug)]
pub struct Service {
    pub id: ServiceId,
    pub name: String,
    /// Datacenter routers serving it. Users get sent to the closest one,
    /// the way GeoDNS would do it.
    pub replicas: Vec<RouterId>,
    /// Relative share of user traffic (Zipf by rank).
    pub popularity: f64,
}

#[derive(Clone, Debug)]
pub struct Topology {
    pub config: TopologyConfig,
    pub ases: Vec<AutonomousSystem>,
    pub routers: Vec<Router>,
    pub links: Vec<Link>,
    pub services: Vec<Service>,
    /// Router each host hangs off. Indexed by `HostId`.
    pub host_router: Vec<RouterId>,
    /// Access line rate of each host, bits per second.
    pub host_rate_bps: Vec<f32>,
    /// Hosts that are servers rather than users, per service replica.
    pub server_hosts: Range<u32>,
    adjacency: Vec<Vec<(LinkId, RouterId)>>,
    inter_as: HashMap<(AsId, AsId), Vec<LinkId>>,
}

impl Topology {
    pub fn generate(config: &TopologyConfig) -> Topology {
        Builder::new(config.clone()).build()
    }

    pub fn node_count(&self) -> usize {
        self.routers.len() + self.host_router.len()
    }

    pub fn host_count(&self) -> usize {
        self.host_router.len()
    }

    pub fn router(&self, r: RouterId) -> &Router {
        &self.routers[r.idx()]
    }

    pub fn link(&self, l: LinkId) -> &Link {
        &self.links[l.idx()]
    }

    pub fn asys(&self, a: AsId) -> &AutonomousSystem {
        &self.ases[a.idx()]
    }

    pub fn as_of(&self, r: RouterId) -> AsId {
        self.routers[r.idx()].asys
    }

    pub fn host_as(&self, h: HostId) -> AsId {
        self.as_of(self.host_router[h.idx()])
    }

    pub fn neighbors(&self, r: RouterId) -> &[(LinkId, RouterId)] {
        &self.adjacency[r.idx()]
    }

    /// All physical links between two ASes, in either direction.
    pub fn links_between(&self, a: AsId, b: AsId) -> &[LinkId] {
        let key = if a < b { (a, b) } else { (b, a) };
        self.inter_as.get(&key).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn relationship(&self, from: AsId, to: AsId) -> Option<Relationship> {
        self.ases[from.idx()]
            .neighbors
            .iter()
            .find(|(n, _)| *n == to)
            .map(|&(_, rel)| rel)
    }

    pub fn city_of(&self, r: RouterId) -> &'static City {
        geo::city(self.routers[r.idx()].city)
    }

    /// The whole router-level graph weighted by IGP cost, ignoring AS
    /// boundaries and policy. Good for "what would a naive shortest path
    /// do" comparisons and for betweenness.
    pub fn router_graph(&self) -> Csr {
        Csr::undirected(
            self.routers.len(),
            self.links
                .iter()
                .map(|l| (l.a.idx(), l.b.idx(), l.cost as u64, l.id.0)),
        )
    }

    pub fn summary(&self) -> TopologySummary {
        let count = |t| self.ases.iter().filter(|a| a.tier == t).count();
        let kind = |k| self.links.iter().filter(|l| l.kind == k).count();
        TopologySummary {
            nodes: self.node_count(),
            routers: self.routers.len(),
            hosts: self.host_count(),
            servers: self.server_hosts.len(),
            links: self.links.len(),
            tier1: count(Tier::Tier1),
            tier2: count(Tier::Tier2),
            stubs: count(Tier::Stub),
            internal_links: kind(LinkKind::Internal),
            peering_links: kind(LinkKind::Peering),
            transit_links: kind(LinkKind::Transit),
            services: self.services.len(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopologySummary {
    pub nodes: usize,
    pub routers: usize,
    pub hosts: usize,
    pub servers: usize,
    pub links: usize,
    pub tier1: usize,
    pub tier2: usize,
    pub stubs: usize,
    pub internal_links: usize,
    pub peering_links: usize,
    pub transit_links: usize,
    pub services: usize,
}

// ---------------------------------------------------------------------------
// Generation
// ---------------------------------------------------------------------------

// Capacities are those of a smaller internet than the real one (a real
// tier-1 backbone is many parallel 400G waves). What matters is the ratio
// to the offered load, which is tuned so busy backbones run at 20-50% and a
// few edges run hot, the way they do in practice.
const GBPS: f64 = 1e9;

const NAME_HEADS: &[&str] = &[
    "Arcturus", "Meridian", "Halcyon", "Borealis", "Cobalt", "Vantage", "Lumen", "Strata",
    "Northwind", "Kestrel", "Obsidian", "Tidewater", "Saffron", "Granite", "Helix", "Juniper",
    "Larkspur", "Monsoon", "Nimbus", "Orchid", "Pioneer", "Quarry", "Redwood", "Sable", "Tundra",
    "Umber", "Verdant", "Willow", "Zephyr", "Aster", "Basalt", "Cinder", "Delta", "Ember",
    "Fjord", "Glacier", "Harbor", "Ironwood", "Jasper", "Kelvin", "Lattice", "Mosaic",
];
const T1_TAILS: &[&str] = &["Global Backbone", "Transit", "Global Crossing", "Worldwide"];
const T2_TAILS: &[&str] = &["Telecom", "Networks", "Fibre", "Carrier", "Communications"];
const STUB_TAILS: &[&str] = &["Broadband", "Online", "Net", "Cable", "Wireless", "Connect"];
const HOST_TAILS: &[&str] = &["Cloud", "Datacenters", "Hosting", "Compute"];

const SERVICE_NAMES: &[&str] = &[
    "vidstream", "photon-social", "quicksearch", "cloudmail", "gamehub", "newswire",
    "musicbox", "shopfront", "mapline", "codeforge", "chatline", "drivebox", "wikiverse",
    "ridealong", "payrail", "learnly", "podcasty", "fitlog", "tixly", "stocksnap",
];

struct Builder {
    cfg: TopologyConfig,
    rng: Rng,
    ases: Vec<AutonomousSystem>,
    routers: Vec<Router>,
    links: Vec<Link>,
    services: Vec<Service>,
    adjacency: Vec<Vec<(LinkId, RouterId)>>,
    pairs: HashSet<(RouterId, RouterId)>,
    used_names: HashSet<String>,
}

impl Builder {
    fn new(cfg: TopologyConfig) -> Self {
        let rng = Rng::new(cfg.seed);
        Builder {
            cfg,
            rng,
            ases: Vec::new(),
            routers: Vec::new(),
            links: Vec::new(),
            services: Vec::new(),
            adjacency: Vec::new(),
            pairs: HashSet::new(),
            used_names: HashSet::new(),
        }
    }

    fn build(mut self) -> Topology {
        assert!(self.cfg.hosting <= self.cfg.stubs, "more hosting stubs than stubs");
        assert!(self.cfg.tier1 >= 2, "need at least two tier-1s to have a core");

        let mut rng = self.rng.fork(1);
        let t1: Vec<AsId> = (0..self.cfg.tier1).map(|_| self.tier1(&mut rng)).collect();
        let t2: Vec<AsId> = (0..self.cfg.tier2).map(|_| self.tier2(&mut rng)).collect();
        let stubs: Vec<AsId> = (0..self.cfg.stubs)
            .map(|i| self.stub(&mut rng, i < self.cfg.hosting))
            .collect();

        let mut rng = self.rng.fork(2);
        self.peer_tier1s(&t1);
        for &a in &t2 {
            self.attach_tier2(&mut rng, a, &t1);
        }
        self.peer_tier2s(&mut rng, &t2);
        for &a in &stubs {
            self.attach_stub(&mut rng, a, &t1, &t2);
        }

        let mut rng = self.rng.fork(3);
        self.place_services(&mut rng);
        let (host_router, host_rate_bps, server_hosts) = self.place_hosts(&mut rng);

        let mut inter_as: HashMap<(AsId, AsId), Vec<LinkId>> = HashMap::new();
        for l in &self.links {
            if l.is_inter_as() {
                let (x, y) = (self.routers[l.a.idx()].asys, self.routers[l.b.idx()].asys);
                let key = if x < y { (x, y) } else { (y, x) };
                inter_as.entry(key).or_default().push(l.id);
            }
        }

        Topology {
            config: self.cfg,
            ases: self.ases,
            routers: self.routers,
            links: self.links,
            services: self.services,
            host_router,
            host_rate_bps,
            server_hosts,
            adjacency: self.adjacency,
            inter_as,
        }
    }

    // -- primitives --------------------------------------------------------

    fn new_as(&mut self, rng: &mut Rng, tier: Tier, region: Region, home: CityId, hosting: bool) -> AsId {
        let id = AsId::from_idx(self.ases.len());
        let tails = match (tier, hosting) {
            (Tier::Tier1, _) => T1_TAILS,
            (Tier::Tier2, _) => T2_TAILS,
            (Tier::Stub, true) => HOST_TAILS,
            (Tier::Stub, false) => STUB_TAILS,
        };
        // Names repeat eventually with 40-odd heads; prefix the city code
        // for stubs, which is what small ISPs tend to look like anyway.
        let name = loop {
            let head = rng.pick(NAME_HEADS);
            let tail = rng.pick(tails);
            let candidate = match tier {
                Tier::Stub => format!("{} {head} {tail}", geo::city(home).name),
                _ => format!("{head} {tail}"),
            };
            if self.used_names.insert(candidate.clone()) {
                break candidate;
            }
        };
        self.ases.push(AutonomousSystem {
            id,
            asn: 64512 + id.0 as u32,
            name,
            tier,
            region,
            home,
            routers: Vec::new(),
            neighbors: Vec::new(),
            hosting,
        });
        id
    }

    fn new_router(&mut self, rng: &mut Rng, asys: AsId, city: CityId, role: RouterRole) -> RouterId {
        let id = RouterId::from_idx(self.routers.len());
        let c = geo::city(city);
        // Spread routers in the same city a little so the map does not
        // stack them. ~30-50 km: metro scale.
        let (lat, lon) = (c.lat + rng.normal() * 0.25, c.lon + rng.normal() * 0.35);
        let tag = match role {
            RouterRole::Core => "cr",
            RouterRole::Border => "br",
            RouterRole::Access => "ar",
            RouterRole::Datacenter => "dc",
        };
        let n = self.ases[asys.idx()]
            .routers
            .iter()
            .filter(|r| {
                let r = &self.routers[r.idx()];
                r.city == city && r.role == role
            })
            .count();
        // Reverse-DNS style, the way they show up in a traceroute.
        let name = format!("{}-{tag}{}.as{}", c.code, n + 1, self.ases[asys.idx()].asn);
        self.routers.push(Router {
            id,
            asys,
            city,
            role,
            name,
            lat,
            lon,
            hosts: 0..0,
        });
        self.adjacency.push(Vec::new());
        self.ases[asys.idx()].routers.push(id);
        id
    }

    fn connect(&mut self, a: RouterId, b: RouterId, kind: LinkKind, capacity_bps: f64) -> Option<LinkId> {
        if a == b {
            return None;
        }
        let key = if a < b { (a, b) } else { (b, a) };
        if !self.pairs.insert(key) {
            return None;
        }
        let (ra, rb) = (&self.routers[a.idx()], &self.routers[b.idx()]);
        let km = geo::haversine_km(ra.lat, ra.lon, rb.lat, rb.lon);
        let delay_s = geo::fibre_delay_s(km);
        let cost = ((delay_s * 1e4).round() as u32).max(1);
        let fragility = self.rng.lognormal(1.0, 0.55);
        let id = LinkId::from_idx(self.links.len());
        self.links.push(Link {
            id,
            a,
            b,
            kind,
            capacity_bps,
            delay_s,
            km,
            cost,
            fragility,
        });
        self.adjacency[a.idx()].push((id, b));
        self.adjacency[b.idx()].push((id, a));
        Some(id)
    }

    fn relate(&mut self, a: AsId, b: AsId, b_is_to_a: Relationship) {
        let na = &mut self.ases[a.idx()].neighbors;
        if na.iter().any(|(n, _)| *n == b) {
            return;
        }
        na.push((b, b_is_to_a));
        self.ases[b.idx()].neighbors.push((a, b_is_to_a.inverse()));
    }

    /// Wire up a set of points-of-presence into a backbone: minimum
    /// spanning tree on distance (so every POP is reachable) plus each POP's
    /// `extra` nearest neighbours (so a single cut does not partition it).
    fn backbone(&mut self, pops: &[(CityId, Vec<RouterId>)], extra: usize, cap: f64) {
        let n = pops.len();
        if n < 2 {
            return;
        }
        let d = |i: usize, j: usize| geo::city_distance_km(pops[i].0, pops[j].0);

        let mut edges: Vec<(usize, usize)> = Vec::new();
        // Prim, O(n^2), n is at most a few dozen.
        let mut in_tree = vec![false; n];
        let mut best = vec![(f64::INFINITY, 0usize); n];
        in_tree[0] = true;
        for j in 1..n {
            best[j] = (d(0, j), 0);
        }
        for _ in 1..n {
            let (j, _) = (0..n)
                .filter(|&j| !in_tree[j])
                .map(|j| (j, best[j].0))
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .unwrap();
            in_tree[j] = true;
            edges.push((best[j].1, j));
            for k in 0..n {
                if !in_tree[k] && d(j, k) < best[k].0 {
                    best[k] = (d(j, k), j);
                }
            }
        }
        for i in 0..n {
            let mut others: Vec<usize> = (0..n).filter(|&j| j != i).collect();
            others.sort_by(|&a, &b| d(i, a).total_cmp(&d(i, b)));
            for &j in others.iter().take(extra) {
                edges.push((i, j));
            }
        }

        let mut seen = HashSet::new();
        for (k, (i, j)) in edges.into_iter().enumerate() {
            if !seen.insert((i.min(j), i.max(j))) {
                continue;
            }
            // Rotate through the routers of each POP so one box does not
            // terminate every long-haul circuit.
            let a = pops[i].1[k % pops[i].1.len()];
            let b = pops[j].1[k % pops[j].1.len()];
            self.connect(a, b, LinkKind::Internal, cap);
        }
    }

    fn pick_cities(&mut self, rng: &mut Rng, pool: &[CityId], k: usize, bias: f64) -> Vec<CityId> {
        let mut pool: Vec<CityId> = pool.to_vec();
        let mut out = Vec::new();
        while out.len() < k && !pool.is_empty() {
            let w = WeightedIndex::new(pool.iter().map(|&c| geo::city(c).hub.powf(bias) + 0.05));
            out.push(pool.swap_remove(w.sample(rng)));
        }
        out
    }

    fn pick_region(rng: &mut Rng) -> Region {
        let w = WeightedIndex::new(Region::ALL.iter().map(|r| r.weight()));
        Region::ALL[w.sample(rng)]
    }

    // -- ASes ---------------------------------------------------------------

    fn tier1(&mut self, rng: &mut Rng) -> AsId {
        let all: Vec<CityId> = (0..CITIES.len()).map(CityId::from_idx).collect();
        let n_pops = 14 + rng.below(8);
        let mut pops = self.pick_cities(rng, &all, n_pops, 2.0);
        // A tier-1 without presence on the three big continents is not one.
        for region in [Region::NorthAmerica, Region::Europe, Region::Asia] {
            if !pops.iter().any(|&c| geo::city(c).region == region) {
                let pool: Vec<_> = geo::cities_in(region).collect();
                pops.extend(self.pick_cities(rng, &pool, 2, 2.0));
            }
        }
        let home = pops[0];
        let id = self.new_as(rng, Tier::Tier1, geo::city(home).region, home, false);

        let mut groups = Vec::new();
        for &c in &pops {
            let n = if geo::city(c).hub >= 0.9 { 2 } else { 1 };
            let rs: Vec<_> = (0..n).map(|_| self.new_router(rng, id, c, RouterRole::Core)).collect();
            if n == 2 {
                self.connect(rs[0], rs[1], LinkKind::Internal, 100.0 * GBPS);
            }
            groups.push((c, rs));
        }
        self.backbone(&groups, 2, 100.0 * GBPS);
        self.ocean_crossings(&groups, 100.0 * GBPS);
        id
    }

    /// Distance-based wiring happily builds a global backbone with a single
    /// cable across each ocean. Real carriers buy capacity on diverse
    /// systems, and close the loop around the planet, precisely so that one
    /// cable cut cannot split them in half. Add those crossings explicitly:
    /// for each continent pair the carrier spans, connect the closest POP
    /// pairs that are not already connected (two for the Atlantic).
    fn ocean_crossings(&mut self, pops: &[(CityId, Vec<RouterId>)], cap: f64) {
        use Region::*;
        let routes = [
            (NorthAmerica, Europe, 2),
            (Europe, Asia, 2),
            (Asia, NorthAmerica, 2),
            (NorthAmerica, SouthAmerica, 1),
            (Europe, Africa, 1),
            (Asia, Oceania, 1),
            (Asia, Africa, 1),
            (Oceania, NorthAmerica, 1),
        ];
        let region_of = |c: CityId| geo::city(c).region;
        for (ra, rb, want) in routes {
            let mut pairs: Vec<(f64, usize, usize)> = Vec::new();
            for (i, (ca, _)) in pops.iter().enumerate() {
                for (j, (cb, _)) in pops.iter().enumerate() {
                    if region_of(*ca) == ra && region_of(*cb) == rb {
                        pairs.push((geo::city_distance_km(*ca, *cb), i, j));
                    }
                }
            }
            pairs.sort_by(|x, y| x.0.total_cmp(&y.0));
            let (mut made, mut used) = (0, HashSet::new());
            for (_, i, j) in pairs {
                if made == want {
                    break;
                }
                // Diverse means different landing cities at both ends.
                if used.contains(&i) || used.contains(&j) {
                    continue;
                }
                let (a, b) = (*pops[i].1.last().unwrap(), pops[j].1[0]);
                if self.connect(a, b, LinkKind::Internal, cap).is_some() {
                    made += 1;
                }
                used.insert(i);
                used.insert(j);
            }
        }
    }

    fn tier2(&mut self, rng: &mut Rng) -> AsId {
        let region = Self::pick_region(rng);
        let pool: Vec<CityId> = geo::cities_in(region).collect();
        let n_pops = (3 + rng.below(5)).min(pool.len());
        let pops = self.pick_cities(rng, &pool, n_pops, 1.0);
        let home = pops[0];
        let id = self.new_as(rng, Tier::Tier2, region, home, false);

        let mut groups = Vec::new();
        for (i, &c) in pops.iter().enumerate() {
            let n = if i == 0 { 2 } else { 1 };
            let rs: Vec<_> = (0..n).map(|_| self.new_router(rng, id, c, RouterRole::Core)).collect();
            if n == 2 {
                self.connect(rs[0], rs[1], LinkKind::Internal, 40.0 * GBPS);
            }
            groups.push((c, rs));
        }
        self.backbone(&groups, 1, 40.0 * GBPS);
        id
    }

    fn stub(&mut self, rng: &mut Rng, hosting: bool) -> AsId {
        let region = if hosting {
            // Datacenters cluster where the interconnection is.
            [Region::NorthAmerica, Region::Europe, Region::Asia][rng.below(3)]
        } else {
            Self::pick_region(rng)
        };
        let pool: Vec<CityId> = geo::cities_in(region).collect();
        let home = self.pick_cities(rng, &pool, 1, if hosting { 2.0 } else { 0.7 })[0];
        let id = self.new_as(rng, Tier::Stub, region, home, hosting);

        let (n_border, n_edge, edge_role, cap) = if hosting {
            (2, 2, RouterRole::Datacenter, 40.0 * GBPS)
        } else {
            let n_border = if rng.chance(0.6) { 2 } else { 1 };
            (n_border, 2 + rng.below(3), RouterRole::Access, 10.0 * GBPS)
        };
        let borders: Vec<_> = (0..n_border)
            .map(|_| self.new_router(rng, id, home, RouterRole::Border))
            .collect();
        let edges: Vec<_> = (0..n_edge)
            .map(|_| self.new_router(rng, id, home, edge_role))
            .collect();
        if n_border == 2 {
            self.connect(borders[0], borders[1], LinkKind::Internal, cap);
        }
        for &e in &edges {
            for &b in &borders {
                self.connect(e, b, LinkKind::Internal, cap);
            }
        }
        // Single-homed access routers in a small ISP are common, but a
        // little lateral redundancy makes failures more interesting.
        for w in edges.windows(2) {
            if rng.chance(0.3) {
                self.connect(w[0], w[1], LinkKind::Internal, cap / 4.0);
            }
        }
        id
    }

    // -- interconnection ----------------------------------------------------

    fn routers_in(&self, a: AsId, city: CityId) -> Vec<RouterId> {
        self.ases[a.idx()]
            .routers
            .iter()
            .copied()
            .filter(|r| self.routers[r.idx()].city == city)
            .collect()
    }

    fn pops_of(&self, a: AsId) -> Vec<CityId> {
        let mut v: Vec<CityId> = self.ases[a.idx()]
            .routers
            .iter()
            .map(|r| self.routers[r.idx()].city)
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// The `k` closest (router of a, router of b) pairs, using each router
    /// of `a` at most once. Candidates on `a`'s side can be restricted (stubs
    /// only interconnect from their border routers).
    fn closest_pairs(&self, a_side: &[RouterId], b: AsId, k: usize) -> Vec<(RouterId, RouterId, f64)> {
        let b_side = &self.ases[b.idx()].routers;
        let mut cands: Vec<(RouterId, RouterId, f64)> = Vec::new();
        for &x in a_side {
            let rx = &self.routers[x.idx()];
            let best = b_side
                .iter()
                .map(|&y| {
                    let ry = &self.routers[y.idx()];
                    (y, geo::haversine_km(rx.lat, rx.lon, ry.lat, ry.lon))
                })
                .min_by(|p, q| p.1.total_cmp(&q.1));
            if let Some((y, d)) = best {
                cands.push((x, y, d));
            }
        }
        cands.sort_by(|p, q| p.2.total_cmp(&q.2));
        let mut used_b = HashSet::new();
        cands.retain(|&(_, y, _)| used_b.insert(y));
        cands.truncate(k);
        cands
    }

    fn distance_to_as(&self, from: CityId, to: AsId) -> f64 {
        self.pops_of(to)
            .into_iter()
            .map(|c| geo::city_distance_km(from, c))
            .fold(f64::INFINITY, f64::min)
    }

    fn peer_tier1s(&mut self, t1: &[AsId]) {
        for (i, &a) in t1.iter().enumerate() {
            for &b in &t1[i + 1..] {
                let pa = self.pops_of(a);
                let pb: HashSet<_> = self.pops_of(b).into_iter().collect();
                let mut shared: Vec<CityId> = pa.into_iter().filter(|c| pb.contains(c)).collect();
                shared.sort_by(|x, y| geo::city(*y).hub.total_cmp(&geo::city(*x).hub));
                let mut made = 0;
                for &c in shared.iter().take(3) {
                    let (ra, rb) = (self.routers_in(a, c), self.routers_in(b, c));
                    if self.connect(ra[0], rb[0], LinkKind::Peering, 40.0 * GBPS).is_some() {
                        made += 1;
                    }
                }
                if made == 0 {
                    let a_side = self.ases[a.idx()].routers.clone();
                    for (x, y, _) in self.closest_pairs(&a_side, b, 2) {
                        self.connect(x, y, LinkKind::Peering, 40.0 * GBPS);
                    }
                }
                self.relate(a, b, Relationship::Peer);
            }
        }
    }

    fn choose_upstreams(&mut self, rng: &mut Rng, home: CityId, pool: &[AsId], k: usize) -> Vec<AsId> {
        let mut pool: Vec<AsId> = pool.to_vec();
        let mut out = Vec::new();
        while out.len() < k && !pool.is_empty() {
            // Closer upstreams are much more likely, but not certain: plenty
            // of networks buy transit from someone a continent away.
            let w = WeightedIndex::new(
                pool.iter()
                    .map(|&p| 1.0 / (1.0 + self.distance_to_as(home, p) / 800.0).powi(2)),
            );
            out.push(pool.swap_remove(w.sample(rng)));
        }
        out
    }

    fn attach_tier2(&mut self, rng: &mut Rng, a: AsId, t1: &[AsId]) {
        let k = if rng.chance(0.4) { 3 } else { 2 };
        let home = self.ases[a.idx()].home;
        for p in self.choose_upstreams(rng, home, t1, k.min(t1.len())) {
            let a_side = self.ases[a.idx()].routers.clone();
            let n_links = if rng.chance(0.5) { 2 } else { 1 };
            for (x, y, _) in self.closest_pairs(&a_side, p, n_links) {
                self.connect(x, y, LinkKind::Transit, 40.0 * GBPS);
            }
            self.relate(a, p, Relationship::Provider);
        }
    }

    fn peer_tier2s(&mut self, rng: &mut Rng, t2: &[AsId]) {
        for (i, &a) in t2.iter().enumerate() {
            for &b in &t2[i + 1..] {
                if self.ases[a.idx()].region != self.ases[b.idx()].region || !rng.chance(0.35) {
                    continue;
                }
                let a_side = self.ases[a.idx()].routers.clone();
                if let Some(&(x, y, _)) = self.closest_pairs(&a_side, b, 1).first() {
                    self.connect(x, y, LinkKind::Peering, 10.0 * GBPS);
                    self.relate(a, b, Relationship::Peer);
                }
            }
        }
    }

    fn attach_stub(&mut self, rng: &mut Rng, a: AsId, t1: &[AsId], t2: &[AsId]) {
        let (home, region, hosting) = {
            let s = &self.ases[a.idx()];
            (s.home, s.region, s.hosting)
        };
        let borders: Vec<RouterId> = self.ases[a.idx()]
            .routers
            .iter()
            .copied()
            .filter(|r| self.routers[r.idx()].role == RouterRole::Border)
            .collect();

        let regional: Vec<AsId> = t2
            .iter()
            .copied()
            .filter(|&p| self.ases[p.idx()].region == region)
            .collect();
        let regional = if regional.is_empty() { t2.to_vec() } else { regional };

        let mut upstreams = Vec::new();
        if hosting {
            // Hosting networks buy from a tier-1 and a regional carrier at
            // least; that is the whole point of being a hosting network.
            upstreams.extend(self.choose_upstreams(rng, home, t1, 1));
            let k = 1 + rng.below(2);
            upstreams.extend(self.choose_upstreams(rng, home, &regional, k));
        } else {
            let k = borders.len();
            for _ in 0..k {
                let pool: Vec<AsId> = if rng.chance(0.12) { t1.to_vec() } else { regional.clone() };
                let pool: Vec<AsId> = pool.into_iter().filter(|p| !upstreams.contains(p)).collect();
                if pool.is_empty() {
                    continue;
                }
                upstreams.extend(self.choose_upstreams(rng, home, &pool, 1));
            }
        }

        let cap = if hosting {
            40.0 * GBPS
        } else if rng.chance(0.25) {
            20.0 * GBPS
        } else {
            10.0 * GBPS
        };
        for (i, p) in upstreams.into_iter().enumerate() {
            let b = borders[i % borders.len()];
            if let Some(&(x, y, _)) = self.closest_pairs(&[b], p, 1).first() {
                self.connect(x, y, LinkKind::Transit, cap);
                self.relate(a, p, Relationship::Provider);
            }
        }
    }

    // -- services and hosts ---------------------------------------------------

    fn place_services(&mut self, rng: &mut Rng) {
        let dcs: Vec<RouterId> = self
            .routers
            .iter()
            .filter(|r| r.role == RouterRole::Datacenter)
            .map(|r| r.id)
            .collect();
        if dcs.is_empty() {
            return;
        }
        let total: f64 = (1..=self.cfg.services).map(|k| 1.0 / k as f64).sum();
        for rank in 0..self.cfg.services {
            // The big services are everywhere; the long tail lives in one
            // building.
            let n_replicas = match rank {
                0..=1 => 4,
                2..=4 => 3,
                5..=8 => 2,
                _ => 1,
            }
            .min(dcs.len());
            // Spread replicas over different hosting ASes where possible.
            let mut by_as: HashMap<AsId, Vec<RouterId>> = HashMap::new();
            for &d in &dcs {
                by_as.entry(self.routers[d.idx()].asys).or_default().push(d);
            }
            let mut as_list: Vec<AsId> = by_as.keys().copied().collect();
            as_list.sort_unstable();
            rng.shuffle(&mut as_list);
            let replicas: Vec<RouterId> = as_list
                .iter()
                .take(n_replicas)
                .map(|a| *rng.pick(&by_as[a]))
                .collect();
            let name = SERVICE_NAMES
                .get(rank)
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("service-{rank}"));
            self.services.push(Service {
                id: ServiceId::from_idx(rank),
                name,
                replicas,
                popularity: (1.0 / (rank + 1) as f64) / total,
            });
        }
    }

    fn place_hosts(&mut self, rng: &mut Rng) -> (Vec<RouterId>, Vec<f32>, Range<u32>) {
        let access: Vec<RouterId> = self
            .routers
            .iter()
            .filter(|r| r.role == RouterRole::Access)
            .map(|r| r.id)
            .collect();
        let mut dc_replicas: Vec<RouterId> = self.services.iter().flat_map(|s| s.replicas.clone()).collect();
        dc_replicas.sort_unstable();
        dc_replicas.dedup();

        let n_servers = dc_replicas.len() * self.cfg.servers_per_replica;
        let budget = self
            .cfg
            .nodes
            .checked_sub(self.routers.len() + n_servers)
            .expect("node budget too small for the routers alone");

        // Uneven population per access router, like real metros.
        let weights: Vec<f64> = access.iter().map(|_| rng.lognormal(1.0, 0.6)).collect();
        let wsum: f64 = weights.iter().sum();
        let mut counts: Vec<usize> = weights
            .iter()
            .map(|w| (w / wsum * budget as f64).floor() as usize)
            .collect();
        // Hand out the rounding remainder so the total is exact.
        let mut short = budget - counts.iter().sum::<usize>();
        let (mut i, n_access) = (0, counts.len());
        while short > 0 {
            counts[i % n_access] += 1;
            short -= 1;
            i += 1;
        }

        let mut host_router = Vec::with_capacity(budget + n_servers);
        let mut host_rate = Vec::with_capacity(budget + n_servers);
        for (&r, &n) in access.iter().zip(&counts) {
            let start = host_router.len() as u32;
            for _ in 0..n {
                host_router.push(r);
                let x = rng.f64();
                let rate = if x < 0.05 {
                    25e6
                } else if x < 0.55 {
                    100e6
                } else if x < 0.85 {
                    300e6
                } else {
                    1e9
                };
                host_rate.push(rate as f32);
            }
            self.routers[r.idx()].hosts = start..host_router.len() as u32;
        }

        let servers_start = host_router.len() as u32;
        for &r in &dc_replicas {
            let start = host_router.len() as u32;
            for _ in 0..self.cfg.servers_per_replica {
                host_router.push(r);
                host_rate.push(25e9);
            }
            self.routers[r.idx()].hosts = start..host_router.len() as u32;
        }
        let servers = servers_start..host_router.len() as u32;
        (host_router, host_rate, servers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::algo::dijkstra;

    #[test]
    fn deterministic() {
        let a = Topology::generate(&TopologyConfig::small(1));
        let b = Topology::generate(&TopologyConfig::small(1));
        assert_eq!(a.summary(), b.summary());
        for (x, y) in a.links.iter().zip(&b.links) {
            assert_eq!((x.a, x.b, x.cost), (y.a, y.b, y.cost));
        }
        let c = Topology::generate(&TopologyConfig::small(2));
        assert_ne!(
            a.links.iter().map(|l| l.cost).collect::<Vec<_>>(),
            c.links.iter().map(|l| l.cost).collect::<Vec<_>>()
        );
    }

    #[test]
    fn hits_the_node_budget_exactly() {
        let cfg = TopologyConfig::small(3);
        let t = Topology::generate(&cfg);
        assert_eq!(t.node_count(), cfg.nodes);
    }

    #[test]
    fn default_config_is_a_hundred_thousand_nodes() {
        let t = Topology::generate(&TopologyConfig::default());
        let s = t.summary();
        assert_eq!(s.nodes, 100_000);
        assert!(s.routers > 1000, "{s:?}");
    }

    #[test]
    fn physically_connected() {
        for seed in 0..5 {
            let t = Topology::generate(&TopologyConfig::small(seed));
            let sp = dijkstra(&t.router_graph(), 0);
            assert!((0..t.routers.len()).all(|v| sp.reachable(v)), "seed {seed}");
        }
    }

    #[test]
    fn every_as_is_internally_connected() {
        let t = Topology::generate(&TopologyConfig::small(4));
        for asys in &t.ases {
            let ids: HashMap<RouterId, usize> = asys.routers.iter().enumerate().map(|(i, &r)| (r, i)).collect();
            let edges = t.links.iter().filter(|l| l.kind == LinkKind::Internal).filter_map(|l| {
                Some((*ids.get(&l.a)?, *ids.get(&l.b)?, l.cost as u64, l.id.0))
            });
            let g = Csr::undirected(ids.len(), edges);
            let sp = dijkstra(&g, 0);
            assert!((0..ids.len()).all(|v| sp.reachable(v)), "{}", asys.name);
        }
    }

    #[test]
    fn relationships_are_symmetric_and_backed_by_links() {
        let t = Topology::generate(&TopologyConfig::small(5));
        for a in &t.ases {
            for &(b, rel) in &a.neighbors {
                assert_eq!(t.relationship(b, a.id), Some(rel.inverse()));
                assert!(!t.links_between(a.id, b).is_empty(), "{} - {}", a.name, t.asys(b).name);
            }
        }
    }

    #[test]
    fn every_non_tier1_has_a_provider() {
        let t = Topology::generate(&TopologyConfig::small(6));
        for a in t.ases.iter().filter(|a| a.tier != Tier::Tier1) {
            assert!(
                a.neighbors.iter().any(|&(_, r)| r == Relationship::Provider),
                "{} has no upstream",
                a.name
            );
        }
    }

    #[test]
    fn services_live_in_datacenters() {
        let t = Topology::generate(&TopologyConfig::small(7));
        assert_eq!(t.services.len(), 5);
        let total: f64 = t.services.iter().map(|s| s.popularity).sum();
        assert!((total - 1.0).abs() < 1e-9);
        for s in &t.services {
            assert!(!s.replicas.is_empty());
            for &r in &s.replicas {
                assert_eq!(t.router(r).role, RouterRole::Datacenter);
                assert!(!t.router(r).hosts.is_empty());
            }
        }
    }

    #[test]
    fn host_ranges_match_host_table() {
        let t = Topology::generate(&TopologyConfig::small(8));
        for r in &t.routers {
            for h in r.hosts.clone() {
                assert_eq!(t.host_router[h as usize], r.id);
            }
        }
    }
}
