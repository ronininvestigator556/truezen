//! Entrainment layers. A session is a stack of these summed together.

use std::f64::consts::{FRAC_PI_4, PI};

use serde::{Deserialize, Serialize};

use crate::noise::{FilterMode, NoiseColor, NoiseGen, Svf};
use crate::osc::{Phasor, Waveform};
use crate::smooth::Smoother;

/// How a layer produces its beat.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LayerKind {
    /// Two carriers a beat apart, one per ear. The beat is generated in the
    /// listener's brainstem and does not exist in the air, so this **requires
    /// headphones** -- on speakers the two tones mix acoustically and you hear
    /// a monaural beat instead.
    #[default]
    Binaural,
    /// Both tones summed into both channels. The beat is real amplitude
    /// modulation in the air, so it survives speakers.
    Monaural,
    /// One carrier, amplitude-gated at the beat rate. The most perceptually
    /// obvious of the three, and speaker-safe.
    Isochronic,
    /// Filtered noise bed. Masks room sound and gives the tones something to
    /// sit in.
    Noise,
}

fn t() -> bool {
    true
}
fn one() -> f64 {
    1.0
}
fn half() -> f64 {
    0.5
}
fn default_carrier() -> f64 {
    200.0
}
fn default_beat() -> f64 {
    10.0
}
fn default_gain() -> f64 {
    0.5
}
fn default_ramp_ms() -> f64 {
    8.0
}
fn default_cutoff() -> f64 {
    2_000.0
}
fn default_q() -> f64 {
    0.707
}

/// Serialisable layer settings. This is the on-disk preset shape, so every
/// field carries a `serde` default: a preset written by an older build must
/// still load.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LayerConfig {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub kind: LayerKind,
    #[serde(default = "t")]
    pub enabled: bool,

    /// Audible pitch the beat rides on. 100-500 Hz works best; binaural beat
    /// perception falls off above roughly 1 kHz.
    #[serde(default = "default_carrier")]
    pub carrier_hz: f64,
    /// The entrainment rate itself.
    #[serde(default = "default_beat")]
    pub beat_hz: f64,
    /// Linear, 0..1.
    #[serde(default = "default_gain")]
    pub gain: f64,
    /// -1 hard left .. +1 hard right. Ignored for binaural layers, where
    /// panning would break the interaural balance the effect depends on.
    #[serde(default)]
    pub pan: f64,
    #[serde(default)]
    pub waveform: Waveform,

    // --- isochronic ---
    /// Fraction of each cycle the tone is on.
    #[serde(default = "half")]
    pub duty: f64,
    /// Raised-cosine attack/release. Below ~5 ms the gate clicks and smears
    /// energy across the spectrum.
    #[serde(default = "default_ramp_ms")]
    pub ramp_ms: f64,
    /// 1.0 gates fully to silence; lower values gate to a floor.
    #[serde(default = "one")]
    pub depth: f64,

    /// Slow amplitude swell, in Hz. Used for breathing cues -- 0.1 Hz is six
    /// cycles a minute, the resonance-breathing rate. 0 disables.
    #[serde(default)]
    pub am_rate_hz: f64,
    /// 0 none .. 1 full depth.
    #[serde(default)]
    pub am_depth: f64,

    // --- noise ---
    #[serde(default)]
    pub noise_color: NoiseColor,
    #[serde(default)]
    pub filter_mode: FilterMode,
    #[serde(default = "default_cutoff")]
    pub filter_cutoff_hz: f64,
    #[serde(default = "default_q")]
    pub filter_q: f64,
    /// Slow cutoff drift, for surf and wind motion. 0 disables.
    #[serde(default)]
    pub lfo_rate_hz: f64,
    #[serde(default)]
    pub lfo_depth: f64,
}

impl Default for LayerConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            kind: LayerKind::default(),
            enabled: true,
            carrier_hz: default_carrier(),
            beat_hz: default_beat(),
            gain: default_gain(),
            pan: 0.0,
            waveform: Waveform::default(),
            duty: half(),
            ramp_ms: default_ramp_ms(),
            depth: one(),
            am_rate_hz: 0.0,
            am_depth: 0.0,
            noise_color: NoiseColor::default(),
            filter_mode: FilterMode::default(),
            filter_cutoff_hz: default_cutoff(),
            filter_q: default_q(),
            lfo_rate_hz: 0.0,
            lfo_depth: 0.0,
        }
    }
}

/// Equal-power pan. Constant perceived loudness across the image, unlike a
/// linear pan which dips ~3 dB in the middle.
#[inline]
fn equal_power(pan: f64) -> (f64, f64) {
    let angle = (pan.clamp(-1.0, 1.0) + 1.0) * FRAC_PI_4;
    (angle.cos(), angle.sin())
}

/// Runtime state for one layer. Built from a `LayerConfig`; parameter changes
/// arrive as new target values on the smoothers rather than direct writes.
pub struct Layer {
    pub config: LayerConfig,
    sample_rate: f64,

    carrier: Smoother,
    beat: Smoother,
    gain: Smoother,
    pan: Smoother,
    /// Depth is a plain amplitude multiplier, so unlike the timing parameters
    /// it must glide or a change steps the output.
    depth: Smoother,

    /// Gate shape held for the duration of the current envelope cycle.
    cycle_duty: f64,
    cycle_ramp_ms: f64,
    last_env_phase: f64,

    // Two phasors serve the two carriers of a binaural or monaural pair, and
    // `osc_a` alone serves isochronic.
    osc_a: Phasor,
    osc_b: Phasor,
    env: Phasor,

    noise_l: NoiseGen,
    noise_r: NoiseGen,
    svf_l: Svf,
    svf_r: Svf,
    lfo: Phasor,
    am: Phasor,
}

/// Frequency glides slower than gain: fast enough to feel responsive under a
/// dragging finger, slow enough that no step is audible as a click.
const TAU_FREQ: f64 = 0.15;
const TAU_GAIN: f64 = 0.02;

impl Layer {
    pub fn new(config: LayerConfig, sample_rate: f64, seed: u64) -> Self {
        let mut carrier = Smoother::new(config.carrier_hz, TAU_FREQ, sample_rate);
        carrier.set_epsilon(1e-4);
        let mut beat = Smoother::new(config.beat_hz, TAU_FREQ, sample_rate);
        beat.set_epsilon(1e-6);
        let mut gain = Smoother::new(0.0, TAU_GAIN, sample_rate);
        // Start silent and glide up, so adding a layer mid-session fades in.
        gain.set_target(if config.enabled { config.gain } else { 0.0 });
        let pan = Smoother::new(config.pan, TAU_GAIN, sample_rate);
        let depth = Smoother::new(config.depth, TAU_GAIN, sample_rate);
        let (cycle_duty, cycle_ramp_ms) = (config.duty, config.ramp_ms);

        let mut layer = Self {
            config,
            sample_rate,
            carrier,
            beat,
            gain,
            pan,
            depth,
            cycle_duty,
            cycle_ramp_ms,
            last_env_phase: 0.0,
            osc_a: Phasor::new(),
            osc_b: Phasor::new(),
            env: Phasor::new(),
            noise_l: NoiseGen::new(seed ^ 0xA5A5_A5A5),
            noise_r: NoiseGen::new(seed ^ 0x5A5A_5A5A),
            svf_l: Svf::new(),
            svf_r: Svf::new(),
            // Offset so the two channels' motion is not identical.
            lfo: Phasor::with_phase(0.0),
            // Start at trough so a breathing cue swells in rather than
            // beginning at full level.
            am: Phasor::with_phase(0.75),
        };
        layer.refresh_filter();
        layer
    }

    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    fn refresh_filter(&mut self) {
        self.svf_l
            .set(self.config.filter_cutoff_hz, self.config.filter_q, self.sample_rate);
        self.svf_r
            .set(self.config.filter_cutoff_hz, self.config.filter_q, self.sample_rate);
    }

    pub fn set_carrier(&mut self, hz: f64) {
        self.config.carrier_hz = hz;
        self.carrier.set_target(hz);
    }

    pub fn set_beat(&mut self, hz: f64) {
        self.config.beat_hz = hz;
        self.beat.set_target(hz);
    }

    pub fn set_gain(&mut self, g: f64) {
        self.config.gain = g;
        if self.config.enabled {
            self.gain.set_target(g);
        }
    }

    pub fn set_pan(&mut self, p: f64) {
        self.config.pan = p;
        self.pan.set_target(p);
    }

    pub fn set_enabled(&mut self, on: bool) {
        self.config.enabled = on;
        // Glide rather than cut, so toggling a layer never clicks.
        self.gain.set_target(if on { self.config.gain } else { 0.0 });
    }

    /// Fraction of each cycle the gate is open. Takes effect at the start of
    /// the next cycle.
    pub fn set_duty(&mut self, duty: f64) {
        self.config.duty = duty.clamp(0.01, 1.0);
    }

    /// Gate attack/release in milliseconds. Takes effect at the next cycle.
    pub fn set_ramp_ms(&mut self, ms: f64) {
        self.config.ramp_ms = ms.max(0.0);
    }

    pub fn set_depth(&mut self, depth: f64) {
        let d = depth.clamp(0.0, 1.0);
        self.config.depth = d;
        self.depth.set_target(d);
    }

    pub fn set_filter(&mut self, cutoff_hz: f64, q: f64) {
        self.config.filter_cutoff_hz = cutoff_hz;
        self.config.filter_q = q;
        self.refresh_filter();
    }

    /// Post-smoothing beat rate: what is actually sounding right now, which
    /// during a ramp differs from the configured target. The UI shows this.
    #[inline]
    pub fn current_beat(&self) -> f64 {
        self.beat.current()
    }

    #[inline]
    pub fn current_carrier(&self) -> f64 {
        self.carrier.current()
    }

    /// Collapse all glides to their targets. Used after a seek, where gliding
    /// from the pre-seek values would be meaningless.
    pub fn snap_parameters(&mut self) {
        self.carrier.reset_to(self.carrier.target());
        self.beat.reset_to(self.beat.target());
        self.gain.reset_to(self.gain.target());
        self.pan.reset_to(self.pan.target());
        self.depth.reset_to(self.depth.target());
    }

    /// True once a disabled layer has finished fading out, meaning it can be
    /// skipped entirely.
    #[inline]
    pub fn is_silent(&self) -> bool {
        !self.config.enabled && self.gain.is_settled() && self.gain.current() <= 0.0
    }

    /// The isochronic gate value for the current envelope phase.
    #[inline]
    fn envelope(&mut self, beat_hz: f64) -> f64 {
        let depth = self.depth.tick().clamp(0.0, 1.0);

        // Below this the gate period exceeds a couple of minutes; treat it as
        // a continuous tone rather than letting the ramp maths degenerate.
        if beat_hz < 0.01 {
            return 1.0;
        }
        self.env.set_freq(beat_hz, self.sample_rate);
        let p = self.env.tick();

        // Adopt new gate timing only at a cycle boundary. Applying a shorter
        // duty immediately could put the phase past the closing edge and cut
        // the tone off mid-pulse, which clicks.
        if p < self.last_env_phase {
            self.cycle_duty = self.config.duty.clamp(0.01, 1.0);
            self.cycle_ramp_ms = self.config.ramp_ms.max(0.0);
        }
        self.last_env_phase = p;

        let duty = self.cycle_duty;

        let gate = if p >= duty {
            0.0
        } else {
            // Position within the "on" window, 0..1.
            let pos = p / duty;
            // Convert the ramp from milliseconds to a fraction of that window.
            // Both ramps must fit, hence the 0.5 clamp.
            let ramp = ((self.cycle_ramp_ms / 1000.0) * beat_hz / duty).clamp(1e-4, 0.5);
            if pos < ramp {
                0.5 - 0.5 * (PI * pos / ramp).cos()
            } else if pos > 1.0 - ramp {
                0.5 - 0.5 * (PI * (1.0 - pos) / ramp).cos()
            } else {
                1.0
            }
        };

        (1.0 - depth) + depth * gate
    }

    /// Produce one stereo frame.
    #[inline]
    pub fn tick(&mut self) -> (f64, f64) {
        let gain = self.gain.tick();
        let carrier = self.carrier.tick();
        let beat = self.beat.tick();
        let pan = self.pan.tick();

        if gain <= 0.0 {
            // Still advance the smoothers (done above) but skip synthesis.
            return (0.0, 0.0);
        }

        let wave = self.config.waveform;
        let sr = self.sample_rate;

        let (mut l, mut r) = match self.config.kind {
            LayerKind::Binaural => {
                // The half-beat split keeps the perceived pitch centred on the
                // carrier as the beat rate changes.
                self.osc_a.set_freq(carrier - beat * 0.5, sr);
                self.osc_b.set_freq(carrier + beat * 0.5, sr);
                (self.osc_a.tick_wave(wave), self.osc_b.tick_wave(wave))
            }
            LayerKind::Monaural => {
                self.osc_a.set_freq(carrier - beat * 0.5, sr);
                self.osc_b.set_freq(carrier + beat * 0.5, sr);
                // Summing before the pan is what makes the beat physical.
                let m = (self.osc_a.tick_wave(wave) + self.osc_b.tick_wave(wave)) * 0.5;
                (m, m)
            }
            LayerKind::Isochronic => {
                self.osc_a.set_freq(carrier, sr);
                let m = self.osc_a.tick_wave(wave) * self.envelope(beat);
                (m, m)
            }
            LayerKind::Noise => {
                let color = self.config.noise_color;
                let mode = self.config.filter_mode;

                if self.config.lfo_rate_hz > 0.0 && self.config.lfo_depth > 0.0 {
                    self.lfo.set_freq(self.config.lfo_rate_hz, sr);
                    let m = (self.lfo.tick() * std::f64::consts::TAU).sin();
                    // Modulate in octaves so the sweep sounds even.
                    let octaves = m * self.config.lfo_depth.clamp(0.0, 4.0);
                    let cutoff = self.config.filter_cutoff_hz * 2f64.powf(octaves);
                    self.svf_l.set(cutoff, self.config.filter_q, sr);
                    self.svf_r.set(cutoff, self.config.filter_q, sr);
                }

                // Independent generators per channel: decorrelated noise is
                // wide and enveloping, whereas mono noise collapses to a point
                // between the ears.
                let nl = self.noise_l.tick(color);
                let nr = self.noise_r.tick(color);
                (self.svf_l.tick(nl, mode), self.svf_r.tick(nr, mode))
            }
        };

        // Panning a binaural layer would unbalance the two ears and weaken or
        // destroy the beat, so it is deliberately not applied here.
        if self.config.kind != LayerKind::Binaural {
            let (gl, gr) = equal_power(pan);
            l *= gl * std::f64::consts::SQRT_2;
            r *= gr * std::f64::consts::SQRT_2;
        }

        let mut gain = gain;
        if self.config.am_rate_hz > 0.0 && self.config.am_depth > 0.0 {
            self.am.set_freq(self.config.am_rate_hz, sr);
            let d = self.config.am_depth.clamp(0.0, 1.0);
            let s = 0.5 + 0.5 * (self.am.tick() * std::f64::consts::TAU).sin();
            gain *= 1.0 - d + d * s;
        }

        (l * gain, r * gain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_power_holds_constant_energy() {
        for pan in [-1.0, -0.5, 0.0, 0.5, 1.0] {
            let (l, r) = equal_power(pan);
            assert!((l * l + r * r - 1.0).abs() < 1e-12, "pan {pan} not equal power");
        }
    }

    #[test]
    fn binaural_channels_differ_but_monaural_channels_match() {
        let cfg = LayerConfig {
            kind: LayerKind::Binaural,
            carrier_hz: 200.0,
            beat_hz: 10.0,
            gain: 1.0,
            ..Default::default()
        };
        let mut bin = Layer::new(cfg.clone(), 48_000.0, 1);
        let mut differed = false;
        for _ in 0..48_000 {
            let (l, r) = bin.tick();
            if (l - r).abs() > 1e-6 {
                differed = true;
            }
        }
        assert!(differed, "binaural channels were identical");

        let mut mono = Layer::new(
            LayerConfig {
                kind: LayerKind::Monaural,
                ..cfg
            },
            48_000.0,
            1,
        );
        for _ in 0..48_000 {
            let (l, r) = mono.tick();
            assert!((l - r).abs() < 1e-12, "monaural channels diverged");
        }
    }

    #[test]
    fn isochronic_envelope_is_continuous() {
        // A hard gate would show a step of ~1.0 here; the raised cosine must
        // keep every sample-to-sample delta small.
        let mut layer = Layer::new(
            LayerConfig {
                kind: LayerKind::Isochronic,
                carrier_hz: 200.0,
                beat_hz: 10.0,
                gain: 1.0,
                ramp_ms: 8.0,
                ..Default::default()
            },
            48_000.0,
            1,
        );
        // Let the gain smoother settle first.
        for _ in 0..10_000 {
            layer.tick();
        }
        let mut prev = layer.tick().0;
        let mut max_delta: f64 = 0.0;
        for _ in 0..48_000 {
            let v = layer.tick().0;
            max_delta = max_delta.max((v - prev).abs());
            prev = v;
        }
        // The carrier itself steps by up to 2*pi*f/sr ~= 0.026 per sample.
        assert!(max_delta < 0.1, "isochronic gate clicked: delta {max_delta}");
    }

    #[test]
    fn zero_beat_isochronic_stays_on() {
        let mut layer = Layer::new(
            LayerConfig {
                kind: LayerKind::Isochronic,
                beat_hz: 0.0,
                gain: 1.0,
                ..Default::default()
            },
            48_000.0,
            1,
        );
        for _ in 0..5_000 {
            layer.tick();
        }
        let energy: f64 = (0..48_000).map(|_| layer.tick().0.abs()).sum();
        assert!(energy > 1000.0, "zero-beat gate silenced the carrier");
    }

    #[test]
    fn disabled_layer_fades_out_rather_than_cutting() {
        let mut layer = Layer::new(
            LayerConfig {
                gain: 1.0,
                ..Default::default()
            },
            48_000.0,
            1,
        );
        for _ in 0..10_000 {
            layer.tick();
        }
        layer.set_enabled(false);
        let first = layer.tick().0.abs();
        assert!(first > 0.0, "layer cut to silence instantly");
        for _ in 0..48_000 {
            layer.tick();
        }
        assert!(layer.is_silent());
    }
}
