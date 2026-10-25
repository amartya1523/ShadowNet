"use client";

import { compact, fixed, int } from "@/lib/format";
import { focusOn, useApp } from "@/lib/state";
import { Empty, Panel } from "../ui/Panel";
import ui from "../ui/controls.module.css";
import styles from "./AiPanel.module.css";

const TOP_ROWS = 6;

export function AiPanel() {
  const ai = useApp((s) => s.frame?.ai ?? null);
  const routing = useApp((s) => s.frame?.routing ?? null);
  const index = useApp((s) => s.index);
  const selected = useApp((s) => (s.selection?.kind === "link" ? s.selection.id : null));

  if (!ai || !index) {
    return (
      <Panel code="06" title="AI · failure prediction" className={styles.panel}>
        <Empty>predictor offline</Empty>
      </Panel>
    );
  }

  const p = ai.predictor;
  const lift = p.auc - p.baselineAuc;
  const topo = index.topology;

  return (
    <Panel
      code="06"
      title="AI · failure prediction"
      className={styles.panel}
      meta={<span className={`${ui.chip} ${ui.chipViolet}`}>online LR</span>}
    >
      <div className={styles.aucBlock}>
        <AucBar label="Predictor" value={p.auc} tone="ai" />
        <AucBar label="Util-only baseline" value={p.baselineAuc} tone="base" />
        <div className={`${styles.aucScale} mono`}>
          <span className={styles.aucScaleTrack}>
            <span>0.5</span>
            <span>ROC-AUC</span>
            <span>1.0</span>
          </span>
        </div>
      </div>

      <div className={styles.stats}>
        <Stat label="Lift" value={`${lift >= 0 ? "+" : ""}${fixed(lift, 2)}`} accent />
        <Stat label="Prec@10" value={fixed(p.precisionAt10, 2)} />
        <Stat label="Labelled" value={compact(p.labelled)} />
        <Stat label="Positives" value={int(p.positives)} />
      </div>

      <div className={styles.sectionLabel}>
        <span>Highest risk links</span>
        <span className="mono">P(fail ≤ H)</span>
      </div>
      {p.top.length === 0 ? (
        <Empty>no link above threshold</Empty>
      ) : (
        <ul className={styles.top}>
          {p.top.slice(0, TOP_ROWS).map(({ link, risk }) => {
            const l = topo.links[link];
            if (!l) return null;
            const a = topo.routers[l.a];
            const b = topo.routers[l.b];
            return (
              <li key={link}>
                <button
                  type="button"
                  className={`${styles.topRow} ${selected === link ? styles.topRowActive : ""}`}
                  onClick={() => focusOn("link", link)}
                  title={`${a.name} ↔ ${b.name}`}
                >
                  <span className={`${styles.linkId} mono`}>L{link}</span>
                  <span className={`${styles.ends} mono`}>
                    {a.city}
                    <span className={styles.arrow}>↔</span>
                    {b.city}
                  </span>
                  <span className={styles.riskBar}>
                    <span style={{ width: `${Math.min(100, risk * 100)}%` }} />
                  </span>
                  <span className={`${styles.riskValue} mono`}>{int(risk * 100)}%</span>
                </button>
              </li>
            );
          })}
        </ul>
      )}

      <div className={`${styles.qr} ${routing === "qrouting" ? styles.qrActive : ""}`}>
        <div className={styles.qrHead}>
          <span>Q-routing</span>
          <span className={`${ui.chip} ${routing === "qrouting" ? ui.chipViolet : ""}`}>
            {routing === "qrouting" ? "in control" : "shadow"}
          </span>
        </div>
        <div className={styles.qrStats}>
          <Stat label="Q updates/s" value={compact(ai.qrouting.updatesPerSec)} />
          <Stat label="Mean |TD|" value={`${fixed(ai.qrouting.meanAbsTdMs, 2)} ms`} />
        </div>
      </div>
    </Panel>
  );
}

function AucBar({ label, value, tone }: { label: string; value: number; tone: "ai" | "base" }) {
  // Scale from chance (0.5) to perfect (1.0): that is the range that means anything.
  const pctWidth = Math.max(0, Math.min(1, (value - 0.5) / 0.5)) * 100;
  return (
    <div className={styles.aucRow}>
      <span className={styles.aucLabel}>{label}</span>
      <span className={styles.aucTrack}>
        <span className={tone === "ai" ? styles.aucFillAi : styles.aucFillBase} style={{ width: `${pctWidth}%` }} />
      </span>
      <span className={`${styles.aucValue} ${tone === "ai" ? styles.aucValueAi : ""} mono`}>{fixed(value, 3)}</span>
    </div>
  );
}

function Stat({ label, value, accent }: { label: string; value: string; accent?: boolean }) {
  return (
    <div className={styles.stat}>
      <span className={styles.statLabel}>{label}</span>
      <span className={`${styles.statValue} ${accent ? styles.statAccent : ""} mono`}>{value}</span>
    </div>
  );
}
