<script lang="ts">
  import MediaGrid from '../components/MediaGrid.svelte';
  import Icon from '../components/Icon.svelte';
  import MediaViewerHost from '../components/MediaViewerHost.svelte';
  import ShareDialog from '../components/ShareDialog.svelte';
  import Thumb from '../components/Thumb.svelte';
  import { get } from '../lib/api';
  import { PagedList, type FolderDetail, type MediaItem } from '../lib/media.svelte';
  import { openViewer } from '../lib/query';
  import { link, onVisible } from '../lib/router.svelte';
  import { session } from '../lib/session.svelte';

  let { id }: { id: number } = $props();

  let folder: FolderDetail | null = $state(null);
  let error = $state('');
  let sharing = $state(false);
  const canShare = $derived(session.current.status === 'signed-in' && session.current.user.role !== 'viewer');
  // The page is keyed on the folder id, so this is created once per folder.
  // svelte-ignore state_referenced_locally
  const media = new PagedList<MediaItem>(`/folders/${id}/media`, 200);

  $effect(() => {
    get<FolderDetail>(`/folders/${id}`)
      .then((f) => (folder = f))
      .catch((e) => (error = (e as Error).message));
  });
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
    <div class="title">
      <h2>{folder.name}</h2>
      {#if canShare}
        <button class="ghost share" onclick={() => (sharing = true)}><Icon name="sharing" size={16} /> Share</button>
      {/if}
    </div>
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

  <MediaGrid items={media.items} onopen={(i) => openViewer(media.items[i].id)} />
  {#if media.error}<p class="error">{media.error}</p>{/if}
  <div use:onVisible={() => media.more()}></div>
</section>

<MediaViewerHost source={media} />

{#if sharing && folder}
  <ShareDialog target={{ folder_id: folder.id, name: folder.name }} onclose={() => (sharing = false)} />
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

  .title {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .share {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    flex: none;
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
