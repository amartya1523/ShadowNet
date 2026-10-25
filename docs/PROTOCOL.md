# shadownetd wire protocol

Everything the dashboard knows comes through here. JSON over HTTP for
request/response, JSON over one WebSocket for the live stream. Default
address `http://127.0.0.1:7070`.

Field names are camelCase. Times are simulated seconds since the run started
unless the name says otherwise. Ids are dense integers and index straight
into the arrays returned by `/api/topology`.

## GET /api/topology

Sent once per run. Changes only after a `reset`.

```jsonc
{
  "runId": 3,                      // bumps on every reset
  "seed": 98430951,
  "summary": { "nodes": 100000, "routers": 1712, "hosts": 97520, "servers": 768,
               "links": 3391, "tier1": 6, "tier2": 36, "stubs": 240, "services": 16 },
  "cities":   [{ "code": "lon", "name": "London", "lat": 51.51, "lon": -0.13, "region": "EU" }],
  "ases":     [{ "id": 0, "asn": 64512, "name": "Meridian Global Backbone",
                 "tier": "tier1" | "tier2" | "stub", "region": "EU", "home": "lon",
                 "hosting": false }],
  "routers":  [{ "id": 0, "as": 0, "name": "lon-cr1.as64512",
                 "role": "core" | "border" | "access" | "datacenter",
                 "city": "lon", "lat": 51.43, "lon": -0.21, "hosts": 0 }],
  "links":    [{ "id": 0, "a": 0, "b": 1, "kind": "internal" | "peering" | "transit",
                 "capacityGbps": 400, "delayMs": 0.41, "km": 33 }],
  "services": [{ "id": 0, "name": "vidstream", "replicas": [812, 1290], "popularity": 0.29 }]
}
```

## WebSocket /ws

Server to client only (commands go over HTTP). Text frames, one JSON object
each, discriminated by `type`.

### hello

```jsonc
{ "type": "hello", "version": 1, "runId": 3,
  "recent": [ /* last 60 events, same shape as frame.events */ ] }
```

If `runId` differs from the topology you hold, refetch `/api/topology`.

### frame (about 5 per second)

```jsonc
{
  "type": "frame",
  "runId": 3,
  "seq": 1042,
  "t": 208.4,
  "paused": false,
  "speed": 1.0,          // requested sim seconds per wall second
  "simRate": 0.98,       // achieved
  "routing": "ospf" | "dv" | "qrouting",
  "chaosMonkey": false,

  "stats": {
    "offeredGbps": 142.1, "deliveredGbps": 139.8, "lossPct": 1.6,
    "latencyMs": { "mean": 61.2, "p50": 48.0, "p95": 141.5, "p99": 188.0 },
    "flows": { "active": 30412, "unreachable": 0, "looping": 0, "blackholed": 12,
               "crowd": 0, "attack": 0 },
    "linksDown": 1, "routersDown": 0, "congestedLinks": 7, "maxUtil": 1.0,
    "eventsPerSec": 18234
  },

  // Parallel arrays, one entry per link id. Integers to keep frames small.
  "links": {
    "util":    [312, 0, 1000],   // permille of capacity, busier direction; >1000 = offered above capacity
    "queueMs": [0, 0, 412],      // queueing delay, tenths of a millisecond, busier direction
    "loss":    [0, 0, 87],       // permille
    "state":   [0, 1, 2],        // 0 up, 1 down, 2 degraded (spike/loss injected), 3 down but not yet detected
    "risk":    [12, 0, 640]      // predictor output, permille (0 for down links)
  },
  "routersDown": [17],

  // A few hundred live flows for the packet animation. path is router ids,
  // source first.
  "flows": [{ "id": 991, "kind": "bg" | "crowd" | "attack", "path": [12, 40, 41, 300],
              "latencyMs": 72.4, "lossPct": 0.0, "mbps": 4.2 }],

  // Events since the previous frame.
  "events": [{ "t": 208.31, "level": "info" | "warn" | "crit", "kind": "link_down",
               "text": "fra-cr1.as64512 <-> ams-cr2.as64512 down (fibre cut)" }],

  // Most recent first, at most 8. An incident opens on a topology change and
  // closes once the control plane has been quiet for 2 s.
  "incidents": [{ "id": 4, "cause": "link 211 down", "t0": 201.0, "open": false,
                  "igpMs": 182.0, "bgpMs": 2140.0, "peakLossPct": 6.1,
                  "blackholedFlowSecs": 31.2 }],

  "ai": {
    "predictor": { "auc": 0.83, "baselineAuc": 0.61, "precisionAt10": 0.4,
                   "labelled": 120400, "positives": 311,
                   "top": [{ "link": 211, "risk": 0.61 }] },
    "qrouting":  { "updatesPerSec": 2.1e6, "meanAbsTdMs": 0.8 }
  }
}
```

## GET /api/history

One point per simulated second, last 600 s.

```jsonc
{ "t": [..], "deliveredGbps": [..], "offeredGbps": [..], "lossPct": [..],
  "meanMs": [..], "p99Ms": [..], "activeFlows": [..], "linksDown": [..] }
```

## POST /api/command

Body is one command. Response `{ "ok": true }` or `{ "ok": false, "error": "..." }`.

```jsonc
{ "cmd": "fail_link", "link": 12 }
{ "cmd": "restore_link", "link": 12 }
{ "cmd": "fail_router", "router": 5 }
{ "cmd": "restore_router", "router": 5 }
{ "cmd": "fail_as", "as": 3 }
{ "cmd": "restore_as", "as": 3 }
{ "cmd": "latency_spike", "link": 12, "ms": 80, "secs": 20 }
{ "cmd": "packet_loss", "link": 12, "pct": 5, "secs": 20 }
{ "cmd": "ddos", "service": 2, "bots": 3000, "mbps": 30, "secs": 40 }
{ "cmd": "flash_crowd", "service": 4, "users": 10000, "mbps": 8, "rampSecs": 5, "secs": 60 }
{ "cmd": "chaos_monkey", "enabled": true, "perMinute": 6 }
{ "cmd": "set_routing", "mode": "ospf" | "dv" | "qrouting" }
{ "cmd": "set_speed", "speed": 2.0 }
{ "cmd": "pause" }
{ "cmd": "resume" }
{ "cmd": "heal_all" }
{ "cmd": "reset", "seed": 123 }              // seed optional
{ "cmd": "run_scenario", "name": "transatlantic-cut" }
```

## GET /api/scenarios

```jsonc
[{ "name": "transatlantic-cut", "title": "Cut the Atlantic",
   "description": "...", "durationSecs": 90 }]
```

## GET /api/trace?src=ROUTER&dst=ROUTER

Walks the live forwarding state, exactly as a flow would.

```jsonc
{ "status": "ok" | "unreachable" | "loop" | "blackhole",
  "hops": [{ "router": 12, "name": "lon-cr1.as64512", "as": 0, "asn": 64512,
             "link": 40, "cumMs": 0.4, "queueMs": 0.0 }],
  "asPath": [64512, 64530] }
```

## GET /api/router/{id}

```jsonc
{ "id": 12, "name": "...", "as": 0, "role": "core", "down": false, "hosts": 0,
  "neighbors": [{ "router": 13, "link": 40, "up": true, "util": 0.31 }],
  "fib": [{ "target": "lon-cr2.as64512" | "exit AS64530", "link": 40, "nextHop": 13, "cost": 4 }] }
```
