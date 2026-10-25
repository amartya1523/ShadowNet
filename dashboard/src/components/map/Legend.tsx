import styles from "./Legend.module.css";

export function Legend() {
  return (
    <div className={styles.legend} aria-hidden>
      <div className={styles.block}>
        <span className={styles.heading}>Link utilisation</span>
        <span className={styles.ramp} />
        <span className={`${styles.scale} mono`}>
          <span>0</span>
          <span>50</span>
          <span>75</span>
          <span>100%</span>
        </span>
      </div>
      <div className={styles.block}>
        <span className={styles.heading}>State</span>
        <span className={styles.item}>
          <i className={styles.lineDegraded} />
          degraded
        </span>
        <span className={styles.item}>
          <i className={styles.lineDown} />
          down
        </span>
        <span className={styles.item}>
          <i className={styles.lineRisk} />
          at risk
        </span>
      </div>
      <div className={styles.block}>
        <span className={styles.heading}>Routers</span>
        <span className={styles.item}>
          <i className={styles.t1} />
          tier-1
        </span>
        <span className={styles.item}>
          <i className={styles.t2} />
          tier-2
        </span>
        <span className={styles.item}>
          <i className={styles.stub} />
          stub
        </span>
        <span className={styles.item}>
          <i className={styles.dc} />
          datacenter
        </span>
      </div>
      <div className={styles.block}>
        <span className={styles.heading}>Traffic</span>
        <span className={styles.item}>
          <i className={`${styles.pkt} ${styles.bg}`} />
          background
        </span>
        <span className={styles.item}>
          <i className={`${styles.pkt} ${styles.crowd}`} />
          crowd
        </span>
        <span className={styles.item}>
          <i className={`${styles.pkt} ${styles.attack}`} />
          attack
        </span>
      </div>
    </div>
  );
}
