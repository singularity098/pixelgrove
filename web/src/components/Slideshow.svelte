<script lang="ts">
  import { untrack } from 'svelte';
  import { fade } from 'svelte/transition';
  import { formatDate, libraryUrls, type MediaItem, type MediaSource, type MediaUrls } from '../lib/media.svelte';
  import { INTERVALS, saveSlideshowSettings, slideshowSettings as settings } from '../lib/slideshow.svelte';
  import Icon from './Icon.svelte';
  import { releaseOnDestroy } from '../lib/video';

  /**
   * A full-screen slideshow over `source`, loading more of it as it goes. Photos are
   * shown for the chosen interval; videos play to the end.
   */
  let {
    source,
    startId = null,
    urls = libraryUrls,
    allowSubfolders = false,
    onclose,
  }: {
    source: MediaSource;
    /** The item to start on (the open photo); otherwise the first, or a random one. */
    startId?: number | null;
    urls?: MediaUrls;
    /** Offer "include subfolders" (the parent swaps `source` when it changes). */
    allowSubfolders?: boolean;
    /** Called with the last item shown. */
    onclose: (lastId: number | null) => void;
  } = $props();

  let root: HTMLDivElement | undefined = $state();
  let current = $state<MediaItem | null>(null);
  let empty = $state(false);
  let playing = $state(true);
  let muted = $state(false);
  let controls = $state(true);
  let panel = $state(false);
  let fullscreen = $state(false);
  // Restarts the progress bar each time a photo's timer starts.
  let tick = $state(0);
  // Element full screen is missing on iPhone.
  const canFullscreen = typeof document.documentElement.requestFullscreen === 'function';

  // What has been shown, for going back; `pos` is the current entry.
  let shown: MediaItem[] = [];
  let pos = $state(-1);
  // Shuffle: ids already shown in this round.
  const played = new Set<number>();
  // The next item, chosen and preloaded ahead of time.
  let upcoming: Promise<MediaItem | null> | null = null;
  // Bumped on every navigation so a slow preload can't overwrite a newer choice.
  let seq = 0;

  const allowed = (m: MediaItem) => settings.videos || m.kind === 'image';
  const done = () => source.total !== null && source.items.length >= source.total;
  const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

  /** Loads another page; false once there's nothing more (waits out a load already in flight). */
  async function loadMore(): Promise<boolean> {
    for (let attempt = 0; attempt < 50; attempt++) {
      if (done()) return false;
      const before = source.items.length;
      if ((await source.more()) || source.items.length > before) return true;
      await sleep(200);
    }
    return false;
  }

  /** The next item in order after `from`, wrapping around at the end. */
  async function nextInOrder(from: MediaItem | null): Promise<MediaItem | null> {
    let i = from ? source.items.findIndex((m) => m.id === from.id) : -1;
    let wrapped = false;
    for (;;) {
      i++;
      if (i >= source.items.length && !(await loadMore())) {
        if (wrapped || !source.items.length) return null;
        wrapped = true;
        i = 0;
      }
      const m = source.items[i];
      if (m && allowed(m)) return m;
      if (wrapped && from && m?.id === from.id) return allowed(from) ? from : null;
    }
  }

  async function nextShuffled(): Promise<MediaItem | null> {
    // Keep growing the pool in the background so later picks cover the whole set.
    if (!done()) void source.more();
    const pool = () => source.items.filter((m) => allowed(m) && !played.has(m.id) && m.id !== current?.id);
    let candidates = pool();
    while (!candidates.length && (await loadMore())) candidates = pool();
    if (!candidates.length) {
      // Everything has been shown: start a new round.
      played.clear();
      candidates = pool();
      if (!candidates.length) return current && allowed(current) ? current : null;
    }
    return candidates[Math.floor(Math.random() * candidates.length)];
  }

  function preload(m: MediaItem | null): Promise<void> {
    if (!m || m.kind !== 'image') return Promise.resolve();
    const img = new Image();
    img.src = urls.large(m);
    return img.decode().catch(() => {});
  }

  /** Chooses the next item and starts loading it. */
  function prepare() {
    const from = current;
    upcoming = (settings.shuffle ? nextShuffled() : nextInOrder(from)).then(async (m) => {
      await preload(m);
      return m;
    });
  }

  async function show(m: MediaItem | null, mine: number) {
    if (mine !== seq) return;
    if (!m) {
      empty = !current;
      return;
    }
    empty = false;
    current = m;
    played.add(m.id);
    prepare();
  }

  async function next() {
    const mine = ++seq;
    if (pos < shown.length - 1) {
      const m = shown[++pos];
      await preload(m);
      show(m, mine);
      return;
    }
    const m = await (upcoming ?? (prepare(), upcoming!));
    if (mine !== seq || !m) return show(m, mine);
    shown.push(m);
    if (shown.length > 500) shown.shift();
    pos = shown.length - 1;
    show(m, mine);
  }

  async function prev() {
    if (pos <= 0) return;
    const mine = ++seq;
    const m = shown[--pos];
    await preload(m);
    show(m, mine);
  }

  /** Starts over (on open, and when the source or a setting changes the order). */
  async function restart(first: MediaItem | null) {
    const mine = ++seq;
    shown = [];
    pos = -1;
    played.clear();
    upcoming = null;
    if (!source.items.length) await loadMore();
    if (mine !== seq) return;
    const start =
      first && allowed(first)
        ? first
        : settings.shuffle
          ? await nextShuffled()
          : await nextInOrder(null);
    if (mine !== seq) return;
    if (start) {
      shown = [start];
      pos = 0;
      await preload(start);
    }
    show(start, mine);
  }

  // (Re)start whenever the source changes, e.g. subfolders switched on.
  $effect(() => {
    source;
    untrack(() => {
      const first = startId === null ? null : (source.items.find((m) => m.id === startId) ?? null);
      restart(current ?? first);
    });
  });

  // Photos advance on a timer; videos when they end.
  $effect(() => {
    if (!playing || !current || current.kind !== 'image') return;
    untrack(() => tick++);
    const timer = setTimeout(next, settings.interval * 1000);
    return () => clearTimeout(timer);
  });

  function setting<K extends keyof typeof settings>(key: K, value: (typeof settings)[K]) {
    settings[key] = value;
    saveSlideshowSettings();
    // The prepared next item may no longer fit.
    if (key === 'shuffle' || key === 'videos') {
      played.clear();
      if (current) played.add(current.id);
      shown = shown.slice(0, pos + 1);
      if (current) prepare();
      if (key === 'videos' && current && !allowed(current)) next();
    }
  }

  // Controls fade out after a few seconds without movement.
  let hideTimer: ReturnType<typeof setTimeout>;
  function wake() {
    controls = true;
    clearTimeout(hideTimer);
    hideTimer = setTimeout(() => {
      if (!panel) controls = false;
    }, 3000);
  }
  $effect(() => {
    wake();
    return () => clearTimeout(hideTimer);
  });

  // Full screen and keeping the screen awake, where the browser allows.
  $effect(() => {
    root?.requestFullscreen?.().catch(() => {});
    const onchange = () => (fullscreen = !!document.fullscreenElement);
    document.addEventListener('fullscreenchange', onchange);
    let lock: { release(): Promise<void> } | null = null;
    const wakeLock = (navigator as { wakeLock?: { request(type: 'screen'): Promise<{ release(): Promise<void> }> } })
      .wakeLock;
    const acquire = () => {
      if (document.visibilityState === 'visible')
        wakeLock?.request('screen').then(
          (l) => (lock = l),
          () => {},
        );
    };
    acquire();
    document.addEventListener('visibilitychange', acquire);
    const overflow = document.body.style.overflow;
    document.body.style.overflow = 'hidden';
    return () => {
      document.removeEventListener('fullscreenchange', onchange);
      document.removeEventListener('visibilitychange', acquire);
      lock?.release().catch(() => {});
      if (document.fullscreenElement) document.exitFullscreen().catch(() => {});
      document.body.style.overflow = overflow;
    };
  });

  function toggleFullscreen() {
    if (document.fullscreenElement) document.exitFullscreen().catch(() => {});
    else root?.requestFullscreen?.().catch(() => {});
  }

  const close = () => onclose(current?.id ?? null);

  function onkeydown(event: KeyboardEvent) {
    if (event.target instanceof HTMLSelectElement || event.target instanceof HTMLInputElement) return;
    wake();
    switch (event.key) {
      case 'Escape':
        if (panel) panel = false;
        else close();
        break;
      case ' ':
      case 'k':
        togglePlay();
        break;
      case 'ArrowRight':
        next();
        break;
      case 'ArrowLeft':
        prev();
        break;
      default:
        return;
    }
    event.preventDefault();
    event.stopImmediatePropagation();
  }

  // Swipe to change slides; tap to show or hide the controls.
  let touch: { x: number; y: number } | null = null;
  function onpointerdown(event: PointerEvent) {
    if (event.target !== event.currentTarget && !(event.target as Element).closest('.slide')) return;
    touch = { x: event.clientX, y: event.clientY };
  }
  function onpointerup(event: PointerEvent) {
    if (!touch) return;
    const dx = event.clientX - touch.x;
    const dy = event.clientY - touch.y;
    touch = null;
    if (Math.abs(dx) > 60 && Math.abs(dx) > Math.abs(dy)) {
      if (dx < 0) next();
      else prev();
      wake();
    } else if (Math.abs(dx) < 10 && Math.abs(dy) < 10) {
      if (event.pointerType === 'mouse') wake();
      else if (controls) {
        controls = false;
        panel = false;
      } else wake();
    }
  }

  /** A video that won't play: move on after a moment (not instantly, so a run of them can't spin). */
  function skipBroken() {
    const id = current?.id;
    setTimeout(() => {
      if (current?.id === id) next();
    }, 1500);
  }

  function videoReady(video: HTMLVideoElement) {
    // Sound is fine after the click that started the slideshow, but fall back to muted.
    video.play().catch(() => {
      muted = true;
      video.muted = true;
      video.play().catch(skipBroken);
    });
  }

  function togglePlay() {
    playing = !playing;
    // Pausing the slideshow pauses a playing video too.
    for (const video of root?.querySelectorAll<HTMLVideoElement>('video.slide') ?? []) {
      if (!playing) video.pause();
      else if (!video.ended) video.play().catch(() => {});
    }
  }
</script>

<svelte:window {onkeydown} />

<div
  class="slideshow"
  class:idle={!controls}
  bind:this={root}
  role="dialog"
  aria-modal="true"
  aria-label="Slideshow"
  tabindex="-1"
  onpointermove={(e) => e.pointerType === 'mouse' && wake()}
  {onpointerdown}
  {onpointerup}>
  {#if current}
    {#key current.id}
      {#if current.kind === 'video'}
        <!-- svelte-ignore a11y_media_has_caption -->
        <video
          class="slide"
          src={urls.file(current) ?? ''}
          playsinline
          {muted}
          onloadeddata={(e) => videoReady(e.currentTarget)}
          onended={() => playing && next()}
          onerror={skipBroken}
          use:releaseOnDestroy
          transition:fade={{ duration: 700 }}></video>
      {:else}
        <img class="slide" src={urls.large(current)} alt={current.name} draggable="false" transition:fade={{ duration: 700 }} />
      {/if}
    {/key}
  {:else if empty}
    <p class="empty">Nothing to show{settings.videos ? '' : ' (videos are switched off)'}.</p>
  {/if}

  <div class="top" class:hidden={!controls}>
    {#if current}
      <div class="caption">
        <div class="name">{current.name}</div>
        <div class="date">{formatDate(current.taken_at, true)}</div>
      </div>
    {/if}
    <button class="icon" onclick={close} aria-label="Close slideshow" title="Close (Esc)"><Icon name="close" /></button>
  </div>

  {#if playing && current?.kind === 'image'}
    {#key tick}
      <div class="progress" style="animation-duration: {settings.interval}s"></div>
    {/key}
  {/if}

  <div class="bar" class:hidden={!controls}>
    {#if panel}
      <div class="panel" role="group" aria-label="Slideshow settings">
        <label>
          Each photo
          <select value={settings.interval} onchange={(e) => setting('interval', Number(e.currentTarget.value))}>
            {#each INTERVALS as seconds (seconds)}
              <option value={seconds}>{seconds} seconds</option>
            {/each}
          </select>
        </label>
        <label class="check">
          <input type="checkbox" checked={settings.shuffle} onchange={(e) => setting('shuffle', e.currentTarget.checked)} />
          Shuffle
        </label>
        <label class="check">
          <input type="checkbox" checked={settings.videos} onchange={(e) => setting('videos', e.currentTarget.checked)} />
          Include videos
        </label>
        {#if allowSubfolders}
          <label class="check">
            <input type="checkbox" checked={settings.subfolders} onchange={(e) => setting('subfolders', e.currentTarget.checked)} />
            Include subfolders
          </label>
        {/if}
      </div>
    {/if}
    <div class="buttons">
      <button class="icon" onclick={prev} disabled={pos <= 0} aria-label="Previous" title="Previous (←)"><Icon name="left" /></button>
      <button class="icon big" onclick={togglePlay} aria-label={playing ? 'Pause' : 'Play'} title={playing ? 'Pause (space)' : 'Play (space)'}>
        <Icon name={playing ? 'pause' : 'play'} size={24} />
      </button>
      <button class="icon" onclick={next} aria-label="Next" title="Next (→)"><Icon name="right" /></button>
      <span class="gap"></span>
      <button class="icon" class:on={settings.shuffle} aria-pressed={settings.shuffle} onclick={() => setting('shuffle', !settings.shuffle)} aria-label="Shuffle" title="Shuffle">
        <Icon name="shuffle" />
      </button>
      {#if current?.kind === 'video'}
        <button class="icon" onclick={() => (muted = !muted)} aria-label={muted ? 'Unmute' : 'Mute'} title={muted ? 'Unmute' : 'Mute'}>
          <Icon name={muted ? 'mute' : 'volume'} />
        </button>
      {/if}
      <button class="icon" class:on={panel} aria-expanded={panel} onclick={() => (panel = !panel)} aria-label="Slideshow settings" title="Settings">
        <Icon name="settings" />
      </button>
      {#if canFullscreen}
        <button class="icon" onclick={toggleFullscreen} aria-label={fullscreen ? 'Exit full screen' : 'Full screen'} title={fullscreen ? 'Exit full screen' : 'Full screen'}>
          <Icon name={fullscreen ? 'exitFullscreen' : 'fullscreen'} />
        </button>
      {/if}
    </div>
  </div>
</div>

<style>
  .slideshow {
    position: fixed;
    inset: 0;
    z-index: 150;
    background: #000;
    color: #f1f3f4;
    overflow: hidden;
    touch-action: none;
    user-select: none;
    outline: none;
  }

  .slideshow.idle {
    cursor: none;
  }

  .slide {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: contain;
  }

  .empty {
    position: absolute;
    top: 50%;
    width: 100%;
    text-align: center;
    color: #b0b6bc;
  }

  .top,
  .bar {
    position: absolute;
    left: 0;
    right: 0;
    z-index: 2;
    transition: opacity 0.3s;
  }

  .hidden {
    opacity: 0;
    pointer-events: none;
  }

  .top {
    top: 0;
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 10px 10px 32px 16px;
    padding-top: max(10px, env(safe-area-inset-top));
    background: linear-gradient(rgb(0 0 0 / 0.55), transparent);
  }

  .caption {
    flex: 1;
    min-width: 0;
    padding-top: 4px;
  }

  .name {
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .date {
    font-size: 0.8rem;
    color: #b0b6bc;
  }

  .bar {
    bottom: 0;
    padding: 32px 12px 12px;
    padding-bottom: max(12px, env(safe-area-inset-bottom));
    background: linear-gradient(transparent, rgb(0 0 0 / 0.6));
  }

  .buttons {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
  }

  .gap {
    width: 16px;
  }

  button.icon {
    width: 44px;
    height: 44px;
    display: grid;
    place-items: center;
    padding: 0;
    border: 0;
    border-radius: 999px;
    background: transparent;
    color: #f1f3f4;
  }

  button.icon.big {
    width: 56px;
    height: 56px;
    background: rgb(255 255 255 / 0.14);
  }

  button.icon:hover:not(:disabled),
  button.icon.on {
    background: rgb(255 255 255 / 0.2);
  }

  button.icon:disabled {
    opacity: 0.35;
  }

  .panel {
    width: min(320px, 100%);
    margin: 0 auto 12px;
    padding: 14px 16px;
    display: grid;
    gap: 12px;
    border-radius: 14px;
    background: rgb(30 33 37 / 0.95);
    border: 1px solid #2c3137;
  }

  .panel label {
    display: grid;
    gap: 6px;
    font-size: 0.9rem;
  }

  .panel label.check {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .panel select {
    background: #22262b;
    color: #f1f3f4;
    border-color: #2c3137;
  }

  .progress {
    position: absolute;
    left: 0;
    bottom: 0;
    z-index: 3;
    height: 3px;
    width: 100%;
    background: var(--accent, #4caf7d);
    opacity: 0.8;
    transform-origin: left;
    animation-name: progress;
    animation-timing-function: linear;
    animation-fill-mode: forwards;
  }

  .idle .progress {
    opacity: 0.35;
  }

  @keyframes progress {
    from {
      transform: scaleX(0);
    }
    to {
      transform: scaleX(1);
    }
  }
</style>
