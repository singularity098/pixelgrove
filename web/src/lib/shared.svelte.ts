import { get } from './api';
import type { MediaItem, MediaSource, Subfolder } from './media.svelte';

export interface ShareInfo {
  kind: 'media' | 'folder';
  locked: boolean;
  name: string | null;
  allow_download: boolean;
  expires_at: number | null;
  media: MediaItem | null;
}

/** One folder inside a shared folder link, loaded page by page. */
export class SharedFolder implements MediaSource {
  name = $state('');
  subfolders: Subfolder[] = $state([]);
  items: MediaItem[] = $state([]);
  total: number | null = $state(null);
  loading = $state(false);
  error = $state('');

  constructor(
    private readonly token: string,
    readonly path: string,
  ) {}

  async more(): Promise<boolean> {
    if (this.loading || this.error || (this.total !== null && this.items.length >= this.total)) return false;
    this.loading = true;
    try {
      const page = await get<{ name: string; subfolders: Subfolder[]; media: { items: MediaItem[]; total: number } }>(
        `/s/${this.token}/folder?path=${encodeURIComponent(this.path)}&offset=${this.items.length}&limit=200`,
      );
      if (this.items.length === 0) {
        this.name = page.name;
        this.subfolders = page.subfolders;
      }
      this.items.push(...page.media.items);
      this.total = page.media.total;
      return page.media.items.length > 0;
    } catch (e) {
      this.error = (e as Error).message;
      return false;
    } finally {
      this.loading = false;
    }
  }

  remove(): void {}
}
