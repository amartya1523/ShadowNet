# ShadowNet design notes

These are the notes I wrote before writing any code, kept up to date as
the implementation drifted. If something here disagrees with the code, the
code wins and this file has a bug.

## 1. What problem are we actually solving?

"Simulate the internet" is not a specification. Stripped down, the questions
we want to be able to ask are:

1. A large provider loses a router, a fibre or a whole datacenter. How long
   until traffic flows again, where does it go, and what breaks on the way?
2. Ten thousand users hit one site at once. Which links melt first, and is
   the bottleneck the server's uplink or something three networks away?
3. Can a learning agent route around congestion better than shortest-path
   routing does, and under which conditions is it worse?
4. Can we see a failure coming from the telemetry a real operator would have?

Every one of those is about *routing state* and *aggregate load* evolving
over time. None of them needs per-packet fidelity on every wire. That single
observation drives the whole design.

## 2. The scale budget

Target: 100 000 nodes on a laptop (8 cores, 8 GB), running at or faster than
real time, while streaming state to a browser.

A packet-level simulator (ns-3 style) moving 100 Gbit/s of simulated traffic
handles ~10^7 packet events per simulated second. Not happening in real time
on one machine. So we split the world in two:

| Layer               | Count      | Modelled as                                   |
|---------------------|------------|-----------------------------------------------|
| Hosts and servers   | ~98 000    | logical endpoints with an access rate          |
| Routers             | ~1 700     | full protocol state machines (OSPF/DV, BGP)    |
| Links               | ~3 500     | fluid queues with capacity, buffer, delay      |
| Autonomous systems  | ~280       | one BGP speaker each                           |
| Flows               | 10^4–10^5  | rate-based demands following forwarding state  |

Control plane: discrete-event and exact. Every LSA, every BGP update, every
SPF run is an event with a timestamp. Convergence times and transient loops
come out of the simulation; they are not hard-coded.

Data plane: fluid. Every `tick` (50 ms simulated) each flow is walked hop by
hop through the *current forwarding tables of the routers it visits* (not a
global oracle), its demand is added to each link, and link queues integrate
`backlog += (offered - capacity) * dt`. Queueing delay is `backlog / capacity`
and loss is whatever overflows the buffer. This captures buffer build-up and
drain, latency spikes, and collateral damage between flows sharing a link,
which is exactly what the questions above need.

Because flows walk per-router FIBs, an inconsistent network during
convergence naturally produces micro-loops (TTL expiry) and black holes. Those
are counted and shown. This is the most important property of the model.

## 3. Topology

Generated, seeded and deterministic. Roughly the shape of the real thing:

* **Tier-1** ASes: global backbones with POPs in many cities, full-mesh
  settlement-free peering with each other.
* **Tier-2** ASes: regional, buy transit from two or three tier-1s, peer with
  some tier-2s in the same region.
* **Stub** ASes: access networks and hosting, buy transit from one or two
  tier-2s (sometimes a tier-1). Hosts hang off their access routers.

Routers sit in real cities (equirectangular projection for the UI) with a
small jitter. Link propagation delay is great-circle distance over fibre
(~4.9 µs/km plus a route-inflation factor), so a New York to London hop costs
about what it does in reality.

Some stub ASes host *services*: a server farm with fat access links and a
popularity weight (Zipf). Background traffic is mostly users talking to
services, which gives the traffic matrix the right skew.

Each link also gets a hidden `fragility` drawn from a log-normal. It is never
exported to the predictor; it only feeds the ground-truth failure process.

## 4. Routing

### Intra-AS

Three interchangeable modes, selectable at runtime per simulation:

* **Link state (OSPF-like).** Routers originate LSAs listing their live
  adjacencies and their external (inter-AS) links. LSAs are flooded with real
  per-hop delays and sequence numbers. On LSDB change a router schedules SPF
  behind a throttle timer, runs Dijkstra *on its own LSDB*, and installs its own
  FIB row after a FIB-programming delay. Egress selection is hot-potato:
  nearest border router (by IGP cost) with a live link to the next-hop AS.
* **Distance vector (RIP-like).** Distributed Bellman-Ford with split horizon
  and poisoned reverse, triggered plus periodic updates, infinity = 16 hops
  scaled to cost. Present mostly to show why nobody runs it on a backbone:
  slower convergence and counting to infinity on some failures.
* **Q-routing (learning).** Boyan & Littman (1994), full-echo variant. Each
  router `x` keeps `Q_x(T, y)`, the estimated delivery time to target `T`
  through action `y` (an intra-AS neighbour, or an inter-AS link when `T` is
  "exit to AS B"). Every tick:
  `Q_x(T,y) <- (1-a) Q_x(T,y) + a (d(x,y) + min_z Q_y(T,z))`
  where `d` is the *measured* link delay, including queueing, plus a loss
  penalty. Forwarding does a soft-min over actions within a slack of the best,
  hashed per flow so a flow sticks to its choice unless the values move. That
  spreads load across near-equal paths, which is where it beats hot-potato.
  Warm-started from IGP distances so switching modes is not a cliff.

### Inter-AS

Simplified BGP, one speaker per AS, one prefix per AS:

* Session between two ASes is up if any link between them is up, after a
  hold-timer detection delay.
* Decision process: local preference by relationship (customer > peer >
  provider), shortest AS path, lowest neighbour ASN.
* Export follows Gao-Rexford: everything to customers; only own and customer
  routes to peers and providers. This keeps the AS graph valley-free and
  guarantees convergence.
* Updates and withdrawals are batched per neighbour behind an MRAI timer, so
  path exploration after a withdrawal shows up as it does in the wild.

BGP timers are scaled down (MRAI 1 s, not 30 s) so experiments fit in a
minute. The scaling factor is documented in the scenario output.

## 5. Traffic

* Background: Poisson flow arrivals, log-normal durations and rates, 70%
  user-to-service (Zipf over services), 30% host-to-host.
* Flash crowd: N users from M stub networks open flows to one service over a
  ramp window.
* DDoS: a botnet of hosts spread over many ASes sends high-rate flows at a
  victim. Attack flows are tagged so collateral damage can be measured.

Per flow, per tick: walk path, record hops; throughput is
`demand * min over hops (1 - loss_i)` capped by access rates; latency is the
sum of hop delays. Network-wide percentiles are flow-weighted.

## 6. Chaos

Scripted (scenario TOML) or interactive (dashboard):

* link cut / restore, router crash / recover, whole-AS outage
* latency spike (+N ms for D seconds), packet loss injection
* DDoS and flash crowds
* "Chaos monkey": a hazard process per link,
  `lambda = base * fragility * (1 + k1 * max(0, util_ewma - 0.75)^2 + k2 * recent_flaps)`.
  Overloaded, flappy, fragile links fail more. This is synthetic, and the
  README says so, but it gives the predictor something real to find.

## 7. Failure prediction

Online logistic regression over per-link features a NOC would have:
utilisation EWMA (fast and slow), peak utilisation, queue occupancy, loss,
latency jitter, flap count in the last minute, time since last failure, and
static betweenness. Labels are "this link went down within the next H
seconds". Feature snapshots go into a ring buffer and are labelled once the
horizon passes, so the model only ever learns from the past. Reported
online: ROC-AUC over a sliding window, precision@10, against a
utilisation-only baseline.

## 8. Process layout

```
             commands (mpsc)            Arc<Frame> (watch)
 HTTP/WS  ------------------>  sim thread  ------------------>  WS clients
 (tokio)                        owns World                      metrics sinks
                                                                (Redis, Timescale)
```

The simulation runs on one dedicated thread that owns all state. No locks in
the hot path. It paces simulated time against the wall clock (speed factor),
and every 100 ms of wall time it builds a `Frame`, serialises it once, and
publishes it. Every client gets the same bytes.

## 9. Crates

* `shadow-graph`: ids, deterministic RNG, geography, topology generation,
  and the pure algorithms (Dijkstra, Bellman-Ford, Brandes betweenness). No
  dependencies, heavily tested.
* `shadow-sim`: event queue, protocol state machines, traffic engine, chaos,
  learning agents, frames.
* `shadownetd`: the binary. HTTP and WebSocket API, scenario runner, metrics
  sinks, benchmark harness.

## 10. Things I decided not to do

* No TCP congestion control. Flows are open-loop at their demand. Modelling
  AIMD in a fluid system is doable but it would hide the routing effects we
  care about behind TCP's own adaptation.
* No iBGP, route reflectors, MED, communities. One speaker per AS.
* No IP addressing. Prefix = AS. Longest-prefix match would add nothing to the
  questions in section 1.
* Hosts are not drawn individually in the UI. 98 000 dots is noise.
