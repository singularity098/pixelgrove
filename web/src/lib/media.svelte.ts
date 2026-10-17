import { get } from './api';

export interface Cover {
  id: number;
  kind: 'image' | 'video';
  v: number;
}

export interface Album {
  id: number;
  name: string;
  path: string;
  count: number;
  latest_taken_at: number | null;
  cover: Cover | null;
}

export interface Subfolder {
  id: number;
  name: string;
  count: number;
  latest_taken_at: number | null;
  cover: Cover | null;
}

export interface FolderDetail {
  id: number;
  name: string;
  path: string;
  media_count: number;
  breadcrumbs: { id: number; name: string }[];
  subfolders: Subfolder[];
}

export interface MediaItem {
  id: number;
  name: string;
  kind: 'image' | 'video';
  width: number | null;
  height: number | null;
  taken_at: number;
  duration_ms: number | null;
  size: number;
  v: number;
}

/** Anything the viewer can page through. */
export interface MediaSource {
  readonly items: MediaItem[];
  readonly total: number | null;
  more(): Promise<boolean>;
  /** Drops an item that no longer belongs (e.g. moved to the trash). */
  remove(id: number): void;
}

export interface FolderHit {
  id: number;
  name: string;
  path: string;
  count: number;
  cover: Cover | null;
}

export interface TreeFolder {
  id: number;
  name: string;
  path: string;
  media_count: number;
  total_count: number;
  subfolder_count: number;
  latest_taken_at: number | null;
}

export interface MediaDetail extends MediaItem {
  folder_id: number;
  path: string;
  modified_at: number;
  tz_offset: number | null;
  camera_make: string | null;
  camera_model: string | null;
  lens: string | null;
  latitude: number | null;
  longitude: number | null;
  description: string | null;
  favorite: boolean;
}

export interface Paged<T> {
  items: T[];
  total: number;
}

export const thumbUrl = (m: { id: number; v: number }) => `/api/media/${m.id}/preview/thumb?v=${m.v}`;
export const largeUrl = (m: { id: number; v: number }) => `/api/media/${m.id}/preview/large?v=${m.v}`;
export const fileUrl = (m: { id: number; v: number }) => `/api/media/${m.id}/file?v=${m.v}`;
export const downloadUrl = (m: { id: number; v: number }) => `/api/media/${m.id}/file?v=${m.v}&download=true`;

/** A growing list loaded page by page. */
export class PagedList<T> {
  items: T[] = $state([]);
  total: number | null = $state(null);
  loading = $state(false);
  error = $state('');

  constructor(
    private readonly path: string,
    private readonly pageSize = 120,
  ) {}

  remove(id: number): void {
    const index = this.items.findIndex((item) => (item as { id: number }).id === id);
    if (index < 0) return;
    this.items.splice(index, 1);
    if (this.total !== null) this.total--;
  }

  get done(): boolean {
    return this.total !== null && this.items.length >= this.total;
  }

  /** Loads the next page; resolves to whether anything was added. */
  async more(): Promise<boolean> {
    if (this.loading || this.done || this.error) return false;
    this.loading = true;
    try {
      const sep = this.path.includes('?') ? '&' : '?';
      const page = await get<Paged<T>>(`${this.path}${sep}offset=${this.items.length}&limit=${this.pageSize}`);
      this.items.push(...page.items);
      this.total = page.total;
      return page.items.length > 0;
    } catch (e) {
      this.error = (e as Error).message;
      return false;
    } finally {
      this.loading = false;
    }
  }
}

export function formatDate(seconds: number | null, withTime = false): string {
  if (seconds === null) return '';
  // Capture times are wall-clock values stored as if UTC.
  const date = new Date(seconds * 1000);
  return date.toLocaleDateString(undefined, {
    timeZone: 'UTC',
    year: 'numeric',
    month: 'short',
    day: 'numeric',
    ...(withTime ? { hour: 'numeric', minute: '2-digit' } : {}),
  });
}

export function formatDuration(ms: number | null): string {
  if (!ms) return '';
  const total = Math.round(ms / 1000);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = String(total % 60).padStart(2, '0');
  return h ? `${h}:${String(m).padStart(2, '0')}:${s}` : `${m}:${s}`;
}

export function formatBytes(bytes: number): string {
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  return `${value.toFixed(unit === 0 || value >= 10 ? 0 : 1)} ${units[unit]}`;
}
