"use client";

import { useState } from "react";
import { int } from "@/lib/format";
import { send } from "@/lib/session";
import { useApp } from "@/lib/state";
import { Empty, Panel } from "../ui/Panel";
import ui from "../ui/controls.module.css";
import styles from "./LeftRail.module.css";

export function LeftRail() {
  return (
    <aside className={styles.rail}>
      <Scenarios />
      <Chaos />
      <RunInfo />
    </aside>
  );
}

function Scenarios() {
  const scenarios = useApp((s) => s.scenarios);
  const t = useApp((s) => s.frame?.t ?? null);
  const runId = useApp((s) => s.topology?.runId ?? null);
  const [running, setRunning] = useState<{ name: string; t0: number; runId: number | null } | null>(null);

  const start = async (name: string) => {
    if (await send({ cmd: "run_scenario", name }, `scenario ${name} started`)) {
      setRunning({ name, t0: t ?? 0, runId });
    }
  };

  return (
    <Panel code="01" title="Scenarios" className={styles.scenarios} bodyClassName={styles.scroll} meta={scenarios.length || undefined}>
      {scenarios.length === 0 ? (
        <Empty>no scenarios loaded</Empty>
      ) : (
        <ul className={styles.list}>
          {scenarios.map((sc) => {
            const active = running?.name === sc.name && running.runId === runId && t !== null;
            const progress = active ? Math.min(1, Math.max(0, (t - running.t0) / Math.max(1, sc.durationSecs))) : 0;
            const live = active && progress < 1;
            return (
              <li key={sc.name} className={`${styles.scenario} ${live ? styles.scenarioLive : ""}`}>
                <div className={styles.scenarioHead}>
                  <div className={styles.scenarioTitle}>{sc.title}</div>
                  <button
                    type="button"
                    className={`${ui.button} ${ui.small} ${live ? ui.warn : ui.primary}`}
                    onClick={() => void start(sc.name)}
                    disabled={t === null}
                  >
                    {live ? "Running" : "Run"}
                  </button>
                </div>
                <p className={styles.scenarioDesc}>{sc.description}</p>
                <div className={`${styles.scenarioMeta} mono`}>
                  <span>{sc.name}</span>
                  <span>{int(sc.durationSecs)}s</span>
                </div>
                {live && (
                  <div className={styles.progress}>
                    <div style={{ transform: `scaleX(${progress})` }} />
                  </div>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </Panel>
  );
}

function Chaos() {
  const services = useApp((s) => s.topology?.services ?? null);
  const chaosMonkey = useApp((s) => s.frame?.chaosMonkey ?? false);
  const crowdFlows = useApp((s) => s.frame?.stats.flows.crowd ?? 0);
  const attackFlows = useApp((s) => s.frame?.stats.flows.attack ?? 0);
  const ready = services !== null;

  const [crowdService, setCrowdService] = useState(0);
  const [users, setUsers] = useState(10000);
  const [ddosService, setDdosService] = useState(1);
  const [bots, setBots] = useState(3000);
  const [seed, setSeed] = useState("");

  const serviceOptions = (services ?? []).map((s) => (
    <option key={s.id} value={s.id}>
      {s.name}
    </option>
  ));
  const serviceName = (id: number) => services?.find((s) => s.id === id)?.name ?? `#${id}`;

  return (
    <Panel code="02" title="Chaos" className={styles.chaos}>
      <div className={styles.group}>
        <div className={styles.groupHead}>
          <span>Flash crowd</span>
          {crowdFlows > 0 && <span className={`${ui.chip} ${ui.chipAmber}`}>{int(crowdFlows)} flows</span>}
        </div>
        <div className={styles.row}>
          <label className={`${ui.field} ${styles.grow}`}>
            <span className={ui.label}>Service</span>
            <select
              className={ui.input}
              value={crowdService}
              disabled={!ready}
              onChange={(e) => setCrowdService(Number(e.target.value))}
            >
              {serviceOptions}
            </select>
          </label>
          <label className={`${ui.field} ${styles.num}`}>
            <span className={ui.label}>Users</span>
            <input
              className={ui.input}
              type="number"
              min={100}
              step={1000}
              value={users}
              onChange={(e) => setUsers(Number(e.target.value))}
            />
          </label>
        </div>
        <button
          type="button"
          className={`${ui.button} ${ui.warn} ${ui.block}`}
          disabled={!ready}
          onClick={() =>
            void send(
              { cmd: "flash_crowd", service: crowdService, users, mbps: 8, rampSecs: 5, secs: 60 },
              `flash crowd → ${serviceName(crowdService)}`,
            )
          }
        >
          Launch flash crowd
        </button>
      </div>

      <div className={styles.group}>
        <div className={styles.groupHead}>
          <span>DDoS</span>
          {attackFlows > 0 && <span className={`${ui.chip} ${ui.chipRed}`}>{int(attackFlows)} flows</span>}
        </div>
        <div className={styles.row}>
          <label className={`${ui.field} ${styles.grow}`}>
            <span className={ui.label}>Victim</span>
            <select
              className={ui.input}
              value={ddosService}
              disabled={!ready}
              onChange={(e) => setDdosService(Number(e.target.value))}
            >
              {serviceOptions}
            </select>
          </label>
          <label className={`${ui.field} ${styles.num}`}>
            <span className={ui.label}>Bots</span>
            <input
              className={ui.input}
              type="number"
              min={10}
              step={500}
              value={bots}
              onChange={(e) => setBots(Number(e.target.value))}
            />
          </label>
        </div>
        <button
          type="button"
          className={`${ui.button} ${ui.danger} ${ui.block}`}
          disabled={!ready}
          onClick={() =>
            void send(
              { cmd: "ddos", service: ddosService, bots, mbps: 30, secs: 40 },
              `DDoS → ${serviceName(ddosService)}`,
            )
          }
        >
          Launch DDoS
        </button>
      </div>

      <div className={`${styles.group} ${styles.monkey}`}>
        <div>
          <div className={styles.groupHead}>
            <span>Chaos monkey</span>
          </div>
          <div className={styles.hint}>fragility-weighted failures · 6/min</div>
        </div>
        <button
          type="button"
          role="switch"
          aria-checked={chaosMonkey}
          aria-label="Chaos monkey"
          disabled={!ready}
          className={`${ui.toggle} ${chaosMonkey ? ui.toggleOn : ""}`}
          onClick={() => void send({ cmd: "chaos_monkey", enabled: !chaosMonkey, perMinute: 6 })}
        />
      </div>

      <div className={styles.actions}>
        <button
          type="button"
          className={`${ui.button} ${ui.primary} ${styles.grow}`}
          disabled={!ready}
          onClick={() => void send({ cmd: "heal_all" }, "all faults cleared")}
        >
          Heal all
        </button>
        <input
          className={`${ui.input} ${styles.seed}`}
          placeholder="seed"
          inputMode="numeric"
          value={seed}
          onChange={(e) => setSeed(e.target.value.replace(/[^0-9]/g, ""))}
          aria-label="Reset seed"
        />
        <button
          type="button"
          className={ui.button}
          disabled={!ready}
          onClick={() => void send(seed ? { cmd: "reset", seed: Number(seed) } : { cmd: "reset" }, "resetting world")}
        >
          Reset
        </button>
      </div>
    </Panel>
  );
}

function RunInfo() {
  const topology = useApp((s) => s.topology);
  if (!topology) {
    return (
      <Panel code="03" title="World">
        <Empty>waiting for topology</Empty>
      </Panel>
    );
  }
  const s = topology.summary;
  const rows: [string, string][] = [
    ["tier-1 / tier-2 / stub", `${s.tier1} / ${s.tier2} / ${s.stubs}`],
    ["routers · links", `${int(s.routers)} · ${int(s.links)}`],
    ["hosts · servers", `${int(s.hosts)} · ${int(s.servers)}`],
    ["services · cities", `${s.services} · ${topology.cities.length}`],
  ];
  return (
    <Panel code="03" title="World" meta={<span className="mono">run #{topology.runId}</span>}>
      <dl className={`${styles.facts} mono`}>
        {rows.map(([k, v]) => (
          <div key={k}>
            <dt>{k}</dt>
            <dd>{v}</dd>
          </div>
        ))}
        <div>
          <dt>seed</dt>
          <dd className={styles.seedValue}>{topology.seed}</dd>
        </div>
      </dl>
    </Panel>
  );
}
