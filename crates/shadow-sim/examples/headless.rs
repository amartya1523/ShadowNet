//! Boot the default 100k-node world, run it for a minute of simulated time
//! with a fibre cut in the middle, and print one line per second.
//!
//!     cargo run --release -p shadow-sim --example headless

use std::time::Instant;

use shadow_sim::time::SEC;
use shadow_sim::{Command, SimConfig, World};

fn main() {
    let t0 = Instant::now();
    let mut w = World::new(SimConfig::default());
    println!("boot: {:.2}s wall, {} nodes", t0.elapsed().as_secs_f64(), w.topo.node_count());
    for e in w.drain_log() {
        println!("  [{:>7.2}] {}", e.t, e.text);
    }

    let busiest = (0..w.topo.links.len())
        .max_by(|&a, &b| {
            let u = |i: usize| w.links.state[i].dir.iter().map(|d| d.offered_bps).fold(0.0, f64::max);
            u(a).total_cmp(&u(b))
        })
        .unwrap() as u32;

    let run = Instant::now();
    for sec in 1..=60 {
        if sec == 20 {
            w.apply(Command::FailLink { link: busiest }).unwrap();
        }
        w.advance(SEC);
        let s = w.stats();
        println!(
            "t={:>3}s  {:>6.1}/{:>6.1} Gbps  loss {:>5.2}%  p50 {:>5.1} p99 {:>6.1} ms  flows {:>6}  broken {:>4}  congested {:>3}  max {:.2}",
            sec,
            s.delivered_gbps,
            s.offered_gbps,
            s.loss_pct,
            s.latency_ms.p50,
            s.latency_ms.p99,
            s.flows.active,
            s.flows.unreachable + s.flows.looping + s.flows.blackholed,
            s.congested_links,
            s.max_util
        );
        for e in w.drain_log() {
            println!("  [{:>7.2}] {}", e.t, e.text);
        }
    }
    let wall = run.elapsed().as_secs_f64();
    let c = w.counters();
    println!("60 s simulated in {wall:.2}s wall ({:.1}x real time)", 60.0 / wall);
    println!("{c:?}");
}
