<script lang="ts">
  import MediaGrid from '../components/MediaGrid.svelte';
  import MoreSpace from '../components/MoreSpace.svelte';
  import Icon from '../components/Icon.svelte';
  import MediaViewerHost from '../components/MediaViewerHost.svelte';
  import ShareDialog from '../components/ShareDialog.svelte';
  import Slideshow from '../components/Slideshow.svelte';
  import Thumb from '../components/Thumb.svelte';
  import { del, get, put } from '../lib/api';
  import { PagedList, type FolderDetail, type MediaItem } from '../lib/media.svelte';
  import { openViewer } from '../lib/query';
  import { link, navigate } from '../lib/router.svelte';
  import { session } from '../lib/session.svelte';
  import { slideshowSettings } from '../lib/slideshow.svelte';

  let { id }: { id: number } = $props();

  let folder = $state<FolderDetail | null>(null);
  let error = $state('');
  let sharing = $state(false);
  const canShare = $derived(session.current.status === 'signed-in' && session.current.user.role !== 'viewer');
  // The page is keyed on the folder id, so this is created once per folder.
  // svelte-ignore state_referenced_locally
  const media = new PagedList<MediaItem>(`/folders/${id}/media`, 200);

  let slideshow = $state(false);
  const hasSubfolders = $derived(!!folder?.subfolders.length);
  const slides = $derived(
    slideshow && hasSubfolders && slideshowSettings.subfolders
      ? new PagedList<MediaItem>(`/folders/${id}/media?recursive=true`, 200)
      : media,
  );

  const loadFolder = () =>
    get<FolderDetail>(`/folders/${id}`)
      .then((f) => (folder = f))
      .catch((e) => (error = (e as Error).message));
  $effect(() => {
    loadFolder();
  });

  // Hiding: admins hide for everyone, others just for themselves.
  const me = $derived(session.current.status === 'signed-in' ? session.current.user : null);
  let hideBusy = $state(false);
  async function toggleHidden() {
    if (!folder || !me || hideBusy) return;
    const f = folder;
    if (f.hidden !== 'this') {
      const message =
        me.role === 'admin'
          ? `Hide “${f.name}” and everything in it from everyone? Admins can still see it with “Show hidden”.`
          : `Hide “${f.name}” and everything in it? Only for you: turn on “Show hidden” to see it again.`;
      if (!confirm(message)) return;
    }
    hideBusy = true;
    try {
      if (f.hidden === 'this') {
        await del(`/folders/${f.id}/hidden`);
      } else {
        await put(`/folders/${f.id}/hidden`);
        if (!me.show_hidden) {
          // It's gone from view: go up a level.
          const parent = f.breadcrumbs.at(-1);
          navigate(parent ? `/folder/${parent.id}` : '/', true);
          return;
        }
      }
      await loadFolder();
    } catch (e) {
      error = (e as Error).message;
    } finally {
      hideBusy = false;
    }
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
    <div class="title">
      <h2>{folder.name}</h2>
      <div class="actions">
        {#if folder.media_count || folder.subfolders.length}
          <button class="ghost action" onclick={() => (slideshow = true)}><Icon name="slideshow" size={16} /> Slideshow</button>
        {/if}
        {#if canShare}
          <button class="ghost action" onclick={() => (sharing = true)}><Icon name="sharing" size={16} /> Share</button>
          {#if folder.hidden !== 'parent'}
            <button class="ghost action" onclick={toggleHidden} disabled={hideBusy}>
              <Icon name={folder.hidden ? 'eye' : 'eyeOff'} size={16} />
              {folder.hidden ? 'Unhide' : 'Hide'}
            </button>
          {/if}
        {/if}
      </div>
    </div>
    {#if folder.hidden}
      <p class="hidden-note small">
        <Icon name="eyeOff" size={16} />
        {folder.hidden === 'this' ? 'This folder is hidden.' : 'A folder above this one is hidden.'}
        You see it because “Show hidden” is on.
      </p>
    {/if}
    <p class="muted small">{folder.media_count.toLocaleString()} item{folder.media_count === 1 ? '' : 's'}</p>

    {#if folder.subfolders.length}
      <div class="subfolders">
        {#each folder.subfolders as sub (sub.id)}
          <a class="subfolder" class:hidden-item={sub.hidden} href="/folder/{sub.id}" onclick={link}>
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
  <MoreSpace list={media} />
  {#if media.error}<p class="error">{media.error}</p>{/if}
</section>

<MediaViewerHost source={media} />

{#if slideshow}
  <Slideshow source={slides} allowSubfolders={hasSubfolders} onclose={() => (slideshow = false)} />
{/if}

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

  .actions {
    display: flex;
    gap: 6px;
    flex: none;
  }

  .action {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }

  @media (max-width: 520px) {
    /* Icon-only buttons on phones, so the folder name keeps its room. */
    .action {
      font-size: 0;
      gap: 0;
      padding: 8px 10px;
    }
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

  .hidden-note {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 4px 0 8px;
    padding: 8px 12px;
    border-radius: 10px;
    background: var(--surface-2);
    color: var(--text-muted);
  }

  .subfolder.hidden-item .cover {
    opacity: 0.4;
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
