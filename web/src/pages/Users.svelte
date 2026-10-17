<script lang="ts">
  import { del, get, patch, post, type Role, type User } from '../lib/api';

  let { me }: { me: User } = $props();

  let users: User[] = $state([]);
  let error = $state('');

  type Draft = { id: number | null; username: string; password: string; role: Role; disabled: boolean; folders: string };
  let draft: Draft | null = $state(null);
  let formError = $state('');
  let busy = $state(false);
  let dialog: HTMLDialogElement | undefined = $state();

  const roles: { value: Role; label: string; hint: string }[] = [
    { value: 'viewer', label: 'Viewer', hint: 'Browse and download only' },
    { value: 'editor', label: 'Editor', hint: 'Also favorites, sharing and trash' },
    { value: 'admin', label: 'Admin', hint: 'Everything, including settings and users' },
  ];

  async function load() {
    try {
      users = await get<User[]>('/users');
    } catch (e) {
      error = (e as Error).message;
    }
  }
  $effect(() => {
    load();
  });

  function open(user: User | null) {
    formError = '';
    draft = user
      ? { id: user.id, username: user.username, password: '', role: user.role, disabled: user.disabled, folders: user.folders.join('\n') }
      : { id: null, username: '', password: '', role: 'viewer', disabled: false, folders: '' };
    dialog?.showModal();
  }

  function close() {
    dialog?.close();
    draft = null;
  }

  async function save(event: SubmitEvent) {
    event.preventDefault();
    if (!draft) return;
    const folders = draft.folders
      .split('\n')
      .map((f) => f.trim())
      .filter(Boolean);
    busy = true;
    formError = '';
    try {
      if (draft.id === null) {
        await post('/users', { username: draft.username, password: draft.password, role: draft.role, folders });
      } else {
        const self = draft.id === me.id;
        await patch(`/users/${draft.id}`, {
          folders,
          ...(self ? {} : { role: draft.role, disabled: draft.disabled }),
          ...(draft.password ? { password: draft.password } : {}),
        });
      }
      close();
      await load();
    } catch (e) {
      formError = (e as Error).message;
    } finally {
      busy = false;
    }
  }

  async function remove(user: User) {
    if (!confirm(`Delete ${user.username}? Their favorites and share links will be removed too.`)) return;
    try {
      await del(`/users/${user.id}`);
      await load();
    } catch (e) {
      error = (e as Error).message;
    }
  }
</script>

<section class="page">
  <div class="heading">
    <h2>Users</h2>
    <button class="primary" onclick={() => open(null)}>Add user</button>
  </div>
  {#if error}<p class="error">{error}</p>{/if}

  <ul class="users">
    {#each users as user (user.id)}
      <li class:disabled={user.disabled}>
        <div class="who">
          <strong>{user.username}</strong>
          {#if user.id === me.id}<span class="tag">you</span>{/if}
          {#if user.disabled}<span class="tag">disabled</span>{/if}
          <div class="muted small">
            {roles.find((r) => r.value === user.role)?.label}
            · {user.folders.length ? `Limited to ${user.folders.length} folder${user.folders.length > 1 ? 's' : ''}` : 'Whole library'}
          </div>
        </div>
        <div class="row-actions">
          <button class="ghost" onclick={() => open(user)}>Edit</button>
          {#if user.id !== me.id}
            <button class="ghost danger" onclick={() => remove(user)}>Delete</button>
          {/if}
        </div>
      </li>
    {/each}
  </ul>
</section>

<dialog bind:this={dialog} onclose={() => (draft = null)}>
  {#if draft}
    {@const self = draft.id === me.id}
    <form onsubmit={save}>
      <h3>{draft.id === null ? 'Add user' : `Edit ${draft.username}`}</h3>
      {#if draft.id === null}
        <label>
          Username
          <input bind:value={draft.username} autocomplete="off" required />
        </label>
      {/if}
      <label>
        {draft.id === null ? 'Password' : 'New password (leave blank to keep)'}
        <input type="password" bind:value={draft.password} autocomplete="new-password" minlength="8" required={draft.id === null} />
      </label>
      <fieldset disabled={self}>
        <legend>Role{self ? " (you can't change your own)" : ''}</legend>
        {#each roles as role (role.value)}
          <label class="choice">
            <input type="radio" name="role" value={role.value} bind:group={draft.role} />
            <span>{role.label} <span class="muted small">— {role.hint}</span></span>
          </label>
        {/each}
      </fieldset>
      <label>
        Limit to folders <span class="muted small">(one path per line, e.g. /media/photos/family; blank = whole library)</span>
        <textarea bind:value={draft.folders} rows="3" spellcheck="false"></textarea>
      </label>
      {#if draft.id !== null && !self}
        <label class="choice">
          <input type="checkbox" bind:checked={draft.disabled} />
          <span>Disabled <span class="muted small">— can't sign in</span></span>
        </label>
      {/if}
      {#if formError}<p class="error" role="alert">{formError}</p>{/if}
      <div class="dialog-actions">
        <button type="button" class="ghost" onclick={close}>Cancel</button>
        <button class="primary" disabled={busy}>Save</button>
      </div>
    </form>
  {/if}
</dialog>

<style>
  .heading {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .users {
    list-style: none;
    padding: 0;
    margin: 16px 0;
    display: grid;
    gap: 8px;
  }

  .users li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 12px 16px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 12px;
  }

  .users li.disabled .who {
    opacity: 0.6;
  }

  .tag {
    font-size: 0.75rem;
    padding: 1px 8px;
    margin-left: 6px;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text-muted);
  }

  .row-actions {
    display: flex;
    gap: 6px;
  }

  dialog {
    width: min(480px, calc(100vw - 32px));
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 16px;
    padding: 24px;
  }

  dialog::backdrop {
    background: rgb(0 0 0 / 0.55);
  }

  dialog h3 {
    margin-top: 0;
  }

  fieldset {
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 8px 12px;
    margin: 0 0 14px;
  }

  .dialog-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 8px;
  }
</style>
