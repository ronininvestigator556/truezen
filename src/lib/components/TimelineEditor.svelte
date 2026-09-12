<script lang="ts">
  import NumberField from "./NumberField.svelte";
  import { clock, denormaliseIn, laneWindow, normaliseIn, quantise, rangeFor, valueAt } from "../curve";
  import type { Curve, LayerConfig, Timeline, Track } from "../types";

  interface Props {
    timeline: Timeline;
    layers: LayerConfig[];
    positionS: number;
    latchedTracks: number;
    /** Called with a complete replacement timeline; edits are atomic swaps. */
    onchange: (t: Timeline) => void;
    onseek: (seconds: number) => void;
    onrelease: (track: number) => void;
  }

  let { timeline, layers, positionS, latchedTracks, onchange, onseek, onrelease }: Props = $props();

  const LANE_H = 62;
  const PAD_Y = 8;
  /** Points used to draw one curve. Enough that an exponential ramp reads as a
   *  curve rather than a chain of straight segments. */
  const SAMPLES = 160;

  let selected = $state<{ track: number; point: number } | null>(null);
  /** Per-lane value window, fitted to each track's own data. */
  let windows = $state<{ lo: number; hi: number }[]>([]);
  let lane: HTMLDivElement | null = $state(null);
  let laneWidth = $state(800);

  let duration = $derived(
    timeline.duration_s ??
      Math.max(60, ...timeline.tracks.map((t) => t.points.at(-1)?.at_s ?? 0)),
  );

  $effect(() => {
    if (!lane) return;
    const ro = new ResizeObserver(([e]) => (laneWidth = e.contentRect.width));
    ro.observe(lane);
    return () => ro.disconnect();
  });

  $effect(() => {
    // Recompute whenever the curve changes, but never mid-drag: a window that
    // rescales under the cursor fights the hand moving the point.
    const tl = timeline;
    if (drag) return;
    windows = tl.tracks.map((t) =>
      laneWindow(
        t.points.map((p) => p.value),
        rangeFor(t.target),
      ),
    );
  });

  function windowFor(ti: number) {
    const r = rangeFor(timeline.tracks[ti].target);
    return windows[ti] ?? { lo: r.min, hi: r.max };
  }

  function trackName(t: Track): string {
    const r = rangeFor(t.target);
    if (t.target.kind === "master_gain") return "Master level";
    const layer = layers[t.target.index];
    return `${layer?.name || `Layer ${t.target.index + 1}`} — ${r.label}`;
  }

  function x(t: number): number {
    return (t / Math.max(duration, 1)) * laneWidth;
  }
  function y(v: number, ti: number): number {
    const r = rangeFor(timeline.tracks[ti].target);
    const w = windowFor(ti);
    const u = normaliseIn(v, w.lo, w.hi, r.log);
    return PAD_Y + (1 - Math.min(1, Math.max(0, u))) * (LANE_H - PAD_Y * 2);
  }

  /** Screen position back to a value, within this lane's window. */
  function valueAtY(clientY: number, top: number, ti: number): number {
    const r = rangeFor(timeline.tracks[ti].target);
    const w = windowFor(ti);
    const u = 1 - (clientY - top - PAD_Y) / (LANE_H - PAD_Y * 2);
    return quantise(denormaliseIn(u, w.lo, w.hi, r.log), r);
  }

  function axisLabel(v: number, ti: number): string {
    const r = rangeFor(timeline.tracks[ti].target);
    return v.toFixed(r.decimals).replace(/\.?0+$/, "") || "0";
  }

  /** Sampled path of the curve as the engine will evaluate it. */
  function path(track: Track, ti: number): string {
    const pts: string[] = [];
    for (let i = 0; i <= SAMPLES; i++) {
      const t = (i / SAMPLES) * duration;
      const v = valueAt(track, t);
      if (v === null) continue;
      pts.push(`${i === 0 ? "M" : "L"}${x(t).toFixed(1)},${y(v, ti).toFixed(1)}`);
    }
    return pts.join(" ");
  }

  function edit(mutate: (t: Timeline) => void) {
    // Deep clone: the timeline travels to the backend as one atomic swap, and
    // mutating the live object in place would make the two disagree.
    const next: Timeline = structuredClone($state.snapshot(timeline));
    mutate(next);
    for (const t of next.tracks) t.points.sort((a, b) => a.at_s - b.at_s);
    onchange(next);
  }

  // --- dragging ----------------------------------------------------------

  let drag = $state<{ track: number; point: number } | null>(null);

  function onPointerDown(e: PointerEvent, ti: number, pi: number) {
    (e.currentTarget as Element).setPointerCapture(e.pointerId);
    drag = { track: ti, point: pi };
    selected = { track: ti, point: pi };
    e.stopPropagation();
  }

  function onPointerMove(e: PointerEvent, ti: number) {
    if (!drag || drag.track !== ti) return;
    const svg = (e.currentTarget as SVGElement).getBoundingClientRect();
    const t = quantiseTime(((e.clientX - svg.left) / svg.width) * duration);
    const v = valueAtY(e.clientY, svg.top, ti);

    edit((next) => {
      const p = next.tracks[ti].points[drag!.point];
      // Shift constrains to the value axis, so a breakpoint's timing can be
      // adjusted without disturbing its value and vice versa.
      if (!e.shiftKey) p.at_s = t;
      if (!e.altKey) p.value = v;
    });
  }

  function quantiseTime(t: number): number {
    return Math.min(duration, Math.max(0, Math.round(t)));
  }

  function onPointerUp(e: PointerEvent) {
    if (!drag) return;
    (e.currentTarget as Element).releasePointerCapture?.(e.pointerId);
    drag = null;
  }

  function addPoint(e: MouseEvent, ti: number) {
    const svg = (e.currentTarget as SVGElement).getBoundingClientRect();
    const track = timeline.tracks[ti];
    const t = quantiseTime(((e.clientX - svg.left) / svg.width) * duration);
    const v = valueAtY(e.clientY, svg.top, ti);

    edit((next) => {
      next.tracks[ti].points.push({ at_s: t, value: v, curve: "exponential" });
    });
    // Re-find the point after sorting so the inspector shows what was added.
    const sorted = [...track.points.map((p) => p.at_s), t].sort((a, b) => a - b);
    selected = { track: ti, point: sorted.indexOf(t) };
  }

  function deleteSelected() {
    if (!selected) return;
    const track = timeline.tracks[selected.track];
    // A track with no breakpoints is invalid and the backend rejects it, so
    // the last point cannot be removed.
    if (track.points.length <= 1) return;
    const { track: ti, point: pi } = selected;
    edit((next) => next.tracks[ti].points.splice(pi, 1));
    selected = null;
  }

  function setCurve(c: Curve) {
    if (!selected) return;
    const { track: ti, point: pi } = selected;
    edit((next) => (next.tracks[ti].points[pi].curve = c));
  }

  function setField(key: "at_s" | "value", v: number) {
    if (!selected) return;
    const { track: ti, point: pi } = selected;
    edit((next) => (next.tracks[ti].points[pi][key] = v));
  }

  let current = $derived(
    selected ? timeline.tracks[selected.track]?.points[selected.point] : undefined,
  );
  let currentRange = $derived(
    selected ? rangeFor(timeline.tracks[selected.track].target) : null,
  );

  const CURVES: Curve[] = ["exponential", "linear", "smoothstep", "hold"];
</script>

<section class="editor">
  <header>
    <h3>Session timeline</h3>
    <span class="hint">
      Drag a breakpoint to move it, double-click a lane to add one. Shift locks the time, Alt locks
      the value.
    </span>
  </header>

  {#if current && currentRange && selected}
    <div class="inspector">
      <span class="which">{trackName(timeline.tracks[selected.track])}</span>
      <NumberField
        label="At"
        unit="s"
        value={current.at_s}
        min={0}
        max={duration}
        step={1}
        decimals={0}
        onchange={(v) => setField("at_s", v)}
      />
      <NumberField
        label="Value"
        unit={currentRange.unit}
        value={current.value}
        min={currentRange.min}
        max={currentRange.max}
        step={currentRange.step}
        decimals={currentRange.decimals}
        log={currentRange.log}
        onchange={(v) => setField("value", v)}
      />
      <div class="curves">
        <span class="clabel">Approach</span>
        <div class="cbuttons">
          {#each CURVES as c (c)}
            <button class:on={current.curve === c} onclick={() => setCurve(c)}>{c}</button>
          {/each}
        </div>
      </div>
      <button
        class="del"
        disabled={timeline.tracks[selected.track].points.length <= 1}
        title={timeline.tracks[selected.track].points.length <= 1
          ? "A track needs at least one breakpoint"
          : "Delete this breakpoint"}
        onclick={deleteSelected}>Delete</button
      >
    </div>
  {/if}

  <div class="lanes" bind:this={lane}>
    {#each timeline.tracks as track, ti (ti)}
      {@const latched = (latchedTracks & (1 << ti)) !== 0}
      <div class="lane" class:latched>
        <div class="lhead">
          <span class="lname">{trackName(track)}</span>
          {#if latched}
            <button class="latch" title="Overridden by hand. Click to hand it back." onclick={() => onrelease(ti)}>
              manual
            </button>
          {/if}
        </div>

        <svg
          role="presentation"
          height={LANE_H}
          viewBox="0 0 {laneWidth} {LANE_H}"
          preserveAspectRatio="none"
          ondblclick={(e) => addPoint(e, ti)}
          onpointermove={(e) => onPointerMove(e, ti)}
          onpointerup={onPointerUp}
          onpointercancel={onPointerUp}
        >
          <line class="grid" x1="0" y1={LANE_H / 2} x2={laneWidth} y2={LANE_H / 2} />
          <path class="curve" d={path(track, ti)} vector-effect="non-scaling-stroke" />

          <!-- The lane is fitted to its own data, so it must say what it shows
               or the shape is unreadable. -->
          <text class="axis" x="4" y="10">{axisLabel(windowFor(ti).hi, ti)}</text>
          <text class="axis" x="4" y={LANE_H - 3}>{axisLabel(windowFor(ti).lo, ti)}</text>

          {#each track.points as p, pi (pi)}
            <circle
              class="bp"
              class:sel={selected?.track === ti && selected?.point === pi}
              cx={x(p.at_s)}
              cy={y(p.value, ti)}
              r="5"
              role="button"
              tabindex="0"
              onpointerdown={(e) => onPointerDown(e, ti, pi)}
              onkeydown={(e) => {
                if (e.key === "Enter" || e.key === " ") selected = { track: ti, point: pi };
                if (e.key === "Backspace" || e.key === "Delete") {
                  selected = { track: ti, point: pi };
                  deleteSelected();
                }
              }}
            />
          {/each}

          <line class="playhead" x1={x(positionS)} y1="0" x2={x(positionS)} y2={LANE_H} />
        </svg>
      </div>
    {/each}
  </div>

  <div class="ruler">
    <input
      type="range"
      min="0"
      max={Math.max(duration, 1)}
      step="1"
      value={positionS}
      oninput={(e) => onseek(Number(e.currentTarget.value))}
    />
    <div class="marks">
      <span class="mono">0:00</span>
      <span class="mono">{clock(duration / 2)}</span>
      <span class="mono">{clock(duration)}</span>
    </div>
  </div>
</section>

<style>
  .editor {
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 10px;
    padding: 12px 14px 14px;
  }
  header {
    display: flex;
    align-items: baseline;
    gap: 12px;
    margin-bottom: 10px;
  }
  h3 {
    font-size: 12px;
    margin: 0;
    font-weight: 600;
  }
  .hint {
    font-size: 10px;
    color: var(--muted);
  }

  .inspector {
    display: flex;
    align-items: flex-end;
    gap: 14px;
    padding: 10px 12px;
    margin-bottom: 12px;
    background: var(--sunken);
    border: 1px solid var(--line);
    border-radius: 8px;
    flex-wrap: wrap;
  }
  .which {
    font-size: 11px;
    color: var(--accent);
    align-self: center;
    min-width: 120px;
  }
  .inspector :global(.field) {
    width: 96px;
  }
  .curves {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .clabel {
    font-size: 10px;
    letter-spacing: 0.07em;
    text-transform: uppercase;
    color: var(--muted);
  }
  .cbuttons {
    display: flex;
    gap: 3px;
  }
  .cbuttons button,
  .del {
    font-size: 10px;
    padding: 5px 8px;
    border-radius: 5px;
    border: 1px solid var(--line);
    background: var(--panel);
    color: var(--muted);
    cursor: pointer;
  }
  .cbuttons button.on {
    color: var(--accent);
    border-color: var(--accent-dim);
    background: var(--accent-bg);
  }
  .del {
    color: var(--danger);
    border-color: var(--line-hi);
  }
  .del:disabled {
    opacity: 0.35;
    cursor: default;
  }

  .lanes {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .lane {
    display: grid;
    grid-template-columns: 170px 1fr;
    gap: 10px;
    align-items: center;
  }
  .lhead {
    display: flex;
    align-items: center;
    gap: 6px;
    justify-content: space-between;
  }
  .lname {
    font-size: 10.5px;
    color: var(--fg-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .latch {
    font-size: 8.5px;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    padding: 1px 5px;
    border-radius: 999px;
    border: 1px solid var(--warn-dim);
    background: transparent;
    color: var(--warn);
    cursor: pointer;
    flex-shrink: 0;
  }

  svg {
    width: 100%;
    display: block;
    background: var(--sunken);
    border: 1px solid var(--line);
    border-radius: 6px;
    touch-action: none;
  }
  .latched svg {
    border-color: var(--warn-dim);
    opacity: 0.55;
  }
  .grid {
    stroke: var(--line);
    stroke-width: 1;
  }
  .curve {
    fill: none;
    stroke: var(--accent);
    stroke-width: 1.5;
  }
  .bp {
    fill: var(--panel);
    stroke: var(--accent);
    stroke-width: 2;
    cursor: grab;
  }
  .bp:hover {
    fill: var(--accent);
  }
  .bp.sel {
    fill: var(--accent);
    stroke: var(--fg);
  }
  .axis {
    fill: var(--muted);
    font-family: var(--mono);
    font-size: 8px;
    opacity: 0.7;
  }
  .playhead {
    stroke: var(--violet);
    stroke-width: 1;
    opacity: 0.85;
  }

  .ruler {
    margin-top: 10px;
    padding-left: 180px;
  }
  .ruler input {
    width: 100%;
    accent-color: var(--violet);
  }
  .marks {
    display: flex;
    justify-content: space-between;
    font-size: 9.5px;
    color: var(--muted);
  }
  .mono {
    font-family: var(--mono);
  }
</style>
