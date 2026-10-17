import { navigate, route } from './router.svelte';

/** The current URL with one query parameter set (or removed when `value` is null). */
export function withParam(key: string, value: string | null): string {
  const params = new URLSearchParams(route.query);
  if (value === null) params.delete(key);
  else params.set(key, value);
  const query = params.toString();
  return query ? `${route.path}?${query}` : route.path;
}

export function setParam(key: string, value: string | null, replace = false): void {
  navigate(withParam(key, value), replace, replace ? history.state : null);
}

/** Opens the viewer on `id`, as a new history entry that closing it will pop. */
export function openViewer(id: number): void {
  navigate(withParam('m', String(id)), false, { viewer: true });
}
