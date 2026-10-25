import { api, LiveSocket, wsUrl } from "./client";
import type { Command, ServerMessage } from "./protocol";
import { ingestFrame, notify, seedLog, setHistory, setTopology, store } from "./state";

const HISTORY_POLL_MS = 4000;

/**
 * Owns the connection to shadownetd: the live socket, topology (re)loading
 * keyed on runId, and history polling. One instance per page.
 */
export class Session {
  private socket: LiveSocket;
  private historyTimer: ReturnType<typeof setInterval> | null = null;
  private topologyRequest: AbortController | null = null;
  private loadingRunId: number | null = null;
  /** Set on every new socket; the first hello then revalidates the held topology. */
  private reconnected = false;

  constructor() {
    this.socket = new LiveSocket(wsUrl(), {
      onMessage: (msg) => this.onMessage(msg),
      onStatus: (status) => {
        store.set({ status });
        if (status === "open") {
          this.reconnected = true;
          void this.loadScenarios();
          void this.pollHistory();
        }
      },
    });
  }

  start(): void {
    this.socket.start();
    this.historyTimer = setInterval(() => void this.pollHistory(), HISTORY_POLL_MS);
  }

  stop(): void {
    this.socket.stop();
    if (this.historyTimer) clearInterval(this.historyTimer);
    this.topologyRequest?.abort();
  }

  private onMessage(msg: ServerMessage): void {
    const held = store.get().topology?.runId ?? null;
    if (msg.type === "hello") {
      // A restarted daemon can come back with the same runId but a different
      // world, so a fresh connection always revalidates.
      if (msg.runId !== held || this.reconnected) void this.loadTopology(msg.runId);
      this.reconnected = false;
      if (msg.recent) seedLog(msg.recent);
      return;
    }
    if (msg.type !== "frame") return;
    if (msg.runId !== held) {
      void this.loadTopology(msg.runId);
      return;
    }
    ingestFrame(msg);
  }

  private async loadTopology(runId: number): Promise<void> {
    if (this.loadingRunId === runId) return;
    this.loadingRunId = runId;
    this.topologyRequest?.abort();
    const ctrl = new AbortController();
    this.topologyRequest = ctrl;
    try {
      const topology = await api.topology(ctrl.signal);
      const held = store.get().topology;
      const unchanged =
        held &&
        held.runId === topology.runId &&
        held.seed === topology.seed &&
        held.routers.length === topology.routers.length &&
        held.links.length === topology.links.length;
      if (!unchanged) setTopology(topology);
      void this.pollHistory();
    } catch (err) {
      if (ctrl.signal.aborted) return;
      store.set({ topologyError: err instanceof Error ? err.message : String(err) });
    } finally {
      if (this.loadingRunId === runId) this.loadingRunId = null;
    }
  }

  private async loadScenarios(): Promise<void> {
    try {
      store.set({ scenarios: await api.scenarios() });
    } catch {
      // Scenarios are optional; the panel shows its empty state.
    }
  }

  private async pollHistory(): Promise<void> {
    const runId = store.get().topology?.runId;
    if (runId === undefined || store.get().status !== "open") return;
    try {
      const h = await api.history();
      if (store.get().topology?.runId === runId) setHistory(h);
    } catch {
      // Next poll will retry; the chart keeps extending from frames meanwhile.
    }
  }
}

/** Sends a command and surfaces failures in the notice strip. */
export async function send(command: Command, okText?: string): Promise<boolean> {
  const res = await api.command(command);
  if (!res.ok) {
    notify("error", `${command.cmd}: ${res.error}`);
    return false;
  }
  if (okText) notify("ok", okText);
  return true;
}
