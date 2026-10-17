<script lang="ts">
  import { get } from '../lib/api';

  type Status = {
    folders: number;
    images: number;
    videos: number;
    scanning: boolean;
    watching: boolean;
    last_full_scan_at: number | null;
  };

  let status: Status | null = $state(null);
  let error = $state('');

  $effect(() => {
    let timer: ReturnType<typeof setTimeout>;
    const load = async () => {
      try {
        status = await get<Status>('/library/status');
        error = '';
      } catch (e) {
        error = (e as Error).message;
      }
      // Poll quickly while a scan runs so the counts visibly fill in.
      timer = setTimeout(load, status?.scanning ? 2000 : 15000);
    };
    load();
    return () => clearTimeout(timer);
  });

  const fmt = (n: number) => n.toLocaleString();
</script>

<section class="page">
  <h2>Library</h2>
  <p class="muted">Browsing arrives in the next milestone. For now, here's what PixelGrove has indexed.</p>
  {#if error}
    <p class="error">{error}</p>
  {:else if status}
    <div class="stats">
      <div class="stat"><span>{fmt(status.images)}</span>photos</div>
      <div class="stat"><span>{fmt(status.videos)}</span>videos</div>
      <div class="stat"><span>{fmt(status.folders)}</span>folders</div>
    </div>
    <p class="muted small">
      {#if status.scanning}
        Scanning…
      {:else if status.last_full_scan_at}
        Last full scan {new Date(status.last_full_scan_at * 1000).toLocaleString()}.
      {/if}
      {status.watching ? 'Watching for changes.' : 'Live watching unavailable; changes are picked up by periodic rescans.'}
    </p>
  {/if}
</section>

<style>
  .stats {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(140px, 1fr));
    gap: 12px;
    margin: 20px 0;
  }

  .stat {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 16px;
    color: var(--text-muted);
  }

  .stat span {
    display: block;
    font-size: 1.8rem;
    font-weight: 600;
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }
</style>
