import type {
  Command,
  CommandResult,
  History,
  RouterDetail,
  Scenario,
  ServerMessage,
  Topology,
  Trace,
} from "./protocol";

// Same origin in production (shadownetd serves the export). For `next dev`
// on :3000, point NEXT_PUBLIC_SHADOWNET_URL at the daemon.
const configuredBase = (process.env.NEXT_PUBLIC_SHADOWNET_URL ?? "").replace(/\/+$/, "");

export function httpBase(): string {
  if (configuredBase) return configuredBase;
  if (typeof window === "undefined") return "";
  return window.location.origin;
}

export function wsUrl(): string {
  const base = new URL(httpBase());
  base.protocol = base.protocol === "https:" ? "wss:" : "ws:";
  base.pathname = "/ws";
  return base.toString();
}

export function displayAddress(): string {
  try {
    return new URL(httpBase()).host;
  } catch {
    return "shadownetd";
  }
}

async function getJson<T>(path: string, signal?: AbortSignal): Promise<T> {
  const res = await fetch(`${httpBase()}${path}`, { signal, cache: "no-store" });
  if (!res.ok) throw new Error(`${path}: HTTP ${res.status}`);
  return (await res.json()) as T;
}

export const api = {
  topology: (signal?: AbortSignal) => getJson<Topology>("/api/topology", signal),
  history: (signal?: AbortSignal) => getJson<History>("/api/history", signal),
  scenarios: (signal?: AbortSignal) => getJson<Scenario[]>("/api/scenarios", signal),
  router: (id: number, signal?: AbortSignal) => getJson<RouterDetail>(`/api/router/${id}`, signal),
  trace: (src: number, dst: number, signal?: AbortSignal) =>
    getJson<Trace>(`/api/trace?src=${src}&dst=${dst}`, signal),

  async command(command: Command): Promise<CommandResult> {
    try {
      const res = await fetch(`${httpBase()}/api/command`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(command),
      });
      const body = (await res.json().catch(() => null)) as CommandResult | null;
      if (body) return body;
      return { ok: false, error: `HTTP ${res.status}` };
    } catch (err) {
      return { ok: false, error: err instanceof Error ? err.message : String(err) };
    }
  },
};

export type SocketStatus = "connecting" | "open" | "closed";

interface LiveSocketHandlers {
  onMessage: (msg: ServerMessage) => void;
  onStatus: (status: SocketStatus) => void;
}

/**
 * The one WebSocket to /ws. Reconnects forever with capped exponential
 * backoff plus jitter, so a restarted daemon is picked up without a reload
 * and several open dashboards do not reconnect in lockstep.
 */
export class LiveSocket {
  private ws: WebSocket | null = null;
  private retryTimer: ReturnType<typeof setTimeout> | null = null;
  private attempt = 0;
  private stopped = false;

  constructor(
    private readonly url: string,
    private readonly handlers: LiveSocketHandlers,
  ) {}

  start(): void {
    this.stopped = false;
    this.connect();
  }

  stop(): void {
    this.stopped = true;
    if (this.retryTimer) clearTimeout(this.retryTimer);
    this.retryTimer = null;
    const ws = this.ws;
    this.ws = null;
    if (ws) {
      ws.onopen = ws.onmessage = ws.onclose = ws.onerror = null;
      ws.close();
    }
  }

  private connect(): void {
    this.handlers.onStatus("connecting");
    let ws: WebSocket;
    try {
      ws = new WebSocket(this.url);
    } catch {
      this.scheduleRetry();
      return;
    }
    this.ws = ws;

    ws.onopen = () => {
      this.attempt = 0;
      this.handlers.onStatus("open");
    };
    ws.onmessage = (ev) => {
      if (typeof ev.data !== "string") return;
      let msg: ServerMessage;
      try {
        msg = JSON.parse(ev.data) as ServerMessage;
      } catch {
        return;
      }
      this.handlers.onMessage(msg);
    };
    ws.onerror = () => ws.close();
    ws.onclose = () => {
      if (this.ws !== ws) return;
      this.ws = null;
      this.handlers.onStatus("closed");
      this.scheduleRetry();
    };
  }

  private scheduleRetry(): void {
    if (this.stopped) return;
    const delay = Math.min(8000, 400 * 2 ** this.attempt) * (0.75 + Math.random() * 0.5);
    this.attempt += 1;
    this.retryTimer = setTimeout(() => this.connect(), delay);
  }
}
