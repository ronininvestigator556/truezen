<script lang="ts">
  import type { PresetSummary } from "../types";

  interface Props {
    presets: PresetSummary[];
    selected: string | null;
    onselect: (id: string) => void;
  }
  let { presets, selected, onselect }: Props = $props();

  let query = $state("");

  let filtered = $derived(
    presets.filter((p) => {
      const q = query.trim().toLowerCase();
      if (!q) return true;
      return (
        p.name.toLowerCase().includes(q) ||
        p.goal.toLowerCase().includes(q) ||
        p.description.toLowerCase().includes(q)
      );
    }),
  );

  // Preserve the order the backend supplied within each goal.
  let grouped = $derived(
    filtered.reduce<Record<string, PresetSummary[]>>((acc, p) => {
      (acc[p.goal] ??= []).push(p);
      return acc;
    }, {}),
  );

  function duration(s: number) {
    if (s <= 0) return "open";
    return `${Math.round(s / 60)}m`;
  }
</script>

<aside>
  <input class="search" type="search" placeholder="Search presets" bind:value={query} />

  <div class="list">
    {#each Object.entries(grouped) as [goal, items] (goal)}
      <h3>{goal}</h3>
      {#each items as p (p.id)}
        <button class="preset" class:active={p.id === selected} onclick={() => onselect(p.id)}>
          <div class="top">
            <span class="name">{p.name}</span>
            <span class="dur">{duration(p.durationS)}</span>
          </div>
          <p class="desc">{p.description}</p>
          <div class="tags">
            <span class="tag">{p.layerCount} layers</span>
            {#if p.requiresHeadphones}
              <span class="tag hp" title="Binaural layers do not work on speakers">headphones</span>
            {/if}
          </div>
        </button>
      {/each}
    {/each}

    {#if filtered.length === 0}
      <p class="empty">No presets match &ldquo;{query}&rdquo;.</p>
    {/if}
  </div>
</aside>

<style>
  aside {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-height: 0;
  }
  .search {
    width: 100%;
    padding: 7px 10px;
    border-radius: 7px;
    border: 1px solid var(--line);
    background: var(--sunken);
    color: var(--fg);
    font-size: 12px;
  }
  .search:focus {
    outline: none;
    border-color: var(--accent-dim);
  }
  .list {
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 5px;
    padding-right: 4px;
  }
  h3 {
    font-size: 9px;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--muted);
    margin: 12px 0 2px;
    font-weight: 600;
  }
  h3:first-child {
    margin-top: 0;
  }
  .preset {
    display: block;
    width: 100%;
    text-align: left;
    padding: 9px 11px;
    border-radius: 8px;
    border: 1px solid transparent;
    background: var(--panel);
    color: var(--fg);
    cursor: pointer;
    transition: border-color 0.12s, background 0.12s;
  }
  .preset:hover {
    border-color: var(--line-hi);
  }
  .preset.active {
    border-color: var(--accent-dim);
    background: var(--accent-bg);
  }
  .top {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 8px;
  }
  .name {
    font-size: 12.5px;
    font-weight: 600;
  }
  .dur {
    font-size: 10px;
    font-family: var(--mono);
    color: var(--muted);
  }
  .desc {
    font-size: 10.5px;
    line-height: 1.45;
    color: var(--muted);
    margin: 4px 0 0;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .tags {
    display: flex;
    gap: 5px;
    margin-top: 6px;
  }
  .tag {
    font-size: 8.5px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    padding: 2px 5px;
    border-radius: 999px;
    background: var(--sunken);
    color: var(--muted);
  }
  .tag.hp {
    color: var(--violet);
    background: var(--violet-bg);
  }
  .empty {
    font-size: 11px;
    color: var(--muted);
    padding: 12px 4px;
  }
</style>
