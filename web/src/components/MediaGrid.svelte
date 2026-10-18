<script lang="ts">
  import { formatDuration, libraryUrls, type MediaItem, type MediaUrls } from '../lib/media.svelte';
  import Icon from './Icon.svelte';
  import Thumb from './Thumb.svelte';

  let {
    items,
    onopen,
    urls = libraryUrls,
  }: { items: MediaItem[]; onopen: (index: number) => void; urls?: MediaUrls } = $props();
</script>

<div class="grid">
  {#each items as item, index (item.id)}
    <button class="tile" onclick={() => onopen(index)} aria-label="Open {item.name}">
      {#if item.kind === 'video'}
        <Thumb media={item} {urls} />
        <span class="badge"><Icon name="play" size={12} />{formatDuration(item.duration_ms)}</span>
      {:else}
        <img src={urls.thumb(item)} alt="" loading="lazy" decoding="async" />
      {/if}
    </button>
  {/each}
</div>

<style>
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: 4px;
  }

  .tile {
    position: relative;
    aspect-ratio: 1;
    padding: 0;
    border: 0;
    border-radius: 4px;
    overflow: hidden;
    background: var(--surface-2);
  }

  .tile:focus-visible {
    outline: 3px solid var(--accent);
    outline-offset: -3px;
  }

  .tile :global(img),
  .tile :global(.placeholder) {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: cover;
    pointer-events: none;
    transition: transform 0.15s ease;
  }

  .tile:hover :global(img) {
    transform: scale(1.03);
  }

  .badge {
    position: absolute;
    right: 6px;
    bottom: 6px;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 2px 7px;
    border-radius: 999px;
    font-size: 0.75rem;
    color: #fff;
    background: rgb(0 0 0 / 0.6);
  }

  @media (max-width: 520px) {
    .grid {
      grid-template-columns: repeat(3, 1fr);
      gap: 2px;
    }

    .tile {
      border-radius: 0;
    }
  }
</style>
