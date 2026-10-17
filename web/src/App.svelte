<script lang="ts">
  import TopBar from './components/TopBar.svelte';
  import Account from './pages/Account.svelte';
  import Home from './pages/Home.svelte';
  import Login from './pages/Login.svelte';
  import Setup from './pages/Setup.svelte';
  import Users from './pages/Users.svelte';
  import { route } from './lib/router.svelte';
  import { loadSession, session } from './lib/session.svelte';

  loadSession();
</script>

{#if session.current.status === 'setup'}
  <Setup />
{:else if session.current.status === 'signed-out'}
  <Login />
{:else if session.current.status === 'signed-in'}
  {@const user = session.current.user}
  <TopBar {user} />
  {#if route.path === '/users' && user.role === 'admin'}
    <Users me={user} />
  {:else if route.path === '/account'}
    <Account {user} />
  {:else}
    <Home />
  {/if}
{/if}
