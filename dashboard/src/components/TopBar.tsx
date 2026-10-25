"use client";

import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { displayAddress } from "@/lib/client";
import { compact, fixed, int, simClock } from "@/lib/format";
import type { RoutingMode } from "@/lib/protocol";
import { send } from "@/lib/session";
import { live, useApp } from "@/lib/state";
import styles from "./TopBar.module.css";
import ui from "./ui/controls.module.css";

const ROUTING: { mode: RoutingMode; label: string }[] = [
  { mode: "ospf", label: "OSPF" },
  { mode: "dv", label: "RIP·DV" },
  { mode: "qrouting", label: "Q-ROUTING" },
];

const SPEEDS = [0.5, 1, 2, 4];

const noSubscribe = () => () => {};

export function TopBar() {
  return (
    <header className={styles.bar}>
      <Brand />
      <div className={styles.divider} />
      <Clock />
      <div className={styles.controls}>
        <RoutingSwitch />
        <PlayControls />
      </div>
      <div className={styles.right}>
        <ScaleBadge />
        <ConnectionStatus />
      </div>
    </header>
  );
}

function Brand() {
  return (
    <div className={styles.brand}>
      <svg className={styles.logo} viewBox="0 0 32 32" aria-hidden>
        <circle cx="16" cy="16" r="13" fill="none" stroke="currentColor" strokeOpacity="0.35" />
        <circle cx="16" cy="16" r="8.5" fill="none" stroke="currentColor" strokeWidth="1.5" />
        <path d="M3 16h26M16 3v26" stroke="currentColor" strokeOpacity="0.35" />
        <circle cx="16" cy="16" r="3" fill="currentColor" />
        <circle cx="24.5" cy="16" r="1.6" fill="var(--violet)" />
      </svg>
      <div>
        <div className={styles.wordmark}>SHADOWNET</div>
        <div className={styles.subtitle}>internet simulator</div>
      </div>
    </div>
  );
}

/**
 * The clock advances between frames at the reported speed so it reads like a
 * running clock rather than a counter that jumps five times a second.
 */
function Clock() {
  const clockRef = useRef<HTMLSpanElement>(null);
  const frame = useApp((s) => s.frame);

  useEffect(() => {
    let raf = 0;
    const loop = () => {
      raf = requestAnimationFrame(loop);
      const f = live.cur;
      const el = clockRef.current;
      if (!el) return;
      if (!f) {
        el.textContent = "--:--.-";
        return;
      }
      const ahead = f.paused ? 0 : Math.min(0.5, (performance.now() - live.receivedAt) / 1000) * f.speed;
      el.textContent = simClock(f.t + ahead);
    };
    loop();
    return () => cancelAnimationFrame(raf);
  }, []);

  const lagging = frame && !frame.paused && frame.simRate < frame.speed * 0.9;
  return (
    <div className={styles.clockGroup}>
      <div className={styles.stat}>
        <span className={styles.statLabel}>Sim time</span>
        <span className={`${styles.clock} mono`}>
          <span className={styles.tPrefix}>T+</span>
          <span ref={clockRef}>--:--.-</span>
        </span>
      </div>
      <div className={styles.stat}>
        <span className={styles.statLabel}>Speed</span>
        <span className={`${styles.statValue} mono`}>{frame ? `${fixed(frame.speed, 1)}×` : "–"}</span>
      </div>
      <div className={styles.stat}>
        <span className={styles.statLabel}>Achieved</span>
        <span className={`${styles.statValue} mono ${lagging ? styles.lagging : ""}`}>
          {frame ? `${fixed(frame.simRate, 2)}×` : "–"}
        </span>
      </div>
      <div className={`${styles.stat} ${styles.eventsStat}`}>
        <span className={styles.statLabel}>Events/s</span>
        <span className={`${styles.statValue} mono`}>{frame ? compact(frame.stats.eventsPerSec) : "–"}</span>
      </div>
    </div>
  );
}

function RoutingSwitch() {
  const routing = useApp((s) => s.frame?.routing ?? null);
  // Optimistic until the next frame reports a different mode than the one we
  // switched away from, whichever mode that turns out to be.
  const [pending, setPending] = useState<{ mode: RoutingMode; from: RoutingMode | null } | null>(null);
  const shown = pending && pending.from === routing ? pending.mode : routing;

  const choose = async (mode: RoutingMode) => {
    if (mode === shown) return;
    setPending({ mode, from: routing });
    const ok = await send({ cmd: "set_routing", mode });
    if (!ok) setPending(null);
  };

  return (
    <div className={styles.controlGroup}>
      <span className={styles.statLabel}>Intra-AS routing</span>
      <div className={ui.segmented} role="radiogroup" aria-label="Routing mode">
        {ROUTING.map(({ mode, label }) => (
          <button
            key={mode}
            type="button"
            role="radio"
            aria-checked={shown === mode}
            disabled={!routing}
            className={`${ui.segment} ${mode === "qrouting" ? ui.segmentAi : ""} ${
              shown === mode ? ui.segmentActive : ""
            }`}
            onClick={() => void choose(mode)}
          >
            {label}
          </button>
        ))}
      </div>
    </div>
  );
}

function PlayControls() {
  const paused = useApp((s) => s.frame?.paused ?? null);
  const speed = useApp((s) => s.frame?.speed ?? null);
  const disabled = paused === null;

  return (
    <>
      <div className={styles.controlGroup}>
        <span className={styles.statLabel}>Run</span>
        <button
          type="button"
          className={`${ui.button} ${paused ? ui.primary : ""} ${styles.play}`}
          disabled={disabled}
          onClick={() => void send({ cmd: paused ? "resume" : "pause" })}
        >
          {paused ? (
            <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden>
              <path d="M1.5 0.8 9 5 1.5 9.2Z" fill="currentColor" />
            </svg>
          ) : (
            <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden>
              <rect x="1.2" y="0.8" width="2.6" height="8.4" fill="currentColor" />
              <rect x="6.2" y="0.8" width="2.6" height="8.4" fill="currentColor" />
            </svg>
          )}
          {paused ? "Resume" : "Pause"}
        </button>
      </div>
      <div className={styles.controlGroup}>
        <span className={styles.statLabel}>Speed</span>
        <div className={ui.segmented} role="radiogroup" aria-label="Simulation speed">
          {SPEEDS.map((v) => (
            <button
              key={v}
              type="button"
              role="radio"
              aria-checked={speed === v}
              disabled={disabled}
              className={`${ui.segment} ${speed === v ? ui.segmentActive : ""}`}
              onClick={() => void send({ cmd: "set_speed", speed: v })}
            >
              {v}×
            </button>
          ))}
        </div>
      </div>
    </>
  );
}

function ScaleBadge() {
  const topology = useApp((s) => s.topology);
  if (!topology) return <div className={`${styles.badge} ${styles.badgeEmpty} mono`}>no topology</div>;
  const s = topology.summary;
  return (
    <div className={`${styles.badge} mono`}>
      <b>{int(s.nodes)}</b> nodes<span className={styles.sep}>·</span>
      <b>{int(s.routers)}</b> routers<span className={styles.sep}>·</span>
      <b>{int(topology.ases.length)}</b> ASes
    </div>
  );
}

function ConnectionStatus() {
  const status = useApp((s) => s.status);
  const hasTopology = useApp((s) => s.topology !== null);
  const address = useSyncExternalStore(noSubscribe, displayAddress, () => "");

  const label =
    status === "open" ? (hasTopology ? "LIVE" : "SYNCING") : status === "connecting" ? "CONNECTING" : "OFFLINE";
  const tone = status === "open" && hasTopology ? styles.ok : status === "closed" ? styles.bad : styles.wait;
  return (
    <div className={`${styles.conn} ${tone}`} title={`shadownetd at ${address}`}>
      <span className={styles.dot} />
      <div>
        <div className={styles.connLabel}>{label}</div>
        <div className={`${styles.connAddr} mono`}>{address}</div>
      </div>
    </div>
  );
}
