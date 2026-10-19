<script lang="ts">
  import { onVisible } from '../lib/router.svelte';

  /**
   * The end of a photo grid that loads in pages. It takes up the room the photos not
   * loaded yet will need (the grid's tiles are square), so the page is as long as the
   * whole list: the scrollbar and fast scroll reach any point, and whatever part of
   * it comes into view loads. Goes right after the grid.
   */
  let { list }: { list: { items: unknown[]; total: number | null; more: () => Promise<boolean> } } = $props();

  let node: HTMLDivElement | undefined = $state();
  let height = $state(0);

  function measure() {
    const grid = node?.previousElementSibling as HTMLElement | null;
    const tile = grid?.firstElementChild as HTMLElement | null;
    if (!grid || !tile || list.total === null) {
      height = 0;
      return;
    }
    const style = getComputedStyle(grid);
    const columns = style.gridTemplateColumns.split(' ').length;
    const gap = parseFloat(style.rowGap) || 0;
    const rows = Math.ceil(list.total / columns) - Math.ceil(list.items.length / columns);
    height = Math.max(0, rows) * (tile.offsetHeight + gap);
  }

  $effect(() => {
    void list.items.length;
    void list.total;
    measure();
  });

  // Column count and tile size change with the window and the columns button.
  $effect(() => {
    const grid = node?.previousElementSibling;
    if (!grid) return;
    const observer = new ResizeObserver(measure);
    observer.observe(grid);
    return () => observer.disconnect();
  });
</script>

<div bind:this={node} class="more" style:height="{height}px" use:onVisible={() => list.more()}></div>

<style>
  .more {
    /* Never collapses to nothing, so the observer always has something to watch. */
    min-height: 1px;
  }
</style>
