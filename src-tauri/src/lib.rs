//! Tauri bridge.
//!
//! A thin, deliberately dumb layer: it owns the [`AudioHost`], forwards
//! commands, and serialises telemetry. No audio logic lives here.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{Emitter, Manager};
use truezen_engine::layer::{LayerConfig, LayerKind};
use truezen_engine::preset::{Goal, Preset};
use truezen_engine::timeline::{LayerParam, Timeline};
use truezen_engine::Meters;
use truezen_host::export::{render_to_wav, ExportOptions};
use truezen_host::{AudioHost, DeviceInfo, HostStatus, Stats};

mod store;
use store::{Source, Store};

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
    /// True when `current` has been edited since it was loaded or saved.
    dirty: bool,
    store: Store,
}

impl App {
    fn new(store: Store) -> Self {
        let (host, host_error) = match AudioHost::spawn(None) {
            Ok(h) => (Some(h), None),
            Err(e) => (None, Some(e.to_string())),
        };
        Self {
            host,
            host_error,
            current: None,
            dirty: false,
            store,
        }
    }

    /// Record that the live preset no longer matches what is on disk.
    fn touch(&mut self) {
        if self.current.is_some() {
            self.dirty = true;
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
    source: Source,
}

fn summarise(p: &Preset, source: Source) -> PresetSummary {
    PresetSummary {
        id: p.id.clone(),
        name: p.name.clone(),
        goal: p.goal.label().to_string(),
        description: p.description.clone(),
        requires_headphones: p.requires_headphones,
        duration_s: p.duration_s(),
        layer_count: p.layers.len(),
        source,
    }
}

#[tauri::command]
fn list_presets(state: State) -> Vec<PresetSummary> {
    let Ok(app) = state.lock() else { return Vec::new() };
    app.store
        .all()
        .iter()
        .map(|(p, s)| summarise(p, *s))
        .collect()
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
    with(&state, |app| {
        let (preset, _) = app.store.get(&id).ok_or_else(|| format!("no preset '{id}'"))?;
        preset.validate()?;
        app.host()?.load_preset(preset.clone()).map_err(|e| e.to_string())?;
        app.current = Some(preset.clone());
        app.dirty = false;
        Ok(preset)
    })
}

// --- library -------------------------------------------------------------

/// The live preset, its origin, and whether it has unsaved edits.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Editing {
    preset: Option<Preset>,
    dirty: bool,
    source: Option<Source>,
    /// Where user presets live, so the UI can point at the folder.
    library_dir: String,
}

#[tauri::command]
fn editing(state: State) -> Editing {
    let Ok(app) = state.lock() else {
        return Editing { preset: None, dirty: false, source: None, library_dir: String::new() };
    };
    let source = app.current.as_ref().and_then(|p| app.store.get(&p.id)).map(|(_, s)| s);
    Editing {
        preset: app.current.clone(),
        dirty: app.dirty,
        source,
        library_dir: app.store.dir().display().to_string(),
    }
}

/// Save the live preset over itself. Saving a factory preset writes a user
/// override rather than touching the shipped one.
#[tauri::command]
fn save_preset(state: State) -> Cmd<PresetSummary> {
    with(&state, |app| {
        let mut preset = app.current.clone().ok_or("nothing is loaded")?;
        preset.refresh_headphone_flag();
        app.store.save(&preset)?;
        app.current = Some(preset.clone());
        app.dirty = false;
        let source = app.store.get(&preset.id).map(|(_, s)| s).unwrap_or(Source::User);
        Ok(summarise(&preset, source))
    })
}

/// Save the live preset under a new name, leaving the original alone.
#[tauri::command]
fn save_preset_as(name: String, state: State) -> Cmd<PresetSummary> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("a preset needs a name".into());
    }
    with(&state, |app| {
        let mut preset = app.current.clone().ok_or("nothing is loaded")?;
        preset.id = app.store.unique_id(&name);
        preset.name = name.clone();
        preset.refresh_headphone_flag();
        app.store.save(&preset)?;
        app.current = Some(preset.clone());
        app.dirty = false;
        Ok(summarise(&preset, Source::User))
    })
}

/// Delete a saved preset. For an id that also exists in the factory set this
/// reverts to the shipped version rather than removing it.
#[tauri::command]
fn delete_preset(id: String, state: State) -> Cmd<()> {
    with(&state, |app| {
        app.store.delete(&id)?;
        // If the deleted preset was loaded, fall back to whatever now answers
        // to that id so the UI is never pointing at something that is gone.
        if app.current.as_ref().is_some_and(|p| p.id == id) {
            match app.store.get(&id) {
                Some((p, _)) => {
                    app.host()?.load_preset(p.clone()).map_err(|e| e.to_string())?;
                    app.current = Some(p);
                }
                None => app.current = None,
            }
            app.dirty = false;
        }
        Ok(())
    })
}

#[tauri::command]
fn export_preset(id: String, path: String, state: State) -> Cmd<()> {
    with(&state, |app| {
        let (preset, _) = app.store.get(&id).ok_or_else(|| format!("no preset '{id}'"))?;
        let json = preset.to_json().map_err(|e| e.to_string())?;
        std::fs::write(&path, json).map_err(|e| format!("could not write {path}: {e}"))
    })
}

#[tauri::command]
fn import_preset(path: String, state: State) -> Cmd<PresetSummary> {
    let text = std::fs::read_to_string(&path).map_err(|e| format!("could not read {path}: {e}"))?;
    let mut preset = Preset::from_json(&text).map_err(|e| format!("{path} is not a preset: {e}"))?;
    preset.validate()?;

    with(&state, |app| {
        // Never silently replace something already in the library: an imported
        // file that happens to share an id is a different preset.
        if app.store.get(&preset.id).is_some() {
            preset.id = app.store.unique_id(&preset.name);
        }
        preset.refresh_headphone_flag();
        app.store.save(&preset)?;
        Ok(summarise(&preset, Source::User))
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
        if let Some(p) = a.current.as_mut() {
            p.master_gain = gain;
        }
        a.touch();
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
        a.touch();
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
        a.touch();
        a.host()?
            .set_layer_enabled(index, on)
            .map_err(|e| e.to_string())
    })
}

// --- audio file layers ---------------------------------------------------

/// Rebuild the session from the edited preset, returning to where it was.
///
/// Adding or removing a layer changes the shape of the layer stack, which the
/// engine can only take as a whole; keeping the position makes it feel like an
/// edit rather than a restart.
fn reload_in_place(app: &mut App) -> Cmd<()> {
    let preset = app.current.clone().ok_or("nothing is loaded")?;
    let at = app.host()?.meters().position_s;
    app.host()?.load_preset(preset).map_err(|e| e.to_string())?;
    if at > 0.0 {
        app.host()?.seek(at).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn add_file_layer(path: String, state: State) -> Cmd<Preset> {
    let name = std::path::Path::new(&path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Audio")
        .to_string();

    with(&state, |app| {
        let preset = app.current.as_mut().ok_or("load a preset first")?;
        preset.layers.push(LayerConfig {
            name,
            kind: LayerKind::File,
            // Imported audio joins under the tones, not over them.
            gain: 0.35,
            file_path: Some(path.clone()),
            loop_file: true,
            ..Default::default()
        });
        app.touch();
        reload_in_place(app)?;
        app.current.clone().ok_or_else(|| "nothing is loaded".into())
    })
}

#[tauri::command]
fn set_layer_file(index: usize, path: String, state: State) -> Cmd<()> {
    with(&state, |app| {
        let looping = {
            let preset = app.current.as_mut().ok_or("nothing is loaded")?;
            let layer = preset.layers.get_mut(index).ok_or("no such layer")?;
            layer.file_path = Some(path.clone());
            layer.loop_file
        };
        app.touch();
        app.host()?
            .attach_file(index, path, looping)
            .map_err(|e| e.to_string())
    })
}

#[tauri::command]
fn set_layer_looping(index: usize, looping: bool, state: State) -> Cmd<()> {
    with(&state, |app| {
        let path = {
            let preset = app.current.as_mut().ok_or("nothing is loaded")?;
            let layer = preset.layers.get_mut(index).ok_or("no such layer")?;
            layer.loop_file = looping;
            layer.file_path.clone()
        };
        app.touch();
        // Looping is decided when the file is opened, so it reopens.
        match path {
            Some(p) => app.host()?.attach_file(index, p, looping).map_err(|e| e.to_string()),
            None => Ok(()),
        }
    })
}

#[tauri::command]
fn set_layer_duck(index: usize, on: bool, depth: f64, state: State) -> Cmd<()> {
    with(&state, |app| {
        if let Some(p) = app.current.as_mut() {
            if let Some(l) = p.layers.get_mut(index) {
                l.ducks_others = on;
                l.duck_depth = depth;
            }
        }
        app.touch();
        app.host()?
            .set_layer_duck(index, on, depth)
            .map_err(|e| e.to_string())
    })
}

#[tauri::command]
fn remove_layer(index: usize, state: State) -> Cmd<Preset> {
    with(&state, |app| {
        {
            let preset = app.current.as_mut().ok_or("nothing is loaded")?;
            if index >= preset.layers.len() {
                return Err("no such layer".into());
            }
            if preset.layers.len() == 1 {
                return Err("a preset needs at least one layer".into());
            }
            preset.layers.remove(index);
            // Automation targets layers by index, so anything pointing past
            // the removed one would silently drive the wrong parameter.
            if let Some(tl) = preset.timeline.as_mut() {
                tl.tracks.retain(|t| match t.target {
                    truezen_engine::timeline::ParamTarget::Layer { index: i, .. } => i != index,
                    _ => true,
                });
                for track in tl.tracks.iter_mut() {
                    if let truezen_engine::timeline::ParamTarget::Layer { index: i, .. } =
                        &mut track.target
                    {
                        if *i > index {
                            *i -= 1;
                        }
                    }
                }
            }
            preset.refresh_headphone_flag();
        }
        app.touch();
        reload_in_place(app)?;
        app.current.clone().ok_or_else(|| "nothing is loaded".into())
    })
}

// --- export --------------------------------------------------------------

#[tauri::command]
async fn export_audio(
    path: String,
    seconds: Option<f64>,
    bits: u16,
    app: tauri::AppHandle,
    state: State<'_>,
) -> Cmd<f64> {
    let preset = state
        .lock()
        .map_err(|_| "audio state is poisoned".to_string())?
        .current
        .clone()
        .ok_or("nothing is loaded")?;

    // Rendering is CPU-bound and can run for seconds; doing it on the async
    // runtime's worker would stall every other command.
    tauri::async_runtime::spawn_blocking(move || {
        render_to_wav(
            &preset,
            ExportOptions { sample_rate: 48_000, bits, seconds },
            &path,
            |p| {
                let _ = app.emit("export-progress", p);
            },
        )
    })
    .await
    .map_err(|e| format!("the render did not finish: {e}"))?
}

// --- timeline ------------------------------------------------------------

/// Replace the session's automation.
///
/// The frontend owns the curve while editing and sends the whole timeline,
/// rather than a per-breakpoint command surface: an edit is a single atomic
/// swap that way, and there is no partial state to get wrong.
#[tauri::command]
fn update_timeline(timeline: Option<Timeline>, state: State) -> Cmd<()> {
    with(&state, |a| {
        let layers = a.current.as_ref().map_or(0, |p| p.layers.len());
        if let Some(tl) = timeline.as_ref() {
            tl.validate(layers)?;
        }
        if let Some(p) = a.current.as_mut() {
            p.timeline = timeline.clone();
        }
        a.touch();
        a.host()?
            .update_timeline(timeline)
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
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // Resolved here rather than in App::new because the config
            // directory is only known once Tauri has a handle.
            let dir: PathBuf = app
                .path()
                .app_config_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join("presets");
            app.manage(Mutex::new(App::new(Store::new(dir))));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_presets,
            editing,
            save_preset,
            save_preset_as,
            delete_preset,
            export_preset,
            import_preset,
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
            update_timeline,
            add_file_layer,
            set_layer_file,
            set_layer_looping,
            set_layer_duck,
            remove_layer,
            export_audio,
            unlatch_all,
            poll,
            health,
            list_devices,
            select_device,
        ])
        .run(tauri::generate_context!())
        .expect("failed to start TrueZen");
}
