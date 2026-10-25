// Types for the shadownetd wire protocol. Mirrors docs/PROTOCOL.md; when the
// two disagree, PROTOCOL.md is the reference and this file should be fixed.

export type Tier = "tier1" | "tier2" | "stub";
export type RouterRole = "core" | "border" | "access" | "datacenter";
export type LinkKind = "internal" | "peering" | "transit";
export type RoutingMode = "ospf" | "dv" | "qrouting";
export type FlowKind = "bg" | "crowd" | "attack";
export type EventLevel = "info" | "warn" | "crit";

export interface TopologySummary {
  nodes: number;
  routers: number;
  hosts: number;
  servers: number;
  links: number;
  tier1: number;
  tier2: number;
  stubs: number;
  services: number;
}

export interface City {
  code: string;
  name: string;
  lat: number;
  lon: number;
  region: string;
}

export interface As {
  id: number;
  asn: number;
  name: string;
  tier: Tier;
  region: string;
  home: string;
  hosting: boolean;
}

export interface Router {
  id: number;
  as: number;
  name: string;
  role: RouterRole;
  city: string;
  lat: number;
  lon: number;
  hosts: number;
}

export interface Link {
  id: number;
  a: number;
  b: number;
  kind: LinkKind;
  capacityGbps: number;
  delayMs: number;
  km: number;
}

export interface Service {
  id: number;
  name: string;
  replicas: number[];
  popularity: number;
}

export interface Topology {
  runId: number;
  seed: number;
  summary: TopologySummary;
  cities: City[];
  ases: As[];
  routers: Router[];
  links: Link[];
  services: Service[];
}

/** Link states as sent in `frame.links.state`. */
export const LinkState = {
  Up: 0,
  Down: 1,
  Degraded: 2,
  /** Physically down, but the routers on either end have not noticed yet. */
  Undetected: 3,
} as const;

export interface FrameStats {
  offeredGbps: number;
  deliveredGbps: number;
  lossPct: number;
  latencyMs: { mean: number; p50: number; p95: number; p99: number };
  flows: {
    active: number;
    unreachable: number;
    looping: number;
    blackholed: number;
    crowd: number;
    attack: number;
  };
  linksDown: number;
  routersDown: number;
  congestedLinks: number;
  maxUtil: number;
  eventsPerSec: number;
}

/** Parallel arrays indexed by link id. All integers. */
export interface FrameLinks {
  /** Permille of capacity in the busier direction; above 1000 means oversubscribed. */
  util: number[];
  /** Queueing delay in tenths of a millisecond. */
  queueMs: number[];
  /** Permille. */
  loss: number[];
  state: number[];
  /** Failure predictor output, permille. */
  risk: number[];
}

export interface FrameFlow {
  id: number;
  kind: FlowKind;
  path: number[];
  latencyMs: number;
  lossPct: number;
  mbps: number;
}

export interface SimEvent {
  t: number;
  level: EventLevel;
  kind: string;
  text: string;
}

export interface Incident {
  id: number;
  cause: string;
  t0: number;
  open: boolean;
  igpMs: number;
  bgpMs: number;
  peakLossPct: number;
  blackholedFlowSecs: number;
}

export interface AiStats {
  predictor: {
    auc: number;
    baselineAuc: number;
    precisionAt10: number;
    labelled: number;
    positives: number;
    top: { link: number; risk: number }[];
  };
  qrouting: { updatesPerSec: number; meanAbsTdMs: number };
}

export interface Frame {
  type: "frame";
  runId: number;
  seq: number;
  t: number;
  paused: boolean;
  speed: number;
  simRate: number;
  routing: RoutingMode;
  chaosMonkey: boolean;
  stats: FrameStats;
  links: FrameLinks;
  routersDown: number[];
  flows: FrameFlow[];
  events: SimEvent[];
  incidents: Incident[];
  ai: AiStats;
}

export interface Hello {
  type: "hello";
  version: number;
  runId: number;
  /** The last few dozen events, so a fresh console does not start blank. */
  recent?: SimEvent[];
}

export type ServerMessage = Hello | Frame;

export interface History {
  t: number[];
  deliveredGbps: number[];
  offeredGbps: number[];
  lossPct: number[];
  meanMs: number[];
  p99Ms: number[];
  activeFlows: number[];
  linksDown: number[];
}

export interface Scenario {
  name: string;
  title: string;
  description: string;
  durationSecs: number;
}

export type TraceStatus = "ok" | "unreachable" | "loop" | "blackhole";

export interface TraceHop {
  router: number;
  name: string;
  as: number;
  asn: number;
  /** Outgoing link towards the next hop. Absent or negative on the last hop. */
  link?: number | null;
  cumMs: number;
  queueMs: number;
}

export interface Trace {
  status: TraceStatus;
  hops: TraceHop[];
  asPath: number[];
}

export interface RouterDetail {
  id: number;
  name: string;
  as: number;
  role: RouterRole;
  down: boolean;
  hosts: number;
  neighbors: { router: number; link: number; up: boolean; util: number }[];
  fib: { target: string; link: number; nextHop: number; cost: number }[];
}

export type Command =
  | { cmd: "fail_link"; link: number }
  | { cmd: "restore_link"; link: number }
  | { cmd: "fail_router"; router: number }
  | { cmd: "restore_router"; router: number }
  | { cmd: "fail_as"; as: number }
  | { cmd: "restore_as"; as: number }
  | { cmd: "latency_spike"; link: number; ms: number; secs: number }
  | { cmd: "packet_loss"; link: number; pct: number; secs: number }
  | { cmd: "ddos"; service: number; bots: number; mbps: number; secs: number }
  | {
      cmd: "flash_crowd";
      service: number;
      users: number;
      mbps: number;
      rampSecs: number;
      secs: number;
    }
  | { cmd: "chaos_monkey"; enabled: boolean; perMinute: number }
  | { cmd: "set_routing"; mode: RoutingMode }
  | { cmd: "set_speed"; speed: number }
  | { cmd: "pause" }
  | { cmd: "resume" }
  | { cmd: "heal_all" }
  | { cmd: "reset"; seed?: number }
  | { cmd: "run_scenario"; name: string };

export type CommandResult = { ok: true } | { ok: false; error: string };
