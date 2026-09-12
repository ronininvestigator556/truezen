//! Adapting a [`FileDecoder`] into something the audio callback can read.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use truezen_engine::source::SampleSource;

use crate::decoder::FileDecoder;

/// How much audio to keep queued ahead of the callback. Generous, because a
/// disk hiccup or an expensive decode must not become a dropout.
const PREBUFFER_S: f64 = 4.0;
/// Frames the decoder thread produces per iteration.
const DECODE_CHUNK: usize = 2048;

/// Drives a decoder directly. Correct only where blocking is acceptable —
/// the offline renderer — never in the audio callback.
pub struct DirectSource {
    decoder: FileDecoder,
}

impl DirectSource {
    pub fn open(path: impl AsRef<Path>, rate: u32, looping: bool) -> Result<Self, String> {
        Ok(Self {
            decoder: FileDecoder::open(path, rate, looping)?,
        })
    }
}

impl SampleSource for DirectSource {
    fn read(&mut self, out: &mut [f32]) -> usize {
        self.decoder.read(out)
    }
    fn finished(&self) -> bool {
        self.decoder.finished()
    }
}

/// A decoder running on its own thread, feeding the callback through a
/// lock-free ring.
pub struct StreamedSource {
    consumer: rtrb::Consumer<f32>,
    stop: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
    /// Times the callback asked for audio the decoder had not produced.
    underruns: Arc<AtomicU64>,
    thread: Option<JoinHandle<()>>,
}

impl StreamedSource {
    pub fn open(path: impl AsRef<Path>, rate: u32, looping: bool) -> Result<Self, String> {
        // Open on this thread so a bad path or unsupported codec is reported
        // to the caller rather than failing silently in the background.
        let mut decoder = FileDecoder::open(path, rate, looping)?;

        let capacity = (PREBUFFER_S * rate as f64) as usize * 2;
        let (mut producer, consumer) = rtrb::RingBuffer::<f32>::new(capacity);
        let stop = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicBool::new(false));

        let thread = {
            let stop = Arc::clone(&stop);
            let done = Arc::clone(&done);
            std::thread::Builder::new()
                .name("truezen-decode".into())
                .spawn(move || {
                    let mut block = vec![0.0f32; DECODE_CHUNK * 2];
                    while !stop.load(Ordering::Relaxed) {
                        if producer.slots() < block.len() {
                            // The ring is full, which is the normal steady
                            // state. Sleeping beats spinning a whole core.
                            std::thread::sleep(Duration::from_millis(5));
                            continue;
                        }
                        let frames = decoder.read(&mut block);
                        if frames == 0 {
                            done.store(true, Ordering::Release);
                            break;
                        }
                        for &s in &block[..frames * 2] {
                            if producer.push(s).is_err() {
                                break;
                            }
                        }
                    }
                    done.store(true, Ordering::Release);
                })
                .map_err(|e| format!("could not start the decoder thread: {e}"))?
        };

        Ok(Self {
            consumer,
            stop,
            done,
            underruns: Arc::new(AtomicU64::new(0)),
            thread: Some(thread),
        })
    }

    pub fn underruns(&self) -> u64 {
        self.underruns.load(Ordering::Relaxed)
    }
}

impl SampleSource for StreamedSource {
    fn read(&mut self, out: &mut [f32]) -> usize {
        let mut written = 0;
        for slot in out.iter_mut() {
            match self.consumer.pop() {
                Ok(v) => {
                    *slot = v;
                    written += 1;
                }
                Err(_) => {
                    *slot = 0.0;
                }
            }
        }
        if written < out.len() && !self.done.load(Ordering::Acquire) {
            // Short only because the decoder fell behind, not because the file
            // ended: worth counting, since it means an audible gap.
            self.underruns.fetch_add(1, Ordering::Relaxed);
        }
        written / 2
    }

    fn finished(&self) -> bool {
        self.done.load(Ordering::Acquire) && self.consumer.is_empty()
    }
}

impl Drop for StreamedSource {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
