# TrueZen

A desktop studio for brainwave entrainment audio — binaural beats, isochronic
tones, monaural beats and multi-layer stacks — for meditation, focus and sleep.

Built around two things that pull against each other: **dial in an exact
carrier and beat frequency and change it live without artifacts**, and **start
most sessions from one click on a preset**.

Runs on macOS, Windows and Linux.

---

## Contents

- [What it actually does](#what-it-actually-does)
- [Installing](#installing)
- [Using it](#using-it)
  - [Play](#play)
  - [Lab](#lab)
  - [The timeline](#the-timeline)
  - [Your own audio](#your-own-audio)
  - [Journal](#journal)
- [Saving and sharing presets](#saving-and-sharing-presets)
- [Rendering to a file](#rendering-to-a-file)
- [Tray, hotkeys and sleep](#tray-hotkeys-and-sleep)
- [The preset library](#the-preset-library)
- [Safety](#safety)
- [An honest note on claims](#an-honest-note-on-claims)
- [Building from source](#building-from-source)
- [How it is put together](#how-it-is-put-together)
- [License](#license)

---

## What it actually does

TrueZen synthesises audio designed to nudge you toward a particular mental
state by presenting a slow rhythm — a *beat frequency* — underneath an audible
tone. It offers four kinds of layer, and a session is any number of them
stacked together.

| Layer | How the beat is made | Headphones? |
|---|---|---|
| **Binaural** | Each ear gets a slightly different frequency. The beat is produced in your brainstem and does not exist in the air. | **Required.** On speakers the two tones mix acoustically and the effect is lost. |
| **Monaural** | Both tones are summed into both channels, so the beat is real amplitude modulation. | Works on speakers. |
| **Isochronic** | One tone, switched on and off at the beat rate with a soft raised-cosine edge. | Works on speakers. The most obvious of the three. |
| **Noise** | Filtered white, pink or brown noise. | Anything. Masks the room and gives the tones something to sit in. |
| **Audio file** | Your own recording — music, rain, a spoken guidance track. | Anything. |

Beat frequencies are conventionally grouped into bands, and TrueZen labels
whatever you dial in:

| Band | Range | Associated with |
|---|---|---|
| delta | below 4 Hz | deep sleep |
| theta | 4–8 Hz | reverie, deep meditation |
| alpha | 8–12 Hz | relaxed, settled |
| SMR | 12–15 Hz | calm focus |
| beta | 15–30 Hz | alert, analytical |
| gamma | above 30 Hz | concentration |

The **carrier** is the pitch you actually hear. Binaural beat perception works
best with carriers between roughly 100 and 500 Hz and falls off above about
1 kHz.

## Installing

### From a release

Download the installer for your platform from the
[releases page](https://github.com/ronininvestigator556/truezen/releases).

Builds are **not code-signed**, so your operating system will object the first
time:

- **macOS** — the first launch is blocked. Right-click the app and choose
  **Open**, or run
  `xattr -dr com.apple.quarantine /Applications/TrueZen.app`.
- **Windows** — SmartScreen shows a warning. Choose **More info** →
  **Run anyway**.
- **Linux** — the `.deb` and `.AppImage` need WebKitGTK and ALSA; the `.deb`
  declares them.

### From source

See [Building from source](#building-from-source).

## Using it

There are three views, switched from the header.

### Play

The session itself, and deliberately almost empty: a progress ring, the live
beat rate with its band, the preset name, and the transport. This is the view
to leave open while you actually listen.

### Lab

Everything else. A rack of layer cards, each with its own controls, above the
timeline editor.

**Every numeric control both scrubs and types.**

| | |
|---|---|
| Drag left/right | scrub the value |
| Click | type an exact value |
| `Shift` + drag | fine — hundredths |
| `Alt` + drag | coarse |
| `↑` / `↓` | nudge |

Frequency fields scrub *geometrically*, so an octave costs the same travel
wherever you are in the range. They also accept names — type `schumann` into a
beat field and you get 7.83 Hz. Also recognised: `om` (136.1), `gamma` (40),
`a440`, `solfeggio` (528).

### The timeline

Presets are journeys, not fixed settings. The timeline gives each automated
parameter its own lane.

- **Drag** a breakpoint to move it
- **Double-click** a lane to add one
- **Shift** locks the time; **Alt** locks the value
- **Click** a breakpoint to open the inspector — exact time, exact value, and
  the curve used to approach it

Curves are `exponential` (the default, and the right one for frequency, since
rate perception is logarithmic), `linear`, `smoothstep` and `hold`.

Each lane is scaled to *its own data* rather than the parameter's full range,
with the extents labelled — a beat track moving between 8 and 10 Hz would
otherwise be a flat line pinned to the floor of a 0–60 Hz axis.

Edits swap the automation without restarting the session.

**Touching an automated control latches it.** The timeline writes to its
parameters continuously, so without this your edit would be silently undone
milliseconds later. Instead your value sticks, the field is marked `manual`,
and that track stops running. **Restore automation** in the transport hands
everything back.

### Your own audio

**+ Add an audio file layer** mixes in a recording of your own. Anything
symphonia decodes works — wav, mp3, flac, m4a, ogg, opus — at any sample rate;
it is resampled on the way in. Mono is centred, surround is folded to its
front pair.

- **Loop** crossfades the seam so a short bed plays continuously without a
  click. The fade is equal-power, which is right for the decorrelated material
  a bed usually is.
- **Duck others** marks the layer as a voice track: everything else drops
  underneath it while it speaks and comes back gently afterwards.

### Journal

Anything you listen to for more than a minute is recorded — which preset, how
long, whether it ran to the end — and afterwards TrueZen asks one question:
how was that? Five dots and an optional note.

The Journal view totals this per preset, so *"which of these actually works for
me"* has an answer built from your own sessions rather than from the
descriptions.

Each entry stores the preset **as it was at the time**, so **Load** brings back
exactly what you heard even if you have edited or deleted it since.

## Saving and sharing presets

Every edit marks the session **unsaved**.

- **Save** writes it to your library.
- **Save as…** makes a copy under a new name.
- **Export…** / **Import…** move a single preset file in or out.

Saving over a built-in preset does not touch the shipped one — it writes an
override that hides it in the list, and **Revert** removes the override to get
the original back. A built-in preset can never be destroyed.

Presets are one JSON file each, so they diff, they version-control, and you can
repair one by hand. An import whose id collides with something you already have
is given a new one rather than silently replacing it.

They live beside the journal:

| | |
|---|---|
| macOS | `~/Library/Application Support/com.truezen.desktop/` |
| Windows | `%APPDATA%\com.truezen.desktop\` |
| Linux | `~/.config/com.truezen.desktop/` |

## Rendering to a file

**Render audio…** writes the session to a 24-bit WAV so you can put it on a
phone. It asks how many minutes and shows the size first — uncompressed audio
is about 17 MB a minute, so a full session is most of a gigabyte.

Rendering runs roughly 500× faster than real time and drives the same engine
code as live playback, decoding file layers directly. The render **is** the
session, not an approximation of it.

The same renderer backs a CLI:

```bash
cargo run --release -p truezen-render -- list
cargo run --release -p truezen-render -- render deep-theta -o session.wav
cargo run --release -p truezen-render -- analyze schumann-ground
```

`analyze` reports what is actually in the signal rather than what the preset
claims. For `schumann-ground` — a 136.1 Hz carrier with a 7.83 Hz beat — it
prints 132.187 Hz left and 140.017 Hz right, a measured beat of 7.8303 Hz.

## Tray, hotkeys and sleep

While a session runs, TrueZen holds a system wake assertion so the machine does
not idle-sleep out from under it. The **display** is deliberately left free to
sleep — a dark screen is welcome; only the audio needs to keep going.

There is a tray icon with play/pause, stop, and the time remaining, plus two
global hotkeys:

| | |
|---|---|
| `Ctrl+Alt+Space` | play / pause |
| `Ctrl+Alt+S` | stop |

Deliberately not Cmd/Ctrl+Shift chords — a hotkey that steals Spotlight or a
text-editing shortcut is worse than no hotkey.

## The preset library

Fourteen presets ship with the app. Each is a timeline, not a fixed setting.

| Preset | Goal | |
|---|---|---|
| Deep Focus | focus | 5 min ramp 10 → 16 Hz beta, brown bed, then holds |
| Gamma Concentration | focus | steady 40 Hz isochronic — speaker-safe |
| Study & Reading | focus | quiet 12 Hz SMR, sits behind reading |
| Creative Reverie | exploration | 7.5 Hz theta on the 136.1 Hz "OM" carrier |
| Alpha Settle | meditation | 10 → 8 Hz over ten minutes, then twenty holding |
| Deep Theta | meditation | 10 → 4.5 Hz over twenty, long hold, deliberate climb back |
| Stack — Expanded State | exploration | three binaural layers at 4, 7 and 10 Hz, emphasis shifting down |
| Sleep Onset | sleep | 8 → 2.5 Hz over twenty-five, four-minute fade to silence |
| Power Nap | sleep | twenty minutes exactly, wakes you on a 12 Hz climb |
| Lucid / REM Window | exploration | 4.5 Hz theta with 40 Hz gamma bursts in the second half |
| Anxiety Downshift | relaxation | 10 Hz alpha swelling at 0.1 Hz — breathe with it |
| Schumann Ground | relaxation | sustained 7.83 Hz, runs until you stop it |
| Body Relaxation | relaxation | 3.5 Hz delta on a low 111 Hz carrier |
| Morning Activation | energy | 6 → 18 Hz over eight minutes |

## Safety

**Audio entrainment carries a rare but documented seizure risk** for people
with photosensitive or audiogenic epilepsy. If that applies to you, or you are
unsure, speak to a doctor first.

**Do not use TrueZen while driving or operating machinery.**

The app shows this on first launch, ramps its output up rather than starting at
full level, and limits at −1 dBFS.

## An honest note on claims

The evidence for brainwave entrainment is real but modest — strongest for
anxiety reduction and some attention measures, inconsistent elsewhere. The
presets here describe **intentions**, not clinical outcomes, and nothing in
this app is a medical device or a treatment.

"Hemi-Sync" is a registered trademark of The Monroe Institute. The multi-layer
technique it refers to is implemented here as **Stack mode**; the name is not
used.

## Building from source

You need [Rust](https://rustup.rs) (stable) and Node 22+.

```bash
git clone https://github.com/ronininvestigator556/truezen.git
cd truezen
npm install
npm run tauri dev      # run it
npx tauri build        # build installers for this platform
```

On Linux you also need WebKitGTK, ALSA and D-Bus headers:

```bash
sudo apt-get install -y libwebkit2gtk-4.1-dev libappindicator3-dev \
  librsvg2-dev libxdo-dev libssl-dev libasound2-dev libdbus-1-dev patchelf
```

### Tests

```bash
cargo test --release                                 # 105 tests, no hardware needed
cargo test --release -p truezen-host -- --ignored    # 8 more, needs an audio device
npm run check                                        # frontend types
```

The DSP suite asserts on **measured spectra**, not on intent: that a binaural
layer's carriers are split by exactly the beat, that isochronic sidebands are
spaced by the beat rate, that the raised-cosine gate ramp suppresses splatter
by a factor of 1250, and that the beat is still exact after rendering four
hours of audio.

There is also a live soak test against real hardware:

```bash
cargo run --release -p truezen-host --example soak -- --seconds 60 --sweep
```

CI builds and tests on all three platforms for every push. Tauri cannot
cross-compile between desktop platforms, so each one gets its own runner.

## How it is put together

```
crates/truezen-engine   pure DSP — no audio device, no UI, no filesystem
crates/truezen-host     cpal stream, lock-free control, file decoding, export
crates/truezen-render   offline renderer and spectral analyser
src-tauri               Tauri bridge, preset store, journal
src                     Svelte 5 frontend
```

Three ideas hold the whole thing up.

**The engine's entire output interface is `process(&mut [f32])`.** It knows
nothing about devices, files or the UI. That is what lets one implementation
serve live playback, offline rendering and the test suite — and it is why an
exported file is bit-for-bit the session you heard.

**Oscillators use f64 phase accumulators, wrapped every sample.** The tempting
alternative, `sin(2π·f·t)` with a running `t`, drifts audibly: an eight-hour
session is 1.4 billion samples, well past f32's mantissa, and the beat
frequency wanders as `t` loses its low bits.

**The audio callback never allocates, locks, logs or makes a syscall.** The
audio thread owns the `cpal::Stream` — which is `!Send` — and does all heap
work; displaced layer stacks are handed *back* over a recycle queue rather than
dropped in the callback, where deallocation could block. Parameter writes are
coalesced before they reach the ring buffer, because a knob drag emits a value
per mouse event and only the newest one is audible.

## License

MIT — see [LICENSE](LICENSE).
