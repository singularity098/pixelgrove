<script lang="ts">
  import { COLUMN_CHOICES, currentColumns, setColumns } from '../lib/columns';
  import Icon from './Icon.svelte';

  /** The header's button for how many columns photo grids have on phones: each tap
   *  steps 2 → 3 → 4 → 2. Only shown on phone-sized screens, where it applies. */
  let columns = $state(currentColumns());

  function next() {
    columns = COLUMN_CHOICES[(COLUMN_CHOICES.indexOf(columns) + 1) % COLUMN_CHOICES.length];
    setColumns(columns);
  }
</script>

<button class="icon columns" onclick={next} aria-label="Grid columns: {columns}. Tap to change" title="{columns} columns">
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
