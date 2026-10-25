import type { Frame, FlowKind } from "@/lib/protocol";
import type { TopoIndex } from "@/lib/topology";
import { type MapGeometry, wrapX } from "./geometry";
import type { Camera } from "./projection";
import { makeGlowSprite, packetRgb } from "./palette";

interface Track {
  kind: FlowKind;
  pathKey: string;
  /** Polyline through the flow's routers, world units, unwrapped. */
  pts: Float32Array;
  /** Cumulative length at each point. */
  cum: Float32Array;
  total: number;
  /** Position of the lead packet along the path, 0..1. */
  phase: number;
  /** Wall-clock ms for one trip; longer for higher-latency flows. */
  tripMs: number;
  /** 0..1, ramps up on arrival and down after the flow leaves the frame. */
  alpha: number;
  alive: boolean;
  dots: number;
}

const FADE_MS = 350;
const TRAIL_STEPS = 4;
const TRAIL_PX = 16;

/**
 * Animated packets. Frames only say which flows exist and where they go;
 * motion between frames is entirely local, so it stays smooth at display
 * rate however irregularly frames arrive.
 */
export class PacketField {
  private tracks = new Map<number, Track>();
  private syncedSeq = -1;
  private sprites: Record<FlowKind, HTMLCanvasElement> | null = null;

  reset(): void {
    this.tracks.clear();
    this.syncedSeq = -1;
  }

  sync(frame: Frame, geo: MapGeometry, index: TopoIndex): void {
    if (frame.seq === this.syncedSeq) return;
    this.syncedSeq = frame.seq;

    for (const t of this.tracks.values()) t.alive = false;
    for (const f of frame.flows) {
      if (f.path.length < 2) continue;
      const pathKey = f.path.join(",");
      const existing = this.tracks.get(f.id);
      const tripMs = Math.min(5200, Math.max(900, f.latencyMs * 22));
      if (existing && existing.pathKey === pathKey) {
        existing.alive = true;
        existing.tripMs = tripMs;
        existing.kind = f.kind;
        continue;
      }
      const line = buildPath(f.path, geo, index);
      if (!line) continue;
      this.tracks.set(f.id, {
        kind: f.kind,
        pathKey,
        ...line,
        // A rerouted flow keeps its place in the cycle so it doesn't visibly jump.
        phase: existing ? existing.phase : Math.random(),
        tripMs,
        alpha: existing ? existing.alpha : 0,
        alive: true,
        dots: f.kind === "bg" ? 1 : 2,
      });
    }
  }

  step(dtMs: number): void {
    for (const [id, t] of this.tracks) {
      t.phase = (t.phase + dtMs / t.tripMs) % 1;
      const da = dtMs / FADE_MS;
      t.alpha = t.alive ? Math.min(1, t.alpha + da) : t.alpha - da;
      if (t.alpha <= 0 && !t.alive) this.tracks.delete(id);
    }
  }

  draw(ctx: CanvasRenderingContext2D, cam: Camera, w: number, h: number, zoom: number): void {
    if (!this.sprites) {
      this.sprites = {
        bg: makeGlowSprite(packetRgb.bg),
        crowd: makeGlowSprite(packetRgb.crowd),
        attack: makeGlowSprite(packetRgb.attack),
      };
    }
    const base = 11 * Math.min(1.8, Math.max(1, Math.sqrt(zoom)));
    const trailWorld = TRAIL_PX / cam.scale;
    const hw = w / 2;
    const hh = h / 2;

    ctx.save();
    ctx.globalCompositeOperation = "lighter";
    for (const t of this.tracks.values()) {
      const sprite = this.sprites[t.kind];
      const size = t.kind === "bg" ? base : base * 1.25;
      for (let d = 0; d < t.dots; d++) {
        const lead = ((t.phase + d / t.dots) % 1) * t.total;
        for (let s = 0; s < TRAIL_STEPS; s++) {
          const dist = lead - s * (trailWorld / TRAIL_STEPS);
          if (dist < 0) break;
          pointAt(t, dist, pos);
          const sx = (wrapX(pos[0]) - cam.cx) * cam.scale + hw;
          const sy = (pos[1] - cam.cy) * cam.scale + hh;
          if (sx < -20 || sy < -20 || sx > w + 20 || sy > h + 20) continue;
          const k = 1 - s / TRAIL_STEPS;
          const sz = size * (0.45 + 0.55 * k);
          ctx.globalAlpha = t.alpha * k * (s === 0 ? 1 : 0.55);
          ctx.drawImage(sprite, sx - sz / 2, sy - sz / 2, sz, sz);
        }
      }
    }
    ctx.restore();
  }
}

function buildPath(
  path: number[],
  geo: MapGeometry,
  index: TopoIndex,
): { pts: Float32Array; cum: Float32Array; total: number } | null {
  const out: number[] = [geo.routerX[path[0]], geo.routerY[path[0]]];
  const links = index.topology.links;
  for (let i = 0; i < path.length - 1; i++) {
    const from = path[i];
    const to = path[i + 1];
    const lid = index.linkBetween(from, to);
    const curX = out[out.length - 2];
    if (lid < 0) {
      const tx = geo.routerX[to];
      out.push(tx + Math.round((curX - tx) / 360) * 360, geo.routerY[to]);
      continue;
    }
    const s = geo.ptStart[lid] * 2;
    const n = geo.ptCount[lid];
    const forward = links[lid].a === from;
    const firstX = forward ? geo.pts[s] : geo.pts[s + (n - 1) * 2];
    const shift = Math.round((curX - firstX) / 360) * 360;
    for (let k = 1; k < n; k++) {
      const j = forward ? k : n - 1 - k;
      out.push(geo.pts[s + j * 2] + shift, geo.pts[s + j * 2 + 1]);
    }
  }
  const count = out.length / 2;
  if (count < 2) return null;
  const pts = new Float32Array(out);
  const cum = new Float32Array(count);
  for (let k = 1; k < count; k++) {
    cum[k] = cum[k - 1] + Math.hypot(pts[k * 2] - pts[k * 2 - 2], pts[k * 2 + 1] - pts[k * 2 - 1]);
  }
  const total = cum[count - 1];
  return total > 0 ? { pts, cum, total } : null;
}

const pos: [number, number] = [0, 0];

function pointAt(t: Track, dist: number, out: [number, number]): void {
  const cum = t.cum;
  let lo = 0;
  let hi = cum.length - 1;
  while (hi - lo > 1) {
    const mid = (lo + hi) >> 1;
    if (cum[mid] < dist) lo = mid;
    else hi = mid;
  }
  const seg = cum[hi] - cum[lo];
  const u = seg > 0 ? (dist - cum[lo]) / seg : 0;
  const p = t.pts;
  out[0] = p[lo * 2] + (p[hi * 2] - p[lo * 2]) * u;
  out[1] = p[lo * 2 + 1] + (p[hi * 2 + 1] - p[lo * 2 + 1]) * u;
}
