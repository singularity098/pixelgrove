<script lang="ts">
  import Icon from '../components/Icon.svelte';
  import ShareDialog from '../components/ShareDialog.svelte';
  import Thumb from '../components/Thumb.svelte';
  import { del, get, type User } from '../lib/api';
  import { formatDate } from '../lib/media.svelte';
  import { link } from '../lib/router.svelte';
  import { copyText, fullUrl, type ShareView } from '../lib/shares';

  let { me }: { me: User } = $props();

  let shares: ShareView[] | null = $state(null);
  let error = $state('');
  let editing: ShareView | null = $state(null);
  let notice = $state('');

  async function load() {
    try {
      shares = await get<ShareView[]>('/shares');
    } catch (e) {
      error = (e as Error).message;
    }
  }
  $effect(() => {
    load();
  });

  const expired = (s: ShareView) => s.expires_at !== null && s.expires_at * 1000 <= Date.now();

  async function copy(share: ShareView) {
    notice = (await copyText(fullUrl(share))) ? `Copied the link to “${share.name}”` : "Couldn't copy the link";
  }

  async function remove(share: ShareView) {
    if (!confirm(`Delete the link to “${share.name}”? Anyone using it will lose access.`)) return;
    try {
      await del(`/shares/${share.id}`);
      shares = shares?.filter((s) => s.id !== share.id) ?? null;
      notice = 'Link deleted';
    } catch (e) {
      notice = (e as Error).message;
    }
  }

  function target(share: ShareView) {
    return share.kind === 'folder' ? `/folder/${share.folder_id}` : `/folder/${share.folder_id}?m=${share.media_id}`;
  }
</script>

<section class="page">
  <h2>Sharing</h2>
  <p class="muted small">
    Links you've shared{me.role === 'admin' ? ' (as an admin you see everyone’s)' : ''}. Share a photo from the viewer, or a folder from its page.
  </p>
  {#if notice}<p class="small" role="status">{notice}</p>{/if}
  {#if error}<p class="error">{error}</p>{/if}
  {#if shares?.length === 0}
    <p class="muted">No links yet.</p>
  {/if}

  <ul class="shares">
    {#each shares ?? [] as share (share.id)}
      <li class:expired={expired(share)}>
        <a class="cover" href={target(share)} onclick={link} aria-label="Open {share.name} in the library">
          <Thumb media={share.cover} />
        </a>
        <div class="main">
          <div class="name">
            <Icon name={share.kind === 'folder' ? 'folder' : 'photos'} size={16} />
            <span>{share.name || 'Missing item'}</span>
          </div>
          <div class="muted small path">{share.path}</div>
          <div class="tags small">
            {#if expired(share)}
              <span class="tag warn">Expired</span>
            {:else if share.expires_at}
              <span class="tag">Expires {formatDate(share.expires_at)}</span>
            {/if}
            {#if share.has_password}<span class="tag">Password</span>{/if}
            {#if !share.allow_download}<span class="tag">No downloads</span>{/if}
            <span class="tag">{share.views} view{share.views === 1 ? '' : 's'}</span>
            {#if me.role === 'admin' && share.created_by !== me.username}<span class="tag">by {share.created_by}</span>{/if}
          </div>
        </div>
        <div class="actions">
          <button class="icon" onclick={() => copy(share)} aria-label="Copy link to {share.name}" title="Copy link"><Icon name="copy" /></button>
          <button class="ghost" onclick={() => (editing = share)}>Edit</button>
          <button class="ghost danger" onclick={() => remove(share)}>Delete</button>
        </div>
      </li>
    {/each}
  </ul>
</section>

{#if editing}
  <ShareDialog
    target={{ name: editing.name }}
    existing={editing}
    onclose={() => (editing = null)}
    onsaved={(updated) => {
      shares = shares?.map((s) => (s.id === updated.id ? updated : s)) ?? null;
      notice = 'Link updated';
    }} />
{/if}

<style>
  .shares {
    list-style: none;
    padding: 0;
    margin: 16px 0;
    display: grid;
    gap: 8px;
  }

  li {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 10px;
    border: 1px solid var(--border);
    border-radius: 12px;
    background: var(--surface);
  }

  li.expired .cover,
  li.expired .main {
    opacity: 0.55;
  }

  .cover {
    width: 64px;
    height: 64px;
    flex: none;
    border-radius: 8px;
    overflow: hidden;
  }

  .main {
    flex: 1;
    min-width: 0;
  }

  .name {
    display: flex;
    align-items: center;
    gap: 6px;
    font-weight: 600;
  }

  .name span,
  .path {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .tags {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 6px;
  }

  .tag {
    padding: 1px 8px;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text-muted);
  }

  .tag.warn {
    color: var(--danger);
  }

  .actions {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .actions .ghost {
    border: 0;
    padding: 8px 10px;
  }

  @media (max-width: 600px) {
    li {
      flex-wrap: wrap;
    }

    .actions {
      width: 100%;
      justify-content: flex-end;
    }
  }
</style>
