const intFmt = new Intl.NumberFormat("en-US", { maximumFractionDigits: 0 });

export function int(n: number): string {
  return Number.isFinite(n) ? intFmt.format(Math.round(n)) : "–";
}

export function fixed(n: number, digits = 1): string {
  if (!Number.isFinite(n)) return "–";
  return n.toLocaleString("en-US", {
    minimumFractionDigits: digits,
    maximumFractionDigits: digits,
  });
}

/** Latency: seconds above 1 s, whole ms above 10, one decimal below, µs under 0.1. */
export function ms(n: number): string {
  if (!Number.isFinite(n)) return "–";
  if (n >= 1000) return `${fixed(n / 1000, 2)} s`;
  if (n >= 10) return `${int(n)} ms`;
  if (n >= 0.1 || n === 0) return `${fixed(n, n === 0 ? 0 : 1)} ms`;
  return `${int(n * 1000)} µs`;
}

export function gbps(n: number): string {
  if (!Number.isFinite(n)) return "–";
  if (n >= 1000) return `${fixed(n / 1000, 2)} Tbps`;
  if (n >= 1) return `${fixed(n, 1)} Gbps`;
  return `${int(n * 1000)} Mbps`;
}

export function capacity(gbpsValue: number): string {
  return gbpsValue >= 1000 ? `${fixed(gbpsValue / 1000, 1)}T` : `${int(gbpsValue)}G`;
}

export function permille(n: number, digits = 0): string {
  return `${fixed(n / 10, digits)}%`;
}

/** 2.1e6 -> "2.1M" */
export function compact(n: number): string {
  if (!Number.isFinite(n)) return "–";
  const abs = Math.abs(n);
  if (abs >= 1e9) return `${fixed(n / 1e9, 1)}B`;
  if (abs >= 1e6) return `${fixed(n / 1e6, 1)}M`;
  if (abs >= 1e4) return `${fixed(n / 1e3, 1)}k`;
  return int(n);
}

/** Sim clock as T+mm:ss.s, or h:mm:ss once past an hour. */
export function simClock(t: number): string {
  if (!Number.isFinite(t)) return "--:--.-";
  const total = Math.max(0, t);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const ss = s.toFixed(1).padStart(4, "0");
  const mm = String(m).padStart(2, "0");
  return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
}
