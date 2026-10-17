<script lang="ts">
  import Icon from '../components/Icon.svelte';
  import MediaViewerHost from '../components/MediaViewerHost.svelte';
  import { get } from '../lib/api';
  import {
    formatBytes,
    formatDate,
    PagedList,
    type FolderDetail,
    type FolderHit,
    type MediaItem,
    type TreeFolder,
  } from '../lib/media.svelte';
  import { openViewer, setParam } from '../lib/query';
  import { link, onVisible, route } from '../lib/router.svelte';

  /** The folder being listed, or null for the top level. */
  let { id }: { id: number | null } = $props();

  let detail: FolderDetail | null = $state(null);
  let folders: TreeFolder[] = $state([]);
  let error = $state('');
  // Keyed on the folder id by the parent, so created once per folder.
  // svelte-ignore state_referenced_locally
  const media = id === null ? null : new PagedList<MediaItem>(`/folders/${id}/media`, 200);

  $effect(() => {
    if (id === null) {
      get<TreeFolder[]>('/files').then((f) => (folders = f), (e) => (error = e.message));
    } else {
      get<FolderDetail>(`/folders/${id}`).then((d) => (detail = d), (e) => (error = e.message));
      get<TreeFolder[]>(`/files/${id}`).then((f) => (folders = f), (e) => (error = e.message));
    }
  });

  // The filter lives in the URL (?filter=) so Back restores it.
  const filter = $derived(route.query.get('filter') ?? '');
  let hits: FolderHit[] | null = $state(null);
  let input = $state(route.query.get('filter') ?? '');
  let debounce: ReturnType<typeof setTimeout>;

  function onInput() {
    clearTimeout(debounce);
    debounce = setTimeout(() => setParam('filter', input.trim() || null, true), 200);
  }

  $effect(() => {
    const q = filter;
    if (!q) {
      hits = null;
      return;
    }
    get<FolderHit[]>(`/folders/filter?q=${encodeURIComponent(q)}&limit=200`).then((h) => {
      if (q === filter) hits = h;
    });
  });
</script>

<section class="page">
  <div class="heading">
    <div>
      <nav class="crumbs small" aria-label="Breadcrumbs">
        <a href="/files" onclick={link}>File View</a>
        {#if detail}
          {#each detail.breadcrumbs as crumb (crumb.id)}
            <span>/</span><a href="/files/{crumb.id}" onclick={link}>{crumb.name}</a>
          {/each}
        {/if}
      </nav>
      <h2>{detail?.name ?? 'File View'}</h2>
    </div>
    <input
      class="filter"
      type="search"
      placeholder="Filter folders…"
      aria-label="Filter folders"
      bind:value={input}
      oninput={onInput} />
  </div>
  {#if error}<p class="error">{error}</p>{/if}

  {#if hits !== null}
    <p class="muted small">{hits.length} folder{hits.length === 1 ? '' : 's'} matching “{filter}”</p>
    <ul class="rows">
      {#each hits as hit (hit.id)}
        <li>
          <a class="row" href="/files/{hit.id}" onclick={link}>
            <span class="icon"><Icon name="folder" /></span>
            <span class="main">
              <span class="name">{hit.name}</span>
              <span class="muted small path">{hit.path}</span>
            </span>
            <span class="muted small">{hit.count ? hit.count.toLocaleString() : ''}</span>
          </a>
        </li>
      {/each}
    </ul>
  {:else}
    <ul class="rows">
      {#each folders as folder (folder.id)}
        <li>
          <a class="row" href="/files/{folder.id}" onclick={link}>
            <span class="icon"><Icon name="folder" /></span>
            <span class="main">
              <span class="name">{folder.name}</span>
              <span class="muted small">
                {[
                  folder.subfolder_count ? `${folder.subfolder_count} folder${folder.subfolder_count === 1 ? '' : 's'}` : '',
                  `${folder.total_count.toLocaleString()} item${folder.total_count === 1 ? '' : 's'}`,
                ]
                  .filter(Boolean)
                  .join(' · ')}
              </span>
            </span>
            <span class="muted small date">{formatDate(folder.latest_taken_at)}</span>
          </a>
        </li>
      {/each}
      {#if media}
        {#each media.items as item, index (item.id)}
          <li>
            <button class="row" onclick={() => openViewer(media.items[index].id)}>
              <span class="icon"><Icon name={item.kind === 'video' ? 'play' : 'photos'} /></span>
              <span class="main"><span class="name">{item.name}</span></span>
              <span class="muted small">{formatBytes(item.size)}</span>
              <span class="muted small date">{formatDate(item.taken_at)}</span>
            </button>
          </li>
        {/each}
      {/if}
    </ul>
    {#if media}
      {#if media.error}<p class="error">{media.error}</p>{/if}
      <div use:onVisible={() => media.more()}></div>
    {/if}
    {#if detail && !folders.length && media?.total === 0}
      <p class="muted">This folder is empty.</p>
    {/if}
  {/if}
</section>

{#if media}<MediaViewerHost source={media} />{/if}

<style>
  .heading {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: 16px;
    flex-wrap: wrap;
    margin-bottom: 12px;
  }

  .crumbs {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    color: var(--text-muted);
    margin-bottom: 4px;
  }

  .crumbs a {
    color: var(--text-muted);
    text-decoration: none;
  }

  .filter {
    max-width: 280px;
    margin: 0;
  }

  .rows {
    list-style: none;
    padding: 0;
    margin: 0;
    border: 1px solid var(--border);
    border-radius: 12px;
    overflow: hidden;
    background: var(--surface);
  }

  .rows:empty {
    display: none;
  }

  .rows li + li {
    border-top: 1px solid var(--border);
  }

  .row {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    padding: 10px 14px;
    border: 0;
    border-radius: 0;
    background: none;
    color: var(--text);
    text-align: left;
    text-decoration: none;
    font: inherit;
  }

  .row:hover {
    background: var(--surface-2);
  }

  .icon {
    color: var(--text-muted);
    display: grid;
  }

  .main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }

  .name,
  .path {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .date {
    width: 100px;
    text-align: right;
    flex: none;
  }

  @media (max-width: 520px) {
    .filter {
      max-width: none;
    }

    .date {
      display: none;
    }
  }
</style>
