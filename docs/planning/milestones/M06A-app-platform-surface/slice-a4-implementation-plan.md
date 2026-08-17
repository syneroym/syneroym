# M06A Slice A4 — The Demo App: Implementation Plan

> **Milestone:** [task.md](task.md)
> · **Slice:** A4 · **Depends on:** A1 (shipped), A2 (shipped), A3 (WebSocket)
>
> **Scope, from task.md:** "A WASM component providing the same functionality
> as `miniapp-demo1-web`: UI bundle via A1, REST endpoints via A2, upload/
> download via the existing `stream` target, and live updates **both ways —
> over SSE and over WebSocket**, so each is exercised end to end."
>
> **Renumbered 2026-08-14.** This slice was A3 when first planned. `D-06A-2`
> was then reversed — WebSocket is built rather than skipped — and the new
> WebSocket slice took A3, because its guest-side API is a design question
> better settled before the app is written than retrofitted after. The demo
> app became A4 and the Playwright suite A5. Decision ids moved `D-A3-n` →
> `D-A4-n`. Everything else below is the original plan, updated where the
> reversal touches it.

---

## §0 What the `D-06A-2` reversal changes here

*References: ADR-0010 (Topic Namespacing), ADR-0014 (Stream Types).*

The original plan replaced the native app's WebSocket with SSE throughout.
task.md now says build both. Concretely:

| Was | Now |
|---|---|
| `/ws` route "replaced by SSE" | A real WebSocket route via A3, **and** an SSE route alongside it |
| Client uses `EventSource` only | Client uses **both**: `EventSource` on `/api/events`, `WebSocket` on `/ws` |
| Echo behaviour "removed — SSE is server→client only" | **Restored** on the WebSocket path; SSE keeps the broadcast-only half |
| Playwright's `WebSocket Echo and Broadcast` rewritten over SSE | Runs **as-is** against a real WebSocket, plus a fifth SSE case (A5) |

The two transports are not redundant here — that is the point. SSE exercises
`subscribe-sse` and the broker fan-out; WebSocket exercises A3's guest boundary
and the bidirectional path. A divergence between them is a finding, which is
the same reason the native fixture is kept.

---

## §1 What the existing app does, verified against the tree

[`miniapp-demo1-web/src/lib.rs`](../../../../test-components/miniapp-demo1-web/src/lib.rs)
is a **native axum binary** deployed as a TCP passthrough service. It provides:

| Feature | How it works today | A4's equivalent |
|---|---|---|
| `GET /` — landing page | `index_handler` returns inline HTML with `<h1>Hello world...` | A1 static asset: an `index.html` served from the asset bundle |
| `GET /comments` — SPA shell | Serves `dist/index.html` (a SolidJS SPA) from embedded assets | A2 guest handler returning the SPA shell via `include_str!("../static/index.html")`. |
| `GET /api/comments` — list | SQLite query → JSON | A2 guest handler: reads `data-layer` store, returns JSON |
| `POST /api/comments` — save | SQLite insert, broadcasts via WebSocket | A2 guest handler: writes to `data-layer` store, publishes to an MQTT topic |
| `GET /api/files` — list | Reads the data directory | A2 guest handler: reads from `data-layer` store (file metadata) |
| `POST /api/files` — upload | Multipart form → local file | `stream` target (`accept-upload`): the existing stream protocol |
| `GET /api/files/{filename}` | Reads file from disk | `stream` target (`accept-download`): stream protocol for download |
| `GET /ws` — WebSocket | Broadcasts timestamps on comment save, echoes client messages | **A3's WebSocket route.** Echo via unicast `send`, broadcast via the pub-sub topic |
| `GET /api/events` — *(new)* | — | SSE via `subscribe-sse` on the same topic |
| Static assets | `rust-embed` fallback handler | A1 asset bundle |

### The SolidJS client

[`dist/assets/index-D4vAOYN-.js`](../../../../test-components/miniapp-demo1-web/static/dist/assets/index-D4vAOYN-.js)
is a bundled SolidJS SPA that:
- Shows a form to submit comments (POST `/api/comments`)
- Lists recent comments (GET `/api/comments`)
- Opens a **WebSocket** to `/ws` for live updates
- Has file upload/download UI (POST/GET `/api/files`, GET `/api/files/{name}`)

**A4 keeps the WebSocket and adds SSE beside it.** The `WebSocket Echo and
Broadcast` test from `webrtc.spec.ts:49-69` runs unchanged in shape; A5 adds a
fifth case for the SSE path.

---

## §2 Findings from reading the tree

### F1 — SPA deep links: A2 §9.8's open question is A4's to solve

[`webrtc.spec.ts:31`](../../../../crates/substrate/tests/e2e/tests/webrtc.spec.ts#L31):
`page.click('text=Comments etc.')` navigates to `/comments`. In the native app,
this is a server-defined route returning `dist/index.html`. In A4, `/comments`
is neither an asset path nor a declared `http_routes` entry — it is a
**client-side SPA route** that must return the SPA's `index.html`.

**Two options:**
1. **Make `/comments` a guest route** that returns the same `index.html`
   content.
2. **Add a `spa-fallback` mechanism to A1's asset serving.** A1 deliberately
   removed SPA fallback (`D-A1-11`).

**I recommend option 1** — a `guest` route returning the SPA shell using `include_str!`.

### F2 — The Playwright tests exercise specific UI selectors

The Playwright tests rely on specific text like `<h1>Hello world from demo1-instance0</h1>`, `text=Live Updates: Connected`, etc. A4's client must produce the exact same DOM structure.

### F3 — The `subscribe-sse` route is already wired

[`http.rs`](../../../../crates/router/src/route_handler/http.rs):
`"subscribe-sse" => self.handle_messaging_sse(route, req).await`. This takes a
`topic` from the route config and subscribes via the MQTT broker.

### F4 — The `accept-upload` stream target is already wired

[`http.rs`](../../../../crates/router/src/route_handler/http.rs):
`handle_stream_route` uses `accept-upload` to feed the request body to the guest.

### F5 — File download requires router work

`handle_stream_route` in A2 refuses any operation except `accept-upload`. A 1 MiB guest limit forces download to also use the stream target, which means the router must be updated to map an HTTP GET to `StreamDirection::Download` via an `accept-download` operation.

### F6 — Public routes and anonymous callers

A2's `validate_route` rejects `public: true` on any non-guest target. However, `stream` and `messaging` targets currently don't gate anonymous callers, meaning they are effectively public under direct WebRTC.

### F7 — The deploy path for A4

[`global-setup.ts`](../../../../crates/substrate/tests/e2e/global-setup.ts)
deploys `miniapp-demo1-web` as a **TCP passthrough**. A4 deploys as a **WASM component**.

---

## §3 Decisions

| # | Decision | Rationale |
|---|---|---|
| **D-A4-1** | `/comments` is a `guest` route returning the SPA `index.html` content via `include_str!("../static/index.html")`. | F1. Avoids reopening `D-A1-11`. Binds the WASM build to the static files layout. |
| **D-A4-2** | The client uses **both** `WebSocket` (`/ws`) and `EventSource` (`/api/events`) for live updates, with separate UI panels. | `D-06A-2` as reversed, F2. |
| **D-A4-3** | The component imports `data-layer/store`, `blob-store/blob-store`, `messaging/host-api`, A3's WebSocket send; exports `http/incoming-handler`, A3's `websocket-handler`, `messaging/guest-api`, and `messaging/stream-types`. | F3, F4, F5. Minimum imports needed. ADR-0014 requires `stream-types`. |
| **D-A4-4** | File metadata lives in `data-layer` (`files`). Upload and download use the `stream` target (`accept-upload`, `accept-download`), storing/reading via blob store. | F5. 1 MiB guest limit forces stream target for download. Router must be updated. |
| **D-A4-5** | `guest` and `websocket` routes declare `"public": true`. `stream` and `messaging` targets omit it (they do not gate anonymous callers today). | F6. `validate_route` rejects `public: true` on non-guest/websocket targets. |
| **D-A4-6** | The component lives at `test-components/miniapp-demo1-wasm/`. Package name `syneroym-test-miniapp-demo1-wasm`. | `D-06A-4`: the demo app is a fixture. |
| **D-A4-7** | The landing page uses `<h1>Hello world from demo1-instance0</h1>`, served from the asset bundle as `index.html`. | F2. Playwright expects these exact selectors. |
| **D-A4-8** | One topic (`comment-updates`) feeds both transports. The guest publishes once; the SSE route and the WebSocket broadcast both read it. | Keeps the two transports provably carrying identical events. |

---

## §4 Component structure

### 4.1 WIT world — `test-components/miniapp-demo1-wasm/wit/world.wit`

```wit
package syneroym-test:miniapp-demo1-wasm@0.1.0;

world miniapp-demo1-wasm {
    // Imports
    import syneroym:data-layer/store@0.1.0;
    import syneroym:blob-store/blob-store@0.1.0;
    import syneroym:messaging/host-api@0.1.0;
    import syneroym:http/websocket@0.1.0;

    // Exports
    export syneroym:http/incoming-handler@0.1.0;
    export syneroym:http/websocket-handler@0.1.0;
    export syneroym:messaging/guest-api@0.1.0;
    export syneroym:messaging/stream-types@0.1.0; // Required by ADR-0014

    // Lifecycle
    export init: func() -> result<_, string>;
    export migrate: func() -> result<_, string>;
}
```

### 4.2 Guest handler routes (`http-request.route` → behaviour)

| Route pattern | Method | Target | Operation | Behaviour |
|---|---|---|---|---|
| `/comments` | `GET` | `guest` | `handle-request` | Returns SPA HTML shell via `include_str!` |
| `/api/comments` | `GET` | `guest` | `handle-request` | Queries `comments` collection |
| `/api/comments` | `POST` | `guest` | `handle-request` | Parses JSON body, inserts into `comments`, publishes to MQTT topic |
| `/api/files` | `GET` | `guest` | `handle-request` | Queries `files` collection |
| `/api/files/{filename}` | `GET` | `stream` | `accept-download` | Stream download handler fetches blob and streams bytes |
| `/api/files` | `POST` | `stream` | `accept-upload` | Stream upload handler stores the file via blob store |
| `/api/events` | `GET` | `messaging` | `subscribe-sse` | SSE subscription to the comment-update topic |
| `/ws` | `GET` | `websocket` | `handle-upgrade` | A3's WebSocket: echo on inbound frame, broadcast from topic |

### 4.3 `custom-config` JSON (the `http_routes` table)

```json
{
  "http_routes": [
    {"method": "GET", "path": "/comments", "target": "guest", "operation": "handle-request", "public": true},
    {"method": "GET", "path": "/api/comments", "target": "guest", "operation": "handle-request", "public": true},
    {"method": "POST", "path": "/api/comments", "target": "guest", "operation": "handle-request", "public": true},
    {"method": "GET", "path": "/api/files", "target": "guest", "operation": "handle-request", "public": true},
    {"method": "GET", "path": "/api/files/{filename}", "target": "stream", "operation": "accept-download", "protocol": "file-download"},
    {"method": "POST", "path": "/api/files", "target": "stream", "operation": "accept-upload", "protocol": "file-upload"},
    {"method": "GET", "path": "/api/events", "target": "messaging", "operation": "subscribe-sse", "topic": "comment-updates"},
    {"method": "GET", "path": "/ws", "target": "websocket", "operation": "handle-upgrade", "topic": "comment-updates", "public": true}
  ]
}
```

### 4.4 Asset bundle contents

```
index.html          # Landing page with <h1>, <a href="/comments">
page1.html          # Static page (preserved from original)
dist/assets/*.js    # Built SolidJS bundle
```

---

## §5 The SolidJS client

Changes from the native client:

1. **WebSocket is kept**, pointed at `/ws`.
2. **Echo is kept.** The client sends `"Hi from client"`, guest echoes it back.
3. **An SSE panel is added**, subscribing to `/api/events`, with its own selector (e.g. `text=SSE: Connected`).
4. **File upload uses `POST /api/files`** with a raw body and `?metadata=<filename>` query param, changing from `multipart/form-data` to match the `stream` target's HTTP mapping.
5. **File download uses `GET /api/files/{name}`** via a link, handled by the `stream` target.

---

## §6 The WASM component — `test-components/miniapp-demo1-wasm/src/lib.rs`

### 6.1 `init()` and `migrate()`

- Creates `comments` and `files` collections.
- Registers stream protocols: `register-stream-protocol("file-upload")` and `register-stream-protocol("file-download")` in **both** `init()` and `migrate()`.

### 6.2 `handle-request` dispatch

```rust
match (request.method.as_str(), request.route.as_str()) {
    ("GET", "/comments")              => serve_comments_page(),
    ("GET", "/api/comments")          => get_comments(),
    ("POST", "/api/comments")         => save_comment(&request),
    ("GET", "/api/files")             => list_files(),
    _                                 => ok_text(404, "not found"),
}
```

### 6.3 `accept-stream-upload` & `handle-stream-request` (download)

**Upload (`file-upload` via `accept-stream-upload`):**
1. Reads filename from stream metadata.
2. Reads chunks from `stream-sink` → assembles bytes.
3. Stores via `blob-store::put-blob` → gets hash.
4. Records `{name, hash, size}` in the `files` collection.

**Download (`file-download` via `handle-stream-request`):**
The component exports `syneroym:messaging/guest-api::handle-stream-request(protocol, peer-id, request-data) -> stream-cursor`. When called for download:
1. Reads filename from `request-data`.
2. Looks up blob hash from `files` collection.
3. Fetches the blob via `get-blob`.
4. The guest pushes bytes through `stream-cursor::next-chunk`.

### 6.4 WebSocket handlers (A3)

```rust
on_open(conn)     => nothing; router handles topic binding via route config.
on_message(conn, frame) => send(conn, echo_payload(frame))   // unicast echo
on_close(conn)    => nothing; the host drops the binding
```

### 6.5 Comment save → publish → both transports

When `POST /api/comments` saves a comment, the guest calls `host-api::publish("comment-updates", timestamp_json)` **once**. Both SSE and WebSocket broadcast read that topic.

---

## §7 Proposed changes

### Router Changes (A2-shaped)

1. **`accept-download` support in `http.rs`:**
   - Modify `handle_stream_route` to support `operation == "accept-download"`, delegating to `StreamDirection::Download`.
   - Modify `dispatch_route` to pass `path_param` into `handle_stream_route`, which is used as the `initial_payload` (or fallback to `?metadata=`).
   - `handle_stream_protocol_request` takes a `writer: Box<dyn AsyncWrite>`. The router must bridge this duplex/channel into a hyper `StreamBody` and send HTTP 200 with headers immediately while streaming the body out.
2. **`validate_route` arm:** Add an arm to `control_plane/src/http_routes.rs` to allow `("stream", "accept-download")`.
3. **SSE Admission Control:** Add a bounded semaphore to `handle_messaging_sse` in `http.rs` to limit concurrent SSE subscribers (fulfilling failure-matrix row 8).

### Test-component (new files)

**[NEW] `test-components/miniapp-demo1-wasm/`**

| File | Purpose |
|---|---|
| `Cargo.toml` | Package `syneroym-test-miniapp-demo1-wasm`, deps: `wit-bindgen`, `serde_json`. Must include `[package.metadata.component.target.dependencies]`. |
| `src/lib.rs` | Guest component (§6) |
| `wit/world.wit` | WIT world (§4.1) |
| `client/` | SolidJS source (package.json, src/*, vite.config.js) |
| `static/index.html` | Landing page |
| `static/page1.html` | Copy from original |
| `routes.json` | The `http_routes` JSON (§4.3) |

### Workspace build

1. Add `test-components/miniapp-demo1-wasm` to the `exclude` array in the root `Cargo.toml`.
2. Add the component to `mise.toml` `build:test-components` and `.github/actions/ci-build-and-test/action.yml`.
3. Client assets are built from `client/` at fixture build time via `mise run build:test-components` (which manages Node 20 as declared in `mise.toml` and runs `npm ci && npm run build` before invoking `cargo component build`). Generated `static/dist/` artifacts remain gitignored, matching `miniapp-demo1-web` and workspace build conventions.

**[MODIFY] `crates/core/src/test_constants.rs`**: Add `miniapp_demo1_wasm_path()` and `MINIAPP_DEMO1_WASM_ROUTES_JSON`.

**[MODIFY] `test-components/README.md`**: Add entry for new component.

---

## §8 Verification plan

### Exit criteria mapping

| # | Exit criterion | How A4 satisfies it |
|---|---|---|
| 1 | Deploys as a single WASM component | Native E2E test asserts instantiation. |
| 3 | `GET /` completes without instantiating the component | Native E2E test asserting instantiation delta = 0 |
| 4 | Repeat asset load returns 304 from ETag | A1's existing behaviour, tested by Playwright |
| 5 | POST rejected by guest returns guest's own status and message | Native E2E test asserting 422 + error payload; Playwright suite |
| 6 | Upload/download round-trips byte-identical | Native E2E test via stream HTTP routes; Playwright suite |
| 7 | SSE subscriber receives update from another session | Native E2E test across distinct client sessions; Playwright A5 |
| 8 | WebSocket echo and broadcast | Native E2E test with text and binary opcodes; Playwright A5 |
| 9 | Cover all rows in the failure matrix | A4 claims row 8 (SSE bounding, `max_sse_subscribers_per_service`). Other WebSocket rows belong to A3 but impact A4/A5. |
| 10 | Commands block runs successfully | `cargo test / clippy / fmt / mise run test:e2e` pass natively. |

### Automated tests

**New Native E2E Test:** Add `crates/substrate/tests/miniapp_demo1_wasm_e2e.rs` to assert:
1. Deploys as a single WASM component (Criterion 1).
2. Guest publish → SSE subscriber (proving `host-api::publish` reaches `/api/events` end-to-end without a browser).
3. Upload/download round-trips over the `stream` HTTP routes.
4. WebSocket echo and pubsub broadcast.

**SSE WebRTC Proxy Spike:** Streaming `EventSource` over the browser WebRTC proxy (`sw.js` + `peer-proxy.js`) remains an open integration point to be proven end-to-end by the Playwright browser suite in Slice A5 (the native E2E tests in A4 prove SSE streaming over Iroh QUIC).

### Manual verification / Commands Block

```bash
cargo +nightly fmt --all
cargo clippy --workspace --all-targets --all-features
cargo test --workspace
mise run test:e2e
```

---

## §9 Documentation, Dependencies, and Deferred-backlog items

1. **Update `status.md`:** Add a "What shipped" + evidence section for A4 (Demo App) to `docs/planning/milestones/M06A-app-platform-surface/status.md`.
2. **Notes on neighbours:**
   - **WebRTC WebSocket shim:** `peer-proxy.js` monkeypatches `window.WebSocket` with a `SynWebSocket` class performing its own HTTP upgrade (Upgrade: websocket, Sec-WebSocket-Key, version 13) over the data channel. The A3 101 handshake must answer exactly what that shim sends.
   - **WebSocket failure matrix:** WebSocket-specific failure matrix rows (concurrent connections, guest traps, frame size caps, half-open connections) belong in A3 but impact A4 and A5 testing.
3. **SPA history fallback in A1's asset serving.** (Update existing row in `deferred-backlog.md:50`). A4 works around it with a guest route, but a real app would want `/*` → `index.html` without instantiating the component.
4. **Multipart form upload through the guest handler.** The stream protocol approach works but is a different client API than standard `multipart/form-data`. A real web app would want the guest to receive a multipart body directly.
