// Equirectangular projection. World units are degrees: x = longitude,
// y = -latitude (so north is up on screen). The camera maps world to CSS
// pixels with a uniform scale; the renderer multiplies in devicePixelRatio.

export interface Camera {
  /** World point at the centre of the canvas. */
  cx: number;
  cy: number;
  /** CSS pixels per degree. */
  scale: number;
}

export interface Bounds {
  minX: number;
  minY: number;
  maxX: number;
  maxY: number;
}

export const WORLD: Bounds = { minX: -180, minY: -90, maxX: 180, maxY: 90 };

export const project = (lon: number, lat: number): [number, number] => [lon, -lat];

export function toWorld(cam: Camera, w: number, h: number, sx: number, sy: number): [number, number] {
  return [(sx - w / 2) / cam.scale + cam.cx, (sy - h / 2) / cam.scale + cam.cy];
}

export function fitCamera(b: Bounds, w: number, h: number, padding = 32): Camera {
  const bw = Math.max(1, b.maxX - b.minX);
  const bh = Math.max(1, b.maxY - b.minY);
  const scale = Math.min((w - padding * 2) / bw, (h - padding * 2) / bh);
  return { cx: (b.minX + b.maxX) / 2, cy: (b.minY + b.maxY) / 2, scale: Math.max(0.5, scale) };
}

/** Zoom by `factor` keeping the world point under (sx, sy) fixed. */
export function zoomAt(cam: Camera, w: number, h: number, sx: number, sy: number, factor: number): Camera {
  const [wx, wy] = toWorld(cam, w, h, sx, sy);
  const scale = cam.scale * factor;
  return {
    scale,
    cx: wx - (sx - w / 2) / scale,
    cy: wy - (sy - h / 2) / scale,
  };
}

/**
 * Keeps at least part of the world on screen so a wild drag cannot lose the
 * map entirely.
 */
export function clampCamera(cam: Camera, minScale: number, maxScale: number): Camera {
  const scale = Math.min(maxScale, Math.max(minScale, cam.scale));
  return {
    scale,
    cx: Math.min(WORLD.maxX, Math.max(WORLD.minX, cam.cx)),
    cy: Math.min(80, Math.max(-80, cam.cy)),
  };
}

/** Frame-rate independent exponential approach of `cur` towards `target`. */
export function easeCamera(cur: Camera, target: Camera, dtMs: number, rate = 14): Camera {
  const k = 1 - Math.exp((-rate * dtMs) / 1000);
  // Interpolate scale in log space so zooming feels uniform.
  const ls = Math.log(cur.scale) + (Math.log(target.scale) - Math.log(cur.scale)) * k;
  const next = {
    cx: cur.cx + (target.cx - cur.cx) * k,
    cy: cur.cy + (target.cy - cur.cy) * k,
    scale: Math.exp(ls),
  };
  const settled =
    Math.abs(next.cx - target.cx) * next.scale < 0.05 &&
    Math.abs(next.cy - target.cy) * next.scale < 0.05 &&
    Math.abs(next.scale / target.scale - 1) < 1e-4;
  return settled ? { ...target } : next;
}
