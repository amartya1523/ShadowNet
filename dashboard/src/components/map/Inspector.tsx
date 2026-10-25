"use client";

import { useEffect, useState } from "react";
import { pickTraceRouter } from "@/lib/actions";
import { api } from "@/lib/client";
import { capacity, int, ms, permille } from "@/lib/format";
import { LinkState, type RouterDetail } from "@/lib/protocol";
import { send } from "@/lib/session";
import { focusOn, select, useApp } from "@/lib/state";
import type { TopoIndex } from "@/lib/topology";
import ui from "../ui/controls.module.css";
import { utilCss } from "./palette";
import styles from "./Inspector.module.css";

const DETAIL_REFRESH_MS = 2000;
const FIB_ROWS = 8;

export function Inspector() {
  const selection = useApp((s) => s.selection);
  const index = useApp((s) => s.index);
  if (!selection || !index) return null;
  return (
    <aside className={styles.inspector} key={`${selection.kind}-${selection.id}`}>
      <button type="button" className={styles.close} onClick={() => select(null)} aria-label="Close inspector">
        ×
      </button>
      {selection.kind === "router" ? (
        <RouterInspector id={selection.id} index={index} />
      ) : (
        <LinkInspector id={selection.id} index={index} />
      )}
    </aside>
  );
}

function useRouterDetail(id: number) {
  const runId = useApp((s) => s.topology?.runId);
  const [detail, setDetail] = useState<{ id: number; data: RouterDetail } | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const ctrl = new AbortController();
    let timer: ReturnType<typeof setTimeout> | null = null;
    const load = async () => {
      try {
        const data = await api.router(id, ctrl.signal);
        setDetail({ id, data });
        setError(null);
      } catch (err) {
        if (!ctrl.signal.aborted) setError(err instanceof Error ? err.message : String(err));
      }
      if (!ctrl.signal.aborted) timer = setTimeout(load, DETAIL_REFRESH_MS);
    };
    void load();
    return () => {
      ctrl.abort();
      if (timer) clearTimeout(timer);
    };
  }, [id, runId]);

  return { detail: detail?.id === id ? detail.data : null, error };
}

function RouterInspector({ id, index }: { id: number; index: TopoIndex }) {
  const { detail, error } = useRouterDetail(id);
  const routersDown = useApp((s) => s.frame?.routersDown ?? null);
  const r = index.topology.routers[id];
  if (!r) return null;
  const as = index.asOfRouter(id);
  const down = detail?.down ?? routersDown?.includes(id) ?? false;
  const asRouters = index.routersByAs[as.id] ?? [];
  const asDown = routersDown !== null && asRouters.length > 0 && asRouters.every((rid) => routersDown.includes(rid));

  return (
    <>
      <div className={styles.kicker}>
        <span className={styles.kind}>Router</span>
        <span>{r.role}</span>
        <span>{as.tier}</span>
        <span className={down ? styles.stateDown : styles.stateUp}>{down ? "down" : "up"}</span>
      </div>
      <h3 className={`${styles.title} mono`}>{r.name}</h3>
      <div className={styles.sub}>
        {as.name} · <span className="mono">AS{as.asn}</span> · {index.cityName.get(r.city) ?? r.city}
      </div>

      <div className={styles.actions}>
        {down ? (
          <button type="button" className={`${ui.button} ${ui.primary} ${ui.small}`} onClick={() => void send({ cmd: "restore_router", router: id })}>
            Recover
          </button>
        ) : (
          <button type="button" className={`${ui.button} ${ui.danger} ${ui.small}`} onClick={() => void send({ cmd: "fail_router", router: id }, `${r.name} crashed`)}>
            Crash router
          </button>
        )}
        {asDown ? (
          <button type="button" className={`${ui.button} ${ui.primary} ${ui.small}`} onClick={() => void send({ cmd: "restore_as", as: as.id })}>
            Restore AS{as.asn}
          </button>
        ) : (
          <button
            type="button"
            className={`${ui.button} ${ui.danger} ${ui.small}`}
            onClick={() => void send({ cmd: "fail_as", as: as.id }, `AS${as.asn} taken down`)}
          >
            Take down AS
          </button>
        )}
        <button type="button" className={`${ui.button} ${ui.small}`} onClick={() => pickTraceRouter(id)}>
          Trace from here
        </button>
      </div>

      <div className={styles.section}>
        <div className={styles.sectionHead}>
          <span>Neighbors</span>
          <span className="mono">{detail ? detail.neighbors.length : index.adjacency[id].length}</span>
        </div>
        {detail ? (
          <ul className={styles.neighbors}>
            {detail.neighbors.map((n) => {
              const nr = index.topology.routers[n.router];
              const u = n.util * 1000;
              return (
                <li key={`${n.link}-${n.router}`}>
                  <button type="button" className={styles.neighbor} onClick={() => focusOn("router", n.router)}>
                    <span className={`${styles.dot} ${n.up ? "" : styles.dotDown}`} />
                    <span className={`${styles.nName} mono`}>{nr?.name ?? n.router}</span>
                    <span className={styles.bar}>
                      <span style={{ width: `${Math.min(100, u / 10)}%`, background: utilCss(u) }} />
                    </span>
                    <span className={`${styles.nUtil} mono`}>{n.up ? permille(u) : "down"}</span>
                  </button>
                </li>
              );
            })}
          </ul>
        ) : (
          <div className={styles.loading}>{error ? `router detail unavailable: ${error}` : "querying router…"}</div>
        )}
      </div>

      {detail && detail.fib.length > 0 && (
        <div className={styles.section}>
          <div className={styles.sectionHead}>
            <span>FIB excerpt</span>
            <span className="mono">
              {Math.min(FIB_ROWS, detail.fib.length)} of {int(detail.fib.length)}
            </span>
          </div>
          <table className={`${styles.fib} mono`}>
            <thead>
              <tr>
                <th>target</th>
                <th>next hop</th>
                <th>cost</th>
              </tr>
            </thead>
            <tbody>
              {detail.fib.slice(0, FIB_ROWS).map((f, i) => (
                <tr key={i}>
                  <td>{f.target}</td>
                  <td>{index.topology.routers[f.nextHop]?.name.split(".")[0] ?? f.nextHop}</td>
                  <td>{int(f.cost)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </>
  );
}

function LinkInspector({ id, index }: { id: number; index: TopoIndex }) {
  const util = useApp((s) => s.frame?.links.util[id] ?? 0);
  const queue = useApp((s) => s.frame?.links.queueMs[id] ?? 0);
  const loss = useApp((s) => s.frame?.links.loss[id] ?? 0);
  const state = useApp((s) => s.frame?.links.state[id] ?? LinkState.Up);
  const risk = useApp((s) => s.frame?.links.risk[id] ?? 0);
  const l = index.topology.links[id];
  if (!l) return null;
  const a = index.topology.routers[l.a];
  const b = index.topology.routers[l.b];
  const asA = index.asOfRouter(l.a);
  const asB = index.asOfRouter(l.b);
  const isDown = state === LinkState.Down || state === LinkState.Undetected;
  const stateLabel = ["up", "down", "degraded", "undetected"][state] ?? "unknown";

  return (
    <>
      <div className={styles.kicker}>
        <span className={styles.kind}>Link L{id}</span>
        <span>{l.kind}</span>
        <span className={isDown ? styles.stateDown : state === LinkState.Degraded ? styles.stateWarn : styles.stateUp}>
          {stateLabel}
        </span>
      </div>
      <h3 className={`${styles.title} mono`}>
        <button type="button" className={styles.endpoint} onClick={() => focusOn("router", l.a)}>
          {a.name}
        </button>
        <span className={styles.arrow}>↔</span>
        <button type="button" className={styles.endpoint} onClick={() => focusOn("router", l.b)}>
          {b.name}
        </button>
      </h3>
      <div className={styles.sub}>
        <span className="mono">AS{asA.asn}</span> {asA.name}
        {asA.id !== asB.id && (
          <>
            {" "}
            → <span className="mono">AS{asB.asn}</span> {asB.name}
          </>
        )}
      </div>

      <div className={styles.gauge}>
        <div className={styles.gaugeHead}>
          <span>Utilisation</span>
          <span className="mono" style={{ color: utilCss(util) }}>
            {permille(util)}
          </span>
        </div>
        <div className={styles.gaugeTrack}>
          <span style={{ width: `${Math.min(100, util / 10)}%`, background: utilCss(util) }} />
          <i style={{ left: "70%" }} />
          <i style={{ left: "90%" }} />
        </div>
      </div>

      <dl className={`${styles.facts} mono`}>
        <div>
          <dt>capacity</dt>
          <dd>{capacity(l.capacityGbps)}</dd>
        </div>
        <div>
          <dt>propagation</dt>
          <dd>{ms(l.delayMs)}</dd>
        </div>
        <div>
          <dt>distance</dt>
          <dd>{int(l.km)} km</dd>
        </div>
        <div>
          <dt>queue</dt>
          <dd className={queue > 50 ? styles.warn : ""}>{ms(queue / 10)}</dd>
        </div>
        <div>
          <dt>loss</dt>
          <dd className={loss > 0 ? styles.warn : ""}>{permille(loss, 1)}</dd>
        </div>
        <div>
          <dt>failure risk</dt>
          <dd className={risk > 300 ? styles.ai : ""}>{permille(risk)}</dd>
        </div>
      </dl>

      <div className={styles.actions}>
        {isDown ? (
          <button type="button" className={`${ui.button} ${ui.primary} ${ui.small}`} onClick={() => void send({ cmd: "restore_link", link: id })}>
            Restore link
          </button>
        ) : (
          <button type="button" className={`${ui.button} ${ui.danger} ${ui.small}`} onClick={() => void send({ cmd: "fail_link", link: id }, `L${id} cut`)}>
            Cut link
          </button>
        )}
        <button
          type="button"
          className={`${ui.button} ${ui.warn} ${ui.small}`}
          disabled={isDown}
          onClick={() => void send({ cmd: "latency_spike", link: id, ms: 80, secs: 20 }, `L${id} +80 ms for 20 s`)}
        >
          +80 ms · 20s
        </button>
        <button
          type="button"
          className={`${ui.button} ${ui.warn} ${ui.small}`}
          disabled={isDown}
          onClick={() => void send({ cmd: "packet_loss", link: id, pct: 5, secs: 20 }, `L${id} 5% loss for 20 s`)}
        >
          5% loss · 20s
        </button>
      </div>
    </>
  );
}
