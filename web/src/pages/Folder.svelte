<script lang="ts">
  import MediaGrid from '../components/MediaGrid.svelte';
  import Thumb from '../components/Thumb.svelte';
  import Viewer from '../components/Viewer.svelte';
  import { get } from '../lib/api';
  import { PagedList, type FolderDetail, type MediaItem } from '../lib/media.svelte';
  import { link, navigate, onVisible, route } from '../lib/router.svelte';

  let { id }: { id: number } = $props();

  let folder: FolderDetail | null = $state(null);
  let error = $state('');
  // Recreated whenever the folder changes.
  const media = $derived(new PagedList<MediaItem>(`/folders/${id}/media`, 200));

  $effect(() => {
    const folderId = id;
    folder = null;
    error = '';
    get<FolderDetail>(`/folders/${folderId}`)
      .then((f) => {
        if (folderId === id) folder = f;
      })
      .catch((e) => (error = (e as Error).message));
  });

  // The open item is in the URL (?m=<media id>) so Back closes the viewer.
  const openId = $derived(Number(route.query.get('m')) || null);
  const openIndex = $derived(openId === null ? -1 : media.items.findIndex((m) => m.id === openId));

  // Opened from a link to an item that isn't loaded yet: keep loading until it is.
  $effect(() => {
    if (openId !== null && openIndex < 0 && !media.done && !media.loading && !media.error) media.more();
  });

  function open(index: number) {
    navigate(`/folder/${id}?m=${media.items[index].id}`);
  }

  function show(index: number) {
    navigate(`/folder/${id}?m=${media.items[index].id}`, true);
  }

  function close() {
    history.length > 1 ? history.back() : navigate(`/folder/${id}`, true);
  }
</script>

<section class="page wide">
  {#if error}
    <p class="error">{error}</p>
  {:else if folder}
    <nav class="crumbs small" aria-label="Breadcrumbs">
      <a href="/" onclick={link}>Photos</a>
      {#each folder.breadcrumbs as crumb (crumb.id)}
        <span>/</span><a href="/folder/{crumb.id}" onclick={link}>{crumb.name}</a>
      {/each}
    </nav>
    <h2>{folder.name}</h2>
    <p class="muted small">{folder.media_count.toLocaleString()} item{folder.media_count === 1 ? '' : 's'}</p>

    {#if folder.subfolders.length}
      <div class="subfolders">
        {#each folder.subfolders as sub (sub.id)}
          <a class="subfolder" href="/folder/{sub.id}" onclick={link}>
            <div class="cover"><Thumb media={sub.cover} /></div>
            <div>
              <div class="name">{sub.name}</div>
              <div class="muted small">{sub.count.toLocaleString()}</div>
            </div>
          </a>
        {/each}
      </div>
    {/if}
  {/if}

  <MediaGrid items={media.items} onopen={open} />
  {#if media.error}<p class="error">{media.error}</p>{/if}
  <div use:onVisible={() => media.more()}></div>
</section>

{#if openIndex >= 0}
  <Viewer
    items={media.items}
    index={openIndex}
    total={media.total ?? media.items.length}
    onindex={show}
    onclose={close}
    onneedmore={() => media.more()} />
{/if}

<style>
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

  .crumbs a:hover {
    color: var(--text);
  }

  .subfolders {
    display: flex;
    gap: 10px;
    overflow-x: auto;
    padding: 4px 0 16px;
    margin-bottom: 8px;
  }

  .subfolder {
    flex: none;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 14px 6px 6px;
    border: 1px solid var(--border);
    border-radius: 12px;
    background: var(--surface);
    color: var(--text);
    text-decoration: none;
    max-width: 260px;
  }

  .subfolder:hover {
    border-color: var(--accent);
  }

  .subfolder .cover {
    width: 48px;
    height: 48px;
    border-radius: 8px;
    overflow: hidden;
    flex: none;
  }

  .name {
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
