//! Rendering a session to an audio file, faster than real time.
//!
//! This drives the same [`Engine::process`] as live playback, so an export is
//! the session, not an approximation of it. File layers are decoded directly
//! rather than through a ring buffer: offline there is no deadline to miss,
//! and a ring would simply starve when the renderer outruns the decoder.

use std::path::Path;

use truezen_engine::engine::{Command, Engine, SessionState};
use truezen_engine::layer::LayerKind;
use truezen_engine::mixer::Dither;
use truezen_engine::preset::Preset;

use crate::filesource::DirectSource;

const BLOCK_FRAMES: usize = 4096;

#[derive(Clone, Copy, Debug)]
pub struct ExportOptions {
    pub sample_rate: u32,
    /// 16, 24, or 32 (32 writes float).
    pub bits: u16,
    /// Overrides the preset's own length. Required for open-ended presets.
    pub seconds: Option<f64>,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            bits: 24,
            seconds: None,
        }
    }
}

/// Render `preset` to a WAV file at `path`.
///
/// `on_progress` is called with 0..1 as the render proceeds, so a long session
/// does not look like a hang.
pub fn render_to_wav(
    preset: &Preset,
    opts: ExportOptions,
    path: impl AsRef<Path>,
    mut on_progress: impl FnMut(f64),
) -> Result<f64, String> {
    preset.validate()?;

    let seconds = match opts.seconds.or_else(|| {
        let d = preset.duration_s();
        (d > 0.0).then_some(d)
    }) {
        Some(s) if s > 0.0 => s,
        _ => return Err("this preset runs until you stop it, so an export needs a length".into()),
    };

    let rate = opts.sample_rate;
    let mut engine = Engine::new(rate as f64);
    let mut state = SessionState::from_preset(preset, rate as f64);

    // Attach file layers before the session starts, so nothing is missing from
    // the first block.
    for (i, cfg) in preset.layers.iter().enumerate() {
        if cfg.kind != LayerKind::File {
            continue;
        }
        let Some(file) = cfg.file_path.as_ref() else {
            continue;
        };
        let source = DirectSource::open(file, rate, cfg.loop_file)
            .map_err(|e| format!("layer {} ({}): {e}", i + 1, cfg.name))?;
        if let Some(layer) = state.layers.get_mut(i) {
            layer.attach_source(Box::new(source));
        }
    }

    engine.apply(Command::Replace(Box::new(state)));
    engine.apply(Command::Play);

    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: rate,
        bits_per_sample: opts.bits,
        sample_format: if opts.bits == 32 {
            hound::SampleFormat::Float
        } else {
            hound::SampleFormat::Int
        },
    };
    let mut writer = hound::WavWriter::create(path.as_ref(), spec)
        .map_err(|e| format!("could not create {}: {e}", path.as_ref().display()))?;

    let total_frames = (seconds * rate as f64) as usize;
    let mut dither = (opts.bits != 32).then(|| Dither::new(opts.bits as u32, 0x7A5E));
    let peak = ((1i64 << (opts.bits.min(31) - 1)) - 1) as f64;

    let mut block = vec![0.0f32; BLOCK_FRAMES * 2];
    let mut done = 0usize;
    let mut last_report = 0.0;

    while done < total_frames {
        let frames = BLOCK_FRAMES.min(total_frames - done);
        let slice = &mut block[..frames * 2];
        engine.process(slice);

        match dither.as_mut() {
            None => {
                for s in slice.iter() {
                    writer.write_sample(*s).map_err(|e| e.to_string())?;
                }
            }
            Some(d) => {
                for s in slice.iter() {
                    let v = d.process(*s as f64).clamp(-1.0, 1.0);
                    writer
                        .write_sample((v * peak) as i32)
                        .map_err(|e| e.to_string())?;
                }
            }
        }

        done += frames;
        let progress = done as f64 / total_frames as f64;
        // Report about every percent, not every block: a 75-minute render is
        // over a thousand blocks and the UI does not need all of them.
        if progress - last_report >= 0.01 || done >= total_frames {
            last_report = progress;
            on_progress(progress);
        }
    }

    writer
        .finalize()
        .map_err(|e| format!("could not finish the file: {e}"))?;
    Ok(seconds)
}
