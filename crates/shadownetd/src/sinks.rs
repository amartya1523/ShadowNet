//! Optional metrics export: TimescaleDB for history you can query later,
//! Redis for anything that wants the live numbers without speaking our
//! WebSocket protocol.
//!
//! Both run on their own task fed by a bounded channel. The simulation never
//! waits on a database; if a sink falls behind, samples are dropped.

use anyhow::{Context, Result};
use shadow_sim::RoutingMode;
use shadow_sim::wire::{LinkArrays, Stats};
use tokio::sync::mpsc;
use tracing::{info, warn};

pub struct Sample {
    pub run_id: u32,
    pub t: f64,
    pub routing: RoutingMode,
    pub stats: Stats,
    /// Full per-link arrays, every few seconds only.
    pub links: Option<LinkArrays>,
}

pub struct SinkConfig {
    pub timescale_url: Option<String>,
    pub redis_url: Option<String>,
}

pub async fn start(cfg: SinkConfig) -> Result<Option<mpsc::Sender<Sample>>> {
    if cfg.timescale_url.is_none() && cfg.redis_url.is_none() {
        return Ok(None);
    }
    let timescale = match &cfg.timescale_url {
        Some(url) => Some(Timescale::connect(url).await?),
        None => None,
    };
    let redis = match &cfg.redis_url {
        Some(url) => Some(Redis::connect(url).await?),
        None => None,
    };
    let (tx, mut rx) = mpsc::channel::<Sample>(64);
    tokio::spawn(async move {
        let (mut timescale, mut redis) = (timescale, redis);
        while let Some(s) = rx.recv().await {
            if let Some(ts) = &mut timescale {
                if let Err(e) = ts.write(&s).await {
                    warn!("timescale write failed: {e:#}");
                }
            }
            if let Some(r) = &mut redis {
                if let Err(e) = r.write(&s).await {
                    warn!("redis write failed: {e:#}");
                }
            }
        }
    });
    Ok(Some(tx))
}

fn mode_name(m: RoutingMode) -> &'static str {
    match m {
        RoutingMode::Ospf => "ospf",
        RoutingMode::Dv => "dv",
        RoutingMode::Qrouting => "qrouting",
    }
}

// -- TimescaleDB ----------------------------------------------------------------

struct Timescale {
    client: tokio_postgres::Client,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS net_stats (
    time            TIMESTAMPTZ      NOT NULL DEFAULT now(),
    run_id          INTEGER          NOT NULL,
    sim_t           DOUBLE PRECISION NOT NULL,
    routing         TEXT             NOT NULL,
    offered_gbps    DOUBLE PRECISION NOT NULL,
    delivered_gbps  DOUBLE PRECISION NOT NULL,
    loss_pct        DOUBLE PRECISION NOT NULL,
    mean_ms         DOUBLE PRECISION NOT NULL,
    p99_ms          DOUBLE PRECISION NOT NULL,
    active_flows    INTEGER          NOT NULL,
    broken_flows    INTEGER          NOT NULL,
    links_down      INTEGER          NOT NULL,
    congested_links INTEGER          NOT NULL
);
CREATE TABLE IF NOT EXISTS link_stats (
    time     TIMESTAMPTZ NOT NULL DEFAULT now(),
    run_id   INTEGER     NOT NULL,
    sim_t    DOUBLE PRECISION NOT NULL,
    link_id  INTEGER     NOT NULL,
    util     REAL        NOT NULL,
    queue_ms REAL        NOT NULL,
    loss     REAL        NOT NULL,
    state    SMALLINT    NOT NULL,
    risk     REAL        NOT NULL
);
CREATE INDEX IF NOT EXISTS link_stats_link_time ON link_stats (link_id, time DESC);
"#;

impl Timescale {
    async fn connect(url: &str) -> Result<Self> {
        let (client, conn) = tokio_postgres::connect(url, tokio_postgres::NoTls)
            .await
            .with_context(|| "connecting to TimescaleDB")?;
        tokio::spawn(async move {
            if let Err(e) = conn.await {
                warn!("timescale connection closed: {e}");
            }
        });
        client.batch_execute(SCHEMA).await.context("creating tables")?;
        // Plain Postgres works too; hypertables are a bonus when the
        // extension is there.
        match client.batch_execute("CREATE EXTENSION IF NOT EXISTS timescaledb").await {
            Ok(()) => {
                for t in ["net_stats", "link_stats"] {
                    client
                        .execute(
                            &format!("SELECT create_hypertable('{t}', 'time', if_not_exists => TRUE, migrate_data => TRUE)"),
                            &[],
                        )
                        .await
                        .with_context(|| format!("making {t} a hypertable"))?;
                }
                info!("timescale sink ready (hypertables)");
            }
            Err(e) => info!("timescale extension unavailable ({e}); using plain tables"),
        }
        Ok(Timescale { client })
    }

    async fn write(&mut self, s: &Sample) -> Result<()> {
        let st = &s.stats;
        let broken = (st.flows.unreachable + st.flows.looping + st.flows.blackholed) as i32;
        self.client
            .execute(
                "INSERT INTO net_stats (run_id, sim_t, routing, offered_gbps, delivered_gbps, loss_pct, mean_ms, \
                 p99_ms, active_flows, broken_flows, links_down, congested_links) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
                &[
                    &(s.run_id as i32),
                    &s.t,
                    &mode_name(s.routing),
                    &st.offered_gbps,
                    &st.delivered_gbps,
                    &st.loss_pct,
                    &st.latency_ms.mean,
                    &st.latency_ms.p99,
                    &(st.flows.active as i32),
                    &broken,
                    &(st.links_down as i32),
                    &(st.congested_links as i32),
                ],
            )
            .await?;
        if let Some(l) = &s.links {
            // One statement for every link via UNNEST; a few thousand rows
            // per call is comfortably inside what Postgres likes.
            let ids: Vec<i32> = (0..l.util.len() as i32).collect();
            let f = |v: &[u16], scale: f32| v.iter().map(|&x| x as f32 / scale).collect::<Vec<f32>>();
            let state: Vec<i16> = l.state.iter().map(|&x| x as i16).collect();
            self.client
                .execute(
                    "INSERT INTO link_stats (run_id, sim_t, link_id, util, queue_ms, loss, state, risk) \
                     SELECT $1, $2, * FROM unnest($3::int[], $4::real[], $5::real[], $6::real[], $7::smallint[], $8::real[])",
                    &[
                        &(s.run_id as i32),
                        &s.t,
                        &ids,
                        &f(&l.util, 1000.0),
                        &f(&l.queue_ms, 10.0),
                        &f(&l.loss, 1000.0),
                        &state,
                        &f(&l.risk, 1000.0),
                    ],
                )
                .await?;
        }
        Ok(())
    }
}

// -- Redis -------------------------------------------------------------------------

struct Redis {
    conn: redis::aio::MultiplexedConnection,
}

impl Redis {
    async fn connect(url: &str) -> Result<Self> {
        let client = redis::Client::open(url).context("parsing redis url")?;
        let conn = client
            .get_multiplexed_async_connection()
            .await
            .context("connecting to redis")?;
        info!("redis sink ready");
        Ok(Redis { conn })
    }

    async fn write(&mut self, s: &Sample) -> Result<()> {
        let body = serde_json::json!({
            "runId": s.run_id,
            "t": s.t,
            "routing": mode_name(s.routing),
            "stats": s.stats,
        })
        .to_string();
        redis::pipe()
            .cmd("SET")
            .arg("shadownet:stats:latest")
            .arg(&body)
            .ignore()
            .cmd("PUBLISH")
            .arg("shadownet:stats")
            .arg(&body)
            .ignore()
            .cmd("XADD")
            .arg("shadownet:stats:stream")
            .arg("MAXLEN")
            .arg("~")
            .arg(3600)
            .arg("*")
            .arg("json")
            .arg(&body)
            .ignore()
            .query_async::<()>(&mut self.conn)
            .await?;
        Ok(())
    }
}
