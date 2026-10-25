"use client";

import { AiPanel } from "./AiPanel";
import { HistoryChart } from "./HistoryChart";
import { Incidents } from "./Incidents";
import styles from "./RightRail.module.css";

export function RightRail() {
  return (
    <aside className={styles.rail}>
      <HistoryChart />
      <Incidents />
      <AiPanel />
    </aside>
  );
}
