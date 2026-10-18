<script lang="ts">
  import { get } from '../lib/api';
  import { formatBytes } from '../lib/media.svelte';

  type Volume = { paths: string[]; total: number; free: number; used: number };
  let volumes: Volume[] = $state([]);

  $effect(() => {
    const load = () => get<Volume[]>('/storage').then((v) => (volumes = v), () => {});
    load();
    const timer = setInterval(load, 5 * 60 * 1000);
    return () => clearInterval(timer);
  });
</script>

{#each volumes as volume (volume.paths.join())}
  {@const fraction = volume.total ? volume.used / volume.total : 0}
  <div class="storage" title={volume.paths.join(', ')}>
    <div class="label small">
      <!-- With several volumes, name each by its folder. -->
      <span class="name">{volumes.length > 1 ? volume.paths[0].split('/').filter(Boolean).at(-1) : 'Storage'}</span>
      <span class="muted">{Math.round(fraction * 100)}%</span>
    </div>
    <div class="bar" role="meter" aria-label="Storage used" aria-valuemin="0" aria-valuemax="100" aria-valuenow={Math.round(fraction * 100)}>
      <div class="fill" class:high={fraction > 0.9} style="width: {fraction * 100}%"></div>
    </div>
    <div class="muted small">{formatBytes(volume.used)} of {formatBytes(volume.total)} used</div>
  </div>
{/each}

<style>
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .storage {
    padding: 10px 14px 4px;
  }

  .label {
    display: flex;
    justify-content: space-between;
    gap: 8px;
    margin-bottom: 6px;
  }

  .bar {
    height: 6px;
    border-radius: 999px;
    background: var(--surface-2);
    overflow: hidden;
    margin-bottom: 6px;
  }

  .fill {
    height: 100%;
    border-radius: inherit;
    background: var(--accent);
  }

  .fill.high {
    background: var(--danger);
  }
</style>
