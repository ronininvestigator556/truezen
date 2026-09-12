//! Audio handed to the engine from outside it.
//!
//! The engine has no filesystem knowledge and never will: decoding, seeking
//! and resampling all live in the host. What crosses the boundary is this
//! trait, whose only obligation is to fill a buffer without blocking.

/// A source of interleaved stereo frames.
///
/// `read` is called from the audio callback, so an implementation must not
/// block, allocate, or touch the filesystem. A streaming implementation
/// belongs on the far side of a ring buffer.
pub trait SampleSource: Send {
    /// Fill `out` with interleaved stereo frames, returning how many frames
    /// were written. Returning fewer than requested means the source is
    /// starved or finished; the engine treats the remainder as silence.
    fn read(&mut self, out: &mut [f32]) -> usize;

    /// True once the source will never produce audio again, so the engine can
    /// stop asking.
    fn finished(&self) -> bool {
        false
    }
}

/// Frames pulled from a source in one go.
///
/// Reading per sample through a trait object would pay dynamic-dispatch cost
/// on every frame; a block amortises it. 128 frames is 2.7 ms at 48 kHz, well
/// inside any device buffer.
pub const SOURCE_BLOCK: usize = 128;

/// Buffers a [`SampleSource`] so the per-sample mixing loop can pull one frame
/// at a time. The buffer is a fixed array, so nothing here allocates.
pub struct Voice {
    source: Box<dyn SampleSource>,
    buf: [f32; SOURCE_BLOCK * 2],
    filled: usize,
    pos: usize,
    starved: bool,
}

impl Voice {
    pub fn new(source: Box<dyn SampleSource>) -> Self {
        Self {
            source,
            buf: [0.0; SOURCE_BLOCK * 2],
            filled: 0,
            pos: 0,
            starved: false,
        }
    }

    /// Recover the underlying source, so it can be dropped off the audio
    /// thread like any other displaced heap data.
    pub fn into_source(self) -> Box<dyn SampleSource> {
        self.source
    }

    /// True when the last refill came up short — the decoder is not keeping up,
    /// or the file has ended.
    pub fn starved(&self) -> bool {
        self.starved
    }

    pub fn finished(&self) -> bool {
        self.source.finished() && self.pos >= self.filled
    }

    #[inline]
    pub fn next_frame(&mut self) -> (f64, f64) {
        if self.pos >= self.filled {
            self.filled = self.source.read(&mut self.buf).min(SOURCE_BLOCK);
            self.starved = self.filled < SOURCE_BLOCK;
            self.pos = 0;
            if self.filled == 0 {
                // Silence rather than a glitch: a starved source should fade
                // into nothing, not repeat its last block.
                return (0.0, 0.0);
            }
        }
        let i = self.pos * 2;
        self.pos += 1;
        (self.buf[i] as f64, self.buf[i + 1] as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Counts up so the test can tell which frame it received.
    struct Ramp {
        n: usize,
        limit: usize,
    }

    impl SampleSource for Ramp {
        fn read(&mut self, out: &mut [f32]) -> usize {
            let frames = out.len() / 2;
            let mut written = 0;
            for f in 0..frames {
                if self.n >= self.limit {
                    break;
                }
                out[f * 2] = self.n as f32;
                out[f * 2 + 1] = -(self.n as f32);
                self.n += 1;
                written += 1;
            }
            written
        }
        fn finished(&self) -> bool {
            self.n >= self.limit
        }
    }

    #[test]
    fn frames_come_out_in_order_across_block_boundaries() {
        let mut v = Voice::new(Box::new(Ramp { n: 0, limit: 1000 }));
        for expected in 0..1000 {
            let (l, r) = v.next_frame();
            assert_eq!(l, expected as f64, "frame {expected} out of order");
            assert_eq!(r, -(expected as f64));
        }
    }

    #[test]
    fn an_exhausted_source_yields_silence_not_a_repeat() {
        let mut v = Voice::new(Box::new(Ramp { n: 0, limit: 10 }));
        for _ in 0..10 {
            v.next_frame();
        }
        for _ in 0..500 {
            assert_eq!(v.next_frame(), (0.0, 0.0), "source repeated after ending");
        }
        assert!(v.finished());
    }

    /// A source that always returns nothing must not spin or panic.
    #[test]
    fn a_silent_source_is_harmless() {
        struct Dead;
        impl SampleSource for Dead {
            fn read(&mut self, _: &mut [f32]) -> usize {
                0
            }
        }
        let mut v = Voice::new(Box::new(Dead));
        for _ in 0..1000 {
            assert_eq!(v.next_frame(), (0.0, 0.0));
        }
        assert!(v.starved());
    }
}
