<script lang="ts">
  import { COLUMN_CHOICES, currentColumns, setColumns, type GridKind } from '../lib/columns';
  import { route } from '../lib/router.svelte';
  import Icon from './Icon.svelte';

  /** The header's button for how many columns grids have on phones: each tap steps
   *  2 → 3 → 4 → 2. Folder cards (the Photos page) and photos each keep their own
   *  count. Only shown on phone-sized screens, where it applies. */
  const kind: GridKind = $derived(route.path === '/' ? 'folders' : 'photos');
  // Bumped on each change, since the count lives outside Svelte's state.
  let changes = $state(0);
  const columns = $derived.by(() => {
    void changes;
    return currentColumns(kind);
  });

  function next() {
    setColumns(kind, COLUMN_CHOICES[(COLUMN_CHOICES.indexOf(columns) + 1) % COLUMN_CHOICES.length]);
    changes++;
  }

  const label = $derived(kind === 'folders' ? 'Folder columns' : 'Photo columns');
</script>

<button class="icon columns" onclick={next} aria-label="{label}: {columns}. Tap to change" title="{label}: {columns}">
  <Icon name="columns{columns}" />
</button>

<style>
  .columns {
    display: none;
  }

  @media (max-width: 520px) {
    .columns {
      display: grid;
    }
  }
</style>
