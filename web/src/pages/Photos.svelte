<script lang="ts">
  import Thumb from '../components/Thumb.svelte';
  import { formatDate, PagedList, type Album } from '../lib/media.svelte';
  import { link, onVisible } from '../lib/router.svelte';

  const SORTS = [
    ['newest', 'Newest first'],
    ['oldest', 'Oldest first'],
    ['name', 'Name'],
    ['count', 'Most photos'],
  ] as const;
  type Sort = (typeof SORTS)[number][0];
  // Remembered per device, like the theme.
  const SORT_KEY = 'pixelgrove-folder-sort';

  const initialSort = readSort();
  let sort: Sort = $state(initialSort);
  let albums = $state(new PagedList<Album>(`/albums?sort=${initialSort}`, 60));

  function readSort(): Sort {
    try {
      const saved = localStorage.getItem(SORT_KEY);
      return SORTS.find(([id]) => id === saved)?.[0] ?? 'newest';
    } catch {
      return 'newest';
    }
  }

  function changeSort(event: Event) {
    // Read the choice from the menu: the binding may update after this handler.
    sort = (event.currentTarget as HTMLSelectElement).value as Sort;
    albums = new PagedList<Album>(`/albums?sort=${sort}`, 60);
    window.scrollTo({ top: 0 });
    // The end-of-list marker may already be in view, so it wouldn't ask by itself.
    albums.more();
    try {
      localStorage.setItem(SORT_KEY, sort);
    } catch {
      // Not remembered; fine.
    }
  }
</script>

<section class="page wide">
  <div class="heading">
    <h2>Photos</h2>
    <select class="sort" aria-label="Sort folders" value={sort} onchange={changeSort}>
      {#each SORTS as [id, label] (id)}
        <option value={id}>{label}</option>
      {/each}
    </select>
  </div>
  {#if albums.total === 0}
    <p class="muted">No photos yet. Add folders under <code>/media</code> and they'll appear here once scanned.</p>
  {/if}
  <div class="albums">
    {#each albums.items as album (album.id)}
      <a class="album" class:hidden-item={album.hidden} href="/folder/{album.id}" onclick={link} title={album.path}>
        <div class="cover"><Thumb media={album.cover} /></div>
        <div class="name">{#if album.hidden}<span class="tag">Hidden</span>{/if}{album.name}</div>
        <div class="meta muted small">
          {album.count.toLocaleString()} · {formatDate(album.latest_taken_at)}
        </div>
      </a>
    {/each}
  </div>
  {#if albums.error}<p class="error">{albums.error}</p>{/if}
  <div use:onVisible={() => albums.more()}></div>
</section>

<style>
  .heading {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .heading h2 {
    margin: 0;
  }

  .sort {
    margin: 0;
    padding: 7px 10px;
    font: inherit;
    font-size: 0.9rem;
    color: var(--text);
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 10px;
  }

  .albums {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(170px, 1fr));
    gap: 20px 14px;
    margin-top: 16px;
  }

  .album {
    color: var(--text);
    text-decoration: none;
    min-width: 0;
  }

  .cover {
    aspect-ratio: 1;
    border-radius: 14px;
    overflow: hidden;
    background: var(--surface-2);
  }

  .hidden-item .cover {
    opacity: 0.4;
  }

  .tag {
    font-size: 0.7rem;
    font-weight: 600;
    padding: 1px 6px;
    margin-right: 6px;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text-muted);
    vertical-align: 1px;
  }

  .album:hover .cover,
  .album:focus-visible .cover {
    outline: 3px solid var(--accent);
    outline-offset: -3px;
  }

  .name {
    margin-top: 8px;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .meta {
    margin-top: 2px;
  }

  @media (max-width: 520px) {
    .albums {
      /* Picked with the header's columns button (lib/columns.ts). */
      grid-template-columns: repeat(var(--folder-columns, 2), 1fr);
      gap: 14px 8px;
    }

    .cover {
      border-radius: 10px;
    }

    .name {
      margin-top: 6px;
      font-size: 0.9rem;
    }

    .meta {
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
    }
  }
</style>
