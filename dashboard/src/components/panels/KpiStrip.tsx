"use client";

import { fixed, gbps, int, ms } from "@/lib/format";
import { useApp } from "@/lib/state";
import { Sparkline } from "../ui/Sparkline";
import styles from "./KpiStrip.module.css";

type Tone = "ok" | "warn" | "bad" | "neutral";

const toneColor: Record<Tone, string> = {
  ok: "var(--cyan)",
  warn: "var(--amber)",
  bad: "var(--red)",
  neutral: "var(--text-2)",
};

interface TileProps {
  label: string;
  value: string;
  unit?: string;
  /** Secondary figure shown opposite the label. */
  note?: string;
  tone: Tone;
  series: number[];
  min?: number;
}

function Tile({ label, value, unit, note, tone, series, min }: TileProps) {
  return (
    <div className={`${styles.tile} ${styles[tone]}`}>
      <div className={styles.labelRow}>
        <span className={styles.label}>{label}</span>
        {note && <span className={`${styles.note} mono`}>{note}</span>}
      </div>
      <div className={styles.valueRow}>
        <span className={`${styles.value} mono`}>{value}</span>
        {unit && <span className={`${styles.unit} mono`}>{unit}</span>}
      </div>
      <Sparkline className={styles.spark} values={series} color={toneColor[tone]} min={min} height={24} />
    </div>
  );
}

function splitUnit(formatted: string): [string, string] {
  const i = formatted.lastIndexOf(" ");
  return i < 0 ? [formatted, ""] : [formatted.slice(0, i), formatted.slice(i + 1)];
}

export function KpiStrip() {
  const frame = useApp((s) => s.frame);
  const kpi = useApp((s) => s.kpi);

  if (!frame) {
    return (
      <div className={styles.strip}>
        {["Delivered", "Loss", "Latency p50", "Latency p99", "Active flows", "Links down", "Congested"].map((l) => (
          <div key={l} className={`${styles.tile} ${styles.neutral}`}>
            <div className={styles.labelRow}>
              <span className={styles.label}>{l}</span>
            </div>
            <div className={styles.valueRow}>
              <span className={`${styles.value} ${styles.placeholder} mono`}>– –</span>
            </div>
            <div className={styles.sparkEmpty} />
          </div>
        ))}
      </div>
    );
  }

  const s = frame.stats;
  const [dv, du] = splitUnit(gbps(s.deliveredGbps));
  const [p50v, p50u] = splitUnit(ms(s.latencyMs.p50));
  const [p99v, p99u] = splitUnit(ms(s.latencyMs.p99));
  const lossTone: Tone = s.lossPct >= 5 ? "bad" : s.lossPct >= 1 ? "warn" : "ok";
  const p99Tone: Tone = s.latencyMs.p99 >= 400 ? "bad" : s.latencyMs.p99 >= 220 ? "warn" : "ok";
  const deficit = s.offeredGbps > 0 ? (1 - s.deliveredGbps / s.offeredGbps) * 100 : 0;

  return (
    <div className={styles.strip}>
      <Tile
        label="Delivered"
        value={dv}
        unit={du}
        note={`of ${fixed(s.offeredGbps, 1)}`}
        tone={deficit > 5 ? "warn" : "ok"}
        series={kpi.deliveredGbps}
      />
      <Tile label="Loss" value={fixed(s.lossPct, 2)} unit="%" tone={lossTone} series={kpi.lossPct} min={0} />
      <Tile label="Latency p50" value={p50v} unit={p50u} tone="ok" series={kpi.p50} />
      <Tile
        label="Latency p99"
        value={p99v}
        unit={p99u}
        note={`p95 ${int(s.latencyMs.p95)}`}
        tone={p99Tone}
        series={kpi.p99}
      />
      <Tile
        label="Active flows"
        value={int(s.flows.active)}
        note={s.flows.blackholed + s.flows.looping > 0 ? `${int(s.flows.blackholed + s.flows.looping)} lost` : undefined}
        tone={s.flows.blackholed + s.flows.looping + s.flows.unreachable > 0 ? "warn" : "ok"}
        series={kpi.activeFlows}
      />
      <Tile
        label="Links down"
        value={int(s.linksDown)}
        note={s.routersDown > 0 ? `${int(s.routersDown)} routers` : undefined}
        tone={s.linksDown + s.routersDown > 0 ? "bad" : "ok"}
        series={kpi.linksDown}
        min={0}
      />
      <Tile
        label="Congested"
        value={int(s.congestedLinks)}
        note={`peak ${int(s.maxUtil * 100)}%`}
        tone={s.congestedLinks >= 10 ? "bad" : s.congestedLinks > 0 ? "warn" : "ok"}
        series={kpi.congestedLinks}
        min={0}
      />
    </div>
  );
}
