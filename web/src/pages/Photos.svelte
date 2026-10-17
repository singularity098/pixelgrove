<script lang="ts">
  import Thumb from '../components/Thumb.svelte';
  import { formatDate, PagedList, type Album } from '../lib/media.svelte';
  import { link, onVisible } from '../lib/router.svelte';

  const albums = new PagedList<Album>('/albums', 60);
</script>

<section class="page wide">
  <h2>Photos</h2>
  {#if albums.total === 0}
    <p class="muted">No photos yet. Add folders under <code>/media</code> and they'll appear here once scanned.</p>
  {/if}
  <div class="albums">
    {#each albums.items as album (album.id)}
      <a class="album" href="/folder/{album.id}" onclick={link} title={album.path}>
        <div class="cover"><Thumb media={album.cover} /></div>
        <div class="name">{album.name}</div>
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
      grid-template-columns: repeat(2, 1fr);
      gap: 16px 10px;
    }
  }
</style>
