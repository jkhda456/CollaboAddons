// The service worker of tests/site/sw/page.html: /sw/cached from its CacheStorage, /sw/made a
// response it makes, /sw/net its own fetch of /api/any?sw=1 (the network process holds that one)
self.addEventListener('install', event => {
    event.waitUntil(caches.open('test').then(cache => cache.put('/sw/cached',
        new Response('from the cache storage', { headers: { 'Content-Type': 'text/plain' } }))).then(() => self.skipWaiting()));
});
self.addEventListener('activate', event => event.waitUntil(self.clients.claim()));
self.addEventListener('fetch', event => {
    const path = new URL(event.request.url).pathname;
    if (path === '/sw/cached')
        event.respondWith(caches.match('/sw/cached'));
    else if (path === '/sw/made')
        event.respondWith(new Response('made by the worker', { headers: { 'Content-Type': 'text/plain' } }));
    else if (path === '/sw/net')
        event.respondWith(fetch('/api/any?sw=1'));
});
