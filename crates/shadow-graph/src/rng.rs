//! A small, fixed PRNG.
//!
//! Every experiment in ShadowNet is reproducible from a seed, and that promise
//! has to survive dependency upgrades. `rand` has changed its default
//! generator and its distribution algorithms between major versions before,
//! so we carry our own: xoshiro256** seeded through SplitMix64, which is what
//! the xoshiro authors recommend.

#[derive(Clone, Debug)]
pub struct Rng {
    s: [u64; 4],
}

#[inline]
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        let mut sm = seed;
        let s = [splitmix64(&mut sm), splitmix64(&mut sm), splitmix64(&mut sm), splitmix64(&mut sm)];
        Rng { s }
    }

    /// An independent stream derived from this one. Used to give each
    /// subsystem (topology, traffic, chaos) its own sequence, so adding a
    /// random draw in one place does not reshuffle everything else.
    pub fn fork(&mut self, salt: u64) -> Rng {
        Rng::new(self.next_u64() ^ salt.wrapping_mul(0xA24B_AED4_963E_E407))
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        let result = self.s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = self.s[1] << 17;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(45);
        result
    }

    /// Uniform in [0, 1).
    #[inline]
    pub fn f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Uniform in [0, n). Lemire's multiply-shift; the bias for the n we use
    /// (at most a few hundred thousand) is far below anything measurable.
    #[inline]
    pub fn below(&mut self, n: usize) -> usize {
        debug_assert!(n > 0);
        ((self.next_u64() as u128 * n as u128) >> 64) as usize
    }

    #[inline]
    pub fn range_f64(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.f64()
    }

    #[inline]
    pub fn chance(&mut self, p: f64) -> bool {
        self.f64() < p
    }

    /// Standard normal via Box-Muller. We throw away the second variate;
    /// nothing here is hot enough to care.
    pub fn normal(&mut self) -> f64 {
        let u1 = 1.0 - self.f64(); // (0, 1], keeps ln() finite
        let u2 = self.f64();
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }

    /// Log-normal parameterised by the *median* rather than mu, because that
    /// is the number you actually have in your head ("flows last about 20s").
    pub fn lognormal(&mut self, median: f64, sigma: f64) -> f64 {
        median * (sigma * self.normal()).exp()
    }

    /// Exponential with the given mean. Inter-arrival times of a Poisson
    /// process.
    pub fn exp(&mut self, mean: f64) -> f64 {
        -mean * (1.0 - self.f64()).ln()
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }

    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.below(i + 1);
            items.swap(i, j);
        }
    }

    /// Draw `k` distinct indices from `0..n` (k <= n). Floyd's algorithm, so
    /// it does not allocate `n` slots when `k` is small.
    pub fn sample_indices(&mut self, n: usize, k: usize) -> Vec<usize> {
        assert!(k <= n, "cannot sample {k} from {n}");
        let mut chosen = std::collections::HashSet::with_capacity(k);
        let mut out = Vec::with_capacity(k);
        for j in (n - k)..n {
            let t = self.below(j + 1);
            let v = if chosen.contains(&t) { j } else { t };
            chosen.insert(v);
            out.push(v);
        }
        out
    }
}

/// Discrete distribution sampled by binary search over the cumulative
/// weights. Built once, sampled many times (service popularity, host
/// placement).
#[derive(Clone, Debug)]
pub struct WeightedIndex {
    cumulative: Vec<f64>,
}

impl WeightedIndex {
    pub fn new(weights: impl IntoIterator<Item = f64>) -> Self {
        let mut acc = 0.0;
        let cumulative: Vec<f64> = weights
            .into_iter()
            .map(|w| {
                assert!(w >= 0.0 && w.is_finite(), "bad weight {w}");
                acc += w;
                acc
            })
            .collect();
        assert!(acc > 0.0, "all weights are zero");
        WeightedIndex { cumulative }
    }

    /// Zipf over `n` ranks with exponent `s`. Rank 0 is the most popular.
    pub fn zipf(n: usize, s: f64) -> Self {
        Self::new((1..=n).map(|k| 1.0 / (k as f64).powf(s)))
    }

    pub fn sample(&self, rng: &mut Rng) -> usize {
        let total = *self.cumulative.last().unwrap();
        let x = rng.f64() * total;
        self.cumulative.partition_point(|&c| c <= x).min(self.cumulative.len() - 1)
    }

    pub fn len(&self) -> usize {
        self.cumulative.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cumulative.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_stream() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..1000 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn known_first_values() {
        // Pin the stream. If this test changes, every published benchmark in
        // the README changes with it.
        let mut r = Rng::new(0);
        assert_eq!(r.next_u64(), 0x99EC_5F36_CB75_F2B4);
        assert_eq!(r.next_u64(), 0xBF6E_1F78_4956_452A);
    }

    #[test]
    fn unit_interval_moments() {
        let mut r = Rng::new(7);
        let n = 200_000;
        let mean = (0..n).map(|_| r.f64()).sum::<f64>() / n as f64;
        assert!((mean - 0.5).abs() < 0.005, "mean {mean}");
    }

    #[test]
    fn normal_moments() {
        let mut r = Rng::new(9);
        let n = 200_000;
        let xs: Vec<f64> = (0..n).map(|_| r.normal()).collect();
        let mean = xs.iter().sum::<f64>() / n as f64;
        let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n as f64;
        assert!(mean.abs() < 0.01, "mean {mean}");
        assert!((var - 1.0).abs() < 0.02, "var {var}");
    }

    #[test]
    fn sample_indices_distinct() {
        let mut r = Rng::new(3);
        let mut v = r.sample_indices(100, 40);
        v.sort_unstable();
        v.dedup();
        assert_eq!(v.len(), 40);
        assert!(v.iter().all(|&x| x < 100));
    }

    #[test]
    fn weighted_index_respects_weights() {
        let mut r = Rng::new(11);
        let w = WeightedIndex::new([1.0, 0.0, 3.0]);
        let mut counts = [0usize; 3];
        for _ in 0..40_000 {
            counts[w.sample(&mut r)] += 1;
        }
        assert_eq!(counts[1], 0);
        let ratio = counts[2] as f64 / counts[0] as f64;
        assert!((ratio - 3.0).abs() < 0.2, "ratio {ratio}");
    }
}
