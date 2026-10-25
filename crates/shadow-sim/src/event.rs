use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::sync::Arc;

use shadow_graph::{AsId, LinkId, RouterId};

use crate::bgp::Update;
use crate::igp::linkstate::Lsa;
use crate::time::Nanos;

#[derive(Debug)]
pub enum Event {
    /// Advance the fluid data plane by one step.
    Tick,
    /// One end of a link has noticed it changed state.
    Detect {
        link: LinkId,
        end: u8,
        up: bool,
    },

    Lsa {
        to: RouterId,
        via: LinkId,
        lsa: Arc<Lsa>,
    },
    Spf {
        router: RouterId,
    },
    FibInstall {
        router: RouterId,
        next: Box<[u32]>,
        cost: Box<[u32]>,
    },

    DvSend {
        router: RouterId,
    },
    DvPeriodic {
        router: RouterId,
    },
    DvRecv {
        to: RouterId,
        via: LinkId,
        vector: Arc<[u8]>,
    },

    Bgp {
        to: AsId,
        from: AsId,
        epoch: u32,
        update: Arc<Update>,
    },
    Mrai {
        asys: AsId,
        neighbor: u16,
    },

    /// A latency spike or loss injection on `link` wears off.
    Clear {
        link: LinkId,
        token: u32,
    },
    /// Automatic repair after a chaos-monkey failure.
    Repair {
        link: LinkId,
        token: u32,
    },
    ScenarioStep {
        run: u32,
        step: usize,
    },
}

struct Entry {
    at: Nanos,
    seq: u64,
    ev: Event,
}

// BinaryHeap is a max-heap; invert so the earliest event (and, for ties, the
// one scheduled first) comes out on top. FIFO among equal timestamps matters:
// two LSAs sent at the same instant over the same link must arrive in order.
impl Ord for Entry {
    fn cmp(&self, other: &Self) -> Ordering {
        other.at.cmp(&self.at).then_with(|| other.seq.cmp(&self.seq))
    }
}

impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        self.at == other.at && self.seq == other.seq
    }
}

impl Eq for Entry {}

#[derive(Default)]
pub struct EventQueue {
    heap: BinaryHeap<Entry>,
    seq: u64,
    now: Nanos,
    pub processed: u64,
}

impl EventQueue {
    pub fn now(&self) -> Nanos {
        self.now
    }

    pub fn at(&mut self, at: Nanos, ev: Event) {
        debug_assert!(at >= self.now, "scheduling into the past: {at} < {}", self.now);
        self.seq += 1;
        self.heap.push(Entry { at, seq: self.seq, ev });
    }

    pub fn after(&mut self, delay: Nanos, ev: Event) {
        self.at(self.now + delay, ev);
    }

    /// Pops the next event if it is due at or before `limit`, advancing the
    /// clock to its timestamp.
    pub fn pop_until(&mut self, limit: Nanos) -> Option<Event> {
        if self.heap.peek()?.at > limit {
            return None;
        }
        let e = self.heap.pop()?;
        self.now = e.at;
        self.processed += 1;
        Some(e.ev)
    }

    /// Move the clock forward with nothing happening (end of a run_until).
    pub fn advance_to(&mut self, t: Nanos) {
        debug_assert!(t >= self.now);
        self.now = t;
    }

    pub fn len(&self) -> usize {
        self.heap.len()
    }

    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn earliest_first_then_fifo() {
        let mut q = EventQueue::default();
        q.at(30, Event::Tick);
        q.at(10, Event::Spf { router: RouterId(1) });
        q.at(10, Event::Spf { router: RouterId(2) });
        q.at(20, Event::Spf { router: RouterId(3) });
        let order: Vec<_> = std::iter::from_fn(|| q.pop_until(u64::MAX))
            .map(|e| match e {
                Event::Spf { router } => router.0,
                Event::Tick => 0,
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(order, vec![1, 2, 3, 0]);
        assert_eq!(q.now(), 30);
    }

    #[test]
    fn respects_limit() {
        let mut q = EventQueue::default();
        q.at(100, Event::Tick);
        assert!(q.pop_until(99).is_none());
        assert!(q.pop_until(100).is_some());
    }
}
