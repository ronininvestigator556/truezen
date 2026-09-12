//! Sample-rate conversion for imported audio.
//!
//! Polyphase windowed-sinc interpolation. The kernel's cutoff scales with the
//! conversion ratio, so downsampling is band-limited by the same filter that
//! does the interpolation — there is no separate anti-alias stage to get wrong.
//!
//! Cheaper interpolators were tried first and measured: Catmull-Rom left spurs
//! only 50 dB down at 44.1 -> 48 kHz, which is audible on music. The tests
//! below measure this one rather than taking it on trust.
//!
//! This runs on the decoder thread, never in the audio callback, so a few
//! dozen multiply-adds per frame cost nothing that matters.

/// Interleaved stereo throughout.
const CH: usize = 2;
/// Taps either side of the read position. 16 gives a transition band tight
/// enough that the stopband sits below -80 dB.
const HALF_TAPS: usize = 16;
const TAPS: usize = HALF_TAPS * 2;
/// Sub-sample positions the kernel is precomputed at. The nearest is used;
/// 512 makes the resulting phase error negligible.
const PHASES: usize = 512;

/// Precomputed polyphase kernel: one row of `TAPS` coefficients per phase.
struct Kernel {
    rows: Vec<f32>,
}

impl Kernel {
    fn new(cutoff: f64) -> Self {
        let mut rows = vec![0.0f32; PHASES * TAPS];
        for q in 0..PHASES {
            let frac = q as f64 / PHASES as f64;
            let mut sum = 0.0;
            for k in 0..TAPS {
                // Tap offsets run from -(HALF_TAPS-1) to +HALF_TAPS.
                let t = (k as f64 - (HALF_TAPS - 1) as f64) - frac;
                let v = cutoff * sinc(cutoff * t) * blackman(t);
                rows[q * TAPS + k] = v as f32;
                sum += v;
            }
            // Normalise each phase to unity gain, so a constant input comes
            // out constant instead of rippling at the phase rate.
            if sum.abs() > 1e-12 {
                for k in 0..TAPS {
                    rows[q * TAPS + k] = (rows[q * TAPS + k] as f64 / sum) as f32;
                }
            }
        }
        Self { rows }
    }

    #[inline]
    fn row(&self, frac: f64) -> &[f32] {
        let q = ((frac * PHASES as f64) as usize).min(PHASES - 1);
        &self.rows[q * TAPS..(q + 1) * TAPS]
    }
}

#[inline]
fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-9 {
        1.0
    } else {
        let px = std::f64::consts::PI * x;
        px.sin() / px
    }
}

/// Blackman window across the tap span, zero outside it.
#[inline]
fn blackman(t: f64) -> f64 {
    let n = HALF_TAPS as f64;
    if t.abs() > n {
        return 0.0;
    }
    let u = (t + n) / (2.0 * n);
    0.42 - 0.5 * (std::f64::consts::TAU * u).cos() + 0.08 * (2.0 * std::f64::consts::TAU * u).cos()
}

pub struct Resampler {
    /// Input frames consumed per output frame.
    step: f64,
    /// Read position within `buf`, split so the fraction never loses
    /// precision. A single f64 index climbs with the length of the file and
    /// its low bits go with it, which makes the output depend on how the
    /// input happened to be chunked and drifts over a long bed.
    pos_int: usize,
    pos_frac: f64,
    buf: Vec<f32>,
    kernel: Option<Kernel>,
    passthrough: bool,
}

impl Resampler {
    pub fn new(source_rate: u32, target_rate: u32) -> Self {
        let passthrough = source_rate == target_rate;
        // Downsampling must band-limit to the new Nyquist; upsampling keeps
        // the full source band. The 0.92 leaves a little transition room
        // rather than pushing the corner right up against Nyquist.
        let cutoff = (target_rate as f64 / source_rate as f64).min(1.0) * 0.92;
        Self {
            step: source_rate as f64 / target_rate as f64,
            // Start past the history the kernel needs to look back over.
            pos_int: HALF_TAPS - 1,
            pos_frac: 0.0,
            // Pre-roll of silence, so output frame 0 lines up with input
            // frame 0 instead of being offset by the kernel's reach. Not
            // wanted when passing through, where the input is copied verbatim.
            buf: if passthrough {
                Vec::new()
            } else {
                vec![0.0; (HALF_TAPS - 1) * CH]
            },
            kernel: (!passthrough).then(|| Kernel::new(cutoff)),
            passthrough,
        }
    }

    pub fn passthrough(&self) -> bool {
        self.passthrough
    }

    /// Append interleaved stereo input at the source rate.
    pub fn push(&mut self, input: &[f32]) {
        self.buf.extend_from_slice(input);
    }

    /// Emit every output frame the buffered input can supply.
    pub fn pull(&mut self, out: &mut Vec<f32>) {
        if self.passthrough {
            out.append(&mut self.buf);
            return;
        }
        let Some(kernel) = self.kernel.as_ref() else { return };

        let frames = self.buf.len() / CH;
        while self.pos_int + HALF_TAPS + 1 < frames {
            let row = kernel.row(self.pos_frac);
            let base = (self.pos_int + 1 - HALF_TAPS) * CH;

            let mut l = 0.0f32;
            let mut r = 0.0f32;
            for (k, &h) in row.iter().enumerate() {
                l += self.buf[base + k * CH] * h;
                r += self.buf[base + k * CH + 1] * h;
            }
            out.push(l);
            out.push(r);

            self.pos_frac += self.step;
            let whole = self.pos_frac.floor();
            self.pos_int += whole as usize;
            self.pos_frac -= whole;
        }

        // Drop input the kernel can no longer reach back to.
        let keep_from = self.pos_int.saturating_sub(HALF_TAPS - 1);
        if keep_from > 0 {
            self.buf.drain(..keep_from * CH);
            self.pos_int -= keep_from;
        }
    }

    /// Flush the tail once no more input is coming, so the last frames of a
    /// file are not silently dropped.
    pub fn finish(&mut self, out: &mut Vec<f32>) {
        if self.passthrough {
            out.append(&mut self.buf);
            return;
        }
        self.buf.extend(std::iter::repeat_n(0.0, (HALF_TAPS + 2) * CH));
        self.pull(out);
        self.buf.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use realfft::RealFftPlanner;

    fn sine(freq: f64, rate: u32, secs: f64) -> Vec<f32> {
        let n = (rate as f64 * secs) as usize;
        (0..n)
            .flat_map(|i| {
                let v = (std::f64::consts::TAU * freq * i as f64 / rate as f64).sin() as f32 * 0.5;
                [v, v]
            })
            .collect()
    }

    /// (dominant frequency, its magnitude, the largest spur elsewhere).
    fn analyse(signal: &[f32], rate: u32) -> (f64, f64, f64) {
        let mono: Vec<f64> = signal.iter().step_by(2).map(|v| *v as f64).collect();
        let n = mono.len().next_power_of_two() / 2;
        let mut planner = RealFftPlanner::<f64>::new();
        let fft = planner.plan_fft_forward(n);
        let mut input = fft.make_input_vec();
        let mut output = fft.make_output_vec();
        // Blackman-Harris, not Hann. Hann's first sidelobe is only 31 dB down
        // and its skirt sits around -50 dB a few bins out, which is louder
        // than anything this resampler produces -- measuring through it
        // reports the window, not the signal.
        let mut wsum = 0.0;
        for (i, s) in input.iter_mut().enumerate() {
            let u = std::f64::consts::TAU * i as f64 / n as f64;
            let w = 0.35875 - 0.48829 * u.cos() + 0.14128 * (2.0 * u).cos()
                - 0.01168 * (3.0 * u).cos();
            wsum += w;
            *s = mono[i] * w;
        }
        fft.process(&mut input, &mut output).unwrap();
        let mags: Vec<f64> = output.iter().map(|c| c.norm() * 2.0 / wsum).collect();

        let peak_bin = (1..mags.len())
            .max_by(|a, b| mags[*a].partial_cmp(&mags[*b]).unwrap())
            .unwrap();
        let bin_hz = rate as f64 / n as f64;
        // Anything more than a few bins from the fundamental is a spur.
        let spur = mags
            .iter()
            .enumerate()
            .filter(|(i, _)| i.abs_diff(peak_bin) > 8 && *i > 0)
            .map(|(_, m)| *m)
            .fold(0.0f64, f64::max);
        (peak_bin as f64 * bin_hz, mags[peak_bin], spur)
    }

    #[test]
    fn matching_rates_pass_straight_through() {
        let mut r = Resampler::new(48_000, 48_000);
        assert!(r.passthrough());
        let input = sine(1000.0, 48_000, 0.1);
        let mut out = Vec::new();
        r.push(&input);
        r.pull(&mut out);
        assert_eq!(out, input, "passthrough altered the signal");
    }

    /// The property that matters: the tone stays where it was and nothing
    /// meaningful appears anywhere else.
    #[test]
    fn resampling_a_sine_stays_clean() {
        for (from, to) in [(44_100u32, 48_000u32), (48_000, 44_100), (22_050, 48_000)] {
            let mut r = Resampler::new(from, to);
            let mut out = Vec::new();
            r.push(&sine(1000.0, from, 1.0));
            r.pull(&mut out);

            let (freq, peak, spur) = analyse(&out, to);
            assert!(
                (freq - 1000.0).abs() < 5.0,
                "{from}->{to}: tone moved to {freq} Hz"
            );
            let spur_db = 20.0 * (spur / peak).log10();
            assert!(
                spur_db < -80.0,
                "{from}->{to}: worst spur only {spur_db:.1} dB down"
            );
        }
    }

    /// Output length should track the ratio, or the file drifts out of sync
    /// with the session over a long bed.
    #[test]
    fn output_length_tracks_the_ratio() {
        let mut r = Resampler::new(44_100, 48_000);
        let mut out = Vec::new();
        r.push(&sine(100.0, 44_100, 10.0));
        r.pull(&mut out);
        r.finish(&mut out);

        let frames = out.len() / 2;
        let expected = 10.0 * 48_000.0;
        let error = (frames as f64 - expected).abs() / expected;
        assert!(error < 0.001, "produced {frames} frames, expected about {expected}");
    }

    /// Downsampling must not fold high content back into the audible band.
    #[test]
    fn downsampling_rejects_content_above_the_new_nyquist() {
        // 18 kHz sampled at 48 k, resampled to 22.05 k: without the lowpass
        // this folds back to about 4 kHz.
        let mut r = Resampler::new(48_000, 22_050);
        let mut out = Vec::new();
        r.push(&sine(18_000.0, 48_000, 1.0));
        r.pull(&mut out);

        let (_, peak, _) = analyse(&out, 22_050);
        assert!(peak < 0.02, "aliased content came through at {peak:.4}");
    }

    #[test]
    fn feeding_in_small_chunks_matches_one_big_push() {
        let input = sine(440.0, 44_100, 0.5);
        let mut whole = Resampler::new(44_100, 48_000);
        let mut a = Vec::new();
        whole.push(&input);
        whole.pull(&mut a);

        let mut piecemeal = Resampler::new(44_100, 48_000);
        let mut b = Vec::new();
        for chunk in input.chunks(2 * 97) {
            piecemeal.push(chunk);
            piecemeal.pull(&mut b);
        }

        assert_eq!(a.len(), b.len(), "chunking changed the output length");
        for (i, (x, y)) in a.iter().zip(&b).enumerate() {
            assert!((x - y).abs() < 1e-6, "chunking changed sample {i}");
        }
    }
}
