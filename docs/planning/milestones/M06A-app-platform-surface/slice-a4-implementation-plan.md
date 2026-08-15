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
| `GET /` — landing page | `index_handler` returns inline HTML with `<h1>Hello world from demo1-instance0</h1>` and a link `Comments etc.` | A1 static asset: an `index.html` served from the asset bundle |
| `GET /comments` — SPA shell | Serves `dist/index.html` (a SolidJS SPA) from embedded `rust-embed` assets | A1 static asset: `comments.html` (or the same `dist/index.html`) served from the asset bundle. **Requires a route or SPA mechanism** — see §2 |
| `GET /api/comments` — list comments | SQLite query → JSON | A2 guest handler: reads `data-layer` store, returns JSON |
| `POST /api/comments` — save comment | SQLite insert, broadcasts timestamp via WebSocket | A2 guest handler: writes to `data-layer` store, publishes to an MQTT topic — which fans out to **both** the SSE subscribers and the WebSocket connections (A3's broadcast reuses the broker) |
| `GET /api/files` — list files | Reads the data directory | A2 guest handler: reads from `data-layer` store (file metadata) |
| `POST /api/files` — upload file | Multipart form → local file | `stream` target (`accept-upload`): the existing stream protocol |
| `GET /api/files/{filename}` — download | Reads file from disk | `stream` target or A2 guest handler returning blob bytes |
| `GET /ws` — WebSocket | Broadcasts timestamps on comment save, echoes client messages | **A3's WebSocket route.** Both behaviours preserved: echo via unicast `send`, broadcast via the pub-sub topic |
| `GET /api/events` — *(new)* | — | SSE via `subscribe-sse` on the same topic, so both transports carry the same events |
| Static assets (`/dist/assets/*`, `/page1.html`) | `rust-embed` fallback handler | A1 asset bundle |

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
   content. The guest reads it from blob store and returns it. Simple, no new
   machinery, but means the component is instantiated for what is logically a
   static page.
2. **Add a `spa-fallback` mechanism to A1's asset serving.** A1 deliberately
   removed SPA fallback (revision-2 finding 7, `D-A1-11`). Adding it back needs
   care to avoid swallowing `/api/*`.

**I recommend option 1** — a `guest` route returning the SPA shell. The cost is
one instantiation per navigation to `/comments`, which is nothing compared to
the API calls on that page. It avoids reopening `D-A1-11`'s removed scope.

### F2 — The Playwright tests exercise specific UI selectors

The Playwright tests rely on:
- `<h1>Hello world from demo1-instance0</h1>` on the landing page
- `text=Comments etc.` link
- `<h2>Comments</h2>` on the comments page
- `textarea[placeholder="Write a comment..."]`
- `button:has-text("Submit")`
- `text=Comment saved!`
- `text=Live Updates: Connected` — driven by the **WebSocket** `onopen`, as in
  the native app. The SSE panel gets its own distinct selector so A5 can assert
  on each transport separately
- `input[type="file"]`, `button:has-text("Upload")`, `text=Upload successful!`
- `ul` elements for comment and file lists

A4's client must produce the same DOM structure and text content for these
selectors.

### F3 — The `subscribe-sse` route is already wired

[`http.rs`](../../../../crates/router/src/route_handler/http.rs):
`"subscribe-sse" => self.handle_messaging_sse(route, req).await`. This takes a
`topic` from the route config and subscribes via the MQTT broker, streaming
`text/event-stream` frames. The guest publishes to this topic via
`syneroym:messaging/host-api::publish`.

### F4 — The `accept-upload` stream target is already wired

[`http.rs`](../../../../crates/router/src/route_handler/http.rs):
`handle_stream_route` uses `accept-upload` to feed the request body to the
guest's `accept-stream-upload` export.

### F5 — File download: guest handler returning blob bytes vs stream protocol

The native app reads from the filesystem. A4 stores uploaded files as blobs. For
download, the guest handler can read a blob via
`syneroym:blob-store/blob-store::get-blob` and return the bytes. At typical test
file sizes (~20 bytes in Playwright tests) this fits easily within the 1 MiB
guest response cap. No stream protocol needed for download.

### F6 — All guest routes need `public: true` for WebRTC tests

A2's `D-A2-7` and §9.10: every route must declare `public: true` because exit
criterion 2 runs in the **direct WebRTC** configuration where `caller = None`.
**This applies to the WebSocket route too** — A3's plan should confirm the
upgrade path honours the same `public` declaration, since an anonymous browser
is exactly what opens it.

### F7 — The deploy path for A4

[`global-setup.ts`](../../../../crates/substrate/tests/e2e/global-setup.ts)
deploys `miniapp-demo1-web` as a **TCP passthrough**
(`--tcp 127.0.0.1:3000`). A4 deploys as a **WASM component** with an asset
bundle and `custom-config` carrying `http_routes`. The test harness for A5 (the
Playwright suite) will use a different `global-setup` that:
- Builds the WASM component for `wasm32-wasip2`
- Builds the SolidJS client
- Creates the asset archive (`.tar.gz`)
- Deploys with `roymctl svc deploy --svc-id ... --interfaces http --wasm <path> --assets <archive> --asset-visibility public --custom-config <routes.json>`

### F8 — the asset bundle must fit A1's frame budget

`D-A1-5`: the deploy frame is 16 MiB, shared with the component binary, and
both expand ~3.57× as JSON integer arrays. A SolidJS bundle is small (tens of
KB), so this is not a constraint in practice — but `build.sh` should fail loudly
rather than produce an archive that only fails at deploy.

---

## §3 Decisions

| # | Decision | Rationale |
|---|---|---|
| **D-A4-1** | `/comments` is a `guest` route returning the SPA `index.html` content, read from the blob store at deploy time (stored via `data-layer` at `init`). Not a new SPA-fallback mechanism in A1. | F1. Avoids reopening `D-A1-11`. One extra instantiation per navigation is nothing beside the API calls that follow. |
| **D-A4-2** | The client is a new SolidJS SPA in `test-components/miniapp-demo1-wasm/client/`, using **both** `WebSocket` (`/ws`) and `EventSource` (`/api/events`) for live updates, with separate UI panels so each is independently assertable. The landing page is a static `index.html` in the asset bundle. | `D-06A-2` as reversed, F2. Two transports over one topic is the cheapest way to prove both work. |
| **D-A4-3** | The component imports `syneroym:data-layer/store`, `syneroym:blob-store/blob-store`, `syneroym:messaging/host-api`, and A3's WebSocket send import; exports `syneroym:http/incoming-handler`, A3's `websocket-handler`, and `syneroym:messaging/guest-api` (for `accept-stream-upload`). | F3, F4, F5, plus A3. These are the minimum imports needed. |
| **D-A4-4** | File metadata (name → blob hash mapping) lives in a `data-layer` collection (`files`). Upload writes a blob and records the mapping. Download reads the mapping, fetches the blob, returns bytes. | F5. No filesystem in WASM. |
| **D-A4-5** | Every `http_routes` entry declares `"public": true`, the WebSocket route included. | F6. WebRTC tests send anonymous traffic. |
| **D-A4-6** | The component lives at `test-components/miniapp-demo1-wasm/`, excluded from the workspace graph (like every other `test-components/` entry). Package name `syneroym-test-miniapp-demo1-wasm`. | `D-06A-4`: the demo app is a fixture. |
| **D-A4-7** | The landing page (`GET /`) uses `<h1>Hello world from demo1-instance0</h1>` and `<a href='/comments'>Comments etc.</a>`, served from the asset bundle as `index.html`. | F2. Playwright expects these exact selectors. The `demo1-instance0` name is hardcoded. |
| **D-A4-8** | One topic (`comment-updates`) feeds both transports. The guest publishes once; the SSE route and the WebSocket broadcast both read it. The guest never fans out itself. | This is what makes A3's "broadcast reuses pub-sub" claim real rather than aspirational, and it keeps the two transports provably carrying identical events. |

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
    // A3: unicast send on an open connection (exact name per A3's plan)
    import syneroym:http/websocket@0.1.0;

    // Exports
    export syneroym:http/incoming-handler@0.1.0;
    export syneroym:http/websocket-handler@0.1.0;   // A3
    export syneroym:messaging/guest-api@0.1.0;

    // Lifecycle
    export init: func() -> result<_, string>;
    export migrate: func() -> result<_, string>;
}
```

> The two A3 interface names are provisional — A4 takes whatever A3's plan
> settles on. They are named here so the dependency is explicit, not to
> pre-empt that decision.

### 4.2 Guest handler routes (`http-request.route` → behaviour)

| Route pattern | Method | Target | Operation | Behaviour |
|---|---|---|---|---|
| `/comments` | `GET` | `guest` | `handle-request` | Returns the SPA HTML shell (stored in `data-layer` at init) |
| `/api/comments` | `GET` | `guest` | `handle-request` | Queries `comments` collection, returns JSON array |
| `/api/comments` | `POST` | `guest` | `handle-request` | Parses JSON body, inserts into `comments`, publishes timestamp to the MQTT topic |
| `/api/files` | `GET` | `guest` | `handle-request` | Queries `files` collection, returns JSON array of `{name, size}` |
| `/api/files/{filename}` | `GET` | `guest` | `handle-request` | Looks up blob hash from `files` collection, fetches blob via `get-blob`, returns bytes |
| `/api/files` | `POST` | `stream` | `accept-upload` | Stream upload handler stores the file via blob store |
| `/api/events` | `GET` | `messaging` | `subscribe-sse` | SSE subscription to the comment-update topic |
| `/ws` | `GET` | `websocket` | `handle-upgrade` | A3's WebSocket: echo on inbound frame, broadcast from the topic |

### 4.3 `custom-config` JSON (the `http_routes` table)

```json
{
  "http_routes": [
    {"method": "GET", "path": "/comments", "target": "guest", "operation": "handle-request", "public": true},
    {"method": "GET", "path": "/api/comments", "target": "guest", "operation": "handle-request", "public": true},
    {"method": "POST", "path": "/api/comments", "target": "guest", "operation": "handle-request", "public": true},
    {"method": "GET", "path": "/api/files", "target": "guest", "operation": "handle-request", "public": true},
    {"method": "GET", "path": "/api/files/{filename}", "target": "guest", "operation": "handle-request", "public": true},
    {"method": "POST", "path": "/api/files", "target": "stream", "operation": "accept-upload", "protocol": "file-upload"},
    {"method": "GET", "path": "/api/events", "target": "messaging", "operation": "subscribe-sse", "topic": "comment-updates"},
    {"method": "GET", "path": "/ws", "target": "websocket", "operation": "handle-upgrade", "topic": "comment-updates", "public": true}
  ]
}
```

> The `websocket` target's exact field names follow A3. `topic` is shown here
> because `D-A4-8` needs the connection bound to the same topic the SSE route
> uses; if A3 instead has the guest subscribe at `on-open`, this field goes
> away and the behaviour is unchanged.

### 4.4 Asset bundle contents

```
index.html          # Landing page with <h1>, <a href="/comments">
page1.html          # Static page (preserved from original)
dist/index.html     # SPA shell (comments page)
dist/assets/*.js    # Built SolidJS bundle
```

---

## §5 The SolidJS client

The client in `test-components/miniapp-demo1-wasm/client/` is a minimal SolidJS
app producing the **exact same DOM selectors** as the existing one, plus one new
panel. Changes from the native client:

1. **WebSocket is kept**, pointed at `/ws`. `onopen` sets
   `Live Updates: Connected`, exactly as today, so `webrtc.spec.ts`'s existing
   assertions hold.
2. **Echo is kept.** The client sends `"Hi from client"`, the guest echoes it
   back over the same connection, and the client renders it — the round trip
   the native test asserts on.
3. **An SSE panel is added**, subscribing to `/api/events`, with its own
   selector (e.g. `text=SSE: Connected`) so A5 can assert on it independently.
   Both panels display events from the same topic, so a single comment save
   updates both.
4. **File upload uses `POST /api/files`** with `multipart/form-data`, same as
   before.
5. **File download uses `GET /api/files/{name}`** via a link, same as before.

> [!IMPORTANT]
> Because both panels are fed by one topic (`D-A4-8`), a comment save must
> update both. A test that sees one panel update and not the other has found a
> real defect in whichever transport stayed silent — that is the divergence
> check this slice exists to provide, and it is cheaper than any amount of
> reasoning about whether the two paths agree.

---

## §6 The WASM component — `test-components/miniapp-demo1-wasm/src/lib.rs`

### 6.1 `init()`

- Creates `comments` collection (id, text)
- Creates `files` collection (name → payload containing `{hash, size}`)
- Stores the SPA HTML shell in the `comments` collection for the `/comments`
  route to return

### 6.2 `handle-request` dispatch

```rust
match (request.method.as_str(), request.route.as_str()) {
    ("GET", "/comments")              => serve_comments_page(),
    ("GET", "/api/comments")          => get_comments(),
    ("POST", "/api/comments")         => save_comment(&request),
    ("GET", "/api/files")             => list_files(),
    ("GET", "/api/files/{filename}")  => download_file(&request),
    _                                 => ok_text(404, "not found"),
}
```

### 6.3 `accept-stream-upload` (file upload)

The component exports `syneroym:messaging/guest-api::accept-stream-upload`. When
the stream protocol `"file-upload"` fires, the guest:
1. Reads the filename from stream metadata
2. Reads chunks from `stream-sink` → assembles bytes
3. Stores via `blob-store::put-blob` → gets hash
4. Records `{name, hash, size}` in the `files` collection

### 6.4 WebSocket handlers (A3)

```rust
on_open(conn)     => bind conn to the "comment-updates" topic, per D-A4-8
on_message(conn, frame) => send(conn, echo_payload(frame))   // unicast echo
on_close(conn, _) => nothing; the host drops the binding
```

The guest never iterates connections and never fans out — broadcast is the
broker's job. Each callback is a short call, well inside the 5s
`dispatch_epoch_timeout_secs` bound that makes the host own connection lifetime
in the first place.

### 6.5 Comment save → publish → both transports

When `POST /api/comments` saves a comment, the guest calls
`host-api::publish("comment-updates", timestamp_json)` **once**. The
`subscribe-sse` route on `/api/events` and A3's WebSocket broadcast both read
that topic. One publish, two egresses, no guest-side fan-out.

---

## §7 Proposed changes

### Test-component (new files)

#### [NEW] `test-components/miniapp-demo1-wasm/`

| File | Purpose |
|---|---|
| `Cargo.toml` | Package `syneroym-test-miniapp-demo1-wasm`, `crate-type = ["cdylib"]`, deps: `wit-bindgen`, `serde_json` |
| `src/lib.rs` | Guest component (§6) |
| `wit/world.wit` | WIT world (§4.1) |
| `wit/deps/http/http.wit` | Copy of `crates/wit_interfaces/wit/http/http.wit` (includes A3's WebSocket interfaces) |
| `wit/deps/data-layer/data-layer.wit` | Copy of `syneroym:data-layer` |
| `wit/deps/blob-store/blob-store.wit` | Copy of `syneroym:blob-store` |
| `wit/deps/messaging/messaging.wit` | Copy of `syneroym:messaging` |
| `client/` | SolidJS source (package.json, src/*, vite.config.js) |
| `static/index.html` | Landing page (`<h1>Hello world...`, link to `/comments`) |
| `static/page1.html` | Copy from original |
| `routes.json` | The `http_routes` JSON (§4.3) |
| `build.sh` | `cd client && npm install && npm run build`, then `tar czf assets.tar.gz -C static .`; **fails if the archive exceeds A1's `MAX_ASSET_BUNDLE_BYTES`** (F8) |

#### [MODIFY] [`test-components/README.md`](../../../../test-components/README.md)

One line for the new component.

#### [MODIFY] [`crates/core/src/test_constants.rs`](../../../../crates/core/src/test_constants.rs)

Add `miniapp_demo1_wasm_path()` and `MINIAPP_DEMO1_WASM_ROUTES_JSON`.

### Workspace build (no changes needed)

The new component is excluded from the workspace graph like every other
`test-components/` entry (root `Cargo.toml`'s `exclude`).

---

## §8 Verification plan

### Exit criteria mapping

| # | Exit criterion | How A4 satisfies it |
|---|---|---|
| 1 | Deploys as a single WASM component | The component is built for `wasm32-wasip2`, deployed with `roymctl svc deploy --wasm` |
| 3 | `GET /` completes without instantiating the component | Served from A1's asset bundle; tested by asserting instantiation delta = 0 |
| 4 | Repeat asset load returns 304 from ETag | A1's existing behaviour, tested by the Playwright suite |
| 5 | POST rejected by guest returns guest's own status and message | `POST /api/comments` with empty text → guest returns 400 |
| 6 | Upload/download round-trips byte-identical | Playwright test uploads a file, downloads it, asserts content matches |
| 7 | SSE subscriber receives update from another session | A5's test; the plumbing is here (`/api/events`, `D-A4-8`) |
| 8 | WebSocket echo and broadcast | A5's test; the plumbing is here (`/ws`, §6.4, `D-A4-8`) |

Exit criteria 2 (Playwright cases pass) and 9 (failure matrix tests) are
**A5's** scope. A4 provides the component that A5 tests.

### Automated tests

```bash
cd test-components/miniapp-demo1-wasm
cargo build --release --target wasm32-wasip2
cd client && npm install && npm run build && cd ..
./build.sh
```

```bash
cargo +nightly fmt --all
cargo clippy --workspace --all-targets --all-features
cargo test --workspace
```

### Manual verification

1. `GET /` shows the landing page with the expected heading
2. Click "Comments etc." navigates to the SPA
3. Submit a comment, see it appear in the list
4. Open a second browser tab — submitting a comment in one updates the other
   **on both panels**, WebSocket and SSE
5. Type into the echo box — the WebSocket panel shows the echoed message
6. Upload a file, see it listed, download it and verify content

---

## §9 Deferred-backlog items A4 will record

1. **SPA history fallback in A1's asset serving.** A4 works around it with a
   guest route (`D-A4-1`), but a real app would want `/*` → `index.html`
   without instantiating the component. The mechanism needs care (`D-A1-11`'s
   removed fallback, `D-A1-4`'s collision detection).
2. **Multipart form upload through the guest handler.** The stream protocol
   approach works but is a different client API than standard
   `multipart/form-data`. A real web app would want the guest to receive a
   multipart body directly.
