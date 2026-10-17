<script lang="ts">
  import Thumb from '../components/Thumb.svelte';
  import { get } from '../lib/api';
  import type { Cover } from '../lib/media.svelte';
  import { link } from '../lib/router.svelte';

  type Filter = Record<string, string | number>;
  type Group = { filter: Filter; label: string; detail: string | null; count: number; cover: Cover | null };
  type Explore = { places: Group[]; countries: Group[]; cameras: Group[]; years: Group[]; kinds: Group[] };

  let explore = $state<Explore | null>(null);
  let error = $state('');

  $effect(() => {
    get<Explore>('/explore').then(
      (e) => (explore = e),
      (e) => (error = (e as Error).message),
    );
  });

  function href(group: Group): string {
    const params = new URLSearchParams(Object.entries(group.filter).map(([k, v]) => [k, String(v)]));
    params.set('title', group.detail ? `${group.label}, ${group.detail}` : group.label);
    return `/explore/items?${params}`;
  }

  const sections = $derived(
    explore
      ? [
          { title: 'Places', groups: explore.places },
          { title: 'Countries', groups: explore.countries },
          { title: 'Cameras', groups: explore.cameras },
          { title: 'Years', groups: explore.years },
          { title: 'Photos & videos', groups: explore.kinds },
        ].filter((s) => s.groups.length)
      : [],
  );
</script>

<section class="page wide">
  <h2>Explore</h2>
  <p class="muted small">Your library by place, camera and year, from the photos' own metadata.</p>
  {#if error}<p class="error">{error}</p>{/if}
  {#if explore && !explore.places.length}
    <p class="muted small">Places appear here for photos with a GPS location.</p>
  {/if}
  {#each sections as section (section.title)}
    <h3>{section.title}</h3>
    <div class="row">
      {#each section.groups as group (href(group))}
        <a class="card" href={href(group)} onclick={link}>
          <div class="cover"><Thumb media={group.cover} /></div>
          <div class="label">{group.label}</div>
          <div class="muted small">{group.detail ? `${group.detail} · ` : ''}{group.count.toLocaleString()}</div>
        </a>
      {/each}
    </div>
  {/each}
</section>

<style>
  h3 {
    margin: 24px 0 10px;
    font-size: 1.05rem;
  }

  .row {
    display: grid;
    grid-auto-flow: column;
    grid-auto-columns: 150px;
    gap: 12px;
    overflow-x: auto;
    padding-bottom: 8px;
    scroll-snap-type: x proximity;
  }

  .card {
    color: var(--text);
    text-decoration: none;
    scroll-snap-align: start;
    min-width: 0;
  }

  .cover {
    aspect-ratio: 1;
    border-radius: 14px;
    overflow: hidden;
    background: var(--surface-2);
  }

  .card:hover .cover,
  .card:focus-visible .cover {
    outline: 3px solid var(--accent);
    outline-offset: -3px;
  }

  .label {
    margin-top: 6px;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  @media (max-width: 520px) {
    .row {
      grid-auto-columns: 120px;
    }
  }
</style>
