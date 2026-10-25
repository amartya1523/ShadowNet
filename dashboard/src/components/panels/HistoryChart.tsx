"use client";

import { useElementSize } from "@/hooks/useElementSize";
import { int } from "@/lib/format";
import { useApp } from "@/lib/state";
import { Empty, Panel } from "../ui/Panel";
import styles from "./HistoryChart.module.css";

const HEIGHT = 132;
const PAD = { top: 10, right: 34, bottom: 18, left: 34 };
const MIN_WINDOW = 60;

const Y_TICKS = 4;

/** Smallest round axis maximum whose quarter steps are also round. */
function niceMax(v: number): number {
  const raw = Math.max(v, 1e-6) / Y_TICKS;
  const mag = 10 ** Math.floor(Math.log10(raw));
  for (const m of [1, 2, 2.5, 5, 10]) if (raw <= m * mag) return m * mag * Y_TICKS;
  return 10 * mag * Y_TICKS;
}

function clock(t: number): string {
  // The window can reach back before the run started; label that as zero
  // rather than printing negative minutes.
  t = Math.max(0, t);
  const m = Math.floor(t / 60);
  const s = Math.floor(t % 60);
  return `${m}:${String(s).padStart(2, "0")}`;
}

export function HistoryChart() {
  const series = useApp((s) => s.series);
  const incidents = useApp((s) => s.frame?.incidents ?? null);
  const [ref, { width }] = useElementSize<HTMLDivElement>();

  const n = series.t.length;
  const lastDelivered = n ? series.deliveredGbps[n - 1] : null;
  const lastP99 = n ? series.p99Ms[n - 1] : null;

  return (
    <Panel
      code="04"
      title="History"
      meta={
        <span className={styles.legend}>
          <i className={styles.keyDelivered} />
          Gbps
          <i className={styles.keyP99} />
          p99 ms
        </span>
      }
    >
      <div ref={ref} className={styles.wrap}>
        {n < 2 || width === 0 ? (
          <Empty>collecting samples…</Empty>
        ) : (
          <Chart
            width={width}
            series={series}
            incidents={incidents ?? []}
            lastDelivered={lastDelivered ?? 0}
            lastP99={lastP99 ?? 0}
          />
        )}
      </div>
    </Panel>
  );
}

const LABEL_GAP_PX = 26;

/** Incident markers inside the window; labels are skipped where they would collide. */
function incidentMarks(
  incidents: ChartProps["incidents"],
  t0: number,
  t1: number,
  x: (v: number) => number,
) {
  let lastLabel = -Infinity;
  return incidents
    .filter((inc) => inc.t0 >= t0 && inc.t0 <= t1)
    .sort((a, b) => a.t0 - b.t0)
    .map((inc) => {
      const px = x(inc.t0);
      const labelled = px - lastLabel >= LABEL_GAP_PX;
      if (labelled) lastLabel = px;
      return { id: inc.id, open: inc.open, px, labelled };
    });
}

interface ChartProps {
  width: number;
  series: { t: number[]; deliveredGbps: number[]; p99Ms: number[] };
  incidents: { id: number; t0: number; open: boolean }[];
  lastDelivered: number;
  lastP99: number;
}

function Chart({ width, series, incidents, lastDelivered, lastP99 }: ChartProps) {
  const { t, deliveredGbps, p99Ms } = series;
  const t1 = t[t.length - 1];
  const t0 = Math.min(t[0], t1 - MIN_WINDOW);
  const gMax = niceMax(Math.max(...deliveredGbps) * 1.1);
  const pMax = niceMax(Math.max(...p99Ms) * 1.1);
  const iw = width - PAD.left - PAD.right;
  const ih = HEIGHT - PAD.top - PAD.bottom;
  const x = (v: number) => PAD.left + ((v - t0) / (t1 - t0)) * iw;
  const yG = (v: number) => PAD.top + (1 - v / gMax) * ih;
  const yP = (v: number) => PAD.top + (1 - v / pMax) * ih;

  let gLine = "";
  let pLine = "";
  for (let i = 0; i < t.length; i++) {
    const cmd = i ? "L" : "M";
    gLine += `${cmd}${x(t[i]).toFixed(1)},${yG(deliveredGbps[i]).toFixed(1)}`;
    pLine += `${cmd}${x(t[i]).toFixed(1)},${yP(p99Ms[i]).toFixed(1)}`;
  }
  const gArea = `${gLine}L${x(t1).toFixed(1)},${PAD.top + ih}L${x(t[0]).toFixed(1)},${PAD.top + ih}Z`;
  const ticks = Array.from({ length: Y_TICKS + 1 }, (_, i) => i / Y_TICKS);
  const span = t1 - t0;
  const xTicks = [0, 0.25, 0.5, 0.75, 1].map((f) => t0 + f * span);

  return (
    <svg width={width} height={HEIGHT} className={styles.svg}>
      <defs>
        <linearGradient id="hist-delivered" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="var(--cyan)" stopOpacity="0.32" />
          <stop offset="1" stopColor="var(--cyan)" stopOpacity="0" />
        </linearGradient>
      </defs>
      {ticks.map((f) => {
        const y = PAD.top + (1 - f) * ih;
        return (
          <g key={f}>
            <line x1={PAD.left} x2={PAD.left + iw} y1={y} y2={y} className={styles.grid} />
            <text x={PAD.left - 6} y={y + 3} className={styles.axisLeft}>
              {int(gMax * f)}
            </text>
            <text x={PAD.left + iw + 6} y={y + 3} className={styles.axisRight}>
              {int(pMax * f)}
            </text>
          </g>
        );
      })}
      {xTicks.map((v, i) => (
        <text
          key={i}
          x={x(v)}
          y={HEIGHT - 4}
          className={styles.axisX}
          textAnchor={i === 0 ? "start" : i === xTicks.length - 1 ? "end" : "middle"}
        >
          {clock(v)}
        </text>
      ))}
      {incidentMarks(incidents, t0, t1, x).map(({ id, open, px, labelled }) => (
        <g key={id}>
          <line x1={px} x2={px} y1={PAD.top} y2={PAD.top + ih} className={open ? styles.incidentOpen : styles.incident} />
          {labelled && (
            <text x={px + 3} y={PAD.top + 8} className={styles.incidentLabel}>
              #{id}
            </text>
          )}
        </g>
      ))}
      <path d={gArea} fill="url(#hist-delivered)" />
      <path d={gLine} className={styles.delivered} />
      <path d={pLine} className={styles.p99} />
      <circle cx={x(t1)} cy={yG(lastDelivered)} r={2.5} className={styles.dotDelivered} />
      <circle cx={x(t1)} cy={yP(lastP99)} r={2.5} className={styles.dotP99} />
    </svg>
  );
}
