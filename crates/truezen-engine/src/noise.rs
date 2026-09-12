//! Noise generation and the filter used to shape it into ambience.

use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NoiseColor {
    White,
    #[default]
    Pink,
    Brown,
}

/// xorshift64*. Deterministic, allocation-free, and far cheaper than a
/// cryptographic generator -- which we have no use for here.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        // A zero state is a fixed point for xorshift, so never allow it.
        Self(if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed })
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in [-1, 1).
    #[inline]
    pub fn next_bipolar(&mut self) -> f64 {
        // Take the top 53 bits so the result uses f64's full mantissa.
        let u = (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
        u * 2.0 - 1.0
    }
}

/// White, pink and brown noise from one shared white source.
#[derive(Clone, Debug)]
pub struct NoiseGen {
    rng: Rng,
    /// Paul Kellet economy pink filter state.
    pink: [f64; 7],
    brown: f64,
}

impl NoiseGen {
    pub fn new(seed: u64) -> Self {
        Self {
            rng: Rng::new(seed),
            pink: [0.0; 7],
            brown: 0.0,
        }
    }

    #[inline]
    pub fn tick(&mut self, color: NoiseColor) -> f64 {
        let white = self.rng.next_bipolar();
        match color {
            NoiseColor::White => white,
            NoiseColor::Pink => self.tick_pink(white),
            NoiseColor::Brown => self.tick_brown(white),
        }
    }

    /// Paul Kellet's economy pink filter: a parallel bank of one-poles that
    /// approximates -3 dB/octave to within about +/-0.05 dB from 10 Hz up.
    #[inline]
    fn tick_pink(&mut self, white: f64) -> f64 {
        let b = &mut self.pink;
        b[0] = 0.99886 * b[0] + white * 0.0555179;
        b[1] = 0.99332 * b[1] + white * 0.0750759;
        b[2] = 0.96900 * b[2] + white * 0.1538520;
        b[3] = 0.86650 * b[3] + white * 0.3104856;
        b[4] = 0.55000 * b[4] + white * 0.5329522;
        b[5] = -0.7616 * b[5] - white * 0.0168980;
        let out = b[0] + b[1] + b[2] + b[3] + b[4] + b[5] + b[6] + white * 0.5362;
        b[6] = white * 0.115926;
        // The bank sums to roughly 4x unity; scale back to about +/-1.
        out * 0.11
    }

    /// Leaky integrator. The leak keeps the random walk from wandering off as
    /// DC over a long session.
    #[inline]
    fn tick_brown(&mut self, white: f64) -> f64 {
        self.brown = (self.brown + 0.02 * white) / 1.02;
        self.brown * 3.5
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FilterMode {
    #[default]
    LowPass,
    BandPass,
    HighPass,
}

/// Topology-preserving-transform state variable filter (Zavalishin / Simper).
///
/// Chosen over a Chamberlin SVF because it stays stable as cutoff approaches
/// Nyquist, which matters when a preset automates the cutoff over a wide
/// range.
#[derive(Clone, Debug, Default)]
pub struct Svf {
    ic1: f64,
    ic2: f64,
    a1: f64,
    a2: f64,
    a3: f64,
    k: f64,
}

impl Svf {
    pub fn new() -> Self {
        let mut f = Self::default();
        f.set(1_000.0, 0.707, 48_000.0);
        f
    }

    pub fn set(&mut self, cutoff_hz: f64, q: f64, sample_rate: f64) {
        // Keep cutoff strictly inside the band; tan() blows up at Nyquist.
        let fc = cutoff_hz.clamp(10.0, sample_rate * 0.49);
        let q = q.max(0.05);
        let g = (std::f64::consts::PI * fc / sample_rate).tan();
        let k = 1.0 / q;
        let a1 = 1.0 / (1.0 + g * (g + k));
        self.a1 = a1;
        self.a2 = g * a1;
        self.a3 = g * self.a2;
        self.k = k;
    }

    pub fn reset(&mut self) {
        self.ic1 = 0.0;
        self.ic2 = 0.0;
    }

    #[inline]
    pub fn tick(&mut self, x: f64, mode: FilterMode) -> f64 {
        let v3 = x - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        match mode {
            FilterMode::LowPass => v2,
            FilterMode::BandPass => v1,
            FilterMode::HighPass => x - self.k * v1 - v2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(samples: &[f64]) -> f64 {
        (samples.iter().map(|s| s * s).sum::<f64>() / samples.len() as f64).sqrt()
    }

    #[test]
    fn all_colors_are_bounded_and_audible() {
        for color in [NoiseColor::White, NoiseColor::Pink, NoiseColor::Brown] {
            let mut n = NoiseGen::new(12345);
            let buf: Vec<f64> = (0..200_000).map(|_| n.tick(color)).collect();
            let peak = buf.iter().fold(0.0f64, |a, b| a.max(b.abs()));
            let level = rms(&buf);
            assert!(peak < 1.5, "{color:?} peak {peak} too hot");
            assert!(level > 0.05, "{color:?} rms {level} too quiet");
        }
    }

    /// Brown noise is a random walk; without the leak it drifts into DC and
    /// eventually clips. Confirm the leak holds it near zero.
    #[test]
    fn brown_noise_does_not_wander_into_dc() {
        let mut n = NoiseGen::new(999);
        let buf: Vec<f64> = (0..2_000_000).map(|_| n.tick(NoiseColor::Brown)).collect();
        let mean = buf.iter().sum::<f64>() / buf.len() as f64;
        assert!(mean.abs() < 0.05, "brown noise drifted to DC {mean}");
    }

    #[test]
    fn lowpass_attenuates_white_noise() {
        let mut n = NoiseGen::new(7);
        let mut f = Svf::new();
        f.set(200.0, 0.707, 48_000.0);
        let dry: Vec<f64> = (0..100_000).map(|_| n.tick(NoiseColor::White)).collect();
        let wet: Vec<f64> = dry.iter().map(|&x| f.tick(x, FilterMode::LowPass)).collect();
        assert!(rms(&wet) < rms(&dry) * 0.5);
    }

    #[test]
    fn filter_stays_stable_at_extreme_cutoff() {
        let mut f = Svf::new();
        f.set(23_900.0, 8.0, 48_000.0);
        let mut n = NoiseGen::new(3);
        for _ in 0..100_000 {
            let v = f.tick(n.tick(NoiseColor::White), FilterMode::HighPass);
            assert!(v.is_finite(), "filter went unstable");
        }
    }
}
