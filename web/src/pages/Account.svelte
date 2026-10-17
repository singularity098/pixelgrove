<script lang="ts">
  import { post, type User } from '../lib/api';

  let { user }: { user: User } = $props();

  let current = $state('');
  let next = $state('');
  let confirm = $state('');
  let message = $state('');
  let error = $state('');
  let busy = $state(false);

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    message = '';
    error = '';
    if (next !== confirm) {
      error = "New passwords don't match.";
      return;
    }
    busy = true;
    try {
      await post('/auth/password', { current_password: current, new_password: next });
      message = 'Password changed. Your other devices have been signed out.';
      current = next = confirm = '';
    } catch (e) {
      error = (e as Error).message;
    } finally {
      busy = false;
    }
  }
</script>

<section class="page narrow">
  <h2>Account</h2>
  <p class="muted">Signed in as <strong>{user.username}</strong> ({user.role}).</p>

  {#if user.role === 'viewer'}
    <p class="muted">This is a view-only account. Ask an administrator to change its password.</p>
  {:else}
    <form onsubmit={submit}>
      <h3>Change password</h3>
      <label>
        Current password
        <input type="password" bind:value={current} autocomplete="current-password" required />
      </label>
      <label>
        New password
        <input type="password" bind:value={next} autocomplete="new-password" minlength="8" required />
      </label>
      <label>
        Confirm new password
        <input type="password" bind:value={confirm} autocomplete="new-password" required />
      </label>
      {#if error}<p class="error" role="alert">{error}</p>{/if}
      {#if message}<p class="success" role="status">{message}</p>{/if}
      <button class="primary" disabled={busy}>Change password</button>
    </form>
  {/if}
</section>
