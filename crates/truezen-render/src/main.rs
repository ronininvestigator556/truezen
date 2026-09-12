//! Offline renderer and spectral analyser.
//!
//! This exists so the DSP can be proven before any UI exists. A wrong beat
//! frequency or a clicking gate is near-impossible to diagnose through a GUI
//! and trivial to catch with an FFT.

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use realfft::RealFftPlanner;
use truezen_engine::engine::{Command, Engine};
use truezen_engine::mixer::Dither;
use truezen_engine::preset::Preset;
use truezen_engine::factory;

/// Block size for offline rendering. Matches a typical device buffer so the
/// offline path exercises the same automation cadence as live playback.
const BLOCK_FRAMES: usize = 1024;

#[derive(Parser)]
#[command(name = "truezen-render", about = "Render and analyse TrueZen sessions offline")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List the built-in presets.
    List,
    /// Render a session to a WAV file.
    Render {
        /// Factory preset id, or a path to a preset JSON file.
        preset: String,
        #[arg(short, long, default_value = "out.wav")]
        out: String,
        /// Seconds to render. Defaults to the preset's own duration; required
        /// for presets that run indefinitely.
        #[arg(short, long)]
        seconds: Option<f64>,
        #[arg(long, default_value_t = 48_000)]
        sample_rate: u32,
        /// 16, 24, or 32 (32 = float).
        #[arg(long, default_value_t = 24)]
        bits: u16,
    },
    /// Render in memory and report what is actually in the signal.
    Analyze {
        preset: String,
        #[arg(short, long, default_value_t = 20.0)]
        seconds: f64,
        #[arg(long, default_value_t = 48_000)]
        sample_rate: u32,
        /// Skip this many seconds before analysing, to let ramps settle.
        #[arg(long, default_value_t = 2.0)]
        skip: f64,
        #[arg(long, default_value_t = 6)]
        peaks: usize,
    },
}

fn load(spec: &str) -> Result<Preset> {
    if let Some(p) = factory::by_id(spec) {
        return Ok(p);
    }
    let text = std::fs::read_to_string(spec)
        .with_context(|| format!("'{spec}' is neither a factory preset id nor a readable file"))?;
    let p = Preset::from_json(&text).with_context(|| format!("parsing {spec}"))?;
    p.validate().map_err(anyhow::Error::msg)?;
    Ok(p)
}

/// Render `seconds` of a preset into an interleaved stereo buffer.
fn render(preset: &Preset, seconds: f64, sample_rate: u32) -> Vec<f32> {
    let mut engine = Engine::new(sample_rate as f64);
    engine.load_preset(preset);
    engine.apply(Command::Play);

    let total = (seconds * sample_rate as f64) as usize;
    let mut buf = vec![0.0f32; total * 2];
    for chunk in buf.chunks_mut(BLOCK_FRAMES * 2) {
        engine.process(chunk);
    }
    buf
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::List => {
            let presets = factory::load_all();
            println!("{} factory presets\n", presets.len());
            for p in presets {
                let dur = p.duration_s();
                let dur = if dur > 0.0 {
                    format!("{:.0} min", dur / 60.0)
                } else {
                    "open-ended".into()
                };
                let hp = if p.requires_headphones { "  [headphones]" } else { "" };
                println!("  {:<24} {:<12} {:>10}{}", p.id, p.goal.label(), dur, hp);
            }
        }

        Cmd::Render { preset, out, seconds, sample_rate, bits } => {
            let preset = load(&preset)?;
            let seconds = match seconds.or_else(|| {
                let d = preset.duration_s();
                (d > 0.0).then_some(d)
            }) {
                Some(s) => s,
                None => bail!("'{}' runs indefinitely; pass --seconds", preset.id),
            };

            let started = std::time::Instant::now();
            let buf = render(&preset, seconds, sample_rate);
            let elapsed = started.elapsed().as_secs_f64();

            let spec = hound::WavSpec {
                channels: 2,
                sample_rate,
                bits_per_sample: bits,
                sample_format: if bits == 32 {
                    hound::SampleFormat::Float
                } else {
                    hound::SampleFormat::Int
                },
            };
            let mut w = hound::WavWriter::create(&out, spec)?;
            match bits {
                32 => {
                    for s in &buf {
                        w.write_sample(*s)?;
                    }
                }
                24 | 16 => {
                    // Dither only when quantising. Undithered truncation of a
                    // long sustained sine leaves correlated error that reads
                    // as a faint tone rather than as noise.
                    let mut d = Dither::new(bits as u32, 0x7A5E);
                    let peak = ((1i32 << (bits - 1)) - 1) as f64;
                    for s in &buf {
                        let v = d.process(*s as f64).clamp(-1.0, 1.0);
                        w.write_sample((v * peak) as i32)?;
                    }
                }
                other => bail!("unsupported bit depth {other}; use 16, 24 or 32"),
            }
            w.finalize()?;

            println!(
                "{} -> {out}  ({:.0}s of audio in {:.2}s, {:.0}x real time)",
                preset.id,
                seconds,
                elapsed,
                seconds / elapsed.max(1e-9)
            );
        }

        Cmd::Analyze { preset, seconds, sample_rate, skip, peaks } => {
            let preset = load(&preset)?;
            let buf = render(&preset, seconds + skip, sample_rate);

            let skip_frames = (skip * sample_rate as f64) as usize;
            let (l, r) = deinterleave(&buf, skip_frames);
            println!("{}  ({} Hz, {:.0}s analysed)\n", preset.id, sample_rate, seconds);

            let lp = top_peaks(&l, sample_rate as f64, peaks);
            let rp = top_peaks(&r, sample_rate as f64, peaks);
            println!("  {:<28}RIGHT", "LEFT");
            for i in 0..peaks {
                let fmt = |p: Option<&(f64, f64)>| {
                    p.map(|(f, m)| format!("{f:>9.3} Hz  {:>6.1} dB", 20.0 * m.log10()))
                        .unwrap_or_default()
                };
                println!("  {:<28}{}", fmt(lp.get(i)), fmt(rp.get(i)));
            }

            // For a binaural layer the two ears carry different frequencies
            // and their difference IS the beat, so this is the direct check
            // that the layer is doing what the preset claims.
            if let (Some((lf, _)), Some((rf, _))) = (lp.first(), rp.first()) {
                let delta = (rf - lf).abs();
                println!("\n  dominant L/R difference: {delta:.4} Hz");
            }
            println!("  peak: {:.3} dBFS", 20.0 * peak(&buf).max(1e-12).log10());
            println!("  DC:   L {:+.2e}  R {:+.2e}", mean(&l), mean(&r));
        }
    }
    Ok(())
}

fn deinterleave(buf: &[f32], skip_frames: usize) -> (Vec<f64>, Vec<f64>) {
    let s = skip_frames * 2;
    let l = buf[s..].iter().step_by(2).map(|v| *v as f64).collect();
    let r = buf[s + 1..].iter().step_by(2).map(|v| *v as f64).collect();
    (l, r)
}

fn peak(buf: &[f32]) -> f64 {
    buf.iter().fold(0.0f64, |a, b| a.max(b.abs() as f64))
}

fn mean(buf: &[f64]) -> f64 {
    buf.iter().sum::<f64>() / buf.len().max(1) as f64
}

/// Strongest spectral components, as (frequency, magnitude).
///
/// A Hann window is essential here: without it the rectangular window's
/// sidelobes smear a 200 Hz sine across tens of bins and swamp the very
/// sidebands we are trying to measure.
fn top_peaks(signal: &[f64], sample_rate: f64, want: usize) -> Vec<(f64, f64)> {
    let n = signal.len().next_power_of_two() / 2;
    if n < 1024 {
        return Vec::new();
    }
    let mut planner = RealFftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(n);
    let mut input = fft.make_input_vec();
    let mut output = fft.make_output_vec();

    let mut window_sum = 0.0;
    for (i, slot) in input.iter_mut().enumerate() {
        let w = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos();
        window_sum += w;
        *slot = signal[i] * w;
    }
    fft.process(&mut input, &mut output).ok();

    let scale = 2.0 / window_sum;
    let mags: Vec<f64> = output.iter().map(|c| c.norm() * scale).collect();

    // Keep only local maxima, so one strong partial does not occupy every
    // slot with its own skirt.
    let mut found: Vec<(f64, f64)> = (1..mags.len() - 1)
        .filter(|&i| mags[i] > mags[i - 1] && mags[i] >= mags[i + 1])
        .map(|i| (interpolated_bin(&mags, i) * sample_rate / n as f64, mags[i]))
        .collect();
    found.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    found.truncate(want);
    found
}

/// Parabolic interpolation across the peak and its neighbours. Without it the
/// reported frequency is quantised to the bin width (~0.7 Hz here), which is
/// far too coarse to verify a 0.01 Hz beat setting.
fn interpolated_bin(mags: &[f64], i: usize) -> f64 {
    let (a, b, c) = (mags[i - 1], mags[i], mags[i + 1]);
    let denom = a - 2.0 * b + c;
    if denom.abs() < 1e-18 {
        return i as f64;
    }
    i as f64 + 0.5 * (a - c) / denom
}
