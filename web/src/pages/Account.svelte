<script lang="ts">
  import { post, type User } from '../lib/api';
  import { currentTheme, setTheme, THEMES } from '../theme';

  let { user }: { user: User } = $props();

  let current = $state('');
  let next = $state('');
  let confirm = $state('');
  let message = $state('');
  let error = $state('');
  let busy = $state(false);
  let theme = $state(currentTheme());

  function chooseTheme(id: string) {
    theme = id;
    setTheme(id);
  }

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

  <h3>Appearance</h3>
  <p class="muted small">Remembered on this device.</p>
  <div class="themes" role="radiogroup" aria-label="Theme">
    {#each THEMES as t (t.id)}
      <button class="theme" role="radio" aria-checked={t.id === theme} class:selected={t.id === theme} onclick={() => chooseTheme(t.id)}>
        <span class="preview" style="background: {t.swatch[0]}">
          <span class="card" style="background: {t.swatch[1]}"><span class="dot" style="background: {t.swatch[2]}"></span></span>
        </span>
        <span>{t.name}</span>
      </button>
    {/each}
  </div>
</section>

<style>
  h3 {
    margin-top: 28px;
  }

  .themes {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(130px, 1fr));
    gap: 10px;
    margin-top: 10px;
  }

  .theme {
    display: grid;
    gap: 6px;
    padding: 6px 6px 8px;
    border: 2px solid var(--border);
    border-radius: 12px;
    background: var(--surface);
    color: var(--text);
    font-size: 0.85rem;
    text-align: left;
  }

  .theme.selected {
    border-color: var(--accent);
  }

  .preview {
    display: flex;
    align-items: flex-end;
    height: 56px;
    padding: 8px;
    border-radius: 8px;
  }

  .card {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    width: 70%;
    height: 22px;
    padding: 0 6px;
    border-radius: 5px;
  }

  .dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
  }
</style>
