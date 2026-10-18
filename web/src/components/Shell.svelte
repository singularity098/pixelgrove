<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { User } from '../lib/api';
  import { link, navigate, route } from '../lib/router.svelte';
  import { signOut } from '../lib/session.svelte';
  import Icon, { type IconName } from './Icon.svelte';
  import Storage from './Storage.svelte';
  import ThemeMenu from './ThemeMenu.svelte';

  let { user, children }: { user: User; children: Snippet } = $props();

  const SIDEBAR_KEY = 'pixelgrove-sidebar';
  const narrow = window.matchMedia('(max-width: 900px)');
  let isNarrow = $state(narrow.matches);
  let pinned = $state(readPinned());
  let drawerOpen = $state(false);

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

  let searchText = $state(route.path === '/search' ? (route.query.get('q') ?? '') : '');
  let searchOpen = $state(false);
  let searchInput: HTMLInputElement | undefined = $state();

  // Keep the box in step with the URL: show the current query, clear it elsewhere.
  $effect(() => {
    searchText = route.path === '/search' ? (route.query.get('q') ?? '') : '';
  });

  function submitSearch(event: SubmitEvent) {
    event.preventDefault();
    const q = searchText.trim();
    if (!q) return;
    searchInput?.blur();
    searchOpen = false;
    navigate(`/search?q=${encodeURIComponent(q)}`);
  }

  function openSearch() {
    searchOpen = true;
    queueMicrotask(() => searchInput?.focus());
  }


  type Item = { href: string; label: string; icon: IconName };
  // View-only accounts have no favorites, sharing or trash.
  const editor = $derived(user.role !== 'viewer');
  const library = $derived<Item[]>([
    { href: '/', label: 'Photos', icon: 'photos' },
    { href: '/timeline', label: 'Timeline', icon: 'timeline' },
    { href: '/explore', label: 'Explore', icon: 'explore' },
    { href: '/map', label: 'Map', icon: 'map' },
    ...(editor ? [{ href: '/sharing', label: 'Sharing', icon: 'sharing' as IconName }] : []),
    ...(editor ? [{ href: '/favorites', label: 'Favorites', icon: 'favorite' as IconName }] : []),
    { href: '/files', label: 'File View', icon: 'files' },
    ...(editor ? [{ href: '/trash', label: 'Trash', icon: 'trash' as IconName }] : []),
  ]);
  const account = $derived<Item[]>([
    ...(user.role === 'admin'
      ? [
          { href: '/users', label: 'Users', icon: 'users' as IconName },
          { href: '/settings', label: 'Settings', icon: 'settings' as IconName },
        ]
      : []),
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
    <form class="search" class:open={searchOpen} role="search" onsubmit={submitSearch}>
      <Icon name="search" size={18} />
      <input
        bind:this={searchInput}
        bind:value={searchText}
        type="search"
        placeholder="Search your library"
        aria-label="Search your library"
        onblur={() => (searchOpen = false)} />
    </form>
    <div class="spacer"></div>
    <button class="icon search-toggle" onclick={openSearch} aria-label="Search"><Icon name="search" /></button>
    <ThemeMenu />
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
    <div class="bottom">
    <Storage />
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
    </div>
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

  .search {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: 0 1 560px;
    margin-left: 24px;
    padding: 0 14px;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text-muted);
  }

  .search:focus-within {
    background: var(--surface);
    box-shadow: 0 0 0 2px var(--accent);
  }

  .search input {
    margin: 0;
    padding: 9px 0;
    border: 0;
    background: transparent;
    outline: none;
  }

  .search-toggle {
    display: none;
  }

  @media (max-width: 700px) {
    .search {
      display: none;
      position: absolute;
      inset: 8px 8px auto;
      margin: 0;
      z-index: 2;
      background: var(--surface);
      box-shadow: 0 0 0 2px var(--accent);
    }

    .search.open {
      display: flex;
    }

    .search-toggle {
      display: grid;
    }
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
