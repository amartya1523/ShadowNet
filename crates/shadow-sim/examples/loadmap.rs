//! Utilisation distribution by link class after warm-up. A tuning aid.
//!
//!     cargo run --release -p shadow-sim --example loadmap

use shadow_graph::{LinkKind, Tier};
use shadow_sim::time::SEC;
use shadow_sim::{SimConfig, World};

fn main() {
    let mut w = World::new(SimConfig::default());
    w.advance(5 * SEC);
    let topo = w.topo.clone();
    let mut classes: std::collections::BTreeMap<String, Vec<f64>> = Default::default();
    for l in &topo.links {
        let s = &w.links.state[l.id.idx()];
        let u = s.dir[0].util.max(s.dir[1].util);
        let ta = topo.asys(topo.as_of(l.a)).tier;
        let tb = topo.asys(topo.as_of(l.b)).tier;
        let hosting = topo.asys(topo.as_of(l.a)).hosting || topo.asys(topo.as_of(l.b)).hosting;
        let key = format!(
            "{:?} {:?}-{:?}{} {:.0}G",
            l.kind,
            ta.min(tb),
            ta.max(tb),
            if hosting { " (hosting)" } else { "" },
            l.capacity_bps / 1e9
        );
        classes.entry(key).or_default().push(u);
    }
    println!("{:<48} {:>5} {:>6} {:>6} {:>6}", "class", "n", "mean", "p90", "max");
    for (k, mut v) in classes {
        v.sort_by(|a, b| a.total_cmp(b));
        let p = |q: f64| v[((v.len() - 1) as f64 * q) as usize];
        let mean = v.iter().sum::<f64>() / v.len() as f64;
        println!("{:<48} {:>5} {:>6.2} {:>6.2} {:>6.2}", k, v.len(), mean, p(0.9), p(1.0));
    }
    let _ = Tier::Stub;
    let _ = LinkKind::Internal;
}
