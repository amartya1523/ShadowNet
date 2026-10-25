//! Link failure prediction from operational telemetry.
//!
//! Online logistic regression. Every second each live link is described by
//! a handful of features any NOC has on a dashboard already; the feature
//! vector is parked until the prediction horizon has passed, then labelled
//! with what actually happened and used for one SGD step. So the model only
//! ever learns from the past, and every score it reports is out-of-sample at
//! the time it was made.
//!
//! It is deliberately simple. The interesting result is not the model, it is
//! that a model this simple beats "alert on high utilisation" by a margin,
//! because flap history and queue behaviour carry information utilisation
//! alone does not.

use std::collections::VecDeque;

use shadow_graph::algo::edge_betweenness;
use shadow_graph::{LinkId, Topology};

use crate::links::{BUFFER_S, LinkTable};

pub const FEATURES: usize = 11;
pub const FEATURE_NAMES: [&str; FEATURES] = [
    "util_fast",
    "util_slow",
    "util_peak",
    "queue_fill",
    "drops",
    "errors",
    "error_trend",
    "jitter",
    "flaps",
    "recent_failure",
    "centrality",
];

#[derive(Clone, Default)]
struct Track {
    util_fast: f64,
    util_slow: f64,
    peak: f64,
    errors_slow: f64,
    last_queue: f64,
    jitter: f64,
    flaps: VecDeque<f64>,
    last_failure: Option<f64>,
    /// Unplanned failures only. Operators pulling a cable on purpose is
    /// not something to predict.
    failures: Vec<f64>,
}

struct Pending {
    t: f64,
    rows: Vec<(LinkId, [f32; FEATURES])>,
}

/// Running mean and variance (Welford) for feature standardisation.
#[derive(Clone, Copy, Default)]
struct Moments {
    n: f64,
    mean: f64,
    m2: f64,
}

impl Moments {
    fn push(&mut self, x: f64) {
        self.n += 1.0;
        let d = x - self.mean;
        self.mean += d / self.n;
        self.m2 += d * (x - self.mean);
    }

    fn scale(&self, x: f64) -> f64 {
        let sd = if self.n > 1.0 { (self.m2 / (self.n - 1.0)).sqrt() } else { 1.0 };
        (x - self.mean) / sd.max(1e-6)
    }
}

pub struct Predictor {
    pub horizon_s: f64,
    tracks: Vec<Track>,
    centrality: Vec<f32>,
    weights: [f64; FEATURES + 1],
    moments: [Moments; FEATURES],
    pending: VecDeque<Pending>,
    top_pending: VecDeque<(f64, Vec<LinkId>)>,
    /// (model score, baseline score, label) for the evaluation window.
    window: VecDeque<(f32, f32, bool)>,
    hits_at_10: VecDeque<f64>,
    pub risk: Vec<f32>,
    pub labelled: u64,
    pub positives: u64,
    negatives: u64,
    pub auc: Option<f64>,
    pub baseline_auc: Option<f64>,
    last_eval: f64,
    lr: f64,
}

const WINDOW: usize = 60_000;

impl Predictor {
    pub fn new(topo: &Topology) -> Self {
        let n = topo.links.len();
        // Exact betweenness over ~1700 routers is a fraction of a second,
        // but pivots keep tests fast and the estimate is plenty for a
        // feature.
        let g = topo.router_graph();
        let pivots: Vec<usize> = (0..topo.routers.len()).step_by((topo.routers.len() / 300).max(1)).collect();
        let b = edge_betweenness(&g, n, Some(&pivots));
        let max = b.iter().copied().fold(1.0, f64::max);
        Predictor {
            horizon_s: 20.0,
            tracks: vec![Track::default(); n],
            centrality: b.iter().map(|&x| ((1.0 + x).ln() / (1.0 + max).ln()) as f32).collect(),
            weights: [0.0; FEATURES + 1],
            moments: [Moments::default(); FEATURES],
            pending: VecDeque::new(),
            top_pending: VecDeque::new(),
            window: VecDeque::new(),
            hits_at_10: VecDeque::new(),
            risk: vec![0.0; n],
            labelled: 0,
            positives: 0,
            negatives: 0,
            auc: None,
            baseline_auc: None,
            last_eval: 0.0,
            lr: 0.01,
        }
    }

    pub fn weights(&self) -> &[f64; FEATURES + 1] {
        &self.weights
    }

    pub fn record_failure(&mut self, l: LinkId, t: f64, planned: bool) {
        let tr = &mut self.tracks[l.idx()];
        tr.flaps.push_back(t);
        tr.last_failure = Some(t);
        if !planned {
            tr.failures.push(t);
        }
    }

    /// Fold this tick's link state into the running trackers.
    pub fn observe(&mut self, links: &LinkTable, dt: f64) {
        let a_fast = 1.0 - (-dt / 2.0).exp();
        let a_slow = 1.0 - (-dt / 30.0).exp();
        let decay = (-dt / 10.0).exp();
        for (tr, s) in self.tracks.iter_mut().zip(&links.state) {
            let u = s.dir[0].util.max(s.dir[1].util);
            let qd = s.dir[0].queue_s.max(s.dir[1].queue_s);
            tr.util_fast += a_fast * (u - tr.util_fast);
            tr.errors_slow += a_slow * (s.wear_errors - tr.errors_slow);
            tr.util_slow += a_slow * (u - tr.util_slow);
            tr.peak = u.max(tr.peak * decay);
            tr.jitter += a_fast * ((qd - tr.last_queue).abs() * 1e3 - tr.jitter);
            tr.last_queue = qd;
        }
    }

    fn features(&self, l: usize, links: &LinkTable, t: f64) -> [f32; FEATURES] {
        let tr = &self.tracks[l];
        let s = &links.state[l];
        let qd = s.dir[0].queue_s.max(s.dir[1].queue_s);
        // Congestion drops only; bit errors have their own counter.
        let drops = 1.0 - (1.0 - s.dir[0].loss.max(s.dir[1].loss)) / (1.0 - s.wear_errors).max(1e-9);
        // Errors as log-scale, the way anyone reads an error counter: the
        // difference between 1e-6 and 1e-4 matters as much as 1e-4 to 1e-2.
        let errors = (s.wear_errors * 1e6).ln_1p();
        let trend = errors - (tr.errors_slow * 1e6).ln_1p();
        let flaps = tr.flaps.iter().filter(|&&f| t - f < 120.0).count() as f64;
        let recent = tr.last_failure.map_or(0.0, |f| (-(t - f) / 30.0).exp());
        [
            tr.util_fast as f32,
            tr.util_slow as f32,
            tr.peak as f32,
            (qd / BUFFER_S) as f32,
            drops.max(0.0) as f32,
            errors as f32,
            trend as f32,
            tr.jitter as f32,
            flaps.ln_1p() as f32,
            recent as f32,
            self.centrality[l],
        ]
    }

    fn score(&self, x: &[f32; FEATURES]) -> f64 {
        let mut z = self.weights[FEATURES];
        for i in 0..FEATURES {
            z += self.weights[i] * self.moments[i].scale(x[i] as f64);
        }
        1.0 / (1.0 + (-z).exp())
    }

    /// Called once per simulated second.
    pub fn sample(&mut self, links: &LinkTable, t: f64) {
        for tr in &mut self.tracks {
            while tr.flaps.front().is_some_and(|&f| t - f > 300.0) {
                tr.flaps.pop_front();
            }
        }

        let mut rows = Vec::with_capacity(self.tracks.len());
        for l in 0..self.tracks.len() {
            if !links.state[l].up {
                self.risk[l] = 0.0;
                continue;
            }
            let x = self.features(l, links, t);
            self.risk[l] = self.score(&x) as f32;
            rows.push((LinkId::from_idx(l), x));
        }
        let mut ranked: Vec<(f32, LinkId)> = rows.iter().map(|(l, _)| (self.risk[l.idx()], *l)).collect();
        ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
        self.top_pending.push_back((t, ranked.iter().take(10).map(|r| r.1).collect()));
        self.pending.push_back(Pending { t, rows });

        while self.pending.front().is_some_and(|p| t - p.t >= self.horizon_s) {
            let p = self.pending.pop_front().unwrap();
            self.learn(p);
        }
        while self.top_pending.front().is_some_and(|p| t - p.0 >= self.horizon_s) {
            let (t0, top) = self.top_pending.pop_front().unwrap();
            let hits = top.iter().filter(|l| self.failed_within(l.idx(), t0)).count();
            self.hits_at_10.push_back(hits as f64 / 10.0);
            if self.hits_at_10.len() > 120 {
                self.hits_at_10.pop_front();
            }
        }
        if t - self.last_eval >= 5.0 {
            self.last_eval = t;
            self.evaluate();
        }
    }

    fn failed_within(&self, l: usize, t0: f64) -> bool {
        self.tracks[l].failures.iter().any(|&f| f > t0 && f <= t0 + self.horizon_s)
    }

    fn learn(&mut self, p: Pending) {
        for (l, x) in p.rows {
            let y = self.failed_within(l.idx(), p.t);
            for i in 0..FEATURES {
                self.moments[i].push(x[i] as f64);
            }
            let score = self.score(&x);
            self.window.push_back((score as f32, x[1], y));
            if self.window.len() > WINDOW {
                self.window.pop_front();
            }
            if y {
                self.positives += 1;
            } else {
                self.negatives += 1;
            }
            self.labelled += 1;

            // Failures are rare (well under 1% of samples), so positives are
            // up-weighted by the running class ratio. Capped low: with a big
            // weight a single positive moves the model further than a
            // thousand negatives can pull it back, and it oscillates.
            let w = if y { (self.negatives as f64 / self.positives.max(1) as f64).clamp(1.0, 20.0) } else { 1.0 };
            let g = (score - y as u8 as f64) * w * self.lr;
            for i in 0..FEATURES {
                let xi = self.moments[i].scale(x[i] as f64);
                self.weights[i] -= g * xi + 1e-4 * self.weights[i];
            }
            self.weights[FEATURES] -= g;
        }
    }

    fn evaluate(&mut self) {
        let pos = self.window.iter().filter(|w| w.2).count();
        if pos < 5 || pos == self.window.len() {
            return;
        }
        self.auc = Some(auc(self.window.iter().map(|w| (w.0, w.2))));
        self.baseline_auc = Some(auc(self.window.iter().map(|w| (w.1, w.2))));
    }

    pub fn precision_at_10(&self) -> Option<f64> {
        if self.hits_at_10.is_empty() {
            None
        } else {
            Some(self.hits_at_10.iter().sum::<f64>() / self.hits_at_10.len() as f64)
        }
    }

    pub fn top(&self, k: usize) -> Vec<(LinkId, f32)> {
        let mut v: Vec<(LinkId, f32)> = self.risk.iter().enumerate().map(|(i, &r)| (LinkId::from_idx(i), r)).collect();
        v.sort_by(|a, b| b.1.total_cmp(&a.1));
        v.truncate(k);
        v
    }
}

/// ROC AUC as the Mann-Whitney statistic, with ties given half credit.
fn auc(scored: impl Iterator<Item = (f32, bool)>) -> f64 {
    let mut v: Vec<(f32, bool)> = scored.collect();
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (mut rank_sum, mut n_pos, mut n_neg) = (0.0, 0.0, 0.0);
    let mut i = 0;
    while i < v.len() {
        let mut j = i;
        while j < v.len() && v[j].0 == v[i].0 {
            j += 1;
        }
        // Average 1-based rank of the tie group.
        let rank = (i + j + 1) as f64 / 2.0;
        for item in &v[i..j] {
            if item.1 {
                rank_sum += rank;
                n_pos += 1.0;
            } else {
                n_neg += 1.0;
            }
        }
        i = j;
    }
    (rank_sum - n_pos * (n_pos + 1.0) / 2.0) / (n_pos * n_neg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auc_extremes() {
        let perfect = [(0.1, false), (0.2, false), (0.8, true), (0.9, true)];
        assert!((auc(perfect.into_iter()) - 1.0).abs() < 1e-12);
        let inverted = [(0.9, false), (0.8, false), (0.2, true), (0.1, true)];
        assert!(auc(inverted.into_iter()).abs() < 1e-12);
        let ties = [(0.5, false), (0.5, true), (0.5, false), (0.5, true)];
        assert!((auc(ties.into_iter()) - 0.5).abs() < 1e-12);
    }

    #[test]
    fn moments_standardise() {
        let mut m = Moments::default();
        for x in [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0] {
            m.push(x);
        }
        assert!((m.mean - 5.0).abs() < 1e-12);
        assert!(m.scale(5.0).abs() < 1e-12);
    }
}
