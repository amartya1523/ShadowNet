import { LinkState, type Frame } from "@/lib/protocol";
import type { TopoIndex } from "@/lib/topology";
import type { MapGeometry } from "./geometry";
import { WORLD, type Camera } from "./projection";
import { colors, makeGlowSprite, utilBucket, utilColor, UTIL_BUCKETS } from "./palette";

export type MapTarget = { kind: "router"; id: number } | { kind: "link"; id: number };

export interface TraceOverlay {
  routers: number[];
  links: number[];
  ok: boolean;
}

export interface SceneInput {
  geo: MapGeometry;
  index: TopoIndex;
  cam: Camera;
  /** Scale at which the whole topology fits; zoom = cam.scale / fitScale. */
  fitScale: number;
  width: number;
  height: number;
  dpr: number;
  now: number;
  prev: Frame | null;
  cur: Frame | null;
  /** 0..1 progress from `prev` to `cur`. */
  blend: number;
  hover: MapTarget | null;
  selection: MapTarget | null;
  trace: TraceOverlay | null;
  traceSrc: number | null;
}

const WIDTH_PX = [0.55, 0.85, 1.25, 1.85];
const HOT_UTIL = 850;
const RISK_GLOW = 300;

function widthClass(capacityGbps: number): number {
  if (capacityGbps <= 10) return 0;
  if (capacityGbps <= 40) return 1;
  if (capacityGbps <= 100) return 2;
  return 3;
}

interface CityMark {
  name: string;
  x: number;
  y: number;
  weight: number;
}

/**
 * Draws one frame of the map. Holds only caches derived from the topology;
 * every input that changes over time arrives in `SceneInput`.
 */
export class MapRenderer {
  private widthOf: Uint8Array = new Uint8Array(0);
  private cities: CityMark[] = [];
  private builtFor: TopoIndex | null = null;
  private downRouters = new Uint8Array(0);
  private downSeq = -1;
  private cityGlow: HTMLCanvasElement | null = null;
  private tier1Glow: HTMLCanvasElement | null = null;
  private monoFont = "monospace";
  private sansFont = "sans-serif";
  private labelBoxes: number[] = [];

  constructor() {
    if (typeof document !== "undefined") {
      const css = getComputedStyle(document.documentElement);
      this.monoFont = css.getPropertyValue("--font-mono").trim() || this.monoFont;
      this.sansFont = css.getPropertyValue("--font-sans").trim() || this.sansFont;
    }
  }

  draw(ctx: CanvasRenderingContext2D, s: SceneInput): void {
    this.prepare(s.index, s.geo);
    const { dpr, width, height } = s;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, height);

    const zoom = s.cam.scale / s.fitScale;
    this.drawGraticule(ctx, s);
    this.drawCityGlow(ctx, s);
    this.drawLinks(ctx, s, zoom);
    this.drawRouters(ctx, s, zoom);
    if (s.trace) this.drawTrace(ctx, s, s.trace);
    this.drawHighlights(ctx, s, zoom);
  }

  /** Overlay text drawn after packets so labels stay readable. */
  drawLabels(ctx: CanvasRenderingContext2D, s: SceneInput): void {
    ctx.setTransform(s.dpr, 0, 0, s.dpr, 0, 0);
    this.labelBoxes.length = 0;
    this.drawRiskTags(ctx, s);
    if (s.trace) this.drawTraceLabels(ctx, s, s.trace);
    this.drawCityLabels(ctx, s);
  }

  private prepare(index: TopoIndex, geo: MapGeometry): void {
    if (this.builtFor === index) return;
    this.builtFor = index;
    const topo = index.topology;
    this.widthOf = new Uint8Array(topo.links.length);
    for (const l of topo.links) this.widthOf[l.id] = widthClass(l.capacityGbps);

    const perCity = new Map<string, { n: number; x: number; y: number }>();
    for (const r of topo.routers) {
      const e = perCity.get(r.city) ?? { n: 0, x: 0, y: 0 };
      e.n += 1;
      e.x += geo.routerX[r.id];
      e.y += geo.routerY[r.id];
      perCity.set(r.city, e);
    }
    this.cities = topo.cities
      .map((c) => {
        const e = perCity.get(c.code);
        return {
          name: c.name,
          x: e ? e.x / e.n : c.lon,
          y: e ? e.y / e.n : -c.lat,
          weight: e?.n ?? 0,
        };
      })
      .sort((a, b) => b.weight - a.weight);
    this.downRouters = new Uint8Array(topo.routers.length);
    this.downSeq = -1;
    this.cityGlow ??= makeGlowSprite(colors.cityGlow, 128);
    this.tier1Glow ??= makeGlowSprite([120, 230, 255], 48);
  }

  private worldTransform(ctx: CanvasRenderingContext2D, s: SceneInput, shiftX = 0): void {
    const k = s.cam.scale * s.dpr;
    ctx.setTransform(
      k,
      0,
      0,
      k,
      s.dpr * (s.width / 2 - (s.cam.cx - shiftX) * s.cam.scale),
      s.dpr * (s.height / 2 - s.cam.cy * s.cam.scale),
    );
  }

  private drawGraticule(ctx: CanvasRenderingContext2D, s: SceneInput): void {
    const px = 1 / s.cam.scale;
    this.worldTransform(ctx, s);
    const step = s.cam.scale > 14 ? 5 : 10;
    const minor = new Path2D();
    const major = new Path2D();
    for (let lon = -180; lon <= 180; lon += step) {
      const p = lon % 30 === 0 ? major : minor;
      p.moveTo(lon, -85);
      p.lineTo(lon, 85);
    }
    for (let lat = -80; lat <= 80; lat += step) {
      const p = lat % 30 === 0 ? major : minor;
      p.moveTo(-180, -lat);
      p.lineTo(180, -lat);
    }
    ctx.lineWidth = px;
    ctx.strokeStyle = colors.graticule;
    ctx.stroke(minor);
    ctx.strokeStyle = colors.graticuleMajor;
    ctx.stroke(major);

    // Equator and tropics as faint dashed bands give the eye a latitude anchor.
    ctx.setLineDash([4 * px, 6 * px]);
    ctx.strokeStyle = "rgba(110, 190, 230, 0.16)";
    ctx.beginPath();
    for (const lat of [23.44, -23.44, 66.56]) {
      ctx.moveTo(-180, -lat);
      ctx.lineTo(180, -lat);
    }
    ctx.stroke();
    ctx.setLineDash([]);

    ctx.strokeStyle = "rgba(110, 190, 230, 0.22)";
    ctx.strokeRect(WORLD.minX, -85, WORLD.maxX - WORLD.minX, 170);

    ctx.setTransform(s.dpr, 0, 0, s.dpr, 0, 0);
    ctx.font = `9px ${this.monoFont}`;
    ctx.fillStyle = colors.graticuleLabel;
    ctx.textBaseline = "top";
    ctx.textAlign = "center";
    for (let lon = -180; lon <= 180; lon += 30) {
      const sx = (lon - s.cam.cx) * s.cam.scale + s.width / 2;
      if (sx < 30 || sx > s.width - 30) continue;
      const label = lon === 0 ? "0°" : `${Math.abs(lon)}°${lon < 0 ? "W" : "E"}`;
      ctx.fillText(label, sx, 8);
    }
    ctx.textAlign = "left";
    ctx.textBaseline = "middle";
    for (let lat = -60; lat <= 60; lat += 30) {
      const sy = (-lat - s.cam.cy) * s.cam.scale + s.height / 2;
      if (sy < 24 || sy > s.height - 16) continue;
      const label = lat === 0 ? "0°" : `${Math.abs(lat)}°${lat < 0 ? "S" : "N"}`;
      ctx.fillText(label, 8, sy);
    }
  }

  private drawCityGlow(ctx: CanvasRenderingContext2D, s: SceneInput): void {
    const sprite = this.cityGlow;
    if (!sprite) return;
    ctx.setTransform(s.dpr, 0, 0, s.dpr, 0, 0);
    ctx.save();
    ctx.globalCompositeOperation = "lighter";
    const zoom = Math.sqrt(s.cam.scale / s.fitScale);
    for (const c of this.cities) {
      if (c.weight === 0) continue;
      const sx = (c.x - s.cam.cx) * s.cam.scale + s.width / 2;
      const sy = (c.y - s.cam.cy) * s.cam.scale + s.height / 2;
      const r = (26 + Math.sqrt(c.weight) * 9) * zoom;
      if (sx < -r || sy < -r || sx > s.width + r || sy > s.height + r) continue;
      ctx.globalAlpha = Math.min(0.2, 0.06 + c.weight / 600);
      ctx.drawImage(sprite, sx - r, sy - r, r * 2, r * 2);
    }
    ctx.restore();
  }

  private drawLinks(ctx: CanvasRenderingContext2D, s: SceneInput, zoom: number): void {
    const { geo, cur, prev, now } = s;
    const n = geo.ax.length;
    const px = 1 / s.cam.scale;
    const widthScale = Math.min(1.8, Math.max(1, zoom ** 0.35));

    const buckets: (Path2D | null)[] = new Array(UTIL_BUCKETS * 4).fill(null);
    const hot = [new Path2D(), new Path2D(), new Path2D(), new Path2D()];
    const down = new Path2D();
    const undetected = new Path2D();
    const degraded = new Path2D();
    const risky: number[] = [];

    const util = cur?.links.util;
    const prevUtil = prev && prev.links.util.length === n ? prev.links.util : null;
    const state = cur?.links.state;
    const risk = cur?.links.risk;
    const blend = s.blend;
    let anyHot = false;

    for (let i = 0; i < n; i++) {
      const st = state ? state[i] : LinkState.Up;
      if (st === LinkState.Down) {
        addCurve(down, geo, i);
        continue;
      }
      if (st === LinkState.Undetected) {
        addCurve(undetected, geo, i);
        continue;
      }
      if (st === LinkState.Degraded) addCurve(degraded, geo, i);

      const u = util ? (prevUtil ? prevUtil[i] + (util[i] - prevUtil[i]) * blend : util[i]) : 0;
      const w = this.widthOf[i];
      const key = utilBucket(u) * 4 + w;
      let p = buckets[key];
      if (!p) p = buckets[key] = new Path2D();
      addCurve(p, geo, i);
      if (u >= HOT_UTIL) {
        addCurve(hot[w], geo, i);
        anyHot = true;
      }
      if (risk && risk[i] > RISK_GLOW) risky.push(i);
    }

    ctx.save();
    this.worldTransform(ctx, s);
    ctx.beginPath();
    ctx.rect(WORLD.minX, WORLD.minY, WORLD.maxX - WORLD.minX, WORLD.maxY - WORLD.minY);
    ctx.clip();
    ctx.lineCap = "round";

    // Risk halo sits underneath so the link colour still reads through it.
    if (risky.length && risk) {
      ctx.globalCompositeOperation = "lighter";
      for (const i of risky) {
        const intensity = Math.min(1, (risk[i] - RISK_GLOW) / 500);
        const pulse = 0.5 + 0.5 * Math.sin(now / 380 + i * 1.7);
        ctx.strokeStyle = `rgba(167, 123, 255, ${(0.12 + 0.3 * intensity * pulse).toFixed(3)})`;
        ctx.lineWidth = (5 + 5 * intensity * pulse) * px * widthScale;
        const p = new Path2D();
        addCurve(p, geo, i);
        ctx.stroke(p);
      }
      ctx.globalCompositeOperation = "source-over";
    }

    for (let b = 0; b < UTIL_BUCKETS; b++) {
      for (let w = 0; w < 4; w++) {
        const p = buckets[b * 4 + w];
        if (!p) continue;
        ctx.strokeStyle = utilColor[b];
        ctx.lineWidth = WIDTH_PX[w] * widthScale * px;
        ctx.stroke(p);
      }
    }

    if (anyHot) {
      ctx.globalCompositeOperation = "lighter";
      ctx.strokeStyle = "rgba(255, 110, 70, 0.16)";
      for (let w = 0; w < 4; w++) {
        ctx.lineWidth = (WIDTH_PX[w] * 4 + 2) * widthScale * px;
        ctx.stroke(hot[w]);
      }
      ctx.globalCompositeOperation = "source-over";
    }

    ctx.setLineDash([5 * px, 4 * px]);
    ctx.lineDashOffset = (-now / 60) * px;
    ctx.strokeStyle = "rgba(255, 181, 71, 0.95)";
    ctx.lineWidth = 1.4 * widthScale * px;
    ctx.stroke(degraded);

    ctx.lineDashOffset = 0;
    ctx.setLineDash([3 * px, 4 * px]);
    ctx.strokeStyle = "rgba(255, 77, 94, 0.75)";
    ctx.lineWidth = 1.3 * widthScale * px;
    ctx.stroke(down);

    // Undetected failures flash: traffic is still being sent into them.
    const flash = 0.5 + 0.5 * Math.sin(now / 90);
    ctx.setLineDash([]);
    ctx.strokeStyle = `rgba(255, 60, 80, ${(0.35 + 0.65 * flash).toFixed(3)})`;
    ctx.lineWidth = (1.6 + 1.6 * flash) * widthScale * px;
    ctx.stroke(undetected);
    ctx.globalCompositeOperation = "lighter";
    ctx.strokeStyle = `rgba(255, 60, 80, ${(0.18 * flash).toFixed(3)})`;
    ctx.lineWidth = 9 * widthScale * px;
    ctx.stroke(undetected);
    ctx.restore();
  }

  private routerIsDown(cur: Frame | null): Uint8Array {
    if (!cur || cur.seq === this.downSeq) return this.downRouters;
    this.downSeq = cur.seq;
    this.downRouters.fill(0);
    for (const r of cur.routersDown) if (r < this.downRouters.length) this.downRouters[r] = 1;
    return this.downRouters;
  }

  private drawRouters(ctx: CanvasRenderingContext2D, s: SceneInput, zoom: number): void {
    const { geo, index } = s;
    const routers = index.topology.routers;
    const down = this.routerIsDown(s.cur);
    const grow = Math.min(2.6, Math.max(1, zoom ** 0.5));
    const hw = s.width / 2;
    const hh = s.height / 2;
    const toSx = (x: number) => (x - s.cam.cx) * s.cam.scale + hw;
    const toSy = (y: number) => (y - s.cam.cy) * s.cam.scale + hh;

    ctx.setTransform(s.dpr, 0, 0, s.dpr, 0, 0);
    const t1 = new Path2D();
    const t2 = new Path2D();
    const stub = new Path2D();
    const dc = new Path2D();
    const dead = new Path2D();
    const glow = this.tier1Glow;

    ctx.save();
    ctx.globalCompositeOperation = "lighter";
    for (let i = 0; i < routers.length; i++) {
      const sx = toSx(geo.routerX[i]);
      const sy = toSy(geo.routerY[i]);
      if (sx < -10 || sy < -10 || sx > s.width + 10 || sy > s.height + 10) continue;
      const tier = index.routerTier[i];
      if (down[i]) {
        const r = 3.2 * grow;
        dead.moveTo(sx - r, sy - r);
        dead.lineTo(sx + r, sy + r);
        dead.moveTo(sx + r, sy - r);
        dead.lineTo(sx - r, sy + r);
        continue;
      }
      if (routers[i].role === "datacenter") {
        const r = 2.1 * grow;
        dc.rect(sx - r, sy - r, r * 2, r * 2);
        continue;
      }
      if (tier === "tier1") {
        const r = 2.3 * grow;
        if (glow) {
          ctx.globalAlpha = 0.3;
          const g = 16 * grow;
          ctx.drawImage(glow, sx - g / 2, sy - g / 2, g, g);
        }
        t1.moveTo(sx + r, sy);
        t1.arc(sx, sy, r, 0, Math.PI * 2);
      } else if (tier === "tier2") {
        const r = 1.6 * grow;
        t2.moveTo(sx + r, sy);
        t2.arc(sx, sy, r, 0, Math.PI * 2);
      } else {
        const r = 1.1 * grow;
        stub.rect(sx - r, sy - r, r * 2, r * 2);
      }
    }
    ctx.restore();

    ctx.fillStyle = colors.stub;
    ctx.fill(stub);
    ctx.fillStyle = colors.tier2;
    ctx.fill(t2);
    ctx.fillStyle = colors.tier1;
    ctx.fill(t1);
    ctx.lineWidth = 1.1;
    ctx.strokeStyle = colors.datacenter;
    ctx.fillStyle = "rgba(10, 20, 36, 0.9)";
    ctx.fill(dc);
    ctx.stroke(dc);
    ctx.lineWidth = 1.6;
    ctx.strokeStyle = colors.down;
    ctx.stroke(dead);
  }

  private drawTrace(ctx: CanvasRenderingContext2D, s: SceneInput, trace: TraceOverlay): void {
    const { geo } = s;
    const px = 1 / s.cam.scale;
    const path = new Path2D();
    for (let i = 0; i < trace.routers.length - 1; i++) {
      const lid = trace.links[i];
      if (lid !== undefined && lid >= 0) addCurve(path, geo, lid);
      else {
        path.moveTo(geo.routerX[trace.routers[i]], geo.routerY[trace.routers[i]]);
        path.lineTo(geo.routerX[trace.routers[i + 1]], geo.routerY[trace.routers[i + 1]]);
      }
    }
    ctx.save();
    this.worldTransform(ctx, s);
    ctx.lineCap = "round";
    ctx.lineJoin = "round";
    ctx.globalCompositeOperation = "lighter";
    ctx.strokeStyle = trace.ok ? colors.traceGlow : "rgba(255, 77, 94, 0.35)";
    ctx.lineWidth = 9 * px;
    ctx.stroke(path);
    ctx.globalCompositeOperation = "source-over";
    ctx.strokeStyle = trace.ok ? colors.trace : colors.red;
    ctx.lineWidth = 2.2 * px;
    ctx.setLineDash([8 * px, 5 * px]);
    ctx.lineDashOffset = (-s.now / 25) * px;
    ctx.stroke(path);
    ctx.restore();
  }

  private drawHighlights(ctx: CanvasRenderingContext2D, s: SceneInput, zoom: number): void {
    const { geo } = s;
    const px = 1 / s.cam.scale;
    const pulse = 0.5 + 0.5 * Math.sin(s.now / 260);
    const linkHighlight = (id: number, strong: boolean) => {
      const p = new Path2D();
      addCurve(p, geo, id);
      ctx.save();
      this.worldTransform(ctx, s);
      ctx.lineCap = "round";
      ctx.globalCompositeOperation = "lighter";
      ctx.strokeStyle = strong ? "rgba(255,255,255,0.22)" : "rgba(255,255,255,0.12)";
      ctx.lineWidth = (strong ? 8 : 6) * px;
      ctx.stroke(p);
      ctx.globalCompositeOperation = "source-over";
      ctx.strokeStyle = colors.selection;
      ctx.lineWidth = (strong ? 2 : 1.4) * px;
      ctx.stroke(p);
      ctx.restore();
    };
    const ring = (id: number, r: number, alpha: number, color = "255,255,255") => {
      const sx = (geo.routerX[id] - s.cam.cx) * s.cam.scale + s.width / 2;
      const sy = (geo.routerY[id] - s.cam.cy) * s.cam.scale + s.height / 2;
      ctx.setTransform(s.dpr, 0, 0, s.dpr, 0, 0);
      ctx.strokeStyle = `rgba(${color},${alpha})`;
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      ctx.arc(sx, sy, r, 0, Math.PI * 2);
      ctx.stroke();
    };
    const grow = Math.min(2.6, Math.max(1, zoom ** 0.5));

    if (s.selection?.kind === "link") linkHighlight(s.selection.id, true);
    if (s.hover?.kind === "link" && !(s.selection?.kind === "link" && s.selection.id === s.hover.id)) {
      linkHighlight(s.hover.id, false);
    }
    if (s.selection?.kind === "router") {
      ring(s.selection.id, (7 + 3 * pulse) * grow, 0.9);
      ring(s.selection.id, (13 + 5 * pulse) * grow, 0.25 * (1 - pulse));
    }
    if (s.hover?.kind === "router") ring(s.hover.id, 6 * grow, 0.75);
    if (s.trace && s.trace.routers.length) {
      ring(s.trace.routers[0], 7 * grow, 0.95, "110,235,255");
      ring(s.trace.routers[s.trace.routers.length - 1], 9 * grow, 0.95, "110,235,255");
    }
    if (s.traceSrc !== null) {
      ring(s.traceSrc, (8 + 4 * pulse) * grow, 0.95, "110,235,255");
      ring(s.traceSrc, (16 + 8 * pulse) * grow, 0.4 * (1 - pulse), "110,235,255");
    }
  }

  private reserve(x: number, y: number, w: number, h: number): boolean {
    const b = this.labelBoxes;
    for (let i = 0; i < b.length; i += 4) {
      if (x < b[i] + b[i + 2] && x + w > b[i] && y < b[i + 1] + b[i + 3] && y + h > b[i + 1]) return false;
    }
    b.push(x, y, w, h);
    return true;
  }

  private drawRiskTags(ctx: CanvasRenderingContext2D, s: SceneInput): void {
    const top = s.cur?.ai.predictor.top ?? [];
    const state = s.cur?.links.state;
    ctx.font = `600 10px ${this.monoFont}`;
    ctx.textBaseline = "middle";
    ctx.textAlign = "left";
    let shown = 0;
    for (const { link, risk } of top) {
      if (shown >= 5 || risk * 1000 < RISK_GLOW) break;
      if (link < 0 || link >= s.geo.midX.length) continue;
      if (state && state[link] !== LinkState.Up && state[link] !== LinkState.Degraded) continue;
      const sx = (s.geo.midX[link] - s.cam.cx) * s.cam.scale + s.width / 2;
      const sy = (s.geo.midY[link] - s.cam.cy) * s.cam.scale + s.height / 2;
      if (sx < 0 || sy < 0 || sx > s.width || sy > s.height) continue;
      const text = `L${link}  ${Math.round(risk * 100)}%`;
      const w = ctx.measureText(text).width + 14;
      const x = sx + 8;
      const y = sy - 22;
      if (!this.reserve(x, y - 8, w, 16)) continue;
      shown++;
      ctx.strokeStyle = "rgba(167, 123, 255, 0.6)";
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(sx, sy);
      ctx.lineTo(x, y);
      ctx.stroke();
      ctx.fillStyle = "rgba(26, 16, 52, 0.92)";
      roundRect(ctx, x, y - 8, w, 16, 3);
      ctx.fill();
      ctx.stroke();
      ctx.fillStyle = "#d9c8ff";
      ctx.fillText(text, x + 7, y + 0.5);
      ctx.fillStyle = colors.violet;
      ctx.beginPath();
      ctx.arc(sx, sy, 2.5, 0, Math.PI * 2);
      ctx.fill();
    }
  }

  private drawTraceLabels(ctx: CanvasRenderingContext2D, s: SceneInput, trace: TraceOverlay): void {
    ctx.font = `600 9px ${this.monoFont}`;
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    let lastX = -1e9;
    let lastY = -1e9;
    trace.routers.forEach((rid, i) => {
      const sx = (s.geo.routerX[rid] - s.cam.cx) * s.cam.scale + s.width / 2;
      const sy = (s.geo.routerY[rid] - s.cam.cy) * s.cam.scale + s.height / 2;
      const last = i === trace.routers.length - 1;
      // Hops inside one city overlap at world zoom; label only the ones that separate.
      if (!last && i > 0 && Math.hypot(sx - lastX, sy - lastY) < 18) return;
      lastX = sx;
      lastY = sy;
      ctx.fillStyle = "rgba(6, 14, 26, 0.95)";
      ctx.strokeStyle = trace.ok ? "rgba(110, 235, 255, 0.9)" : colors.red;
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.arc(sx, sy - 13, 7.5, 0, Math.PI * 2);
      ctx.fill();
      ctx.stroke();
      ctx.fillStyle = "#e8fbff";
      ctx.fillText(String(i + 1), sx, sy - 12.5);
      this.labelBoxes.push(sx - 8, sy - 21, 16, 16);
    });
  }

  private drawCityLabels(ctx: CanvasRenderingContext2D, s: SceneInput): void {
    const zoom = s.cam.scale / s.fitScale;
    const limit = Math.round(18 * zoom * zoom);
    ctx.font = `500 9.5px ${this.sansFont}`;
    ctx.textAlign = "left";
    ctx.textBaseline = "middle";
    ctx.letterSpacing = "1.2px";
    let shown = 0;
    for (const c of this.cities) {
      if (shown >= limit || c.weight === 0) break;
      const sx = (c.x - s.cam.cx) * s.cam.scale + s.width / 2;
      const sy = (c.y - s.cam.cy) * s.cam.scale + s.height / 2;
      if (sx < 0 || sy < 0 || sx > s.width - 60 || sy > s.height) continue;
      const text = c.name.toUpperCase();
      const w = ctx.measureText(text).width;
      const x = sx + 9;
      const y = sy + 9;
      if (!this.reserve(x - 2, y - 7, w + 4, 14)) continue;
      shown++;
      ctx.fillStyle = "rgba(4, 9, 18, 0.55)";
      ctx.fillRect(x - 2, y - 6, w + 4, 12);
      ctx.fillStyle = colors.cityLabel;
      ctx.fillText(text, x, y);
    }
    ctx.letterSpacing = "0px";
  }
}

function addCurve(p: Path2D, geo: MapGeometry, i: number): void {
  const straight = geo.ptCount[i] <= 2;
  p.moveTo(geo.ax[i], geo.ay[i]);
  if (straight) p.lineTo(geo.bx[i], geo.by[i]);
  else p.quadraticCurveTo(geo.cx[i], geo.cy[i], geo.bx[i], geo.by[i]);
  const off = geo.wrap[i];
  if (off) {
    p.moveTo(geo.ax[i] + off, geo.ay[i]);
    if (straight) p.lineTo(geo.bx[i] + off, geo.by[i]);
    else p.quadraticCurveTo(geo.cx[i] + off, geo.cy[i], geo.bx[i] + off, geo.by[i]);
  }
}

function roundRect(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number): void {
  ctx.beginPath();
  ctx.roundRect(x, y, w, h, r);
}
