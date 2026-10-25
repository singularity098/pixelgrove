<script lang="ts">
  import Icon from '../components/Icon.svelte';
  import Logo from '../components/Logo.svelte';
  import MediaGrid from '../components/MediaGrid.svelte';
  import MoreSpace from '../components/MoreSpace.svelte';
  import MediaViewerHost from '../components/MediaViewerHost.svelte';
  import Slideshow from '../components/Slideshow.svelte';
  import ThemeMenu from '../components/ThemeMenu.svelte';
  import Thumb from '../components/Thumb.svelte';
  import Viewer from '../components/Viewer.svelte';
  import { get, post } from '../lib/api';
  import { formatDate, shareUrls } from '../lib/media.svelte';
  import { openViewer, withParam } from '../lib/query';
  import { navigate, route } from '../lib/router.svelte';
  import { SharedFolder, type ShareInfo } from '../lib/shared.svelte';
  import { slideshowSettings } from '../lib/slideshow.svelte';

  /** A share link, opened by anyone (no account needed). */
  let { token }: { token: string } = $props();

  let info = $state<ShareInfo | null>(null);
  let error = $state('');
  let password = $state('');
  let unlockError = $state('');
  let busy = $state(false);

  $effect(() => {
    get<ShareInfo>(`/s/${token}`).then(
      (i) => (info = i),
      (e) => (error = (e as Error).message),
    );
  });

  async function unlock(event: SubmitEvent) {
    event.preventDefault();
    busy = true;
    unlockError = '';
    try {
      info = await post<ShareInfo>(`/s/${token}/unlock`, { password });
    } catch (e) {
      unlockError = (e as Error).message;
    } finally {
      busy = false;
    }
  }

  const urls = $derived(shareUrls(token, info?.allow_download ?? false));
  // The folder within the share being shown (?p=sub/folder).
  const subpath = $derived(route.query.get('p') ?? '');
  const folder = $derived(info && !info.locked && info.kind === 'folder' ? new SharedFolder(token, subpath) : null);
  const crumbs = $derived(subpath ? subpath.split('/') : []);

  let slideshow = $state(false);
  const hasSubfolders = $derived(!!folder?.subfolders.length);
  const slides = $derived(
    slideshow && folder && hasSubfolders && slideshowSettings.subfolders
      ? new SharedFolder(token, subpath, true)
      : folder,
  );

  function openFolder(path: string) {
    navigate(`/s/${token}${path ? `?p=${encodeURIComponent(path)}` : ''}`);
  }

</script>

<header>
  <Logo />
  <div class="spacer"></div>
  <ThemeMenu />
</header>

{#if error}
  <main class="center">
    <div class="card">
      <h1>Link unavailable</h1>
      <p class="muted">{error}</p>
    </div>
  </main>
{:else if info?.locked}
  <main class="center">
    <form class="card" onsubmit={unlock}>
      <h1>Password needed</h1>
      <p class="muted">Enter the password you were given to view this {info.kind === 'folder' ? 'folder' : 'item'}.</p>
      <label>
        Password
        <!-- svelte-ignore a11y_autofocus -->
        <input type="password" bind:value={password} autocomplete="off" autofocus required />
      </label>
      {#if unlockError}<p class="error" role="alert">{unlockError}</p>{/if}
      <button class="primary" disabled={busy}>View</button>
    </form>
  </main>
{:else if info?.kind === 'media' && info.media}
  <Viewer
    items={[info.media]}
    index={0}
    total={1}
    onindex={() => {}}
    onclose={() => {}}
    onneedmore={async () => false}
    onremoved={() => {}}
    {urls}
    readOnly
    closable={false} />
{:else if folder}
  {#key folder}
    <section class="page wide">
      <nav class="crumbs small" aria-label="Breadcrumbs">
        <button class="crumb" onclick={() => openFolder('')}>{info?.name}</button>
        {#each crumbs as crumb, i (i)}
          <span>/</span>
          <button class="crumb" onclick={() => openFolder(crumbs.slice(0, i + 1).join('/'))}>{crumb}</button>
        {/each}
      </nav>
      <div class="title">
        <h2>{crumbs.at(-1) ?? info?.name}</h2>
        {#if folder.items.length || folder.subfolders.length}
          <button class="ghost action" onclick={() => (slideshow = true)}><Icon name="slideshow" size={16} /> Slideshow</button>
        {/if}
      </div>
      <p class="muted small">
        {#if folder.total !== null}{folder.total.toLocaleString()} item{folder.total === 1 ? '' : 's'}{/if}
        {#if info?.expires_at} · link expires {formatDate(info.expires_at)}{/if}
      </p>
      {#if folder.subfolders.length}
        <div class="subfolders">
          {#each folder.subfolders as sub (sub.id)}
            <button class="subfolder" onclick={() => openFolder(subpath ? `${subpath}/${sub.name}` : sub.name)}>
              <div class="cover"><Thumb media={sub.cover} {urls} /></div>
              <div>
                <div class="name">{sub.name}</div>
                <div class="muted small">{sub.count.toLocaleString()}</div>
              </div>
            </button>
          {/each}
        </div>
      {/if}
      <MediaGrid items={folder.items} {urls} onopen={(i) => openViewer(folder.items[i].id)} />
      <MoreSpace list={folder} />
      {#if folder.error}<p class="error">{folder.error}</p>{/if}
    </section>
    <MediaViewerHost source={folder} {urls} readOnly />
    {#if slideshow && slides}
      <Slideshow source={slides} {urls} allowSubfolders={hasSubfolders} onclose={() => (slideshow = false)} />
    {/if}
  {/key}
{/if}

<style>
  .title {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .action {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    flex: none;
  }

  header {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 56px;
    padding: 0 12px 0 16px;
    border-bottom: 1px solid var(--border);
  }


  .spacer {
    flex: 1;
  }

  .center {
    display: grid;
    place-items: center;
    padding: 48px 16px;
  }

  .card {
    width: 100%;
    max-width: 380px;
    padding: 28px 24px;
    border: 1px solid var(--border);
    border-radius: 16px;
    background: var(--surface);
  }

  .card h1 {
    margin: 0 0 6px;
    font-size: 1.3rem;
  }

  .crumbs {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    color: var(--text-muted);
  }

  .crumb {
    border: 0;
    padding: 0;
    background: none;
    color: var(--text-muted);
  }

  .crumb:hover {
    color: var(--text);
  }

  .subfolders {
    display: flex;
    gap: 10px;
    overflow-x: auto;
    padding: 4px 0 16px;
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
    text-align: left;
    max-width: 260px;
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
