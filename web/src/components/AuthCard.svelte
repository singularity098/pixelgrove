<script lang="ts">
  import type { Snippet } from 'svelte';

  let { title, subtitle, children }: { title: string; subtitle?: string; children: Snippet } = $props();
</script>

<main>
  <div class="card">
    <img src="/favicon.svg" alt="" width="48" height="48" />
    <h1>{title}</h1>
    {#if subtitle}<p class="muted">{subtitle}</p>{/if}
    {@render children()}
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
    font-size: 1.5rem;
    letter-spacing: -0.01em;
  }

  .muted {
    color: var(--text-muted);
    margin: 0 0 20px;
  }
</style>
