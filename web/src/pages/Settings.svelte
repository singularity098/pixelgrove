<script lang="ts">
  import Icon from '../components/Icon.svelte';
  import { del, get, post } from '../lib/api';

  type MediaRoot = {
    path: string;
    configured: boolean;
    folders: number;
    media: number;
    available: boolean;
    writable: boolean;
  };
  type Status = {
    folders: number;
    images: number;
    videos: number;
    scanning: boolean;
    watching: boolean;
    last_full_scan_at: number | null;
  };
  type Directories = { path: string; parent: string | null; dirs: string[] };

  let roots: MediaRoot[] = $state([]);
  let status = $state<Status | null>(null);
  let version = $state('');
  let error = $state('');

  let newPath = $state('');
  let addError = $state('');
  let busy = $state(false);
  let browser = $state<Directories | null>(null);
  let browseError = $state('');

  async function loadRoots() {
    try {
      roots = await get<MediaRoot[]>('/settings/roots');
    } catch (e) {
      error = (e as Error).message;
    }
  }

  async function loadStatus() {
    try {
      status = await get<Status>('/library/status');
    } catch {
      // Shown as unknown.
    }
  }

  $effect(() => {
    loadRoots();
    loadStatus();
    get<{ version: string }>('/health').then((h) => (version = h.version), () => {});
  });

  // While a scan runs, keep the numbers fresh.
  $effect(() => {
    if (!status?.scanning) return;
    const timer = setInterval(async () => {
      await loadStatus();
      if (!status?.scanning) loadRoots();
    }, 2000);
    return () => clearInterval(timer);
  });

  async function add(event: SubmitEvent) {
    event.preventDefault();
    busy = true;
    addError = '';
    try {
      roots = await post<MediaRoot[]>('/settings/roots', { path: newPath });
      newPath = '';
      browser = null;
      // The new folder is indexed in the background; poll until that's done.
      if (status) status.scanning = true;
    } catch (e) {
      addError = (e as Error).message;
    } finally {
      busy = false;
    }
  }

  async function remove(root: MediaRoot) {
    if (
      !confirm(
        `Remove ${root.path} from the library? Its ${root.media.toLocaleString()} items, and favorites and share links for them, will be removed from PixelGrove. The files themselves are not touched.`,
      )
    )
      return;
    try {
      roots = await del<MediaRoot[]>('/settings/roots', { path: root.path });
      await loadStatus();
    } catch (e) {
      error = (e as Error).message;
    }
  }

  async function rescan() {
    try {
      await post('/library/rescan');
      if (status) status.scanning = true;
    } catch (e) {
      error = (e as Error).message;
    }
  }

  async function browse(path: string) {
    browseError = '';
    try {
      browser = await get<Directories>(`/settings/directories?path=${encodeURIComponent(path)}`);
      newPath = browser.path;
    } catch (e) {
      browseError = (e as Error).message;
    }
  }

  const join = (dir: string, name: string) => `${dir === '/' ? '' : dir}/${name}`;

  function ago(seconds: number): string {
    const minutes = Math.round((Date.now() / 1000 - seconds) / 60);
    if (minutes < 1) return 'just now';
    if (minutes < 60) return `${minutes} min ago`;
    const hours = Math.round(minutes / 60);
    if (hours < 24) return `${hours} h ago`;
    return new Date(seconds * 1000).toLocaleString();
  }
</script>

<section class="page">
  <h2>Settings</h2>
  {#if error}<p class="error" role="alert">{error}</p>{/if}

  <h3>Library</h3>
  <div class="card stats">
    {#if status}
      <div><strong>{status.images.toLocaleString()}</strong><span class="muted small">photos</span></div>
      <div><strong>{status.videos.toLocaleString()}</strong><span class="muted small">videos</span></div>
      <div><strong>{status.folders.toLocaleString()}</strong><span class="muted small">folders</span></div>
      <div class="scan">
        <span class="muted small">
          {#if status.scanning}
            <span class="dot"></span> Scanning…
          {:else if status.last_full_scan_at}
            Last full scan {ago(status.last_full_scan_at)}
          {:else}
            Not scanned yet
          {/if}
          {#if !status.watching} · live updates unavailable, relying on scheduled rescans{/if}
        </span>
        <button class="ghost" onclick={rescan} disabled={status.scanning}>Rescan now</button>
      </div>
    {:else}
      <p class="muted small">Loading…</p>
    {/if}
  </div>

  <h3>Media folders</h3>
  <p class="muted small">
    PixelGrove indexes everything in these folders and keeps watching them for changes. Folders
    must be mounted into the container (for example as <code>/media/…</code>).
  </p>
  <ul class="roots">
    {#each roots as root (root.path)}
      <li>
        <div class="root">
          <div class="path"><Icon name="folder" size={18} /> {root.path}</div>
          <div class="muted small">
            {root.media.toLocaleString()} items in {root.folders.toLocaleString()} folders
            {#if root.configured}<span class="tag" title="Set with PIXELGROVE_MEDIA_ROOTS or pixelgrove.toml">from configuration</span>{/if}
            {#if !root.available}<span class="tag warn">missing</span>
            {:else if !root.writable}<span class="tag" title="Mounted read-only, so Trash is unavailable here">read-only</span>{/if}
          </div>
        </div>
        {#if !root.configured}
          <button class="ghost danger" onclick={() => remove(root)}>Remove</button>
        {/if}
      </li>
    {/each}
  </ul>

  <form class="card add" onsubmit={add}>
    <label>
      Add a folder
      <div class="input-row">
        <input bind:value={newPath} placeholder="/media/more-photos" spellcheck="false" autocomplete="off" required />
        <button type="button" class="ghost" onclick={() => browse(newPath || '/')}>Browse</button>
        <button class="primary" disabled={busy || !newPath.trim()}>Add</button>
      </div>
    </label>
    {#if addError}<p class="error" role="alert">{addError}</p>{/if}
    {#if browseError}<p class="error">{browseError}</p>{/if}
    {#if browser}
      <div class="browser" role="group" aria-label="Folders in {browser.path}">
        <div class="where small">
          {#if browser.parent !== null}
            <button type="button" class="link" onclick={() => browse(browser!.parent!)}>
              <Icon name="back" size={16} /> Up
            </button>
          {/if}
          <span class="muted">{browser.path}</span>
        </div>
        {#each browser.dirs as dir (dir)}
          <button type="button" class="dir" onclick={() => browse(join(browser!.path, dir))}>
            <Icon name="folder" size={16} />
            {dir}
          </button>
        {:else}
          <p class="muted small">No folders here.</p>
        {/each}
      </div>
    {/if}
  </form>

  <h3>About</h3>
  <p class="muted small">
    PixelGrove {version}. Other options (sign-in, preview cache, rescan interval, exclusions) are set with
    environment variables or <code>/config/pixelgrove.toml</code>.
  </p>
</section>

<style>
  h3 {
    margin: 28px 0 10px;
  }

  .card {
    padding: 14px 16px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 12px;
  }

  .stats {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 12px 28px;
  }

  .stats > div {
    display: grid;
  }

  .stats strong {
    font-size: 1.3rem;
  }

  .stats .scan {
    flex: 1;
    min-width: 220px;
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 12px;
  }

  .dot {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--accent);
    animation: pulse 1s ease-in-out infinite alternate;
  }

  @keyframes pulse {
    to {
      opacity: 0.3;
    }
  }

  .roots {
    list-style: none;
    padding: 0;
    margin: 12px 0;
    display: grid;
    gap: 8px;
  }

  .roots li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 12px 16px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 12px;
  }

  .root {
    min-width: 0;
  }

  .path {
    display: flex;
    align-items: center;
    gap: 8px;
    font-weight: 600;
    overflow-wrap: anywhere;
  }

  .tag {
    font-size: 0.75rem;
    padding: 1px 8px;
    margin-left: 6px;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text-muted);
    white-space: nowrap;
  }

  .tag.warn {
    color: var(--danger, #e5534b);
  }

  .add label {
    display: grid;
    gap: 8px;
  }

  .input-row {
    display: flex;
    gap: 8px;
  }

  .input-row input {
    flex: 1;
    min-width: 0;
    font-family: ui-monospace, monospace;
  }

  .browser {
    margin-top: 12px;
    max-height: 320px;
    overflow-y: auto;
    display: grid;
    border-top: 1px solid var(--border);
    padding-top: 8px;
  }

  .where {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 4px 0 8px;
  }

  .link,
  .dir {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    border: 0;
    background: none;
    color: var(--text);
    padding: 8px 6px;
    border-radius: 8px;
    text-align: left;
  }

  .dir:hover,
  .link:hover {
    background: var(--surface-2);
  }

  code {
    font-size: 0.85em;
  }

  @media (max-width: 520px) {
    .input-row {
      flex-wrap: wrap;
    }

    .input-row input {
      flex-basis: 100%;
    }
  }
</style>
