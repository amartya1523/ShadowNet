import type { ReactNode } from "react";
import styles from "./Panel.module.css";

interface PanelProps {
  code: string;
  title: string;
  meta?: ReactNode;
  className?: string;
  bodyClassName?: string;
  children: ReactNode;
}

export function Panel({ code, title, meta, className, bodyClassName, children }: PanelProps) {
  return (
    <section className={`${styles.panel} ${className ?? ""}`}>
      <header className={styles.head}>
        <span className={styles.code}>{code}</span>
        <h2 className={styles.title}>{title}</h2>
        {meta !== undefined && <div className={styles.meta}>{meta}</div>}
      </header>
      <div className={`${styles.body} ${bodyClassName ?? ""}`}>{children}</div>
    </section>
  );
}

export function Empty({ children }: { children: ReactNode }) {
  return <div className={styles.empty}>{children}</div>;
}
