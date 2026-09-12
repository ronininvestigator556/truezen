<script lang="ts">
  /**
   * A drag-scrub numeric field that is also a text input.
   *
   * Dialing an exact frequency is the app's headline requirement, so typing
   * `7.83` has to be as easy as dragging to it. Click to type, drag to scrub,
   * Shift for fine steps and Alt for coarse.
   */
  interface Props {
    value: number;
    label: string;
    unit?: string;
    min?: number;
    max?: number;
    /** Base step for one arrow-key press and the finest drag increment. */
    step?: number;
    decimals?: number;
    /** Scrub geometrically. Correct for frequency, where perception is
     *  logarithmic and a linear drag across 20-2000 Hz feels lopsided. */
    log?: boolean;
    disabled?: boolean;
    /** Shown when this value is overriding an automation track. */
    latched?: boolean;
    onchange: (v: number) => void;
    /** Hand the parameter back to its timeline track. */
    onrelease?: () => void;
  }

  let {
    value,
    label,
    unit = "",
    min = 0,
    max = 100,
    step = 0.01,
    decimals = 2,
    log = false,
    disabled = false,
    latched = false,
    onchange,
    onrelease,
  }: Props = $props();

  /** Typed shortcuts for frequencies that are awkward to remember. */
  const CONSTANTS: Record<string, number> = {
    schumann: 7.83,
    om: 136.1,
    a440: 440,
    solfeggio: 528,
    gamma: 40,
  };

  let editing = $state(false);
  let draft = $state("");
  let dragging = $state(false);
  let input: HTMLInputElement | null = $state(null);

  let display = $derived(format(value, decimals));

  /**
   * Trim trailing zeros, but only past a decimal point.
   *
   * Trimming unconditionally eats zeros off whole numbers -- 1200 renders as
   * "12" and a 2000 Hz cutoff as "2" -- which silently misreports the value.
   */
  function format(v: number, dp: number): string {
    const fixed = v.toFixed(dp);
    if (!fixed.includes(".")) return fixed;
    return fixed.replace(/\.?0+$/, "") || "0";
  }

  function clamp(v: number) {
    return Math.min(max, Math.max(min, v));
  }

  function commit(raw: string) {
    const key = raw.trim().toLowerCase();
    const named = CONSTANTS[key];
    const parsed = named ?? Number.parseFloat(key);
    if (Number.isFinite(parsed)) onchange(clamp(parsed));
    editing = false;
  }

  function beginEdit() {
    if (disabled) return;
    draft = display;
    editing = true;
    // Wait for the input to exist before selecting its contents.
    queueMicrotask(() => input?.select());
  }

  // --- drag scrubbing ----------------------------------------------------

  let startX = 0;
  let startValue = 0;
  let moved = 0;

  /** Pixels of travel to cross the whole range at normal sensitivity. */
  const SPAN_PX = 420;

  function sensitivity(e: PointerEvent | KeyboardEvent) {
    if (e.shiftKey) return 0.12;
    if (e.altKey) return 4;
    return 1;
  }

  function onPointerDown(e: PointerEvent) {
    if (disabled || editing) return;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    dragging = true;
    startX = e.clientX;
    startValue = value;
    moved = 0;
    e.preventDefault();
  }

  function onPointerMove(e: PointerEvent) {
    if (!dragging) return;
    const dx = e.clientX - startX;
    moved = Math.max(moved, Math.abs(dx));
    const frac = (dx / SPAN_PX) * sensitivity(e);

    let next: number;
    if (log && min > 0) {
      // Constant ratio per pixel, so an octave costs the same travel wherever
      // you are in the range.
      const decades = Math.log10(max / min);
      next = startValue * Math.pow(10, frac * decades);
    } else {
      next = startValue + frac * (max - min);
    }
    onchange(clamp(round(next)));
  }

  function onPointerUp(e: PointerEvent) {
    if (!dragging) return;
    dragging = false;
    (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
    // A press that did not travel is a click, and a click means "let me type".
    if (moved < 3) beginEdit();
  }

  function round(v: number) {
    const q = Math.round(v / step) * step;
    // Re-round to kill the float noise that dividing by a decimal step leaves.
    return Number.parseFloat(q.toFixed(6));
  }

  function onKeyDown(e: KeyboardEvent) {
    if (disabled) return;
    const dir = e.key === "ArrowUp" ? 1 : e.key === "ArrowDown" ? -1 : 0;
    if (dir !== 0) {
      e.preventDefault();
      const mult = e.shiftKey ? 1 : e.altKey ? 100 : 10;
      onchange(clamp(round(value + dir * step * mult)));
    } else if (e.key === "Enter") {
      beginEdit();
    }
  }
</script>

<div class="field" class:disabled class:latched>
  <div class="row">
    <span class="label">{label}</span>
    {#if latched}
      <button
        class="latch"
        title="Overriding the session timeline. Click to hand this parameter back."
        onclick={() => onrelease?.()}>manual</button
      >
    {/if}
  </div>

  {#if editing}
    <input
      bind:this={input}
      class="entry"
      type="text"
      bind:value={draft}
      onblur={() => commit(draft)}
      onkeydown={(e) => {
        if (e.key === "Enter") commit(draft);
        if (e.key === "Escape") editing = false;
      }}
    />
  {:else}
    <div
      class="scrub"
      class:dragging
      role="spinbutton"
      tabindex={disabled ? -1 : 0}
      aria-label={label}
      aria-valuenow={value}
      aria-valuemin={min}
      aria-valuemax={max}
      onpointerdown={onPointerDown}
      onpointermove={onPointerMove}
      onpointerup={onPointerUp}
      onpointercancel={onPointerUp}
      onkeydown={onKeyDown}
    >
      <span class="value">{display}</span>
      {#if unit}<span class="unit">{unit}</span>{/if}
    </div>
  {/if}
</div>

<style>
  .field {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
  }
  .label {
    font-size: 10px;
    letter-spacing: 0.07em;
    text-transform: uppercase;
    color: var(--muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .latch {
    font-size: 9px;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    padding: 1px 5px;
    border-radius: 999px;
    border: 1px solid var(--warn-dim);
    background: transparent;
    color: var(--warn);
    cursor: pointer;
  }
  .latch:hover {
    background: var(--warn-dim);
  }

  .scrub,
  .entry {
    font-family: var(--mono);
    font-size: 15px;
    font-variant-numeric: tabular-nums;
    padding: 5px 8px;
    border-radius: 6px;
    border: 1px solid var(--line);
    background: var(--sunken);
    color: var(--fg);
    width: 100%;
  }
  .scrub {
    display: flex;
    align-items: baseline;
    gap: 4px;
    cursor: ew-resize;
    user-select: none;
    touch-action: none;
  }
  .scrub:hover,
  .scrub:focus-visible {
    border-color: var(--accent-dim);
    outline: none;
  }
  .scrub.dragging {
    border-color: var(--accent);
    background: var(--sunken-hi);
  }
  .latched .scrub {
    border-color: var(--warn-dim);
  }
  .value {
    flex: 1;
  }
  .unit {
    font-size: 11px;
    color: var(--muted);
  }
  .entry {
    outline: none;
    border-color: var(--accent);
  }
  .disabled {
    opacity: 0.4;
    pointer-events: none;
  }
</style>
