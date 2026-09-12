<script lang="ts">
  import type { PresetSource } from "../types";

  interface Props {
    name: string;
    source: PresetSource | null;
    dirty: boolean;
    libraryDir: string;
    onsave: () => void;
    onsaveas: (name: string) => void;
    ondelete: () => void;
    onexport: () => void;
    onimport: () => void;
    onrender: (seconds: number) => void;
    /** Render progress 0..1, or null when not rendering. */
    rendering: number | null;
    /** The preset's own length in seconds; 0 when it runs open-ended. */
    durationS: number;
  }
  let {
    name,
    source,
    dirty,
    libraryDir,
    onsave,
    onsaveas,
    ondelete,
    onexport,
    onimport,
    onrender,
    rendering,
    durationS,
  }: Props = $props();

  let choosingLength = $state(false);
  let minutes = $state(30);

  // 24-bit stereo at 48 kHz. Uncompressed audio is big, and a 45-minute
  // session is most of a gigabyte -- worth saying before the save dialog
  // rather than after the file lands.
  const BYTES_PER_SECOND = 48_000 * 2 * 3;

  let estimate = $derived(() => {
    const bytes = minutes * 60 * BYTES_PER_SECOND;
    const gb = bytes / 1_000_000_000;
    return gb >= 1 ? `${gb.toFixed(1)} GB` : `${Math.round(bytes / 1_000_000)} MB`;
  });

  function beginRender() {
    minutes = durationS > 0 ? Math.round(durationS / 60) : 30;
    choosingLength = true;
  }

  function confirmRender() {
    choosingLength = false;
    onrender(Math.max(1, minutes) * 60);
  }

  let naming = $state(false);
  let draft = $state("");
  let input: HTMLInputElement | null = $state(null);

  function beginSaveAs() {
    // Default to a distinct name so a hurried Enter cannot collide with the
    // preset already loaded.
    draft = source === "factory" ? name : `${name} copy`;
    naming = true;
    queueMicrotask(() => input?.select());
  }

  function commit() {
    const n = draft.trim();
    naming = false;
    if (n) onsaveas(n);
  }

  const LABEL: Record<PresetSource, string> = {
    factory: "built in",
    user: "yours",
    override: "edited built-in",
  };
</script>

<div class="bar">
  {#if source}
    <span class="src src-{source}">{LABEL[source]}</span>
  {/if}
  {#if dirty}
    <span class="dirty" title="This preset has changes that are not saved">unsaved</span>
  {/if}

  <div class="spacer"></div>

  {#if choosingLength}
    <span class="prompt">Render how many minutes?</span>
    <input
      class="mins"
      type="number"
      min="1"
      max="480"
      bind:value={minutes}
      onkeydown={(e) => {
        if (e.key === "Enter") confirmRender();
        if (e.key === "Escape") choosingLength = false;
      }}
    />
    <span class="size">about {estimate()} as 24-bit WAV</span>
    <button onclick={confirmRender}>Choose a file…</button>
    <button onclick={() => (choosingLength = false)}>Cancel</button>
  {:else if naming}
    <input
      bind:this={input}
      class="namer"
      bind:value={draft}
      placeholder="Preset name"
      onblur={commit}
      onkeydown={(e) => {
        if (e.key === "Enter") commit();
        if (e.key === "Escape") naming = false;
      }}
    />
  {:else}
    <button onclick={onsave} disabled={!dirty} title={
      source === "factory"
        ? "Saves your changes as a copy that replaces the built-in one in the list"
        : "Save changes"
    }>Save</button>
    <button onclick={beginSaveAs}>Save as…</button>
    <button onclick={beginRender} disabled={rendering !== null}>
      {rendering === null ? "Render audio…" : `Rendering ${Math.round(rendering * 100)}%`}
    </button>
    <button onclick={onexport}>Export…</button>
    <button onclick={onimport}>Import…</button>
    {#if source === "user" || source === "override"}
      <button
        class="del"
        onclick={ondelete}
        title={source === "override"
          ? "Discard your edits and go back to the built-in preset"
          : "Delete this preset"}
      >
        {source === "override" ? "Revert" : "Delete"}
      </button>
    {/if}
  {/if}
</div>

{#if libraryDir}
  <p class="where">Your presets are files in {libraryDir}</p>
{/if}

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 9px 12px;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 10px;
  }
  .spacer {
    flex: 1;
  }
  .src,
  .dirty {
    font-size: 9px;
    letter-spacing: 0.07em;
    text-transform: uppercase;
    padding: 2px 7px;
    border-radius: 999px;
    background: var(--sunken);
    color: var(--muted);
  }
  .src-user {
    color: var(--accent);
    background: var(--accent-bg);
  }
  .src-override {
    color: var(--violet);
    background: var(--violet-bg);
  }
  .dirty {
    color: var(--warn);
    border: 1px solid var(--warn-dim);
    background: transparent;
  }
  button {
    font-size: 11px;
    padding: 5px 11px;
    border-radius: 6px;
    border: 1px solid var(--line);
    background: var(--sunken);
    color: var(--fg);
    cursor: pointer;
  }
  button:hover:not(:disabled) {
    border-color: var(--line-hi);
  }
  button:disabled {
    opacity: 0.35;
    cursor: default;
  }
  .del {
    color: var(--danger);
  }
  .prompt {
    font-size: 11px;
    color: var(--fg-dim);
  }
  .mins {
    width: 62px;
    font-family: var(--mono);
    font-size: 12px;
    padding: 5px 8px;
    border-radius: 6px;
    border: 1px solid var(--accent);
    background: var(--sunken);
    color: var(--fg);
    outline: none;
  }
  .size {
    font-size: 10.5px;
    color: var(--muted);
  }
  .namer {
    flex: 1;
    max-width: 280px;
    font-size: 12px;
    padding: 5px 9px;
    border-radius: 6px;
    border: 1px solid var(--accent);
    background: var(--sunken);
    color: var(--fg);
    outline: none;
  }
  .where {
    font-size: 9.5px;
    color: var(--muted);
    margin: 5px 2px 0;
    font-family: var(--mono);
    word-break: break-all;
  }
</style>
