//! Run the chaos monkey for a while and watch the failure predictor learn.
//!
//!     cargo run --release -p shadow-sim --example predictor [minutes] [failures-per-minute]

use shadow_sim::brain::predictor::FEATURE_NAMES;
use shadow_sim::time::SEC;
use shadow_sim::{Command, SimConfig, World};

fn main() {
    let mut args = std::env::args().skip(1);
    let minutes: u64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(10);
    let rate: f64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(40.0);
    let mut w = World::new(SimConfig::default());
    w.apply(Command::ChaosMonkey { enabled: true, per_minute: Some(rate) }).unwrap();
    for m in 1..=minutes {
        if m % 4 == 2 {
            let svc = w.topo.services.iter().position(|s| s.replicas.len() == 1).unwrap_or(0) as u16;
            w.apply(Command::FlashCrowd { service: svc, users: 10_000, mbps: 8.0, ramp_secs: 5.0, secs: 90.0 })
                .unwrap();
        }
        w.advance(60 * SEC);
        w.drain_log();
        let p = w.predictor();
        println!(
            "min {m:>3}  auc {:>5}  base {:>5}  p@10 {:>5}  labelled {:>8}  pos {:>6}",
            p.auc.map_or("-".into(), |x| format!("{x:.3}")),
            p.baseline_auc.map_or("-".into(), |x| format!("{x:.3}")),
            p.precision_at_10().map_or("-".into(), |x| format!("{x:.3}")),
            p.labelled,
            p.positives
        );
    }
    for (n, x) in FEATURE_NAMES.iter().zip(w.predictor().weights()) {
        println!("  {n:<16} {x:+.3}");
    }
}
