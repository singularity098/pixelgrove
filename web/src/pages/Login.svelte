<script lang="ts">
  import AuthCard from '../components/AuthCard.svelte';
  import { post, type User } from '../lib/api';
  import { signedIn } from '../lib/session.svelte';

  let username = $state('');
  let password = $state('');
  let error = $state('');
  let busy = $state(false);

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    error = '';
    busy = true;
    try {
      signedIn(await post<User>('/auth/login', { username, password }));
    } catch (e) {
      error = (e as Error).message;
      password = '';
    } finally {
      busy = false;
    }
  }
</script>

<AuthCard title="PixelGrove" subtitle="Sign in to your library.">
  <form onsubmit={submit}>
    <label>
      Username
      <!-- svelte-ignore a11y_autofocus -->
      <input bind:value={username} autocomplete="username" autofocus required />
    </label>
    <label>
      Password
      <input type="password" bind:value={password} autocomplete="current-password" required />
    </label>
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <button class="primary" disabled={busy}>{busy ? 'Signing in…' : 'Sign in'}</button>
  </form>
</AuthCard>
