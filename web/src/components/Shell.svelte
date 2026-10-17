<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { User } from '../lib/api';
  import { link, route } from '../lib/router.svelte';
  import { signOut } from '../lib/session.svelte';
  import { currentTheme, setTheme, type Theme } from '../theme';
  import Icon, { type IconName } from './Icon.svelte';

  let { user, children }: { user: User; children: Snippet } = $props();

  const SIDEBAR_KEY = 'pixelgrove-sidebar';
  const narrow = window.matchMedia('(max-width: 900px)');
  let isNarrow = $state(narrow.matches);
  let pinned = $state(readPinned());
  let drawerOpen = $state(false);
  let theme: Theme = $state(currentTheme());

  function readPinned(): boolean {
    try {
      return localStorage.getItem(SIDEBAR_KEY) !== 'hidden';
    } catch {
      return true;
    }
  }

  $effect(() => {
    const onChange = () => {
      isNarrow = narrow.matches;
      drawerOpen = false;
    };
    narrow.addEventListener('change', onChange);
    return () => narrow.removeEventListener('change', onChange);
  });

  // Close the mobile drawer after navigating.
  $effect(() => {
    route.path;
    drawerOpen = false;
  });

  const sidebarVisible = $derived(isNarrow ? drawerOpen : pinned);

  function toggleSidebar() {
    if (isNarrow) {
      drawerOpen = !drawerOpen;
      return;
    }
    pinned = !pinned;
    try {
      localStorage.setItem(SIDEBAR_KEY, pinned ? 'shown' : 'hidden');
    } catch {
      // Not remembered; fine.
    }
  }

  function toggleTheme() {
    theme = theme === 'dark' ? 'light' : 'dark';
    setTheme(theme);
  }

  type Item = { href: string; label: string; icon: IconName };
  const library: Item[] = [
    { href: '/', label: 'Photos', icon: 'photos' },
    { href: '/timeline', label: 'Timeline', icon: 'timeline' },
    { href: '/explore', label: 'Explore', icon: 'explore' },
    { href: '/map', label: 'Map', icon: 'map' },
    { href: '/sharing', label: 'Sharing', icon: 'sharing' },
    { href: '/favorites', label: 'Favorites', icon: 'favorite' },
    { href: '/files', label: 'File View', icon: 'files' },
    { href: '/trash', label: 'Trash', icon: 'trash' },
  ];
  const account = $derived<Item[]>([
    ...(user.role === 'admin' ? [{ href: '/users', label: 'Users', icon: 'users' as IconName }] : []),
    { href: '/account', label: 'Account', icon: 'account' },
  ]);

  function isActive(href: string): boolean {
    if (href === '/') return route.path === '/' || route.path.startsWith('/folder/');
    return route.path === href || route.path.startsWith(`${href}/`);
  }
</script>

<div class="shell" class:with-sidebar={sidebarVisible && !isNarrow}>
  <header>
    <button class="icon" onclick={toggleSidebar} aria-label="Toggle sidebar" aria-expanded={sidebarVisible}>
      <Icon name="menu" />
    </button>
    <a class="brand" href="/" onclick={link}>
      <img src="/favicon.svg" alt="" width="26" height="26" />
      <span>PixelGrove</span>
    </a>
    <div class="spacer"></div>
    <button class="icon" onclick={toggleTheme} aria-label="Switch to {theme === 'dark' ? 'light' : 'dark'} mode" title="Switch to {theme === 'dark' ? 'light' : 'dark'} mode">
      <Icon name={theme === 'dark' ? 'sun' : 'moon'} />
    </button>
    <button class="icon" onclick={signOut} aria-label="Sign out" title="Sign out ({user.username})">
      <Icon name="signOut" />
    </button>
  </header>

  {#if isNarrow && drawerOpen}
    <button class="scrim" aria-label="Close sidebar" onclick={() => (drawerOpen = false)}></button>
  {/if}

  <nav class="sidebar" class:open={sidebarVisible} class:drawer={isNarrow} aria-label="Main" inert={!sidebarVisible}>
    <ul>
      {#each library as item (item.href)}
        <li>
          <a href={item.href} onclick={link} class:active={isActive(item.href)}>
            <Icon name={item.icon} />
            <span>{item.label}</span>
          </a>
        </li>
      {/each}
    </ul>
    <ul class="secondary">
      {#each account as item (item.href)}
        <li>
          <a href={item.href} onclick={link} class:active={isActive(item.href)}>
            <Icon name={item.icon} />
            <span>{item.label}</span>
          </a>
        </li>
      {/each}
    </ul>
  </nav>

  <main>
    {@render children()}
  </main>
</div>

<style>
  .shell {
    --header-h: 56px;
    --sidebar-w: 232px;
    min-height: 100%;
  }

  header {
    position: sticky;
    top: 0;
    z-index: 30;
    height: var(--header-h);
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 8px;
    background: color-mix(in srgb, var(--bg) 88%, transparent);
    backdrop-filter: blur(12px);
    border-bottom: 1px solid var(--border);
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 8px;
    font-weight: 600;
    font-size: 1.05rem;
    color: var(--text);
    text-decoration: none;
  }

  .spacer {
    flex: 1;
  }

  .sidebar {
    position: fixed;
    top: var(--header-h);
    bottom: 0;
    left: 0;
    z-index: 20;
    width: var(--sidebar-w);
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    padding: 12px 8px;
    overflow-y: auto;
    background: var(--bg);
    transform: translateX(-100%);
    transition: transform 0.2s ease;
  }

  .sidebar.open {
    transform: none;
  }

  .sidebar.drawer {
    top: 0;
    z-index: 50;
    padding-top: 16px;
    background: var(--surface);
  }

  .sidebar.drawer.open {
    box-shadow: 0 0 40px rgb(0 0 0 / 0.4);
  }

  .scrim {
    position: fixed;
    inset: 0;
    z-index: 40;
    border: 0;
    border-radius: 0;
    padding: 0;
    background: rgb(0 0 0 / 0.5);
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 2px;
  }

  .secondary {
    padding-top: 12px;
    border-top: 1px solid var(--border);
  }

  .sidebar a {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 9px 14px;
    border-radius: 999px;
    color: var(--text);
    text-decoration: none;
    font-size: 0.95rem;
  }

  .sidebar a:hover {
    background: var(--surface-2);
  }

  .sidebar a.active {
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    color: var(--accent);
    font-weight: 600;
  }

  main {
    transition: margin-left 0.2s ease;
  }

  .with-sidebar main {
    margin-left: var(--sidebar-w);
  }

  @media (prefers-reduced-motion: reduce) {
    .sidebar,
    main {
      transition: none;
    }
  }
</style>
