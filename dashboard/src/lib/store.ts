import { useSyncExternalStore } from "react";

type Listener = () => void;

export interface Store<S> {
  get(): S;
  set(patch: Partial<S> | ((s: S) => Partial<S>)): void;
  subscribe(listener: Listener): () => void;
}

/**
 * Minimal external store. State is replaced shallowly on every `set`, so
 * selectors that return an existing slice keep referential identity and
 * React skips the re-render.
 */
export function createStore<S extends object>(initial: S): Store<S> {
  let state = initial;
  const listeners = new Set<Listener>();
  return {
    get: () => state,
    set(patch) {
      const next = typeof patch === "function" ? patch(state) : patch;
      state = { ...state, ...next };
      for (const l of listeners) l();
    },
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
  };
}

export function useStoreSlice<S extends object, T>(store: Store<S>, selector: (s: S) => T): T {
  return useSyncExternalStore(
    store.subscribe,
    () => selector(store.get()),
    () => selector(store.get()),
  );
}
