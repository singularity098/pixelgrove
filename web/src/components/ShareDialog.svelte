<script lang="ts">
  import { patch, post } from '../lib/api';
  import { copyText, expiryChoices, fullUrl, type ShareView } from '../lib/shares';

  /**
   * Creates a share link for `target`, or edits `existing`. Shows the link once made.
   */
  let {
    target,
    existing = null,
    onclose,
    onsaved = () => {},
  }: {
    target: { media_id?: number; folder_id?: number; name: string };
    existing?: ShareView | null;
    onclose: () => void;
    onsaved?: (share: ShareView) => void;
  } = $props();

  let dialog: HTMLDialogElement | undefined = $state();
  // svelte-ignore state_referenced_locally
  let expiry = $state<string>(existing ? 'keep' : 'null');
  let password = $state('');
  // svelte-ignore state_referenced_locally
  let removePassword = $state(false);
  // svelte-ignore state_referenced_locally
  let allowDownload = $state(existing?.allow_download ?? true);
  let created: ShareView | null = $state(null);
  let error = $state('');
  let busy = $state(false);
  let copied = $state(false);

  $effect(() => {
    dialog?.showModal();
  });

  /** Closes and tells the parent right away (the dialog's own close event comes a
   * moment later). */
  function close() {
    dialog?.close();
    onclose();
  }

  async function save(event: SubmitEvent) {
    event.preventDefault();
    busy = true;
    error = '';
    try {
      const days = expiry === 'keep' ? undefined : expiry === 'null' ? null : Number(expiry);
      let share: ShareView;
      if (existing) {
        share = await patch<ShareView>(`/shares/${existing.id}`, {
          ...(days !== undefined ? { expires_in_days: days } : {}),
          ...(removePassword ? { password: null } : password ? { password } : {}),
          allow_download: allowDownload,
        });
        onsaved(share);
        close();
        return;
      }
      share = await post<ShareView>('/shares', {
        ...(target.media_id ? { media_id: target.media_id } : { folder_id: target.folder_id }),
        expires_in_days: days,
        password: password || null,
        allow_download: allowDownload,
      });
      created = share;
      onsaved(share);
    } catch (e) {
      error = (e as Error).message;
    } finally {
      busy = false;
    }
  }

  async function copy() {
    if (created && (await copyText(fullUrl(created)))) {
      copied = true;
      setTimeout(() => (copied = false), 2000);
    }
  }
</script>

<dialog bind:this={dialog} onclose={onclose} onkeydown={(e) => e.stopPropagation()}>
  {#if created}
    <h3>Link created</h3>
    <p class="muted small">Anyone with this link can view “{target.name}”{created.has_password ? ' after entering the password' : ''}.</p>
    <div class="link">
      <input readonly value={fullUrl(created)} onfocus={(e) => (e.currentTarget as HTMLInputElement).select()} aria-label="Share link" />
      <button class="primary" type="button" onclick={copy}>{copied ? 'Copied' : 'Copy'}</button>
    </div>
    {#if created.url.startsWith('/')}
      <p class="muted small">Tip: set <code>PIXELGROVE_PUBLIC_URL</code> so links use your public address.</p>
    {/if}
    <div class="dialog-actions">
      <button type="button" class="primary" onclick={close}>Done</button>
    </div>
  {:else}
    <form onsubmit={save}>
      <h3>{existing ? 'Edit link' : 'Share'} “{target.name}”</h3>
      <label>
        Link expires
        <select bind:value={expiry}>
          {#if existing}<option value="keep">Keep current</option>{/if}
          {#each expiryChoices as choice (choice.label)}
            <option value={String(choice.days)}>{choice.days === null ? 'Never' : `In ${choice.label}`}</option>
          {/each}
        </select>
      </label>
      <label>
        {existing?.has_password ? 'New password (optional)' : 'Password (optional)'}
        <input type="password" bind:value={password} autocomplete="new-password" disabled={removePassword} placeholder="Anyone with the link can view" />
      </label>
      {#if existing?.has_password}
        <label class="choice"><input type="checkbox" bind:checked={removePassword} /> <span>Remove the password</span></label>
      {/if}
      <label class="choice">
        <input type="checkbox" bind:checked={allowDownload} />
        <span>Allow downloads <span class="muted small">— off still lets people view</span></span>
      </label>
      {#if error}<p class="error" role="alert">{error}</p>{/if}
      <div class="dialog-actions">
        <button type="button" class="ghost" onclick={close}>Cancel</button>
        <button class="primary" disabled={busy}>{existing ? 'Save' : 'Create link'}</button>
      </div>
    </form>
  {/if}
</dialog>

<style>
  dialog {
    width: min(460px, calc(100vw - 32px));
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 16px;
    padding: 24px;
  }

  dialog::backdrop {
    background: rgb(0 0 0 / 0.6);
  }

  h3 {
    margin: 0 0 14px;
    overflow-wrap: anywhere;
  }

  select {
    display: block;
    width: 100%;
    margin-top: 6px;
    padding: 10px 12px;
    font: inherit;
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 10px;
  }

  .link {
    display: flex;
    gap: 8px;
  }

  .link input {
    margin: 0;
    font-family: ui-monospace, monospace;
    font-size: 0.85rem;
  }

  .dialog-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 16px;
  }
</style>
