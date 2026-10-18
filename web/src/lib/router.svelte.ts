/** A minimal history-API router: the current path and query, and navigation. */
export const route = $state({ path: location.pathname, query: new URLSearchParams(location.search) });

type EntryState = Record<string, unknown> | null;

// Scroll positions. Pages load their content after they appear, so the browser's own
// restoring would find a short page and land at the top. Instead each history entry
// gets a key, its position is saved when leaving it, and Back/Forward (or a reload)
// scrolls back to it as the content comes in.
const POSITIONS_KEY = 'pixelgrove-scroll';
const positions = new Map<string, number>(readPositions());
history.scrollRestoration = 'manual';
let currentKey = entryKey();

function readPositions(): [string, number][] {
  try {
    return JSON.parse(sessionStorage.getItem(POSITIONS_KEY) ?? '[]');
  } catch {
    return [];
  }
}

function newKey(): string {
  return Math.random().toString(36).slice(2, 10);
}

/** The current history entry's key, giving it one if it has none yet. */
function entryKey(): string {
  const state = history.state as EntryState;
  if (typeof state?.key === 'string') return state.key;
  const key = newKey();
  history.replaceState({ ...state, key }, '');
  return key;
}

function savePosition(): void {
  positions.delete(currentKey);
  positions.set(currentKey, Math.round(scrollY));
  // Keep the most recent entries only.
  while (positions.size > 100) positions.delete(positions.keys().next().value!);
  try {
    sessionStorage.setItem(POSITIONS_KEY, JSON.stringify([...positions]));
  } catch {
    // Not kept across reloads; Back still works within this page load.
  }
}

let restoring = 0;

/**
 * Scrolls to `y`, following the page as it grows (lists load more as they come into
 * view) until it's there and the page has settled. Stops early if the user scrolls.
 */
function restorePosition(y: number): void {
  const run = ++restoring;
  const deadline = performance.now() + 5000;
  let lastHeight = -1;
  let steady = 0;
  const cancel = () => {
    if (run === restoring) restoring++;
  };
  const inputs = ['wheel', 'touchstart', 'keydown', 'pointerdown'] as const;
  const done = () => inputs.forEach((type) => removeEventListener(type, cancel));
  inputs.forEach((type) => addEventListener(type, cancel, { passive: true }));
  const step = () => {
    if (run !== restoring) return done();
    const height = document.documentElement.scrollHeight;
    scrollTo(0, Math.min(y, Math.max(0, height - innerHeight)));
    // Once there, wait a few frames for anything still loading above to settle.
    steady = height === lastHeight && scrollY >= y - 1 ? steady + 1 : 0;
    lastHeight = height;
    if (steady >= 10 || performance.now() > deadline) return done();
    requestAnimationFrame(step);
  };
  // Start once the page for the new URL has rendered.
  requestAnimationFrame(step);
}

window.addEventListener('popstate', () => {
  savePosition();
  currentKey = entryKey();
  route.path = location.pathname;
  route.query = new URLSearchParams(location.search);
  restorePosition(positions.get(currentKey) ?? 0);
});

// Kept for a reload, which restores it too.
window.addEventListener('pagehide', savePosition);
if (positions.has(currentKey)) restorePosition(positions.get(currentKey)!);

/**
 * Navigates to `url` (path plus optional `?query`). `replace` avoids a history entry.
 * A new page starts at the top, except for the viewer (`state.viewer`), which opens
 * over the page.
 */
export function navigate(url: string, replace = false, state: EntryState = null): void {
  const target = new URL(url, location.origin);
  if (target.pathname + target.search === location.pathname + location.search) return;
  savePosition();
  if (!replace) currentKey = newKey();
  history[replace ? 'replaceState' : 'pushState']({ ...state, key: currentKey }, '', target.pathname + target.search);
  route.path = target.pathname;
  route.query = new URLSearchParams(target.search);
  if (!replace && !state?.viewer) {
    restoring++;
    scrollTo(0, 0);
  }
}

/** Click handler for internal links, so they navigate without a page load. */
export function link(event: MouseEvent): void {
  if (event.defaultPrevented || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey) return;
  const anchor = event.currentTarget as HTMLAnchorElement;
  event.preventDefault();
  navigate(anchor.getAttribute('href') ?? '/');
}

/**
 * Svelte action for infinite scroll: while the element is in (or near) view, calls
 * `load` repeatedly until it returns false (nothing more to load).
 */
export function onVisible(node: HTMLElement, load: () => Promise<boolean>) {
  let visible = false;
  let running = false;
  const run = async () => {
    if (running) return;
    running = true;
    try {
      while (visible && (await load())) {
        // Let layout and the observer catch up before checking again.
        await new Promise(requestAnimationFrame);
      }
    } finally {
      running = false;
    }
  };
  const observer = new IntersectionObserver(
    (entries) => {
      visible = entries.some((e) => e.isIntersecting);
      if (visible) run();
    },
    { rootMargin: '1200px' },
  );
  observer.observe(node);
  return {
    update(next: () => Promise<boolean>) {
      load = next;
    },
    destroy() {
      observer.disconnect();
    },
  };
}
