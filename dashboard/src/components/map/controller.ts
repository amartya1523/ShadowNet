import { live, store, type AppState } from "@/lib/state";
import type { Trace } from "@/lib/protocol";
import type { TopoIndex } from "@/lib/topology";
import { buildGeometry, hitLink, hitRouter, type MapGeometry } from "./geometry";
import { PacketField } from "./packets";
import {
  clampCamera,
  easeCamera,
  fitCamera,
  toWorld,
  zoomAt,
  type Camera,
} from "./projection";
import { MapRenderer, type MapTarget, type TraceOverlay } from "./renderer";

export interface MapCallbacks {
  onHover(target: MapTarget | null, x: number, y: number): void;
  onPick(target: MapTarget | null, ev: { shift: boolean }): void;
  onFps?(fps: number): void;
}

const MAX_ZOOM = 40;
const DRAG_SLOP = 4;
const HIT_PX_ROUTER = 7;
const HIT_PX_LINK = 5;

/**
 * Imperative half of the map: camera, pointer handling and the render loop.
 * Lives outside React so a 60 fps canvas never causes a reconciliation.
 */
export class MapController {
  private ctx: CanvasRenderingContext2D;
  private renderer = new MapRenderer();
  private packets = new PacketField();
  private geo: MapGeometry | null = null;
  private index: TopoIndex | null = null;

  private cam: Camera = { cx: 0, cy: 0, scale: 1 };
  private target: Camera = { cx: 0, cy: 0, scale: 1 };
  private fit: Camera = { cx: 0, cy: 0, scale: 1 };
  /** Untouched cameras follow resizes; once the user pans, we leave it alone. */
  private userMoved = false;

  private width = 0;
  private height = 0;
  private dpr = 1;
  private raf = 0;
  private lastTs = 0;
  private fpsAcc = { frames: 0, since: 0 };

  private drag: { x: number; y: number; cam: Camera; moved: boolean; id: number } | null = null;
  private hover: MapTarget | null = null;
  private lastFocusNonce = 0;
  private traceCache: { result: Trace | null; overlay: TraceOverlay | null } = { result: null, overlay: null };

  private disposers: (() => void)[] = [];

  constructor(
    private readonly canvas: HTMLCanvasElement,
    private readonly cb: MapCallbacks,
  ) {
    const ctx = canvas.getContext("2d", { alpha: true });
    if (!ctx) throw new Error("2D canvas not available");
    this.ctx = ctx;
    this.bindEvents();
    const ro = new ResizeObserver(() => this.resize());
    ro.observe(canvas);
    this.disposers.push(() => ro.disconnect());
    this.resize();
    this.lastFocusNonce = store.get().focus?.nonce ?? 0;
    this.raf = requestAnimationFrame(this.tick);
  }

  dispose(): void {
    cancelAnimationFrame(this.raf);
    for (const d of this.disposers) d();
  }

  setTopology(index: TopoIndex | null): void {
    if (index === this.index) return;
    this.index = index;
    this.geo = index ? buildGeometry(index.topology) : null;
    this.packets.reset();
    this.hover = null;
    this.userMoved = false;
    this.refit(true);
  }

  resetView(): void {
    this.userMoved = false;
    this.target = { ...this.fit };
  }

  zoomBy(factor: number): void {
    this.userMoved = true;
    this.target = clampCamera(
      zoomAt(this.target, this.width, this.height, this.width / 2, this.height / 2, factor),
      this.fit.scale * 0.6,
      this.fit.scale * MAX_ZOOM,
    );
  }

  get zoom(): number {
    return this.target.scale / this.fit.scale;
  }

  private resize(): void {
    const rect = this.canvas.getBoundingClientRect();
    const dpr = Math.min(window.devicePixelRatio || 1, 2.5);
    this.width = Math.max(1, rect.width);
    this.height = Math.max(1, rect.height);
    this.dpr = dpr;
    this.canvas.width = Math.round(this.width * dpr);
    this.canvas.height = Math.round(this.height * dpr);
    this.refit(!this.userMoved);
  }

  private refit(snap: boolean): void {
    if (!this.geo) return;
    const b = this.geo.bounds;
    // Pad the data bounds so edge cities are not glued to the frame, and
    // leave room at the bottom for the legend overlay.
    const padded = { minX: b.minX - 6, maxX: b.maxX + 6, minY: b.minY - 7, maxY: b.maxY + 9 };
    this.fit = fitCamera(padded, this.width, this.height, 24);
    if (snap) {
      this.cam = { ...this.fit };
      this.target = { ...this.fit };
    }
  }

  private focus(state: AppState): void {
    const f = state.focus;
    if (!f || f.nonce === this.lastFocusNonce || !this.geo) return;
    this.lastFocusNonce = f.nonce;
    const geo = this.geo;
    let x: number;
    let y: number;
    if (f.kind === "link") {
      x = geo.midX[f.id];
      y = geo.midY[f.id];
    } else {
      x = geo.routerX[f.id];
      y = geo.routerY[f.id];
    }
    if (x === undefined || y === undefined) return;
    this.userMoved = true;
    this.target = clampCamera(
      { cx: x, cy: y, scale: Math.max(this.target.scale, this.fit.scale * 3) },
      this.fit.scale * 0.6,
      this.fit.scale * MAX_ZOOM,
    );
  }

  private traceOverlay(state: AppState): TraceOverlay | null {
    const result = state.trace?.result ?? null;
    if (result === this.traceCache.result) return this.traceCache.overlay;
    let overlay: TraceOverlay | null = null;
    if (result && this.index && result.hops.length > 0) {
      const routers = result.hops.map((h) => h.router);
      const links = routers.slice(0, -1).map((r, i) => {
        const l = result.hops[i].link;
        return typeof l === "number" && l >= 0 ? l : this.index!.linkBetween(r, routers[i + 1]);
      });
      overlay = { routers, links, ok: result.status === "ok" };
    }
    this.traceCache = { result, overlay };
    return overlay;
  }

  private tick = (ts: number): void => {
    this.raf = requestAnimationFrame(this.tick);
    const dt = this.lastTs ? Math.min(100, ts - this.lastTs) : 16;
    this.lastTs = ts;
    this.reportFps(ts);

    const state = store.get();
    this.focus(state);
    this.cam = easeCamera(this.cam, this.target, dt);

    const geo = this.geo;
    const index = this.index;
    if (!geo || !index) {
      this.ctx.setTransform(1, 0, 0, 1, 0, 0);
      this.ctx.clearRect(0, 0, this.canvas.width, this.canvas.height);
      return;
    }

    const cur = live.cur && live.cur.runId === index.topology.runId ? live.cur : null;
    const prev = cur && live.prev && live.prev.runId === cur.runId ? live.prev : null;
    if (cur) this.packets.sync(cur, geo, index);
    if (!cur?.paused) this.packets.step(dt * Math.min(2, Math.max(0.5, cur?.speed ?? 1)));

    const traceSrc = state.trace && state.trace.dst === null ? state.trace.src : null;
    const scene = {
      geo,
      index,
      cam: this.cam,
      fitScale: this.fit.scale,
      width: this.width,
      height: this.height,
      dpr: this.dpr,
      now: ts,
      prev,
      cur,
      blend: Math.min(1, (performance.now() - live.receivedAt) / live.interval),
      hover: this.hover,
      selection: state.selection,
      trace: this.traceOverlay(state),
      traceSrc,
    };
    this.renderer.draw(this.ctx, scene);
    this.ctx.setTransform(this.dpr, 0, 0, this.dpr, 0, 0);
    this.packets.draw(this.ctx, this.cam, this.width, this.height, this.cam.scale / this.fit.scale);
    this.renderer.drawLabels(this.ctx, scene);
  };

  private reportFps(ts: number): void {
    if (!this.cb.onFps) return;
    const acc = this.fpsAcc;
    acc.frames++;
    if (!acc.since) acc.since = ts;
    if (ts - acc.since >= 1000) {
      this.cb.onFps((acc.frames * 1000) / (ts - acc.since));
      acc.frames = 0;
      acc.since = ts;
    }
  }

  private hitTest(sx: number, sy: number): MapTarget | null {
    if (!this.geo) return null;
    const [wx, wy] = toWorld(this.cam, this.width, this.height, sx, sy);
    const zoom = this.cam.scale / this.fit.scale;
    const grow = Math.min(2.6, Math.max(1, zoom ** 0.5));
    const r = hitRouter(this.geo, wx, wy, (HIT_PX_ROUTER * grow) / this.cam.scale);
    if (r >= 0) return { kind: "router", id: r };
    const l = hitLink(this.geo, wx, wy, HIT_PX_LINK / this.cam.scale);
    if (l >= 0) return { kind: "link", id: l };
    return null;
  }

  private local(ev: { clientX: number; clientY: number }): [number, number] {
    const rect = this.canvas.getBoundingClientRect();
    return [ev.clientX - rect.left, ev.clientY - rect.top];
  }

  private setHover(target: MapTarget | null, x: number, y: number): void {
    this.hover = target;
    this.canvas.style.cursor = this.drag?.moved ? "grabbing" : target ? "pointer" : "grab";
    this.cb.onHover(target, x, y);
  }

  private bindEvents(): void {
    const c = this.canvas;
    const on = <K extends keyof HTMLElementEventMap>(
      type: K,
      fn: (ev: HTMLElementEventMap[K]) => void,
      opts?: AddEventListenerOptions,
    ) => {
      c.addEventListener(type, fn as EventListener, opts);
      this.disposers.push(() => c.removeEventListener(type, fn as EventListener, opts));
    };

    on("pointerdown", (ev) => {
      if (ev.button !== 0) return;
      const [x, y] = this.local(ev);
      this.drag = { x, y, cam: { ...this.target }, moved: false, id: ev.pointerId };
      c.setPointerCapture(ev.pointerId);
    });

    on("pointermove", (ev) => {
      const [x, y] = this.local(ev);
      const d = this.drag;
      if (d) {
        if (!d.moved && Math.hypot(x - d.x, y - d.y) > DRAG_SLOP) {
          d.moved = true;
          this.userMoved = true;
          this.setHover(null, x, y);
        }
        if (d.moved) {
          const next = clampCamera(
            {
              scale: d.cam.scale,
              cx: d.cam.cx - (x - d.x) / d.cam.scale,
              cy: d.cam.cy - (y - d.y) / d.cam.scale,
            },
            this.fit.scale * 0.6,
            this.fit.scale * MAX_ZOOM,
          );
          // Drag tracks the pointer exactly; easing here feels like lag.
          this.target = next;
          this.cam = { ...next };
          return;
        }
      }
      this.setHover(this.hitTest(x, y), x, y);
    });

    const endDrag = (ev: PointerEvent, cancelled: boolean) => {
      const d = this.drag;
      if (!d || d.id !== ev.pointerId) return;
      this.drag = null;
      if (c.hasPointerCapture(ev.pointerId)) c.releasePointerCapture(ev.pointerId);
      const [x, y] = this.local(ev);
      if (!d.moved && !cancelled) this.cb.onPick(this.hitTest(x, y), { shift: ev.shiftKey });
      this.setHover(cancelled ? null : this.hitTest(x, y), x, y);
    };
    on("pointerup", (ev) => endDrag(ev, false));
    on("pointercancel", (ev) => endDrag(ev, true));
    on("pointerleave", () => {
      if (!this.drag) this.setHover(null, 0, 0);
    });

    on(
      "wheel",
      (ev) => {
        ev.preventDefault();
        const [x, y] = this.local(ev);
        // Trackpads send many small deltas, mice a few large ones; exp keeps both sane.
        const delta = ev.deltaMode === 1 ? ev.deltaY * 16 : ev.deltaY;
        const factor = Math.exp(-delta * 0.0015);
        this.userMoved = true;
        this.target = clampCamera(
          zoomAt(this.target, this.width, this.height, x, y, factor),
          this.fit.scale * 0.6,
          this.fit.scale * MAX_ZOOM,
        );
      },
      { passive: false },
    );

    on("dblclick", (ev) => {
      ev.preventDefault();
      this.resetView();
    });
  }
}
