// Service Worker Logic

const CACHE_NAME = 'peer-proxy-shell-v1';

self.addEventListener('install', (event) => {
    console.debug('[SW] Installing');
    self.skipWaiting();
});

self.addEventListener('activate', (event) => {
    console.debug('[SW] Activating');
    event.waitUntil(
        caches.keys().then((cacheNames) => {
            return Promise.all(
                cacheNames.map((cacheName) => {
                    if (cacheName !== CACHE_NAME) {
                        return caches.delete(cacheName);
                    }
                })
            );
        }).then(() => self.clients.claim())
    );
});

self.addEventListener('fetch', (event) => {
    const url = new URL(event.request.url);
    if (url.origin !== self.location.origin) return;
    if (url.searchParams.has('sw')) return;
    if (url.pathname.startsWith('/__syneroym/')) return;
    console.debug("[SW] ----- Starting overridden Fetch for", event)

    event.respondWith(
        (async () => {
            // Always serve App Shell for navigation to keep the proxy logic alive
            if (event.request.mode === 'navigate') {
                console.debug("[SW] Navigation request detected. Serving App Shell from cache if available.");
                const cache = await caches.open(CACHE_NAME);
                const cachedResponse = await cache.match(event.request);
                
                const networkFetch = fetch(event.request).then((networkResponse) => {
                    cache.put(event.request, networkResponse.clone());
                    return networkResponse;
                }).catch(err => {
                    console.debug("[SW] Network fetch failed, relying on cache.", err);
                    if (!cachedResponse) {
                        return new Response("<h1>Offline</h1><p>Failed to fetch App Shell.</p>", { status: 502, headers: { 'Content-Type': 'text/html' } });
                    }
                });

                return cachedResponse || await networkFetch;
            }

            try {
                // Find a client (window) to handle the WebRTC request
                const clientsList = await self.clients.matchAll({ includeUncontrolled: true, type: 'window' });
                const client = clientsList[0];

                if (!client) {
                    return new Response("<h1>Gateway Not Connected</h1><p>Please open the gateway page.</p>", {
                        status: 503,
                        headers: { 'Content-Type': 'text/html' }
                    });
                }

                return await proxyRequestToClient(client, event.request);

            } catch (err) {
                console.error("[SW] Proxy logic failed:", err);
                return new Response("<h1>Peer Proxy Error</h1><p>" + err.toString() + "</p>", {
                    status: 502,
                    headers: { 'Content-Type': 'text/html' }
                });
            }
        })()
    );
});

async function proxyRequestToClient(client, request) {
    const channel = new MessageChannel();

    const headers = [];
    for (const [k, v] of request.headers) {
        headers.push([k, v]);
    }

    let bodyStream = null;
    let transfer = [channel.port2];
    if (request.body) {
        bodyStream = request.body;
        transfer.push(bodyStream);
    }

    const msg = {
        type: 'REQUEST',
        url: request.url,
        method: request.method,
        headers: headers,
        body: bodyStream
    };

    const responsePromise = new Promise((resolve, reject) => {
        channel.port1.onmessage = (event) => {
            const data = event.data;
            if (data.type === 'RESPONSE') {
                resolve(new Response(data.body, { status: data.status, headers: new Headers(data.headers) }));
            } else if (data.type === 'ERROR') {
                resolve(new Response(data.message || "Unknown Error", { status: 502 }));
            }
        };
    });

    client.postMessage(msg, transfer);
    return responsePromise;
}
