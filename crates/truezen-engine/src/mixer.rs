//! Master output stage: limiting and dither.

use crate::noise::Rng;

/// Default ceiling, -1 dBFS. Leaving a dB of headroom keeps inter-sample
/// peaks from clipping downstream resamplers and lossy encoders.
pub const DEFAULT_CEILING_DB: f64 = -1.0;

#[inline]
pub fn db_to_linear(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

#[inline]
pub fn linear_to_db(linear: f64) -> f64 {
    if linear <= 1e-12 {
        -240.0
    } else {
        20.0 * linear.log10()
    }
}

/// Waveshaping limiter.
///
/// `tanh` is effectively transparent at normal levels -- at -20 dBFS the error
/// is under 0.4% -- and compresses smoothly into the ceiling instead of
/// clipping. No lookahead, so it stays cheap and adds no latency; for stacked
/// sine layers, which have a low crest factor and no transients, that trade is
/// the right one.
#[derive(Clone, Debug)]
pub struct Limiter {
    ceiling: f64,
}

impl Default for Limiter {
    fn default() -> Self {
        Self::new(DEFAULT_CEILING_DB)
    }
}

impl Limiter {
    pub fn new(ceiling_db: f64) -> Self {
        Self {
            ceiling: db_to_linear(ceiling_db).clamp(1e-4, 1.0),
        }
    }

    pub fn set_ceiling_db(&mut self, db: f64) {
        self.ceiling = db_to_linear(db).clamp(1e-4, 1.0);
    }

    pub fn ceiling(&self) -> f64 {
        self.ceiling
    }

    #[inline]
    pub fn process(&self, x: f64) -> f64 {
        (x / self.ceiling).tanh() * self.ceiling
    }
}

/// Triangular-PDF dither for fixed-point export.
///
/// Only used when rendering to 16-bit. Truncating a long sustained sine to
/// 16 bits without dither produces correlated quantisation error, which on
/// this material is audible as a faint tone rather than as noise.
#[derive(Clone, Debug)]
pub struct Dither {
    rng: Rng,
    lsb: f64,
}

impl Dither {
    pub fn new(bit_depth: u32, seed: u64) -> Self {
        Self {
            rng: Rng::new(seed),
            lsb: 1.0 / (1u64 << (bit_depth - 1)) as f64,
        }
    }

    #[inline]
    pub fn process(&mut self, x: f64) -> f64 {
        // Two independent uniforms sum to a triangular distribution, which
        // fully decorrelates the error from the signal.
        let a = self.rng.next_bipolar();
        let b = self.rng.next_bipolar();
        x + (a + b) * 0.5 * self.lsb
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_conversion_round_trips() {
        for db in [-60.0, -20.0, -6.0, -1.0, 0.0] {
            assert!((linear_to_db(db_to_linear(db)) - db).abs() < 1e-9);
        }
    }

    #[test]
    fn limiter_never_exceeds_ceiling() {
        let lim = Limiter::new(-1.0);
        for x in [-100.0, -2.0, -1.0, 0.0, 1.0, 2.0, 100.0f64] {
            assert!(
                lim.process(x).abs() <= lim.ceiling() + 1e-12,
                "escaped at {x}"
            );
        }
    }

    #[test]
    fn limiter_is_near_transparent_at_normal_levels() {
        let lim = Limiter::new(-1.0);
        let x = db_to_linear(-20.0);
        let err = (lim.process(x) - x).abs() / x;
        assert!(err < 0.01, "limiter coloured a quiet signal by {err}");
    }

    #[test]
    fn dither_is_small_and_zero_mean() {
        let mut d = Dither::new(16, 42);
        let n = 200_000;
        let mean: f64 = (0..n).map(|_| d.process(0.0)).sum::<f64>() / n as f64;
        let lsb = 1.0 / 32_768.0;
        assert!(mean.abs() < lsb * 0.01, "dither has DC bias {mean}");
        for _ in 0..1000 {
            assert!(d.process(0.0).abs() <= lsb);
        }
    }
}
