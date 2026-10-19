<script lang="ts">
  import MediaGrid from '../components/MediaGrid.svelte';
  import MoreSpace from '../components/MoreSpace.svelte';
  import MediaViewerHost from '../components/MediaViewerHost.svelte';
  import { PagedList, type MediaItem } from '../lib/media.svelte';
  import { openViewer } from '../lib/query';

  const favorites = new PagedList<MediaItem>('/favorites', 200);
</script>

<section class="page wide">
  <h2>Favorites</h2>
  {#if favorites.total === 0}
    <p class="muted">No favorites yet. Open a photo and tap the heart to add it here.</p>
  {:else if favorites.total !== null}
    <p class="muted small">{favorites.total.toLocaleString()} item{favorites.total === 1 ? '' : 's'}</p>
  {/if}
  <MediaGrid items={favorites.items} onopen={(i) => openViewer(favorites.items[i].id)} />
  <MoreSpace list={favorites} />
  {#if favorites.error}<p class="error">{favorites.error}</p>{/if}
</section>

<MediaViewerHost source={favorites} />
