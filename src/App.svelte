<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
  import { api } from "./lib/api";
  import { clock } from "./lib/curve";
  import LibraryBar from "./lib/components/LibraryBar.svelte";
  import LayerCard from "./lib/components/LayerCard.svelte";
  import NumberField from "./lib/components/NumberField.svelte";
  import PlayView from "./lib/components/PlayView.svelte";
  import PresetBrowser from "./lib/components/PresetBrowser.svelte";
  import TimelineEditor from "./lib/components/TimelineEditor.svelte";
  import Visualizer from "./lib/components/Visualizer.svelte";
  import {
    bandFor,
    type DeviceInfo,
    type Health,
    type LayerParam,
    type Meters,
    type Preset,
    type PresetSource,
    type PresetSummary,
    type Timeline,
  } from "./lib/types";

  const SAFETY_KEY = "truezen.safety.ack.v1";
  const VIEW_KEY = "truezen.view";

  type View = "play" | "lab";

  let presets = $state<PresetSummary[]>([]);
  let preset = $state<Preset | null>(null);
  let devices = $state<DeviceInfo[]>([]);
  let health = $state<Health | null>(null);
  let meters = $state<Meters | null>(null);
  let wave = $state<number[]>([]);
  let env = $state<number[]>([]);
  let error = $state<string | null>(null);
  let acknowledged = $state(false);
  let masterGain = $state(0.7);
  let view = $state<View>("play");
  let dirty = $state(false);
  let source = $state<PresetSource | null>(null);
  let libraryDir = $state("");
  let notice = $state<string | null>(null);
  let exporting = $state<number | null>(null);

  let playing = $derived(meters?.playing ?? false);
  let sampleRate = $derived(health?.status.sampleRate || 48000);
  let duration = $derived(meters?.durationS ?? 0);
  let position = $derived(meters?.positionS ?? 0);
  let latchedAny = $derived((meters?.latchedTracks ?? 0) !== 0);
  let needsHeadphones = $derived(preset?.requires_headphones ?? false);
  let monoOutput = $derived(health ? !health.status.binauralCapable : false);

  function fail(e: unknown) {
    error = e instanceof Error ? e.message : String(e);
  }

  $effect(() => {
    acknowledged = localStorage.getItem(SAFETY_KEY) === "1";
    const saved = localStorage.getItem(VIEW_KEY);
    if (saved === "play" || saved === "lab") view = saved;

    Promise.all([api.listPresets(), api.listDevices().catch(() => []), api.health(), api.editing()])
      .then(([p, d, h, e]) => {
        presets = p;
        devices = d;
        health = h;
        libraryDir = e.libraryDir;
        if (h.error) error = h.error;
      })
      .catch(fail);

    // 30 Hz is smooth enough for meters and a scope, and keeps the IPC traffic
    // to one round trip per frame.
    const poll = setInterval(async () => {
      try {
        const f = await api.poll();
        meters = f.meters;
        wave = f.wave;
        env = f.env;
      } catch {
        /* the host is down; `health` below reports why */
      }
    }, 33);

    const vitals = setInterval(async () => {
      try {
        health = await api.health();
      } catch {
        /* ignore */
      }
    }, 1000);

    return () => {
      clearInterval(poll);
      clearInterval(vitals);
    };
  });

  async function choose(id: string) {
    try {
      preset = await api.loadPreset(id);
      masterGain = preset.master_gain;
      dirty = false;
      source = presets.find((p) => p.id === id)?.source ?? null;
      error = null;
    } catch (e) {
      fail(e);
    }
  }

  /// Re-read the library after anything that could have changed it.
  async function refreshLibrary(selectId?: string) {
    presets = await api.listPresets();
    const e = await api.editing();
    dirty = e.dirty;
    source = e.source;
    libraryDir = e.libraryDir;
    if (selectId) source = presets.find((p) => p.id === selectId)?.source ?? source;
  }

  function announce(msg: string) {
    notice = msg;
    setTimeout(() => (notice = null), 4000);
  }

  async function save() {
    try {
      const s = await api.savePreset();
      await refreshLibrary(s.id);
      announce(`Saved “${s.name}”.`);
    } catch (e) {
      fail(e);
    }
  }

  async function saveAs(name: string) {
    try {
      const s = await api.savePresetAs(name);
      preset = await api.currentPreset();
      await refreshLibrary(s.id);
      announce(`Saved “${s.name}” to your library.`);
    } catch (e) {
      fail(e);
    }
  }

  async function remove() {
    if (!preset) return;
    const reverting = source === "override";
    try {
      await api.deletePreset(preset.id);
      preset = await api.currentPreset();
      await refreshLibrary(preset?.id);
      announce(reverting ? "Reverted to the built-in preset." : "Deleted.");
    } catch (e) {
      fail(e);
    }
  }

  const AUDIO_EXTS = ["wav", "mp3", "flac", "m4a", "aac", "ogg", "opus", "aiff", "aif"];

  async function pickAudio(): Promise<string | null> {
    const path = await openDialog({
      multiple: false,
      filters: [{ name: "Audio", extensions: AUDIO_EXTS }],
    });
    return typeof path === "string" ? path : null;
  }

  async function addFileLayer() {
    try {
      const path = await pickAudio();
      if (!path) return;
      preset = await api.addFileLayer(path);
      dirty = true;
      announce("Added the audio layer.");
    } catch (e) {
      fail(e);
    }
  }

  async function pickLayerFile(index: number) {
    try {
      const path = await pickAudio();
      if (!path) return;
      await api.setLayerFile(index, path);
      if (preset?.layers[index]) preset.layers[index].file_path = path;
      dirty = true;
    } catch (e) {
      fail(e);
    }
  }

  function setLooping(index: number, looping: boolean) {
    if (preset?.layers[index]) preset.layers[index].loop_file = looping;
    dirty = true;
    api.setLayerLooping(index, looping).catch(fail);
  }

  function setDuck(index: number, on: boolean) {
    const layer = preset?.layers[index];
    if (layer) layer.ducks_others = on;
    dirty = true;
    api.setLayerDuck(index, on, layer?.duck_depth ?? 0.6).catch(fail);
  }

  async function removeLayer(index: number) {
    try {
      preset = await api.removeLayer(index);
      dirty = true;
    } catch (e) {
      fail(e);
    }
  }

  async function exportAudio() {
    if (!preset) return;
    try {
      // An open-ended preset has no length of its own, so pick a sensible one.
      const seconds = preset.timeline ? null : 1800;
      const path = await saveDialog({
        defaultPath: `${preset.id}.wav`,
        filters: [{ name: "WAV audio", extensions: ["wav"] }],
      });
      if (!path) return;

      exporting = 0;
      const unlisten = await listen<number>("export-progress", (e) => {
        exporting = e.payload;
      });
      try {
        const rendered = await api.exportAudio(path, seconds, 24);
        announce(`Rendered ${Math.round(rendered / 60)} minutes to ${path}.`);
      } finally {
        unlisten();
        exporting = null;
      }
    } catch (e) {
      exporting = null;
      fail(e);
    }
  }

  async function exportPreset() {
    if (!preset) return;
    try {
      const path = await saveDialog({
        defaultPath: `${preset.id}.json`,
        filters: [{ name: "TrueZen preset", extensions: ["json"] }],
      });
      if (!path) return;
      await api.exportPreset(preset.id, path);
      announce(`Exported to ${path}.`);
    } catch (e) {
      fail(e);
    }
  }

  async function importPreset() {
    try {
      const path = await openDialog({
        multiple: false,
        filters: [{ name: "TrueZen preset", extensions: ["json"] }],
      });
      if (typeof path !== "string") return;
      const s = await api.importPreset(path);
      await refreshLibrary();
      await choose(s.id);
      announce(`Imported “${s.name}”.`);
    } catch (e) {
      fail(e);
    }
  }

  /** Which timeline track drives a layer parameter, and whether it is latched. */
  function trackFor(index: number, param: LayerParam) {
    const tracks = preset?.timeline?.tracks;
    if (!tracks) return null;
    const i = tracks.findIndex(
      (t) => t.target.kind === "layer" && t.target.index === index && t.target.param === param,
    );
    if (i < 0) return null;
    return { track: i, latched: ((meters?.latchedTracks ?? 0) & (1 << i)) !== 0 };
  }

  function onParam(index: number, param: LayerParam, value: number) {
    if (!preset) return;
    // Update locally first: polling only reports the first layer's beat and
    // carrier, so the UI would otherwise lag or snap back under the cursor.
    const layer = preset.layers[index];
    if (layer) {
      if (param === "carrier") layer.carrier_hz = value;
      else if (param === "beat") layer.beat_hz = value;
      else if (param === "gain") layer.gain = value;
      else if (param === "pan") layer.pan = value;
      else if (param === "filter_cutoff") layer.filter_cutoff_hz = value;
      else if (param === "duty") layer.duty = value;
      else if (param === "ramp_ms") layer.ramp_ms = value;
      else if (param === "depth") layer.depth = value;
    }
    dirty = true;
    api.setLayerParam(index, param, value).catch(fail);
  }

  function onEnabled(index: number, on: boolean) {
    if (preset?.layers[index]) preset.layers[index].enabled = on;
    dirty = true;
    api.setLayerEnabled(index, on).catch(fail);
  }

  function setGain(v: number) {
    masterGain = v;
    dirty = true;
    api.setMasterGain(v).catch(fail);
  }

  function setView(v: View) {
    view = v;
    localStorage.setItem(VIEW_KEY, v);
  }

  function onTimeline(t: Timeline) {
    if (!preset) return;
    // Keep the local copy in step so the editor redraws from the same shape the
    // engine just received.
    preset.timeline = t;
    dirty = true;
    api.updateTimeline(t).catch(fail);
  }

  function acknowledge() {
    localStorage.setItem(SAFETY_KEY, "1");
    acknowledged = true;
  }

</script>

{#if !acknowledged}
  <div class="scrim">
    <div class="notice">
      <h2>Before you start</h2>
      <p>
        Audio entrainment carries a rare but documented seizure risk for people with photosensitive
        or audiogenic epilepsy. If that applies to you, or you are unsure, speak to a doctor first.
      </p>
      <p>Do not use TrueZen while driving or operating machinery.</p>
      <p class="fine">
        The evidence for entrainment is real but modest &mdash; strongest for anxiety reduction and
        some attention measures, inconsistent elsewhere. Presets here describe intentions, not
        clinical outcomes.
      </p>
      <button class="primary" onclick={acknowledge}>I understand</button>
    </div>
  </div>
{/if}

<div class="app">
  <header class="bar">
    <div class="brand">
      <span class="mark"></span>
      TrueZen
    </div>

    <nav class="views">
      <button class:on={view === "play"} onclick={() => setView("play")}>Play</button>
      <button class:on={view === "lab"} onclick={() => setView("lab")}>Lab</button>
    </nav>

    <div class="spacer"></div>

    {#if devices.length > 1}
      <select
        class="devices"
        title="Output device"
        value={devices.find((d) => d.name === health?.status.deviceName)?.id ?? ""}
        onchange={(e) => api.selectDevice(e.currentTarget.value || null).catch(fail)}
      >
        {#each devices as d (d.id)}
          <option value={d.id}>{d.name}{d.isDefault ? " (default)" : ""}</option>
        {/each}
      </select>
    {/if}

    {#if health}
      <div class="vitals" title="Audio callback load and health">
        <span class="dot" class:ok={health.status.running} class:bad={!health.status.running}></span>
        <span class="device">{health.status.deviceName ?? "no device"}</span>
        <span class="sep">·</span>
        <span class="mono">{(health.status.sampleRate / 1000).toFixed(1)} kHz</span>
        <span class="sep">·</span>
        <span class="mono">{(health.stats.lastLoadPermille / 10).toFixed(1)}%</span>
        {#if health.stats.overloads > 0}
          <span class="warnpill">{health.stats.overloads} overloads</span>
        {/if}
      </div>
    {/if}
  </header>

  {#if error}
    <div class="banner err">
      {error}
      <button onclick={() => (error = null)}>dismiss</button>
    </div>
  {/if}

  {#if notice}
    <div class="banner ok">{notice}</div>
  {/if}

  {#if needsHeadphones && monoOutput}
    <div class="banner warn">
      This preset uses binaural layers, but the output device is mono. Binaural beats need a
      different frequency in each ear and will not work.
    </div>
  {:else if needsHeadphones}
    <div class="banner info">
      Headphones required &mdash; the beat in this preset exists only when each ear hears a
      different frequency.
    </div>
  {/if}

  <main>
    <PresetBrowser {presets} selected={preset?.id ?? null} onselect={choose} />

    {#if !preset}
      <!-- The empty state belongs to neither view: with nothing loaded there is
           no session to sit with and nothing to edit. -->
      <section class="stage">
        <div class="placeholder">
          <p>Choose a preset to begin.</p>
          <p class="fine">
            <strong>Play</strong> is the session itself &mdash; a progress ring, the live beat rate,
            and little else. <strong>Lab</strong> opens the layer rack and the timeline editor.
          </p>
          <p class="fine">
            Every control is drag-to-scrub and click-to-type. Shift for fine steps, Alt for coarse.
            Frequency fields also accept names &mdash; try typing <code>schumann</code>.
          </p>
        </div>
      </section>
    {:else if view === "play"}
      <section class="stage">
        <PlayView
          {preset}
          {meters}
          {masterGain}
          onplay={() => (playing ? api.pause() : api.play()).catch(fail)}
          onstop={() => api.stop().catch(fail)}
          ongain={setGain}
          onseek={(s) => api.seek(s).catch(fail)}
        />
      </section>
    {:else}
      <section class="stage">
        <div class="transport">
          <button class="play" onclick={() => (playing ? api.pause() : api.play()).catch(fail)}
            disabled={!preset}>
            {playing ? "Pause" : "Play"}
          </button>
          <button onclick={() => api.stop().catch(fail)} disabled={!preset}>Stop</button>

          <div class="progress">
            <input
              type="range"
              min="0"
              max={Math.max(duration, 1)}
              step="1"
              value={position}
              disabled={!preset || duration <= 0}
              oninput={(e) => api.seek(Number(e.currentTarget.value)).catch(fail)}
            />
            <div class="times">
              <span class="mono">{clock(position)}</span>
              <span class="mono">{duration > 0 ? clock(duration) : "open-ended"}</span>
            </div>
          </div>

          {#if meters}
            <div class="live">
              <div class="big mono">{meters.beatHz.toFixed(2)}<small>Hz</small></div>
              <div class="band">{bandFor(meters.beatHz).name}</div>
            </div>
          {/if}

          <div class="master">
            <NumberField
              label="Master"
              value={masterGain}
              min={0}
              max={1}
              step={0.005}
              decimals={3}
              onchange={setGain}
            />
          </div>

          {#if latchedAny}
            <button class="restore" onclick={() => api.unlatchAll().catch(fail)}>
              Restore automation
            </button>
          {/if}
        </div>

        <div class="viz">
          <Visualizer {wave} {env} {sampleRate} {playing} />
        </div>

        <div class="rack">
            <LibraryBar
              name={preset.name}
              {source}
              {dirty}
              {libraryDir}
              onsave={save}
              onsaveas={saveAs}
              ondelete={remove}
              onexport={exportPreset}
              onimport={importPreset}
              onrender={exportAudio}
              rendering={exporting}
            />
            <div class="rackhead">
              <h2>{preset.name}</h2>
              <p>{preset.description}</p>
            </div>
            {#each preset.layers as layer, i (i)}
              <LayerCard
                {layer}
                index={i}
                {trackFor}
                onparam={onParam}
                onenabled={onEnabled}
                onrelease={(track) => api.setTrackLatched(track, false).catch(fail)}
                onpickfile={pickLayerFile}
                onlooping={setLooping}
                onduck={setDuck}
                onremove={removeLayer}
              />
            {/each}

            <button class="addlayer" onclick={addFileLayer}>+ Add an audio file layer</button>

            {#if preset.timeline && preset.timeline.tracks.length > 0}
              <TimelineEditor
                timeline={preset.timeline}
                layers={preset.layers}
                positionS={position}
                latchedTracks={meters?.latchedTracks ?? 0}
                onchange={onTimeline}
                onseek={(s) => api.seek(s).catch(fail)}
                onrelease={(track) => api.setTrackLatched(track, false).catch(fail)}
              />
            {:else}
              <p class="noauto">
                This preset holds its settings for as long as it runs &mdash; there is no timeline
                to edit.
              </p>
            {/if}
        </div>
      </section>
    {/if}

  </main>
</div>

<style>
  .app {
    height: 100vh;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 16px;
    border-bottom: 1px solid var(--line);
    background: var(--panel);
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 8px;
    font-weight: 600;
    letter-spacing: 0.02em;
  }
  .mark {
    width: 14px;
    height: 14px;
    border-radius: 50%;
    border: 2px solid var(--accent);
    box-shadow: 6px 0 0 -2px var(--violet);
    margin-right: 6px;
  }
  .spacer {
    flex: 1;
  }
  .views {
    display: flex;
    gap: 2px;
    padding: 2px;
    border-radius: 8px;
    background: var(--sunken);
    border: 1px solid var(--line);
    margin-left: 8px;
  }
  .views button {
    padding: 4px 14px;
    border-radius: 6px;
    border: none;
    background: transparent;
    color: var(--muted);
    font-size: 11.5px;
    cursor: pointer;
  }
  .views button.on {
    background: var(--panel);
    color: var(--fg);
  }
  .devices {
    background: var(--sunken);
    border: 1px solid var(--line);
    border-radius: 6px;
    color: var(--fg-dim);
    font-size: 11px;
    padding: 4px 7px;
    max-width: 220px;
  }
  .devices:focus {
    outline: none;
    border-color: var(--accent-dim);
  }
  .vitals {
    display: flex;
    align-items: center;
    gap: 7px;
    font-size: 11px;
    color: var(--muted);
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--muted);
  }
  .dot.ok {
    background: var(--accent);
  }
  .dot.bad {
    background: var(--danger);
  }
  .sep {
    opacity: 0.4;
  }
  .warnpill {
    color: var(--danger);
    border: 1px solid var(--danger);
    border-radius: 999px;
    padding: 1px 6px;
    font-size: 9px;
  }

  .banner {
    padding: 8px 16px;
    font-size: 11.5px;
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .banner.err {
    background: color-mix(in srgb, var(--danger) 15%, transparent);
    color: var(--danger);
  }
  .banner.warn {
    background: color-mix(in srgb, var(--warn) 15%, transparent);
    color: var(--warn);
  }
  .banner.info {
    background: var(--violet-bg);
    color: var(--violet);
  }
  .banner.ok {
    background: var(--accent-bg);
    color: var(--accent);
  }
  .banner button {
    margin-left: auto;
    background: none;
    border: none;
    color: inherit;
    text-decoration: underline;
    cursor: pointer;
    font-size: 11px;
  }

  main {
    flex: 1;
    display: grid;
    grid-template-columns: 290px 1fr;
    gap: 14px;
    padding: 14px 16px 16px;
    min-height: 0;
  }
  .stage {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-height: 0;
    min-width: 0;
  }

  .transport {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 11px 14px;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 10px;
  }
  .transport button {
    padding: 7px 14px;
    border-radius: 7px;
    border: 1px solid var(--line);
    background: var(--sunken);
    color: var(--fg);
    font-size: 12px;
    cursor: pointer;
  }
  .transport button:hover:not(:disabled) {
    border-color: var(--line-hi);
  }
  .transport button:disabled {
    opacity: 0.35;
    cursor: default;
  }
  .play {
    min-width: 72px;
    border-color: var(--accent-dim) !important;
    color: var(--accent) !important;
  }
  .restore {
    color: var(--warn) !important;
    border-color: var(--warn-dim) !important;
  }
  .progress {
    flex: 1;
    min-width: 90px;
  }
  .progress input {
    width: 100%;
    accent-color: var(--accent);
  }
  .times {
    display: flex;
    justify-content: space-between;
    font-size: 10px;
    color: var(--muted);
    margin-top: 1px;
  }
  .live {
    text-align: right;
    min-width: 92px;
  }
  .big {
    font-size: 21px;
    font-variant-numeric: tabular-nums;
    line-height: 1;
  }
  .big small {
    font-size: 10px;
    color: var(--muted);
    margin-left: 3px;
  }
  .live .band {
    font-size: 9px;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--muted);
    margin-top: 3px;
  }
  .master {
    width: 96px;
  }

  .viz {
    height: 190px;
    flex-shrink: 0;
  }

  .rack {
    display: flex;
    flex-direction: column;
    gap: 10px;
    overflow-y: auto;
    min-height: 0;
    padding-right: 4px;
  }
  .rackhead h2 {
    font-size: 15px;
    margin: 0 0 3px;
  }
  .rackhead p {
    font-size: 11.5px;
    color: var(--muted);
    margin: 0 0 4px;
    line-height: 1.5;
    max-width: 68ch;
  }
  .addlayer {
    align-self: flex-start;
    font-size: 11.5px;
    padding: 8px 14px;
    border-radius: 8px;
    border: 1px dashed var(--line-hi);
    background: transparent;
    color: var(--muted);
    cursor: pointer;
  }
  .addlayer:hover {
    border-color: var(--accent-dim);
    color: var(--accent);
  }
  .noauto {
    font-size: 11.5px;
    color: var(--muted);
    padding: 10px 2px;
  }
  .placeholder {
    padding: 36px 8px;
    color: var(--muted);
    font-size: 13px;
  }
  .placeholder .fine {
    font-size: 11.5px;
    margin-top: 10px;
    max-width: 58ch;
    line-height: 1.6;
  }
  .placeholder strong {
    color: var(--fg-dim);
    font-weight: 600;
  }
  code {
    font-family: var(--mono);
    background: var(--sunken);
    padding: 1px 4px;
    border-radius: 3px;
    font-size: 11px;
  }

  .scrim {
    position: fixed;
    inset: 0;
    background: rgba(4, 7, 12, 0.86);
    display: grid;
    place-items: center;
    z-index: 50;
    padding: 24px;
  }
  .notice {
    max-width: 460px;
    background: var(--panel);
    border: 1px solid var(--line-hi);
    border-radius: 12px;
    padding: 22px 24px;
  }
  .notice h2 {
    margin: 0 0 12px;
    font-size: 16px;
  }
  .notice p {
    font-size: 12.5px;
    line-height: 1.6;
    color: var(--fg-dim);
    margin: 0 0 11px;
  }
  .notice .fine {
    font-size: 11px;
    color: var(--muted);
  }
  .primary {
    margin-top: 6px;
    padding: 8px 16px;
    border-radius: 7px;
    border: 1px solid var(--accent-dim);
    background: var(--accent-bg);
    color: var(--accent);
    font-size: 12.5px;
    cursor: pointer;
  }
  .primary:hover {
    background: var(--accent-dim);
  }

  .mono {
    font-family: var(--mono);
  }
</style>
