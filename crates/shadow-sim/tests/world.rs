//! End-to-end checks on a small world. Each test boots its own network, so
//! they are independent and can run in parallel.

use std::collections::HashMap;

use shadow_graph::algo::{Csr, Graph, bellman_ford, dijkstra};
use shadow_graph::{LinkKind, RouterId, Topology};
use shadow_sim::time::SEC;
use shadow_sim::{Command, PathStatus, RoutingMode, SimConfig, World};

fn world(seed: u64) -> World {
    World::new(SimConfig::small(seed))
}

fn broken(w: &World) -> usize {
    w.traffic.flows.iter().filter(|f| f.status != PathStatus::Ok).count()
}

/// Internal graph of one AS with local indices, for reference answers.
fn as_graph(
    topo: &Topology,
    asys: usize,
    weight: impl Fn(&shadow_graph::Link) -> u64,
) -> (Csr, HashMap<RouterId, usize>) {
    let a = &topo.ases[asys];
    let idx: HashMap<RouterId, usize> = a.routers.iter().enumerate().map(|(i, &r)| (r, i)).collect();
    let edges = topo
        .links
        .iter()
        .filter(|l| l.kind == LinkKind::Internal)
        .filter_map(|l| Some((*idx.get(&l.a)?, *idx.get(&l.b)?, weight(l), l.id.0)));
    (Csr::undirected(a.routers.len(), edges), idx)
}

#[test]
fn boots_into_a_healthy_network() {
    let mut w = world(1);
    w.advance(3 * SEC);
    let s = w.stats();
    assert!(s.flows.active > 1000, "{s:?}");
    assert_eq!(broken(&w), 0, "flows broken in a healthy network");
    assert!(s.loss_pct < 1.0, "{s:?}");
    assert!(s.latency_ms.p50 > 1.0 && s.latency_ms.p50 < 400.0, "{s:?}");
}

#[test]
fn link_state_converges_to_dijkstra() {
    let mut w = world(2);
    w.advance(SEC);
    let topo = w.topo.clone();
    for a in 0..topo.ases.len() {
        let (g, idx) = as_graph(&topo, a, |l| l.cost as u64);
        for (&r, &i) in &idx {
            let sp = dijkstra(&g, i);
            let info = w.router_info(r);
            for row in &info.fib {
                // Only internal targets have a straightforward reference.
                let Some((&t, &j)) = idx.iter().find(|(t, _)| topo.router(**t).name == row.target) else {
                    continue;
                };
                assert_ne!(t, r);
                assert_eq!(row.cost, Some(sp.dist[j] as u32), "{} -> {}", topo.router(r).name, row.target);
            }
        }
    }
}

#[test]
fn distance_vector_converges_to_hop_counts() {
    let mut w = world(3);
    w.apply(Command::SetRouting { mode: RoutingMode::Dv }).unwrap();
    w.advance(SEC);
    let topo = w.topo.clone();
    for a in 0..topo.ases.len() {
        let (g, idx) = as_graph(&topo, a, |_| 1);
        let mut arcs = Vec::new();
        for u in 0..g.node_count() {
            g.for_each_edge(u, |v, w, _| arcs.push((u, v, w as i64)));
        }
        for (&r, &i) in &idx {
            let bf = bellman_ford(g.node_count(), &arcs, i).unwrap();
            for row in w.router_info(r).fib {
                let Some((_, &j)) = idx.iter().find(|(t, _)| topo.router(**t).name == row.target) else {
                    continue;
                };
                assert_eq!(row.cost.map(|c| c as i64), bf.dist[j], "{} -> {}", topo.router(r).name, row.target);
            }
        }
    }
}

#[test]
fn single_link_failure_is_survived() {
    let mut w = world(4);
    w.advance(2 * SEC);
    // Busiest internal link that has an alternative inside its AS.
    let topo = w.topo.clone();
    let victim = topo
        .links
        .iter()
        .filter(|l| l.kind == LinkKind::Internal)
        .filter(|l| {
            let (g, idx) = as_graph(&topo, topo.as_of(l.a).idx(), |x| if x.id == l.id { u64::MAX / 4 } else { 1 });
            let sp = dijkstra(&g, idx[&l.a]);
            sp.dist[idx[&l.b]] < u64::MAX / 4
        })
        .max_by(|a, b| {
            let u = |l: &shadow_graph::Link| w.links.state[l.id.idx()].dir.iter().map(|d| d.util).fold(0.0, f64::max);
            u(a).total_cmp(&u(b))
        })
        .unwrap()
        .id;
    w.apply(Command::FailLink { link: victim.0 }).unwrap();
    w.advance(5 * SEC);
    assert_eq!(broken(&w), 0, "traffic did not reroute around {victim}");
    let inc = &w.incidents()[0];
    let igp = inc.igp_ms.expect("IGP should have reacted");
    // Detection (150 ms) + flooding + SPF throttle (50 ms) + FIB (20 ms).
    assert!((150.0..1500.0).contains(&igp), "IGP convergence {igp} ms");
}

#[test]
fn router_crash_and_recovery() {
    let mut w = world(5);
    w.advance(2 * SEC);
    let core = w
        .topo
        .routers
        .iter()
        .find(|r| r.role == shadow_graph::RouterRole::Core && w.topo.neighbors(r.id).len() >= 3)
        .unwrap()
        .id;
    w.apply(Command::FailRouter { router: core.0 }).unwrap();
    w.advance(8 * SEC);
    assert_eq!(w.stats().routers_down, 1);
    w.apply(Command::RestoreRouter { router: core.0 }).unwrap();
    w.advance(10 * SEC);
    assert_eq!(w.stats().routers_down, 0);
    assert_eq!(w.stats().links_down, 0);
    assert_eq!(broken(&w), 0, "network did not heal after the router came back");
}

#[test]
fn hosting_as_outage_withdraws_and_recovers() {
    let mut w = world(6);
    w.advance(2 * SEC);
    let svc = &w.topo.services[0];
    let asys = w.topo.as_of(svc.replicas[0]);
    w.apply(Command::FailAs { asys: asys.0 }).unwrap();
    w.advance(10 * SEC);
    // Flows to the dead AS are unreachable; nobody else is.
    for f in &w.traffic.flows {
        let touches = w.topo.as_of(f.src) == asys || w.topo.as_of(f.dst) == asys;
        if !touches {
            assert_eq!(f.status, PathStatus::Ok, "collateral damage on flow {}", f.id);
        }
    }
    w.apply(Command::RestoreAs { asys: asys.0 }).unwrap();
    w.advance(15 * SEC);
    assert_eq!(broken(&w), 0);
}

#[test]
fn q_routing_forwards_without_loops() {
    let mut w = world(7);
    w.apply(Command::SetRouting { mode: RoutingMode::Qrouting }).unwrap();
    w.advance(5 * SEC);
    let s = w.stats();
    assert_eq!(s.flows.looping, 0, "{s:?}");
    assert_eq!(broken(&w), 0, "{s:?}");
}

#[test]
fn same_seed_same_numbers() {
    let run = || {
        let mut w = world(8);
        w.apply(Command::ChaosMonkey { enabled: true, per_minute: Some(30.0) }).unwrap();
        w.advance(20 * SEC);
        let s = w.stats();
        (s.links_down, (s.delivered_gbps * 1e6).round() as i64, w.events_processed())
    };
    assert_eq!(run(), run());
}

#[test]
fn flash_crowd_shows_up_as_load() {
    let mut w = world(9);
    w.advance(2 * SEC);
    let before = w.stats().offered_gbps;
    w.apply(Command::FlashCrowd { service: 0, users: 2000, mbps: 8.0, ramp_secs: 1.0, secs: 20.0 }).unwrap();
    w.advance(3 * SEC);
    let s = w.stats();
    assert_eq!(s.flows.crowd, 2000);
    assert!(s.offered_gbps > before + 5.0, "{before} -> {}", s.offered_gbps);
}
