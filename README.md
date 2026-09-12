# TrueZen

A desktop brainwave-entrainment studio: binaural beats, isochronic tones,
monaural beats and multi-layer stacks, for meditation, focus and sleep.

Built around two goals that pull in opposite directions — dial in an exact
carrier and beat frequency and change it live without artifacts, *and* start
most sessions from one click on a preset.

## Status

| Phase | | |
|---|---|---|
| 0 | Workspace scaffold | done |
| 1 | DSP engine, preset library, offline render | done |
| 2 | Real-time audio host | done |
| 3 | Tauri bridge + Lab view | done |
| 4 | Timeline editor + Play view | done |
| 5 | Preset import/export | |
| 6 | File layers + WAV export | |
| 7 | Journal, tray, hotkeys | |
| 8 | Packaging + CI | |

## Layout

```
crates/truezen-engine   pure DSP — no audio device, no UI, no filesystem
crates/truezen-host     cpal stream, lock-free control, telemetry
crates/truezen-render   offline renderer and spectral analyser
src-tauri               Tauri bridge — owns the host, forwards commands
src                     Svelte 5 frontend: preset browser, Lab view, visualiser
```

`truezen-engine`'s entire output interface is `process(&mut [f32])`. That is
what lets one implementation serve live playback, offline rendering and the
test suite.

## Try it

```sh
cargo run --release -p truezen-render -- list
cargo run --release -p truezen-render -- analyze schumann-ground
cargo run --release -p truezen-render -- render deep-theta -o session.wav
```

`analyze` reports what is actually in the signal rather than what the preset
claims. For `schumann-ground` (136.1 Hz carrier, 7.83 Hz beat) it prints
132.187 Hz left and 140.017 Hz right — a measured beat of 7.8303 Hz.

Live audio soak test, on real hardware:

```sh
cargo run --release -p truezen-host --example soak -- --seconds 60 --sweep
```

## Run the app

```sh
npm install
npm run tauri dev
```

Every numeric control is drag-to-scrub and click-to-type: Shift for fine steps,
Alt for coarse, arrow keys to nudge. Frequency fields also accept names, so
typing `schumann` gives you 7.83 Hz.

There are two views. **Play** is the session itself: a progress ring, the live
beat rate, and little else. **Lab** opens the layer rack and the timeline
editor.

Presets are timelines, not static settings, so most of them are actively
driving their own parameters. Touching an automated control **latches** it: your
value sticks and that timeline track stops running, marked `manual` on the
field. "Restore automation" in the transport hands everything back.

In the timeline editor, drag a breakpoint to move it or double-click a lane to
add one; Shift locks the time and Alt locks the value. Each lane is scaled to
its own data rather than the parameter's full range, so a beat track moving
between 8 and 10 Hz reads as a shape instead of a flat line at the bottom of a
0-60 Hz axis. Edits swap the automation without restarting the session.

## Tests

```sh
cargo test --release                          # 68 tests, no hardware needed
cargo test --release -p truezen-host -- --ignored   # 8 tests, needs an audio device
```

The DSP suite asserts on measured spectra, not on intent: binaural carrier
split, isochronic sideband spacing, that the raised-cosine gate ramp suppresses
splatter, and that the beat is still exact after rendering four hours of audio.

## A note on claims

The evidence for brainwave entrainment is real but modest — strongest for
anxiety reduction and some attention measures, inconsistent elsewhere. Presets
here are framed as intentions, never as clinical claims.

"Hemi-Sync" is a registered trademark of The Monroe Institute. The multi-layer
technique is implemented here; the name is not used for it.

## Safety

Audio entrainment carries a rare but documented seizure risk for people with
photosensitive or audiogenic epilepsy. Do not use while driving or operating
machinery.

## License

MIT
