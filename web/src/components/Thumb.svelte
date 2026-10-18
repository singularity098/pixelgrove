<script lang="ts">
  import { libraryUrls, type Cover, type MediaUrls } from '../lib/media.svelte';
  import { cachedFrame, frameOf, serverFrames } from '../lib/video';

  /** A cover or tile image; videos show a frame (made by the server, or grabbed here). */
  let {
    media,
    alt = '',
    urls = libraryUrls,
  }: { media: Cover | null; alt?: string; urls?: MediaUrls } = $props();
  let failed = $state(false);

  // Videos: a frame grabbed once the tile is near the screen (see lib/video.ts).
  const videoUrl = $derived(media?.kind === 'video' ? urls.file({ ...media, kind: 'video' }) : null);
  let frame = $state<string | null>(null);
  // Try the server's thumbnail first; grab a frame here only if it has none.
  let grab = $state(!serverFrames.available);
  $effect(() => {
    frame = videoUrl ? (cachedFrame(videoUrl) ?? null) : null;
  });

  function noServerFrame() {
    serverFrames.available = false;
    grab = true;
  }

  function grabWhenVisible(node: HTMLElement, url: string) {
    let job: ReturnType<typeof frameOf> | null = null;
    const observer = new IntersectionObserver(
      (entries) => {
        if (!entries.some((e) => e.isIntersecting) || job) return;
        job = frameOf(url);
        job.promise.then((f) => {
          if (f) frame = f;
          else failed = true;
        });
        observer.disconnect();
      },
      { rootMargin: '300px' },
    );
    observer.observe(node);
    return {
      destroy() {
        observer.disconnect();
        job?.cancel();
      },
    };
  }
</script>

{#if !media || failed}
  <div class="placeholder"></div>
{:else if videoUrl && !grab}
  <img src={urls.thumb(media)} {alt} loading="lazy" decoding="async" onerror={noServerFrame} />
{:else if videoUrl}
  {#if frame}
    <img src={frame} {alt} />
  {:else}
    <div class="placeholder" use:grabWhenVisible={videoUrl}></div>
  {/if}
{:else}
  <img src={urls.thumb(media)} {alt} loading="lazy" decoding="async" onerror={() => (failed = true)} />
{/if}

<style>
  img,
  .placeholder {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: cover;
    background: var(--surface-2);
    pointer-events: none;
  }
</style>
