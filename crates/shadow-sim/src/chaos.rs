//! The chaos monkey: a hazard process that breaks links on its own.
//!
//! Each link fails as a Poisson process whose rate depends on things you
//! would expect to matter in a real network: how hard the link has been
//! driven lately, whether it has been flapping, and a hidden per-link
//! fragility (old optics, a backhoe-prone conduit). The rate is synthetic;
//! what matters is that it is driven by observable stress plus noise, so the
//! failure predictor has signal to find but cannot be perfect.

use shadow_graph::{LinkId, Rng, Topology};

use crate::links::LinkTable;

pub struct Monkey {
    pub enabled: bool,
    /// Expected failures per minute across the whole network for a link at
    /// rest with average fragility.
    pub per_minute: f64,
    util_ewma: Vec<f64>,
    flap_heat: Vec<f64>,
    mean_fragility: f64,
    rng: Rng,
}

/// Time constant of the stress the hazard reacts to. Long enough that a
/// one-tick burst does not matter, short enough that a sustained overload
/// does within the predictor's horizon.
const STRESS_TAU_S: f64 = 20.0;
const FLAP_TAU_S: f64 = 90.0;

impl Monkey {
    pub fn new(topo: &Topology, seed: u64) -> Self {
        let n = topo.links.len();
        Monkey {
            enabled: false,
            per_minute: 12.0,
            util_ewma: vec![0.0; n],
            flap_heat: vec![0.0; n],
            mean_fragility: topo.links.iter().map(|l| l.fragility).sum::<f64>() / n.max(1) as f64,
            rng: Rng::new(seed),
        }
    }

    pub fn note_failure(&mut self, l: LinkId) {
        self.flap_heat[l.idx()] += 1.0;
    }

    /// Hazard multiplier from stress alone, before fragility. Exposed so the
    /// tests (and the README) can state what the ground truth is.
    pub fn stress_factor(util: f64, flap_heat: f64) -> f64 {
        let over = (util - 0.7).max(0.0);
        1.0 + 60.0 * over * over + 2.0 * flap_heat
    }

    /// Advance the stress trackers and return the links that fail this step.
    pub fn step(&mut self, topo: &Topology, links: &LinkTable, dt: f64) -> Vec<LinkId> {
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
        let base = self.per_minute / 60.0 / topo.links.len() as f64 / self.mean_fragility;
        let mut failed = Vec::new();
        for (i, s) in links.state.iter().enumerate() {
            if !s.up {
                continue;
            }
            let rate = base
                * topo.links[i].fragility
                * Self::stress_factor(self.util_ewma[i], self.flap_heat[i]);
            if self.rng.chance(1.0 - (-rate * dt).exp()) {
                failed.push(LinkId::from_idx(i));
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
        // A link pinned at 100% is several times more dangerous than one at
        // rest; that is the effect the predictor is supposed to discover.
        assert!(Monkey::stress_factor(1.0, 0.0) > 5.0);
    }
}
