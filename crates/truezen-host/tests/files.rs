//! Decoding, resampling and looping real audio files.
//!
//! Every fixture is generated here rather than checked in, so the tests carry
//! no binary assets and cover whatever rates and layouts they need.

use std::path::PathBuf;

use truezen_engine::source::SampleSource;
use truezen_host::decoder::FileDecoder;
use truezen_host::filesource::{DirectSource, StreamedSource};

const ENGINE_RATE: u32 = 48_000;

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("truezen-files-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

/// A WAV of a steady tone, at whatever rate and channel count is asked for.
fn write_wav(name: &str, freq: f64, rate: u32, channels: u16, secs: f64) -> PathBuf {
    let path = tmp(name);
    let spec = hound::WavSpec {
        channels,
        sample_rate: rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(&path, spec).unwrap();
    let frames = (rate as f64 * secs) as usize;
    for i in 0..frames {
        let v = (std::f64::consts::TAU * freq * i as f64 / rate as f64).sin() * 0.5;
        let s = (v * i16::MAX as f64) as i16;
        for _ in 0..channels {
            w.write_sample(s).unwrap();
        }
    }
    w.finalize().unwrap();
    path
}

/// Read `frames`, advancing by what was actually produced so a slow streaming
/// source leaves no zero-filled holes in the middle of the result.
/// Noise, which is what an ambient bed actually is. Deterministic so a
/// failure reproduces.
fn write_noise_wav(name: &str, rate: u32, secs: f64) -> PathBuf {
    let path = tmp(name);
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(&path, spec).unwrap();
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    for _ in 0..(rate as f64 * secs) as usize {
        for _ in 0..2 {
            state ^= state >> 12;
            state ^= state << 25;
            state ^= state >> 27;
            let u = ((state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f64 / 8_388_608.0) - 1.0;
            w.write_sample((u * 0.4 * i16::MAX as f64) as i16).unwrap();
        }
    }
    w.finalize().unwrap();
    path
}

fn drain(source: &mut dyn SampleSource, frames: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; frames * 2];
    let mut done = 0;
    let mut idle = 0;
    while done < frames {
        let n = (frames - done).min(1024);
        let got = source.read(&mut out[done * 2..(done + n) * 2]);
        if got == 0 {
            if source.finished() || idle > 200 {
                break;
            }
            idle += 1;
            std::thread::sleep(std::time::Duration::from_millis(2));
            continue;
        }
        idle = 0;
        done += got;
    }
    out
}

fn dominant_hz(signal: &[f32], rate: u32) -> f64 {
    let mono: Vec<f64> = signal.iter().step_by(2).map(|v| *v as f64).collect();
    let n = mono.len().next_power_of_two() / 2;
    let mut planner = realfft::RealFftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(n);
    let mut input = fft.make_input_vec();
    let mut output = fft.make_output_vec();
    for (i, s) in input.iter_mut().enumerate() {
        let w = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos();
        *s = mono[i] * w;
    }
    fft.process(&mut input, &mut output).unwrap();
    let peak = (1..output.len())
        .max_by(|a, b| output[*a].norm().partial_cmp(&output[*b].norm()).unwrap())
        .unwrap();
    peak as f64 * rate as f64 / n as f64
}

fn rms(signal: &[f32]) -> f64 {
    (signal.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / signal.len().max(1) as f64).sqrt()
}

#[test]
fn decodes_a_wav_and_resamples_it_to_the_engine_rate() {
    let path = write_wav("tone-44k.wav", 440.0, 44_100, 2, 1.0);
    let mut d = FileDecoder::open(&path, ENGINE_RATE, false).unwrap();

    let mut out = vec![0.0f32; 48_000 * 2];
    let cap = out.len();
    let mut total = 0;
    while total < 48_000 {
        let end = (total * 2 + 4096).min(cap);
        let n = d.read(&mut out[total * 2..end]);
        if n == 0 {
            break;
        }
        total += n;
    }

    assert_eq!(d.source_rate(), 44_100, "did not report the file's rate");
    // One second in at 44.1 k should give about one second out at 48 k.
    assert!(total > 47_000, "only produced {total} frames from a 1 s file");
    let freq = dominant_hz(&out[..total * 2], ENGINE_RATE);
    assert!((freq - 440.0).abs() < 5.0, "tone came through at {freq} Hz");
}

/// A mono file must play from both ears, not just the left.
#[test]
fn mono_files_are_centred_not_left_only() {
    let path = write_wav("mono.wav", 300.0, 48_000, 1, 0.5);
    let mut s = DirectSource::open(&path, ENGINE_RATE, false).unwrap();
    let out = drain(&mut s, 20_000);

    let left: Vec<f32> = out.iter().step_by(2).copied().collect();
    let right: Vec<f32> = out.iter().skip(1).step_by(2).copied().collect();
    assert!(rms(&left) > 0.05, "mono file produced no left channel");
    assert!(rms(&right) > 0.05, "mono file produced no right channel");
    for (i, (l, r)) in left.iter().zip(&right).enumerate() {
        assert!((l - r).abs() < 1e-6, "channels differ at frame {i}");
    }
}

/// Matching rates should not go through the interpolator at all.
#[test]
fn a_file_already_at_the_engine_rate_passes_through() {
    let path = write_wav("tone-48k.wav", 1000.0, 48_000, 2, 0.5);
    let mut s = DirectSource::open(&path, ENGINE_RATE, false).unwrap();
    let out = drain(&mut s, 20_000);
    let freq = dominant_hz(&out, ENGINE_RATE);
    assert!((freq - 1000.0).abs() < 3.0, "tone moved to {freq} Hz");
}

/// The point of looping: a short bed keeps playing without a gap.
///
/// Tested with noise, because that is what a bed is. The crossfade is
/// equal-power, which is correct for decorrelated material; correlated
/// material — a sustained pure tone — can partially cancel across the seam
/// instead, and `a_looping_tone_has_no_click` covers that case separately.
#[test]
fn a_looping_bed_plays_past_its_own_length_without_a_gap() {
    let path = write_noise_wav("loop-noise.wav", 48_000, 0.5);
    let mut s = DirectSource::open(&path, ENGINE_RATE, true).unwrap();
    let out = drain(&mut s, 96_000);

    assert!(!s.finished(), "a looping source reported itself finished");

    let win = 480 * 2;
    for (i, chunk) in out.chunks(win).enumerate() {
        if i == 0 {
            continue;
        }
        assert!(
            rms(chunk) > 0.05,
            "silent gap at {:.2}s into the loop",
            (i * win / 2) as f64 / ENGINE_RATE as f64
        );
    }
}

/// However the seam is blended, it must not step. A click is audible in a way
/// a momentary level dip is not.
#[test]
fn a_looping_tone_has_no_click() {
    let path = write_wav("loop-tone.wav", 220.0, 48_000, 2, 0.5);
    let mut s = DirectSource::open(&path, ENGINE_RATE, true).unwrap();
    let out = drain(&mut s, 96_000);

    let left: Vec<f32> = out.iter().step_by(2).copied().collect();
    let worst = left.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
    // A 220 Hz sine at 0.5 moves at most 0.0144 per sample; a seam would show
    // a step near full scale.
    assert!(worst < 0.02, "loop seam clicked: step of {worst}");
}

#[test]
fn a_non_looping_file_ends_and_says_so() {
    let path = write_wav("short.wav", 500.0, 48_000, 2, 0.2);
    let mut s = DirectSource::open(&path, ENGINE_RATE, false).unwrap();
    let out = drain(&mut s, 48_000);
    assert!(s.finished(), "source did not report finishing");
    // The tail past the file's length must be silence, not repeats.
    let tail = &out[30_000 * 2..];
    assert!(rms(tail) < 1e-6, "played something after the file ended");
}

#[test]
fn a_missing_or_unreadable_file_is_an_error_not_a_panic() {
    assert!(DirectSource::open("/nonexistent/nope.wav", ENGINE_RATE, false).is_err());

    let junk = tmp("junk.wav");
    std::fs::write(&junk, b"this is not audio").unwrap();
    assert!(DirectSource::open(&junk, ENGINE_RATE, false).is_err());
}

/// The streaming path is what the audio callback actually uses.
#[test]
fn the_streamed_source_delivers_the_same_audio() {
    let path = write_wav("stream.wav", 660.0, 44_100, 2, 1.0);
    let mut s = StreamedSource::open(&path, ENGINE_RATE, true).unwrap();

    // Let the prebuffer fill before reading, as it would before playback.
    std::thread::sleep(std::time::Duration::from_millis(400));
    let out = drain(&mut s, 48_000);

    assert_eq!(s.underruns(), 0, "the decoder thread fell behind");
    let freq = dominant_hz(&out, ENGINE_RATE);
    assert!((freq - 660.0).abs() < 5.0, "streamed tone came through at {freq} Hz");
}
