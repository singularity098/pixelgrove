<script lang="ts">
  import { currentTheme, setTheme, type Theme } from './theme';

  type Health = { status: string; version: string };

  let theme: Theme = $state(currentTheme());
  let health: Health | null = $state(null);
  let error = $state(false);

  $effect(() => {
    fetch('/api/health')
      .then((r) => (r.ok ? r.json() : Promise.reject(r.status)))
      .then((h: Health) => (health = h))
      .catch(() => (error = true));
  });

  function toggleTheme() {
    theme = theme === 'dark' ? 'light' : 'dark';
    setTheme(theme);
  }
</script>

<main>
  <div class="card">
    <img src="/favicon.svg" alt="" width="56" height="56" />
    <h1>PixelGrove</h1>
    <p class="muted">Your photo and video library is on its way.</p>
    <p class="status">
      {#if health}
        <span class="dot ok"></span> Server v{health.version} is running
      {:else if error}
        <span class="dot bad"></span> Can't reach the server
      {:else}
        <span class="dot"></span> Connecting…
      {/if}
    </p>
    <button onclick={toggleTheme}>
      Switch to {theme === 'dark' ? 'light' : 'dark'} mode
    </button>
  </div>
</main>

<style>
  main {
    min-height: 100%;
    display: grid;
    place-items: center;
    padding: 16px;
  }

  .card {
    width: 100%;
    max-width: 380px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 16px;
    padding: 32px 24px;
    text-align: center;
  }

  h1 {
    margin: 12px 0 4px;
    font-size: 1.6rem;
    letter-spacing: -0.01em;
  }

  .muted {
    color: var(--text-muted);
    margin: 0 0 20px;
  }

  .status {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    margin: 0 0 24px;
    font-size: 0.9rem;
  }

  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--text-muted);
  }

  .dot.ok {
    background: var(--accent);
  }

  .dot.bad {
    background: var(--danger);
  }

  button {
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 8px 16px;
    cursor: pointer;
  }

  button:hover {
    border-color: var(--accent);
  }
</style>
