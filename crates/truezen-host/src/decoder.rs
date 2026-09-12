//! Decoding user audio files into stereo at the engine's sample rate.
//!
//! Blocking and allocating by nature, so this never runs in the audio
//! callback — [`crate::filesource`] puts it behind a ring buffer for live use,
//! and the offline renderer drives it directly.

use std::collections::VecDeque;
use std::fs::File;
use std::path::{Path, PathBuf};

use symphonia::core::audio::GenericAudioBufferRef;
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

use crate::resample::Resampler;

const CH: usize = 2;
/// Length of the loop crossfade. Long enough to hide a seam in ambient
/// material, short enough not to smear a rhythmic one.
const XFADE_S: f64 = 0.25;

struct Reader {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
}

impl Reader {
    fn open(path: &Path) -> Result<Self, String> {
        let file =
            File::open(path).map_err(|e| format!("could not open {}: {e}", path.display()))?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());

        // The extension is only a hint; symphonia still probes the content, so
        // a mislabelled file still opens.
        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }

        let format = symphonia::default::get_probe()
            .probe(
                &hint,
                mss,
                FormatOptions::default(),
                MetadataOptions::default(),
            )
            .map_err(|e| format!("unsupported or corrupt audio file: {e}"))?;

        let track = format
            .default_track(TrackType::Audio)
            .ok_or("the file contains no audio track")?;
        let track_id = track.id;
        let params = track
            .codec_params
            .as_ref()
            .and_then(|p| p.audio())
            .ok_or("the audio track has no decodable parameters")?;

        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(params, &AudioDecoderOptions::default())
            .map_err(|e| format!("no decoder for this audio: {e}"))?;

        Ok(Self {
            format,
            decoder,
            track_id,
        })
    }
}

pub struct FileDecoder {
    path: PathBuf,
    target_rate: u32,
    looping: bool,
    reader: Option<Reader>,
    resampler: Resampler,
    source_rate: u32,

    /// Decoded, resampled, stereo, ready to hand out. Only ever holds
    /// finalised output.
    ready: VecDeque<f32>,
    /// Freshly decoded output, before the head/tail bookkeeping. Kept separate
    /// from `ready` so a post-loop skip cannot eat the crossfade that was just
    /// written into it.
    pending: Vec<f32>,
    /// The opening of the file, kept to crossfade into when looping.
    head: Vec<f32>,
    /// Output held back so a crossfade has something to fade out of.
    tail: Vec<f32>,
    /// Output frames to discard after a loop, already played in the crossfade.
    skip: usize,
    /// Crossfade length in frames. Shortened once the file's length is known:
    /// a 0.25 s fade at each end of a 0.5 s loop would consume the whole file
    /// and leave nothing but blended fade blocks.
    xfade: usize,
    /// Output frames produced on the first pass, which is how the length
    /// becomes known -- containers do not reliably report it.
    produced: usize,
    length_known: bool,
    finished: bool,
    scratch: Vec<f32>,
    stereo: Vec<f32>,
}

impl FileDecoder {
    pub fn open(path: impl AsRef<Path>, target_rate: u32, looping: bool) -> Result<Self, String> {
        let path = path.as_ref().to_path_buf();
        let reader = Reader::open(&path)?;
        Ok(Self {
            path,
            target_rate,
            looping,
            reader: Some(reader),
            // Replaced once the first packet reveals the true source rate.
            resampler: Resampler::new(target_rate, target_rate),
            source_rate: 0,
            ready: VecDeque::new(),
            pending: Vec::new(),
            head: Vec::new(),
            tail: Vec::new(),
            skip: 0,
            xfade: (XFADE_S * target_rate as f64) as usize,
            produced: 0,
            length_known: false,
            finished: false,
            scratch: Vec::new(),
            stereo: Vec::new(),
        })
    }

    pub fn finished(&self) -> bool {
        self.finished && self.ready.is_empty()
    }

    pub fn source_rate(&self) -> u32 {
        self.source_rate
    }

    fn xfade_frames(&self) -> usize {
        self.xfade
    }

    /// Decode one packet into `ready`. Returns false at end of stream.
    fn decode_packet(&mut self) -> bool {
        let Some(reader) = self.reader.as_mut() else {
            return false;
        };

        let packet = loop {
            match reader.format.next_packet() {
                Ok(Some(p)) if p.track_id == reader.track_id => break p,
                Ok(Some(_)) => continue,
                Ok(None) => return false,
                Err(_) => return false,
            }
        };

        let decoded = match reader.decoder.decode(&packet) {
            Ok(d) => d,
            // A single corrupt packet should not end playback of the file.
            Err(SymphoniaError::DecodeError(_)) => return true,
            Err(_) => return false,
        };

        let rate = decoded.spec().rate();
        if self.source_rate != rate {
            self.source_rate = rate;
            self.resampler = Resampler::new(rate, self.target_rate);
        }

        to_stereo(&decoded, &mut self.scratch, &mut self.stereo);
        self.resampler.push(&self.stereo);

        let mut out = Vec::new();
        self.resampler.pull(&mut out);
        self.pending.extend(out);
        true
    }

    /// Reopen from the beginning, for looping.
    fn restart(&mut self) {
        match Reader::open(&self.path) {
            Ok(r) => {
                self.reader = Some(r);
                self.resampler = Resampler::new(self.source_rate.max(1), self.target_rate);
                // The opening frames were already heard in the crossfade.
                self.skip = self.head.len() / CH;
            }
            // The file has gone away mid-session; stop rather than spin.
            Err(_) => {
                self.reader = None;
                self.finished = true;
            }
        }
    }

    fn on_end_of_stream(&mut self) {
        // The first pass reveals the length. Cap the fade at a quarter of it
        // so a short loop still spends most of its time playing the file
        // rather than crossfading.
        if !self.length_known {
            self.length_known = true;
            let cap = (self.produced / 4).max(1);
            if cap < self.xfade {
                self.xfade = cap;
                self.head.truncate(cap * CH);
            }
        }

        let xf = self.xfade_frames();
        // Anything held beyond the (possibly shortened) fade is ordinary
        // audio and must still be played.
        if self.tail.len() > xf * CH {
            let release = self.tail.len() - xf * CH;
            self.ready.extend(self.tail.drain(..release));
        }
        if self.looping && self.head.len() / CH >= xf && xf > 0 && self.tail.len() / CH >= xf {
            // Equal-power crossfade from the end of the file into its start,
            // so a looping bed has no seam.
            let start = self.tail.len() - xf * CH;
            for f in 0..xf {
                let t = f as f32 / xf as f32;
                let (fade_out, fade_in) = (
                    (1.0 - t) * std::f32::consts::FRAC_PI_2,
                    t * std::f32::consts::FRAC_PI_2,
                );
                for c in 0..CH {
                    let a = self.tail[start + f * CH + c] * fade_out.sin();
                    let b = self.head[f * CH + c] * fade_in.sin();
                    self.ready.push_back(a + b);
                }
            }
            self.tail.clear();
            self.restart();
        } else {
            // Nothing to fade into: emit what was held back and stop.
            self.ready.extend(self.tail.drain(..));
            self.reader = None;
            self.finished = true;
        }
    }

    /// Move freshly decoded audio into `tail`, releasing all but the
    /// crossfade window into `ready`.
    fn release(&mut self) {
        let xf = self.xfade_frames();

        // Discard the opening of a restarted file, already heard in the
        // crossfade. This applies only to newly decoded audio -- draining it
        // from `ready` would consume the crossfade itself.
        if self.skip > 0 {
            let available = self.pending.len() / CH;
            let drop = self.skip.min(available);
            self.pending.drain(..drop * CH);
            self.skip -= drop;
        }

        if !self.looping || xf == 0 {
            self.ready.extend(self.pending.drain(..));
            return;
        }

        // Capture the opening from the freshly produced audio, in stream
        // order. Reading it out of `tail` instead would be wrong after the
        // first pass: by then `tail` holds the most recent frames, not the
        // file's beginning, and the crossfade would blend unrelated audio.
        let fresh: Vec<f32> = std::mem::take(&mut self.pending);
        if self.head.len() < xf * CH {
            let want = xf * CH - self.head.len();
            let take = want.min(fresh.len());
            self.head.extend_from_slice(&fresh[..take]);
        }
        self.produced += fresh.len() / CH;
        self.tail.extend_from_slice(&fresh);

        // Hold back one crossfade's worth; release the rest.
        let hold = xf * CH;
        if self.tail.len() > hold {
            let release = self.tail.len() - hold;
            self.ready.extend(self.tail.drain(..release));
        }
    }

    /// Fill `out` with interleaved stereo. Returns frames written.
    pub fn read(&mut self, out: &mut [f32]) -> usize {
        let want = out.len() / CH;
        while self.ready.len() / CH < want && !self.finished {
            if self.reader.is_none() {
                break;
            }
            if !self.decode_packet() {
                // Flush whatever the resampler still holds before deciding the
                // stream is over.
                let mut rest = Vec::new();
                self.resampler.finish(&mut rest);
                self.pending.extend(rest);
                self.release();
                self.on_end_of_stream();
                if self.finished {
                    break;
                }
                continue;
            }
            self.release();
        }

        let frames = want.min(self.ready.len() / CH);
        for slot in out.iter_mut().take(frames * CH) {
            *slot = self.ready.pop_front().unwrap_or(0.0);
        }
        // Anything not filled is silence, not stale data.
        for slot in out.iter_mut().skip(frames * CH) {
            *slot = 0.0;
        }
        frames
    }
}

/// Fold any channel layout down to stereo.
fn to_stereo(buf: &GenericAudioBufferRef<'_>, scratch: &mut Vec<f32>, out: &mut Vec<f32>) {
    let frames = buf.frames();
    let total = buf.samples_interleaved();
    if frames == 0 || total == 0 {
        out.clear();
        return;
    }
    scratch.resize(total, 0.0);
    buf.copy_to_slice_interleaved(scratch.as_mut_slice());

    let channels = total / frames;
    out.clear();
    out.reserve(frames * CH);
    match channels {
        0 => {}
        // Mono plays from both ears rather than only the left.
        1 => {
            for &s in scratch.iter().take(frames) {
                out.push(s);
                out.push(s);
            }
        }
        // Surround is folded by taking the front pair, which keeps the
        // dialogue and music where they belong without a downmix matrix.
        _ => {
            for f in 0..frames {
                out.push(scratch[f * channels]);
                out.push(scratch[f * channels + 1]);
            }
        }
    }
}
