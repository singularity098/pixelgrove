<script lang="ts">
  /**
   * Plays a Matroska video that the server repackages for the browser on the fly. That
   * stream can't be seeked by the browser, so this player has its own seek bar: seeking
   * asks the server for the stream again from the new time (`&t=`). If the server can't
   * stream it (no ffmpeg, or a codec it can't repackage), the player falls back to the
   * original file, which the browser seeks itself with its usual controls.
   */
  import Icon from './Icon.svelte';
  import { releaseOnDestroy } from '../lib/video';

  let { src, poster, durationMs }: { src: string; poster: string; durationMs: number | null } = $props();

  let root = $state<HTMLElement>();
  let video = $state<HTMLVideoElement>();
  /** Where the current stream starts, in seconds of the video. */
  let offset = $state(0);
  let current = $state(0);
  let duration = $state(0);
  let paused = $state(true);
  let muted = $state(false);
  /** Playing the original file instead of the stream. */
  let native = $state(false);
  /** The seek bar's value while it's being dragged. */
  let dragging = $state<number | null>(null);

  const url = $derived(native ? src : `${src}&t=${offset.toFixed(3)}`);
  const total = $derived(durationMs ? durationMs / 1000 : offset + (Number.isFinite(duration) ? duration : 0));
  const position = $derived(dragging ?? Math.min(offset + current, total || Infinity));

  function onerror() {
    if (native) return;
    native = true;
    offset = 0;
  }

  function seek(to: number) {
    dragging = null;
    if (native) {
      video!.currentTime = to;
      return;
    }
    // Browsers won't seek within the stream, even where it's buffered: start it over.
    const start = Math.max(0, Math.min(to, total - 1));
    current = 0;
    if (start === offset) video?.load();
    else offset = start;
  }

  function toggle() {
    if (!video) return;
    if (video.paused) video.play().catch(() => {});
    else video.pause();
  }

  function fullscreen() {
    if (document.fullscreenElement) document.exitFullscreen().catch(() => {});
    else root?.requestFullscreen?.().catch(() => {});
  }

  function onkeydown(event: KeyboardEvent) {
    if (native || (event.target as Element).closest?.('input, button, textarea')) return;
    if (event.key === ' ' || event.key === 'k') toggle();
    else return;
    event.preventDefault();
  }

  function clock(seconds: number) {
    const s = Math.max(0, Math.floor(seconds));
    const h = Math.floor(s / 3600);
    const mm = String(Math.floor((s % 3600) / 60)).padStart(h ? 2 : 1, '0');
    return `${h ? `${h}:` : ''}${mm}:${String(s % 60).padStart(2, '0')}`;
  }
</script>

<svelte:window {onkeydown} />

<div class="player" bind:this={root}>
  <!-- svelte-ignore a11y_media_has_caption -->
  <video
    bind:this={video}
    src={url}
    {poster}
    controls={native}
    autoplay
    playsinline
    bind:currentTime={current}
    bind:duration
    bind:paused
    bind:muted
    {onerror}
    onclick={() => !native && toggle()}
    use:releaseOnDestroy></video>
  {#if !native}
    <div class="bar">
      <button onclick={toggle} aria-label={paused ? 'Play' : 'Pause'} title={paused ? 'Play (space)' : 'Pause (space)'}>
        <Icon name={paused ? 'play' : 'pause'} size={18} />
      </button>
      <span class="time">{clock(position)}{total ? ` / ${clock(total)}` : ''}</span>
      <input
        type="range"
        min="0"
        max={total || 0}
        step="any"
        value={position}
        disabled={!total}
        aria-label="Seek"
        oninput={(e) => (dragging = Number(e.currentTarget.value))}
        onchange={(e) => seek(Number(e.currentTarget.value))} />
      <button onclick={() => (muted = !muted)} aria-label={muted ? 'Unmute' : 'Mute'} title={muted ? 'Unmute' : 'Mute'}>
        <Icon name={muted ? 'mute' : 'volume'} size={18} />
      </button>
      <button onclick={fullscreen} aria-label="Full screen" title="Full screen"><Icon name="fullscreen" size={18} /></button>
    </div>
  {/if}
</div>

<style>
  .player {
    position: absolute;
    inset: 0;
    background: #000;
  }

  video {
    width: 100%;
    height: 100%;
    object-fit: contain;
  }

  .bar {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 12px calc(8px + env(safe-area-inset-bottom));
    background: linear-gradient(transparent, rgb(0 0 0 / 0.7));
    color: #fff;
  }

  button {
    display: grid;
    place-items: center;
    width: 36px;
    height: 36px;
    border: 0;
    border-radius: 50%;
    background: transparent;
    color: inherit;
    cursor: pointer;
  }

  button:hover {
    background: rgb(255 255 255 / 0.15);
  }

  .time {
    font-size: 13px;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  input {
    flex: 1;
    min-width: 0;
    accent-color: #fff;
  }
</style>
