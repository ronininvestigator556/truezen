//! Tauri bridge.
//!
//! A thin, deliberately dumb layer: it owns the [`AudioHost`], forwards
//! commands, and serialises telemetry. No audio logic lives here.

use std::sync::Mutex;

use serde::Serialize;
use truezen_engine::factory;
use truezen_engine::preset::{Goal, Preset};
use truezen_engine::timeline::LayerParam;
use truezen_engine::Meters;
use truezen_host::{AudioHost, DeviceInfo, HostStatus, Stats};

/// A failure the UI can show. Commands return `Result<_, String>` because
/// Tauri needs the error serialisable and there is nothing the frontend can do
/// with a typed variant that it cannot do with the message.
type Cmd<T> = Result<T, String>;

struct App {
    host: Option<AudioHost>,
    /// Why the host could not start. Kept so the UI can explain a silent app
    /// rather than simply appearing broken.
    host_error: Option<String>,
    current: Option<Preset>,
}

impl App {
    fn new() -> Self {
        match AudioHost::spawn(None) {
            Ok(host) => Self {
                host: Some(host),
                host_error: None,
                current: None,
            },
            Err(e) => Self {
                host: None,
                host_error: Some(e.to_string()),
                current: None,
            },
        }
    }

    fn host(&self) -> Cmd<&AudioHost> {
        self.host.as_ref().ok_or_else(|| {
            self.host_error
                .clone()
                .unwrap_or_else(|| "audio host is not running".into())
        })
    }
}

type State<'a> = tauri::State<'a, Mutex<App>>;

fn with<T>(state: &State, f: impl FnOnce(&mut App) -> Cmd<T>) -> Cmd<T> {
    let mut app = state.lock().map_err(|_| "audio state is poisoned".to_string())?;
    f(&mut app)
}

// --- library -------------------------------------------------------------

/// Enough of a preset to render the browser without shipping every layer.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PresetSummary {
    id: String,
    name: String,
    goal: String,
    description: String,
    requires_headphones: bool,
    duration_s: f64,
    layer_count: usize,
}

impl From<&Preset> for PresetSummary {
    fn from(p: &Preset) -> Self {
        Self {
            id: p.id.clone(),
            name: p.name.clone(),
            goal: p.goal.label().to_string(),
            description: p.description.clone(),
            requires_headphones: p.requires_headphones,
            duration_s: p.duration_s(),
            layer_count: p.layers.len(),
        }
    }
}

#[tauri::command]
fn list_presets() -> Vec<PresetSummary> {
    factory::load_all().iter().map(PresetSummary::from).collect()
}

#[tauri::command]
fn goals() -> Vec<String> {
    [
        Goal::Focus,
        Goal::Meditation,
        Goal::Relaxation,
        Goal::Sleep,
        Goal::Exploration,
        Goal::Energy,
    ]
    .iter()
    .map(|g| g.label().to_string())
    .collect()
}

#[tauri::command]
fn load_preset(id: String, state: State) -> Cmd<Preset> {
    let preset = factory::by_id(&id).ok_or_else(|| format!("no preset '{id}'"))?;
    preset.validate()?;
    with(&state, |app| {
        app.host()?.load_preset(preset.clone()).map_err(|e| e.to_string())?;
        app.current = Some(preset.clone());
        Ok(preset)
    })
}

#[tauri::command]
fn current_preset(state: State) -> Option<Preset> {
    state.lock().ok().and_then(|a| a.current.clone())
}

// --- transport -----------------------------------------------------------

#[tauri::command]
fn play(state: State) -> Cmd<()> {
    with(&state, |a| a.host()?.play().map_err(|e| e.to_string()))
}

#[tauri::command]
fn pause(state: State) -> Cmd<()> {
    with(&state, |a| a.host()?.pause().map_err(|e| e.to_string()))
}

#[tauri::command]
fn stop(state: State) -> Cmd<()> {
    with(&state, |a| a.host()?.stop().map_err(|e| e.to_string()))
}

#[tauri::command]
fn seek(seconds: f64, state: State) -> Cmd<()> {
    with(&state, |a| a.host()?.seek(seconds).map_err(|e| e.to_string()))
}

// --- parameters ----------------------------------------------------------

#[tauri::command]
fn set_master_gain(gain: f64, state: State) -> Cmd<()> {
    with(&state, |a| {
        a.host()?.set_master_gain(gain).map_err(|e| e.to_string())
    })
}

/// `param` is the snake_case name used in the preset format, so the frontend
/// and the JSON on disk speak the same vocabulary.
fn parse_param(name: &str) -> Cmd<LayerParam> {
    Ok(match name {
        "carrier" => LayerParam::Carrier,
        "beat" => LayerParam::Beat,
        "gain" => LayerParam::Gain,
        "pan" => LayerParam::Pan,
        "filter_cutoff" => LayerParam::FilterCutoff,
        "duty" => LayerParam::Duty,
        "ramp_ms" => LayerParam::RampMs,
        "depth" => LayerParam::Depth,
        other => return Err(format!("unknown parameter '{other}'")),
    })
}

#[tauri::command]
fn set_layer_param(index: usize, param: String, value: f64, state: State) -> Cmd<()> {
    let param = parse_param(&param)?;
    with(&state, |a| {
        // Mirror the change into the retained preset so the Lab view survives
        // a device rebuild, which reconstructs the layer stack from it.
        if let Some(p) = a.current.as_mut() {
            if let Some(l) = p.layers.get_mut(index) {
                match param {
                    LayerParam::Carrier => l.carrier_hz = value,
                    LayerParam::Beat => l.beat_hz = value,
                    LayerParam::Gain => l.gain = value,
                    LayerParam::Pan => l.pan = value,
                    LayerParam::FilterCutoff => l.filter_cutoff_hz = value,
                    LayerParam::Duty => l.duty = value,
                    LayerParam::RampMs => l.ramp_ms = value,
                    LayerParam::Depth => l.depth = value,
                }
            }
        }
        a.host()?
            .set_layer_param(index, param, value)
            .map_err(|e| e.to_string())
    })
}

#[tauri::command]
fn set_layer_enabled(index: usize, on: bool, state: State) -> Cmd<()> {
    with(&state, |a| {
        if let Some(p) = a.current.as_mut() {
            if let Some(l) = p.layers.get_mut(index) {
                l.enabled = on;
            }
            p.refresh_headphone_flag();
        }
        a.host()?
            .set_layer_enabled(index, on)
            .map_err(|e| e.to_string())
    })
}

// --- automation latching -------------------------------------------------

#[tauri::command]
fn set_track_latched(track: usize, latched: bool, state: State) -> Cmd<()> {
    with(&state, |a| {
        a.host()?
            .set_track_latched(track, latched)
            .map_err(|e| e.to_string())
    })
}

#[tauri::command]
fn unlatch_all(state: State) -> Cmd<()> {
    with(&state, |a| a.host()?.unlatch_all().map_err(|e| e.to_string()))
}

// --- telemetry -----------------------------------------------------------

/// One poll's worth of UI state.
///
/// Meters and scope travel together: the UI redraws them in the same frame, so
/// splitting them would double the IPC round trips for no benefit.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Frame {
    meters: Meters,
    /// Raw mono samples, chronological. The UI both draws these and runs its
    /// own FFT over them.
    wave: Vec<f32>,
    /// Slow peak envelope, long enough a window to see the beat pulse.
    env: Vec<f32>,
}

#[tauri::command]
fn poll(state: State) -> Cmd<Frame> {
    with(&state, |a| {
        let host = a.host()?;
        let scope = host.scope();
        Ok(Frame {
            meters: host.meters(),
            wave: scope.wave.to_vec(),
            env: scope.env.to_vec(),
        })
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Health {
    status: HostStatus,
    stats: Stats,
    error: Option<String>,
}

#[tauri::command]
fn health(state: State) -> Health {
    let app = match state.lock() {
        Ok(a) => a,
        Err(_) => {
            return Health {
                status: HostStatus::default(),
                stats: Stats::default(),
                error: Some("audio state is poisoned".into()),
            }
        }
    };
    match app.host.as_ref() {
        Some(h) => Health {
            status: h.status(),
            stats: h.stats(),
            error: None,
        },
        None => Health {
            status: HostStatus::default(),
            stats: Stats::default(),
            error: app.host_error.clone(),
        },
    }
}

// --- devices -------------------------------------------------------------

#[tauri::command]
fn list_devices() -> Cmd<Vec<DeviceInfo>> {
    AudioHost::devices().map_err(|e| e.to_string())
}

#[tauri::command]
fn select_device(id: Option<String>, state: State) -> Cmd<()> {
    with(&state, |a| {
        a.host()?.select_device(id).map_err(|e| e.to_string())
    })
}

pub fn run() {
    tauri::Builder::default()
        .manage(Mutex::new(App::new()))
        .invoke_handler(tauri::generate_handler![
            list_presets,
            goals,
            load_preset,
            current_preset,
            play,
            pause,
            stop,
            seek,
            set_master_gain,
            set_layer_param,
            set_layer_enabled,
            set_track_latched,
            unlatch_all,
            poll,
            health,
            list_devices,
            select_device,
        ])
        .run(tauri::generate_context!())
        .expect("failed to start TrueZen");
}
