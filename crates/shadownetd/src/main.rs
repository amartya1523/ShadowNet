use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use shadow_graph::TopologyConfig;
use shadow_sim::time::SEC;
use shadow_sim::wire::TopologyView;
use shadow_sim::{RoutingMode, SimConfig, World};
use tracing::info;
use tracing_subscriber::EnvFilter;

mod api;
mod bench;
mod runner;
mod scenarios;
mod sinks;

#[derive(Parser)]
#[command(name = "shadownetd", version, about = "A hundred thousand machines that don't exist.")]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the simulation and serve the API and dashboard (the default).
    Serve(ServeArgs),
    /// Run a scenario headless and print what happens.
    Run {
        scenario: String,
        #[arg(long, default_value_t = TopologyConfig::default().seed)]
        seed: u64,
        #[arg(long, value_enum)]
        routing: Option<Routing>,
    },
    /// Reproduce the benchmark tables from the README.
    Bench {
        #[arg(long, default_value = "all")]
        suite: String,
        #[arg(long, default_value_t = TopologyConfig::default().seed)]
        seed: u64,
        /// Write the markdown here as well as to stdout.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Print the generated topology as JSON (what /api/topology serves).
    Topology {
        #[arg(long, default_value_t = TopologyConfig::default().seed)]
        seed: u64,
        #[arg(long, default_value_t = 100_000)]
        nodes: usize,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum Routing {
    Ospf,
    Dv,
    Qrouting,
}

impl From<Routing> for RoutingMode {
    fn from(r: Routing) -> Self {
        match r {
            Routing::Ospf => RoutingMode::Ospf,
            Routing::Dv => RoutingMode::Dv,
            Routing::Qrouting => RoutingMode::Qrouting,
        }
    }
}

#[derive(Args, Default)]
struct ServeArgs {
    #[arg(long, default_value = "127.0.0.1:7070", env = "SHADOWNET_ADDR")]
    addr: Option<SocketAddr>,
    #[arg(long)]
    seed: Option<u64>,
    /// Total nodes: routers + hosts + servers.
    #[arg(long, default_value_t = 100_000)]
    nodes: usize,
    /// Background flows alive at once.
    #[arg(long, default_value_t = 30_000)]
    flows: usize,
    /// Simulated seconds per wall second.
    #[arg(long, default_value_t = 1.0)]
    speed: f64,
    #[arg(long, value_enum)]
    routing: Option<Routing>,
    /// Built dashboard to serve at /. Defaults to dashboard/out if present.
    #[arg(long = "static")]
    static_dir: Option<PathBuf>,
    /// Extra scenario directory; overrides built-ins with the same name.
    #[arg(long)]
    scenarios: Option<PathBuf>,
    /// e.g. postgres://shadownet:shadownet@localhost:5432/shadownet
    #[arg(long, env = "SHADOWNET_TIMESCALE_URL")]
    timescale: Option<String>,
    /// e.g. redis://localhost:6379
    #[arg(long, env = "SHADOWNET_REDIS_URL")]
    redis: Option<String>,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,tower_http=warn")))
        .with_target(false)
        .init();

    let cli = Cli::parse();
    match cli.cmd.unwrap_or(Cmd::Serve(ServeArgs {
        addr: Some("127.0.0.1:7070".parse().unwrap()),
        nodes: 100_000,
        flows: 30_000,
        speed: 1.0,
        ..ServeArgs::default()
    })) {
        Cmd::Serve(args) => serve(args),
        Cmd::Run { scenario, seed, routing } => run_headless(&scenario, seed, routing.map(Into::into)),
        Cmd::Bench { suite, seed, out } => {
            let md = bench::run(&suite, seed);
            print!("{md}");
            if let Some(path) = out {
                std::fs::write(&path, &md).with_context(|| format!("writing {}", path.display()))?;
            }
            Ok(())
        }
        Cmd::Topology { seed, nodes } => {
            let topo = shadow_graph::Topology::generate(&TopologyConfig {
                seed,
                nodes,
                ..TopologyConfig::default()
            });
            println!("{}", serde_json::to_string_pretty(&TopologyView::new(&topo, 1))?);
            Ok(())
        }
    }
}

fn serve(args: ServeArgs) -> Result<()> {
    let mut sim = SimConfig::default();
    sim.topology.nodes = args.nodes;
    if let Some(seed) = args.seed {
        sim.topology.seed = seed;
    }
    sim.traffic.concurrent = args.flows;
    if let Some(r) = args.routing {
        sim.routing = r.into();
    }
    let scenarios = scenarios::load(args.scenarios.as_deref())?;
    let static_dir = args
        .static_dir
        .or_else(|| Some(PathBuf::from("dashboard/out")).filter(|p| p.join("index.html").exists()));
    let addr = args.addr.unwrap_or_else(|| "127.0.0.1:7070".parse().unwrap());

    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    rt.block_on(async move {
        let sinks = sinks::start(sinks::SinkConfig {
            timescale_url: args.timescale,
            redis_url: args.redis,
        })
        .await?;
        let handle = runner::spawn(
            runner::RunnerConfig {
                sim,
                speed: args.speed,
                frame_interval: Duration::from_millis(200),
            },
            scenarios,
            sinks,
        );
        match &static_dir {
            Some(d) => info!("serving dashboard from {}", d.display()),
            None => info!("no dashboard build found; API only"),
        }
        let app = api::router(handle, static_dir);
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .with_context(|| format!("binding {addr}"))?;
        info!("listening on http://{addr}");
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await?;
        Ok(())
    })
}

fn run_headless(name: &str, seed: u64, routing: Option<RoutingMode>) -> Result<()> {
    let scenario = scenarios::load(None)?
        .into_iter()
        .find(|s| s.name == name)
        .with_context(|| format!("no scenario named {name:?}"))?;
    let mut cfg = SimConfig::default();
    cfg.topology.seed = seed;
    if let Some(r) = routing {
        cfg.routing = r;
    }
    let mut w = World::new(cfg);
    for e in w.drain_log() {
        println!("[{:>7.2}] {}", e.t, e.text);
    }
    let secs = scenario.duration.ceil() as u64;
    w.start_scenario(scenario);
    for _ in 0..secs {
        w.advance(SEC);
        let s = w.stats();
        for e in w.drain_log() {
            println!("[{:>7.2}] {:<5} {}", e.t, e.level, e.text);
        }
        println!(
            "          t={:>5.1}  {:>6.1} Gbps  loss {:>5.2}%  p50 {:>5.1} ms  p99 {:>6.1} ms  broken {:>5}  down {:>3}",
            w.t(),
            s.delivered_gbps,
            s.loss_pct,
            s.latency_ms.p50,
            s.latency_ms.p99,
            s.flows.unreachable + s.flows.looping + s.flows.blackholed,
            s.links_down
        );
    }
    Ok(())
}
