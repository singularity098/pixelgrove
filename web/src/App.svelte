<script lang="ts">
  import FastScroll from './components/FastScroll.svelte';
  import Shell from './components/Shell.svelte';
  import Account from './pages/Account.svelte';
  import Folder from './pages/Folder.svelte';
  import Login from './pages/Login.svelte';
  import Explore from './pages/Explore.svelte';
  import ExploreItems from './pages/ExploreItems.svelte';
  import Favorites from './pages/Favorites.svelte';
  import Files from './pages/Files.svelte';
  import Trash from './pages/Trash.svelte';
  import Photos from './pages/Photos.svelte';
  import Search from './pages/Search.svelte';
  import Shared from './pages/Shared.svelte';
  import Sharing from './pages/Sharing.svelte';
  import Timeline from './pages/Timeline.svelte';
  import Setup from './pages/Setup.svelte';
  import Settings from './pages/Settings.svelte';
  import Users from './pages/Users.svelte';
  import { route } from './lib/router.svelte';
  import { loadSession, session } from './lib/session.svelte';

  // Share links are public: they don't need (or check) a session.
  const shareToken = /^\/s\/([0-9a-f]{32})$/.exec(location.pathname)?.[1] ?? null;
  if (!shareToken) loadSession();


  const folderId = $derived(Number(/^\/folder\/(\d+)$/.exec(route.path)?.[1]) || null);
  const filesMatch = $derived(/^\/files(?:\/(\d+))?$/.exec(route.path));
  const searchQuery = $derived(route.path === '/search' ? (route.query.get('q') ?? '').trim() : '');
</script>

<FastScroll />

{#if shareToken}
  <Shared token={shareToken} />
{:else if session.current.status === 'setup'}
  <Setup />
{:else if session.current.status === 'signed-out'}
  <Login />
{:else if session.current.status === 'signed-in'}
  {@const user = session.current.user}
  <Shell {user}>
    <!-- Turning "show hidden" on or off reloads the page's lists. -->
    {#key user.show_hidden}
    {#if folderId !== null}
      {#key folderId}
        <Folder id={folderId} />
      {/key}
    {:else if filesMatch}
      {#key filesMatch[1]}
        <Files id={filesMatch[1] ? Number(filesMatch[1]) : null} />
      {/key}
    {:else if route.path === '/sharing' && user.role !== 'viewer'}
      <Sharing me={user} />
    {:else if route.path === '/favorites' && user.role !== 'viewer'}
      <Favorites />
    {:else if route.path === '/trash' && user.role !== 'viewer'}
      <Trash />
    {:else if route.path === '/timeline'}
      <Timeline />
    {:else if searchQuery}
      {#key searchQuery}
        <Search query={searchQuery} />
      {/key}
    {:else if route.path === '/users' && user.role === 'admin'}
      <Users me={user} />
    {:else if route.path === '/settings' && user.role === 'admin'}
      <Settings />
    {:else if route.path === '/account'}
      <Account {user} />
    {:else if route.path === '/explore'}
      <Explore />
    {:else if route.path === '/explore/items'}
      {#key route.query.toString()}
        <ExploreItems query={route.query} />
      {/key}
    {:else if route.path === '/map'}
      <!-- Leaflet is loaded only when the map is opened. -->
      {#await import('./pages/Map.svelte') then { default: Map }}
        <Map />
      {/await}
    {:else}
      <Photos />
    {/if}
    {/key}
  </Shell>
{/if}
