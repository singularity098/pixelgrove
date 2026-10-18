<script lang="ts">
  import { del, get, post, put } from '../lib/api';
  import { session } from '../lib/session.svelte';
  import {
    formatBytes,
    formatDate,
    formatDuration,
    libraryUrls,
    type MediaDetail,
    type MediaItem,
    type MediaUrls,
  } from '../lib/media.svelte';
  import Icon from './Icon.svelte';
  import ShareDialog from './ShareDialog.svelte';
  import Slideshow from './Slideshow.svelte';
  import { releaseOnDestroy } from '../lib/video';

  let {
    items,
    index,
    total,
    onindex,
    onclose,
    onneedmore,
    onremoved,
    urls = libraryUrls,
    readOnly = false,
    closable = true,
  }: {
    items: MediaItem[];
    index: number;
    total: number;
    onindex: (index: number) => void;
    onclose: () => void;
    onneedmore: () => Promise<boolean>;
    onremoved: (id: number) => void;
    /** Where media comes from: the library, or a share link. */
    urls?: MediaUrls;
    /** Share links: no favorites, trash or sharing. */
    readOnly?: boolean;
    closable?: boolean;
  } = $props();

  const canEdit = $derived(
    !readOnly && session.current.status === 'signed-in' && session.current.user.role !== 'viewer',
  );
  let sharing = $state(false);
  let slideshow = $state(false);
  // The overflow menu on narrow screens.
  let menu = $state(false);
  // The slideshow pages through the same list, loading more as it goes.
  const slides = {
    get items() {
      return items;
    },
    get total() {
      return total;
    },
    more: () => onneedmore(),
    remove() {},
  };

  function endSlideshow(lastId: number | null) {
    slideshow = false;
    const i = items.findIndex((m) => m.id === lastId);
    if (i >= 0 && i !== index) onindex(i);
  }

  const MAX_SCALE = 8;
  const item = $derived(items[index]);

  let stage: HTMLDivElement | undefined = $state();
  let scale = $state(1);
  let tx = $state(0);
  let ty = $state(0);
  let showInfo = $state(false);
  let detail: MediaDetail | null = $state(null);
  let toast = $state('');
  let toastTimer: ReturnType<typeof setTimeout>;
  // Zoomed-in photos switch to the full-resolution original.
  let wantOriginal = $state(false);
  let gestureActive = $state(false);

  // Reset zoom when the item changes, and prefetch neighbours.
  $effect(() => {
    item.id;
    resetZoom();
    wantOriginal = false;
    for (const neighbour of [items[index + 1], items[index - 1]]) {
      if (neighbour?.kind === 'image') new Image().src = urls.large(neighbour);
    }
    if (index >= items.length - 3 && items.length < total) onneedmore();
  });

  // Details back the info panel and the favorite button.
  $effect(() => {
    const id = item.id;
    detail = null;
    get<MediaDetail>(urls.detail(id)).then(
      (d) => {
        if (d.id === item.id) detail = d;
      },
      () => {},
    );
  });

  async function toggleFavorite() {
    if (!detail) return;
    const d = detail;
    const favorite = !d.favorite;
    d.favorite = favorite;
    try {
      await (favorite ? put(`/media/${d.id}/favorite`) : del(`/media/${d.id}/favorite`));
      flash(favorite ? 'Added to favorites' : 'Removed from favorites');
    } catch (e) {
      d.favorite = !favorite;
      flash((e as Error).message);
    }
  }

  let trashing = $state(false);
  // Hiding: admins hide for everyone, others just for themselves.
  const isAdmin = $derived(session.current.status === 'signed-in' && session.current.user.role === 'admin');
  const showingHidden = $derived(session.current.status === 'signed-in' && session.current.user.show_hidden);
  let hiding = $state(false);
  async function toggleHidden() {
    if (!detail || hiding || detail.hidden === 'folder') return;
    const d = detail;
    hiding = true;
    try {
      if (d.hidden === 'photo') {
        await del(`/media/${d.id}/hidden`);
        d.hidden = undefined;
        flash('No longer hidden');
      } else {
        await put(`/media/${d.id}/hidden`);
        flash(isAdmin ? 'Hidden from everyone' : 'Hidden (only for you)');
        // With "show hidden" on it stays, marked; otherwise it leaves the list.
        if (showingHidden) d.hidden = 'photo';
        else onremoved(d.id);
      }
    } catch (e) {
      flash((e as Error).message);
    } finally {
      hiding = false;
    }
  }

  async function moveToTrash() {
    if (trashing || !confirm(`Move “${item.name}” to the trash? You can restore it for 30 days.`)) return;
    trashing = true;
    const id = item.id;
    try {
      await post(`/media/${id}/trash`);
      onremoved(id);
      flash('Moved to trash');
    } catch (e) {
      flash((e as Error).message);
    } finally {
      trashing = false;
    }
  }

  $effect(() => {
    const previous = document.body.style.overflow;
    document.body.style.overflow = 'hidden';
    return () => {
      document.body.style.overflow = previous;
    };
  });

  function resetZoom() {
    scale = 1;
    tx = 0;
    ty = 0;
  }

  function flash(message: string) {
    toast = message;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (toast = ''), 2500);
  }

  const hasPrev = $derived(index > 0);
  const hasNext = $derived(index < items.length - 1);
  const prev = () => hasPrev && onindex(index - 1);
  const next = () => hasNext && onindex(index + 1);

  /** Zooms to `target` keeping the stage point (px, py) (relative to centre) fixed. */
  function zoomTo(target: number, px = 0, py = 0) {
    const s = Math.min(MAX_SCALE, Math.max(1, target));
    if (s === 1) {
      resetZoom();
      return;
    }
    tx = px - (px - tx) * (s / scale);
    ty = py - (py - ty) * (s / scale);
    scale = s;
    if (s > 1.5) wantOriginal = true;
    clampPan();
  }

  function clampPan() {
    if (!stage) return;
    const maxX = ((scale - 1) * stage.clientWidth) / 2;
    const maxY = ((scale - 1) * stage.clientHeight) / 2;
    tx = Math.max(-maxX, Math.min(maxX, tx));
    ty = Math.max(-maxY, Math.min(maxY, ty));
  }

  function stagePoint(clientX: number, clientY: number): [number, number] {
    const rect = stage!.getBoundingClientRect();
    return [clientX - rect.left - rect.width / 2, clientY - rect.top - rect.height / 2];
  }

  function onwheel(event: WheelEvent) {
    if (item.kind !== 'image') return;
    event.preventDefault();
    const factor = Math.exp(-event.deltaY * (event.ctrlKey ? 0.01 : 0.0025));
    zoomTo(scale * factor, ...stagePoint(event.clientX, event.clientY));
  }

  function ondblclick(event: MouseEvent) {
    if (item.kind !== 'image') return;
    if (scale > 1) resetZoom();
    else zoomTo(2.5, ...stagePoint(event.clientX, event.clientY));
  }

  // Pointer gestures: drag to pan (zoomed) or swipe (not zoomed); two fingers pinch.
  const pointers = new Map<number, { x: number; y: number }>();
  let start = { x: 0, y: 0, tx: 0, ty: 0, scale: 1, distance: 0, mid: [0, 0] as [number, number] };

  function snapshot() {
    const pts = [...pointers.values()];
    const [a, b] = pts;
    start = {
      x: a.x,
      y: a.y,
      tx,
      ty,
      scale,
      distance: b ? Math.hypot(a.x - b.x, a.y - b.y) : 0,
      mid: b ? stagePoint((a.x + b.x) / 2, (a.y + b.y) / 2) : [0, 0],
    };
  }

  function onpointerdown(event: PointerEvent) {
    menu = false;
    // Buttons on the photo (previous/next) need their own clicks, so don't capture.
    if (item.kind !== 'image' || event.button !== 0 || (event.target as Element).closest('button')) return;
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    pointers.set(event.pointerId, { x: event.clientX, y: event.clientY });
    gestureActive = true;
    snapshot();
  }

  function onpointermove(event: PointerEvent) {
    if (!pointers.has(event.pointerId)) return;
    pointers.set(event.pointerId, { x: event.clientX, y: event.clientY });
    const pts = [...pointers.values()];
    if (pts.length >= 2) {
      const distance = Math.hypot(pts[0].x - pts[1].x, pts[0].y - pts[1].y);
      // Anchor the pinch at the starting midpoint.
      const s = Math.min(MAX_SCALE, Math.max(1, (start.scale * distance) / (start.distance || distance)));
      const [mx, my] = start.mid;
      tx = mx - (mx - start.tx) * (s / start.scale);
      ty = my - (my - start.ty) * (s / start.scale);
      scale = s;
      if (s > 1.5) wantOriginal = true;
      clampPan();
    } else if (scale > 1) {
      tx = start.tx + (event.clientX - start.x);
      ty = start.ty + (event.clientY - start.y);
      clampPan();
    } else {
      // Not zoomed: follow the finger horizontally as swipe feedback.
      tx = event.clientX - start.x;
    }
  }

  function onpointerup(event: PointerEvent) {
    if (!pointers.has(event.pointerId)) return;
    pointers.delete(event.pointerId);
    if (pointers.size > 0) {
      snapshot();
      return;
    }
    gestureActive = false;
    if (scale <= 1) {
      const dx = event.clientX - start.x;
      if (dx < -60 && hasNext) next();
      else if (dx > 60 && hasPrev) prev();
      else resetZoom();
    }
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.target instanceof HTMLInputElement || slideshow) return;
    switch (event.key) {
      case 'Escape':
        if (sharing) return;
        if (menu) menu = false;
        else if (scale > 1) resetZoom();
        else if (closable) onclose();
        break;
      case 'ArrowLeft':
        prev();
        break;
      case 'ArrowRight':
        next();
        break;
      case '+':
      case '=':
        zoomTo(scale * 1.5);
        break;
      case '-':
        zoomTo(scale / 1.5);
        break;
      case 'i':
        showInfo = !showInfo;
        break;
      case 'f':
        if (canEdit) toggleFavorite();
        break;
      case 'Delete':
        if (canEdit) moveToTrash();
        break;
      case 'h':
        if (canEdit) toggleHidden();
        break;
      default:
        return;
    }
    event.preventDefault();
  }

  async function copyImage() {
    if (!navigator.clipboard?.write || typeof ClipboardItem === 'undefined') {
      flash('Copying images needs a secure (HTTPS) connection.');
      return;
    }
    // Clipboards reliably accept only PNG, so convert the large preview.
    const png = (async () => {
      const blob = await (await fetch(urls.large(item))).blob();
      const bitmap = await createImageBitmap(blob);
      const canvas = document.createElement('canvas');
      canvas.width = bitmap.width;
      canvas.height = bitmap.height;
      canvas.getContext('2d')!.drawImage(bitmap, 0, 0);
      return await new Promise<Blob>((resolve, reject) =>
        canvas.toBlob((b) => (b ? resolve(b) : reject(new Error('encode failed'))), 'image/png'),
      );
    })();
    try {
      // Passing the promise keeps Safari happy (the write starts within the click).
      await navigator.clipboard.write([new ClipboardItem({ 'image/png': png })]);
      flash('Copied to clipboard');
    } catch {
      flash("Couldn't copy this image.");
    }
  }

  // Zooming in swaps to the original, when it may be fetched.
  const original = $derived(urls.file(item));
  const src = $derived(
    item.kind === 'image' ? (wantOriginal && original ? original : urls.large(item)) : (original ?? ''),
  );
  const download = $derived(urls.download(item));
</script>

<svelte:window {onkeydown} />

<div class="viewer" role="dialog" aria-modal="true" aria-label={item.name}>
  <div class="toolbar">
    {#if closable}
      <button class="icon" onclick={onclose} aria-label="Close" title="Close (Esc)"><Icon name="back" /></button>
    {/if}
    <div class="title">
      <div class="name">{item.name}</div>
      <div class="sub">{formatDate(item.taken_at, true)}{total > 1 ? ` · ${index + 1} of ${total.toLocaleString()}` : ''}{detail?.hidden ? ' · Hidden' : ''}</div>
    </div>
    {#if canEdit}
      <button class="icon" style="order: 4" class:on={detail?.favorite} onclick={toggleFavorite} disabled={!detail} aria-pressed={detail?.favorite ?? false} aria-label={detail?.favorite ? 'Remove from favorites' : 'Add to favorites'} title={detail?.favorite ? 'Remove from favorites' : 'Add to favorites'}>
        <Icon name={detail?.favorite ? 'heartFilled' : 'favorite'} />
      </button>
      <button class="icon" style="order: 5" onclick={() => (sharing = true)} aria-label="Share" title="Share"><Icon name="sharing" /></button>
    {/if}
    <button class="icon" style="order: 8" class:on={showInfo} onclick={() => (showInfo = !showInfo)} aria-label="Info" title="Info (i)"><Icon name="info" /></button>
    <!-- Inline on wide screens; a menu behind "More" on phones. -->
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
    <div class="more-menu" class:open={menu} onclick={() => (menu = false)}>
      {#if item.kind === 'image'}
        <button class="icon" style="order: 2" onclick={() => (scale > 1 ? resetZoom() : zoomTo(2.5))} aria-label={scale > 1 ? 'Zoom out' : 'Zoom in'} title={scale > 1 ? 'Zoom out' : 'Zoom in'}>
          <Icon name={scale > 1 ? 'zoomOut' : 'zoomIn'} /><span class="label">{scale > 1 ? 'Zoom out' : 'Zoom in'}</span>
        </button>
        <button class="icon" style="order: 3" onclick={copyImage} aria-label="Copy to clipboard" title="Copy to clipboard">
          <Icon name="copy" /><span class="label">Copy to clipboard</span>
        </button>
      {/if}
      {#if download}
        <a class="icon-link" style="order: 6" href={download} download={item.name} aria-label="Download" title="Download">
          <Icon name="download" /><span class="label">Download</span>
        </a>
      {/if}
      {#if total > 1}
        <button class="icon" style="order: 7" onclick={() => (slideshow = true)} aria-label="Slideshow" title="Slideshow">
          <Icon name="slideshow" /><span class="label">Slideshow</span>
        </button>
      {/if}
      {#if canEdit}
        <button
          class="icon"
          style="order: 9"
          onclick={toggleHidden}
          disabled={!detail || hiding || detail.hidden === 'folder'}
          aria-label={detail?.hidden ? 'Unhide' : 'Hide'}
          title={detail?.hidden === 'folder' ? 'In a hidden folder (unhide the folder)' : detail?.hidden ? 'Unhide (h)' : isAdmin ? 'Hide from everyone (h)' : 'Hide (h)'}>
          <Icon name={detail?.hidden ? 'eye' : 'eyeOff'} /><span class="label">{detail?.hidden ? 'Unhide' : 'Hide'}</span>
        </button>
        <button class="icon" style="order: 10" onclick={moveToTrash} disabled={trashing} aria-label="Move to trash" title="Move to trash (Delete)">
          <Icon name="trash" /><span class="label">Move to trash</span>
        </button>
      {/if}
    </div>
    <button class="icon more" class:on={menu} onclick={() => (menu = !menu)} aria-expanded={menu} aria-label="More actions" title="More"><Icon name="more" /></button>
  </div>

  <div class="body">
    <div
      class="stage"
      bind:this={stage}
      {onwheel}
      {ondblclick}
      {onpointerdown}
      {onpointermove}
      {onpointerup}
      onpointercancel={onpointerup}
      role="presentation">
      {#key item.id}
        {#if item.kind === 'video'}
          <!-- svelte-ignore a11y_media_has_caption -->
          <video src={src} poster={urls.large(item)} controls autoplay playsinline use:releaseOnDestroy></video>
        {:else}
          <img
            {src}
            alt={item.name}
            draggable="false"
            class:animate={!gestureActive}
            style="transform: translate({tx}px, {ty}px) scale({scale})" />
        {/if}
      {/key}

      {#if hasPrev}
        <button class="nav left" onclick={prev} aria-label="Previous"><Icon name="left" size={28} /></button>
      {/if}
      {#if hasNext}
        <button class="nav right" onclick={next} aria-label="Next"><Icon name="right" size={28} /></button>
      {/if}
    </div>

    {#if showInfo}
      <aside class="info">
        <h3>Info</h3>
        {#if detail}
          <dl>
            <dt>Taken</dt>
            <dd>{formatDate(detail.taken_at, true)}</dd>
            {#if detail.width && detail.height}
              <dt>Dimensions</dt>
              <dd>{detail.width} × {detail.height} ({((detail.width * detail.height) / 1e6).toFixed(1)} MP)</dd>
            {/if}
            {#if detail.duration_ms}
              <dt>Duration</dt>
              <dd>{formatDuration(detail.duration_ms)}</dd>
            {/if}
            <dt>Size</dt>
            <dd>{formatBytes(detail.size)}</dd>
            {#if detail.camera_make || detail.camera_model}
              <dt>Camera</dt>
              <dd>{[detail.camera_make, detail.camera_model].filter(Boolean).join(' ')}</dd>
            {/if}
            {#if detail.lens}
              <dt>Lens</dt>
              <dd>{detail.lens}</dd>
            {/if}
            {#if detail.latitude != null && detail.longitude != null}
              <dt>Location</dt>
              <dd>
                <a href="https://www.openstreetmap.org/?mlat={detail.latitude}&mlon={detail.longitude}#map=15/{detail.latitude}/{detail.longitude}" target="_blank" rel="noopener noreferrer">
                  {detail.latitude.toFixed(5)}, {detail.longitude.toFixed(5)}
                </a>
              </dd>
            {/if}
            {#if detail.description}
              <dt>Description</dt>
              <dd>{detail.description}</dd>
            {/if}
            {#if detail.path}
              <dt>File</dt>
              <dd class="path">{detail.path}</dd>
            {/if}
            <dt>Modified</dt>
            <dd>{new Date(detail.modified_at * 1000).toLocaleString()}</dd>
          </dl>
        {:else}
          <p class="muted small">Loading…</p>
        {/if}
      </aside>
    {/if}
  </div>

  {#if toast}<div class="toast" role="status">{toast}</div>{/if}
  {#if sharing}
    <ShareDialog target={{ media_id: item.id, name: item.name }} onclose={() => (sharing = false)} />
  {/if}
  {#if slideshow}
    <Slideshow source={slides} startId={item.id} {urls} onclose={endSlideshow} />
  {/if}
</div>

<style>
  .viewer {
    position: fixed;
    inset: 0;
    z-index: 100;
    display: flex;
    flex-direction: column;
    background: #000;
    color: #f1f3f4;
    /* The viewer is always dark, whatever the theme. */
    --border: #2c3137;
    --surface-2: #22262b;
  }

  .toolbar {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 6px 8px;
    background: linear-gradient(rgb(0 0 0 / 0.6), transparent);
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    z-index: 2;
  }

  .toolbar button.icon,
  .icon-link {
    color: #f1f3f4;
    width: 40px;
    height: 40px;
    display: grid;
    place-items: center;
    border-radius: 999px;
    border: 0;
    background: transparent;
  }

  .toolbar button.icon[aria-pressed='true'] {
    color: #ff6b81;
  }

  .toolbar button.icon:hover,
  .icon-link:hover,
  .toolbar button.on {
    background: rgb(255 255 255 / 0.14);
  }

  .title {
    order: 1;
    flex: 1;
    min-width: 0;
    padding: 0 6px;
  }

  .name {
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .sub {
    font-size: 0.8rem;
    color: #b0b6bc;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .body {
    flex: 1;
    display: flex;
    min-height: 0;
  }

  .stage {
    position: relative;
    flex: 1;
    overflow: hidden;
    touch-action: none;
    user-select: none;
  }

  /* Fill the stage and letterbox, so any aspect ratio fits. */
  .stage img,
  .stage video {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: contain;
  }

  .stage img {
    will-change: transform;
    cursor: zoom-in;
  }

  .stage img.animate {
    transition: transform 0.15s ease;
  }

  .nav {
    position: absolute;
    top: 50%;
    transform: translateY(-50%);
    width: 48px;
    height: 48px;
    display: grid;
    place-items: center;
    padding: 0;
    border: 0;
    border-radius: 999px;
    color: #fff;
    background: rgb(0 0 0 / 0.4);
    opacity: 0;
    transition: opacity 0.15s;
  }

  .stage:hover .nav,
  .nav:focus-visible {
    opacity: 1;
  }

  .nav.left {
    left: 12px;
  }

  .nav.right {
    right: 12px;
  }

  .info {
    width: 320px;
    flex: none;
    overflow-y: auto;
    padding: 64px 20px 20px;
    background: #121417;
    border-left: 1px solid #2c3137;
  }

  .info h3 {
    margin: 0 0 12px;
  }

  dl {
    margin: 0;
    display: grid;
    gap: 2px;
  }

  dt {
    margin-top: 10px;
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: #9aa0a6;
  }

  dd {
    margin: 0;
    overflow-wrap: anywhere;
  }

  .info a {
    color: #8ee4b4;
  }

  .path {
    font-family: ui-monospace, monospace;
    font-size: 0.8rem;
  }

  .toast {
    position: absolute;
    left: 50%;
    bottom: 32px;
    transform: translateX(-50%);
    padding: 10px 18px;
    border-radius: 999px;
    background: rgb(40 44 48 / 0.95);
    color: #fff;
    font-size: 0.9rem;
  }

  .more-menu {
    display: contents;
  }

  .more-menu .label,
  .toolbar button.more {
    display: none;
  }

  .toolbar button.more {
    order: 11;
  }

  @media (max-width: 700px) {
    .toolbar button.more {
      display: grid;
    }

    .more-menu {
      display: none;
      position: absolute;
      top: 52px;
      right: 8px;
      min-width: 210px;
      padding: 6px;
      flex-direction: column;
      border-radius: 12px;
      background: rgb(30 33 37 / 0.97);
      border: 1px solid #2c3137;
      box-shadow: 0 8px 24px rgb(0 0 0 / 0.5);
    }

    .more-menu.open {
      display: flex;
    }

    .more-menu .label {
      display: inline;
    }

    .toolbar .more-menu button.icon,
    .more-menu .icon-link {
      width: auto;
      height: 44px;
      display: flex;
      align-items: center;
      gap: 14px;
      padding: 0 12px;
      border-radius: 8px;
      text-decoration: none;
      font-size: 0.95rem;
    }

    .info {
      position: absolute;
      left: 0;
      right: 0;
      bottom: 0;
      width: auto;
      max-height: 55%;
      padding-top: 20px;
      border-left: 0;
      border-top: 1px solid #2c3137;
      border-radius: 16px 16px 0 0;
    }

    .nav {
      display: none;
    }
  }

  @media (hover: none) {
    .nav {
      display: none;
    }
  }
</style>
