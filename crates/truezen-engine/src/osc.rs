//! Oscillators built on f64 phase accumulators.
//!
//! Phase is stored in **turns** (`[0, 1)`) rather than radians: wrapping is a
//! single compare-and-subtract, and the `TAU` multiply happens once at sine
//! evaluation instead of once per accumulate.
//!
//! The accumulator is f64 and is wrapped every sample. The tempting
//! alternative -- `sin(TAU * f * t)` with a running `t` -- drifts audibly: an
//! 8 hour session is ~1.4e9 samples, well past f32's 24-bit mantissa, and the
//! beat frequency wanders as `t` loses low bits.

use std::f64::consts::TAU;

use serde::{Deserialize, Serialize};

/// Carrier waveshape. Sine is correct for binaural work -- a pure tone puts
/// all its energy at the carrier, so the interaural difference is unambiguous.
/// The richer shapes exist for isochronic and drone layers, where harmonics
/// are part of the character.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Waveform {
    #[default]
    Sine,
    Triangle,
    Square,
    Saw,
}

/// f64 phase accumulator. One per oscillator, per channel.
#[derive(Clone, Debug, Default)]
pub struct Phasor {
    phase: f64,
    inc: f64,
}

impl Phasor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start at an explicit phase offset in turns. Used to give stacked layers
    /// distinct starting points so they do not all peak together on sample 0.
    pub fn with_phase(phase: f64) -> Self {
        Self {
            phase: phase.rem_euclid(1.0),
            inc: 0.0,
        }
    }

    /// Frequency is clamped below Nyquist; the wrap below assumes `inc < 1.0`.
    #[inline]
    pub fn set_freq(&mut self, hz: f64, sample_rate: f64) {
        let nyquist = sample_rate * 0.5;
        self.inc = hz.clamp(0.0, nyquist) / sample_rate;
    }

    #[inline]
    pub fn phase(&self) -> f64 {
        self.phase
    }

    #[inline]
    pub fn inc(&self) -> f64 {
        self.inc
    }

    pub fn reset(&mut self) {
        self.phase = 0.0;
    }

    /// Advance one sample, returning the phase *before* advancing.
    #[inline]
    pub fn tick(&mut self) -> f64 {
        let p = self.phase;
        self.phase += self.inc;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        p
    }

    /// Advance and evaluate the given waveshape.
    #[inline]
    pub fn tick_wave(&mut self, wave: Waveform) -> f64 {
        let p = self.tick();
        match wave {
            Waveform::Sine => (p * TAU).sin(),
            Waveform::Triangle => triangle(p),
            Waveform::Square => square_blep(p, self.inc),
            Waveform::Saw => saw_blep(p, self.inc),
        }
    }
}

/// Naive triangle. Its harmonics fall off as 1/n^2, so at the carrier
/// frequencies this app uses the aliasing is far below the noise floor and a
/// bandlimited version is not worth the cost.
#[inline]
fn triangle(p: f64) -> f64 {
    let x = if p < 0.5 { p * 2.0 } else { 2.0 - p * 2.0 };
    x * 2.0 - 1.0
}

/// PolyBLEP correction, applied at each discontinuity to suppress the
/// aliasing that a naive square or saw would fold back down into the audible
/// band.
#[inline]
fn poly_blep(t: f64, dt: f64) -> f64 {
    if dt <= 0.0 {
        return 0.0;
    }
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

#[inline]
fn saw_blep(p: f64, dt: f64) -> f64 {
    let naive = 2.0 * p - 1.0;
    naive - poly_blep(p, dt)
}

#[inline]
fn square_blep(p: f64, dt: f64) -> f64 {
    let naive = if p < 0.5 { 1.0 } else { -1.0 };
    // Two discontinuities per cycle: the rising edge at 0 and the falling
    // edge at 0.5.
    naive + poly_blep(p, dt) - poly_blep((p + 0.5) % 1.0, dt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_wraps_into_unit_range() {
        let mut ph = Phasor::new();
        ph.set_freq(997.0, 48_000.0);
        for _ in 0..200_000 {
            let p = ph.tick();
            assert!((0.0..1.0).contains(&p), "phase escaped unit range: {p}");
        }
    }

    #[test]
    fn frequency_is_clamped_below_nyquist() {
        let mut ph = Phasor::new();
        ph.set_freq(40_000.0, 48_000.0);
        assert!(ph.inc() <= 0.5);
    }

    /// The property the whole app rests on: after a long run the accumulated
    /// phase must still match the closed-form prediction.
    #[test]
    fn phase_does_not_drift_over_an_hour() {
        let sr = 48_000.0;
        let freq = 200.0;
        let n = 48_000u64 * 3_600;

        let mut ph = Phasor::new();
        ph.set_freq(freq, sr);
        for _ in 0..n {
            ph.tick();
        }

        let expected = ((freq / sr) * n as f64).rem_euclid(1.0);
        let diff = (ph.phase() - expected).abs();
        let wrapped = diff.min(1.0 - diff);
        assert!(wrapped < 1e-6, "phase drifted by {wrapped} turns over 1h");
    }

    #[test]
    fn blep_keeps_square_bounded() {
        let mut ph = Phasor::new();
        ph.set_freq(1000.0, 48_000.0);
        for _ in 0..48_000 {
            let v = ph.tick_wave(Waveform::Square);
            assert!(v.abs() <= 2.0, "square blew up: {v}");
        }
    }
}
