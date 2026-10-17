<script lang="ts">
  import Shell from './components/Shell.svelte';
  import Account from './pages/Account.svelte';
  import ComingSoon from './pages/ComingSoon.svelte';
  import Folder from './pages/Folder.svelte';
  import Login from './pages/Login.svelte';
  import Favorites from './pages/Favorites.svelte';
  import Files from './pages/Files.svelte';
  import Trash from './pages/Trash.svelte';
  import Photos from './pages/Photos.svelte';
  import Search from './pages/Search.svelte';
  import Timeline from './pages/Timeline.svelte';
  import Setup from './pages/Setup.svelte';
  import Users from './pages/Users.svelte';
  import { route } from './lib/router.svelte';
  import { loadSession, session } from './lib/session.svelte';

  loadSession();

  const upcoming: Record<string, [string, string]> = {
    '/sharing': ['Sharing', 'M6'],
    '/map': ['Map', 'M7'],
    '/explore': ['Explore', 'M7'],
  };

  const folderId = $derived(Number(/^\/folder\/(\d+)$/.exec(route.path)?.[1]) || null);
  const filesMatch = $derived(/^\/files(?:\/(\d+))?$/.exec(route.path));
  const searchQuery = $derived(route.path === '/search' ? (route.query.get('q') ?? '').trim() : '');
</script>

{#if session.current.status === 'setup'}
  <Setup />
{:else if session.current.status === 'signed-out'}
  <Login />
{:else if session.current.status === 'signed-in'}
  {@const user = session.current.user}
  <Shell {user}>
    {#if folderId !== null}
      {#key folderId}
        <Folder id={folderId} />
      {/key}
    {:else if filesMatch}
      {#key filesMatch[1]}
        <Files id={filesMatch[1] ? Number(filesMatch[1]) : null} />
      {/key}
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
    {:else if route.path === '/account'}
      <Account {user} />
    {:else if upcoming[route.path]}
      <ComingSoon title={upcoming[route.path][0]} milestone={upcoming[route.path][1]} />
    {:else}
      <Photos />
    {/if}
  </Shell>
{/if}
