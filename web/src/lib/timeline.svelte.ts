import { get } from './api';
import type { MediaItem, MediaSource } from './media.svelte';

export interface Month {
  month: string;
  count: number;
}

/** The timeline stream, paged with a (taken_at, id) cursor; can restart at a month. */
export class Timeline implements MediaSource {
  items: MediaItem[] = $state([]);
  hasMore = $state(true);
  loading = $state(false);
  error = $state('');
  /** Set when the list starts part-way through (after jumping to a month). */
  startedBefore: number | null = $state(null);
  private generation = 0;

  get total(): number | null {
    return this.hasMore ? null : this.items.length;
  }

  /** Restarts the list with items taken before `before` (null = newest). */
  reset(before: number | null): void {
    this.generation++;
    this.items = [];
    this.hasMore = true;
    this.error = '';
    this.loading = false;
    this.startedBefore = before;
  }

  async more(): Promise<boolean> {
    if (this.loading || !this.hasMore || this.error) return false;
    this.loading = true;
    const generation = this.generation;
    const last = this.items.at(-1);
    const cursor = last
      ? `&before=${last.taken_at}&before_id=${last.id}`
      : this.startedBefore !== null
        ? `&before=${this.startedBefore}`
        : '';
    try {
      const page = await get<{ items: MediaItem[]; more: boolean }>(`/timeline?limit=200${cursor}`);
      if (generation !== this.generation) return false;
      this.items.push(...page.items);
      this.hasMore = page.more;
      return page.items.length > 0;
    } catch (e) {
      if (generation === this.generation) this.error = (e as Error).message;
      return false;
    } finally {
      if (generation === this.generation) this.loading = false;
    }
  }
}

/** Start of the month after `YYYY-MM`, as wall-clock seconds (matching taken_at). */
export function endOfMonth(month: string): number {
  const [y, m] = month.split('-').map(Number);
  return Date.UTC(y, m, 1) / 1000;
}

export function monthLabel(month: string, short = false): string {
  const [y, m] = month.split('-').map(Number);
  return new Date(Date.UTC(y, m - 1, 1)).toLocaleDateString(undefined, {
    timeZone: 'UTC',
    month: short ? 'short' : 'long',
    year: 'numeric',
  });
}
