<script lang="ts">
  import MediaGrid from '../components/MediaGrid.svelte';
  import MediaViewerHost from '../components/MediaViewerHost.svelte';
  import { get } from '../lib/api';
  import type { MediaItem } from '../lib/media.svelte';
  import { openViewer } from '../lib/query';
  import { onVisible } from '../lib/router.svelte';
  import { endOfMonth, monthLabel, Timeline, type Month } from '../lib/timeline.svelte';

  const timeline = new Timeline();
  let months: Month[] = $state([]);
  let activeMonth: string | null = $state(null);

  $effect(() => {
    get<Month[]>('/timeline/months').then((m) => (months = m));
  });

  // Group consecutive items by day (capture times are wall-clock, stored as UTC).
  type Day = { key: string; label: string; start: number; items: MediaItem[] };
  const days = $derived.by(() => {
    const groups: Day[] = [];
    timeline.items.forEach((item, index) => {
      const date = new Date(item.taken_at * 1000);
      const key = date.toISOString().slice(0, 10);
      if (groups.at(-1)?.key !== key) {
        groups.push({
          key,
          label: date.toLocaleDateString(undefined, { timeZone: 'UTC', weekday: 'short', month: 'short', day: 'numeric', year: 'numeric' }),
          start: index,
          items: [],
        });
      }
      groups.at(-1)!.items.push(item);
    });
    return groups;
  });

  const years = $derived.by(() => {
    const byYear = new Map<string, Month[]>();
    for (const month of months) {
      const year = month.month.slice(0, 4);
      byYear.set(year, [...(byYear.get(year) ?? []), month]);
    }
    return [...byYear];
  });

  function jump(month: string | null) {
    activeMonth = month;
    timeline.reset(month === null ? null : endOfMonth(month));
    window.scrollTo({ top: 0 });
    timeline.more();
  }
</script>

<div class="layout">
  <section class="page wide">
    <h2>Timeline</h2>
    {#if timeline.startedBefore !== null}
      <p class="muted small">
        Showing from {activeMonth ? monthLabel(activeMonth) : 'earlier'} back.
        <button class="link" onclick={() => jump(null)}>Back to newest</button>
      </p>
    {/if}
    {#each days as day (day.key)}
      <h3 class="day">{day.label}</h3>
      <MediaGrid items={day.items} onopen={(i) => openViewer(timeline.items[day.start + i].id)} />
    {/each}
    {#if !timeline.loading && timeline.items.length === 0 && !timeline.hasMore}
      <p class="muted">Nothing here yet.</p>
    {/if}
    {#if timeline.error}<p class="error">{timeline.error}</p>{/if}
    <div use:onVisible={() => timeline.more()}></div>
  </section>

  {#if months.length}
    <nav class="scrubber" aria-label="Jump to month">
      <select
        class="month-select"
        aria-label="Jump to month"
        value={activeMonth ?? ''}
        onchange={(e) => jump((e.currentTarget as HTMLSelectElement).value || null)}>
        <option value="">Newest</option>
        {#each months as month (month.month)}
          <option value={month.month}>{monthLabel(month.month)} ({month.count})</option>
        {/each}
      </select>
      <ol class="years">
        {#each years as [year, yearMonths] (year)}
          <li>
            <button class="year" onclick={() => jump(yearMonths[0].month)}>{year}</button>
            <ol>
              {#each yearMonths as month (month.month)}
                <li>
                  <button class:active={activeMonth === month.month} onclick={() => jump(month.month)} title="{month.count} items">
                    {monthLabel(month.month, true).split(' ')[0]}
                  </button>
                </li>
              {/each}
            </ol>
          </li>
        {/each}
      </ol>
    </nav>
  {/if}
</div>

<MediaViewerHost source={timeline} />

<style>
  .layout {
    display: flex;
    align-items: flex-start;
  }

  .layout > .page {
    flex: 1;
    min-width: 0;
  }

  .day {
    margin: 22px 0 8px;
    font-size: 0.95rem;
    font-weight: 600;
  }

  button.link {
    border: 0;
    padding: 0;
    background: none;
    color: var(--accent);
    cursor: pointer;
  }

  .scrubber {
    position: sticky;
    top: 56px;
    width: 120px;
    max-height: calc(100vh - 56px);
    overflow-y: auto;
    padding: 20px 8px;
    flex: none;
  }

  .month-select {
    display: none;
  }

  .years,
  .years ol {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .years > li {
    margin-bottom: 10px;
  }

  .scrubber button {
    display: block;
    width: 100%;
    text-align: right;
    border: 0;
    border-radius: 6px;
    padding: 2px 8px;
    background: none;
    color: var(--text-muted);
    font-size: 0.8rem;
  }

  .scrubber button.year {
    color: var(--text);
    font-weight: 600;
    font-size: 0.9rem;
  }

  .scrubber button:hover,
  .scrubber button.active {
    color: var(--accent);
    background: var(--surface-2);
  }

  @media (max-width: 700px) {
    .layout {
      flex-direction: column-reverse;
    }

    .scrubber {
      position: sticky;
      top: 56px;
      z-index: 5;
      width: 100%;
      max-height: none;
      padding: 8px 16px;
      background: var(--bg);
    }

    .years {
      display: none;
    }

    .month-select {
      display: block;
      width: 100%;
      padding: 8px;
      font: inherit;
      color: var(--text);
      background: var(--surface);
      border: 1px solid var(--border);
      border-radius: 10px;
    }
  }
</style>
