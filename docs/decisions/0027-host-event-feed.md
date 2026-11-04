# ADR 0027: Host event feed

Reader: developers building services or clients on Syneroym, and operators running substrates.

## Status
Accepted

## Related
- ADR-0016 (caller identity on native dispatch)
- ADR-0020 (stable logical service identity and member certificates)
- ADR-0024 (client gateway and auth service session tokens)
- ADR-0025 (conversation capability owns history)

## Context
A client or local service needs to learn when data changes without polling over JSON-RPC. For example, a user viewing a chat thread wants incoming messages and delivery receipts to appear right away. Before this decision, the user interface polled by reopening screens or clicking refresh buttons.

A substrate can host multiple services and app instances. Different apps must not see each other's events. Event delivery must be lightweight, safe under untrusted guests, and resilient when a browser reconnects or sleeps.

## Decision
1. **One event bus per substrate.** The substrate owns an in-memory event bus (`EventBus`). It is shared across services and the HTTP router.
2. **Strict scoping by app instance.** Every event belongs to exactly one app instance. The host tags each event using the writer's registered app instance. A service outside any app instance cannot record events. A reader receives only the events of its own app instance.
3. **Small metadata-only events.** An event carries:
   - `cursor`: a run identifier and a monotonic sequence number (`<run_id>-<seq>`).
   - `kind`: what changed (for example, `conversation.message` or `transaction.card`), up to 64 bytes.
   - `service`: the service alias that recorded it, up to 64 bytes.
   - `subject`: the entity that changed (for example, a conversation identifier), up to 128 bytes.
   - `item`: an optional sub-entity (for example, a message identifier), up to 128 bytes.
   - `at_ms`: millisecond UNIX timestamp.
   An event carries no payload body and no secrets. A client reads the event metadata and fetches detailed state through standard service methods.
4. **Ring buffer in memory.** Each app instance keeps an in-memory window of up to 10 000 events, retained for at most 24 hours. The window is not persisted to disk. On restart, the bus starts with a new run identifier.
5. **Cursor catch-up and reload.** A reader tracks the last cursor it received.
   - If the cursor is valid and within the current window, the bus delivers missed events oldest-first.
   - If the cursor is from a previous run (after a restart), older than the retained window, ahead of the head, or malformed, the bus signals `reload`.
   - On `reload`, the client re-fetches its baseline data and continues from the current head cursor.
6. **Server-Sent Events (SSE) for external clients.** The router exposes `GET /_syneroym/events` on each service hostname.
   - Requires `Accept: text/event-stream`.
   - Requires an authenticated gateway session of the app's owner. Non-owners get `403 Forbidden`. Requests without a valid session get `401 Unauthorized`.
   - Browser authentication: `EventSource` cannot send custom HTTP headers like `Authorization: Bearer`. For browser clients, the session token is sent via a cookie named `syneroym_session` scoped strictly to `Path=/_syneroym/events` with `SameSite=Lax` and `Max-Age` of one hour. The Hub writes the cookie again before every connect, so it does not need to know the token's own expiry. The router rejects an expired token, so a cookie that outlives its session grants nothing. RPC endpoints continue to authenticate via `Authorization: Bearer` headers.
   - Feed stream lifecycle and revocation: An open SSE stream validates session revocation and token expiry on connect and on every wake-up tick (event change or 25-second keep-alive). If the session token is revoked or expires, the router immediately terminates the stream. When the client attempts to reconnect, it receives `401 Unauthorized`.
   - Answers with four frame types: `ready` (on initial connect without a cursor), `change` (one frame per event), `reload` (when catch-up is not possible), and `: keep-alive` comments (sent every 25 seconds of silence).
   - The gateway forwards `/_syneroym/events` to the service named in the `Host` header.
7. **Pull interface for guest services.** WebAssembly components and native services read and record events through the `syneroym:events@0.1.0` interface (`record` and `read`). There is no push export to guests in this version.
8. **Concurrency and backpressure limits.**
   - At most 16 concurrent feed streams per app instance. Additional connection attempts receive `503 Service Unavailable` with `Retry-After: 1`.
   - Single-read limit is capped at 500 events per query to avoid unbounded memory buffers.
9. **Commit-before-announce rule.** A service records an event only after its database transaction has committed. If an event record fails, the underlying transaction and method call do not fail.
10. **Browser connection management.** Because browser HTTP/1.1 connection pools allow about 6 connections per host, the user interface holds a feed open only while its tab is visible. When hidden, it closes the feed. When shown again, it reconnects using its last cursor. On disconnection, it reconnects with exponential backoff and jitter.

## Consequences
- A user sees new messages and state changes live with no manual refresh or polling loops.
- Memory usage in the substrate is strictly bounded by constants.
- Service code never needs WebSocket plumbing for event notifications.
- Restarting a substrate invalidates client cursors, causing connected clients to execute one clean reload.
- Mobile and background browser tabs drop connections cleanly without exhausting gateway connection slots.

## Alternatives considered
- **WebSocket connection.** Rejected: SSE is simpler, unidirectional, works over standard HTTP/1.1 proxies, and has built-in browser reconnection support.
- **Durable event log on disk.** Rejected: adds storage overhead and migration complexity for transient notifications. State is already stored in the underlying service database; the event feed only announces changes.
- **Push callback to guest components.** Postponed: requires asynchronous guest callback infrastructure and lifecycle management. Pull-based reading is sufficient for current services.
- **Passing payload data in events.** Rejected: payload data would duplicate storage, risk leaking encrypted or private fields across services, and make events unbounded in size.
