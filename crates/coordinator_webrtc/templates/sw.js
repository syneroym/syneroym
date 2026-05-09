// Service Worker Logic

self.addEventListener('install', (event) => {
    console.debug('[SW] Installing');
    self.skipWaiting();
});

self.addEventListener('activate', (event) => {
    console.debug('[SW] Activating');
    event.waitUntil(clients.claim());
});

self.addEventListener('fetch', (event) => {
    const url = new URL(event.request.url);
    if (url.origin !== self.location.origin) return;
    if (url.searchParams.has('sw')) return;
    if (url.pathname === '/__syneroym/sw.js') return;
    console.debug("[SW] ----- Starting overridden Fetch for", event)

    event.respondWith(
        (async () => {
            // Always serve App Shell for navigation to keep the proxy logic alive
            if (event.request.mode === 'navigate') {
                console.debug("[SW] Navigation request detected. Serving App Shell.");
                return fetch(event.request);
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
    return new Promise(async (resolve, reject) => {
        const channel = new MessageChannel();

        const headers = [];
        for (const [k, v] of request.headers) {
            headers.push([k, v]);
        }

        const msg = {
            type: 'REQ_HEAD',
            url: request.url,
            method: request.method,
            headers: headers,
            hasBody: !!request.body
        };

        channel.port1.onmessage = (event) => {
            const data = event.data;

            if (data.type === 'REQ_BODY_START') {
                console.debug("[SW] Received msg REQ_BODY_START from client");
                if (request.body) {
                    console.debug("[SW] and request has body:", request.body);
                    const reader = request.body.getReader();
                    (async () => {
                        try {
                            while (true) {
                                const { done, value } = await reader.read();
                                if (done) {
                                    if (value) {
                                        channel.port1.postMessage({ type: 'REQ_BODY_END', chunk: value }, [value.buffer]);
                                    } else {
                                        channel.port1.postMessage({ type: 'REQ_BODY_END' });
                                    }
                                    break;
                                }
                                channel.port1.postMessage({ type: 'REQ_BODY_CHUNK', chunk: value }, [value.buffer]);
                            }
                        } catch (err) {
                            console.error("[SW] Stream read error", err);
                            channel.port1.postMessage({ type: 'REQ_BODY_ERROR', message: err.toString() });
                        }
                    })();
                } else {
                    channel.port1.postMessage({ type: 'REQ_BODY_END' });
                }
            } else if (data.type === 'RESPONSE_HEAD') {
                const stream = new ReadableStream({
                    start(controller) {
                        // We need to listen to messages for chunks now.
                        // Since we are inside the onmessage handler, we need to handle future messages here
                        // OR (better) we update the main handler to dispatch to the controller.

                        // Actually, purely defining the stream here is tricky because we lose the 'onmessage' scope.
                        // Let's attach the controller to a shared variable or update the handler.
                        channel.port1.onmessage = (evt) => {
                            const d = evt.data;
                            if (d.type === 'RESPONSE_CHUNK') {
                                controller.enqueue(d.chunk);
                            } else if (d.type === 'RESPONSE_END') {
                                controller.close();
                                channel.port1.close();
                            }
                        };
                    }
                });
                resolve(new Response(stream, { status: data.status, headers: new Headers(data.headers) }));
            } else if (data.type === 'ERROR') {
                resolve(new Response(data.message || "Unknown Error", { status: 502 }));
            }
        };

        client.postMessage(msg, [channel.port2]);
    });
}

