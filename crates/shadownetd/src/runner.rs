//! The simulation thread.
//!
//! The world is single-threaded by design, so it lives on its own OS thread
//! and everything else talks to it through a channel. Requests are either
//! commands or closures that read from the world; frames come back out
//! through a broadcast channel, serialised once.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use shadow_sim::time::MS;
use shadow_sim::wire::{Frame, LogEvent, TopologyView};
use shadow_sim::{Command, Scenario, SimConfig, World};
use tokio::sync::{broadcast, oneshot};
use tracing::{info, warn};

use crate::sinks::Sample;

pub type Query = Box<dyn FnOnce(&World) -> serde_json::Value + Send>;

pub enum Request {
    Command(Command, oneshot::Sender<Result<(), String>>),
    Query(Query, oneshot::Sender<serde_json::Value>),
}

/// What the HTTP side shares. Cheap to clone.
#[derive(Clone)]
pub struct Handle {
    tx: mpsc::Sender<Request>,
    pub frames: broadcast::Sender<Arc<str>>,
    pub topology: Arc<RwLock<Arc<str>>>,
    pub run_id: Arc<AtomicU32>,
    pub scenarios: Arc<Vec<Scenario>>,
    recent: Arc<Mutex<VecDeque<LogEvent>>>,
}

/// Events kept for clients that connect late.
const RECENT_EVENTS: usize = 60;

impl Handle {
    pub async fn command(&self, cmd: Command) -> Result<(), String> {
        let (tx, rx) = oneshot::channel();
        self.tx.send(Request::Command(cmd, tx)).map_err(|_| "simulation thread is gone".to_string())?;
        rx.await.map_err(|_| "simulation thread dropped the request".to_string())?
    }

    pub async fn query<F>(&self, f: F) -> Option<serde_json::Value>
    where
        F: FnOnce(&World) -> serde_json::Value + Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        self.tx.send(Request::Query(Box::new(f), tx)).ok()?;
        rx.await.ok()
    }

    pub fn hello(&self) -> String {
        let recent: Vec<LogEvent> = self.recent.lock().unwrap().iter().cloned().collect();
        serde_json::json!({
            "type": "hello",
            "version": 1,
            "runId": self.run_id.load(Ordering::Relaxed),
            "recent": recent,
        })
        .to_string()
    }

}

pub struct RunnerConfig {
    pub sim: SimConfig,
    pub speed: f64,
    pub frame_interval: Duration,
}

/// Spawns the simulation thread and returns a handle to it.
pub fn spawn(cfg: RunnerConfig, scenarios: Vec<Scenario>, sinks: Option<tokio::sync::mpsc::Sender<Sample>>) -> Handle {
    let (tx, rx) = mpsc::channel();
    let (frames, _) = broadcast::channel(32);
    let started = Instant::now();
    let world = World::new(cfg.sim.clone());
    info!(
        nodes = world.topo.node_count(),
        routers = world.topo.routers.len(),
        links = world.topo.links.len(),
        "world ready in {:.0?}",
        started.elapsed()
    );
    let handle = Handle {
        tx,
        frames: frames.clone(),
        topology: Arc::new(RwLock::new(topology_json(&world, 1))),
        run_id: Arc::new(AtomicU32::new(1)),
        scenarios: Arc::new(scenarios),
        recent: Arc::new(Mutex::new(VecDeque::new())),
    };
    let mut runner = Runner {
        world,
        cfg,
        rx,
        handle: handle.clone(),
        sinks,
        speed: 1.0,
        paused: false,
        seq: 0,
        sim_rate: 0.0,
        last_sample_sec: -1,
    };
    runner.speed = runner.cfg.speed;
    std::thread::Builder::new()
        .name("sim".into())
        .spawn(move || runner.run())
        .expect("failed to spawn the simulation thread");
    handle
}

fn topology_json(world: &World, run_id: u32) -> Arc<str> {
    serde_json::to_string(&TopologyView::new(&world.topo, run_id)).expect("topology serialises").into()
}

struct Runner {
    world: World,
    cfg: RunnerConfig,
    rx: mpsc::Receiver<Request>,
    handle: Handle,
    sinks: Option<tokio::sync::mpsc::Sender<Sample>>,
    speed: f64,
    paused: bool,
    seq: u64,
    sim_rate: f64,
    last_sample_sec: i64,
}

/// Never try to catch up more than this much simulated time in one go. If
/// the machine cannot keep up we slow down rather than spiral.
const MAX_BACKLOG_S: f64 = 0.5;
/// Share of each frame interval we are willing to spend simulating.
const BUSY_SHARE: f64 = 0.8;

impl Runner {
    fn run(mut self) {
        let tick = 50 * MS;
        let mut last = Instant::now();
        let mut backlog_s = 0.0f64;
        let mut next_frame = Instant::now();
        let mut rate_window = (Instant::now(), self.world.t());

        loop {
            // Drain everything that is waiting before doing any work.
            loop {
                match self.rx.try_recv() {
                    Ok(req) => self.handle(req),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => return,
                }
            }

            let now = Instant::now();
            if !self.paused {
                backlog_s =
                    (backlog_s + (now - last).as_secs_f64() * self.speed).min(MAX_BACKLOG_S * self.speed.max(1.0));
            }
            last = now;

            let budget = self.cfg.frame_interval.mul_f64(BUSY_SHARE);
            let started = Instant::now();
            while backlog_s >= 0.05 && started.elapsed() < budget {
                self.world.advance(tick);
                backlog_s -= 0.05;
            }

            let elapsed = rate_window.0.elapsed().as_secs_f64();
            if elapsed >= 1.0 {
                let sim = self.world.t() - rate_window.1;
                self.sim_rate = sim / elapsed;
                rate_window = (Instant::now(), self.world.t());
            }

            if Instant::now() >= next_frame {
                self.publish();
                next_frame += self.cfg.frame_interval;
                if next_frame < Instant::now() {
                    next_frame = Instant::now() + self.cfg.frame_interval;
                }
            }

            let wait = next_frame
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(if self.paused { 50 } else { 4 }));
            match self.rx.recv_timeout(wait) {
                Ok(req) => self.handle(req),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    fn handle(&mut self, req: Request) {
        match req {
            Request::Query(f, reply) => {
                let _ = reply.send(f(&self.world));
            }
            Request::Command(cmd, reply) => {
                let _ = reply.send(self.command(cmd));
            }
        }
    }

    fn command(&mut self, cmd: Command) -> Result<(), String> {
        match cmd {
            Command::SetSpeed { speed } => {
                if !(0.05..=64.0).contains(&speed) {
                    return Err("speed must be between 0.05 and 64".into());
                }
                self.speed = speed;
                Ok(())
            }
            Command::Pause => {
                self.paused = true;
                Ok(())
            }
            Command::Resume => {
                self.paused = false;
                Ok(())
            }
            Command::Reset { seed } => {
                let mut sim = self.cfg.sim.clone();
                if let Some(seed) = seed {
                    sim.topology.seed = seed;
                }
                let started = Instant::now();
                self.world = World::new(sim.clone());
                self.cfg.sim = sim;
                let run_id = self.handle.run_id.fetch_add(1, Ordering::Relaxed) + 1;
                *self.handle.topology.write().unwrap() = topology_json(&self.world, run_id);
                self.last_sample_sec = -1;
                self.handle.recent.lock().unwrap().clear();
                info!(run_id, "reset in {:.0?}", started.elapsed());
                let _ = self.handle.frames.send(self.handle.hello().into());
                Ok(())
            }
            Command::RunScenario { name } => {
                let s = self
                    .handle
                    .scenarios
                    .iter()
                    .find(|s| s.name == name)
                    .cloned()
                    .ok_or_else(|| format!("no scenario named {name:?}"))?;
                info!(scenario = %s.name, "starting scenario");
                self.world.start_scenario(s);
                Ok(())
            }
            other => self.world.apply(other),
        }
    }

    fn publish(&mut self) {
        self.seq += 1;
        let w = &mut self.world;
        let frame = Frame {
            kind: "frame",
            run_id: self.handle.run_id.load(Ordering::Relaxed),
            seq: self.seq,
            t: (w.t() * 1000.0).round() / 1000.0,
            paused: self.paused,
            speed: self.speed,
            sim_rate: (self.sim_rate * 100.0).round() / 100.0,
            routing: w.mode(),
            chaos_monkey: w.monkey_enabled(),
            stats: w.stats(),
            links: w.link_arrays(),
            routers_down: w.routers_down(),
            flows: w.flow_sample(),
            events: w.drain_log(),
            incidents: w.incidents(),
            ai: w.ai(),
        };

        {
            let mut recent = self.handle.recent.lock().unwrap();
            recent.extend(frame.events.iter().cloned());
            let excess = recent.len().saturating_sub(RECENT_EVENTS);
            recent.drain(..excess);
        }

        if let Some(sinks) = &self.sinks {
            let sec = frame.t.floor() as i64;
            if sec != self.last_sample_sec {
                self.last_sample_sec = sec;
                let links = (sec % 5 == 0).then(|| frame.links.clone());
                let sample = Sample {
                    run_id: frame.run_id,
                    t: frame.t,
                    routing: frame.routing,
                    stats: frame.stats.clone(),
                    links,
                };
                if sinks.try_send(sample).is_err() {
                    warn!("metrics sink is falling behind; dropping a sample");
                }
            }
        }

        match serde_json::to_string(&frame) {
            // No receivers is fine: nobody has the dashboard open.
            Ok(json) => {
                let _ = self.handle.frames.send(json.into());
            }
            Err(e) => warn!("frame did not serialise: {e}"),
        }
    }
}
