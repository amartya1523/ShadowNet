"use client";

import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { clearTrace, pickTraceRouter, setTraceMode } from "@/lib/actions";
import { wsUrl } from "@/lib/client";
import { select, store, useApp } from "@/lib/state";
import { Inspector } from "./Inspector";
import { Legend } from "./Legend";
import { MapController } from "./controller";
import type { MapTarget } from "./renderer";
import { HoverCard } from "./HoverCard";
import { TracePanel } from "./TracePanel";
import styles from "./Map.module.css";

const TIP_W = 270;
const noSubscribe = () => () => {};

function sameTarget(a: MapTarget | null, b: MapTarget | null): boolean {
  return a === b || (!!a && !!b && a.kind === b.kind && a.id === b.id);
}

export function MapView() {
  const index = useApp((s) => s.index);
  const traceMode = useApp((s) => s.traceMode);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const tipRef = useRef<HTMLDivElement>(null);
  const ctrlRef = useRef<MapController | null>(null);
  const [hover, setHover] = useState<MapTarget | null>(null);
  const [fps, setFps] = useState(0);
  const [zoom, setZoom] = useState(1);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctrl = new MapController(canvas, {
      onHover(target, x, y) {
        const tip = tipRef.current;
        if (tip && target) {
          const w = canvas.clientWidth;
          const h = canvas.clientHeight;
          const left = x + 18 + TIP_W > w ? x - TIP_W - 14 : x + 18;
          const top = Math.min(Math.max(8, y - 10), h - 190);
          tip.style.transform = `translate(${left}px, ${top}px)`;
        }
        setHover((prev) => (sameTarget(prev, target) ? prev : target));
      },
      onPick(target, ev) {
        const s = store.get();
        if (target?.kind === "router" && (ev.shift || s.traceMode)) {
          pickTraceRouter(target.id);
          return;
        }
        select(target);
      },
      onFps(value) {
        setFps(Math.round(value));
        setZoom(ctrl.zoom);
      },
    });
    ctrlRef.current = ctrl;
    ctrl.setTopology(store.get().index);
    return () => {
      ctrl.dispose();
      ctrlRef.current = null;
    };
  }, []);

  useEffect(() => {
    ctrlRef.current?.setTopology(index);
  }, [index]);

  useEffect(() => {
    const onKey = (ev: KeyboardEvent) => {
      if (ev.target instanceof HTMLInputElement || ev.target instanceof HTMLSelectElement) return;
      if (ev.key === "Escape") {
        if (store.get().trace || store.get().traceMode) clearTrace();
        else select(null);
      } else if (ev.key === "t" && !ev.metaKey && !ev.ctrlKey) {
        setTraceMode(!store.get().traceMode);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <section className={`${styles.map} ${traceMode ? styles.tracing : ""}`}>
      <canvas ref={canvasRef} className={styles.canvas} />
      <div className={styles.vignette} />

      <div className={styles.hudTop}>
        <div className={styles.hudTitle}>
          <span className={styles.liveDot} />
          Global topology
        </div>
        <div className={`${styles.hudMeta} mono`}>
          equirectangular · {zoom.toFixed(1)}× · {fps} fps
        </div>
      </div>

      <Legend />

      <div className={styles.toolbar}>
        <button
          type="button"
          className={`${styles.tool} ${traceMode ? styles.toolActive : ""}`}
          onClick={() => setTraceMode(!traceMode)}
          disabled={!index}
          title="Trace mode (T). Or shift-click two routers."
        >
          <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden>
            <circle cx="2.5" cy="11.5" r="1.8" fill="none" stroke="currentColor" strokeWidth="1.3" />
            <circle cx="11.5" cy="2.5" r="1.8" fill="none" stroke="currentColor" strokeWidth="1.3" />
            <path d="M4 10 7 7.5 6 5.5 10 4" fill="none" stroke="currentColor" strokeWidth="1.3" strokeDasharray="1.6 1.4" />
          </svg>
          Trace
        </button>
        <div className={styles.toolGroup}>
          <button type="button" className={styles.tool} onClick={() => ctrlRef.current?.zoomBy(1 / 1.6)} aria-label="Zoom out">
            −
          </button>
          <button type="button" className={styles.tool} onClick={() => ctrlRef.current?.resetView()} aria-label="Reset view">
            <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden>
              <path d="M1 4V1h3M11 4V1H8M1 8v3h3M11 8v3H8" fill="none" stroke="currentColor" strokeWidth="1.3" />
            </svg>
          </button>
          <button type="button" className={styles.tool} onClick={() => ctrlRef.current?.zoomBy(1.6)} aria-label="Zoom in">
            +
          </button>
        </div>
      </div>

      {traceMode && <TraceHint />}

      <div ref={tipRef} className={styles.tipAnchor} style={{ width: TIP_W }}>
        {hover && index && <HoverCard target={hover} index={index} />}
      </div>

      <div className={styles.dock}>
        <TracePanel />
        <Inspector />
      </div>
      <Notice />
      {!index && <Connecting />}
    </section>
  );
}

function TraceHint() {
  const src = useApp((s) => (s.trace && s.trace.dst === null ? s.trace.src : null));
  const name = useApp((s) => (src !== null ? s.topology?.routers[src]?.name : null));
  return (
    <div className={`${styles.traceHint} mono`}>
      {src === null ? "trace · pick a source router" : `trace · from ${name} · pick a destination`}
      <span className={styles.kbd}>esc</span>
    </div>
  );
}

function Notice() {
  const notice = useApp((s) => s.notice);
  if (!notice) return null;
  // Keyed on the notice so each one restarts the CSS fade.
  return (
    <div key={notice.key} className={`${styles.notice} ${notice.level === "error" ? styles.noticeError : ""} mono`}>
      {notice.text}
    </div>
  );
}

function Connecting() {
  const status = useApp((s) => s.status);
  const error = useApp((s) => s.topologyError);
  const url = useSyncExternalStore(noSubscribe, wsUrl, () => "");
  const line =
    status === "open"
      ? error
        ? `topology request failed: ${error}`
        : "loading topology…"
      : status === "closed"
        ? "shadownetd unreachable · retrying"
        : "connecting to shadownetd…";
  return (
    <div className={styles.connecting}>
      <div className={styles.radar}>
        <span />
        <span />
        <span />
      </div>
      <div className={styles.connectingText}>{line}</div>
      <div className={`${styles.connectingAddr} mono`}>{url}</div>
    </div>
  );
}
