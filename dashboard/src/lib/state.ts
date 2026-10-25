import type { SocketStatus } from "./client";
import type { EventLevel, Frame, History, Scenario, SimEvent, Topology, Trace } from "./protocol";
import { createStore, useStoreSlice } from "./store";
import { indexTopology, type TopoIndex } from "./topology";

export type Selection = { kind: "router"; id: number } | { kind: "link"; id: number };

export interface TraceState {
  src: number;
  dst: number | null;
  result: Trace | null;
  loading: boolean;
  error: string | null;
}

export interface FocusRequest {
  kind: "router" | "link";
  id: number;
  nonce: number;
}

export interface LogEntry {
  key: number;
  t: number;
  level: EventLevel;
  kind: string;
  text: string;
}

export interface Notice {
  key: number;
  level: "ok" | "error";
  text: string;
}

/** Simulated-second series for the history chart. */
export interface ChartSeries {
  t: number[];
  deliveredGbps: number[];
  p99Ms: number[];
}

/** Per-frame samples for the KPI sparklines, oldest first. */
export interface KpiSamples {
  deliveredGbps: number[];
  lossPct: number[];
  p50: number[];
  p99: number[];
  activeFlows: number[];
  linksDown: number[];
  congestedLinks: number[];
}

export interface AppState {
  status: SocketStatus;
  topology: Topology | null;
  index: TopoIndex | null;
  topologyError: string | null;
  frame: Frame | null;
  scenarios: Scenario[];
  series: ChartSeries;
  kpi: KpiSamples;
  log: LogEntry[];
  selection: Selection | null;
  traceMode: boolean;
  trace: TraceState | null;
  focus: FocusRequest | null;
  notice: Notice | null;
}

const LOG_LIMIT = 50;
const KPI_SAMPLES = 90;
const HISTORY_SECS = 600;

const emptySeries = (): ChartSeries => ({ t: [], deliveredGbps: [], p99Ms: [] });
const emptyKpi = (): KpiSamples => ({
  deliveredGbps: [],
  lossPct: [],
  p50: [],
  p99: [],
  activeFlows: [],
  linksDown: [],
  congestedLinks: [],
});

export const store = createStore<AppState>({
  status: "connecting",
  topology: null,
  index: null,
  topologyError: null,
  frame: null,
  scenarios: [],
  series: emptySeries(),
  kpi: emptyKpi(),
  log: [],
  selection: null,
  traceMode: false,
  trace: null,
  focus: null,
  notice: null,
});

export function useApp<T>(selector: (s: AppState) => T): T {
  return useStoreSlice(store, selector);
}

/**
 * The map renders at display rate and must not go through React, so the two
 * most recent frames live here as plain mutable data that the rAF loop reads.
 */
export const live = {
  prev: null as Frame | null,
  cur: null as Frame | null,
  receivedAt: 0,
  /** Smoothed wall-clock gap between frames, used to interpolate link colours. */
  interval: 200,
};

let logKey = 0;
let noticeKey = 0;

export function setTopology(topology: Topology): void {
  live.prev = null;
  live.cur = null;
  store.set({
    topology,
    index: indexTopology(topology),
    topologyError: null,
    frame: null,
    series: emptySeries(),
    kpi: emptyKpi(),
    selection: null,
    trace: null,
    traceMode: false,
  });
}

export function ingestFrame(frame: Frame): void {
  const now = performance.now();
  if (live.cur) {
    const gap = now - live.receivedAt;
    if (gap > 0 && gap < 2000) live.interval = live.interval * 0.8 + gap * 0.2;
  }
  live.prev = live.cur;
  live.cur = frame;
  live.receivedAt = now;

  store.set((s) => {
    const patch: Partial<AppState> = {
      frame,
      kpi: pushKpi(s.kpi, frame),
      series: pushSeries(s.series, frame),
    };
    if (frame.events.length > 0) {
      const added = frame.events.map((e) => ({ key: ++logKey, ...e }));
      patch.log = [...s.log, ...added].slice(-LOG_LIMIT);
    }
    return patch;
  });
}

/** Seed the event log from a hello, unless we already have one. */
export function seedLog(events: SimEvent[]): void {
  store.set((s) => (s.log.length > 0 ? {} : { log: events.map((e) => ({ key: ++logKey, ...e })).slice(-LOG_LIMIT) }));
}

export function setHistory(h: History): void {
  const t = h.t.slice(-HISTORY_SECS);
  const from = h.t.length - t.length;
  store.set({
    series: {
      t,
      deliveredGbps: h.deliveredGbps.slice(from),
      p99Ms: h.p99Ms.slice(from),
    },
  });
}

export function notify(level: Notice["level"], text: string): void {
  store.set({ notice: { key: ++noticeKey, level, text } });
}

export function select(selection: Selection | null): void {
  store.set({ selection });
}

export function focusOn(kind: FocusRequest["kind"], id: number): void {
  store.set((s) => ({
    focus: { kind, id, nonce: (s.focus?.nonce ?? 0) + 1 },
    selection: { kind, id },
  }));
}

function pushKpi(k: KpiSamples, f: Frame): KpiSamples {
  const push = (arr: number[], v: number) => {
    const next = arr.length >= KPI_SAMPLES ? arr.slice(1) : arr.slice();
    next.push(v);
    return next;
  };
  const s = f.stats;
  return {
    deliveredGbps: push(k.deliveredGbps, s.deliveredGbps),
    lossPct: push(k.lossPct, s.lossPct),
    p50: push(k.p50, s.latencyMs.p50),
    p99: push(k.p99, s.latencyMs.p99),
    activeFlows: push(k.activeFlows, s.flows.active),
    linksDown: push(k.linksDown, s.linksDown),
    congestedLinks: push(k.congestedLinks, s.congestedLinks),
  };
}

// /api/history is one point per simulated second; between polls we extend it
// from frames at the same resolution so the chart moves continuously.
function pushSeries(series: ChartSeries, f: Frame): ChartSeries {
  const last = series.t.length ? series.t[series.t.length - 1] : -Infinity;
  if (f.t < last) return emptySeriesFrom(f);
  if (f.t - last < 1) return series;
  const drop = series.t.length >= HISTORY_SECS ? 1 : 0;
  return {
    t: [...series.t.slice(drop), f.t],
    deliveredGbps: [...series.deliveredGbps.slice(drop), f.stats.deliveredGbps],
    p99Ms: [...series.p99Ms.slice(drop), f.stats.latencyMs.p99],
  };
}

function emptySeriesFrom(f: Frame): ChartSeries {
  return { t: [f.t], deliveredGbps: [f.stats.deliveredGbps], p99Ms: [f.stats.latencyMs.p99] };
}
