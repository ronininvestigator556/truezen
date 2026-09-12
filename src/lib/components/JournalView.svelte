<script lang="ts">
  import type { PresetStat, SessionRow } from "../types";

  interface Props {
    sessions: SessionRow[];
    stats: PresetStat[];
    onreplay: (id: number) => void;
    ondiscard: (id: number) => void;
  }
  let { sessions, stats, onreplay, ondiscard }: Props = $props();

  function duration(s: number): string {
    const m = Math.round(s / 60);
    if (m < 60) return `${m}m`;
    return `${Math.floor(m / 60)}h ${m % 60}m`;
  }

  function when(unix: number): string {
    const d = new Date(unix * 1000);
    const days = Math.floor((Date.now() - d.getTime()) / 86_400_000);
    const time = d.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
    if (days === 0) return `Today ${time}`;
    if (days === 1) return `Yesterday ${time}`;
    if (days < 7) return `${d.toLocaleDateString(undefined, { weekday: "long" })} ${time}`;
    return d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
  }

  let totalTime = $derived(stats.reduce((a, s) => a + s.totalS, 0));
  let totalSessions = $derived(stats.reduce((a, s) => a + s.sessions, 0));
</script>

<div class="journal">
  {#if sessions.length === 0}
    <div class="empty">
      <p>No sessions yet.</p>
      <p class="fine">
        Anything you listen to for more than a minute is recorded here, with the preset exactly as
        it was at the time. Rate a few and this becomes a picture of what actually works for you
        rather than what the descriptions claim.
      </p>
    </div>
  {:else}
    <div class="summary">
      <div><span class="big mono">{totalSessions}</span><span class="cap">sessions</span></div>
      <div><span class="big mono">{duration(totalTime)}</span><span class="cap">listened</span></div>
    </div>

    <section>
      <h3>By preset</h3>
      <table>
        <thead>
          <tr>
            <th>Preset</th>
            <th class="n">Sessions</th>
            <th class="n">Time</th>
            <th class="n">Finished</th>
            <th class="n">Rating</th>
          </tr>
        </thead>
        <tbody>
          {#each stats as s (s.presetId)}
            <tr>
              <td>{s.presetName}</td>
              <td class="n mono">{s.sessions}</td>
              <td class="n mono">{duration(s.totalS)}</td>
              <td class="n mono">{s.completed}/{s.sessions}</td>
              <td class="n mono">
                {#if s.avgRating !== null}
                  <span class="rate">{s.avgRating.toFixed(1)}</span>
                  <span class="of">from {s.rated}</span>
                {:else}
                  <span class="of">not rated</span>
                {/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </section>

    <section>
      <h3>Recent</h3>
      <ul>
        {#each sessions as s (s.id)}
          <li>
            <div class="head">
              <span class="name">{s.presetName}</span>
              <span class="meta mono">{when(s.startedAt)} · {duration(s.listenedS)}</span>
              {#if s.completed}<span class="tag">finished</span>{/if}
              {#if s.rating !== null}
                <span class="dots" aria-label="rated {s.rating} of 5">
                  {#each [1, 2, 3, 4, 5] as n (n)}
                    <span class="dot" class:on={n <= s.rating!}></span>
                  {/each}
                </span>
              {/if}
              <div class="grow"></div>
              <button onclick={() => onreplay(s.id)} title="Load this session's settings exactly as they were">
                Load
              </button>
              <button class="del" onclick={() => ondiscard(s.id)} title="Remove from the journal">
                &times;
              </button>
            </div>
            {#if s.note}<p class="note">{s.note}</p>{/if}
          </li>
        {/each}
      </ul>
    </section>
  {/if}
</div>

<style>
  .journal {
    overflow-y: auto;
    padding-right: 6px;
    display: flex;
    flex-direction: column;
    gap: 22px;
  }
  .empty {
    padding: 40px 8px;
    color: var(--muted);
  }
  .empty .fine {
    font-size: 11.5px;
    max-width: 60ch;
    line-height: 1.65;
    margin-top: 10px;
  }
  .summary {
    display: flex;
    gap: 34px;
  }
  .summary div {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .big {
    font-size: 26px;
    line-height: 1;
  }
  .cap {
    font-size: 9.5px;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--muted);
  }
  h3 {
    font-size: 10px;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--muted);
    margin: 0 0 9px;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12px;
  }
  th {
    text-align: left;
    font-size: 9.5px;
    letter-spacing: 0.07em;
    text-transform: uppercase;
    color: var(--muted);
    font-weight: 600;
    padding: 0 10px 6px 0;
    border-bottom: 1px solid var(--line);
  }
  td {
    padding: 7px 10px 7px 0;
    border-bottom: 1px solid var(--line);
  }
  .n {
    text-align: right;
    padding-right: 0;
  }
  .rate {
    color: var(--accent);
  }
  .of {
    color: var(--muted);
    font-size: 10px;
    margin-left: 5px;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  li {
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 8px;
    padding: 9px 12px;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .grow {
    flex: 1;
  }
  .name {
    font-size: 12.5px;
    font-weight: 600;
  }
  .meta {
    font-size: 10.5px;
    color: var(--muted);
  }
  .tag {
    font-size: 8.5px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    padding: 2px 5px;
    border-radius: 999px;
    background: var(--accent-bg);
    color: var(--accent);
  }
  .dots {
    display: flex;
    gap: 3px;
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--line-hi);
  }
  .dot.on {
    background: var(--accent);
  }
  .head button {
    font-size: 10.5px;
    padding: 4px 9px;
    border-radius: 6px;
    border: 1px solid var(--line);
    background: var(--sunken);
    color: var(--fg);
    cursor: pointer;
  }
  .head button:hover {
    border-color: var(--line-hi);
  }
  .del {
    color: var(--muted) !important;
    padding: 4px 8px !important;
  }
  .del:hover {
    color: var(--danger) !important;
  }
  .note {
    font-size: 11.5px;
    color: var(--fg-dim);
    margin: 7px 0 0;
    line-height: 1.55;
  }
  .mono {
    font-family: var(--mono);
  }
</style>
