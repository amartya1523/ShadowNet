//! The chaos monkey: links that break on their own.
//!
//! Real circuits rarely die out of a clear sky. Optics age, a connector
//! works loose, a fibre gets bent in a duct, and for a while before the
//! link drops the interface error counters climb. That is what operators'
//! predictive maintenance looks for, so it is what we model:
//!
//! * **Wear-out** (most failures): a link starts *degrading*. Its error
//!   rate grows over 20-80 s, accelerating. Most degrading links then fail
//!   hard; about a quarter recover on their own (a false alarm, from the
//!   predictor's point of view).
//! * **Sudden** failures (the rest): a backhoe. No warning at all.
//!
//! How often a link starts to degrade depends on a hidden per-link
//! fragility and on stress an operator can see: sustained high utilisation
//! and recent flapping. The process is synthetic and the README says so;
//! what matters is that it is driven by observables plus noise, so the
//! failure predictor has real signal to find and cannot be perfect.

use shadow_graph::{LinkId, Rng, Topology};

use crate::links::LinkTable;

/// Share of the failure budget that arrives with no warning.
const SUDDEN_SHARE: f64 = 0.3;
/// Chance a degradation episode ends in a failure rather than recovering.
const WEAR_FAILS: f64 = 0.75;

const STRESS_TAU_S: f64 = 20.0;
const FLAP_TAU_S: f64 = 90.0;

#[derive(Clone, Copy, Debug)]
struct Wear {
    elapsed: f64,
    duration: f64,
    peak_errors: f64,
    fails: bool,
}

pub struct Monkey {
    pub enabled: bool,
    /// Expected failures per minute across the whole network for links at
    /// rest with average fragility.
    pub per_minute: f64,
    util_ewma: Vec<f64>,
    flap_heat: Vec<f64>,
    wear: Vec<Option<Wear>>,
    mean_fragility: f64,
    rng: Rng,
}

impl Monkey {
    pub fn new(topo: &Topology, seed: u64) -> Self {
        let n = topo.links.len();
        Monkey {
            enabled: false,
            per_minute: 12.0,
            util_ewma: vec![0.0; n],
            flap_heat: vec![0.0; n],
            wear: vec![None; n],
            mean_fragility: topo.links.iter().map(|l| l.fragility).sum::<f64>() / n.max(1) as f64,
            rng: Rng::new(seed),
        }
    }

    pub fn note_failure(&mut self, l: LinkId) {
        self.flap_heat[l.idx()] += 1.0;
    }

    /// Multiplier on the wear-out onset rate from stress alone. Public so
    /// the tests (and the README) can state exactly what the ground truth is.
    pub fn stress_factor(util: f64, flap_heat: f64) -> f64 {
        let over = (util - 0.6).max(0.0);
        1.0 + 40.0 * over * over + 3.0 * flap_heat
    }

    /// Stop all wear in progress (the monkey was switched off, or everything
    /// was healed).
    pub fn calm(&mut self, links: &mut LinkTable) {
        for (w, s) in self.wear.iter_mut().zip(&mut links.state) {
            *w = None;
            s.wear_errors = 0.0;
        }
    }

    /// Advance stress trackers and wear, and return the links that fail
    /// during this step.
    pub fn step(&mut self, topo: &Topology, links: &mut LinkTable, dt: f64) -> Vec<LinkId> {
        let a_util = 1.0 - (-dt / STRESS_TAU_S).exp();
        let a_flap = (-dt / FLAP_TAU_S).exp();
        for (i, s) in links.state.iter().enumerate() {
            let u = s.dir[0].util.max(s.dir[1].util);
            self.util_ewma[i] += a_util * (u - self.util_ewma[i]);
            self.flap_heat[i] *= a_flap;
        }
        if !self.enabled || self.per_minute <= 0.0 {
            return Vec::new();
        }

        let per_link = self.per_minute / 60.0 / topo.links.len() as f64 / self.mean_fragility;
        let sudden_rate = per_link * SUDDEN_SHARE;
        let onset_rate = per_link * (1.0 - SUDDEN_SHARE) / WEAR_FAILS;

        let mut failed = Vec::new();
        for i in 0..links.state.len() {
            if !links.state[i].up {
                self.wear[i] = None;
                links.state[i].wear_errors = 0.0;
                continue;
            }
            let fragility = topo.links[i].fragility;

            if let Some(w) = &mut self.wear[i] {
                w.elapsed += dt;
                let x = (w.elapsed / w.duration).min(1.0);
                links.state[i].wear_errors = w.peak_errors * x * x;
                if w.elapsed >= w.duration {
                    if w.fails {
                        failed.push(LinkId::from_idx(i));
                    }
                    self.wear[i] = None;
                    links.state[i].wear_errors = 0.0;
                }
                continue;
            }

            if self.rng.chance(1.0 - (-sudden_rate * fragility * dt).exp()) {
                failed.push(LinkId::from_idx(i));
                continue;
            }
            let onset = onset_rate * fragility * Self::stress_factor(self.util_ewma[i], self.flap_heat[i]);
            if self.rng.chance(1.0 - (-onset * dt).exp()) {
                self.wear[i] = Some(Wear {
                    elapsed: 0.0,
                    duration: self.rng.range_f64(20.0, 80.0),
                    // Errors at the end of the episode: 0.2% to ~3%, the range
                    // where an operator would start paying attention.
                    peak_errors: self.rng.lognormal(0.008, 0.6).min(0.05),
                    fails: self.rng.chance(WEAR_FAILS),
                });
            }
        }
        failed
    }

    /// How long a monkey-induced outage lasts. Log-normal: most are short
    /// (a flap, a reboot), some are a truck through a fibre duct.
    pub fn repair_time_s(&mut self) -> f64 {
        self.rng.lognormal(12.0, 0.7).clamp(2.0, 180.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stress_is_monotone() {
        let mut last = 0.0;
        for i in 0..=20 {
            let f = Monkey::stress_factor(i as f64 / 20.0, 0.0);
            assert!(f >= last);
            last = f;
        }
        assert_eq!(Monkey::stress_factor(0.5, 0.0), 1.0);
        // A link pinned at 100% is several times more likely to start
        // wearing out than one at rest.
        assert!(Monkey::stress_factor(1.0, 0.0) > 5.0);
        assert!(Monkey::stress_factor(0.5, 1.0) > 3.0);
    }
}
