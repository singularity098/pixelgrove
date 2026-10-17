/** A minimal history-API router: the current path, and navigation that updates it. */
export const route = $state({ path: location.pathname });

window.addEventListener('popstate', () => {
  route.path = location.pathname;
});

export function navigate(path: string): void {
  if (path === route.path) return;
  history.pushState(null, '', path);
  route.path = path;
}

/** Click handler for internal links, so they navigate without a page load. */
export function link(event: MouseEvent): void {
  if (event.defaultPrevented || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey) return;
  const anchor = event.currentTarget as HTMLAnchorElement;
  event.preventDefault();
  navigate(anchor.getAttribute('href') ?? '/');
}
