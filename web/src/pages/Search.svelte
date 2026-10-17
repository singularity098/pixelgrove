<script lang="ts">
  import MediaGrid from '../components/MediaGrid.svelte';
  import MediaViewerHost from '../components/MediaViewerHost.svelte';
  import Thumb from '../components/Thumb.svelte';
  import { openViewer } from '../lib/query';
  import { link, onVisible } from '../lib/router.svelte';
  import { SearchResults } from '../lib/search.svelte';

  let { query }: { query: string } = $props();
  // The page is keyed on the query, so this is created once per search.
  // svelte-ignore state_referenced_locally
  const results = new SearchResults(query);
</script>

<section class="page wide">
  <h2>Results for “{query}”</h2>
  {#if results.total !== null}
    <p class="muted small">
      {results.folders.length ? `${results.folders.length} folder${results.folders.length === 1 ? '' : 's'} · ` : ''}{results.total.toLocaleString()}
      item{results.total === 1 ? '' : 's'}
    </p>
  {/if}

  {#if results.folders.length}
    <h3>Folders</h3>
    <div class="folders">
      {#each results.folders as folder (folder.id)}
        <a class="folder" href="/folder/{folder.id}" onclick={link} title={folder.path}>
          <div class="cover"><Thumb media={folder.cover} /></div>
          <div class="text">
            <div class="name">{folder.name}</div>
            <div class="muted small path">{folder.path}</div>
          </div>
        </a>
      {/each}
    </div>
  {/if}

  {#if results.items.length}
    <h3>Photos and videos</h3>
    <MediaGrid items={results.items} onopen={(i) => openViewer(results.items[i].id)} />
  {:else if results.total === 0 && !results.folders.length}
    <p class="muted">Nothing matched. Search looks in folder and file names, dates (like 2023-07), cameras, lenses and descriptions.</p>
  {/if}
  {#if results.error}<p class="error">{results.error}</p>{/if}
  <div use:onVisible={() => results.more()}></div>
</section>

<MediaViewerHost source={results} />

<style>
  h3 {
    margin: 20px 0 10px;
    font-size: 1rem;
  }

  .folders {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
    gap: 10px;
  }

  .folder {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px;
    border: 1px solid var(--border);
    border-radius: 12px;
    background: var(--surface);
    color: var(--text);
    text-decoration: none;
    min-width: 0;
  }

  .folder:hover {
    border-color: var(--accent);
  }

  .cover {
    width: 52px;
    height: 52px;
    border-radius: 8px;
    overflow: hidden;
    flex: none;
  }

  .text {
    min-width: 0;
  }

  .name,
  .path {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .name {
    font-weight: 600;
  }
</style>
