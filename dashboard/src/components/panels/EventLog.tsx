"use client";

import { fixed } from "@/lib/format";
import { useApp } from "@/lib/state";
import { Empty, Panel } from "../ui/Panel";
import styles from "./EventLog.module.css";

export function EventLog() {
  const log = useApp((s) => s.log);
  const status = useApp((s) => s.status);
  const crit = log.filter((e) => e.level === "crit").length;
  const warn = log.filter((e) => e.level === "warn").length;

  return (
    <Panel
      code="LOG"
      title="Event stream"
      className={styles.panel}
      bodyClassName={styles.body}
      meta={
        <>
          <span className={styles.count}>
            <i className={styles.critDot} />
            {crit}
          </span>
          <span className={styles.count}>
            <i className={styles.warnDot} />
            {warn}
          </span>
          <span>last {log.length}</span>
        </>
      }
    >
      {log.length === 0 ? (
        <Empty>{status === "open" ? "quiet. no control-plane events yet" : "connecting to shadownetd…"}</Empty>
      ) : (
        // column-reverse keeps the view pinned to the newest line without scroll bookkeeping.
        <ol className={`${styles.lines} mono`}>
          {log
            .slice()
            .reverse()
            .map((e) => (
              <li key={e.key} className={`${styles.line} ${styles[e.level]}`}>
                <span className={styles.t}>{fixed(e.t, 2).padStart(8, " ")}</span>
                <span className={styles.level}>{e.level}</span>
                <span className={styles.kind}>{e.kind}</span>
                <span className={styles.text}>{e.text}</span>
              </li>
            ))}
        </ol>
      )}
    </Panel>
  );
}
