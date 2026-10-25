"use client";

import { clearTrace } from "@/lib/actions";
import { fixed, ms } from "@/lib/format";
import { focusOn, useApp } from "@/lib/state";
import ui from "../ui/controls.module.css";
import styles from "./TracePanel.module.css";

const STATUS_CHIP = {
  ok: ui.chipCyan,
  unreachable: ui.chipRed,
  loop: ui.chipAmber,
  blackhole: ui.chipRed,
} as const;

export function TracePanel() {
  const trace = useApp((s) => s.trace);
  const topology = useApp((s) => s.topology);
  if (!trace || !topology) return null;
  const src = topology.routers[trace.src];
  const dst = trace.dst !== null ? topology.routers[trace.dst] : null;
  const result = trace.result;
  const hops = result?.hops ?? [];
  const total = hops.length ? hops[hops.length - 1].cumMs : 0;

  return (
    <aside className={styles.panel}>
      <header className={styles.head}>
        <span className={styles.label}>traceroute</span>
        {result && <span className={`${ui.chip} ${STATUS_CHIP[result.status]}`}>{result.status}</span>}
        {trace.loading && <span className={`${ui.chip}`}>tracing…</span>}
        <button type="button" className={styles.close} onClick={clearTrace} aria-label="Clear trace">
          ×
        </button>
      </header>
      <div className={`${styles.route} mono`}>
        <span>{src?.name}</span>
        <span className={styles.to}>→</span>
        <span className={dst ? "" : styles.pending}>{dst?.name ?? "pick destination"}</span>
      </div>

      {trace.error && <div className={styles.error}>{trace.error}</div>}

      {result && (
        <>
          <div className={styles.summary}>
            <div>
              <span className={styles.sumLabel}>hops</span>
              <span className="mono">{hops.length}</span>
            </div>
            <div>
              <span className={styles.sumLabel}>one-way</span>
              <span className="mono">{ms(total)}</span>
            </div>
            <div>
              <span className={styles.sumLabel}>AS path</span>
              <span className={`${styles.asPath} mono`}>
                {result.asPath.map((a, i) => (
                  <span key={i}>
                    {i > 0 && <i>›</i>}
                    {a}
                  </span>
                ))}
              </span>
            </div>
          </div>
          <ol className={`${styles.hops} mono`}>
            {hops.map((h, i) => {
              const prevAs = i > 0 ? hops[i - 1].asn : h.asn;
              return (
                <li key={i} className={h.asn !== prevAs ? styles.asBoundary : ""}>
                  <button type="button" className={styles.hop} onClick={() => focusOn("router", h.router)}>
                    <span className={styles.n}>{String(i + 1).padStart(2, " ")}</span>
                    <span className={styles.hopName}>{h.name}</span>
                    <span className={styles.asn}>AS{h.asn}</span>
                    <span className={styles.ms}>
                      {fixed(h.cumMs, h.cumMs < 10 ? 2 : 1)}
                      <small>ms</small>
                    </span>
                    <span className={styles.msBar}>
                      <span style={{ width: `${total > 0 ? (h.cumMs / total) * 100 : 0}%` }} />
                    </span>
                  </button>
                  {h.queueMs > 0.5 && <span className={styles.queue}>queue +{fixed(h.queueMs, 1)} ms</span>}
                </li>
              );
            })}
          </ol>
        </>
      )}
    </aside>
  );
}
