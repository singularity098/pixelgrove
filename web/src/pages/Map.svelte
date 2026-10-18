<script lang="ts">
  import 'leaflet/dist/leaflet.css';
  import L from 'leaflet';
  import Supercluster from 'supercluster';
  import MediaGrid from '../components/MediaGrid.svelte';
  import MediaViewerHost from '../components/MediaViewerHost.svelte';
  import { get } from '../lib/api';
  import { LocalList } from '../lib/local.svelte';
  import { thumbUrl, type MediaItem } from '../lib/media.svelte';
  import { openViewer } from '../lib/query';

  /** `[id, lat, lon, taken_at, v, is_video]` from /api/map/points. */
  type Point = [number, number, number, number, number, 0 | 1];
  type Props = { id: number; t: number; v: number; video: boolean };
  // Each cluster tracks its newest photo (preferring photos over videos) as its cover.
  type ClusterProps = { cover: number; coverT: number; coverV: number; coverVideo: boolean };

  let container: HTMLDivElement | undefined = $state();
  let total = $state<number | null>(null);
  let error = $state('');
  let selection = $state<LocalList | null>(null);
  let selectionLoading = $state(false);

  const MAX_SELECTION = 500;

  $effect(() => {
    if (!container) return;
    const map = L.map(container, { zoomControl: true, worldCopyJump: true }).setView([20, 0], 2);
    L.tileLayer('https://tile.openstreetmap.org/{z}/{x}/{y}.png', {
      maxZoom: 19,
      // OpenStreetMap blocks tile requests without a Referer, and the app otherwise
      // sends none. Send just this server's address (never the page path).
      referrerPolicy: 'strict-origin',
      attribution: '&copy; <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a> contributors',
    }).addTo(map);
    const layer = L.layerGroup().addTo(map);
    const index = new Supercluster<Props, ClusterProps>({
      radius: 70,
      maxZoom: 18,
      map: (p) => ({ cover: p.id, coverT: p.t, coverV: p.v, coverVideo: p.video }),
      reduce: (acc, p) => {
        const better = acc.coverVideo !== p.coverVideo ? acc.coverVideo : p.coverT > acc.coverT;
        if (better) Object.assign(acc, { cover: p.cover, coverT: p.coverT, coverV: p.coverV, coverVideo: p.coverVideo });
      },
    });
    let loaded = false;

    function render() {
      if (!loaded) return;
      layer.clearLayers();
      const b = map.getBounds();
      const clusters = index.getClusters([b.getWest(), b.getSouth(), b.getEast(), b.getNorth()], Math.round(map.getZoom()));
      for (const feature of clusters) {
        const [lon, lat] = feature.geometry.coordinates;
        const p = feature.properties as Partial<ClusterProps & Props & { cluster: boolean; point_count: number; cluster_id: number }>;
        const cover = p.cluster ? { id: p.cover!, v: p.coverV!, video: p.coverVideo! } : { id: p.id!, v: p.v!, video: p.video! };
        const count = p.cluster ? p.point_count! : 1;
        const image = cover.video ? '<span class="pin-video">▶</span>' : `<img src="${thumbUrl(cover)}" alt="" loading="lazy">`;
        const icon = L.divIcon({
          className: 'pin',
          html: `<div class="pin-inner">${image}${count > 1 ? `<span class="pin-count">${count > 999 ? `${Math.round(count / 1000)}k` : count}</span>` : ''}</div>`,
          iconSize: [56, 56],
          iconAnchor: [28, 28],
        });
        L.marker([lat, lon], { icon, keyboard: true, title: `${count} item${count === 1 ? '' : 's'}` })
          .on('click', () => {
            const ids = p.cluster
              ? index.getLeaves(p.cluster_id!, Infinity).map((leaf) => leaf.properties)
              : [p as Props];
            select(ids);
          })
          .addTo(layer);
      }
    }

    map.on('moveend', render);
    // The selection panel and window resizes change the map's size; Leaflet must be told.
    const resize = new ResizeObserver(() => map.invalidateSize());
    resize.observe(container);
    // The page may be gone by the time the points arrive.
    let alive = true;
    get<Point[]>('/map/points').then(
      (points) => {
        if (!alive) return;
        total = points.length;
        index.load(
          points.map(([id, lat, lon, t, v, video]) => ({
            type: 'Feature' as const,
            geometry: { type: 'Point' as const, coordinates: [lon, lat] },
            properties: { id, t, v, video: video === 1 },
          })),
        );
        loaded = true;
        if (points.length) {
          map.fitBounds(L.latLngBounds(points.map(([, lat, lon]) => [lat, lon] as [number, number])), {
            padding: [40, 40],
            maxZoom: 12,
          });
        }
        render();
      },
      (e) => {
        if (alive) error = (e as Error).message;
      },
    );

    return () => {
      alive = false;
      resize.disconnect();
      map.stop();
      map.remove();
    };
  });

  async function select(props: Props[]) {
    const newest = [...props].sort((a, b) => b.t - a.t).slice(0, MAX_SELECTION);
    selectionLoading = true;
    try {
      const items = await get<MediaItem[]>(`/media/batch?ids=${newest.map((p) => p.id).join(',')}`);
      selection = new LocalList(items);
    } catch (e) {
      error = (e as Error).message;
    } finally {
      selectionLoading = false;
    }
  }
</script>

<div class="layout" class:has-selection={selection !== null}>
  <div class="map" bind:this={container}></div>
  {#if total === 0}
    <div class="notice">No photos with a location yet. Photos from phones usually have one.</div>
  {/if}
  {#if error}<div class="notice error">{error}</div>{/if}
  {#if selection}
    <section class="panel">
      <div class="panel-head">
        <strong>{selection.items.length.toLocaleString()} item{selection.items.length === 1 ? '' : 's'}{selection.items.length === MAX_SELECTION ? ' (newest)' : ''}</strong>
        <button class="ghost" onclick={() => (selection = null)}>Close</button>
      </div>
      <div class="panel-body">
        <MediaGrid items={selection.items} onopen={(i) => openViewer(selection!.items[i].id)} />
      </div>
    </section>
    <MediaViewerHost source={selection} />
  {:else if selectionLoading}
    <div class="notice">Loading…</div>
  {/if}
</div>

<style>
  .layout {
    position: relative;
    height: calc(100dvh - 56px);
    display: flex;
    flex-direction: column;
  }

  .map {
    flex: 1;
    min-height: 0;
    z-index: 0;
  }

  .panel {
    height: 42%;
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--border);
    background: var(--bg);
  }

  .panel-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px 12px;
  }

  .panel-body {
    flex: 1;
    overflow-y: auto;
    padding: 0 12px 12px;
  }

  .notice {
    position: absolute;
    top: 12px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 500;
    padding: 8px 16px;
    border-radius: 999px;
    background: var(--surface);
    border: 1px solid var(--border);
    font-size: 0.9rem;
  }

  /* Markers: a rounded thumbnail with a count badge. */
  :global(.pin) {
    background: none;
    border: 0;
  }

  :global(.pin-inner) {
    position: relative;
    width: 56px;
    height: 56px;
    border-radius: 12px;
    border: 3px solid #fff;
    box-shadow: 0 2px 8px rgb(0 0 0 / 0.45);
    overflow: hidden;
    background: #333;
    display: grid;
    place-items: center;
  }

  :global(.pin-inner img) {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  :global(.pin-video) {
    color: #fff;
    font-size: 20px;
  }

  :global(.pin-count) {
    position: absolute;
    right: 2px;
    bottom: 2px;
    padding: 0 6px;
    border-radius: 999px;
    background: var(--accent);
    color: var(--accent-contrast);
    font: 600 12px/18px system-ui, sans-serif;
  }

  /* Dim the light OpenStreetMap tiles in dark mode. */
  :global([data-theme='dark'] .leaflet-tile) {
    filter: brightness(0.65) invert(1) contrast(2.6) hue-rotate(200deg) saturate(0.35) brightness(0.75);
  }

  :global([data-theme='dark'] .leaflet-container) {
    background: #1b1f23;
  }
</style>
