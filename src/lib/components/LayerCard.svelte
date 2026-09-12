<script lang="ts">
  import NumberField from "./NumberField.svelte";
  import { bandFor, type LayerConfig, type LayerParam } from "../types";

  interface Props {
    layer: LayerConfig;
    index: number;
    /** Which timeline track drives this parameter, and whether it is latched. */
    trackFor: (index: number, param: LayerParam) => { track: number; latched: boolean } | null;
    onparam: (index: number, param: LayerParam, value: number) => void;
    onenabled: (index: number, on: boolean) => void;
    onrelease: (track: number) => void;
  }

  let { layer, index, trackFor, onparam, onenabled, onrelease }: Props = $props();

  let expanded = $state(false);

  const KIND_LABEL: Record<string, string> = {
    binaural: "binaural",
    monaural: "monaural",
    isochronic: "isochronic",
    noise: "noise",
  };

  let hasTone = $derived(layer.kind !== "noise");
  let band = $derived(bandFor(layer.beat_hz));

  function latch(param: LayerParam) {
    return trackFor(index, param)?.latched ?? false;
  }
  function release(param: LayerParam) {
    const t = trackFor(index, param);
    if (t) onrelease(t.track);
  }
</script>

<article class="card" class:off={!layer.enabled}>
  <header>
    <label class="toggle">
      <input
        type="checkbox"
        checked={layer.enabled}
        onchange={(e) => onenabled(index, e.currentTarget.checked)}
      />
      <span></span>
    </label>

    <div class="title">
      <span class="name">{layer.name || `Layer ${index + 1}`}</span>
      <span class="kind kind-{layer.kind}">{KIND_LABEL[layer.kind]}</span>
    </div>

    {#if hasTone}
      <span class="band" title={band.hint}>{band.name}</span>
    {/if}

    <button class="more" onclick={() => (expanded = !expanded)} aria-expanded={expanded}>
      {expanded ? "less" : "more"}
    </button>
  </header>

  <div class="grid">
    {#if hasTone}
      <NumberField
        label="Carrier"
        unit="Hz"
        value={layer.carrier_hz}
        min={20}
        max={2000}
        step={0.1}
        decimals={2}
        log
        latched={latch("carrier")}
        onchange={(v) => onparam(index, "carrier", v)}
        onrelease={() => release("carrier")}
      />
      <NumberField
        label="Beat"
        unit="Hz"
        value={layer.beat_hz}
        min={0}
        max={60}
        step={0.01}
        decimals={2}
        latched={latch("beat")}
        onchange={(v) => onparam(index, "beat", v)}
        onrelease={() => release("beat")}
      />
    {/if}

    <NumberField
      label="Level"
      value={layer.gain}
      min={0}
      max={1}
      step={0.005}
      decimals={3}
      latched={latch("gain")}
      onchange={(v) => onparam(index, "gain", v)}
      onrelease={() => release("gain")}
    />

    {#if layer.kind === "binaural"}
      <!-- Panning a binaural layer unbalances the ears and weakens or destroys
           the beat, so the engine ignores it and the control is not offered. -->
      <div class="note">Pan is fixed: binaural needs both ears level.</div>
    {:else}
      <NumberField
        label="Pan"
        value={layer.pan}
        min={-1}
        max={1}
        step={0.01}
        decimals={2}
        latched={latch("pan")}
        onchange={(v) => onparam(index, "pan", v)}
        onrelease={() => release("pan")}
      />
    {/if}
  </div>

  {#if expanded}
    <div class="grid secondary">
      {#if layer.kind === "isochronic"}
        <NumberField
          label="Duty"
          value={layer.duty}
          min={0.05}
          max={1}
          step={0.01}
          decimals={2}
          onchange={(v) => onparam(index, "duty", v)}
        />
        <NumberField
          label="Ramp"
          unit="ms"
          value={layer.ramp_ms}
          min={0.5}
          max={100}
          step={0.5}
          decimals={1}
          onchange={(v) => onparam(index, "ramp_ms", v)}
        />
        <NumberField
          label="Depth"
          value={layer.depth}
          min={0}
          max={1}
          step={0.01}
          decimals={2}
          onchange={(v) => onparam(index, "depth", v)}
        />
      {/if}

      {#if layer.kind === "noise"}
        <NumberField
          label="Cutoff"
          unit="Hz"
          value={layer.filter_cutoff_hz}
          min={30}
          max={18000}
          step={1}
          decimals={0}
          log
          latched={latch("filter_cutoff")}
          onchange={(v) => onparam(index, "filter_cutoff", v)}
          onrelease={() => release("filter_cutoff")}
        />
        <div class="readout">
          <span class="rlabel">Colour</span><span class="rvalue">{layer.noise_color}</span>
        </div>
        <div class="readout">
          <span class="rlabel">Filter</span>
          <span class="rvalue">{layer.filter_mode.replace("_", " ")}</span>
        </div>
      {/if}

      {#if hasTone}
        <div class="readout">
          <span class="rlabel">Wave</span><span class="rvalue">{layer.waveform}</span>
        </div>
      {/if}

      {#if layer.am_rate_hz > 0}
        <div class="readout">
          <span class="rlabel">Swell</span>
          <span class="rvalue">{layer.am_rate_hz} Hz &times; {layer.am_depth}</span>
        </div>
      {/if}
    </div>
  {/if}
</article>

<style>
  .card {
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 10px;
    padding: 12px 14px 14px;
    transition: opacity 0.15s, border-color 0.15s;
  }
  .card.off {
    opacity: 0.45;
  }
  header {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 12px;
  }
  .title {
    display: flex;
    align-items: baseline;
    gap: 8px;
    flex: 1;
    min-width: 0;
  }
  .name {
    font-size: 13px;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .kind {
    font-size: 9px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    padding: 2px 6px;
    border-radius: 999px;
    background: var(--sunken);
    color: var(--muted);
    white-space: nowrap;
  }
  .kind-binaural {
    color: var(--accent);
    background: var(--accent-bg);
  }
  .kind-isochronic {
    color: var(--violet);
    background: var(--violet-bg);
  }
  .band {
    font-size: 10px;
    letter-spacing: 0.06em;
    color: var(--muted);
    font-family: var(--mono);
  }
  .more {
    font-size: 10px;
    color: var(--muted);
    background: none;
    border: none;
    cursor: pointer;
    padding: 2px 4px;
  }
  .more:hover {
    color: var(--fg);
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(110px, 1fr));
    gap: 10px 12px;
  }
  .secondary {
    margin-top: 12px;
    padding-top: 12px;
    border-top: 1px solid var(--line);
  }
  .note,
  .readout {
    font-size: 10px;
    color: var(--muted);
    align-self: center;
    line-height: 1.4;
  }
  .readout {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .rlabel {
    letter-spacing: 0.07em;
    text-transform: uppercase;
  }
  .rvalue {
    font-family: var(--mono);
    font-size: 13px;
    color: var(--fg);
  }

  .toggle input {
    position: absolute;
    opacity: 0;
    width: 0;
    height: 0;
  }
  .toggle span {
    display: block;
    width: 30px;
    height: 17px;
    border-radius: 999px;
    background: var(--sunken);
    border: 1px solid var(--line);
    position: relative;
    cursor: pointer;
    transition: background 0.15s;
  }
  .toggle span::after {
    content: "";
    position: absolute;
    top: 2px;
    left: 2px;
    width: 11px;
    height: 11px;
    border-radius: 50%;
    background: var(--muted);
    transition: transform 0.15s, background 0.15s;
  }
  .toggle input:checked + span {
    background: var(--accent-bg);
    border-color: var(--accent-dim);
  }
  .toggle input:checked + span::after {
    transform: translateX(13px);
    background: var(--accent);
  }
  .toggle input:focus-visible + span {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
</style>
