#!/usr/bin/env node
// Stand-in for shadownetd while developing the dashboard. Speaks the protocol
// in docs/PROTOCOL.md on :7070 with a generated world and a toy traffic model.
// Nothing here is a simulation; it only has to look plausible and react to
// every command the dashboard can send.
//
//   node scripts/mock-server.mjs [--port 7070] [--seed 98430951]
//
// If dashboard/out exists (after `npm run build`) it is served at / too, so
// the static export can be checked exactly as shadownetd will serve it.

import { createHash } from "node:crypto";
import { existsSync, readFileSync, statSync } from "node:fs";
import http from "node:http";
import { dirname, extname, join, normalize } from "node:path";
import { fileURLToPath } from "node:url";

const args = new Map();
for (let i = 2; i < process.argv.length; i += 2) args.set(process.argv[i].replace(/^--/, ""), process.argv[i + 1]);
const PORT = Number(args.get("port") ?? process.env.PORT ?? 7070);
const INITIAL_SEED = Number(args.get("seed") ?? 98430951);
const FRAME_MS = 200;
const OUT_DIR = join(dirname(fileURLToPath(import.meta.url)), "..", "out");

// ---------------------------------------------------------------- utilities

function mulberry32(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

let rand = mulberry32(INITIAL_SEED);
const randInt = (lo, hi) => lo + Math.floor(rand() * (hi - lo + 1));
const pick = (arr) => arr[Math.floor(rand() * arr.length)];
const clamp = (v, lo, hi) => Math.max(lo, Math.min(hi, v));
const gauss = () => {
  const u = 1 - rand();
  const v = rand();
  return Math.sqrt(-2 * Math.log(u)) * Math.cos(2 * Math.PI * v);
};

function shuffle(arr) {
  for (let i = arr.length - 1; i > 0; i--) {
    const j = Math.floor(rand() * (i + 1));
    [arr[i], arr[j]] = [arr[j], arr[i]];
  }
  return arr;
}

function haversineKm(lat1, lon1, lat2, lon2) {
  const r = Math.PI / 180;
  const dLat = (lat2 - lat1) * r;
  const dLon = (lon2 - lon1) * r;
  const a = Math.sin(dLat / 2) ** 2 + Math.cos(lat1 * r) * Math.cos(lat2 * r) * Math.sin(dLon / 2) ** 2;
  return 2 * 6371 * Math.asin(Math.sqrt(a));
}

class MinHeap {
  constructor() {
    this.k = [];
    this.v = [];
  }
  get size() {
    return this.k.length;
  }
  push(key, val) {
    const k = this.k;
    const v = this.v;
    k.push(key);
    v.push(val);
    let i = k.length - 1;
    while (i > 0) {
      const p = (i - 1) >> 1;
      if (k[p] <= k[i]) break;
      [k[p], k[i]] = [k[i], k[p]];
      [v[p], v[i]] = [v[i], v[p]];
      i = p;
    }
  }
  pop() {
    const k = this.k;
    const v = this.v;
    const top = v[0];
    const lk = k.pop();
    const lv = v.pop();
    if (k.length) {
      k[0] = lk;
      v[0] = lv;
      let i = 0;
      for (;;) {
        const l = i * 2 + 1;
        const r = l + 1;
        let m = i;
        if (l < k.length && k[l] < k[m]) m = l;
        if (r < k.length && k[r] < k[m]) m = r;
        if (m === i) break;
        [k[m], k[i]] = [k[i], k[m]];
        [v[m], v[i]] = [v[i], v[m]];
        i = m;
      }
    }
    return top;
  }
}

// ---------------------------------------------------------------- geography

// [code, name, lat, lon, region, weight]. Weight steers how many networks
// put a point of presence there.
const CITIES = [
  ["nyc", "New York", 40.71, -74.01, "NA", 5],
  ["ash", "Ashburn", 39.04, -77.49, "NA", 5],
  ["chi", "Chicago", 41.88, -87.63, "NA", 4],
  ["dal", "Dallas", 32.78, -96.8, "NA", 4],
  ["lax", "Los Angeles", 34.05, -118.24, "NA", 5],
  ["sjc", "San Jose", 37.34, -121.89, "NA", 5],
  ["sea", "Seattle", 47.61, -122.33, "NA", 3],
  ["mia", "Miami", 25.76, -80.19, "NA", 4],
  ["atl", "Atlanta", 33.75, -84.39, "NA", 3],
  ["den", "Denver", 39.74, -104.99, "NA", 2],
  ["tor", "Toronto", 43.65, -79.38, "NA", 3],
  ["mtl", "Montreal", 45.5, -73.57, "NA", 2],
  ["yvr", "Vancouver", 49.28, -123.12, "NA", 2],
  ["mex", "Mexico City", 19.43, -99.13, "NA", 2],
  ["phx", "Phoenix", 33.45, -112.07, "NA", 1],
  ["bos", "Boston", 42.36, -71.06, "NA", 2],
  ["gru", "São Paulo", -23.55, -46.63, "SA", 4],
  ["rio", "Rio de Janeiro", -22.91, -43.17, "SA", 2],
  ["eze", "Buenos Aires", -34.6, -58.38, "SA", 2],
  ["scl", "Santiago", -33.45, -70.67, "SA", 2],
  ["bog", "Bogotá", 4.71, -74.07, "SA", 2],
  ["lim", "Lima", -12.05, -77.04, "SA", 1],
  ["lon", "London", 51.51, -0.13, "EU", 5],
  ["ams", "Amsterdam", 52.37, 4.9, "EU", 5],
  ["fra", "Frankfurt", 50.11, 8.68, "EU", 5],
  ["par", "Paris", 48.86, 2.35, "EU", 4],
  ["mad", "Madrid", 40.42, -3.7, "EU", 3],
  ["mil", "Milan", 45.46, 9.19, "EU", 3],
  ["sto", "Stockholm", 59.33, 18.07, "EU", 3],
  ["waw", "Warsaw", 52.23, 21.01, "EU", 2],
  ["vie", "Vienna", 48.21, 16.37, "EU", 2],
  ["zrh", "Zurich", 47.38, 8.54, "EU", 2],
  ["dub", "Dublin", 53.35, -6.26, "EU", 3],
  ["mrs", "Marseille", 43.3, 5.37, "EU", 3],
  ["hel", "Helsinki", 60.17, 24.94, "EU", 2],
  ["lis", "Lisbon", 38.72, -9.14, "EU", 2],
  ["ist", "Istanbul", 41.01, 28.98, "EU", 3],
  ["mow", "Moscow", 55.76, 37.62, "EU", 3],
  ["ath", "Athens", 37.98, 23.73, "EU", 1],
  ["buh", "Bucharest", 44.43, 26.1, "EU", 1],
  ["dxb", "Dubai", 25.2, 55.27, "ME", 4],
  ["ruh", "Riyadh", 24.71, 46.68, "ME", 2],
  ["tlv", "Tel Aviv", 32.09, 34.78, "ME", 2],
  ["cai", "Cairo", 30.04, 31.24, "AF", 2],
  ["jnb", "Johannesburg", -26.2, 28.05, "AF", 3],
  ["cpt", "Cape Town", -33.92, 18.42, "AF", 2],
  ["los", "Lagos", 6.52, 3.38, "AF", 2],
  ["nbo", "Nairobi", -1.29, 36.82, "AF", 2],
  ["cmn", "Casablanca", 33.57, -7.59, "AF", 1],
  ["bom", "Mumbai", 19.08, 72.88, "AS", 4],
  ["del", "New Delhi", 28.61, 77.21, "AS", 3],
  ["maa", "Chennai", 13.08, 80.27, "AS", 3],
  ["blr", "Bangalore", 12.97, 77.59, "AS", 2],
  ["sin", "Singapore", 1.35, 103.82, "AS", 5],
  ["hkg", "Hong Kong", 22.32, 114.17, "AS", 5],
  ["tyo", "Tokyo", 35.68, 139.69, "AS", 5],
  ["osa", "Osaka", 34.69, 135.5, "AS", 2],
  ["sel", "Seoul", 37.57, 126.98, "AS", 3],
  ["tpe", "Taipei", 25.03, 121.57, "AS", 2],
  ["sha", "Shanghai", 31.23, 121.47, "AS", 3],
  ["bjs", "Beijing", 39.9, 116.4, "AS", 2],
  ["bkk", "Bangkok", 13.76, 100.5, "AS", 2],
  ["kul", "Kuala Lumpur", 3.14, 101.69, "AS", 2],
  ["jkt", "Jakarta", -6.21, 106.85, "AS", 3],
  ["mnl", "Manila", 14.6, 120.98, "AS", 2],
  ["syd", "Sydney", -33.87, 151.21, "OC", 4],
  ["mel", "Melbourne", -37.81, 144.96, "OC", 2],
  ["per", "Perth", -31.95, 115.86, "OC", 1],
  ["akl", "Auckland", -36.85, 174.76, "OC", 2],
];

const cities = CITIES.map(([code, name, lat, lon, region, weight]) => ({ code, name, lat, lon, region, weight }));
const cityByCode = new Map(cities.map((c) => [c.code, c]));
const REGIONS = ["NA", "SA", "EU", "ME", "AF", "AS", "OC"];
const REGION_WEIGHT = { NA: 6, SA: 3, EU: 7, ME: 2, AF: 2, AS: 7, OC: 2 };

// Ocean crossings a global backbone builds when it has POPs on both sides.
const LONG_HAUL = [
  ["NA", "EU"],
  ["NA", "EU"],
  ["NA", "AS"],
  ["NA", "SA"],
  ["EU", "AS"],
  ["EU", "ME"],
  ["ME", "AS"],
  ["EU", "AF"],
  ["AS", "OC"],
  ["NA", "OC"],
  ["SA", "AF"],
];

const TIER1_NAMES = [
  "Meridian Global Backbone",
  "Arclight Transit",
  "Northstar Carrier",
  "Helix Networks",
  "Tessera Global",
  "Corvid Telecom",
];
const TIER2_WORDS = ["Pacific", "Atlas", "Boreal", "Coastal", "Delta", "Summit", "Harbor", "Vector", "Lumen", "Granite", "Cobalt", "Aurora"];
const TIER2_SUFFIX = ["Fiber", "Networks", "Telecom", "Connect", "Communications", "Broadband"];
const STUB_WORDS = ["Kestrel", "Juniper", "Orchid", "Quartz", "Willow", "Ember", "Falcon", "Maple", "Onyx", "Saffron", "Tidal", "Birch", "Nimbus", "Cedar", "Pebble", "Zephyr", "Indigo", "Larch"];
const STUB_SUFFIX = ["ISP", "Online", "Hosting", "Cloud", "Net", "Access", "Datacenters", "Telecom"];
const SERVICE_NAMES = ["vidstream", "socialgraph", "searchd", "cdn-edge", "payments", "chatline", "mapsd", "storefront", "gamelobby", "musicbox", "newsfeed", "cloudvm", "mailhub", "docs", "adsrv", "telemetry"];

// ---------------------------------------------------------------- topology

function generateTopology(seed, runId) {
  rand = mulberry32(seed);
  const ases = [];
  const routers = [];
  const links = [];
  const linkSet = new Set();
  const nameCounter = new Map();

  const addAs = (tier, name, region, home, hosting = false) => {
    const as = { id: ases.length, asn: 0, name, tier, region, home, hosting };
    as.asn = tier === "tier1" ? 64512 + as.id * 3 : tier === "tier2" ? 64600 + as.id : 65000 + as.id * 7;
    ases.push(as);
    return as;
  };
  const roleCode = { core: "cr", border: "br", access: "ar", datacenter: "dc" };
  const addRouter = (as, cityCode, role, hosts = 0) => {
    const c = cityByCode.get(cityCode);
    const key = `${as.id}:${cityCode}:${role}`;
    const n = (nameCounter.get(key) ?? 0) + 1;
    nameCounter.set(key, n);
    const spread = as.tier === "stub" ? 0.9 : 0.45;
    const r = {
      id: routers.length,
      as: as.id,
      name: `${cityCode}-${roleCode[role]}${n}.as${as.asn}`,
      role,
      city: cityCode,
      lat: +(c.lat + gauss() * spread * 0.6).toFixed(3),
      lon: +(c.lon + gauss() * spread).toFixed(3),
      hosts,
    };
    routers.push(r);
    return r;
  };
  const addLink = (a, b, kind, capacityGbps) => {
    if (a.id === b.id) return null;
    const key = a.id < b.id ? `${a.id}-${b.id}` : `${b.id}-${a.id}`;
    if (linkSet.has(key)) return null;
    linkSet.add(key);
    const km = Math.max(2, Math.round(haversineKm(a.lat, a.lon, b.lat, b.lon) * 1.15));
    const l = { id: links.length, a: a.id, b: b.id, kind, capacityGbps, delayMs: +(km * 0.0049 + 0.03).toFixed(3), km };
    links.push(l);
    return l;
  };
  const dist = (a, b) => haversineKm(a.lat, a.lon, b.lat, b.lon);
  const nearest = (from, candidates, k = 1) =>
    candidates
      .filter((c) => c !== from)
      .map((c) => [dist(from, c), c])
      .sort((x, y) => x[0] - y[0])
      .slice(0, k)
      .map((x) => x[1]);

  // Connect a set of routers: k nearest neighbours, then stitch any leftover
  // components together so every AS is internally connected.
  const meshInternal = (rs, k, capacity) => {
    for (const r of rs) for (const n of nearest(r, rs, k)) addLink(r, n, "internal", capacity(r, n));
    const parent = new Map(rs.map((r) => [r.id, r.id]));
    const find = (x) => (parent.get(x) === x ? x : (parent.set(x, find(parent.get(x))), parent.get(x)));
    for (const l of links) if (parent.has(l.a) && parent.has(l.b)) parent.set(find(l.a), find(l.b));
    for (;;) {
      const groups = new Map();
      for (const r of rs) {
        const g = find(r.id);
        if (!groups.has(g)) groups.set(g, []);
        groups.get(g).push(r);
      }
      if (groups.size <= 1) break;
      const [first, ...rest] = [...groups.values()];
      let best = null;
      for (const a of first) for (const g of rest) for (const b of g) {
        const d = dist(a, b);
        if (!best || d < best[0]) best = [d, a, b];
      }
      addLink(best[1], best[2], "internal", capacity(best[1], best[2]));
      parent.set(find(best[1].id), find(best[2].id));
    }
  };

  const weightedCities = (pool, n) => {
    const chosen = new Set();
    const items = pool.slice();
    while (chosen.size < Math.min(n, items.length)) {
      const total = items.reduce((s, c) => s + (chosen.has(c.code) ? 0 : c.weight ** 1.6), 0);
      let x = rand() * total;
      for (const c of items) {
        if (chosen.has(c.code)) continue;
        x -= c.weight ** 1.6;
        if (x <= 0) {
          chosen.add(c.code);
          break;
        }
      }
    }
    return [...chosen];
  };

  // Tier 1: global backbones.
  const tier1 = [];
  for (const name of TIER1_NAMES) {
    const as = addAs("tier1", name, "global", "");
    const pops = weightedCities(cities, randInt(28, 36));
    as.home = pops[0];
    as.region = cityByCode.get(pops[0]).region;
    const rs = [];
    for (const code of pops) {
      const n = cityByCode.get(code).weight >= 3 ? 2 : 1;
      for (let i = 0; i < n; i++) rs.push(addRouter(as, code, "core"));
    }
    meshInternal(rs, 3, (a, b) => (dist(a, b) < 50 ? 400 : pick([100, 400, 400])));
    for (const [ra, rb] of LONG_HAUL) {
      const A = rs.filter((r) => cityByCode.get(r.city).region === ra);
      const B = rs.filter((r) => cityByCode.get(r.city).region === rb);
      if (!A.length || !B.length) continue;
      let best = null;
      for (const a of A) for (const b of B) {
        const d = dist(a, b) * (0.7 + rand() * 0.6);
        if (!best || d < best[0]) best = [d, a, b];
      }
      addLink(best[1], best[2], "internal", pick([100, 400]));
    }
    tier1.push({ as, routers: rs });
  }
  // Settlement-free full mesh between tier 1s, at shared cities.
  for (let i = 0; i < tier1.length; i++) {
    for (let j = i + 1; j < tier1.length; j++) {
      const A = tier1[i].routers;
      const B = tier1[j].routers;
      const shared = A.filter((a) => B.some((b) => b.city === a.city));
      shuffle(shared);
      for (const a of shared.slice(0, randInt(2, 4))) {
        const b = B.find((x) => x.city === a.city);
        addLink(a, b, "peering", 400);
      }
    }
  }

  // Tier 2: regional networks buying transit from tier 1s.
  const tier2 = [];
  const regionPool = REGIONS.flatMap((r) => Array(REGION_WEIGHT[r]).fill(r));
  for (let i = 0; i < 36; i++) {
    const region = regionPool[i % regionPool.length];
    const pool = cities.filter((c) => c.region === region);
    const pops = weightedCities(pool, randInt(6, Math.min(13, pool.length)));
    const as = addAs("tier2", `${pick(TIER2_WORDS)} ${pick(TIER2_SUFFIX)}`, region, pops[0]);
    const rs = [];
    for (const code of pops) {
      rs.push(addRouter(as, code, "core"));
      if (rand() < 0.6) rs.push(addRouter(as, code, "border"));
    }
    meshInternal(rs, 3, () => pick([40, 100, 100]));
    const providers = shuffle(tier1.slice()).slice(0, randInt(2, 3));
    for (const p of providers) {
      const local = rs.filter((r) => p.routers.some((x) => x.city === r.city));
      const r = local.length ? pick(local) : pick(rs);
      const peer = nearest(r, p.routers, 1)[0];
      addLink(r, peer, "transit", 100);
      if (rand() < 0.5) {
        const r2 = pick(rs);
        addLink(r2, nearest(r2, p.routers, 1)[0], "transit", 100);
      }
    }
    tier2.push({ as, routers: rs });
  }
  for (const t of tier2) {
    const peers = tier2.filter((o) => o !== t && o.as.region === t.as.region);
    for (const o of shuffle(peers).slice(0, randInt(1, 2))) {
      const a = pick(t.routers);
      addLink(a, nearest(a, o.routers, 1)[0], "peering", pick([40, 100]));
    }
  }

  // Stubs: access networks and hosting.
  const stubs = [];
  const totalHosts = 97520;
  for (let i = 0; i < 240; i++) {
    const region = pick(regionPool);
    const home = weightedCities(cities.filter((c) => c.region === region), 1)[0];
    const hosting = i % 6 === 0;
    const as = addAs("stub", `${pick(STUB_WORDS)} ${pick(STUB_SUFFIX)}`, region, home, hosting);
    const border = addRouter(as, home, "border");
    const rs = [border];
    const access = randInt(1, 4);
    for (let k = 0; k < access; k++) {
      const city = rand() < 0.75 ? home : weightedCities(cities.filter((c) => c.region === region), 1)[0];
      const ar = addRouter(as, city, "access");
      rs.push(ar);
      addLink(border, ar, "internal", pick([10, 40]));
    }
    if (hosting) {
      for (let k = 0; k < randInt(1, 2); k++) {
        const dc = addRouter(as, home, "datacenter");
        rs.push(dc);
        addLink(border, dc, "internal", 100);
      }
    }
    for (let k = 2; k < rs.length; k++) if (rand() < 0.5) addLink(rs[k - 1], rs[k], "internal", 10);
    const regional = tier2.filter((t) => t.as.region === region);
    const providers = shuffle(regional.length ? regional : tier2.slice()).slice(0, randInt(1, 3));
    for (const p of providers) addLink(border, nearest(border, p.routers, 1)[0], "transit", hosting ? 100 : pick([10, 40]));
    if (rand() < 0.15) addLink(border, nearest(border, pick(tier1).routers, 1)[0], "transit", 40);
    stubs.push({ as, routers: rs });
  }

  // Hosts hang off access routers.
  const accessRouters = routers.filter((r) => r.role === "access");
  let remaining = totalHosts;
  accessRouters.forEach((r, i) => {
    const share = i === accessRouters.length - 1 ? remaining : Math.round((totalHosts / accessRouters.length) * (0.4 + rand() * 1.2));
    r.hosts = Math.max(0, Math.min(remaining, share));
    remaining -= r.hosts;
  });

  const dcs = routers.filter((r) => r.role === "datacenter");
  const zipf = SERVICE_NAMES.map((_, i) => 1 / (i + 1) ** 1.1);
  const zsum = zipf.reduce((a, b) => a + b, 0);
  const services = SERVICE_NAMES.map((name, i) => ({
    id: i,
    name,
    replicas: shuffle(dcs.slice()).slice(0, randInt(2, 3)).map((r) => r.id),
    popularity: +(zipf[i] / zsum).toFixed(3),
  }));

  const t1 = ases.filter((a) => a.tier === "tier1").length;
  const t2 = ases.filter((a) => a.tier === "tier2").length;
  return {
    runId,
    seed,
    summary: {
      nodes: routers.length + totalHosts + 768,
      routers: routers.length,
      hosts: totalHosts,
      servers: 768,
      links: links.length,
      tier1: t1,
      tier2: t2,
      stubs: ases.length - t1 - t2,
      services: services.length,
    },
    cities: cities.map(({ code, name, lat, lon, region }) => ({ code, name, lat, lon, region })),
    ases,
    routers,
    links,
    services,
  };
}

// ---------------------------------------------------------------- world state

let world;

function createWorld(seed, runId) {
  const topo = generateTopology(seed, runId);
  const nl = topo.links.length;
  const nr = topo.routers.length;
  const adj = Array.from({ length: nr }, () => []);
  for (const l of topo.links) {
    adj[l.a].push(l.id);
    adj[l.b].push(l.id);
  }
  const base = new Float64Array(nl);
  const fragility = new Float64Array(nl);
  const phase = new Float64Array(nl);
  for (const l of topo.links) {
    const backbone = topo.ases[topo.routers[l.a].as].tier === "tier1" && l.kind !== "transit";
    // Mostly quiet links with a long tail of busy ones, as on a real network.
    base[l.id] = clamp((backbone ? 0.16 : 0.08) + Math.exp(gauss() * 0.7) * (backbone ? 0.14 : 0.1), 0.02, 0.86);
    fragility[l.id] = Math.exp(gauss() * 0.9);
    phase[l.id] = rand() * Math.PI * 2;
  }
  rand = mulberry32(seed ^ 0x5bd1e995);

  return {
    topo,
    adj,
    t: 0,
    seq: 0,
    paused: false,
    speed: 1,
    routing: "ospf",
    chaosMonkey: false,
    chaosPerMinute: 6,
    version: 0,
    spfCache: new Map(),
    base,
    fragility,
    phase,
    util: new Float64Array(nl),
    utilEwma: new Float64Array(nl),
    flaps: new Float64Array(nl),
    queue: new Float64Array(nl),
    loss: new Float64Array(nl),
    risk: new Float64Array(nl),
    linkDown: new Uint8Array(nl),
    detectAt: new Float64Array(nl),
    autoRestoreAt: new Float64Array(nl),
    spikeUntil: new Float64Array(nl),
    lossUntil: new Float64Array(nl),
    lossPct: new Float64Array(nl),
    routerDown: new Uint8Array(nr),
    flows: [],
    nextFlowId: 1,
    attacks: [],
    crowds: [],
    pendingEvents: [],
    incidents: [],
    nextIncidentId: 1,
    history: { t: [], deliveredGbps: [], offeredGbps: [], lossPct: [], meanMs: [], p99Ms: [], activeFlows: [], linksDown: [] },
    lastHistoryT: -1,
    schedule: [],
    stats: null,
    ai: { auc: 0.79, baselineAuc: 0.61, precisionAt10: 0.4, labelled: 0, positives: 0, td: 0.8 },
    nextBackgroundFailure: 25 + rand() * 20,
  };
}

const R = () => world.topo.routers;
const L = () => world.topo.links;
const asOf = (rid) => world.topo.ases[world.topo.routers[rid].as];

function emit(level, kind, text) {
  world.pendingEvents.push({ t: +world.t.toFixed(2), level, kind, text });
}

function linkName(id) {
  const l = L()[id];
  return `${R()[l.a].name} <-> ${R()[l.b].name}`;
}

function linkUsable(id) {
  // Forwarding keeps using a dead link until the failure is detected.
  const l = L()[id];
  if (world.routerDown[l.a] || world.routerDown[l.b]) return false;
  return world.linkDown[id] === 0 || world.t < world.detectAt[id];
}

function linkPhysicallyUp(id) {
  const l = L()[id];
  return world.linkDown[id] === 0 && !world.routerDown[l.a] && !world.routerDown[l.b];
}

function bumpVersion() {
  world.version++;
  world.spfCache.clear();
}

function spf(src) {
  const cached = world.spfCache.get(src);
  if (cached) return cached;
  const n = R().length;
  const dist = new Float64Array(n).fill(Infinity);
  const via = new Int32Array(n).fill(-1);
  const heap = new MinHeap();
  dist[src] = 0;
  heap.push(0, src);
  const q = world.routing === "qrouting";
  const dv = world.routing === "dv";
  while (heap.size) {
    const u = heap.pop();
    for (const lid of world.adj[u]) {
      if (!linkUsable(lid)) continue;
      const l = L()[lid];
      const v = l.a === u ? l.b : l.a;
      if (world.routerDown[v]) continue;
      let w = l.delayMs + 0.4;
      if (l.kind !== "internal") w += 6; // policy: prefer staying on-net
      if (dv) w = 1 + l.delayMs * 0.05; // hop count, roughly
      if (q) w += (world.queue[lid] / 10) * 1.5 + Math.max(0, world.util[lid] - 0.8) * 40;
      const nd = dist[u] + w;
      if (nd < dist[v]) {
        dist[v] = nd;
        via[v] = lid;
        heap.push(nd, v);
      }
    }
  }
  const res = { dist, via };
  world.spfCache.set(src, res);
  return res;
}

function path(src, dst) {
  if (world.routerDown[src] || world.routerDown[dst]) return null;
  const { dist, via } = spf(src);
  if (!Number.isFinite(dist[dst])) return null;
  const routers = [dst];
  const links = [];
  let cur = dst;
  while (cur !== src) {
    const lid = via[cur];
    links.push(lid);
    const l = L()[lid];
    cur = l.a === cur ? l.b : l.a;
    routers.push(cur);
  }
  return { routers: routers.reverse(), links: links.reverse() };
}

// ---------------------------------------------------------------- traffic

function accessRouters() {
  return (world._access ??= R().filter((r) => r.role === "access").map((r) => r.id));
}

function makeFlow(kind, src, dst, life, gbps) {
  const p = path(src, dst);
  if (!p || p.links.length === 0) return null;
  return {
    id: world.nextFlowId++,
    kind,
    src,
    dst,
    path: p.routers,
    links: p.links,
    born: world.t,
    until: world.t + life,
    gbps,
    version: world.version,
  };
}

function spawnBackground() {
  const services = world.topo.services;
  const acc = accessRouters();
  const src = pick(acc);
  let dst;
  if (rand() < 0.72) {
    let x = rand();
    let svc = services[0];
    for (const s of services) {
      x -= s.popularity;
      if (x <= 0) {
        svc = s;
        break;
      }
    }
    dst = pick(svc.replicas);
  } else dst = pick(acc);
  return makeFlow("bg", src, dst, 3 + rand() * 9, 0.35 + rand() * 0.5);
}

function refreshFlows() {
  const w = world;
  w.flows = w.flows.filter((f) => f.until > w.t);
  for (const f of w.flows) {
    if (f.version === w.version) continue;
    const p = path(f.src, f.dst);
    f.version = w.version;
    if (p) {
      f.path = p.routers;
      f.links = p.links;
    } else {
      f.until = w.t;
    }
  }
  w.flows = w.flows.filter((f) => f.until > w.t);
  let bg = w.flows.filter((f) => f.kind === "bg").length;
  for (let guard = 0; bg < 230 && guard < 400; guard++) {
    const f = spawnBackground();
    if (f) {
      w.flows.push(f);
      bg++;
    }
  }

  for (const c of w.crowds) {
    const target = Math.round(clamp((w.t - c.t0) / c.rampSecs, 0, 1) * Math.min(110, c.users / 90));
    let n = w.flows.filter((f) => f.kind === "crowd" && f.tag === c).length;
    const acc = accessRouters().filter((r) => asOf(r).region === c.region || rand() < 0.35);
    for (let guard = 0; n < target && guard < 300; guard++) {
      const f = makeFlow("crowd", pick(acc), pick(c.replicas), 4 + rand() * 6, 0.5 + rand() * 0.4);
      if (f) {
        f.tag = c;
        w.flows.push(f);
        n++;
      }
    }
  }
  for (const a of w.attacks) {
    const target = Math.min(140, Math.round(a.bots / 22));
    let n = w.flows.filter((f) => f.kind === "attack" && f.tag === a).length;
    for (let guard = 0; n < target && guard < 300; guard++) {
      const f = makeFlow("attack", pick(accessRouters()), a.replica, 3 + rand() * 5, (a.mbps / 30) * (0.7 + rand() * 0.5));
      if (f) {
        f.tag = a;
        w.flows.push(f);
        n++;
      }
    }
  }
  w.crowds = w.crowds.filter((c) => c.until > w.t);
  w.attacks = w.attacks.filter((a) => a.until > w.t);
  for (const f of w.flows) if (f.tag && f.tag.until <= w.t) f.until = Math.min(f.until, w.t + 1);
}

// ---------------------------------------------------------------- failures

// When each incident's IGP and BGP will have converged; kept off the
// incident object so it never leaks into frames.
const convergeAt = new WeakMap();

function openIncident(cause, interAs) {
  const w = world;
  const igpBase = w.routing === "dv" ? 1800 + rand() * 4200 : w.routing === "qrouting" ? 260 + rand() * 500 : 140 + rand() * 260;
  const inc = {
    id: w.nextIncidentId++,
    cause,
    t0: +w.t.toFixed(2),
    open: true,
    igpMs: 0,
    bgpMs: 0,
    peakLossPct: 0,
    blackholedFlowSecs: 0,
  };
  convergeAt.set(inc, { igp: igpBase, bgp: interAs ? 900 + rand() * 3200 : 0 });
  w.incidents.unshift(inc);
  w.incidents = w.incidents.slice(0, 8);
  return inc;
}

function failLink(id, cause = "fibre cut", { auto = false, incident = true } = {}) {
  const w = world;
  if (w.linkDown[id]) return false;
  w.linkDown[id] = 1;
  w.detectAt[id] = w.t + 0.6 + rand() * 1.4;
  w.autoRestoreAt[id] = auto ? w.t + 18 + rand() * 25 : 0;
  w.flaps[id] += 1;
  w.ai.positives += 1;
  emit("crit", "link_down", `${linkName(id)} down (${cause})`);
  if (incident) openIncident(`link ${id} down`, L()[id].kind !== "internal");
  return true;
}

function restoreLink(id, quiet = false) {
  const w = world;
  if (!w.linkDown[id]) return false;
  w.linkDown[id] = 0;
  w.autoRestoreAt[id] = 0;
  bumpVersion();
  if (!quiet) emit("info", "link_up", `${linkName(id)} restored`);
  return true;
}

function failRouter(id, cause = "crash") {
  const w = world;
  if (w.routerDown[id]) return false;
  w.routerDown[id] = 1;
  for (const lid of w.adj[id]) {
    w.detectAt[lid] = w.t + 0.8 + rand();
  }
  bumpVersion();
  emit("crit", "router_down", `${R()[id].name} ${cause}`);
  return true;
}

function restoreRouter(id) {
  const w = world;
  if (!w.routerDown[id]) return false;
  w.routerDown[id] = 0;
  bumpVersion();
  emit("info", "router_up", `${R()[id].name} back up, adjacencies re-forming`);
  return true;
}

function processControlPlane() {
  const w = world;
  for (let i = 0; i < L().length; i++) {
    if (w.linkDown[i] && w.detectAt[i] > 0 && w.t >= w.detectAt[i]) {
      w.detectAt[i] = -1;
      bumpVersion();
      emit("warn", "adjacency_lost", `${R()[L()[i].a].name.split(".")[0]} lost adjacency to ${R()[L()[i].b].name.split(".")[0]}, SPF scheduled`);
    }
    if (w.autoRestoreAt[i] > 0 && w.t >= w.autoRestoreAt[i]) restoreLink(i);
    if (w.spikeUntil[i] > 0 && w.t >= w.spikeUntil[i]) {
      w.spikeUntil[i] = 0;
      emit("info", "fault_cleared", `latency spike on L${i} cleared`);
    }
    if (w.lossUntil[i] > 0 && w.t >= w.lossUntil[i]) {
      w.lossUntil[i] = 0;
      w.lossPct[i] = 0;
      emit("info", "fault_cleared", `loss injection on L${i} cleared`);
    }
  }
  for (const inc of w.incidents) {
    if (!inc.open) continue;
    const age = (w.t - inc.t0) * 1000;
    const target = convergeAt.get(inc);
    if (inc.igpMs === 0 && age >= target.igp + 600) {
      inc.igpMs = +target.igp.toFixed(1);
      emit("info", "igp_converged", `IGP converged in ${Math.round(inc.igpMs)} ms (${w.routing})`);
    }
    if (target.bgp > 0 && inc.bgpMs === 0 && age >= target.bgp + 600) {
      inc.bgpMs = +target.bgp.toFixed(1);
      emit("info", "bgp_converged", `BGP converged in ${(inc.bgpMs / 1000).toFixed(2)} s, ${randInt(3, 40)} updates`);
    }
    if (age >= Math.max(target.igp, target.bgp) + 2600) {
      inc.open = false;
      if (inc.bgpMs === 0 && target.bgp > 0) inc.bgpMs = +target.bgp.toFixed(1);
      if (inc.igpMs === 0) inc.igpMs = +target.igp.toFixed(1);
    }
  }
}

function chaosStep(dt) {
  const w = world;
  const rate = w.chaosMonkey ? w.chaosPerMinute / 60 : 0;
  const fire = w.chaosMonkey ? rand() < rate * dt : w.t >= w.nextBackgroundFailure;
  if (!fire) return;
  if (!w.chaosMonkey) w.nextBackgroundFailure = w.t + 35 + rand() * 40;
  // Weighted by hidden fragility and load: the predictor gets to be right.
  let total = 0;
  const weights = new Float64Array(L().length);
  for (let i = 0; i < L().length; i++) {
    if (!linkPhysicallyUp(i)) continue;
    weights[i] = w.fragility[i] * (0.2 + w.risk[i] / 200);
    total += weights[i];
  }
  let x = rand() * total;
  for (let i = 0; i < weights.length; i++) {
    x -= weights[i];
    if (x <= 0) {
      failLink(i, w.chaosMonkey ? "chaos monkey" : pick(["fibre cut", "optics failure", "line card reset"]), { auto: true });
      return;
    }
  }
}

// Background control-plane chatter, so the event log reads like a network
// that is alive rather than one that only speaks during outages.
function chatter(dt) {
  const w = world;
  if (rand() > dt * 0.9) return;
  const r = pick(R());
  const as = asOf(r.id);
  const lid = pick(w.adj[r.id] ?? [0]);
  const busiest = w.util.reduce((best, u, i) => (u > w.util[best] ? i : best), 0);
  const templates = [
    () => ["info", "spf", `SPF on ${r.name} (${randInt(3, 28)} ms, ${randInt(40, 260)} nodes)`],
    () => ["info", "lsa", `LSA seq 0x${randInt(4096, 65535).toString(16)} flooded from ${r.name}`],
    () => ["info", "bgp_update", `AS${as.asn} sent ${randInt(1, 12)} updates (MRAI flush)`],
    () => ["info", "fib", `${r.name} programmed ${randInt(2, 40)} FIB entries`],
    () => ["info", "predictor", `model refit: ${randInt(800, 2400)} new labels, AUC ${w.ai.auc.toFixed(3)}`],
    () =>
      w.util[busiest] > 0.85
        ? ["warn", "congestion", `L${busiest} at ${Math.round(w.util[busiest] * 100)}% (${linkName(busiest)})`]
        : ["info", "telemetry", `peak link util ${Math.round(w.util[busiest] * 100)}% on L${busiest}`],
    () => (w.queue[lid] > 50 ? ["warn", "queue", `L${lid} queue ${(w.queue[lid] / 10).toFixed(1)} ms`] : ["info", "bfd", `BFD session up ${r.name} L${lid}`]),
  ];
  const [level, kind, text] = pick(templates)();
  emit(level, kind, text);
}

// ---------------------------------------------------------------- tick

function step(dt) {
  const w = world;
  w.t += dt;
  processControlPlane();
  chaosStep(dt);
  chatter(dt);
  for (const s of w.schedule.filter((s) => s.at <= w.t)) s.run();
  w.schedule = w.schedule.filter((s) => s.at > w.t);
  refreshFlows();

  const nl = L().length;
  const flowLoad = new Float64Array(nl);
  for (const f of w.flows) for (const lid of f.links) flowLoad[lid] += f.gbps;

  const diurnal = 0.88 + 0.12 * Math.sin(w.t / 90);
  let congested = 0;
  let maxUtil = 0;
  let downCount = 0;
  for (let i = 0; i < nl; i++) {
    const l = L()[i];
    if (!linkPhysicallyUp(i)) {
      w.util[i] = 0;
      w.queue[i] = 0;
      w.loss[i] = 0;
      if (w.linkDown[i]) downCount++;
      continue;
    }
    const wobble = 0.82 + 0.18 * Math.sin(w.t / (8 + (i % 13)) + w.phase[i]) + gauss() * 0.02;
    let u = w.base[i] * wobble * diurnal + flowLoad[i] / l.capacityGbps * 2.2;
    if (w.routing === "qrouting") u *= 0.96;
    u = clamp(u, 0, 1.3);
    w.util[i] = w.util[i] * 0.5 + u * 0.5;
    w.utilEwma[i] = w.utilEwma[i] * 0.97 + w.util[i] * 0.03;
    const over = Math.max(0, w.util[i] - 0.88);
    w.queue[i] = over > 0 ? Math.min(900, over * 1600 + rand() * 30) : rand() < 0.3 ? rand() * 6 : 0;
    if (w.spikeUntil[i] > w.t) w.queue[i] += 800;
    w.loss[i] = Math.max(0, w.util[i] - 1) * 350 + (w.lossUntil[i] > w.t ? w.lossPct[i] * 10 : 0);
    if (w.util[i] > 0.9) congested++;
    maxUtil = Math.max(maxUtil, w.util[i]);
  }

  for (let i = 0; i < nl; i++) {
    w.flaps[i] *= Math.exp(-dt / 60);
    if (!linkPhysicallyUp(i)) {
      w.risk[i] = 0;
      continue;
    }
    const z = -4.6 + 1.1 * Math.log(w.fragility[i]) + 9 * Math.max(0, w.utilEwma[i] - 0.6) + 5.5 * Math.max(0, w.util[i] - 0.85) + 1.2 * w.flaps[i];
    const target = 1000 / (1 + Math.exp(-z));
    w.risk[i] = w.risk[i] * 0.9 + target * 0.1;
  }

  // Aggregate stats. Displayed flows are a sample; scale them up.
  const bgOffered = 128 * diurnal + 6 * Math.sin(w.t / 23);
  const crowdOffered = w.flows.filter((f) => f.kind === "crowd").reduce((s, f) => s + f.gbps, 0) * 0.6;
  const attackOffered = w.flows.filter((f) => f.kind === "attack").reduce((s, f) => s + f.gbps, 0) * 0.5;
  const offered = bgOffered + crowdOffered + attackOffered;

  let lostWeight = 0;
  let blackholed = 0;
  const lat = [];
  for (const f of w.flows) {
    let ms = 0;
    let survive = 1;
    let dead = false;
    for (const lid of f.links) {
      if (!linkPhysicallyUp(lid)) dead = true;
      ms += L()[lid].delayMs + w.queue[lid] / 10 + (w.spikeUntil[lid] > w.t ? 80 : 0);
      survive *= 1 - w.loss[lid] / 1000;
    }
    f.latencyMs = ms * 1.15 + 6 + rand() * 4;
    f.lossPct = dead ? 100 : (1 - survive) * 100;
    if (dead) blackholed++;
    lostWeight += (f.lossPct / 100) * f.gbps;
    if (!dead) lat.push(f.latencyMs);
  }
  const flowGbps = w.flows.reduce((s, f) => s + f.gbps, 0) || 1;
  const lossFrac = clamp(lostWeight / flowGbps, 0, 0.9);
  lat.sort((a, b) => a - b);
  const q = (p) => (lat.length ? lat[Math.min(lat.length - 1, Math.floor(p * lat.length))] : 0);
  const mean = lat.length ? lat.reduce((a, b) => a + b, 0) / lat.length : 0;
  const active = Math.round(30400 * diurnal + (crowdOffered + attackOffered) * 40 + gauss() * 120);
  const blackholedReal = Math.round((blackholed / Math.max(1, w.flows.length)) * active);

  for (const inc of w.incidents) {
    if (!inc.open) continue;
    inc.peakLossPct = +Math.max(inc.peakLossPct, lossFrac * 100).toFixed(2);
    inc.blackholedFlowSecs = +(inc.blackholedFlowSecs + blackholedReal * dt * 0.01).toFixed(1);
  }

  w.stats = {
    offeredGbps: +offered.toFixed(1),
    deliveredGbps: +(offered * (1 - lossFrac)).toFixed(1),
    lossPct: +(lossFrac * 100).toFixed(2),
    latencyMs: { mean: +mean.toFixed(1), p50: +q(0.5).toFixed(1), p95: +q(0.95).toFixed(1), p99: +q(0.99).toFixed(1) },
    flows: {
      active,
      unreachable: 0,
      looping: w.routing === "dv" && w.incidents.some((i) => i.open) ? randInt(0, 40) : 0,
      blackholed: blackholedReal,
      crowd: Math.round(crowdOffered * 90),
      attack: Math.round(attackOffered * 45),
    },
    linksDown: downCount,
    routersDown: w.routerDown.reduce((a, b) => a + b, 0),
    congestedLinks: congested,
    maxUtil: +Math.min(1.45, maxUtil).toFixed(3),
    eventsPerSec: Math.round(17600 + 2400 * Math.sin(w.t / 11) + gauss() * 300 + (w.incidents.some((i) => i.open) ? 9000 : 0)),
  };

  const ai = w.ai;
  ai.labelled += Math.round(nl * dt * 0.2);
  ai.auc = clamp(ai.auc + (0.84 - ai.auc) * 0.002 + gauss() * 0.0015, 0.7, 0.93);
  ai.baselineAuc = clamp(ai.baselineAuc + (0.615 - ai.baselineAuc) * 0.01 + gauss() * 0.001, 0.55, 0.68);
  ai.precisionAt10 = clamp(ai.precisionAt10 + gauss() * 0.01 + (0.45 - ai.precisionAt10) * 0.01, 0.1, 0.8);
  const openRecent = w.incidents.some((i) => i.open);
  ai.td = ai.td * 0.95 + (w.routing === "qrouting" && openRecent ? 6 : 0.8) * 0.05;

  // qrouting reacts to load, so its view of the graph keeps changing.
  if (w.routing === "qrouting" && Math.floor(w.t / 2) !== Math.floor((w.t - dt) / 2)) bumpVersion();

  if (Math.floor(w.t) > w.lastHistoryT) {
    w.lastHistoryT = Math.floor(w.t);
    const h = w.history;
    h.t.push(+w.t.toFixed(1));
    h.deliveredGbps.push(w.stats.deliveredGbps);
    h.offeredGbps.push(w.stats.offeredGbps);
    h.lossPct.push(w.stats.lossPct);
    h.meanMs.push(w.stats.latencyMs.mean);
    h.p99Ms.push(w.stats.latencyMs.p99);
    h.activeFlows.push(w.stats.flows.active);
    h.linksDown.push(w.stats.linksDown);
    if (h.t.length > 600) for (const k of Object.keys(h)) h[k].shift();
  }
}

function buildFrame() {
  const w = world;
  const nl = L().length;
  const util = new Array(nl);
  const queueMs = new Array(nl);
  const loss = new Array(nl);
  const state = new Array(nl);
  const risk = new Array(nl);
  for (let i = 0; i < nl; i++) {
    util[i] = Math.round(w.util[i] * 1000);
    queueMs[i] = Math.round(w.queue[i]);
    loss[i] = Math.round(w.loss[i]);
    const l = L()[i];
    const routerDead = w.routerDown[l.a] || w.routerDown[l.b];
    if (w.linkDown[i] || routerDead) state[i] = w.detectAt[i] > 0 && w.t < w.detectAt[i] ? 3 : 1;
    else state[i] = w.spikeUntil[i] > w.t || w.lossUntil[i] > w.t ? 2 : 0;
    risk[i] = state[i] === 1 || state[i] === 3 ? 0 : Math.round(w.risk[i]);
  }
  const top = [];
  for (let i = 0; i < nl; i++) if (risk[i] > 0) top.push({ link: i, risk: +(risk[i] / 1000).toFixed(3) });
  top.sort((a, b) => b.risk - a.risk);

  const events = w.pendingEvents;
  w.pendingEvents = [];
  return {
    type: "frame",
    runId: w.topo.runId,
    seq: ++w.seq,
    t: +w.t.toFixed(2),
    paused: w.paused,
    speed: w.speed,
    simRate: w.paused ? 0 : +(w.speed * (0.97 + rand() * 0.03)).toFixed(3),
    routing: w.routing,
    chaosMonkey: w.chaosMonkey,
    stats: w.stats,
    links: { util, queueMs, loss, state, risk },
    routersDown: [...w.routerDown.keys()].filter((i) => w.routerDown[i]),
    flows: w.flows.map((f) => ({
      id: f.id,
      kind: f.kind,
      path: f.path,
      latencyMs: +f.latencyMs.toFixed(1),
      lossPct: +f.lossPct.toFixed(2),
      mbps: +(f.gbps * 10).toFixed(1),
    })),
    events,
    incidents: w.incidents,
    ai: {
      predictor: {
        auc: +w.ai.auc.toFixed(3),
        baselineAuc: +w.ai.baselineAuc.toFixed(3),
        precisionAt10: +(Math.round(w.ai.precisionAt10 * 10) / 10).toFixed(1),
        labelled: w.ai.labelled,
        positives: w.ai.positives,
        top: top.slice(0, 10),
      },
      qrouting: { updatesPerSec: Math.round(2.1e6 + gauss() * 4e4), meanAbsTdMs: +w.ai.td.toFixed(2) },
    },
  };
}

// ---------------------------------------------------------------- commands

const SCENARIOS = [
  {
    name: "transatlantic-cut",
    title: "Cut the Atlantic",
    description: "Every fibre between North America and Europe fails within two seconds. Watch traffic swing via Asia and the control plane chase it.",
    durationSecs: 90,
    run() {
      const between = L().filter((l) => {
        const ra = cityByCode.get(R()[l.a].city).region;
        const rb = cityByCode.get(R()[l.b].city).region;
        return (ra === "NA" && rb === "EU") || (ra === "EU" && rb === "NA");
      });
      between.forEach((l, i) =>
        world.schedule.push({ at: world.t + i * 0.3, run: () => failLink(l.id, "subsea cable cut", { incident: i === 0 }) }),
      );
      world.schedule.push({ at: world.t + 60, run: () => between.forEach((l) => restoreLink(l.id, true)) });
      emit("warn", "scenario", `scenario transatlantic-cut: ${between.length} links scheduled to fail`);
    },
  },
  {
    name: "frankfurt-dark",
    title: "Frankfurt goes dark",
    description: "Power loss at the largest European interconnect: every router in Frankfurt crashes, then recovers a minute later.",
    durationSecs: 75,
    run() {
      const rs = R().filter((r) => r.city === "fra");
      rs.forEach((r) => failRouter(r.id, "power loss"));
      openIncident("fra power loss", true);
      world.schedule.push({ at: world.t + 60, run: () => rs.forEach((r) => restoreRouter(r.id)) });
    },
  },
  {
    name: "launch-day",
    title: "Launch day",
    description: "Ten thousand users from every region pile onto vidstream over five seconds. Find the first link to melt.",
    durationSecs: 60,
    run() {
      startCrowd(0, 10000, 5, 60);
    },
  },
  {
    name: "botnet-payments",
    title: "Botnet vs payments",
    description: "A 3,000-bot volumetric attack on the payments service. Count the collateral damage on links nobody attacked.",
    durationSecs: 40,
    run() {
      startDdos(4, 3000, 30, 40);
    },
  },
  {
    name: "learning-vs-spf",
    title: "Learning vs shortest path",
    description: "Switch to Q-routing during a flash crowd and compare p99 against the OSPF run in the history chart.",
    durationSecs: 80,
    run() {
      startCrowd(1, 8000, 5, 70);
      world.schedule.push({ at: world.t + 25, run: () => setRouting("qrouting") });
    },
  },
  {
    name: "chaos-hour",
    title: "Chaos hour",
    description: "Chaos monkey at 12 failures a minute, weighted by fragility and load. The predictor's top-10 should light up first.",
    durationSecs: 60,
    run() {
      world.chaosMonkey = true;
      world.chaosPerMinute = 12;
      emit("warn", "chaos_monkey", "chaos monkey enabled at 12/min");
      world.schedule.push({
        at: world.t + 60,
        run: () => {
          world.chaosMonkey = false;
          world.chaosPerMinute = 6;
          emit("info", "chaos_monkey", "chaos monkey disabled");
        },
      });
    },
  },
];

function startCrowd(service, users, rampSecs, secs) {
  const svc = world.topo.services[service];
  if (!svc) throw new Error(`no service ${service}`);
  world.crowds.push({ t0: world.t, until: world.t + secs, rampSecs: Math.max(0.1, rampSecs), users, replicas: svc.replicas, region: pick(REGIONS) });
  emit("warn", "flash_crowd", `flash crowd: ${users.toLocaleString("en-US")} users → ${svc.name}`);
}

function startDdos(service, bots, mbps, secs) {
  const svc = world.topo.services[service];
  if (!svc) throw new Error(`no service ${service}`);
  world.attacks.push({ until: world.t + secs, bots, mbps, replica: svc.replicas[0] });
  emit("crit", "ddos", `DDoS: ${bots.toLocaleString("en-US")} bots → ${svc.name} (${R()[svc.replicas[0]].name})`);
}

function setRouting(mode) {
  if (!["ospf", "dv", "qrouting"].includes(mode)) throw new Error(`unknown routing mode ${mode}`);
  if (world.routing === mode) return;
  world.routing = mode;
  bumpVersion();
  emit("info", "routing", `intra-AS routing switched to ${mode}`);
}

function handleCommand(c) {
  const w = world;
  const need = (cond, msg) => {
    if (!cond) throw new Error(msg);
  };
  const linkOk = (id) => need(Number.isInteger(id) && id >= 0 && id < L().length, `bad link ${id}`);
  const routerOk = (id) => need(Number.isInteger(id) && id >= 0 && id < R().length, `bad router ${id}`);
  switch (c.cmd) {
    case "fail_link":
      linkOk(c.link);
      need(failLink(c.link, "operator"), "link already down");
      break;
    case "restore_link":
      linkOk(c.link);
      need(restoreLink(c.link), "link is up");
      break;
    case "fail_router":
      routerOk(c.router);
      need(failRouter(c.router, "crashed (operator)"), "router already down");
      openIncident(`router ${c.router} down`, R()[c.router].role === "border");
      break;
    case "restore_router":
      routerOk(c.router);
      need(restoreRouter(c.router), "router is up");
      break;
    case "fail_as": {
      const as = w.topo.ases[c.as];
      need(as, `bad as ${c.as}`);
      for (const r of R()) if (r.as === as.id) failRouter(r.id, `down (AS${as.asn} outage)`);
      openIncident(`AS${as.asn} outage`, true);
      break;
    }
    case "restore_as": {
      const as = w.topo.ases[c.as];
      need(as, `bad as ${c.as}`);
      for (const r of R()) if (r.as === as.id) restoreRouter(r.id);
      break;
    }
    case "latency_spike":
      linkOk(c.link);
      w.spikeUntil[c.link] = w.t + (c.secs ?? 20);
      emit("warn", "latency_spike", `L${c.link} +${c.ms ?? 80} ms for ${c.secs ?? 20} s`);
      break;
    case "packet_loss":
      linkOk(c.link);
      w.lossUntil[c.link] = w.t + (c.secs ?? 20);
      w.lossPct[c.link] = c.pct ?? 5;
      emit("warn", "packet_loss", `L${c.link} dropping ${c.pct ?? 5}% for ${c.secs ?? 20} s`);
      break;
    case "ddos":
      startDdos(c.service, c.bots ?? 3000, c.mbps ?? 30, c.secs ?? 40);
      break;
    case "flash_crowd":
      startCrowd(c.service, c.users ?? 10000, c.rampSecs ?? 5, c.secs ?? 60);
      break;
    case "chaos_monkey":
      w.chaosMonkey = !!c.enabled;
      w.chaosPerMinute = c.perMinute ?? 6;
      emit("warn", "chaos_monkey", w.chaosMonkey ? `chaos monkey enabled at ${w.chaosPerMinute}/min` : "chaos monkey disabled");
      break;
    case "set_routing":
      setRouting(c.mode);
      break;
    case "set_speed":
      need(typeof c.speed === "number" && c.speed > 0 && c.speed <= 16, "speed out of range");
      w.speed = c.speed;
      break;
    case "pause":
      w.paused = true;
      break;
    case "resume":
      w.paused = false;
      break;
    case "heal_all":
      for (let i = 0; i < L().length; i++) {
        restoreLink(i, true);
        w.spikeUntil[i] = 0;
        w.lossUntil[i] = 0;
      }
      for (let i = 0; i < R().length; i++) restoreRouter(i);
      w.attacks = [];
      w.crowds = [];
      emit("info", "heal_all", "all injected faults cleared");
      break;
    case "reset":
      resetWorld(Number.isFinite(c.seed) ? c.seed : Math.floor(Math.random() * 1e8), w.topo.runId + 1);
      break;
    case "run_scenario": {
      const sc = SCENARIOS.find((s) => s.name === c.name);
      need(sc, `unknown scenario ${c.name}`);
      emit("info", "scenario", `running scenario "${sc.title}"`);
      sc.run();
      break;
    }
    default:
      throw new Error(`unknown command ${c.cmd}`);
  }
}

function trace(src, dst) {
  const p = path(src, dst);
  if (!p) {
    const r = R()[src];
    return {
      status: world.routerDown[src] ? "blackhole" : "unreachable",
      hops: r ? [{ router: src, name: r.name, as: r.as, asn: asOf(src).asn, link: null, cumMs: 0, queueMs: 0 }] : [],
      asPath: r ? [asOf(src).asn] : [],
    };
  }
  let cum = 0;
  let dead = false;
  const hops = p.routers.map((rid, i) => {
    const lid = p.links[i];
    const q = lid !== undefined ? world.queue[lid] / 10 : 0;
    const hop = { router: rid, name: R()[rid].name, as: R()[rid].as, asn: asOf(rid).asn, link: lid ?? null, cumMs: +cum.toFixed(2), queueMs: +q.toFixed(1) };
    if (lid !== undefined) {
      cum += L()[lid].delayMs + q + 0.05;
      if (!linkPhysicallyUp(lid)) dead = true;
    }
    return hop;
  });
  const asPath = [];
  for (const h of hops) if (asPath[asPath.length - 1] !== h.asn) asPath.push(h.asn);
  return { status: dead ? "blackhole" : "ok", hops, asPath };
}

function routerDetail(id) {
  const r = R()[id];
  if (!r) return null;
  const neighbors = world.adj[id].map((lid) => {
    const l = L()[lid];
    return { router: l.a === id ? l.b : l.a, link: lid, up: linkPhysicallyUp(lid), util: +world.util[lid].toFixed(3) };
  });
  const fib = [];
  const { dist, via } = spf(id);
  const firstHop = (dst) => {
    let cur = dst;
    let lid = -1;
    while (cur !== id && via[cur] >= 0) {
      lid = via[cur];
      const l = L()[lid];
      cur = l.a === cur ? l.b : l.a;
    }
    return lid;
  };
  const sameAs = R().filter((x) => x.as === r.as && x.id !== id).slice(0, 8);
  for (const x of sameAs) {
    const lid = firstHop(x.id);
    if (lid < 0 || !Number.isFinite(dist[x.id])) continue;
    const l = L()[lid];
    fib.push({ target: x.name, link: lid, nextHop: l.a === id ? l.b : l.a, cost: Math.round(dist[x.id] * 10) });
  }
  const seen = new Set([r.as]);
  for (const x of R()) {
    if (seen.has(x.as) || fib.length > 30) continue;
    if (!Number.isFinite(dist[x.id])) continue;
    seen.add(x.as);
    const lid = firstHop(x.id);
    if (lid < 0) continue;
    const l = L()[lid];
    fib.push({ target: `exit AS${world.topo.ases[x.as].asn}`, link: lid, nextHop: l.a === id ? l.b : l.a, cost: Math.round(dist[x.id] * 10) });
  }
  return { id, name: r.name, as: r.as, role: r.role, down: !!world.routerDown[id], hosts: r.hosts, neighbors, fib };
}

// ---------------------------------------------------------------- lifecycle

function resetWorld(seed, runId) {
  world = createWorld(seed, runId);
  // Fast-forward so the dashboard opens on a world with history and a
  // couple of past incidents instead of a blank chart.
  world.schedule.push({ at: 31, run: () => failLink(pickLongHaul(), "fibre cut", { auto: true }) });
  world.schedule.push({ at: 74, run: () => failLink(pickLongHaul(), "optics failure", { auto: true }) });
  for (let i = 0; i < 600; i++) step(0.2);
  world.pendingEvents = world.pendingEvents.slice(-30);
  for (const c of clients) send(c, JSON.stringify({ type: "hello", version: 1, runId }));
  console.log(`world: run ${runId} seed ${seed} · ${R().length} routers · ${L().length} links · ${world.topo.ases.length} ASes`);
}

function pickLongHaul() {
  const cands = L().filter((l) => l.km > 2500 && linkPhysicallyUp(l.id));
  return pick(cands.length ? cands : L()).id;
}

// ---------------------------------------------------------------- websocket

const clients = new Set();

function encodeFrame(text) {
  const payload = Buffer.from(text);
  const n = payload.length;
  let header;
  if (n < 126) header = Buffer.from([0x81, n]);
  else if (n < 65536) {
    header = Buffer.alloc(4);
    header[0] = 0x81;
    header[1] = 126;
    header.writeUInt16BE(n, 2);
  } else {
    header = Buffer.alloc(10);
    header[0] = 0x81;
    header[1] = 127;
    header.writeBigUInt64BE(BigInt(n), 2);
  }
  return Buffer.concat([header, payload]);
}

function send(socket, text) {
  if (!socket.destroyed) socket.write(encodeFrame(text));
}

function acceptWebSocket(req, socket) {
  const key = req.headers["sec-websocket-key"];
  if (!key) return socket.destroy();
  const accept = createHash("sha1").update(key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").digest("base64");
  socket.write(
    "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n" +
      `Sec-WebSocket-Accept: ${accept}\r\n\r\n`,
  );
  socket.setNoDelay(true);
  clients.add(socket);
  send(socket, JSON.stringify({ type: "hello", version: 1, runId: world.topo.runId }));
  socket.on("data", (buf) => {
    // Clients never send data in this protocol; only honour close and ping.
    const opcode = buf[0] & 0x0f;
    if (opcode === 0x8) socket.end(Buffer.from([0x88, 0]));
    else if (opcode === 0x9) socket.write(Buffer.from([0x8a, 0]));
  });
  const drop = () => clients.delete(socket);
  socket.on("close", drop);
  socket.on("error", drop);
}

// ---------------------------------------------------------------- http

const CORS = {
  "Access-Control-Allow-Origin": "*",
  "Access-Control-Allow-Methods": "GET, POST, OPTIONS",
  "Access-Control-Allow-Headers": "Content-Type",
};

function json(res, status, body) {
  res.writeHead(status, { "Content-Type": "application/json", "Cache-Control": "no-store", ...CORS });
  res.end(JSON.stringify(body));
}

const MIME = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript",
  ".css": "text/css",
  ".json": "application/json",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".ico": "image/x-icon",
  ".woff2": "font/woff2",
  ".txt": "text/plain",
};

function serveStatic(pathname, res) {
  if (!existsSync(OUT_DIR)) return false;
  const rel = normalize(decodeURIComponent(pathname)).replace(/^(\.\.[/\\])+/, "");
  let file = join(OUT_DIR, rel);
  if (!file.startsWith(OUT_DIR)) return false;
  if (existsSync(file) && statSync(file).isDirectory()) file = join(file, "index.html");
  if (!existsSync(file) && existsSync(`${file}.html`)) file = `${file}.html`;
  if (!existsSync(file)) return false;
  res.writeHead(200, { "Content-Type": MIME[extname(file)] ?? "application/octet-stream" });
  res.end(readFileSync(file));
  return true;
}

const server = http.createServer((req, res) => {
  const url = new URL(req.url, "http://localhost");
  if (req.method === "OPTIONS") {
    res.writeHead(204, CORS);
    return res.end();
  }
  const p = url.pathname;
  if (req.method === "GET" && p === "/api/topology") return json(res, 200, world.topo);
  if (req.method === "GET" && p === "/api/history") return json(res, 200, world.history);
  if (req.method === "GET" && p === "/api/scenarios") {
    return json(res, 200, SCENARIOS.map(({ name, title, description, durationSecs }) => ({ name, title, description, durationSecs })));
  }
  if (req.method === "GET" && p === "/api/trace") {
    const src = Number(url.searchParams.get("src"));
    const dst = Number(url.searchParams.get("dst"));
    if (!R()[src] || !R()[dst]) return json(res, 400, { error: "bad src/dst" });
    return json(res, 200, trace(src, dst));
  }
  const m = p.match(/^\/api\/router\/(\d+)$/);
  if (req.method === "GET" && m) {
    const d = routerDetail(Number(m[1]));
    return d ? json(res, 200, d) : json(res, 404, { error: "no such router" });
  }
  if (req.method === "POST" && p === "/api/command") {
    let body = "";
    req.on("data", (c) => (body += c));
    req.on("end", () => {
      try {
        handleCommand(JSON.parse(body));
        json(res, 200, { ok: true });
      } catch (err) {
        json(res, 200, { ok: false, error: err.message });
      }
    });
    return;
  }
  if (req.method === "GET" && serveStatic(p, res)) return;
  json(res, 404, { error: "not found" });
});

server.on("upgrade", (req, socket) => {
  if (new URL(req.url, "http://localhost").pathname !== "/ws") return socket.destroy();
  acceptWebSocket(req, socket);
});

resetWorld(INITIAL_SEED, 1);

let last = performance.now();
setInterval(() => {
  const now = performance.now();
  const wall = Math.min(1, (now - last) / 1000);
  last = now;
  if (!world.paused) step(wall * world.speed);
  const text = JSON.stringify(buildFrame());
  for (const c of clients) send(c, text);
}, FRAME_MS);

server.listen(PORT, "127.0.0.1", () => {
  console.log(`mock shadownetd on http://127.0.0.1:${PORT}${existsSync(OUT_DIR) ? " (serving dashboard/out)" : ""}`);
});
