# ShadowNet dashboard

Live operations console for shadownetd: a world map of every router and link
with animated traffic, KPIs, convergence incidents, failure-prediction stats,
and controls for injecting faults. It speaks the protocol in
[`docs/PROTOCOL.md`](../docs/PROTOCOL.md) and nothing else.

It is a Next.js app built as a static export. shadownetd serves `out/` at
`http://127.0.0.1:7070`, so in production the dashboard talks to its own
origin and there is no Node server involved.

## Running

```sh
npm install

# against a real daemon on :7070
NEXT_PUBLIC_SHADOWNET_URL=http://127.0.0.1:7070 npm run dev   # http://localhost:3000

# without the daemon: start the mock in another terminal first
npm run mock                                                  # ws/http on :7070
NEXT_PUBLIC_SHADOWNET_URL=http://127.0.0.1:7070 npm run dev

# production build, written to out/
npm run build
```

`npm run mock` also serves `out/` if it exists, so after `npm run build` you
can open http://127.0.0.1:7070 and see the export exactly as shadownetd will
serve it. The mock takes `--port` and `--seed`.

`npm run lint` and `npm run typecheck` run ESLint and `tsc`.

Keyboard and mouse on the map: drag to pan, wheel to zoom, double-click to
reset. Click a router or link to inspect it. Shift-click two routers, or press
`T` and pick two, to trace the live forwarding path. `Esc` clears.

## Layout

```
src/
  app/                  layout, page, global styles and design tokens
  lib/
    protocol.ts         types mirroring docs/PROTOCOL.md
    client.ts           HTTP helpers and the reconnecting WebSocket
    session.ts          socket lifecycle, topology reload on runId change, history polling
    state.ts            app store, plus the mutable frame buffer the map reads
    store.ts            ~40-line external store on useSyncExternalStore
    topology.ts         per-run lookups (adjacency, router pair -> link)
    actions.ts          traceroute flow
    format.ts           number formatting
  components/
    map/
      projection.ts     equirectangular camera: fit, zoom-at-cursor, easing
      geometry.ts       link curves, flattened polylines, spatial hash for hit tests
      renderer.ts       canvas drawing: graticule, links, routers, overlays, labels
      packets.ts        packet animation along flow paths
      controller.ts     render loop and pointer handling, outside React
      MapView.tsx       React shell: HUD, tooltip, inspector, trace panel
    panels/             KPI strip, scenarios, chaos, history, incidents, AI, event log
    ui/                 panel frame, sparkline, shared control styles
scripts/
  mock-server.mjs       dependency-free stand-in for shadownetd
```

### Rendering

Frames arrive about five times a second; the map draws at display rate. The
latest two frames sit in a plain mutable buffer (`live` in `state.ts`) that
the canvas loop reads directly, so React never re-renders for animation. Link
colours are interpolated between those two frames, and packets move along
their flow's path entirely on the client, with trip time proportional to the
flow's latency. Links are batched into one `Path2D` per colour and width
bucket, which keeps a full redraw of ~3,500 links, ~1,700 routers and a few
hundred flows comfortably inside a 60 fps budget.

The side panels subscribe to slices of the store and update at frame rate.
