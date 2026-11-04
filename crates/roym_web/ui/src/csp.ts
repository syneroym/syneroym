// The Hub page has two Content Security Policy `<meta>` tags. A browser
// enforces every policy it gets, so the narrower rule wins. `index.html`
// holds the fixed rules and a wide `connect-src`. This module adds the exact
// one: the Hub may call itself and its own auth service, and nothing else.
// The auth origin depends on the host the Hub is served from, so no static
// file can name it.

/** The `connect-src` policy for a Hub whose auth service is `origin`. */
export function connectPolicy(origin: string): string {
  return `connect-src 'self' ${origin}`;
}

/** Adds that policy as a second CSP `<meta>` in `doc.head`. */
export function installConnectPolicy(doc: Document, origin: string): void {
  const meta = doc.createElement("meta");
  meta.httpEquiv = "Content-Security-Policy";
  meta.content = connectPolicy(origin);
  doc.head.appendChild(meta);
}
