import { api } from "./client";
import { store } from "./state";

let traceRequest: AbortController | null = null;

export function setTraceMode(on: boolean): void {
  store.set((s) => ({ traceMode: on, trace: on ? s.trace : s.trace?.result ? s.trace : null }));
}

export function clearTrace(): void {
  traceRequest?.abort();
  store.set({ trace: null, traceMode: false });
}

/**
 * First pick sets the source, second runs the trace. Picking again after a
 * finished trace starts over from the new router.
 */
export function pickTraceRouter(router: number): void {
  const cur = store.get().trace;
  if (!cur || cur.dst !== null || cur.src === router) {
    traceRequest?.abort();
    store.set({ trace: { src: router, dst: null, result: null, loading: false, error: null } });
    return;
  }
  void runTrace(cur.src, router);
}

export async function runTrace(src: number, dst: number): Promise<void> {
  traceRequest?.abort();
  const ctrl = new AbortController();
  traceRequest = ctrl;
  store.set({ trace: { src, dst, result: null, loading: true, error: null }, traceMode: false });
  try {
    const result = await api.trace(src, dst, ctrl.signal);
    if (ctrl.signal.aborted) return;
    store.set({ trace: { src, dst, result, loading: false, error: null } });
  } catch (err) {
    if (ctrl.signal.aborted) return;
    store.set({
      trace: {
        src,
        dst,
        result: null,
        loading: false,
        error: err instanceof Error ? err.message : String(err),
      },
    });
  }
}
