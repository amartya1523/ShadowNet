"use client";

import { fixed, int } from "@/lib/format";
import type { Incident } from "@/lib/protocol";
import { useApp } from "@/lib/state";
import { Empty, Panel } from "../ui/Panel";
import ui from "../ui/controls.module.css";
import styles from "./Incidents.module.css";

/** Convergence times are shown in ms below 10 s, seconds above. */
function convergence(msValue: number): [string, string] {
  if (!Number.isFinite(msValue) || msValue <= 0) return ["–", ""];
  if (msValue < 10_000) return [int(msValue), "ms"];
  return [fixed(msValue / 1000, 1), "s"];
}

export function Incidents() {
  const incidents = useApp((s) => s.frame?.incidents ?? null);
  const t = useApp((s) => s.frame?.t ?? 0);
  const open = incidents?.filter((i) => i.open).length ?? 0;

  return (
    <Panel
      code="05"
      title="Incidents"
      className={styles.panel}
      bodyClassName={styles.body}
      meta={
        open > 0 ? (
          <span className={`${ui.chip} ${ui.chipRed} ${styles.openChip}`}>{open} open</span>
        ) : incidents ? (
          <span>all converged</span>
        ) : undefined
      }
    >
      {!incidents ? (
        <Empty>connecting to shadownetd…</Empty>
      ) : incidents.length === 0 ? (
        <Empty>no incidents this run. break something</Empty>
      ) : (
        <>
          <Hero incident={incidents[0]} now={t} />
          {incidents.length > 1 && (
            <ul className={styles.list}>
              {incidents.slice(1).map((inc) => (
                <Row key={inc.id} incident={inc} />
              ))}
            </ul>
          )}
        </>
      )}
    </Panel>
  );
}

function Hero({ incident: inc, now }: { incident: Incident; now: number }) {
  const [igp, igpUnit] = convergence(inc.igpMs);
  const [bgp, bgpUnit] = convergence(inc.bgpMs);
  return (
    <div className={`${styles.hero} ${inc.open ? styles.heroOpen : ""}`}>
      <div className={styles.heroHead}>
        <span className={`${styles.id} mono`}>#{inc.id}</span>
        <span className={styles.cause}>{inc.cause}</span>
        <span className={`${ui.chip} ${inc.open ? ui.chipRed : ui.chipCyan}`}>
          {inc.open ? "converging" : "converged"}
        </span>
      </div>
      <div className={styles.metrics}>
        <div className={styles.metric}>
          <span className={styles.metricLabel}>IGP converged</span>
          <span className={`${styles.big} mono`}>
            {inc.open && inc.igpMs <= 0 ? "…" : igp}
            <small>{igpUnit}</small>
          </span>
        </div>
        <div className={styles.metric}>
          <span className={styles.metricLabel}>BGP converged</span>
          {inc.bgpMs <= 0 && !inc.open ? (
            <span className={`${styles.big} ${styles.na} mono`}>
              n/a<small>intra-AS</small>
            </span>
          ) : (
            <span className={`${styles.big} ${styles.bgp} mono`}>
              {inc.bgpMs <= 0 ? "…" : bgp}
              <small>{bgpUnit}</small>
            </span>
          )}
        </div>
      </div>
      <div className={`${styles.footer} mono`}>
        <span>
          peak loss <b className={inc.peakLossPct >= 5 ? styles.bad : styles.warn}>{fixed(inc.peakLossPct, 1)}%</b>
        </span>
        <span>
          blackholed <b>{fixed(inc.blackholedFlowSecs, 1)}</b> flow·s
        </span>
        <span>
          t0 <b>{fixed(inc.t0, 1)}s</b>
          {inc.open && <> · {fixed(Math.max(0, now - inc.t0), 1)}s ago</>}
        </span>
      </div>
    </div>
  );
}

function Row({ incident: inc }: { incident: Incident }) {
  const [igp, igpUnit] = convergence(inc.igpMs);
  const [bgp, bgpUnit] = convergence(inc.bgpMs);
  return (
    <li className={`${styles.row} mono`}>
      <span className={`${styles.statusDot} ${inc.open ? styles.dotOpen : ""}`} />
      <span className={styles.rowId}>#{inc.id}</span>
      <span className={styles.rowCause}>{inc.cause}</span>
      <span className={styles.rowNum} title="IGP convergence">
        {igp}
        <small>{igpUnit}</small>
      </span>
      <span className={`${styles.rowNum} ${styles.bgp}`} title="BGP convergence">
        {bgp}
        <small>{bgpUnit}</small>
      </span>
      <span className={styles.rowLoss}>{fixed(inc.peakLossPct, 1)}%</span>
    </li>
  );
}
