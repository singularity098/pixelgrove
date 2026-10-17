<script lang="ts">
  import type { MediaSource } from '../lib/media.svelte';
  import { setParam } from '../lib/query';
  import { route } from '../lib/router.svelte';
  import Viewer from './Viewer.svelte';

  /**
   * Shows the viewer for `?m=<media id>` over any page, paging through `source`.
   * Keeping the open item in the URL means Back closes the viewer and links work.
   */
  let { source }: { source: MediaSource } = $props();

  const openId = $derived(Number(route.query.get('m')) || null);
  const openIndex = $derived(openId === null ? -1 : source.items.findIndex((m) => m.id === openId));
  let opened = false;

  // Opened from a link to an item that isn't loaded yet: keep loading until it is.
  $effect(() => {
    if (openId !== null && openIndex < 0) source.more();
  });

  $effect(() => {
    if (openIndex >= 0) opened = true;
  });

  function show(index: number) {
    setParam('m', String(source.items[index].id), true);
  }

  /** The open item left the list (moved to the trash): show its neighbour. */
  function removed(id: number) {
    const index = source.items.findIndex((m) => m.id === id);
    source.remove(id);
    if (!source.items.length) {
      close();
      return;
    }
    const next = source.items[Math.min(index, source.items.length - 1)];
    setParam('m', String(next.id), true);
  }

  function close() {
    // If we pushed the viewer entry ourselves, pop it so Back doesn't reopen it.
    if (opened && history.state?.viewer) history.back();
    else setParam('m', null, true);
  }
</script>

{#if openIndex >= 0}
  <Viewer
    items={source.items}
    index={openIndex}
    total={source.total ?? source.items.length}
    onindex={show}
    onclose={close}
    onremoved={removed}
    onneedmore={() => source.more()} />
{/if}
