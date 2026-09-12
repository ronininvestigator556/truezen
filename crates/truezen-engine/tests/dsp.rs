//! Spectral and long-run verification of the synthesis engine.
//!
//! These assert on what is actually in the signal rather than on what the
//! code intended to put there. A wrong beat frequency, a clicking gate or a
//! drifting phase accumulator all sound "roughly fine" in casual listening
//! and are unambiguous in an FFT.

use realfft::RealFftPlanner;
use truezen_engine::engine::{Command, Engine};
use truezen_engine::layer::{LayerConfig, LayerKind};
use truezen_engine::noise::NoiseColor;
use truezen_engine::preset::Preset;
use truezen_engine::timeline::LayerParam;
use truezen_engine::{factory, Goal};

const SR: f64 = 48_000.0;
const BLOCK: usize = 1024;

fn preset_of(layers: Vec<LayerConfig>) -> Preset {
    Preset {
        id: "test".into(),
        name: "Test".into(),
        goal: Goal::Focus,
        master_gain: 0.9,
        layers,
        ..Default::default()
    }
}

fn engine_for(preset: &Preset) -> Engine {
    let mut e = Engine::new(SR);
    e.load_preset(preset);
    e.apply(Command::Play);
    e
}

/// Render `seconds`, returning interleaved stereo.
fn render(preset: &Preset, seconds: f64) -> Vec<f32> {
    let mut e = engine_for(preset);
    let mut buf = vec![0.0f32; (seconds * SR) as usize * 2];
    for c in buf.chunks_mut(BLOCK * 2) {
        e.process(c);
    }
    buf
}

/// Render `total_s` but keep only the final `tail_s`, so multi-hour runs stay
/// within a sensible memory footprint.
fn render_tail(preset: &Preset, total_s: f64, tail_s: f64) -> Vec<f32> {
    let mut e = engine_for(preset);
    let total_frames = (total_s * SR) as usize;
    let tail_frames = (tail_s * SR) as usize;
    let mut scratch = vec![0.0f32; BLOCK * 2];
    let mut tail = Vec::with_capacity(tail_frames * 2);

    let mut done = 0usize;
    while done < total_frames {
        let n = BLOCK.min(total_frames - done);
        let slice = &mut scratch[..n * 2];
        e.process(slice);
        if done + n > total_frames - tail_frames {
            tail.extend_from_slice(slice);
        }
        done += n;
    }
    tail
}

fn channel(buf: &[f32], ch: usize) -> Vec<f64> {
    buf[ch..].iter().step_by(2).map(|v| *v as f64).collect()
}

/// Windowed magnitude spectrum and its bin width, for a signal at `rate`.
fn spectrum_at(signal: &[f64], rate: f64) -> (Vec<f64>, f64) {
    let n = signal.len().next_power_of_two() / 2;
    let mut planner = RealFftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(n);
    let mut input = fft.make_input_vec();
    let mut output = fft.make_output_vec();

    let mut wsum = 0.0;
    for (i, slot) in input.iter_mut().enumerate() {
        // Hann. A rectangular window smears a pure sine across enough bins to
        // bury the sidebands these tests measure.
        let w = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos();
        wsum += w;
        *slot = signal[i] * w;
    }
    fft.process(&mut input, &mut output).unwrap();
    let scale = 2.0 / wsum;
    (output.iter().map(|c| c.norm() * scale).collect(), rate / n as f64)
}

fn spectrum(signal: &[f64]) -> (Vec<f64>, f64) {
    spectrum_at(signal, SR)
}

/// Frequency of the strongest component, refined by parabolic interpolation
/// so the result is not quantised to the bin width.
fn dominant_hz_at(signal: &[f64], rate: f64) -> f64 {
    let (mags, bin_hz) = spectrum_at(signal, rate);
    let i = (1..mags.len() - 1)
        .max_by(|&a, &b| mags[a].partial_cmp(&mags[b]).unwrap())
        .unwrap();
    let (a, b, c) = (mags[i - 1], mags[i], mags[i + 1]);
    let denom = a - 2.0 * b + c;
    let offset = if denom.abs() < 1e-18 { 0.0 } else { 0.5 * (a - c) / denom };
    (i as f64 + offset) * bin_hz
}

fn dominant_hz(signal: &[f64]) -> f64 {
    dominant_hz_at(signal, SR)
}

fn peak(buf: &[f32]) -> f64 {
    buf.iter().fold(0.0f64, |a, b| a.max(b.abs() as f64))
}

fn rms(buf: &[f32]) -> f64 {
    (buf.iter().map(|v| (*v as f64) * (*v as f64)).sum::<f64>() / buf.len().max(1) as f64).sqrt()
}

fn binaural(carrier: f64, beat: f64) -> LayerConfig {
    LayerConfig {
        kind: LayerKind::Binaural,
        carrier_hz: carrier,
        beat_hz: beat,
        gain: 0.8,
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------

/// The core claim of a binaural layer: each ear receives the carrier offset by
/// half the beat, so the interaural difference equals the beat exactly.
#[test]
fn binaural_carriers_are_split_by_exactly_the_beat() {
    for (carrier, beat) in [(200.0, 10.0), (136.1, 7.83), (100.0, 4.0), (300.0, 0.5)] {
        let buf = render(&preset_of(vec![binaural(carrier, beat)]), 20.0);
        let l = dominant_hz(&channel(&buf, 0));
        let r = dominant_hz(&channel(&buf, 1));

        assert!((l - (carrier - beat / 2.0)).abs() < 0.01, "left {l} for {carrier}/{beat}");
        assert!((r - (carrier + beat / 2.0)).abs() < 0.01, "right {r} for {carrier}/{beat}");
        assert!(
            ((r - l) - beat).abs() < 0.01,
            "beat came out {} instead of {beat}",
            r - l
        );
    }
}

/// Sub-hertz precision is a headline requirement: the user must be able to
/// dial 7.83 and get 7.83, not 7.8.
#[test]
fn fractional_beat_frequencies_are_honoured() {
    for beat in [0.10, 4.55, 7.83, 12.07] {
        let buf = render(&preset_of(vec![binaural(220.0, beat)]), 40.0);
        let measured = dominant_hz(&channel(&buf, 1)) - dominant_hz(&channel(&buf, 0));
        assert!(
            (measured - beat).abs() < 0.005,
            "asked {beat} Hz, measured {measured} Hz"
        );
    }
}

/// An isochronic layer is amplitude modulation, so its spectrum must be the
/// carrier plus sidebands at integer multiples of the beat.
#[test]
fn isochronic_sidebands_are_spaced_by_the_beat() {
    let (carrier, beat) = (300.0, 10.0);
    let buf = render(
        &preset_of(vec![LayerConfig {
            kind: LayerKind::Isochronic,
            carrier_hz: carrier,
            beat_hz: beat,
            gain: 0.8,
            ramp_ms: 8.0,
            ..Default::default()
        }]),
        20.0,
    );

    let (mags, bin_hz) = spectrum(&channel(&buf, 0));
    let at = |hz: f64| {
        let i = (hz / bin_hz).round() as usize;
        // Take a small neighbourhood: the true peak can sit between bins.
        mags[i - 2..=i + 2].iter().cloned().fold(0.0f64, f64::max)
    };

    let c = at(carrier);
    assert!(c > 0.0);
    for k in 1..=3 {
        let k = k as f64;
        let upper = at(carrier + k * beat);
        let lower = at(carrier - k * beat);
        assert!(upper > c * 0.01, "missing upper sideband {k}");
        assert!(lower > c * 0.01, "missing lower sideband {k}");
    }
    // Nothing between the sidebands: that gap is what says the modulation is
    // periodic at the beat rate and not something noisier.
    assert!(at(carrier + beat / 2.0) < c * 0.01, "energy between sidebands");
}

/// The raised-cosine ramp is the difference between a clean pulse and a click.
/// A hard gate scatters energy far from the carrier; the ramp must not.
#[test]
fn the_raised_cosine_ramp_suppresses_spectral_splatter() {
    let iso = |ramp_ms: f64| LayerConfig {
        kind: LayerKind::Isochronic,
        carrier_hz: 300.0,
        beat_hz: 10.0,
        gain: 0.8,
        ramp_ms,
        ..Default::default()
    };

    // Measured well above the carrier. An 8 ms ramp has a corner around
    // 125 Hz, so both ramps still have comparable energy a few hundred Hz out;
    // the difference only becomes stark past about 1 kHz. 2 kHz is
    // unambiguously in the region that should be empty.
    let far_energy = |ramp_ms: f64| {
        let buf = render(&preset_of(vec![iso(ramp_ms)]), 20.0);
        let (mags, bin_hz) = spectrum(&channel(&buf, 0));
        let start = (2_000.0 / bin_hz) as usize;
        mags[start..].iter().map(|m| m * m).sum::<f64>()
    };

    let smooth = far_energy(8.0);
    let harsh = far_energy(0.02);
    assert!(
        smooth < harsh * 0.01,
        "ramp did not suppress splatter: {smooth:.3e} smooth vs {harsh:.3e} hard-gated"
    );
}

/// A monaural layer sums both tones into each channel, so the beat exists as
/// real amplitude modulation in the air. Confirm the envelope actually
/// modulates at the beat rate.
#[test]
fn monaural_beat_appears_in_the_envelope() {
    let beat = 8.0;
    let buf = render(
        &preset_of(vec![LayerConfig {
            kind: LayerKind::Monaural,
            carrier_hz: 250.0,
            beat_hz: beat,
            gain: 0.8,
            ..Default::default()
        }]),
        40.0,
    );

    // Rectify to recover the envelope, then decimate hard. Rectifying a
    // 250 Hz carrier puts a strong component at 500 Hz, which would otherwise
    // dominate the spectrum and hide the 8 Hz envelope entirely.
    const DECIM: usize = 480;
    let env_rate = SR / DECIM as f64;
    let l = channel(&buf, 0);
    let env: Vec<f64> = l
        .chunks(DECIM)
        .map(|c| c.iter().map(|v| v.abs()).sum::<f64>() / c.len() as f64)
        .collect();
    let mean = env.iter().sum::<f64>() / env.len() as f64;
    let centred: Vec<f64> = env.iter().map(|v| v - mean).collect();

    let measured = dominant_hz_at(&centred, env_rate);
    assert!(
        (measured - beat).abs() < 0.1,
        "monaural envelope modulated at {measured} Hz, expected {beat}"
    );
}

/// Dialing a frequency during playback must not step the waveform. This is
/// the property the parameter smoothers exist to guarantee.
#[test]
fn sweeping_the_carrier_live_produces_no_discontinuity() {
    let mut e = engine_for(&preset_of(vec![binaural(100.0, 10.0)]));
    let mut out = vec![0.0f32; BLOCK * 2];
    let mut prev = 0.0f32;
    let mut max_delta = 0.0f64;

    // 400 abrupt retargets across two octaves -- far more violent than a real
    // drag, which arrives smoothly.
    for step in 0..400 {
        let carrier = 100.0 + 300.0 * (step as f64 / 400.0);
        e.apply(Command::SetLayerParam {
            index: 0,
            param: LayerParam::Carrier,
            value: carrier,
        });
        e.process(&mut out);
        // Step by 2: the buffer is interleaved, so adjacent entries are
        // opposite channels carrying different frequencies. Comparing across
        // them measures the stereo difference, not a discontinuity.
        for &s in out.iter().step_by(2) {
            max_delta = max_delta.max((s - prev).abs() as f64);
            prev = s;
        }
    }

    // A 400 Hz sine at this amplitude steps by at most ~0.019 per sample; a
    // zipper from an unsmoothed jump would be an order of magnitude larger.
    assert!(max_delta < 0.05, "carrier sweep produced a step of {max_delta}");
}

#[test]
fn seeking_does_not_produce_a_discontinuity() {
    let preset = factory::by_id("deep-theta").unwrap();
    let mut e = engine_for(&preset);
    let mut out = vec![0.0f32; BLOCK * 2];
    for _ in 0..200 {
        e.process(&mut out);
    }
    // Last left sample, to compare against the next block's first left sample.
    let before = out[out.len() - 2];

    // Jump deep into the session, where the automated beat is very different.
    e.apply(Command::Seek { seconds: 1500.0 });
    e.process(&mut out);
    let after = out[0];

    assert!(
        (after - before).abs() < 0.1,
        "seek jumped the output by {}",
        (after - before).abs()
    );
    assert!(out.iter().all(|s| s.is_finite()));
}

/// The limiter is the last line of defence: no stack of layers, however hot,
/// may exceed the ceiling.
#[test]
fn the_limiter_holds_the_ceiling_under_a_deliberately_hot_stack() {
    let layers: Vec<LayerConfig> = (0..8)
        .map(|i| LayerConfig {
            kind: LayerKind::Binaural,
            carrier_hz: 150.0 + i as f64 * 40.0,
            beat_hz: 6.0,
            gain: 1.0,
            ..Default::default()
        })
        .collect();
    let mut p = preset_of(layers);
    p.master_gain = 1.0;

    let buf = render(&p, 10.0);
    // -1 dBFS ceiling, with a hair of slack for f32 rounding.
    assert!(peak(&buf) <= 0.8913 + 1e-4, "peak {} exceeded ceiling", peak(&buf));
    assert!(buf.iter().all(|s| s.is_finite()));
}

/// Every shipped preset must render cleanly. This is the cheapest guard
/// against a bad edit to the preset JSON.
#[test]
fn every_factory_preset_renders_cleanly() {
    for preset in factory::load_all() {
        let buf = render(&preset, 20.0);

        assert!(buf.iter().all(|s| s.is_finite()), "{}: produced NaN or inf", preset.id);
        assert!(peak(&buf) <= 0.8913 + 1e-4, "{}: clipped at {}", preset.id, peak(&buf));
        assert!(rms(&buf) > 0.001, "{}: effectively silent", preset.id);

        let l = channel(&buf, 0);
        let dc = l.iter().sum::<f64>() / l.len() as f64;
        assert!(dc.abs() < 0.01, "{}: DC offset {dc}", preset.id);
    }
}

/// A noise-only layer must still be wide: independent generators per channel
/// give an enveloping bed, whereas a shared one collapses to a point between
/// the ears.
#[test]
fn noise_channels_are_decorrelated() {
    let buf = render(
        &preset_of(vec![LayerConfig {
            kind: LayerKind::Noise,
            noise_color: NoiseColor::Pink,
            gain: 0.8,
            ..Default::default()
        }]),
        10.0,
    );
    let (l, r) = (channel(&buf, 0), channel(&buf, 1));

    let n = l.len() as f64;
    let (ml, mr) = (l.iter().sum::<f64>() / n, r.iter().sum::<f64>() / n);
    let cov: f64 = l.iter().zip(&r).map(|(a, b)| (a - ml) * (b - mr)).sum::<f64>() / n;
    let sl = (l.iter().map(|a| (a - ml).powi(2)).sum::<f64>() / n).sqrt();
    let sr_ = (r.iter().map(|b| (b - mr).powi(2)).sum::<f64>() / n).sqrt();
    let corr = cov / (sl * sr_);

    assert!(corr.abs() < 0.1, "noise channels correlated at {corr}");
}

/// The property the f64 phase accumulator exists to guarantee. An f32
/// accumulator, or a `sin(2*pi*f*t)` formulation with a running `t`, drifts
/// measurably well before this point.
///
/// Renders four hours and analyses only the final ten seconds.
#[test]
fn the_beat_is_still_exact_after_four_hours() {
    let (carrier, beat) = (200.0, 7.83);
    let tail = render_tail(&preset_of(vec![binaural(carrier, beat)]), 4.0 * 3600.0, 10.0);

    let l = dominant_hz(&channel(&tail, 0));
    let r = dominant_hz(&channel(&tail, 1));
    let measured = r - l;

    assert!(
        (measured - beat).abs() < 0.01,
        "after 4h the beat read {measured} Hz instead of {beat} Hz"
    );
    assert!((l - (carrier - beat / 2.0)).abs() < 0.02, "left drifted to {l}");
    assert!((r - (carrier + beat / 2.0)).abs() < 0.02, "right drifted to {r}");
}

// ---------------------------------------------------------------------------
// Latch-on-touch
// ---------------------------------------------------------------------------

/// Beat of the first layer, after `seconds` of playback.
fn beat_after(engine: &mut Engine, seconds: f64) -> f64 {
    let frames = (seconds * SR) as usize;
    let mut out = vec![0.0f32; BLOCK * 2];
    let mut done = 0;
    while done < frames {
        engine.process(&mut out);
        done += BLOCK;
    }
    engine.meters().beat_hz as f64
}

/// Without latching, a manual edit to an automated parameter is undone by the
/// next automation block 1.3 ms later. `deep-theta` ramps its beat from 10 Hz
/// down, so the timeline is actively writing the whole time.
#[test]
fn touching_an_automated_parameter_latches_it() {
    let preset = factory::by_id("deep-theta").unwrap();
    let mut e = engine_for(&preset);

    beat_after(&mut e, 2.0);
    e.apply(Command::SetLayerParam {
        index: 0,
        param: LayerParam::Beat,
        value: 6.5,
    });

    // Long enough that the timeline would have reasserted itself many
    // thousands of times.
    let beat = beat_after(&mut e, 5.0);
    assert!(
        (beat - 6.5).abs() < 0.01,
        "timeline overwrote the manual value: beat is {beat}, expected 6.5"
    );
    assert_eq!(e.meters().latched_tracks, 0b1, "track was not reported latched");
}

#[test]
fn releasing_a_latch_returns_the_parameter_to_the_timeline() {
    let preset = factory::by_id("deep-theta").unwrap();
    let mut e = engine_for(&preset);

    beat_after(&mut e, 2.0);
    e.apply(Command::SetLayerParam {
        index: 0,
        param: LayerParam::Beat,
        value: 6.5,
    });
    assert!((beat_after(&mut e, 2.0) - 6.5).abs() < 0.01);

    e.apply(Command::UnlatchAll);
    let beat = beat_after(&mut e, 2.0);

    // Back under timeline control, which near the start of deep-theta is
    // close to 10 Hz.
    assert!(beat > 9.0, "beat did not return to the timeline: {beat}");
    assert_eq!(e.meters().latched_tracks, 0);
}

/// Latching must be per-track: dialing one parameter cannot silently freeze
/// another layer's automation.
#[test]
fn latching_one_parameter_leaves_the_others_automated() {
    let preset = factory::by_id("morning-activation").unwrap();
    // Tracks: 0 = layer0 beat, 1 = layer1 beat, 2 = layer1 gain.
    let mut e = engine_for(&preset);

    beat_after(&mut e, 1.0);
    e.apply(Command::SetLayerParam {
        index: 0,
        param: LayerParam::Beat,
        value: 15.0,
    });
    beat_after(&mut e, 2.0);

    assert_eq!(e.meters().latched_tracks, 0b001, "wrong tracks latched");

    // Layer 1's gain track is untouched, so it must still be climbing from 0.
    let gain = e.state().layers[1].config.gain;
    assert!(gain > 0.0, "an unlatched gain track stopped advancing");
}

/// A parameter with no automation track can still be set; there is simply
/// nothing to latch.
#[test]
fn setting_an_unautomated_parameter_latches_nothing() {
    let preset = factory::by_id("schumann-ground").unwrap();
    let mut e = engine_for(&preset);
    e.apply(Command::SetLayerParam {
        index: 0,
        param: LayerParam::Carrier,
        value: 180.0,
    });
    // Ten time constants of the 150 ms frequency smoother, so the glide has
    // demonstrably finished rather than merely got close.
    beat_after(&mut e, 1.5);
    assert_eq!(e.meters().latched_tracks, 0);
    assert!((e.meters().carrier_hz as f64 - 180.0).abs() < 0.1);
}

/// Loading a new preset must start from a clean slate; a latch carried over
/// from the previous session would silently disable automation.
#[test]
fn loading_a_preset_clears_every_latch() {
    let mut e = engine_for(&factory::by_id("deep-theta").unwrap());
    e.apply(Command::SetLayerParam {
        index: 0,
        param: LayerParam::Beat,
        value: 6.5,
    });
    beat_after(&mut e, 1.0);
    assert_ne!(e.meters().latched_tracks, 0);

    e.load_preset(&factory::by_id("deep-theta").unwrap());
    e.apply(Command::Play);
    let beat = beat_after(&mut e, 1.0);
    assert_eq!(e.meters().latched_tracks, 0, "latch survived a preset load");
    assert!(beat > 9.0, "automation did not resume: {beat}");
}

/// Gate timing changes must land on a cycle boundary. Shrinking the duty while
/// the gate is open would otherwise cut the tone off mid-pulse, which clicks.
#[test]
fn changing_duty_live_does_not_click() {
    let mut e = engine_for(&preset_of(vec![LayerConfig {
        kind: LayerKind::Isochronic,
        carrier_hz: 200.0,
        beat_hz: 6.0,
        gain: 0.8,
        ramp_ms: 8.0,
        ..Default::default()
    }]));

    let mut out = vec![0.0f32; BLOCK * 2];
    // Settle the startup ramp first.
    for _ in 0..40 {
        e.process(&mut out);
    }

    let mut prev = out[out.len() - 2];
    let mut max_delta = 0.0f64;
    for step in 0..300 {
        // Sweep the duty across its whole useful range, far faster than a drag.
        let duty = 0.15 + 0.7 * ((step as f64 * 0.05).sin() * 0.5 + 0.5);
        e.apply(Command::SetLayerParam {
            index: 0,
            param: LayerParam::Duty,
            value: duty,
        });
        e.process(&mut out);
        for &s in out.iter().step_by(2) {
            max_delta = max_delta.max((s - prev).abs() as f64);
            prev = s;
        }
    }

    // A 200 Hz carrier steps by at most ~0.02 per sample; a mid-pulse cut
    // would show a step near the full amplitude.
    assert!(max_delta < 0.05, "duty change clicked: step of {max_delta}");
}

/// Depth is a straight amplitude multiplier, so it is smoothed rather than
/// held to cycle boundaries.
#[test]
fn changing_depth_live_does_not_click() {
    let mut e = engine_for(&preset_of(vec![LayerConfig {
        kind: LayerKind::Isochronic,
        carrier_hz: 200.0,
        beat_hz: 6.0,
        gain: 0.8,
        ..Default::default()
    }]));

    let mut out = vec![0.0f32; BLOCK * 2];
    for _ in 0..40 {
        e.process(&mut out);
    }

    let mut prev = out[out.len() - 2];
    let mut max_delta = 0.0f64;
    for step in 0..200 {
        // Alternate between the extremes every block: the harshest case.
        let depth = if step % 2 == 0 { 0.0 } else { 1.0 };
        e.apply(Command::SetLayerParam {
            index: 0,
            param: LayerParam::Depth,
            value: depth,
        });
        e.process(&mut out);
        for &s in out.iter().step_by(2) {
            max_delta = max_delta.max((s - prev).abs() as f64);
            prev = s;
        }
    }
    assert!(max_delta < 0.05, "depth change clicked: step of {max_delta}");
}
