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
| 3 | Tauri bridge + Lab view | in progress |
| 4 | Timeline editor + Play view | |
| 5 | Preset import/export | |
| 6 | File layers + WAV export | |
| 7 | Journal, tray, hotkeys | |
| 8 | Packaging + CI | |

## Layout

```
crates/truezen-engine   pure DSP — no audio device, no UI, no filesystem
crates/truezen-host     cpal stream, lock-free control, telemetry
crates/truezen-render   offline renderer and spectral analyser
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

## Tests

```sh
cargo test --release                          # 55 tests, no hardware needed
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
