//! Audio-thread telemetry: visualiser data and callback load.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Raw mono samples handed to the UI. Contiguous and in chronological order,
/// so the UI can both draw them and run its own FFT over them.
pub const WAVE_POINTS: usize = 1024;
/// Peak-envelope points. One per [`ENV_DECIMATION`] samples, giving a window
/// long enough to see a slow beat pulse rather than just the carrier.
pub const ENV_POINTS: usize = 256;
pub const ENV_DECIMATION: usize = 64;

/// A visualiser frame. `Copy` and fixed-size so it moves through a triple
/// buffer with no allocation.
#[derive(Clone, Copy)]
pub struct Scope {
    pub wave: [f32; WAVE_POINTS],
    pub env: [f32; ENV_POINTS],
}

impl Default for Scope {
    fn default() -> Self {
        Self {
            wave: [0.0; WAVE_POINTS],
            env: [0.0; ENV_POINTS],
        }
    }
}

/// Accumulates scope data on the audio thread. Both buffers are rings;
/// `snapshot` rotates them into chronological order so the reader never has
/// to know about the write cursor.
pub struct ScopeWriter {
    wave: [f32; WAVE_POINTS],
    wave_pos: usize,
    env: [f32; ENV_POINTS],
    env_pos: usize,
    env_peak: f32,
    env_count: usize,
}

impl Default for ScopeWriter {
    fn default() -> Self {
        Self {
            wave: [0.0; WAVE_POINTS],
            wave_pos: 0,
            env: [0.0; ENV_POINTS],
            env_pos: 0,
            env_peak: 0.0,
            env_count: 0,
        }
    }
}

impl ScopeWriter {
    #[inline]
    pub fn push(&mut self, mono: f32) {
        self.wave[self.wave_pos] = mono;
        self.wave_pos = (self.wave_pos + 1) % WAVE_POINTS;

        // Peak rather than mean: an envelope built from means understates the
        // depth of an isochronic gate.
        self.env_peak = self.env_peak.max(mono.abs());
        self.env_count += 1;
        if self.env_count >= ENV_DECIMATION {
            self.env[self.env_pos] = self.env_peak;
            self.env_pos = (self.env_pos + 1) % ENV_POINTS;
            self.env_peak = 0.0;
            self.env_count = 0;
        }
    }

    pub fn snapshot(&self) -> Scope {
        let mut s = Scope::default();
        for i in 0..WAVE_POINTS {
            s.wave[i] = self.wave[(self.wave_pos + i) % WAVE_POINTS];
        }
        for i in 0..ENV_POINTS {
            s.env[i] = self.env[(self.env_pos + i) % ENV_POINTS];
        }
        s
    }
}

/// Callback health counters.
///
/// `cpal` does not report underruns directly, so rather than claim to detect
/// them we measure what we can see: how long each callback took against the
/// time it had. A sustained load near 100% is what precedes an audible
/// dropout.
#[derive(Default)]
pub struct Telemetry {
    callbacks: AtomicU64,
    /// Callbacks that used more than 80% of their deadline.
    overloads: AtomicU64,
    /// Stream errors reported by the backend.
    errors: AtomicU64,
    /// Commands dropped because the queue was full.
    dropped_commands: AtomicU64,
    last_load: AtomicU32,
    max_load: AtomicU32,
}

#[derive(Copy, Clone, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub callbacks: u64,
    pub overloads: u64,
    pub errors: u64,
    pub dropped_commands: u64,
    /// Fraction of the available time the last callback used, in per mille.
    pub last_load_permille: u32,
    pub max_load_permille: u32,
}

impl Telemetry {
    /// Called at the end of every audio callback.
    ///
    /// `Instant::now` is a vDSO read on every platform we target -- no
    /// syscall, no lock, no allocation -- which makes it safe here and is
    /// standard practice for audio load metering.
    pub fn record(&self, elapsed_s: f64, frames: usize, sample_rate: f64) {
        let budget = frames as f64 / sample_rate;
        let load = if budget > 0.0 {
            elapsed_s / budget
        } else {
            0.0
        };
        let permille = (load * 1000.0).clamp(0.0, 100_000.0) as u32;

        self.callbacks.fetch_add(1, Ordering::Relaxed);
        if load > 0.8 {
            self.overloads.fetch_add(1, Ordering::Relaxed);
        }
        self.last_load.store(permille, Ordering::Relaxed);
        self.max_load.fetch_max(permille, Ordering::Relaxed);
    }

    pub fn note_error(&self) {
        self.errors.fetch_add(1, Ordering::Relaxed);
    }

    pub fn note_dropped_command(&self) {
        self.dropped_commands.fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> Stats {
        Stats {
            callbacks: self.callbacks.load(Ordering::Relaxed),
            overloads: self.overloads.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
            dropped_commands: self.dropped_commands.load(Ordering::Relaxed),
            last_load_permille: self.last_load.load(Ordering::Relaxed),
            max_load_permille: self.max_load.load(Ordering::Relaxed),
        }
    }

    /// Reset the peak between soak runs.
    pub fn reset_max_load(&self) {
        self.max_load.store(0, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_snapshot_is_in_chronological_order() {
        let mut w = ScopeWriter::default();
        // Write one and a half rings, so the cursor has wrapped.
        for i in 0..(WAVE_POINTS + WAVE_POINTS / 2) {
            w.push(i as f32);
        }
        let s = w.snapshot();
        // The oldest retained sample sits at index 0 and values ascend.
        for i in 1..WAVE_POINTS {
            assert!(
                s.wave[i] > s.wave[i - 1],
                "wave not chronological at {i}: {} then {}",
                s.wave[i - 1],
                s.wave[i]
            );
        }
        assert_eq!(
            *s.wave.last().unwrap(),
            (WAVE_POINTS + WAVE_POINTS / 2 - 1) as f32
        );
    }

    #[test]
    fn envelope_tracks_peaks_not_means() {
        let mut w = ScopeWriter::default();
        // A single loud sample in an otherwise silent block must survive.
        for i in 0..ENV_DECIMATION {
            w.push(if i == 3 { 0.9 } else { 0.0 });
        }
        let s = w.snapshot();
        assert!(
            s.env.iter().any(|v| (*v - 0.9).abs() < 1e-6),
            "peak was averaged away"
        );
    }

    #[test]
    fn load_is_measured_against_the_deadline() {
        let t = Telemetry::default();
        // 1024 frames at 48 kHz is a 21.3 ms budget; 10.6 ms is half of it.
        t.record(0.01066, 1024, 48_000.0);
        let s = t.snapshot();
        assert!(
            (s.last_load_permille as i64 - 500).abs() < 20,
            "got {}",
            s.last_load_permille
        );
        assert_eq!(s.overloads, 0);

        t.record(0.020, 1024, 48_000.0);
        assert_eq!(t.snapshot().overloads, 1);
    }
}
