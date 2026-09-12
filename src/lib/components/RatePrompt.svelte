<script lang="ts">
  /**
   * Asked once, right after a session ends.
   *
   * The rating is what turns the journal from a log into something useful, but
   * it has to be answerable in one click straight out of a meditation — hence
   * five dots, an optional note, and a dismiss that costs nothing.
   */
  interface Props {
    presetName: string;
    onrate: (rating: number, note: string | null) => void;
    ondismiss: () => void;
  }
  let { presetName, onrate, ondismiss }: Props = $props();

  let rating = $state(0);
  let note = $state("");

  const LABELS = ["", "nothing", "a little", "some", "good", "deep"];

  function submit() {
    if (rating === 0) return;
    onrate(rating, note.trim() || null);
  }
</script>

<div class="prompt" role="dialog" aria-label="How was that session?">
  <div class="row">
    <div class="ask">
      <span class="q">How was that?</span>
      <span class="which">{presetName}</span>
    </div>

    <div class="dots">
      {#each [1, 2, 3, 4, 5] as n (n)}
        <button
          class="dot"
          class:on={n <= rating}
          aria-label="{n} of 5 — {LABELS[n]}"
          title={LABELS[n]}
          onclick={() => {
            rating = n;
          }}
        ></button>
      {/each}
    </div>
    <span class="label">{rating > 0 ? LABELS[rating] : ""}</span>

    <input
      class="note"
      placeholder="Anything worth remembering? (optional)"
      bind:value={note}
      onkeydown={(e) => {
        if (e.key === "Enter") submit();
        if (e.key === "Escape") ondismiss();
      }}
    />

    <button class="save" disabled={rating === 0} onclick={submit}>Save</button>
    <button class="skip" onclick={ondismiss}>Not now</button>
  </div>
</div>

<style>
  .prompt {
    background: var(--panel);
    border: 1px solid var(--accent-dim);
    border-radius: 10px;
    padding: 10px 14px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
  }
  .ask {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .q {
    font-size: 12.5px;
    font-weight: 600;
  }
  .which {
    font-size: 10px;
    color: var(--muted);
  }
  .dots {
    display: flex;
    gap: 6px;
  }
  .dot {
    width: 17px;
    height: 17px;
    border-radius: 50%;
    border: 1px solid var(--line-hi);
    background: transparent;
    cursor: pointer;
    padding: 0;
  }
  .dot:hover {
    border-color: var(--accent);
  }
  .dot.on {
    background: var(--accent);
    border-color: var(--accent);
  }
  .label {
    font-size: 10.5px;
    color: var(--accent);
    min-width: 52px;
  }
  .note {
    flex: 1;
    min-width: 170px;
    font-size: 11.5px;
    padding: 6px 9px;
    border-radius: 6px;
    border: 1px solid var(--line);
    background: var(--sunken);
    color: var(--fg);
    outline: none;
  }
  .note:focus {
    border-color: var(--accent-dim);
  }
  button.save,
  button.skip {
    font-size: 11px;
    padding: 6px 12px;
    border-radius: 6px;
    border: 1px solid var(--line);
    background: var(--sunken);
    color: var(--fg);
    cursor: pointer;
  }
  .save {
    border-color: var(--accent-dim);
    color: var(--accent);
  }
  .save:disabled {
    opacity: 0.35;
    cursor: default;
  }
  .skip {
    color: var(--muted);
  }
</style>
