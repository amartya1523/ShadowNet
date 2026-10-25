//! Simulated time is an integer count of nanoseconds. Floating point clocks
//! drift and make event ordering depend on the order you added things up in.

pub type Nanos = u64;

pub const US: Nanos = 1_000;
pub const MS: Nanos = 1_000_000;
pub const SEC: Nanos = 1_000_000_000;

#[inline]
pub fn from_secs(s: f64) -> Nanos {
    debug_assert!(s >= 0.0, "negative duration {s}");
    (s * SEC as f64).round() as Nanos
}

#[inline]
pub fn to_secs(t: Nanos) -> f64 {
    t as f64 / SEC as f64
}

#[inline]
pub fn to_ms(t: Nanos) -> f64 {
    t as f64 / MS as f64
}

/// Protocol timers. The IGP ones are realistic for a modern network with
/// BFD; the BGP ones are scaled down (real MRAI is 30 s for eBGP) so a whole
/// experiment fits in a couple of minutes. Scaling is uniform-ish, so the
/// *shape* of BGP convergence survives.
#[derive(Clone, Debug)]
pub struct Timers {
    /// Time for BFD (or loss of light) to notice a dead link.
    pub detect_down: Nanos,
    /// Links come back slower than they go away: hold-down before trusting a
    /// recovered circuit.
    pub detect_up: Nanos,
    pub lsa_processing: Nanos,
    pub spf_initial: Nanos,
    pub spf_hold: Nanos,
    pub fib_install: Nanos,
    pub dv_triggered: Nanos,
    pub dv_periodic: Nanos,
    pub bgp_processing: Nanos,
    pub bgp_mrai: Nanos,
    /// Data-plane integration step.
    pub tick: Nanos,
}

impl Default for Timers {
    fn default() -> Self {
        Timers {
            detect_down: 150 * MS,
            detect_up: 800 * MS,
            lsa_processing: MS,
            spf_initial: 50 * MS,
            spf_hold: 200 * MS,
            fib_install: 20 * MS,
            dv_triggered: 40 * MS,
            dv_periodic: 2 * SEC,
            bgp_processing: 5 * MS,
            bgp_mrai: SEC,
            tick: 50 * MS,
        }
    }
}
