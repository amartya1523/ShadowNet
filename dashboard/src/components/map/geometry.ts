import type { Topology } from "@/lib/protocol";
import { project, type Bounds } from "./projection";

/**
 * World-space geometry for a topology, built once per run. Links are
 * quadratic curves bowed towards the nearer pole, which is roughly what a
 * great circle looks like on an equirectangular map. Each curve is also
 * flattened into a polyline for packet animation and hit testing.
 */
export interface MapGeometry {
  routerX: Float32Array;
  routerY: Float32Array;
  /** Curve endpoints and control point, end point unwrapped across the antimeridian. */
  ax: Float32Array;
  ay: Float32Array;
  cx: Float32Array;
  cy: Float32Array;
  bx: Float32Array;
  by: Float32Array;
  /** 0, or the x offset (±360) of the second copy drawn for links crossing the antimeridian. */
  wrap: Float32Array;
  midX: Float32Array;
  midY: Float32Array;
  /** Flattened curve points, [x0, y0, x1, y1, ...] per link. */
  pts: Float32Array;
  ptStart: Uint32Array;
  ptCount: Uint16Array;
  routerGrid: SpatialGrid;
  linkGrid: SpatialGrid;
  bounds: Bounds;
}

const CELL = 2;

export class SpatialGrid {
  private cells = new Map<number, number[]>();

  private static key(ix: number, iy: number): number {
    return (ix + 2048) * 4096 + (iy + 2048);
  }

  insertPoint(id: number, x: number, y: number): void {
    this.insertCell(id, Math.floor(x / CELL), Math.floor(y / CELL));
  }

  insertBox(id: number, x0: number, y0: number, x1: number, y1: number): void {
    const ix0 = Math.floor(Math.min(x0, x1) / CELL);
    const ix1 = Math.floor(Math.max(x0, x1) / CELL);
    const iy0 = Math.floor(Math.min(y0, y1) / CELL);
    const iy1 = Math.floor(Math.max(y0, y1) / CELL);
    for (let ix = ix0; ix <= ix1; ix++) for (let iy = iy0; iy <= iy1; iy++) this.insertCell(id, ix, iy);
  }

  /** Ids in every cell touching the square of half-size `r` around (x, y). May repeat. */
  query(x: number, y: number, r: number, out: number[]): number[] {
    out.length = 0;
    const ix0 = Math.floor((x - r) / CELL);
    const ix1 = Math.floor((x + r) / CELL);
    const iy0 = Math.floor((y - r) / CELL);
    const iy1 = Math.floor((y + r) / CELL);
    for (let ix = ix0; ix <= ix1; ix++) {
      for (let iy = iy0; iy <= iy1; iy++) {
        const cell = this.cells.get(SpatialGrid.key(ix, iy));
        if (cell) for (const id of cell) out.push(id);
      }
    }
    return out;
  }

  private insertCell(id: number, ix: number, iy: number): void {
    const k = SpatialGrid.key(ix, iy);
    const cell = this.cells.get(k);
    if (cell) {
      if (cell[cell.length - 1] !== id) cell.push(id);
    } else this.cells.set(k, [id]);
  }
}

export function buildGeometry(topo: Topology): MapGeometry {
  const nr = topo.routers.length;
  const nl = topo.links.length;
  const routerX = new Float32Array(nr);
  const routerY = new Float32Array(nr);
  const routerGrid = new SpatialGrid();
  const bounds: Bounds = { minX: Infinity, minY: Infinity, maxX: -Infinity, maxY: -Infinity };

  for (const r of topo.routers) {
    const [x, y] = project(r.lon, r.lat);
    routerX[r.id] = x;
    routerY[r.id] = y;
    routerGrid.insertPoint(r.id, x, y);
    bounds.minX = Math.min(bounds.minX, x);
    bounds.maxX = Math.max(bounds.maxX, x);
    bounds.minY = Math.min(bounds.minY, y);
    bounds.maxY = Math.max(bounds.maxY, y);
  }
  if (nr === 0) Object.assign(bounds, { minX: -180, maxX: 180, minY: -70, maxY: 70 });

  const g = {
    ax: new Float32Array(nl),
    ay: new Float32Array(nl),
    cx: new Float32Array(nl),
    cy: new Float32Array(nl),
    bx: new Float32Array(nl),
    by: new Float32Array(nl),
    wrap: new Float32Array(nl),
    midX: new Float32Array(nl),
    midY: new Float32Array(nl),
  };
  const ptStart = new Uint32Array(nl);
  const ptCount = new Uint16Array(nl);
  const flat: number[] = [];
  const linkGrid = new SpatialGrid();

  for (const l of topo.links) {
    const i = l.id;
    const ax = routerX[l.a];
    const ay = routerY[l.a];
    let bx = routerX[l.b];
    const by = routerY[l.b];
    if (bx - ax > 180) bx -= 360;
    else if (ax - bx > 180) bx += 360;

    const dx = bx - ax;
    const dy = by - ay;
    const len = Math.hypot(dx, dy);
    let cx = (ax + bx) / 2;
    let cy = (ay + by) / 2;
    if (len > 3) {
      // East-west links bow towards the pole; north-south ones stay straight.
      const bend = len * Math.min(0.2, 0.06 + len / 600) * (Math.abs(dx) / len);
      let nx = -dy / len;
      let ny = dx / len;
      const towardsPole = cy < 0 ? -1 : 1;
      if (Math.sign(ny) !== towardsPole) {
        nx = -nx;
        ny = -ny;
      }
      cx += nx * bend;
      cy += ny * bend;
    }

    g.ax[i] = ax;
    g.ay[i] = ay;
    g.bx[i] = bx;
    g.by[i] = by;
    g.cx[i] = cx;
    g.cy[i] = cy;
    g.wrap[i] = bx > 180 ? -360 : bx < -180 ? 360 : 0;
    g.midX[i] = 0.25 * ax + 0.5 * cx + 0.25 * bx;
    g.midY[i] = 0.25 * ay + 0.5 * cy + 0.25 * by;

    const n = len > 3 ? Math.min(32, Math.max(6, Math.ceil(len / 2))) : 2;
    ptStart[i] = flat.length / 2;
    ptCount[i] = n;
    let px = ax;
    let py = ay;
    for (let k = 0; k < n; k++) {
      const t = k / (n - 1);
      const u = 1 - t;
      const x = u * u * ax + 2 * u * t * cx + t * t * bx;
      const y = u * u * ay + 2 * u * t * cy + t * t * by;
      flat.push(x, y);
      if (k > 0) {
        linkGrid.insertBox(i, px, py, x, y);
        if (g.wrap[i]) linkGrid.insertBox(i, px + g.wrap[i], py, x + g.wrap[i], y);
      }
      px = x;
      py = y;
    }
  }

  return {
    routerX,
    routerY,
    ...g,
    pts: new Float32Array(flat),
    ptStart,
    ptCount,
    routerGrid,
    linkGrid,
    bounds,
  };
}

const scratch: number[] = [];

export function hitRouter(geo: MapGeometry, x: number, y: number, radius: number): number {
  let best = -1;
  let bestD = radius * radius;
  for (const id of geo.routerGrid.query(x, y, radius, scratch)) {
    const d = (geo.routerX[id] - x) ** 2 + (geo.routerY[id] - y) ** 2;
    if (d <= bestD) {
      bestD = d;
      best = id;
    }
  }
  return best;
}

export function hitLink(geo: MapGeometry, x: number, y: number, radius: number): number {
  let best = -1;
  let bestD = radius * radius;
  for (const id of geo.linkGrid.query(x, y, radius, scratch)) {
    const shifts = geo.wrap[id] ? [0, geo.wrap[id]] : [0];
    const s = geo.ptStart[id] * 2;
    const n = geo.ptCount[id];
    for (const off of shifts) {
      for (let k = 0; k < n - 1; k++) {
        const d = segDist2(
          x,
          y,
          geo.pts[s + k * 2] + off,
          geo.pts[s + k * 2 + 1],
          geo.pts[s + k * 2 + 2] + off,
          geo.pts[s + k * 2 + 3],
        );
        if (d < bestD) {
          bestD = d;
          best = id;
        }
      }
    }
  }
  return best;
}

function segDist2(px: number, py: number, x0: number, y0: number, x1: number, y1: number): number {
  const dx = x1 - x0;
  const dy = y1 - y0;
  const l2 = dx * dx + dy * dy;
  let t = l2 > 0 ? ((px - x0) * dx + (py - y0) * dy) / l2 : 0;
  t = Math.max(0, Math.min(1, t));
  const ex = x0 + t * dx - px;
  const ey = y0 + t * dy - py;
  return ex * ex + ey * ey;
}

/** Wraps a world x back into [-180, 180). */
export function wrapX(x: number): number {
  return ((((x + 180) % 360) + 360) % 360) - 180;
}
