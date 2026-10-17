<script lang="ts">
  import AuthCard from '../components/AuthCard.svelte';
  import { post, type User } from '../lib/api';
  import { signedIn } from '../lib/session.svelte';

  let username = $state('admin');
  let password = $state('');
  let confirm = $state('');
  let error = $state('');
  let busy = $state(false);

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    error = '';
    if (password !== confirm) {
      error = "Passwords don't match.";
      return;
    }
    busy = true;
    try {
      signedIn(await post<User>('/setup', { username, password }));
    } catch (e) {
      error = (e as Error).message;
    } finally {
      busy = false;
    }
  }
</script>

<AuthCard title="Welcome to PixelGrove" subtitle="Create the administrator account to get started.">
  <form onsubmit={submit}>
    <label>
      Username
      <input bind:value={username} autocomplete="username" required />
    </label>
    <label>
      Password
      <input type="password" bind:value={password} autocomplete="new-password" minlength="8" required />
    </label>
    <label>
      Confirm password
      <input type="password" bind:value={confirm} autocomplete="new-password" required />
    </label>
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <button class="primary" disabled={busy}>{busy ? 'Creating…' : 'Create account'}</button>
  </form>
</AuthCard>
