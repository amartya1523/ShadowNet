-- Handy views once a few runs have been recorded. Loaded at first start of
-- the TimescaleDB container; harmless to run by hand against plain Postgres
-- after shadownetd has created its tables.

CREATE TABLE IF NOT EXISTS net_stats (
    time TIMESTAMPTZ NOT NULL DEFAULT now(), run_id INTEGER NOT NULL, sim_t DOUBLE PRECISION NOT NULL,
    routing TEXT NOT NULL, offered_gbps DOUBLE PRECISION NOT NULL, delivered_gbps DOUBLE PRECISION NOT NULL,
    loss_pct DOUBLE PRECISION NOT NULL, mean_ms DOUBLE PRECISION NOT NULL, p99_ms DOUBLE PRECISION NOT NULL,
    active_flows INTEGER NOT NULL, broken_flows INTEGER NOT NULL, links_down INTEGER NOT NULL,
    congested_links INTEGER NOT NULL
);

-- Worst ten seconds of each run.
CREATE OR REPLACE VIEW worst_moments AS
SELECT DISTINCT ON (run_id) run_id, sim_t, routing, loss_pct, p99_ms, broken_flows
FROM net_stats
ORDER BY run_id, loss_pct DESC;

-- Same experiment, different routing: mean loss and tail latency per mode.
CREATE OR REPLACE VIEW by_routing AS
SELECT routing,
       count(*)                       AS seconds,
       round(avg(loss_pct)::numeric, 3) AS mean_loss_pct,
       round(avg(p99_ms)::numeric, 1)   AS mean_p99_ms,
       round(max(p99_ms)::numeric, 1)   AS worst_p99_ms
FROM net_stats
GROUP BY routing;
