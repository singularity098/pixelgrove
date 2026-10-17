import type { MediaItem, MediaSource } from './media.svelte';

/** A fixed, already-loaded list (e.g. the photos in a map cluster). */
export class LocalList implements MediaSource {
  items: MediaItem[] = $state([]);

  constructor(items: MediaItem[]) {
    this.items = items;
  }

  get total(): number {
    return this.items.length;
  }

  async more(): Promise<boolean> {
    return false;
  }

  remove(id: number): void {
    this.items = this.items.filter((item) => item.id !== id);
  }
}
