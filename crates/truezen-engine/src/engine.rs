//! The engine: layer stack, transport, automation, master chain.
//!
//! This type knows nothing about audio devices, files, or the UI. Its entire
//! output interface is [`Engine::process`], which is what lets the same code
//! drive a real-time callback, an offline render, and the test suite.
//!
//! **Real-time contract.** `process` and `apply` must never allocate, lock,
//! log, or make a syscall. Anything needing the heap is built by the caller
//! and handed over in a `Box`; the displaced `Box` is handed back for the
//! caller to drop off-thread.

use crate::layer::{Layer, LayerConfig};
use crate::mixer::Limiter;
use crate::preset::Preset;
use crate::smooth::Smoother;
use crate::timeline::{LayerParam, ParamTarget, Timeline};

/// Automation is re-evaluated this often. At 48 kHz that is every 1.3 ms --
/// far finer than any perceptible change, and it keeps the per-sample loop
/// free of breakpoint searching.
pub const AUTOMATION_BLOCK: usize = 64;

const TAU_MASTER: f64 = 0.03;
/// Transport fades. Long enough to be inaudible as a click, short enough that
/// pause feels immediate.
const TAU_TRANSPORT: f64 = 0.015;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum Transport {
    #[default]
    Stopped,
    Playing,
    Paused,
}

/// The swappable half of the engine. Built off the audio thread and moved in
/// via [`Command::Replace`].
pub struct SessionState {
    pub layers: Vec<Layer>,
    pub timeline: Option<Timeline>,
    pub master_gain: f64,
    /// One flag per timeline track. A latched track stops being evaluated, so
    /// the value the user dialed in survives instead of being overwritten on
    /// the next automation block.
    ///
    /// Sized once when the session is built, so latching never allocates.
    pub track_latched: Vec<bool>,
}

impl SessionState {
    pub fn empty() -> Self {
        Self {
            layers: Vec::new(),
            timeline: None,
            master_gain: 0.7,
            track_latched: Vec::new(),
        }
    }

    /// Build from a preset. Allocates, so call this on a worker thread.
    pub fn from_preset(preset: &Preset, sample_rate: f64) -> Self {
        let layers = preset
            .layers
            .iter()
            .enumerate()
            .map(|(i, cfg)| Layer::new(cfg.clone(), sample_rate, 0x51ED_0000 ^ i as u64))
            .collect();
        let track_count = preset.timeline.as_ref().map_or(0, |t| t.tracks.len());
        Self {
            layers,
            timeline: preset.timeline.clone(),
            master_gain: preset.master_gain,
            track_latched: vec![false; track_count],
        }
    }
}

/// A snapshot for the UI. `Copy` and scalar-only, so it can go through a
/// triple buffer without allocation.
#[derive(Copy, Clone, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Meters {
    pub peak_l: f32,
    pub peak_r: f32,
    pub rms_l: f32,
    pub rms_r: f32,
    pub position_s: f64,
    pub duration_s: f64,
    pub playing: bool,
    pub finished: bool,
    /// Live beat and carrier of the first audible beat layer, post-smoothing,
    /// so the UI shows what is actually sounding rather than the target.
    pub beat_hz: f32,
    pub carrier_hz: f32,
    /// Bit `n` set means timeline track `n` is latched and no longer driving
    /// its parameter. Tracks beyond 32 are not represented; presets do not
    /// come close to that many.
    pub latched_tracks: u32,
}

/// Real-time-safe control messages.
pub enum Command {
    Play,
    Pause,
    Stop,
    Seek { seconds: f64 },
    SetMasterGain(f64),
    SetLayerEnabled { index: usize, on: bool },
    SetLayerParam { index: usize, param: LayerParam, value: f64 },
    /// Hand a parameter back to its automation track.
    SetTrackLatched { track: usize, latched: bool },
    /// Restore every automated parameter to timeline control.
    UnlatchAll,
    /// Swap in a whole new layer stack. The displaced state is returned for
    /// the caller to drop on its own thread.
    Replace(Box<SessionState>),
}

pub struct Engine {
    sample_rate: f64,
    /// Boxed so `Replace` is a pointer swap rather than an allocation.
    state: Box<SessionState>,
    master: Smoother,
    transport_gain: Smoother,
    limiter: Limiter,
    transport: Transport,
    /// Session position in samples. Integer, so it cannot accumulate drift.
    pos: u64,
    fade: f64,
    finished: bool,
    meters: Meters,
}

impl Engine {
    pub fn new(sample_rate: f64) -> Self {
        let state = Box::new(SessionState::empty());
        let mut master = Smoother::new(state.master_gain, TAU_MASTER, sample_rate);
        master.set_epsilon(1e-6);
        Self {
            sample_rate,
            state,
            master,
            // Start at zero so the very first block ramps up: no startup thump.
            transport_gain: Smoother::new(0.0, TAU_TRANSPORT, sample_rate),
            limiter: Limiter::default(),
            transport: Transport::Stopped,
            pos: 0,
            fade: 1.0,
            finished: false,
            meters: Meters::default(),
        }
    }

    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    pub fn transport(&self) -> Transport {
        self.transport
    }

    pub fn meters(&self) -> Meters {
        self.meters
    }

    pub fn position_s(&self) -> f64 {
        self.pos as f64 / self.sample_rate
    }

    pub fn duration_s(&self) -> f64 {
        self.state.timeline.as_ref().map_or(0.0, Timeline::duration)
    }

    pub fn state(&self) -> &SessionState {
        &self.state
    }

    /// Convenience for tests and offline rendering, where allocating is fine.
    pub fn load_preset(&mut self, preset: &Preset) {
        let state = Box::new(SessionState::from_preset(preset, self.sample_rate));
        self.apply(Command::Replace(state));
    }

    /// Returns any displaced heap data for the caller to drop off-thread.
    pub fn apply(&mut self, cmd: Command) -> Option<Box<SessionState>> {
        match cmd {
            Command::Play => {
                self.finished = false;
                self.transport = Transport::Playing;
                self.transport_gain.set_target(1.0);
            }
            Command::Pause => {
                self.transport = Transport::Paused;
                self.transport_gain.set_target(0.0);
            }
            Command::Stop => {
                self.transport = Transport::Stopped;
                self.transport_gain.set_target(0.0);
                self.pos = 0;
                self.finished = false;
            }
            Command::Seek { seconds } => {
                self.pos = (seconds.max(0.0) * self.sample_rate) as u64;
                self.finished = false;
                // Jump the smoothers rather than gliding: after a seek the
                // previous values have no relationship to the new position.
                self.snap_automation();
            }
            Command::SetMasterGain(g) => {
                let g = g.clamp(0.0, 1.0);
                self.state.master_gain = g;
                self.master.set_target(g);
                self.latch_matching(ParamTarget::MasterGain);
            }
            Command::SetLayerEnabled { index, on } => {
                if let Some(l) = self.state.layers.get_mut(index) {
                    l.set_enabled(on);
                }
            }
            Command::SetLayerParam { index, param, value } => {
                if let Some(l) = self.state.layers.get_mut(index) {
                    apply_layer_param(l, param, value);
                }
                // Touch latches. A hand on the control beats the timeline
                // until the user explicitly gives the parameter back --
                // otherwise the next automation block, 1.3 ms later, silently
                // undoes the edit.
                self.latch_matching(ParamTarget::Layer { index, param });
            }
            Command::SetTrackLatched { track, latched } => {
                if let Some(f) = self.state.track_latched.get_mut(track) {
                    *f = latched;
                }
                // Releasing a latch should take effect at once rather than at
                // the end of the current automation block.
                if !latched {
                    self.tick_automation();
                }
            }
            Command::UnlatchAll => {
                self.state.track_latched.iter_mut().for_each(|f| *f = false);
                self.tick_automation();
            }
            Command::Replace(new_state) => {
                self.master.set_target(new_state.master_gain);
                let old = std::mem::replace(&mut self.state, new_state);
                self.pos = 0;
                self.finished = false;
                self.fade = 1.0;
                self.snap_automation();
                return Some(old);
            }
        }
        None
    }

    /// Push every automated parameter to its value at the current position,
    /// without gliding.
    fn snap_automation(&mut self) {
        let t = self.position_s();
        let SessionState { layers, timeline, track_latched, .. } = &mut *self.state;
        let Some(tl) = timeline.as_ref() else { return };
        for (i, track) in tl.tracks.iter().enumerate() {
            if track_latched.get(i).copied().unwrap_or(false) {
                continue;
            }
            let Some(v) = track.value_at(t) else { continue };
            match track.target {
                ParamTarget::MasterGain => {
                    self.master.reset_to(v.clamp(0.0, 1.0));
                }
                ParamTarget::Layer { index, param } => {
                    if let Some(l) = layers.get_mut(index) {
                        apply_layer_param(l, param, v);
                        l.snap_parameters();
                    }
                }
            }
        }
        self.fade = tl.fade_gain_at(t);
    }

    /// Latch every track driving `target`, so the user's value sticks.
    fn latch_matching(&mut self, target: ParamTarget) {
        let SessionState { timeline, track_latched, .. } = &mut *self.state;
        let Some(tl) = timeline.as_ref() else { return };
        for (i, track) in tl.tracks.iter().enumerate() {
            if track.target == target {
                if let Some(f) = track_latched.get_mut(i) {
                    *f = true;
                }
            }
        }
    }

    fn latched_bitmask(&self) -> u32 {
        self.state
            .track_latched
            .iter()
            .take(32)
            .enumerate()
            .fold(0u32, |acc, (i, on)| if *on { acc | (1 << i) } else { acc })
    }

    /// Re-evaluate automation for the block starting at the current position.
    fn tick_automation(&mut self) {
        let t = self.pos as f64 / self.sample_rate;
        // Split the borrow so the timeline can be read while layers are
        // written.
        let SessionState { layers, timeline, track_latched, .. } = &mut *self.state;
        let Some(tl) = timeline.as_ref() else {
            self.fade = 1.0;
            return;
        };
        for (i, track) in tl.tracks.iter().enumerate() {
            if track_latched.get(i).copied().unwrap_or(false) {
                continue;
            }
            let Some(v) = track.value_at(t) else { continue };
            match track.target {
                ParamTarget::MasterGain => self.master.set_target(v.clamp(0.0, 1.0)),
                ParamTarget::Layer { index, param } => {
                    if let Some(l) = layers.get_mut(index) {
                        apply_layer_param(l, param, v);
                    }
                }
            }
        }
        self.fade = tl.fade_gain_at(t);

        if tl.is_finished(t) && !self.finished {
            self.finished = true;
            self.transport = Transport::Stopped;
            self.transport_gain.set_target(0.0);
        }
    }

    /// Render interleaved stereo f32 into `out`.
    ///
    /// `out.len()` must be even; any trailing odd sample is left untouched.
    pub fn process(&mut self, out: &mut [f32]) {
        let frames = out.len() / 2;
        let mut peak_l = 0.0f64;
        let mut peak_r = 0.0f64;
        let mut sum_l = 0.0f64;
        let mut sum_r = 0.0f64;

        let mut done = 0usize;
        while done < frames {
            let n = (frames - done).min(AUTOMATION_BLOCK);

            if self.transport == Transport::Playing {
                self.tick_automation();
            }

            for f in 0..n {
                let idx = (done + f) * 2;

                let tg = self.transport_gain.tick();
                let master = self.master.tick();

                // Fully faded out: skip synthesis entirely. A stopped session
                // should not burn CPU, which matters for a tray-resident app.
                if tg <= 0.0 && self.transport != Transport::Playing {
                    out[idx] = 0.0;
                    out[idx + 1] = 0.0;
                    continue;
                }

                let mut l = 0.0f64;
                let mut r = 0.0f64;
                for layer in self.state.layers.iter_mut() {
                    let (ll, rr) = layer.tick();
                    l += ll;
                    r += rr;
                }

                let g = master * tg * self.fade;
                let l = self.limiter.process(l * g);
                let r = self.limiter.process(r * g);

                peak_l = peak_l.max(l.abs());
                peak_r = peak_r.max(r.abs());
                sum_l += l * l;
                sum_r += r * r;

                out[idx] = l as f32;
                out[idx + 1] = r as f32;
            }

            if self.transport == Transport::Playing {
                self.pos += n as u64;
            }
            done += n;
        }

        self.update_meters(frames, peak_l, peak_r, sum_l, sum_r);
    }

    fn update_meters(&mut self, frames: usize, pl: f64, pr: f64, sl: f64, sr: f64) {
        let n = frames.max(1) as f64;
        let (beat, carrier) = self
            .state
            .layers
            .iter()
            .find(|l| l.config.enabled && l.config.kind != crate::layer::LayerKind::Noise)
            .map(|l| (l.current_beat(), l.current_carrier()))
            .unwrap_or((0.0, 0.0));

        self.meters = Meters {
            peak_l: pl as f32,
            peak_r: pr as f32,
            rms_l: (sl / n).sqrt() as f32,
            rms_r: (sr / n).sqrt() as f32,
            position_s: self.position_s(),
            duration_s: self.duration_s(),
            playing: self.transport == Transport::Playing,
            finished: self.finished,
            beat_hz: beat as f32,
            carrier_hz: carrier as f32,
            latched_tracks: self.latched_bitmask(),
        };
    }
}

fn apply_layer_param(layer: &mut Layer, param: LayerParam, value: f64) {
    match param {
        LayerParam::Carrier => layer.set_carrier(value),
        LayerParam::Beat => layer.set_beat(value),
        LayerParam::Gain => layer.set_gain(value.clamp(0.0, 1.0)),
        LayerParam::Pan => layer.set_pan(value.clamp(-1.0, 1.0)),
        LayerParam::FilterCutoff => {
            let q = layer.config.filter_q;
            layer.set_filter(value, q);
        }
        LayerParam::Duty => layer.set_duty(value),
        LayerParam::RampMs => layer.set_ramp_ms(value),
        LayerParam::Depth => layer.set_depth(value),
    }
}

/// Helper for building a layer stack off-thread.
pub fn build_layers(configs: &[LayerConfig], sample_rate: f64) -> Vec<Layer> {
    configs
        .iter()
        .enumerate()
        .map(|(i, c)| Layer::new(c.clone(), sample_rate, 0x51ED_0000 ^ i as u64))
        .collect()
}
