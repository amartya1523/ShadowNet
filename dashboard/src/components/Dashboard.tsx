"use client";

import { useEffect } from "react";
import { Session } from "@/lib/session";
import styles from "./Dashboard.module.css";
import { EventLog } from "./panels/EventLog";
import { KpiStrip } from "./panels/KpiStrip";
import { LeftRail } from "./panels/LeftRail";
import { RightRail } from "./panels/RightRail";
import { TopBar } from "./TopBar";
import { MapView } from "./map/MapView";

export function Dashboard() {
  useEffect(() => {
    const session = new Session();
    session.start();
    return () => session.stop();
  }, []);

  return (
    <div className={styles.shell}>
      <TopBar />
      <main className={styles.main}>
        <LeftRail />
        <div className={styles.center}>
          <KpiStrip />
          <MapView />
          <EventLog />
        </div>
        <RightRail />
      </main>
    </div>
  );
}
