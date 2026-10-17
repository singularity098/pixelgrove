<script lang="ts">
  import { libraryUrls, type Cover, type MediaUrls } from '../lib/media.svelte';

  /** A cover or tile image; videos show their first frame. */
  let {
    media,
    alt = '',
    urls = libraryUrls,
  }: { media: Cover | null; alt?: string; urls?: MediaUrls } = $props();
  let failed = $state(false);
</script>

{#if !media || failed}
  <div class="placeholder"></div>
{:else if media.kind === 'video'}
  <video src="{urls.file(media)}#t=0.5" preload="metadata" muted playsinline disablepictureinpicture tabindex="-1"></video>
{:else}
  <img src={urls.thumb(media)} {alt} loading="lazy" decoding="async" onerror={() => (failed = true)} />
{/if}

<style>
  img,
  video,
  .placeholder {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: cover;
    background: var(--surface-2);
    pointer-events: none;
  }
</style>
