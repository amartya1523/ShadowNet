"use client";

import { capacity, fixed, int, ms, permille } from "@/lib/format";
import { LinkState } from "@/lib/protocol";
import { useApp } from "@/lib/state";
import type { TopoIndex } from "@/lib/topology";
import { utilCss } from "./palette";
import type { MapTarget } from "./renderer";
import styles from "./HoverCard.module.css";

const STATE_LABEL = ["up", "down", "degraded", "down · undetected"];

export function HoverCard({ target, index }: { target: MapTarget; index: TopoIndex }) {
  return target.kind === "router" ? <RouterCard id={target.id} index={index} /> : <LinkCard id={target.id} index={index} />;
}

function RouterCard({ id, index }: { id: number; index: TopoIndex }) {
  const links = useApp((s) => s.frame?.links ?? null);
  const down = useApp((s) => s.frame?.routersDown.includes(id) ?? false);
  const r = index.topology.routers[id];
  if (!r) return null;
  const as = index.asOfRouter(id);
  const adj = index.adjacency[id];

  let maxUtil = 0;
  let maxQueue = 0;
  let maxLoss = 0;
  let maxRisk = 0;
  let upLinks = 0;
  if (links) {
    for (const l of adj) {
      maxUtil = Math.max(maxUtil, links.util[l] ?? 0);
      maxQueue = Math.max(maxQueue, links.queueMs[l] ?? 0);
      maxLoss = Math.max(maxLoss, links.loss[l] ?? 0);
      maxRisk = Math.max(maxRisk, links.risk[l] ?? 0);
      if (links.state[l] === LinkState.Up || links.state[l] === LinkState.Degraded) upLinks++;
    }
  }

  return (
    <div className={styles.card}>
      <div className={styles.kicker}>
        <span className={`${styles.tier} ${styles[as.tier]}`}>{as.tier}</span>
        <span>{r.role}</span>
        {down && <span className={styles.down}>down</span>}
      </div>
      <div className={`${styles.name} mono`}>{r.name}</div>
      <div className={styles.sub}>
        {as.name} · <span className="mono">AS{as.asn}</span>
      </div>
      <div className={styles.sub}>{index.cityName.get(r.city) ?? r.city}</div>
      <dl className={`${styles.grid} mono`}>
        <dt>links up</dt>
        <dd>
          {upLinks}/{adj.length}
        </dd>
        <dt>peak util</dt>
        <dd style={{ color: utilCss(maxUtil) }}>{permille(maxUtil)}</dd>
        <dt>peak queue</dt>
        <dd>{ms(maxQueue / 10)}</dd>
        <dt>peak loss</dt>
        <dd>{permille(maxLoss, 1)}</dd>
        <dt>peak risk</dt>
        <dd className={maxRisk > 300 ? styles.risk : ""}>{permille(maxRisk)}</dd>
        {r.hosts > 0 && (
          <>
            <dt>hosts</dt>
            <dd>{int(r.hosts)}</dd>
          </>
        )}
      </dl>
      <div className={styles.hint}>click to inspect · shift-click to trace</div>
    </div>
  );
}

function LinkCard({ id, index }: { id: number; index: TopoIndex }) {
  const util = useApp((s) => s.frame?.links.util[id] ?? 0);
  const queue = useApp((s) => s.frame?.links.queueMs[id] ?? 0);
  const loss = useApp((s) => s.frame?.links.loss[id] ?? 0);
  const state = useApp((s) => s.frame?.links.state[id] ?? 0);
  const risk = useApp((s) => s.frame?.links.risk[id] ?? 0);
  const l = index.topology.links[id];
  if (!l) return null;
  const a = index.topology.routers[l.a];
  const b = index.topology.routers[l.b];
  const asA = index.asOfRouter(l.a);
  const asB = index.asOfRouter(l.b);

  return (
    <div className={styles.card}>
      <div className={styles.kicker}>
        <span className="mono">L{id}</span>
        <span>{l.kind}</span>
        <span className={state === LinkState.Up ? styles.up : state === LinkState.Degraded ? styles.degraded : styles.down}>
          {STATE_LABEL[state] ?? "unknown"}
        </span>
      </div>
      <div className={`${styles.name} mono`}>
        {a.name}
        <br />↔ {b.name}
      </div>
      <div className={styles.sub}>
        {asA.id === asB.id ? (
          <>
            {asA.name} · <span className="mono">AS{asA.asn}</span>
          </>
        ) : (
          <span className="mono">
            AS{asA.asn} ↔ AS{asB.asn}
          </span>
        )}
      </div>
      <div className={styles.utilBar}>
        <span style={{ width: `${Math.min(100, util / 10)}%`, background: utilCss(util) }} />
      </div>
      <dl className={`${styles.grid} mono`}>
        <dt>util</dt>
        <dd style={{ color: utilCss(util) }}>{permille(util)}</dd>
        <dt>capacity</dt>
        <dd>{capacity(l.capacityGbps)}</dd>
        <dt>queue</dt>
        <dd>{ms(queue / 10)}</dd>
        <dt>propagation</dt>
        <dd>
          {ms(l.delayMs)} · {int(l.km)} km
        </dd>
        <dt>loss</dt>
        <dd className={loss > 0 ? styles.lossy : ""}>{permille(loss, 1)}</dd>
        <dt>risk</dt>
        <dd className={risk > 300 ? styles.risk : ""}>{fixed(risk / 10, 0)}%</dd>
      </dl>
      <div className={styles.hint}>click for actions</div>
    </div>
  );
}
