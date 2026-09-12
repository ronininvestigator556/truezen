//! One-pole parameter smoothing.
//!
//! Every continuous parameter the user can move passes through a `Smoother`
//! before it reaches an oscillator. Without this, changing a frequency mid
//! session steps the phase increment discontinuously and you hear a zipper.

/// Exponential one-pole smoother.
#[derive(Clone, Debug)]
pub struct Smoother {
    current: f64,
    target: f64,
    coeff: f64,
    /// Below this distance we snap to target, which both settles the value
    /// exactly and keeps the filter state out of denormal range.
    ///
    /// This is absolute, so it must be set relative to the parameter's range:
    /// the default suits gains and unit-scale values, while a carrier in Hz
    /// wants something coarser. 1e-6 is inaudible either as a gain error
    /// (-120 dB) or as a frequency error.
    epsilon: f64,
}

impl Smoother {
    /// `tau_s` is the time constant: the value covers ~63% of the remaining
    /// distance per `tau_s` seconds.
    pub fn new(initial: f64, tau_s: f64, sample_rate: f64) -> Self {
        let mut s = Self {
            current: initial,
            target: initial,
            coeff: 1.0,
            epsilon: 1e-6,
        };
        s.set_tau(tau_s, sample_rate);
        s
    }

    pub fn set_tau(&mut self, tau_s: f64, sample_rate: f64) {
        self.coeff = if tau_s <= 0.0 || sample_rate <= 0.0 {
            1.0
        } else {
            1.0 - (-1.0 / (tau_s * sample_rate)).exp()
        };
    }

    /// Absolute snap distance. Scale this with the parameter's natural range
    /// so a 300 Hz carrier and a 0-1 gain both settle sensibly.
    pub fn set_epsilon(&mut self, epsilon: f64) {
        self.epsilon = epsilon.max(0.0);
    }

    #[inline]
    pub fn set_target(&mut self, target: f64) {
        self.target = target;
    }

    /// Jump immediately, skipping the glide. Used on preset load and on
    /// timeline seek, where a glide from the old value would be wrong.
    #[inline]
    pub fn reset_to(&mut self, value: f64) {
        self.current = value;
        self.target = value;
    }

    #[inline]
    pub fn target(&self) -> f64 {
        self.target
    }

    #[inline]
    pub fn current(&self) -> f64 {
        self.current
    }

    #[inline]
    pub fn is_settled(&self) -> bool {
        (self.target - self.current).abs() <= self.epsilon
    }

    #[inline]
    pub fn tick(&mut self) -> f64 {
        let delta = self.target - self.current;
        if delta.abs() <= self.epsilon {
            self.current = self.target;
        } else {
            self.current += delta * self.coeff;
        }
        self.current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reaches_target_and_settles_exactly() {
        let mut s = Smoother::new(0.0, 0.05, 48_000.0);
        s.set_target(440.0);
        for _ in 0..48_000 {
            s.tick();
        }
        assert!(s.is_settled());
        assert_eq!(s.current(), 440.0, "must settle exactly, not asymptotically");
    }

    #[test]
    fn tau_governs_rate() {
        // After exactly one tau the one-pole should have covered ~63.2%.
        let mut s = Smoother::new(0.0, 0.1, 48_000.0);
        s.set_target(1.0);
        for _ in 0..4_800 {
            s.tick();
        }
        assert!((s.current() - 0.632).abs() < 0.01, "got {}", s.current());
    }

    #[test]
    fn reset_skips_the_glide() {
        let mut s = Smoother::new(10.0, 0.5, 48_000.0);
        s.reset_to(4.0);
        assert_eq!(s.tick(), 4.0);
    }
}
