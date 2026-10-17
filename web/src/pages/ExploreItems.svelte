<script lang="ts">
  import MediaGrid from '../components/MediaGrid.svelte';
  import MediaViewerHost from '../components/MediaViewerHost.svelte';
  import { PagedList, type MediaItem } from '../lib/media.svelte';
  import { openViewer } from '../lib/query';
  import { link, onVisible } from '../lib/router.svelte';

  /** Items for one Explore group; the filter comes from the URL query. */
  let { query }: { query: URLSearchParams } = $props();

  // Keyed on the filter by the parent, so created once per group.
  // svelte-ignore state_referenced_locally
  const title = query.get('title') ?? 'Explore';
  // svelte-ignore state_referenced_locally
  const filter = new URLSearchParams([...query].filter(([k]) => ['country', 'place', 'make', 'model', 'year', 'kind'].includes(k)));
  const items = new PagedList<MediaItem>(`/explore/items?${filter}`, 200);
</script>

<section class="page wide">
  <nav class="small"><a href="/explore" onclick={link}>Explore</a></nav>
  <h2>{title}</h2>
  {#if items.total !== null}
    <p class="muted small">{items.total.toLocaleString()} item{items.total === 1 ? '' : 's'}</p>
  {/if}
  <MediaGrid items={items.items} onopen={(i) => openViewer(items.items[i].id)} />
  {#if items.error}<p class="error">{items.error}</p>{/if}
  <div use:onVisible={() => items.more()}></div>
</section>

<MediaViewerHost source={items} />

<style>
  nav a {
    color: var(--text-muted);
    text-decoration: none;
  }
</style>
