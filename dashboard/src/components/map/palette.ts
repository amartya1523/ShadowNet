// Canvas colours. These mirror the CSS custom properties in globals.css;
// canvas cannot read CSS variables cheaply every frame, so they are copied.

export const colors = {
  graticule: "rgba(90, 140, 190, 0.07)",
  graticuleMajor: "rgba(90, 160, 210, 0.14)",
  graticuleLabel: "rgba(120, 165, 205, 0.38)",
  cityLabel: "rgba(160, 196, 226, 0.5)",
  cityGlow: [70, 170, 230] as const,
  tier1: "#e9fbff",
  tier2: "#5fd4e0",
  stub: "#4a6c92",
  datacenter: "#f1f5ff",
  down: "#ff4d5e",
  amber: "#ffb547",
  red: "#ff4d5e",
  violet: "#a77bff",
  trace: "#f2fbff",
  traceGlow: "rgba(110, 235, 255, 0.35)",
  selection: "#ffffff",
};

/** RGB for a packet head, by flow kind. */
export const packetRgb = {
  bg: [150, 240, 255],
  crowd: [255, 190, 80],
  attack: [255, 70, 150],
} as const;

const UTIL_STOPS: [number, number, number, number, number][] = [
  // permille, r, g, b, alpha
  [0, 40, 100, 140, 0.16],
  [250, 38, 160, 195, 0.32],
  [500, 60, 215, 230, 0.6],
  [650, 120, 240, 225, 0.8],
  [750, 255, 196, 80, 0.85],
  [900, 255, 140, 60, 0.92],
  [1000, 255, 70, 85, 1],
];

export const UTIL_BUCKETS = 48;
const UTIL_MAX = 1050;

function lerpStops(p: number): [number, number, number, number] {
  for (let i = 1; i < UTIL_STOPS.length; i++) {
    const [p1, r1, g1, b1, a1] = UTIL_STOPS[i];
    const [p0, r0, g0, b0, a0] = UTIL_STOPS[i - 1];
    if (p <= p1) {
      const t = (p - p0) / (p1 - p0);
      return [r0 + (r1 - r0) * t, g0 + (g1 - g0) * t, b0 + (b1 - b0) * t, a0 + (a1 - a0) * t];
    }
  }
  const last = UTIL_STOPS[UTIL_STOPS.length - 1];
  return [last[1], last[2], last[3], last[4]];
}

/** Pre-built `rgba()` strings so the hot loop never formats colours. */
export const utilColor: string[] = Array.from({ length: UTIL_BUCKETS }, (_, i) => {
  const [r, g, b, a] = lerpStops((i / (UTIL_BUCKETS - 1)) * UTIL_MAX);
  return `rgba(${r | 0}, ${g | 0}, ${b | 0}, ${a.toFixed(3)})`;
});

export function utilBucket(permille: number): number {
  const i = Math.round((Math.max(0, permille) / UTIL_MAX) * (UTIL_BUCKETS - 1));
  return Math.min(UTIL_BUCKETS - 1, i);
}

/** CSS colour for a util value outside the canvas (tooltips, inspector bars). */
export function utilCss(permille: number): string {
  if (permille >= 900) return "var(--red)";
  if (permille >= 700) return "var(--amber)";
  return "var(--cyan)";
}

/** Soft round sprite used for glows; drawImage is far cheaper than shadowBlur. */
export function makeGlowSprite(rgb: readonly number[], size = 64): HTMLCanvasElement {
  const c = document.createElement("canvas");
  c.width = c.height = size;
  const ctx = c.getContext("2d");
  if (!ctx) return c;
  const g = ctx.createRadialGradient(size / 2, size / 2, 0, size / 2, size / 2, size / 2);
  const [r, gg, b] = rgb;
  g.addColorStop(0, `rgba(255,255,255,1)`);
  g.addColorStop(0.12, `rgba(${r},${gg},${b},0.95)`);
  g.addColorStop(0.35, `rgba(${r},${gg},${b},0.35)`);
  g.addColorStop(1, `rgba(${r},${gg},${b},0)`);
  ctx.fillStyle = g;
  ctx.fillRect(0, 0, size, size);
  return c;
}
