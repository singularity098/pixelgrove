/** A minimal history-API router: the current path and query, and navigation. */
export const route = $state({ path: location.pathname, query: new URLSearchParams(location.search) });

window.addEventListener('popstate', () => {
  route.path = location.pathname;
  route.query = new URLSearchParams(location.search);
});

/** Navigates to `url` (path plus optional `?query`). `replace` avoids a history entry. */
export function navigate(url: string, replace = false, state: unknown = null): void {
  const target = new URL(url, location.origin);
  if (target.pathname + target.search === location.pathname + location.search) return;
  history[replace ? 'replaceState' : 'pushState'](state, '', target.pathname + target.search);
  route.path = target.pathname;
  route.query = new URLSearchParams(target.search);
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
