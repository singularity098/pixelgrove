import { get } from './api';
import type { FolderHit, MediaItem, MediaSource } from './media.svelte';

/** Search results for one query: matching folders plus a growing list of media. */
export class SearchResults implements MediaSource {
  folders: FolderHit[] = $state([]);
  items: MediaItem[] = $state([]);
  total: number | null = $state(null);
  loading = $state(false);
  error = $state('');

  constructor(private readonly query: string) {}

  remove(id: number): void {
    const index = this.items.findIndex((item) => item.id === id);
    if (index < 0) return;
    this.items.splice(index, 1);
    if (this.total !== null) this.total--;
  }

  async more(): Promise<boolean> {
    if (this.loading || this.error || (this.total !== null && this.items.length >= this.total)) return false;
    this.loading = true;
    try {
      const page = await get<{ folders: FolderHit[]; media: { items: MediaItem[]; total: number } }>(
        `/search?q=${encodeURIComponent(this.query)}&offset=${this.items.length}&limit=120`,
      );
      if (this.items.length === 0) this.folders = page.folders;
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
}
