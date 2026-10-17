<script lang="ts">
  import type { User } from '../lib/api';
  import { link, route } from '../lib/router.svelte';
  import { signOut } from '../lib/session.svelte';
  import { currentTheme, setTheme, type Theme } from '../theme';

  let { user }: { user: User } = $props();
  let theme: Theme = $state(currentTheme());

  function toggleTheme() {
    theme = theme === 'dark' ? 'light' : 'dark';
    setTheme(theme);
  }

  const links = $derived([
    { href: '/', label: 'Library' },
    ...(user.role === 'admin' ? [{ href: '/users', label: 'Users' }] : []),
    { href: '/account', label: 'Account' },
  ]);
</script>

<header>
  <a class="brand" href="/" onclick={link}>
    <img src="/favicon.svg" alt="" width="24" height="24" />
    <span>PixelGrove</span>
  </a>
  <nav>
    {#each links as item (item.href)}
      <a href={item.href} onclick={link} class:active={route.path === item.href}>{item.label}</a>
    {/each}
  </nav>
  <div class="actions">
    <button class="icon" onclick={toggleTheme} title="Switch to {theme === 'dark' ? 'light' : 'dark'} mode" aria-label="Toggle theme">
      {theme === 'dark' ? '☀' : '☾'}
    </button>
    <button class="ghost" onclick={signOut}>Sign out</button>
  </div>
</header>

<style>
  header {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 10px 16px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
    flex-wrap: wrap;
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 8px;
    font-weight: 600;
    color: var(--text);
    text-decoration: none;
  }

  nav {
    display: flex;
    gap: 4px;
    flex: 1;
  }

  nav a {
    padding: 6px 10px;
    border-radius: 8px;
    color: var(--text-muted);
    text-decoration: none;
  }

  nav a:hover,
  nav a.active {
    color: var(--text);
    background: var(--surface-2);
  }

  .actions {
    display: flex;
    gap: 8px;
    margin-left: auto;
  }

  @media (max-width: 520px) {
    nav {
      order: 3;
      flex-basis: 100%;
    }
  }
</style>
