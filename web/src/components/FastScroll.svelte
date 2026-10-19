<script lang="ts">
  /**
   * A handle on the right edge for touch screens, to drag through long pages quickly.
   * It shows while the page scrolls and fades out after; dragging it scrolls the page
   * to the same fraction. Headings marked `data-scroll-label` (Timeline days) are
   * shown in a bubble beside it while dragging.
   */
  const coarse = window.matchMedia('(pointer: coarse)');
  const HANDLE = 48;
  /** Pages shorter than this many screens don't need it. */
  const MIN_SCREENS = 4;

  let touch = $state(coarse.matches);
  let visible = $state(false);
  let dragging = $state(false);
  let fraction = $state(0);
  let label = $state('');
  let top = $state(0);
  let trackHeight = $state(0);
  let hideTimer: ReturnType<typeof setTimeout>;
  let grabOffset = 0;
  let labels: { y: number; text: string }[] = [];

  $effect(() => {
    const onChange = () => (touch = coarse.matches);
    coarse.addEventListener('change', onChange);
    return () => coarse.removeEventListener('change', onChange);
  });

  /** The page's own header (when there is one) stays clear of the handle. */
  function measureTrack() {
    top = Math.max(0, document.querySelector('header')?.getBoundingClientRect().bottom ?? 0);
    trackHeight = innerHeight - top - HANDLE;
  }

  const maxScroll = () => document.documentElement.scrollHeight - innerHeight;

  function onscroll() {
    if (!touch) return;
    const max = maxScroll();
    if (max < innerHeight * (MIN_SCREENS - 1)) {
      visible = false;
      return;
    }
    if (!dragging) {
      measureTrack();
      fraction = Math.min(1, scrollY / max);
    }
    visible = true;
    clearTimeout(hideTimer);
    if (!dragging) hideTimer = setTimeout(() => (visible = false), 1500);
  }

  function onpointerdown(event: PointerEvent) {
    event.preventDefault();
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    dragging = true;
    clearTimeout(hideTimer);
    measureTrack();
    grabOffset = event.clientY - (top + fraction * trackHeight);
    labels = [...document.querySelectorAll<HTMLElement>('[data-scroll-label]')].map((el) => ({
      y: el.getBoundingClientRect().top + scrollY,
      text: el.dataset.scrollLabel ?? '',
    }));
    updateLabel();
  }

  function onpointermove(event: PointerEvent) {
    if (!dragging) return;
    fraction = Math.min(1, Math.max(0, (event.clientY - grabOffset - top) / trackHeight));
    scrollTo(0, fraction * maxScroll());
    updateLabel();
  }

  function onpointerup() {
    if (!dragging) return;
    dragging = false;
    label = '';
    hideTimer = setTimeout(() => (visible = false), 1500);
  }

  /** The last labelled heading at or above the top of the screen. */
  function updateLabel() {
    const y = scrollY + top + 1;
    let lo = 0;
    let hi = labels.length - 1;
    let found = -1;
    while (lo <= hi) {
      const mid = (lo + hi) >> 1;
      if (labels[mid].y <= y) {
        found = mid;
        lo = mid + 1;
      } else hi = mid - 1;
    }
    label = labels[Math.max(0, found)]?.text ?? '';
  }
</script>

<svelte:window {onscroll} onresize={measureTrack} />

{#if touch}
  <div
    class="handle"
    class:shown={visible || dragging}
    class:dragging
    style:top="{top + fraction * trackHeight}px"
    aria-hidden="true"
    {onpointerdown}
    {onpointermove}
    {onpointerup}
    onpointercancel={onpointerup}>
    <svg width="14" height="20" viewBox="0 0 14 20" aria-hidden="true">
      <path d="M3 7l4-4 4 4M3 13l4 4 4-4" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" />
    </svg>
    {#if dragging && label}<span class="label">{label}</span>{/if}
  </div>
{/if}

<style>
  .handle {
    position: fixed;
    right: 0;
    z-index: 25;
    width: 36px;
    height: 48px;
    display: grid;
    place-items: center;
    border-radius: 24px 0 0 24px;
    color: var(--text);
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-right: 0;
    box-shadow: 0 2px 12px rgb(0 0 0 / 0.3);
    touch-action: none;
    user-select: none;
    opacity: 0;
    pointer-events: none;
    transform: translateX(8px);
    transition:
      opacity 0.2s,
      transform 0.2s;
  }

  .handle.shown {
    opacity: 1;
    pointer-events: auto;
    transform: none;
  }

  .handle.dragging {
    background: var(--accent);
    color: var(--accent-contrast);
    border-color: var(--accent);
  }

  .label {
    position: absolute;
    right: calc(100% + 12px);
    top: 50%;
    transform: translateY(-50%);
    padding: 8px 14px;
    border-radius: 999px;
    white-space: nowrap;
    font-weight: 600;
    font-size: 0.95rem;
    color: var(--text);
    background: var(--surface);
    border: 1px solid var(--border);
    box-shadow: 0 4px 16px rgb(0 0 0 / 0.35);
  }

  @media (prefers-reduced-motion: reduce) {
    .handle {
      transition: none;
    }
  }
</style>
