//! Real-time audio host.
//!
//! # Threading
//!
//! Three domains, with strict boundaries:
//!
//! 1. **Audio callback** -- owns the [`Engine`]. Never allocates, locks, logs
//!    or makes a syscall. Reads commands from an SPSC ring and writes meter
//!    and scope snapshots into triple buffers.
//! 2. **Audio thread** (`run`) -- owns the `cpal::Stream`, which is `!Send` on
//!    most backends and so can never leave the thread that built it. Does all
//!    heap work: building layer stacks, dropping displaced ones, rebuilding
//!    streams.
//! 3. **Caller threads** -- the UI. Talk to the audio thread over an ordinary
//!    channel and read telemetry through mutexes that the audio callback never
//!    touches.
//!
//! Nothing shared with the callback is ever locked. Live frequency dialing is
//! exactly the operation that glitches if it is.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{Device, SampleFormat, StreamConfig};

use truezen_engine::engine::{Command, Engine, Meters, Recycled, SessionState, TimelineSwap};
use truezen_engine::preset::Preset;
use truezen_engine::layer::LayerKind;
use truezen_engine::timeline::Timeline;
use truezen_engine::timeline::LayerParam;
use truezen_engine::Transport;

pub mod decoder;
pub mod device;
pub mod export;
pub mod filesource;
pub mod resample;
pub mod telemetry;

pub use device::{DeviceInfo, PREFERRED_FRAMES};
pub use telemetry::{Scope, Stats, ENV_POINTS, WAVE_POINTS};

use crate::filesource::StreamedSource;
use telemetry::{ScopeWriter, Telemetry};

/// Commands buffered between the UI and the audio callback. Generous, because
/// a fast knob drag can produce a burst and dropping those would be visible.
const COMMAND_CAPACITY: usize = 1024;
/// Displaced layer stacks and timelines awaiting disposal off-thread.
const RECYCLE_CAPACITY: usize = 64;
/// Largest block the callback will render in one go. Preallocated, so a
/// backend that hands us an unexpectedly large buffer is chunked rather than
/// triggering an allocation in the callback.
const SCRATCH_FRAMES: usize = 8192;
/// Requests collapsed together in one pass of the audio thread's loop.
const MAX_BATCH: usize = 512;

#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error("no output device available")]
    NoOutputDevice,
    #[error("audio device error: {0}")]
    Cpal(String),
    #[error("no usable output configuration: {0}")]
    Config(String),
    #[error("unsupported sample format {0:?}")]
    UnsupportedFormat(SampleFormat),
    #[error("could not build audio stream: {0}")]
    BuildStream(String),
    #[error("could not start audio stream: {0}")]
    Play(String),
    #[error("audio thread is not running")]
    Dead,
}

impl From<cpal::Error> for HostError {
    fn from(e: cpal::Error) -> Self {
        match e.kind() {
            cpal::ErrorKind::DeviceNotAvailable => HostError::NoOutputDevice,
            _ => HostError::Cpal(e.to_string()),
        }
    }
}

/// What the host is currently doing. Surfaced in the UI so a silent app is
/// never a mystery.
#[derive(Clone, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostStatus {
    pub device_name: Option<String>,
    pub sample_rate: u32,
    pub channels: u16,
    pub running: bool,
    /// Set when the device is mono, where binaural layers cannot work.
    pub binaural_capable: bool,
    pub last_error: Option<String>,
}

/// Messages from the UI to the audio thread.
enum Request {
    Engine(Command),
    LoadPreset(Box<Preset>),
    UpdateTimeline(Box<Option<Timeline>>),
    AttachFile { index: usize, path: String, looping: bool },
    DetachFile { index: usize },
    SelectDevice(Option<String>),
    /// Raised by the stream error callback.
    DeviceLost(String),
    Shutdown,
}

struct Shared {
    meters: Mutex<Option<triple_buffer::Output<Meters>>>,
    scope: Mutex<Option<triple_buffer::Output<Scope>>>,
    status: Mutex<HostStatus>,
    telemetry: Telemetry,
}

impl Shared {
    fn meters(&self) -> Meters {
        self.meters
            .lock()
            .ok()
            .and_then(|mut g| g.as_mut().map(|o| *o.read()))
            .unwrap_or_default()
    }

    fn scope(&self) -> Scope {
        self.scope
            .lock()
            .ok()
            .and_then(|mut g| g.as_mut().map(|o| *o.read()))
            .unwrap_or_default()
    }
}

/// Handle to the running audio thread. Cloneable-by-reference via `Arc` in the
/// application; dropping it shuts the thread down.
pub struct AudioHost {
    tx: Sender<Request>,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl AudioHost {
    /// Start the audio thread and open `device` (or the system default).
    ///
    /// Returns once the first stream attempt has completed, so
    /// [`AudioHost::status`] is meaningful immediately.
    pub fn spawn(device: Option<String>) -> Result<Self, HostError> {
        let (tx, rx) = mpsc::channel();
        let shared = Arc::new(Shared {
            meters: Mutex::new(None),
            scope: Mutex::new(None),
            status: Mutex::new(HostStatus::default()),
            telemetry: Telemetry::default(),
        });

        let (ready_tx, ready_rx) = mpsc::channel();
        let thread = {
            let shared = Arc::clone(&shared);
            let tx = tx.clone();
            std::thread::Builder::new()
                .name("truezen-audio".into())
                .spawn(move || run(rx, tx, shared, device, ready_tx))
                .map_err(|e| HostError::BuildStream(e.to_string()))?
        };

        // Surface a first-open failure to the caller rather than leaving a
        // silent app with no explanation.
        match ready_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e),
            Err(_) => return Err(HostError::BuildStream("audio thread did not start".into())),
        }

        Ok(Self {
            tx,
            shared,
            thread: Some(thread),
        })
    }

    pub fn devices() -> Result<Vec<DeviceInfo>, HostError> {
        device::output_devices()
    }

    fn send(&self, r: Request) -> Result<(), HostError> {
        self.tx.send(r).map_err(|_| HostError::Dead)
    }

    fn command(&self, c: Command) -> Result<(), HostError> {
        self.send(Request::Engine(c))
    }

    /// Replace the session. The layer stack is built on the audio thread, not
    /// in the callback.
    pub fn load_preset(&self, preset: Preset) -> Result<(), HostError> {
        self.send(Request::LoadPreset(Box::new(preset)))
    }

    pub fn play(&self) -> Result<(), HostError> {
        self.command(Command::Play)
    }

    pub fn pause(&self) -> Result<(), HostError> {
        self.command(Command::Pause)
    }

    pub fn stop(&self) -> Result<(), HostError> {
        self.command(Command::Stop)
    }

    pub fn seek(&self, seconds: f64) -> Result<(), HostError> {
        self.command(Command::Seek { seconds })
    }

    pub fn set_master_gain(&self, gain: f64) -> Result<(), HostError> {
        self.command(Command::SetMasterGain(gain))
    }

    pub fn set_layer_param(&self, index: usize, param: LayerParam, value: f64) -> Result<(), HostError> {
        self.command(Command::SetLayerParam { index, param, value })
    }

    pub fn set_layer_enabled(&self, index: usize, on: bool) -> Result<(), HostError> {
        self.command(Command::SetLayerEnabled { index, on })
    }

    /// Replace the session's automation, leaving the layers and the clock
    /// alone. Pass `None` to remove automation entirely.
    pub fn update_timeline(&self, timeline: Option<Timeline>) -> Result<(), HostError> {
        self.send(Request::UpdateTimeline(Box::new(timeline)))
    }

    /// Give a layer audio from a file. Opening happens on the audio thread,
    /// not in the callback, and the result is reported through
    /// [`AudioHost::status`] if it fails.
    pub fn attach_file(&self, index: usize, path: String, looping: bool) -> Result<(), HostError> {
        self.send(Request::AttachFile { index, path, looping })
    }

    pub fn detach_file(&self, index: usize) -> Result<(), HostError> {
        self.send(Request::DetachFile { index })
    }

    pub fn set_layer_duck(&self, index: usize, on: bool, depth: f64) -> Result<(), HostError> {
        self.command(Command::SetLayerDuck { index, on, depth })
    }

    /// Hand an automated parameter back to its timeline track.
    ///
    /// Dialing a parameter latches it automatically; this is the way back.
    pub fn set_track_latched(&self, track: usize, latched: bool) -> Result<(), HostError> {
        self.command(Command::SetTrackLatched { track, latched })
    }

    /// Return every latched parameter to timeline control.
    pub fn unlatch_all(&self) -> Result<(), HostError> {
        self.command(Command::UnlatchAll)
    }

    /// Switch output device, resuming at the current session position.
    pub fn select_device(&self, name: Option<String>) -> Result<(), HostError> {
        self.send(Request::SelectDevice(name))
    }

    pub fn meters(&self) -> Meters {
        self.shared.meters()
    }

    pub fn scope(&self) -> Scope {
        self.shared.scope()
    }

    pub fn stats(&self) -> Stats {
        self.shared.telemetry.snapshot()
    }

    pub fn reset_stats(&self) {
        self.shared.telemetry.reset_max_load();
    }

    pub fn status(&self) -> HostStatus {
        self.shared.status.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

impl Drop for AudioHost {
    fn drop(&mut self) {
        let _ = self.tx.send(Request::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

// ---------------------------------------------------------------------------
// Audio thread
// ---------------------------------------------------------------------------

/// A live stream and its channels to the callback.
struct Active {
    /// Dropping this stops the stream; it must outlive the channels below.
    stream: cpal::Stream,
    cmd_tx: rtrb::Producer<Command>,
    recycle_rx: rtrb::Consumer<Recycled>,
    sample_rate: f64,
}

/// Everything the audio thread must remember in order to rebuild a stream and
/// carry on where it left off.
#[derive(Default)]
struct Desired {
    preset: Option<Preset>,
    transport: Transport,
    position_s: f64,
    master_gain: Option<f64>,
}

fn run(
    rx: Receiver<Request>,
    self_tx: Sender<Request>,
    shared: Arc<Shared>,
    mut device_name: Option<String>,
    ready: Sender<Result<(), HostError>>,
) {
    let mut desired = Desired::default();
    let mut batch: Vec<Request> = Vec::with_capacity(MAX_BATCH);
    let mut shutdown = false;

    let first = build(device_name.as_deref(), &shared, &self_tx);
    let mut active = match first {
        Ok(a) => {
            let _ = ready.send(Ok(()));
            Some(a)
        }
        Err(e) => {
            set_error(&shared, &e);
            // Report the failure but keep the thread alive: the user may plug
            // a device in, and `select_device` must still work.
            let _ = ready.send(Err(e));
            None
        }
    };

    loop {
        // A timeout rather than a blocking recv, so recycled allocations are
        // reclaimed promptly even when the UI is idle.
        let request = match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(r) => Some(r),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => break,
        };

        if let Some(a) = active.as_mut() {
            drain_recycle(a);
        }

        let Some(request) = request else { continue };

        // Take everything else already queued, then collapse redundant writes.
        // A knob drag emits a value per mouse event -- far faster than the
        // callback drains -- and only the newest one is audible. Without this
        // a fast drag can overflow the ring and lose commands.
        batch.push(request);
        while batch.len() < MAX_BATCH {
            match rx.try_recv() {
                Ok(r) => batch.push(r),
                Err(_) => break,
            }
        }
        coalesce(&mut batch);

        for request in batch.drain(..) {
        match request {
            Request::Shutdown => {
                shutdown = true;
                continue;
            }

            Request::Engine(cmd) => {
                track_transport(&mut desired, &cmd);
                if let Some(a) = active.as_mut() {
                    push_command(a, &shared, cmd);
                }
            }

            Request::LoadPreset(preset) => {
                desired.position_s = 0.0;
                desired.master_gain = None;
                desired.preset = Some(*preset);
                if let Some(a) = active.as_mut() {
                    let p = desired.preset.as_ref().unwrap();
                    let state = Box::new(SessionState::from_preset(p, a.sample_rate));
                    push_command(a, &shared, Command::Replace(state));
                    // A preset that references audio files must reopen them,
                    // or its file layers load silent.
                    attach_files(a, &desired, &shared);
                }
            }

            Request::UpdateTimeline(timeline) => {
                // Build the swap here, off the audio thread, and remember it so
                // a later device rebuild reconstructs the edited session rather
                // than the preset as it was shipped.
                if let Some(p) = desired.preset.as_mut() {
                    p.timeline = (*timeline).clone();
                }
                if let Some(a) = active.as_mut() {
                    let swap = Box::new(TimelineSwap::new(*timeline));
                    push_command(a, &shared, Command::ReplaceTimeline(swap));
                }
            }

            Request::AttachFile { index, path, looping } => {
                // Remember it so a device rebuild reopens the same file.
                if let Some(p) = desired.preset.as_mut() {
                    if let Some(l) = p.layers.get_mut(index) {
                        l.file_path = Some(path.clone());
                        l.loop_file = looping;
                    }
                }
                if let Some(a) = active.as_mut() {
                    match StreamedSource::open(&path, a.sample_rate as u32, looping) {
                        Ok(src) => push_command(
                            a,
                            &shared,
                            Command::AttachSource { index, source: Box::new(src) },
                        ),
                        Err(e) => set_status(&shared, |s| s.last_error = Some(e)),
                    }
                }
            }

            Request::DetachFile { index } => {
                if let Some(p) = desired.preset.as_mut() {
                    if let Some(l) = p.layers.get_mut(index) {
                        l.file_path = None;
                    }
                }
                if let Some(a) = active.as_mut() {
                    push_command(a, &shared, Command::DetachSource { index });
                }
            }

            Request::SelectDevice(name) => {
                device_name = name;
                remember_position(&shared, &mut desired);
                active = rebuild(&mut desired, device_name.as_deref(), &shared, &self_tx);
            }

            Request::DeviceLost(reason) => {
                shared.telemetry.note_error();
                set_status(&shared, |s| {
                    s.running = false;
                    s.last_error = Some(reason.clone());
                });
                remember_position(&shared, &mut desired);
                // Fall back to the system default and forget the selection:
                // the most common cause is headphones being unplugged, and the
                // built-in output is what the user now expects to hear.
                device_name = None;
                active = rebuild(&mut desired, device_name.as_deref(), &shared, &self_tx);
            }
        }
        }

        if shutdown {
            break;
        }
    }

    // Stop the stream before the channels it writes into are dropped.
    if let Some(a) = active.take() {
        let _ = a.stream.pause();
        drop(a);
    }
}

fn track_transport(desired: &mut Desired, cmd: &Command) {
    match cmd {
        Command::Play => desired.transport = Transport::Playing,
        Command::Pause => desired.transport = Transport::Paused,
        Command::Stop => {
            desired.transport = Transport::Stopped;
            desired.position_s = 0.0;
        }
        Command::Seek { seconds } => desired.position_s = *seconds,
        Command::SetMasterGain(g) => desired.master_gain = Some(*g),
        _ => {}
    }
}

fn remember_position(shared: &Shared, desired: &mut Desired) {
    let m = shared.meters();
    if m.position_s > 0.0 {
        desired.position_s = m.position_s;
    }
}

/// Rebuild the stream and restore the session to where it was.
fn rebuild(
    desired: &mut Desired,
    device_name: Option<&str>,
    shared: &Arc<Shared>,
    self_tx: &Sender<Request>,
) -> Option<Active> {
    match build(device_name, shared, self_tx) {
        Ok(mut a) => {
            if let Some(preset) = desired.preset.as_ref() {
                let state = Box::new(SessionState::from_preset(preset, a.sample_rate));
                push_command(&mut a, shared, Command::Replace(state));
            }
            attach_files(&mut a, desired, shared);
            if let Some(g) = desired.master_gain {
                push_command(&mut a, shared, Command::SetMasterGain(g));
            }
            if desired.position_s > 0.0 {
                push_command(
                    &mut a,
                    shared,
                    Command::Seek {
                        seconds: desired.position_s,
                    },
                );
            }
            // Resume playing only if we were: a device change while paused
            // must not start making sound.
            if desired.transport == Transport::Playing {
                push_command(&mut a, shared, Command::Play);
            }
            Some(a)
        }
        Err(e) => {
            set_error(shared, &e);
            None
        }
    }
}

/// Drop parameter writes that a later write in the same batch supersedes.
///
/// Only the newest value of any given parameter is audible, so keeping the
/// earlier ones costs queue space and buys nothing. Transport commands, preset
/// loads and device changes are never collapsed -- each one means something
/// distinct.
fn coalesce(batch: &mut Vec<Request>) {
    if batch.len() < 2 {
        return;
    }
    let mut seen_param: Vec<(usize, LayerParam)> = Vec::new();
    let mut seen_master = false;
    let mut keep = vec![true; batch.len()];

    // Backwards, so the surviving write of each parameter is the last one.
    for (i, req) in batch.iter().enumerate().rev() {
        match req {
            Request::Engine(Command::SetLayerParam { index, param, .. }) => {
                let key = (*index, *param);
                if seen_param.contains(&key) {
                    keep[i] = false;
                } else {
                    seen_param.push(key);
                }
            }
            Request::Engine(Command::SetMasterGain(_)) => {
                if seen_master {
                    keep[i] = false;
                } else {
                    seen_master = true;
                }
            }
            _ => {}
        }
    }

    let mut i = 0;
    batch.retain(|_| {
        let k = keep[i];
        i += 1;
        k
    });
}

/// Reopen every file layer the current preset names.
fn attach_files(active: &mut Active, desired: &Desired, shared: &Shared) {
    let Some(preset) = desired.preset.as_ref() else { return };
    for (index, cfg) in preset.layers.iter().enumerate() {
        if cfg.kind != LayerKind::File {
            continue;
        }
        let Some(path) = cfg.file_path.as_ref() else { continue };
        match StreamedSource::open(path, active.sample_rate as u32, cfg.loop_file) {
            Ok(src) => push_command(
                active,
                shared,
                Command::AttachSource { index, source: Box::new(src) },
            ),
            // A moved or deleted file leaves the layer silent and says so,
            // rather than failing the whole preset.
            Err(e) => set_status(shared, |s| {
                s.last_error = Some(format!("{}: {e}", cfg.name));
            }),
        }
    }
}

fn push_command(active: &mut Active, shared: &Shared, cmd: Command) {
    let mut cmd = cmd;
    // Back off rather than dropping immediately: the callback drains the whole
    // queue each period, so a full queue clears within one buffer period. The
    // budget here deliberately spans several of those, since a large buffer at
    // a low sample rate can be over 40 ms.
    for _ in 0..600 {
        match active.cmd_tx.push(cmd) {
            Ok(()) => return,
            Err(rtrb::PushError::Full(returned)) => {
                cmd = returned;
                std::thread::sleep(Duration::from_micros(200));
            }
        }
    }
    shared.telemetry.note_dropped_command();
}

/// Drop layer stacks and timelines displaced by an edit. Doing this here
/// rather than in the callback is the whole point of the recycle queue:
/// deallocation can block, and blocking in the callback is a dropout.
fn drain_recycle(active: &mut Active) {
    while let Ok(old) = active.recycle_rx.pop() {
        drop(old);
    }
}

fn set_status(shared: &Shared, f: impl FnOnce(&mut HostStatus)) {
    if let Ok(mut s) = shared.status.lock() {
        f(&mut s);
    }
}

fn set_error(shared: &Shared, e: &HostError) {
    set_status(shared, |s| {
        s.running = false;
        s.last_error = Some(e.to_string());
    });
}

/// Open a device and start a stream on it.
fn build(
    device_name: Option<&str>,
    shared: &Arc<Shared>,
    self_tx: &Sender<Request>,
) -> Result<Active, HostError> {
    let dev = device::resolve(device_name)?;
    let name = Some(dev.to_string());
    let chosen = device::choose_config(&dev)?;
    let channels = chosen.config.channels;
    let sample_rate = chosen.config.sample_rate as f64;

    let (cmd_tx, cmd_rx) = rtrb::RingBuffer::<Command>::new(COMMAND_CAPACITY);
    let (recycle_tx, recycle_rx) = rtrb::RingBuffer::<Recycled>::new(RECYCLE_CAPACITY);
    let (meters_in, meters_out) = triple_buffer::triple_buffer(&Meters::default());
    let (scope_in, scope_out) = triple_buffer::triple_buffer(&Scope::default());

    let stream = build_stream(
        &dev,
        &chosen.config,
        chosen.format,
        Engine::new(sample_rate),
        cmd_rx,
        recycle_tx,
        meters_in,
        scope_in,
        Arc::clone(shared),
        self_tx.clone(),
    )?;
    stream.play().map_err(|e| HostError::Play(e.to_string()))?;

    if let Ok(mut g) = shared.meters.lock() {
        *g = Some(meters_out);
    }
    if let Ok(mut g) = shared.scope.lock() {
        *g = Some(scope_out);
    }
    set_status(shared, |s| {
        s.device_name = name;
        s.sample_rate = sample_rate as u32;
        s.channels = channels;
        s.running = true;
        // Binaural beats depend on each ear receiving a different frequency,
        // which a mono output physically cannot deliver.
        s.binaural_capable = channels >= 2;
        s.last_error = None;
    });

    Ok(Active {
        stream,
        cmd_tx,
        recycle_rx,
        sample_rate,
    })
}

#[allow(clippy::too_many_arguments)]
fn build_stream(
    device: &Device,
    config: &StreamConfig,
    format: SampleFormat,
    engine: Engine,
    cmd_rx: rtrb::Consumer<Command>,
    recycle_tx: rtrb::Producer<Recycled>,
    meters: triple_buffer::Input<Meters>,
    scope: triple_buffer::Input<Scope>,
    shared: Arc<Shared>,
    self_tx: Sender<Request>,
) -> Result<cpal::Stream, HostError> {
    match format {
        SampleFormat::F32 => {
            make::<f32>(device, config, engine, cmd_rx, recycle_tx, meters, scope, shared, self_tx)
        }
        SampleFormat::I16 => {
            make::<i16>(device, config, engine, cmd_rx, recycle_tx, meters, scope, shared, self_tx)
        }
        SampleFormat::U16 => {
            make::<u16>(device, config, engine, cmd_rx, recycle_tx, meters, scope, shared, self_tx)
        }
        SampleFormat::I32 => {
            make::<i32>(device, config, engine, cmd_rx, recycle_tx, meters, scope, shared, self_tx)
        }
        other => Err(HostError::UnsupportedFormat(other)),
    }
}

#[allow(clippy::too_many_arguments)]
fn make<T>(
    device: &Device,
    config: &StreamConfig,
    mut engine: Engine,
    mut cmd_rx: rtrb::Consumer<Command>,
    mut recycle_tx: rtrb::Producer<Recycled>,
    mut meters: triple_buffer::Input<Meters>,
    mut scope_out: triple_buffer::Input<Scope>,
    shared: Arc<Shared>,
    self_tx: Sender<Request>,
) -> Result<cpal::Stream, HostError>
where
    T: cpal::SizedSample + cpal::FromSample<f32> + Send + 'static,
{
    let channels = config.channels as usize;
    let sample_rate = config.sample_rate as f64;

    // Preallocated: the callback must never allocate.
    let mut scratch = vec![0.0f32; SCRATCH_FRAMES * 2];
    let mut writer = ScopeWriter::default();
    let telemetry = Arc::clone(&shared);

    let stream = device
        .build_output_stream(
            *config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                let started = Instant::now();

                // Drain control messages first so this block reflects them.
                while let Ok(cmd) = cmd_rx.pop() {
                    if let Some(displaced) = engine.apply(cmd) {
                        // Hand the old stack back rather than dropping it
                        // here; deallocation can block.
                        let _ = recycle_tx.push(displaced);
                    }
                }

                let frames = data.len() / channels.max(1);
                let mut done = 0usize;
                while done < frames {
                    let n = (frames - done).min(SCRATCH_FRAMES);
                    let block = &mut scratch[..n * 2];
                    engine.process(block);

                    for f in 0..n {
                        let l = block[f * 2];
                        let r = block[f * 2 + 1];
                        let base = (done + f) * channels;
                        match channels {
                            0 => {}
                            1 => data[base] = T::from_sample((l + r) * 0.5),
                            _ => {
                                data[base] = T::from_sample(l);
                                data[base + 1] = T::from_sample(r);
                                // Silence any surround channels rather than
                                // leaving whatever the backend had there.
                                for c in 2..channels {
                                    data[base + c] = T::from_sample(0.0f32);
                                }
                            }
                        }
                        writer.push((l + r) * 0.5);
                    }
                    done += n;
                }

                meters.write(engine.meters());
                scope_out.write(writer.snapshot());
                telemetry
                    .telemetry
                    .record(started.elapsed().as_secs_f64(), frames, sample_rate);
            },
            move |err| {
                // Not the real-time callback -- the stream is already broken,
                // so allocating to report it is fine.
                let _ = self_tx.send(Request::DeviceLost(err.to_string()));
            },
            None,
        )
        .map_err(|e| HostError::BuildStream(e.to_string()))?;

    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn param(index: usize, param: LayerParam, value: f64) -> Request {
        Request::Engine(Command::SetLayerParam { index, param, value })
    }

    /// Describe a batch compactly so assertions read clearly.
    fn shape(batch: &[Request]) -> Vec<String> {
        batch
            .iter()
            .map(|r| match r {
                Request::Engine(Command::SetLayerParam { index, param, value }) => {
                    format!("p{index}:{param:?}={value}")
                }
                Request::Engine(Command::SetMasterGain(g)) => format!("master={g}"),
                Request::Engine(Command::Play) => "play".into(),
                Request::Engine(Command::Stop) => "stop".into(),
                Request::Engine(Command::Seek { seconds }) => format!("seek={seconds}"),
                Request::Engine(_) => "cmd".into(),
                Request::LoadPreset(_) => "load".into(),
            Request::UpdateTimeline(_) => "timeline".into(),
            Request::AttachFile { .. } => "attach".into(),
            Request::DetachFile { .. } => "detach".into(),
                Request::SelectDevice(_) => "device".into(),
                Request::DeviceLost(_) => "lost".into(),
                Request::Shutdown => "shutdown".into(),
            })
            .collect()
    }

    #[test]
    fn only_the_last_write_of_a_parameter_survives() {
        let mut batch = vec![
            param(0, LayerParam::Carrier, 100.0),
            param(0, LayerParam::Carrier, 200.0),
            param(0, LayerParam::Carrier, 300.0),
        ];
        coalesce(&mut batch);
        assert_eq!(shape(&batch), vec!["p0:Carrier=300"]);
    }

    #[test]
    fn distinct_parameters_and_layers_are_kept_apart() {
        let mut batch = vec![
            param(0, LayerParam::Carrier, 100.0),
            param(0, LayerParam::Beat, 7.0),
            param(1, LayerParam::Carrier, 400.0),
            param(0, LayerParam::Carrier, 150.0),
        ];
        coalesce(&mut batch);
        // The superseded layer-0 carrier goes; everything else stays, in order.
        assert_eq!(
            shape(&batch),
            vec!["p0:Beat=7", "p1:Carrier=400", "p0:Carrier=150"]
        );
    }

    /// Transport commands each mean something distinct, so collapsing them
    /// would change behaviour rather than just save queue space.
    #[test]
    fn transport_commands_are_never_collapsed() {
        let mut batch = vec![
            Request::Engine(Command::Play),
            Request::Engine(Command::Seek { seconds: 10.0 }),
            Request::Engine(Command::Seek { seconds: 20.0 }),
            Request::Engine(Command::Stop),
        ];
        coalesce(&mut batch);
        assert_eq!(shape(&batch), vec!["play", "seek=10", "seek=20", "stop"]);
    }

    #[test]
    fn only_the_last_master_gain_survives() {
        let mut batch = vec![
            Request::Engine(Command::SetMasterGain(0.1)),
            Request::Engine(Command::Play),
            Request::Engine(Command::SetMasterGain(0.9)),
        ];
        coalesce(&mut batch);
        assert_eq!(shape(&batch), vec!["play", "master=0.9"]);
    }

    #[test]
    fn a_drag_collapses_to_a_single_write() {
        // 500 mousemove events, as a fast drag would produce.
        let mut batch: Vec<Request> = (0..500)
            .map(|i| param(0, LayerParam::Beat, 4.0 + i as f64 * 0.01))
            .collect();
        coalesce(&mut batch);
        assert_eq!(batch.len(), 1);
        assert_eq!(shape(&batch), vec!["p0:Beat=8.99"]);
    }

    #[test]
    fn short_batches_are_left_alone() {
        let mut batch = vec![param(0, LayerParam::Beat, 4.0)];
        coalesce(&mut batch);
        assert_eq!(batch.len(), 1);
        let mut empty: Vec<Request> = vec![];
        coalesce(&mut empty);
        assert!(empty.is_empty());
    }
}
