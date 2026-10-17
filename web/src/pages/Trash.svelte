<script lang="ts">
  import Icon from '../components/Icon.svelte';
  import { del, post } from '../lib/api';
  import { formatDate, PagedList } from '../lib/media.svelte';
  import { onVisible } from '../lib/router.svelte';

  type Entry = {
    id: number;
    name: string;
    kind: 'image' | 'video';
    original: string;
    duration_ms: number | null;
    deleted_at: number;
    expires_at: number;
    v: number;
  };

  let trash = $state(new PagedList<Entry>('/trash', 200));
  let busy = $state(false);
  let message = $state('');

  const daysLeft = (entry: Entry) => Math.max(0, Math.ceil((entry.expires_at - Date.now() / 1000) / 86400));
  const thumb = (e: Entry) => `/api/trash/${e.id}/preview/thumb?v=${e.v}`;
  const file = (e: Entry) => `/api/trash/${e.id}/file?v=${e.v}`;

  async function act(entry: Entry, action: 'restore' | 'delete') {
    if (action === 'delete' && !confirm(`Delete “${entry.name}” permanently? This can't be undone.`)) return;
    busy = true;
    try {
      if (action === 'restore') await post(`/trash/${entry.id}/restore`);
      else await del(`/trash/${entry.id}`);
      trash.remove(entry.id);
      message = action === 'restore' ? `Restored to ${entry.original}` : 'Deleted permanently';
    } catch (e) {
      message = (e as Error).message;
    } finally {
      busy = false;
    }
  }

  async function emptyTrash() {
    if (!confirm('Delete everything in the trash permanently? This can\'t be undone.')) return;
    busy = true;
    try {
      await del('/trash');
      trash = new PagedList<Entry>('/trash', 200);
      await trash.more();
      message = 'Trash emptied';
    } catch (e) {
      message = (e as Error).message;
    } finally {
      busy = false;
    }
  }
</script>

<section class="page wide">
  <div class="heading">
    <div>
      <h2>Trash</h2>
      <p class="muted small">Items are deleted permanently after 30 days.</p>
    </div>
    {#if trash.items.length}
      <button class="ghost danger" onclick={emptyTrash} disabled={busy}>Empty trash</button>
    {/if}
  </div>
  {#if message}<p class="small" role="status">{message}</p>{/if}
  {#if trash.total === 0}
    <p class="muted">The trash is empty.</p>
  {/if}

  <ul class="grid">
    {#each trash.items as entry (entry.id)}
      <li class="card">
        <div class="thumb">
          {#if entry.kind === 'video'}
            <video src="{file(entry)}#t=0.5" preload="metadata" muted playsinline></video>
          {:else}
            <img src={thumb(entry)} alt="" loading="lazy" />
          {/if}
          <span class="days">{daysLeft(entry)} day{daysLeft(entry) === 1 ? '' : 's'} left</span>
        </div>
        <div class="info">
          <div class="name" title={entry.name}>{entry.name}</div>
          <div class="muted small path" title={entry.original}>From {entry.original}</div>
          <div class="muted small">Deleted {formatDate(entry.deleted_at)}</div>
        </div>
        <div class="actions">
          <button class="ghost" onclick={() => act(entry, 'restore')} disabled={busy}><Icon name="restore" size={16} /> Restore</button>
          <button class="ghost danger" onclick={() => act(entry, 'delete')} disabled={busy} aria-label="Delete {entry.name} permanently">Delete</button>
        </div>
      </li>
    {/each}
  </ul>
  {#if trash.error}<p class="error">{trash.error}</p>{/if}
  <div use:onVisible={() => trash.more()}></div>
</section>

<style>
  .heading {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
  }

  .grid {
    list-style: none;
    margin: 16px 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: 14px;
  }

  .card {
    border: 1px solid var(--border);
    border-radius: 12px;
    overflow: hidden;
    background: var(--surface);
    display: flex;
    flex-direction: column;
  }

  .thumb {
    position: relative;
    aspect-ratio: 4 / 3;
    background: var(--surface-2);
  }

  .thumb img,
  .thumb video {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
    opacity: 0.85;
  }

  .days {
    position: absolute;
    left: 8px;
    bottom: 8px;
    padding: 2px 8px;
    border-radius: 999px;
    font-size: 0.75rem;
    color: #fff;
    background: rgb(0 0 0 / 0.6);
  }

  .info {
    padding: 10px 12px 4px;
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

  .actions {
    display: flex;
    justify-content: space-between;
    padding: 6px 8px 10px;
    margin-top: auto;
  }

  .actions button {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    border: 0;
  }

  @media (max-width: 520px) {
    .grid {
      grid-template-columns: repeat(2, 1fr);
      gap: 8px;
    }
  }
</style>
