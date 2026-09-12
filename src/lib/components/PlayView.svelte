<script lang="ts">
  import NumberField from "./NumberField.svelte";
  import { clock } from "../curve";
  import { bandFor, type Meters, type Preset } from "../types";

  interface Props {
    preset: Preset;
    meters: Meters | null;
    masterGain: number;
    onplay: () => void;
    onstop: () => void;
    ongain: (v: number) => void;
    onseek: (s: number) => void;
  }
  let { preset, meters, masterGain, onplay, onstop, ongain, onseek }: Props = $props();

  const R = 118;
  const C = 2 * Math.PI * R;

  let position = $derived(meters?.positionS ?? 0);
  let duration = $derived(meters?.durationS ?? 0);
  let playing = $derived(meters?.playing ?? false);
  let beat = $derived(meters?.beatHz ?? 0);
  let band = $derived(bandFor(beat));
  // An open-ended preset has no progress to show, so the ring stays full.
  let progress = $derived(duration > 0 ? Math.min(1, position / duration) : 1);
  let remaining = $derived(duration > 0 ? Math.max(0, duration - position) : 0);
</script>

<div class="play">
  <div class="ring">
    <svg viewBox="0 0 280 280">
      <circle class="track" cx="140" cy="140" r={R} />
      <circle
        class="fill"
        class:idle={!playing}
        cx="140"
        cy="140"
        r={R}
        stroke-dasharray="{C * progress} {C}"
        transform="rotate(-90 140 140)"
      />
    </svg>

    <div class="center">
      <div class="hz mono">
        {beat.toFixed(2)}<small>Hz</small>
      </div>
      <div class="band" title={band.hint}>{band.name}</div>
      <div class="elapsed mono">
        {clock(position)}{#if duration > 0}<span class="of"> / {clock(duration)}</span>{/if}
      </div>
    </div>
  </div>

  <h1>{preset.name}</h1>
  <p class="desc">{preset.description}</p>

  <div class="controls">
    <button class="big" onclick={onplay}>{playing ? "Pause" : "Play"}</button>
    <button onclick={onstop}>Stop</button>
    <div class="gain">
      <NumberField
        label="Level"
        value={masterGain}
        min={0}
        max={1}
        step={0.005}
        decimals={3}
        onchange={ongain}
      />
    </div>
  </div>

  {#if duration > 0}
    <input
      class="scrub"
      type="range"
      min="0"
      max={duration}
      step="1"
      value={position}
      oninput={(e) => onseek(Number(e.currentTarget.value))}
    />
    <div class="left mono">
      {#if meters?.finished}
        session complete
      {:else}
        {clock(remaining)} remaining
      {/if}
    </div>
  {:else}
    <div class="left">runs until you stop it</div>
  {/if}
</div>

<style>
  .play {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 4px;
    height: 100%;
    padding: 20px;
    overflow-y: auto;
  }
  .ring {
    position: relative;
    width: 280px;
    height: 280px;
    flex-shrink: 0;
  }
  svg {
    width: 100%;
    height: 100%;
  }
  .track {
    fill: none;
    stroke: var(--line);
    stroke-width: 3;
  }
  .fill {
    fill: none;
    stroke: var(--accent);
    stroke-width: 3;
    stroke-linecap: round;
    transition: stroke-dasharray 0.3s linear, stroke 0.3s;
  }
  .fill.idle {
    stroke: var(--line-hi);
  }
  .center {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 4px;
  }
  .hz {
    font-size: 46px;
    font-variant-numeric: tabular-nums;
    line-height: 1;
    letter-spacing: -0.02em;
  }
  .hz small {
    font-size: 15px;
    color: var(--muted);
    margin-left: 4px;
  }
  .band {
    font-size: 10px;
    letter-spacing: 0.18em;
    text-transform: uppercase;
    color: var(--accent);
  }
  .elapsed {
    font-size: 12px;
    color: var(--muted);
    margin-top: 6px;
  }
  .of {
    opacity: 0.6;
  }

  h1 {
    font-size: 19px;
    margin: 14px 0 0;
    font-weight: 600;
  }
  .desc {
    font-size: 12px;
    color: var(--muted);
    line-height: 1.6;
    max-width: 46ch;
    text-align: center;
    margin: 6px 0 0;
  }

  .controls {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 18px;
  }
  .controls button {
    padding: 9px 18px;
    border-radius: 8px;
    border: 1px solid var(--line);
    background: var(--sunken);
    color: var(--fg);
    font-size: 12.5px;
    cursor: pointer;
  }
  .controls button:hover {
    border-color: var(--line-hi);
  }
  .big {
    min-width: 96px;
    border-color: var(--accent-dim) !important;
    color: var(--accent) !important;
  }
  .gain {
    width: 92px;
  }

  .scrub {
    width: min(440px, 100%);
    margin-top: 18px;
    accent-color: var(--accent);
  }
  .left {
    font-size: 11px;
    color: var(--muted);
    margin-top: 6px;
  }
  .mono {
    font-family: var(--mono);
  }
</style>
