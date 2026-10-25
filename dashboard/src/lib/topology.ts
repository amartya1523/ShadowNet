import type { As, Tier, Topology } from "./protocol";

/** Lookups derived once per topology; everything here is immutable afterwards. */
export interface TopoIndex {
  topology: Topology;
  routerTier: Tier[];
  routersByAs: number[][];
  /** Link ids adjacent to each router. */
  adjacency: number[][];
  cityName: Map<string, string>;
  linkBetween(a: number, b: number): number;
  asOfRouter(router: number): As;
}

export function indexTopology(topology: Topology): TopoIndex {
  const n = topology.routers.length;
  const routerTier: Tier[] = new Array(n);
  const routersByAs: number[][] = topology.ases.map(() => []);
  const adjacency: number[][] = Array.from({ length: n }, () => []);
  const pairs = new Map<number, number>();

  for (const r of topology.routers) {
    routerTier[r.id] = topology.ases[r.as]?.tier ?? "stub";
    routersByAs[r.as]?.push(r.id);
  }
  for (const l of topology.links) {
    adjacency[l.a].push(l.id);
    adjacency[l.b].push(l.id);
    // Parallel links between the same pair keep the first; the map only
    // needs one to draw a packet path.
    if (!pairs.has(l.a * n + l.b)) {
      pairs.set(l.a * n + l.b, l.id);
      pairs.set(l.b * n + l.a, l.id);
    }
  }

  return {
    topology,
    routerTier,
    routersByAs,
    adjacency,
    cityName: new Map(topology.cities.map((c) => [c.code, c.name])),
    linkBetween: (a, b) => pairs.get(a * n + b) ?? -1,
    asOfRouter: (router) => topology.ases[topology.routers[router].as],
  };
}
