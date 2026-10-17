// PixelGrove service worker: makes the app installable and quick to open by caching the
// app shell. Photos and API responses are never cached here (the browser's HTTP cache
// already handles previews), so nothing private lingers in the service worker cache.
const CACHE = 'pixelgrove-shell-v1';

self.addEventListener('install', () => self.skipWaiting());

self.addEventListener('activate', (event) => {
  event.waitUntil(
    (async () => {
      for (const key of await caches.keys()) {
        if (key !== CACHE) await caches.delete(key);
      }
      await self.clients.claim();
    })(),
  );
});

self.addEventListener('fetch', (event) => {
  const request = event.request;
  const url = new URL(request.url);
  if (request.method !== 'GET' || url.origin !== location.origin || url.pathname.startsWith('/api/')) return;

  if (request.mode === 'navigate') {
    // Network first so the app is always current; fall back to the cached shell offline.
    event.respondWith(
      (async () => {
        try {
          const response = await fetch(request);
          if (response.ok) (await caches.open(CACHE)).put('/', response.clone());
          return response;
        } catch {
          return (await caches.match('/')) ?? Response.error();
        }
      })(),
    );
    return;
  }

  if (url.pathname.startsWith('/assets/')) {
    // Fingerprinted build output never changes: cache first.
    event.respondWith(
      (async () => {
        const cached = await caches.match(request);
        if (cached) return cached;
        const response = await fetch(request);
        if (response.ok) (await caches.open(CACHE)).put(request, response.clone());
        return response;
      })(),
    );
  }
});
