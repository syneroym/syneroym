# M06A Slice A5 — The Playwright Suite: Implementation Plan

> **Milestone:** [task.md](task.md) · **Slice:** A5 · **Depends on:** A1, A2,
> A3, A4 (all Complete) · **Status:** planned, not started
>
> **Scope, from task.md:** "The four cases from `webrtc.spec.ts` re-pointed at
> the demo app — including echo/broadcast over a real WebSocket, matching the
> native fixture — plus a fifth covering the SSE path."
>
> This plan is written to be executed without further reasoning: every type
> and signature change, every call site, and pseudo-code for the non-obvious
> parts. Where an input document is ambiguous or stale against the tree, §8
> says so instead of guessing.

---

## §1 What A5 has to close

A4 proved the demo app works over **Iroh QUIC, from Rust**
(`crates/substrate/tests/miniapp_demo1_wasm_e2e.rs`, 6 tests). A5 proves the
same app works **from a real browser, over the WebRTC data channel and the
coordinator blind tunnel**, driven by Playwright.

Milestone exit criteria and who closes them:

| # | Criterion | Closed by |
|---|---|---|
| 1 | Deploys as a single WASM component | A5's `global-setup.ts` (a real `roymctl svc deploy --wasm`, no TCP/native helper for this app) |
| 2 | All five Playwright cases pass in the direct WebRTC configuration | A5's new spec |
| 3 | `GET /` and every asset request completes without instantiating the component | **Already closed by A4** (`test_miniapp_demo1_wasm_static_asset_serving_zero_instantiations`). Playwright cannot read the counter; A5 does **not** re-prove it |
| 4 | Repeat asset load returns 304 from the `ETag` | A4 (Rust) + A5 observes it indirectly; no browser assertion required (see D-A5-8) |
| 5 | A POST the guest rejects returns the guest's own status and message | A4 (Rust) **and A5 in the browser** — the comment case submits an empty comment and asserts both the rendered failure and, via `page.evaluate`, the 422 and the guest's own message (§4.7) |
| 6 | Multi-chunk upload/download round-trips byte-identical | A5's file case, with a multi-chunk payload (D-A5-6) |
| 7 | An SSE subscriber receives an update published by a **different browser session** | A5's fifth case, two browser contexts |
| 8 | A WebSocket client echoes a frame off the guest, **and** a frame published by a different browser session is broadcast to it | A5's WebSocket case, two browser contexts |
| 9 | Every failure-matrix row has a test | Already covered by A1/A2/A4 Rust tests — **audited row by row in §1.1**, not asserted. A5 adds no new matrix test |
| 10 | fmt / clippy / `cargo test --workspace` / `mise run test:e2e` clean | A5's verification pass |

### §1.1 Failure-matrix audit (exit criterion 9)

A5 is the milestone's last slice, so criterion 9 closes here. Each row of
task.md's failure and security matrix, against the test that actually covers
it (verified by reading the tests, not by trusting earlier slices' claims).
This table goes into `status.md` as A5's evidence for criterion 9.

| Row | Case | Covering test |
|---|---|---|
| 1 | `../` or absolute path in the bundle | `control_plane::assets::tests::traversal_entry_is_rejected_with_nothing_written` |
| 2 | Bundle over a size cap | `over_compressed_bundle_cap_is_rejected_before_any_read`, `over_unpacked_cap_aborts_mid_read_and_stops_writing`, `over_file_count_cap_is_rejected`, `oversized_non_file_entry_is_still_bounded_by_the_unpacked_cap` (all `assets.rs`); the authoritative combined-frame check by `sdk::frame_size_tests`'s three unit tests (`lib.rs:1149-1173`) — `a_request_within_the_frame_limit_is_accepted`, `a_request_right_at_the_limit_is_accepted`, `a_request_one_byte_over_the_limit_is_refused_naming_the_method_and_both_sizes` |
| 3 | Path absent from the manifest → 405, no blob lookup, no instantiation | `static_assets_e2e::test_static_asset_serving_index_etag_and_directory_rewrite` (the miss assertion at `static_assets_e2e.rs:259-264`, plus the zero-instantiation delta in the same test) |
| 4 | One service asking for another's assets | `static_assets_e2e::test_static_asset_cross_service_isolation` |
| 5 | Guest handler traps or exceeds its epoch bound | `guest_http_e2e::test_trap_and_spin_return_500_and_a_new_stream_still_succeeds`. **Known open half:** a guest blocked *inside a host call* is not interrupted by the epoch deadline — task.md's own row 5 scopes this out ("deliberately untested rather than tested badly") and it already has a `deferred-backlog.md` row. A5 does not close it |
| 6 | Malformed or oversized guest response | `guest_http_e2e::test_huge_and_bad_header_return_500_with_no_partial_body`, plus `build_guest_response`'s seven unit tests in `route_handler/http.rs` |
| 7 | A service that never declared public assets is asked for one | `static_assets_e2e::test_static_asset_private_visibility_matches_no_bundle` |
| 8 | Many concurrent subscribers bounded per service | SSE: `http_passthrough_e2e::test_sse_permit_exhaustion_returns_503_service_unavailable`; guest HTTP: `guest_http_e2e::test_guest_http_concurrency_limit_returns_503_with_retry_after` and `test_guest_http_requests_within_budget_all_succeed_via_queuing`; WebSocket: `websocket_e2e::test_websocket_concurrency_limit_returns_503_with_retry_after` |

Two notes on scope, so the audit is not read as wider than it is. The matrix
has eight rows and none of them is WebSocket-specific; A4 plan §9.2's list of
"WebSocket failure rows" (frame caps, a trap inside `on-message`, half-open
connections) is a **suggestion of rows nobody wrote**, not matrix rows left
unmet. A5 does not invent them here, but files them as one backlog row (§7)
rather than leaving the observation buried in a completed slice's plan.

**Three of the browser-path pieces do not work today.** They are found in §2
and fixed in §4; none of them is visible from the Rust e2e suite, which is
exactly why this slice exists.

---

## §2 Findings from reading the tree

### F-A5-1 (blocker) — the browser's WebSocket shim dials `raw://`, which can never reach A3's upgrade bridge on a WASM service

[`peer-proxy.js:511`](../../../../crates/coordinator_webrtc/templates/peer-proxy.js#L511)
builds the `SynWebSocket` preamble as:

```js
const preamble = `raw://${TARGET_INTERFACE}|${serviceId}?enc=ecdh-p256&pubkey=placeholder\n`;
```

`raw://` selects `RouteProtocol::Raw`, and `RouteHandler::plan_pipeline`
([dispatch.rs:320-350](../../../../crates/router/src/route_handler/dispatch.rs#L320))
has **no arm for `(Raw, NativeHostChannel)`** — which is the endpoint this
plan's alias resolves to (F-A5-5). It falls to `_ => ServiceStage::Unsupported`
(dispatch.rs:350), and `handle_raw_stream`'s catch-all answers
`"ServiceStage Unsupported is not supported for Raw transport"`
([io.rs:487-490](../../../../crates/router/src/route_handler/io.rs#L487)). The
connection dies before a single HTTP byte is parsed.

The failure wears a second face worth knowing, because it is the one an
implementer hits if the alias is built differently: with an **app-declared**
interface the endpoint is `WasmChannel`, `(Raw, WasmChannel)` *does* have an
arm (`ServiceStage::WasmComponent`), and the request lands in
`handle_stream_protocol_request`
([io.rs:502-516](../../../../crates/router/src/route_handler/io.rs#L502)),
which rejects it for a missing `dir=upload|download` instead. Same cause, two
different error strings — neither is an HTTP upgrade.

It works today only because the native fixture is a `TcpHostPort` endpoint:
`plan_pipeline` forces `TransportStage::Raw` + `TcpProxy` for those and copies
bytes to the axum process, which does its own upgrade.

**Fix (§4.2):** the shim uses the `http://` scheme, like the fetch path
already does ([peer-proxy.js:865](../../../../crates/coordinator_webrtc/templates/peer-proxy.js#L865)).
For a `WasmChannel`/`NativeHostChannel` endpoint that selects
`TransportStage::Http` → hyper → A3's `handle_websocket_route`. For a
`TcpHostPort` endpoint the `(_, TcpHostPort)` arm still forces raw
passthrough, so **the native fixture's behaviour is unchanged** — the same
bytes reach axum either way.

### F-A5-2 (blocker) — the browser proxy never decodes `Transfer-Encoding: chunked`

`handleSWRequest`
([peer-proxy.js:874-909](../../../../crates/coordinator_webrtc/templates/peer-proxy.js#L874))
parses the response head, then forwards every following byte verbatim into the
`TransformStream` it hands the service worker. Nothing de-chunks.

Which of the demo app's responses are chunked:

| Response | Framing | Why |
|---|---|---|
| Static assets (A1) | `Content-Length` | `try_handle_asset` sets it explicitly ([http.rs:821](../../../../crates/router/src/route_handler/http.rs#L821)) |
| Guest routes (A2) | `Content-Length` | host-computed, always |
| `GET /api/files/{name}` (A4 `accept-download`) | **chunked** | `StreamBody`, no length ([http.rs:1417-1427](../../../../crates/router/src/route_handler/http.rs#L1417)) |
| `GET /api/events` (SSE) | **chunked** | `StreamBody` ([http.rs:1227](../../../../crates/router/src/route_handler/http.rs#L1227)) |

A4's own Rust harness proves the framing: `miniapp_demo1_wasm_e2e.rs` carries a
`decode_chunked_body` helper and uses it on exactly these two paths. In the
browser, a download currently yields the chunk-size lines mixed into the file
bytes, and the SSE stream carries hex length lines between frames.

**Fix (§4.3):** an incremental chunked decoder in `handleSWRequest`, plus
stripping `transfer-encoding`/`content-length` from the synthetic `Response`
headers.

### F-A5-3 (blocker) — SSE frames name the event with the internal namespaced topic, so `EventSource.onmessage` never fires

`handle_messaging_sse` writes `format_sse_frame(&topic, &payload)` with the
topic the broker delivered
([http.rs:1218-1225](../../../../crates/router/src/route_handler/http.rs#L1218)),
and that topic is the namespaced one:
`namespace_topic(service_id, topic)` → `svc/<service-id>/comment-updates`
([mqtt_broker/src/lib.rs:70](../../../../crates/mqtt_broker/src/lib.rs#L70)).
`format_sse_frame` emits it as an `event:` line
([http.rs:391-407](../../../../crates/router/src/route_handler/http.rs#L391)).

An SSE event with a name is dispatched under that name; `onmessage` only
receives unnamed (`message`) events. The demo client uses `onmessage`
([SseLiveUpdates.tsx:16](../../../../test-components/miniapp-demo1-wasm/client/src/SseLiveUpdates.tsx#L16)),
so **it would never update**, and no client can practically subscribe by name
because the name embeds a DID chosen at deploy time.

A4's Rust SSE tests missed it because both assert on the payload substring
only (`miniapp_demo1_wasm_e2e.rs:538`, `http_passthrough_e2e.rs:616`).

**Fix (§4.1):** the `event:` name becomes the **service-relative** topic
(`comment-updates`) — the substrate's `svc/<did>/` namespace is an internal
detail and does not belong on the wire to a browser.

### F-A5-4 — a WASM deploy through `roymctl` needs a KEK injected first

Asset unpacking, the data-layer store, and the blob store all wrap their DEK
with the substrate-global KEK; `KeyStore` returns "Encryption KEK is required
but has not been injected" until one is present
([data_keystore/src/lib.rs:12](../../../../crates/data_keystore/src/lib.rs#L12)).
The Rust harness does `ctx.substrate_client.inject_kek(...)` in every A4 test.
`global-setup.ts` never injects one — it has not needed to, because the
native fixture keeps its own SQLite file outside the substrate.

`roymctl kek inject <hex>` exists
([commands.rs:98-102](../../../../apps/roymctl/src/commands.rs#L98),
[commands/security.rs:34](../../../../apps/roymctl/src/commands/security.rs#L34))
and requires a substrate-admin caller, i.e. `--as owner`.

### F-A5-5 — the bootstrap alias for the WASM app must name the `http-native` interface

The coordinator interpolates the hostname's `-i<hash>` label verbatim into
`TARGET_INTERFACE` ([bootstrap.rs:187-264](../../../../crates/coordinator_webrtc/src/bootstrap.rs#L187)),
and the router canonicalizes it via `EndpointRegistry::resolve_interface`
([local_registry.rs:209](../../../../crates/core/src/local_registry.rs#L209)).

- `-i` **omitted** → "the service's one app-declared interface". The WASM app
  declares four (§4.5), so `resolve_interface` returns `None` → local miss →
  the router falls into community-registry/DHT relay resolution. Broken.
- `-i` = hash of an **app-declared** interface → `WasmChannel` →
  `plan_pipeline` yields `ServiceStage::WasmComponent` with
  `AdaptationStage::JsonRpcToWasm`. Guest/WebSocket/stream routes would still
  work (they call the engine directly), but **static assets would break**:
  `try_handle_asset` reaches blob bytes through `dispatch_native(...
  "blob-store", "open-download")`, which goes through `dispatch_json_rpc_once`
  with that pipeline and would try to call the guest
  ([http.rs:841-866](../../../../crates/router/src/route_handler/http.rs#L841)).
- `-i` = hash of **`http-native`** → `NativeHostChannel` → `NativeService`
  pipeline, which is exactly what `open_http_stream` uses in every A2/A3/A4
  Rust test (`miniapp_demo1_wasm_e2e.rs:191`). Correct.

`http-native` is registered for every deployed service regardless of type
([orchestration.rs:2022](../../../../crates/control_plane/src/service/orchestration.rs#L2022),
`NATIVE_CAPABILITY_INTERFACES`), so it is always resolvable.

### F-A5-6 — the echo half of exit criterion 8 is unreachable in under 10 seconds

`LastUpdated.tsx` only sends `"Hi from client"` on a 10-second interval, with
no send on open
([LastUpdated.tsx:19-25](../../../../test-components/miniapp-demo1-wasm/client/src/LastUpdated.tsx#L19)).
The guest answers with `{"recdMsg":"Received: <msg>"}` (unicast, proven in
`miniapp_demo1_wasm_e2e.rs:586`). A browser assertion on the echo therefore
either waits 10 s or never lands within a normal timeout.

### F-A5-7 — no asset archive artifact exists, and build order matters

Nothing in the tree produces a `.tar.gz` for `miniapp-demo1-wasm/static/`;
A4's Rust tests build a tar in memory. `global-setup.ts` must pack one.

Order is load-bearing: `src/lib.rs` embeds the SPA shell with
`include_str!("../static/dist/index.html")`
([lib.rs:65](../../../../test-components/miniapp-demo1-wasm/src/lib.rs#L65)),
and that file names the hashed bundle (`/dist/assets/index-<hash>.js`). So:
**client `npm run build` → `cargo component build` → `tar`**. This is the same
order `mise run build:test-components` uses
([mise.toml:30-39](../../../../mise.toml#L30)).

Archive entry paths: `reject_archive_entry_path` accepts `./x`
([deploy_docs.rs:162](../../../../crates/core/src/deploy_docs.rs#L162)) and
`normalize_asset_path` rewrites entries to `/index.html`,
`/dist/assets/index-<hash>.js`
([assets.rs:258](../../../../crates/control_plane/src/assets.rs#L258)), which
is the shape `resolve_asset` looks up. So `tar -czf … -C static .` is safe.

Sizes are comfortably inside every cap: component 268 KB, JS bundle 16.6 KB,
`MAX_ASSET_BUNDLE_BYTES` 2 MiB, RPC frame 16 MiB.

### F-A5-8 — the SPA's own selectors already match the native fixture

`static/index.html` carries `<h1>Hello world from demo1-instance0</h1>` and
the `Comments etc.` link; `App.tsx`, `RecentComments.tsx` and `FilesManager.tsx`
differ from the native client only in the upload call shape and the added SSE
panel (verified by diff). No ported selector changes at all (D-A5-7); the new
assertions add hooks rather than replacing anything. The
`<ul>` order the native spec relies on (`.first()` = comments, `.last()` =
files) still holds.

### F-A5-9 — `/comments` navigation and asset loads already work through the proxy

The proxy hijacks same-origin clicks and re-fetches through the service worker
([peer-proxy.js:648-675](../../../../crates/coordinator_webrtc/templates/peer-proxy.js#L648),
`reloadContent` at 975). `GET /comments` is an A2 guest route returning the SPA
shell with `Content-Length`, and the module script it names is an A1 asset,
also with `Content-Length` — neither is affected by F-A5-2.

### F-A5-10 — the default 30 s per-test timeout cannot hold a two-session case

[`playwright.config.ts`](../../../../crates/substrate/tests/e2e/playwright.config.ts)
sets no `timeout`, so every test in the WebRTC suite runs on Playwright's 30 s
default, and `beforeEach` spends part of that budget before the body starts.
The native cases fit because their generous `{ timeout: 35000 }` waits are
ceilings that resolve in a second or two, not durations. A5's two-session
cases genuinely cost more: a second `browser.newContext()` performs a full
WebRTC bootstrap, service-worker registration, and SPA navigation inside the
same test budget. `playwright-multihop.config.ts:7` already sets
`timeout: 60000` for strictly less work.

### F-A5-11 — exit criterion 5's browser half is cheap and was being skipped

`App.tsx` renders `Error saving comment.` on any non-ok response
([App.tsx:29-31](../../../../test-components/miniapp-demo1-wasm/client/src/App.tsx#L29)),
and the guest answers an empty comment with 422 +
`{"error":"invalid payload: text is required"}`
([lib.rs:198-204](../../../../test-components/miniapp-demo1-wasm/src/lib.rs#L198)).
Criterion 5 is about the guest's own status *and message* reaching the client,
which the rendered string alone does not show — but one `page.evaluate` POST
reads both off the wire. Nothing about this needs new fixture code.

---

## §3 Decisions

| # | Decision | Rationale |
|---|---|---|
| **D-A5-1** | A **new spec file**, `tests/wasm-app.spec.ts`. `webrtc.spec.ts` is left untouched and keeps running against the native fixture. | task.md's non-goals: "Retiring `miniapp-demo1-web`. It stays… a divergence between them is a finding." Re-pointing the existing file would delete the native half of that comparison. |
| **D-A5-2** | Both fixtures are deployed by the **same** `global-setup.ts`, on the same substrate, under separate identities/nicknames (`demo1`, `demo1wasm`). | One substrate, one setup cost. Nothing in the two services shares state. |
| **D-A5-3** | The new spec is parameterized over `forceTunnel ∈ {false, true}`, matching `webrtc.spec.ts`. | Criterion 2 only demands direct WebRTC, but the blind-tunnel path is where the ECDH `Decryptor` handles a *streaming* body for the first time (SSE, download). Cheap coverage of a genuinely different code path. |
| **D-A5-4** | The SSE `event:` name becomes the **service-relative** topic. | F-A5-3. The alternative (drop the `event:` line) throws away per-message topic information that a wildcard subscription would need. |
| **D-A5-5** | Chunked decoding lives in `peer-proxy.js`, not in the service worker. | `sw.js` receives an already-decoded stream and stays a dumb pipe; the proxy page is where the HTTP/1.1 wire format is already parsed (`parseHeaders`). |
| **D-A5-6** | The browser upload case uses a **256 KiB** deterministic payload, and asserts a byte-length + checksum round trip computed in-page. | Criterion 6 ("multiple chunks"): a `File` body streams in ~64 KiB reads, so 256 KiB is several data-channel messages and several `read-chunk` hops. Comparing text in the test process would be slow and noisy. |
| **D-A5-7** | Every ported assertion keeps `webrtc.spec.ts`'s selectors **verbatim**; `data-testid` hooks appear only in assertions with no native counterpart. In practice: `GET /` + navigate and file upload/download are untouched, the comment case only *adds* criterion 5's rejection assertions, and `WebSocket Echo and Broadcast` plus the new SSE case use testids for the echo, timestamp and status values. | The unmodified cases must fail for the same reasons the native ones fail; that is the point of keeping both fixtures. The WebSocket case cannot stay verbatim and still meet exit criterion 8, which asks for the echo *and* a cross-session broadcast that the native case never asserts (§8.2). |
| **D-A5-8** | A5 asserts **no** ETag/304 and **no** instantiation count in the browser. | Criteria 3 and 4 are already closed by A4's Rust tests with direct access to the counter and the header; a browser test would restate them weakly (Chrome's own cache makes a 304 assertion flaky). |
| **D-A5-9** | The multi-hop suite (`multi-hop.spec.ts`) is **not** extended to the WASM app. | task.md marks it "Stretch, not required". Recorded as a backlog row instead. |
| **D-A5-10** | The demo client sends one `"Hi from client"` frame immediately on open, keeping the 10 s interval. | F-A5-6. Fixture-only change; makes the echo assertion fast and deterministic. |
| **D-A5-11** | The new spec raises its own per-test timeout to **90 s** with `test.describe.configure`; `playwright.config.ts` keeps no global `timeout`, so `webrtc.spec.ts` stays on Playwright's 30 s default. | F-A5-10. A two-session case pays for a second full WebRTC bootstrap inside one test's budget, which 30 s cannot hold. Raising it config-wide (as `playwright-multihop.config.ts:7` does) would also loosen the existing native suite, whose current budget is proven — a hang there should keep failing in 30 s, not 90. |

---

## §4 Changes, file by file

### 4.1 `crates/router/src/route_handler/http.rs` — SSE event name (F-A5-3)

**Add** a free function beside `format_sse_frame` (which is unchanged):

```rust
/// The `svc/<service-id>/` prefix `namespace_topic` adds is a substrate
/// implementation detail. An SSE subscriber names topics the way the route
/// table does, so the wire carries the service-relative name -- a browser
/// cannot subscribe by a name that embeds the deployment's own DID.
/// A topic that is not in this service's namespace (a cross-service
/// subscription) is passed through whole.
fn service_relative_topic<'a>(service_id: &str, topic: &'a str) -> &'a str {
    topic
        .strip_prefix("svc/")
        .and_then(|rest| rest.strip_prefix(service_id))
        .and_then(|rest| rest.strip_prefix('/'))
        .unwrap_or(topic)
}
```

**Change** `handle_messaging_sse` ([http.rs:1156-1234](../../../../crates/router/src/route_handler/http.rs#L1156)):

- line 1173 currently does `let service_id = self.preamble.service_id.clone();`
  and line 1186 **moves** it into the `sse_permits` entry lookup. Clone twice:
  keep `service_id` for the permit map and add
  `let sse_service_id = self.preamble.service_id.clone();` for the stream state.
- carry it in the unfold state and use it for the frame name:

```rust
let stream = stream::unfold(
    (receiver, handle, permit, sse_service_id),
    |(mut receiver, handle, permit, sid)| async move {
        let (topic, payload) = receiver.recv().await?;
        let name = service_relative_topic(&sid, &topic);
        let frame = Frame::data(Bytes::from(format_sse_frame(name, &payload)));
        Some((Ok::<_, Infallible>(frame), (receiver, handle, permit, sid)))
    },
);
```

**New unit tests** (same `mod tests`):

- `service_relative_topic_strips_this_services_namespace` —
  `("svc-a", "svc/svc-a/comment-updates") == "comment-updates"`.
- `service_relative_topic_leaves_a_foreign_or_unprefixed_topic_whole` —
  `("svc-a", "svc/svc-b/x") == "svc/svc-b/x"`, `("svc-a", "plain") == "plain"`.

**No other caller.** `format_sse_frame` has exactly one call site; its two
existing unit tests call the function directly and are unaffected.

### 4.2 `crates/coordinator_webrtc/templates/peer-proxy.js` — WebSocket preamble scheme (F-A5-1)

Line 511, inside `SynWebSocket`'s constructor:

```js
-        const preamble = `raw://${TARGET_INTERFACE}|${serviceId}?enc=ecdh-p256&pubkey=placeholder\n`;
+        // `http://`, not `raw://`: the substrate answers an inbound
+        // WebSocket upgrade inside its HTTP bridge, and no `raw://`
+        // pipeline reaches it. Against the native-capability endpoint a
+        // sandboxed app is served through, `raw://` has no pipeline arm
+        // at all and is refused as an unsupported service stage; against
+        // an app-declared interface it lands in the ADR-0014
+        // stream-protocol path, which demands a `dir=` parameter a
+        // WebSocket has no answer for. A TCP passthrough service is
+        // unaffected either way -- its endpoint forces raw byte copying
+        // regardless of the scheme.
+        const preamble = `http://${TARGET_INTERFACE}|${serviceId}?enc=ecdh-p256&pubkey=placeholder\n`;
```

Nothing else changes: `connectTunnel`'s `isRaw` argument is already unused, the
data-channel path still strips the query (`preamble.split('?')[0]`), and the
upgrade request the shim writes after connecting (lines 585-594) is exactly
what `validate_websocket_upgrade_headers` accepts
([http.rs:1576-1596](../../../../crates/router/src/route_handler/http.rs#L1576)):
`Upgrade: websocket`, `Connection: Upgrade`, a non-empty `Sec-WebSocket-Key`,
`Sec-WebSocket-Version: 13`.

### 4.3 `crates/coordinator_webrtc/templates/peer-proxy.js` — chunked response decoding (F-A5-2)

**Add** near `WSFrameDecoder` (same file, no new files — the template is
served as one asset):

```js
// HTTP/1.1 chunked-transfer decoder. The substrate answers every streaming
// body (SSE, stream-protocol download) with `Transfer-Encoding: chunked`,
// since neither has a length to declare up front. Without this the chunk
// size lines land in the body the page sees.
class ChunkedDecoder {
    constructor(onData, onEnd) {
        this.onData = onData; this.onEnd = onEnd;
        this.buf = new Uint8Array(0);
        this.state = 'size';   // 'size' | 'data' | 'crlf' | 'trailer' | 'done'
        this.remaining = 0;
    }
    addBytes(bytes) {
        // append to this.buf (same growth pattern as WSFrameDecoder)
        for (;;) {
            if (this.state === 'done') return;
            if (this.state === 'size') {
                const i = indexOfCRLF(this.buf);            // -1 => need more
                if (i === -1) return;
                const line = decodeAscii(this.buf.slice(0, i));
                this.buf = this.buf.slice(i + 2);
                const size = parseInt(line.split(';')[0].trim(), 16);
                if (!Number.isFinite(size)) { this.state = 'done'; this.onEnd(); return; }
                if (size === 0) { this.state = 'trailer'; continue; }
                this.remaining = size; this.state = 'data';
            } else if (this.state === 'data') {
                if (this.buf.length === 0) return;
                const n = Math.min(this.remaining, this.buf.length);
                this.onData(this.buf.slice(0, n));
                this.buf = this.buf.slice(n);
                this.remaining -= n;
                if (this.remaining === 0) this.state = 'crlf';
            } else if (this.state === 'crlf') {
                if (this.buf.length < 2) return;
                this.buf = this.buf.slice(2);               // discard chunk CRLF
                this.state = 'size';
            } else if (this.state === 'trailer') {
                // Trailers are not used by anything this substrate sends;
                // wait for the terminating CRLF, then finish.
                const i = indexOfCRLF(this.buf);
                if (i === -1) return;
                this.buf = this.buf.slice(i + 2);
                this.state = 'done'; this.onEnd(); return;
            }
        }
    }
}
```

`indexOfCRLF(bytes)` and `decodeAscii(bytes)` are two small helpers beside
`findDoubleCRLF` (line 1058).

**Change** `handleSWRequest`'s `onData`
([peer-proxy.js:874-909](../../../../crates/coordinator_webrtc/templates/peer-proxy.js#L874)):

```
on header completion:
    { status, headers } = parseHeaders(headerStr)
    isChunked = (headers.get('transfer-encoding') || '').toLowerCase().includes('chunked')
    if isChunked:
        chunkDecoder = new ChunkedDecoder(
            (b)  => responseWriter.write(b),
            ()   => { try { responseWriter.close() } catch (_) {} })
    headerList = [...headers] minus 'transfer-encoding' and, when chunked,
                 'content-length'          // the framing is gone by the time
                                           // the page sees the body
    port.postMessage({ type:'RESPONSE', status, headers: headerList, body: readable }, [readable])
    headersParsed = true
    if bodyBytes.length: isChunked ? chunkDecoder.addBytes(bodyBytes)
                                   : responseWriter.write(bodyBytes)
else (headersParsed):
    isChunked ? chunkDecoder.addBytes(bytes) : responseWriter.write(bytes)
```

`onClose` (line 911) is unchanged: closing an already-closed writer is
swallowed by the existing `try`.

**Why this is safe for existing traffic:** every non-streaming response the
substrate sends carries `Content-Length` (assets at
[http.rs:821](../../../../crates/router/src/route_handler/http.rs#L821), guest
responses via `build_guest_response`, JSON errors via `full_body`), so
`isChunked` is false and the path is byte-for-byte what it is today. The
native fixture is axum, which also sets a length for its fixed bodies.

### 4.4 `test-components/miniapp-demo1-wasm/client/` — fixture client (F-A5-3, F-A5-6, D-A5-7)

`src/SseLiveUpdates.tsx`:

- Replace `eventSource.onmessage = …` (line 16) with
  `eventSource.addEventListener('comment-updates', handler)`, where `handler`
  keeps the existing body. The name is the route-declared topic from
  `routes.json`, now that §4.1 stops leaking the namespaced form.
- Add `data-testid="sse-status"` to the status text and
  `data-testid="sse-last-updated"` to the timestamp `<span>` (lines 41-43).

`src/LastUpdated.tsx`:

- In `socket.onopen` (line 16), send once before starting the interval:
  ```ts
  socket.send('Hi from client');   // one immediate frame so the echo is
                                   // observable without waiting a full interval
  ```
- Add the three test hooks (lines 58-66). `{status()}` is currently a bare
  text node next to `<strong>Live Updates:</strong>`, so it needs a wrapping
  element of its own — the other two values already sit in `<span>`s:
  ```tsx
  <strong>Live Updates:</strong> <span data-testid="ws-status">{status()}</span>
  <br />
  Last comment added at: <span data-testid="ws-last-updated">{lastUpdated()}</span>
  <br />
  Received Msg: <span data-testid="ws-recd-msg">{recdMsg()}</span>
  ```
  The plain-text `Live Updates: <status>` reading the ported cases match is
  unchanged by the wrapper.

Nothing in the WASM guest changes.

### 4.5 `crates/substrate/tests/e2e/global-setup.ts` — build, deploy, expose the WASM app

The native flow is untouched; A5's steps interleave with it. **The exact
order, since three of the steps have hard dependencies on each other:**

| # | Step | Anchor in today's file | Depends on |
|---|---|---|---|
| 1 | `.e2e-data` created | line 14 (existing) | — |
| 2 | WASM client `npm run build` | beside the existing client build, line 24-26 | 1 (nothing; writes into the source tree) |
| 3 | `cargo component build --release` | after step 2, before the cargo binary builds at line 29-31 | **2** — `src/lib.rs` `include_str!`s `static/dist/index.html`, which step 2 regenerates with a fresh hashed bundle name |
| 4 | `tar -czf …/miniapp-demo1-wasm-assets.tar.gz` | after step 3 | **2** (packs `static/`, including the freshly built `dist/`) and 1 (writes into `.e2e-data`) |
| 5 | substrate spawned, DID extracted | lines 92-124 (existing) | — |
| 6 | 4 s readiness wait | line 136-137 (existing) | 5 |
| 7 | **`roymctl kek inject`** | immediately after step 6, before any deploy | **6** — `kek inject` goes through the same `client_for` + `wait_for_ready(5s)` path `svc deploy` does and resolves the substrate through the registry; placing it at step 5 races startup |
| 8 | native identity / registry / deploy | lines 139-168 (existing) | 6 |
| 9 | WASM identity / registry / alias / deploy | after step 8 | **3, 4, 7** |
| 10 | `process.env.WASM_APP_*` exported | after step 9 | 9 |

Constants at the top of the file:

```ts
const WASM_FIXTURE_DIR = path.join(WORKSPACE_DIR, 'test-components/miniapp-demo1-wasm');
const WASM_ARTIFACT = path.join(
  WASM_FIXTURE_DIR, 'target/wasm32-wasip2/release/syneroym_test_miniapp_demo1_wasm.wasm');
```

**Steps 2-3 — build** (beside the existing client build at line 24-26, so both
happen before any cargo work):

```ts
console.log('Building miniapp-demo1-wasm client + component...');
// Order matters: src/lib.rs include_str!s static/dist/index.html, which the
// client build regenerates with a fresh hashed bundle name.
execSync('npm ci || npm install', { cwd: path.join(WASM_FIXTURE_DIR, 'client'), stdio: 'inherit' });
execSync('npm run build', { cwd: path.join(WASM_FIXTURE_DIR, 'client'), stdio: 'inherit' });
// Always --release: that is the path crates/core/src/test_constants.rs names,
// and the fixture is excluded from the workspace build graph.
execSync('cargo component build --release --target wasm32-wasip2',
         { cwd: WASM_FIXTURE_DIR, stdio: 'inherit' });
```

**Step 4 — pack the asset bundle** (right after the component build; `TEST_DIR`
already exists by line 14):

```ts
const assetsArchive = path.join(TEST_DIR, 'miniapp-demo1-wasm-assets.tar.gz');
// COPYFILE_DISABLE stops macOS tar from adding ._ AppleDouble entries, which
// would land in the manifest as junk paths.
execSync(`COPYFILE_DISABLE=1 tar -czf "${assetsArchive}" -C "${path.join(WASM_FIXTURE_DIR, 'static')}" .`,
         { cwd: WORKSPACE_DIR, stdio: 'inherit' });
```

**Step 7 — inject the KEK** (F-A5-4). This goes **after** the existing 4 s
readiness wait at line 136-137, not after the DID line at 124: at 124 the
substrate has only printed its identity, while `kek inject` dials it through
the registry with a 5 s `wait_for_ready`, so the earlier placement races
startup for no gain. Put it immediately before the native deploy block:

```ts
console.log('Injecting substrate KEK...');
execSync(`"${ROYMCTL_BIN}" --dir ${TEST_DIR} --api-url http://127.0.0.1:7661 ` +
         `--substrate ${substrateDid} --as owner kek inject ${'21'.repeat(32)}`,
         { cwd: WORKSPACE_DIR, stdio: 'inherit' });
```

**Steps 9-10 — identity, registry, alias, deploy** (mirrors the native block
exactly, with `--interface http-native` per F-A5-5):

```ts
execSync(`"${ROYMCTL_BIN}" --dir ${TEST_DIR} identity create --name demo1wasm`, …);
const wasmDid = /(did:key:[a-z0-9]+)/.exec(
  execSync(`"${ROYMCTL_BIN}" --dir ${TEST_DIR} identity show --name demo1wasm`).toString())[1];

execSync(`"${ROYMCTL_BIN}" --dir ${TEST_DIR} --api-url http://127.0.0.1:7661 ` +
         `registry register --identity demo1wasm --substrate ${substrateDid} --nickname demo1wasm`, …);

// `--interface http-native`: the HTTP bridge (assets, guest routes, SSE,
// WebSocket) is reached through the reserved native-capability interface, the
// same one crates/substrate/tests/*_e2e.rs put in their preamble. An
// app-declared interface would resolve to a WasmChannel pipeline and break
// static-asset serving, which reaches blob bytes through native dispatch.
const wasmAlias = execSync(
  `"${ROYMCTL_BIN}" alias ${wasmDid} --nickname demo1wasm --interface http-native`,
  { cwd: WORKSPACE_DIR }).toString().trim().split('\n').pop()!.trim();

const WASM_IFACES = [
  'syneroym:http/incoming-handler@0.1.0',
  'syneroym:http/websocket-handler@0.1.0',
  'syneroym:messaging/guest-api@0.1.0',
  'syneroym:messaging/stream-types@0.1.0',
].join(',');

execSync(`"${ROYMCTL_BIN}" --dir ${TEST_DIR} --api-url http://127.0.0.1:7661 ` +
  `--substrate ${substrateDid} --as owner svc deploy --svc-id ${wasmDid} ` +
  `--interfaces ${WASM_IFACES} --wasm "${WASM_ARTIFACT}" ` +
  `--assets "${assetsArchive}" --asset-visibility public ` +
  `--custom-config "${path.join(WASM_FIXTURE_DIR, 'routes.json')}"`,
  { cwd: WORKSPACE_DIR, stdio: 'inherit' });

process.env.WASM_APP_DID = wasmDid;
process.env.WASM_APP_ALIAS = wasmAlias;
```

The interface list matches `miniapp_manifest_with_assets` in
`miniapp_demo1_wasm_e2e.rs:50-55` — the shape already proven to deploy.
`routes.json` is passed unchanged; it is the same file
`test_constants::MINIAPP_DEMO1_WASM_ROUTES_JSON` embeds.

`global-teardown.ts` needs **no change** (no new process; `.e2e-data` removal
already covers the archive).

### 4.6 `crates/substrate/tests/e2e/playwright.config.ts`

```ts
-  testMatch: '**/webrtc.spec.ts',
+  testMatch: ['**/webrtc.spec.ts', '**/wasm-app.spec.ts'],
```

`playwright-multihop.config.ts` is unchanged (D-A5-9).

### 4.7 `crates/substrate/tests/e2e/tests/wasm-app.spec.ts` (new)

Shape. All four ported cases are copied from `webrtc.spec.ts` with no change
but the alias env var; two are then extended — the comment case with
criterion 5's rejection half, the WebSocket case per D-A5-7. The fifth (SSE)
is new:

```ts
import { test, expect, Browser, Page } from '@playwright/test';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

const APP_URL = (forceTunnel: boolean) =>
  `http://${process.env.WASM_APP_ALIAS}:7662/?force_tunnel=${forceTunnel}`;

// Opens a second, independent browser session on the same app and parks it on
// the comments page -- exit criteria 7 and 8 both say "a different browser
// session", which one page cannot demonstrate.
async function openSecondSession(browser: Browser, forceTunnel: boolean) {
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.goto(APP_URL(forceTunnel));
  // Same settle the primary session's beforeEach does (webrtc.spec.ts:23):
  // the service worker has to take control before the first proxied fetch.
  await page.waitForLoadState('networkidle');
  await expect(page.locator('h1')).toContainText('Hello world from demo1-instance0', { timeout: 30000 });
  await page.click('text=Comments etc.');
  await expect(page.locator('h2')).toContainText('Comments', { timeout: 35000 });
  return { context, page };
}

async function postComment(page: Page, text: string) {
  await page.fill('textarea[placeholder="Write a comment..."]', text);
  await page.click('button:has-text("Submit")');
  await expect(page.locator('text=Comment saved!')).toBeVisible({ timeout: 35000 });
}

[false, true].forEach(forceTunnel => {
  test.describe(`WASM SynApp E2E (forceTunnel=${forceTunnel})`, () => {
    // D-A5-11: a two-session case pays for a second full WebRTC bootstrap
    // inside one test's budget, which the 30 s default cannot hold. Scoped
    // here rather than in playwright.config.ts so the native suite keeps its
    // proven, tighter budget.
    test.describe.configure({ timeout: 90_000 });

    test.beforeEach(async ({ page }) => { /* identical to webrtc.spec.ts:7-28, APP_URL */ });

    test('GET / and navigate to comments', /* verbatim from webrtc.spec.ts:30-33 */);

    test('POST /api/comments and verify recent comments', async ({ page }) => {
      /* verbatim from webrtc.spec.ts:35-47 -- click through, submit, assert
         "Comment saved!" and the text in the first <ul> -- then, for exit
         criterion 5, the rejection half: */

      // The UI half: App.tsx renders this on any non-ok response.
      await page.fill('textarea[placeholder="Write a comment..."]', '');
      await page.click('button:has-text("Submit")');
      await expect(page.locator('text=Error saving comment.')).toBeVisible({ timeout: 35000 });

      // The wire half: the guest's own status *and* message, which the
      // rendered string alone does not show.
      const rejected = await page.evaluate(async () => {
        const res = await fetch('/api/comments', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ text: '   ' }),
        });
        return { status: res.status, body: await res.text() };
      });
      expect(rejected.status).toBe(422);
      expect(rejected.body).toContain('text is required');
    });

    test('WebSocket Echo and Broadcast', async ({ page, browser }) => {
      await page.click('text=Comments etc.');
      await expect(page.locator('h2')).toContainText('Comments', { timeout: 35000 });
      await expect(page.locator('text=Live Updates: Connected')).toBeVisible({ timeout: 15000 });

      // Echo (unicast, guest -> this connection only): the client sends one
      // frame on open and the guest answers {"recdMsg":"Received: ..."}.
      await expect(page.getByTestId('ws-recd-msg'))
        .toHaveText('Received: Hi from client', { timeout: 20000 });

      // Broadcast from a different session.
      const before = await page.getByTestId('ws-last-updated').innerText();
      const other = await openSecondSession(browser, forceTunnel);
      try {
        await postComment(other.page, `WS broadcast ${Date.now()}`);
        await expect(page.getByTestId('ws-last-updated')).not.toHaveText(before, { timeout: 35000 });
      } finally { await other.context.close(); }
    });

    test('File Upload and Download', async ({ page }) => {
      await page.click('text=Comments etc.');
      await expect(page.locator('h2')).toContainText('Comments', { timeout: 35000 });

      // 256 KiB: several data-channel messages on the way up and several
      // `read-chunk` hops on the way down (exit criterion 6).
      const fileName = `wasm-upload-${Date.now()}.bin`;
      const filePath = path.join(os.tmpdir(), fileName);          // never inside tests/
      const bytes = Buffer.alloc(256 * 1024);
      for (let i = 0; i < bytes.length; i++) bytes[i] = i % 251;
      fs.writeFileSync(filePath, bytes);
      try {
        await page.locator('input[type="file"]').setInputFiles(filePath);
        await page.click('button:has-text("Upload")');
        await expect(page.locator('text=Upload successful!')).toBeVisible({ timeout: 35000 });
        await expect(page.locator('ul').last()).toContainText(fileName, { timeout: 35000 });

        const result = await page.evaluate(async (name) => {
          const res = await fetch(`/api/files/${encodeURIComponent(name)}`);
          const buf = new Uint8Array(await res.arrayBuffer());
          let sum = 0; for (const b of buf) sum = (sum + b) % 4294967296;
          return { status: res.status, length: buf.length, sum };
        }, fileName);

        let expectedSum = 0; for (const b of bytes) expectedSum = (expectedSum + b) % 4294967296;
        expect(result.status).toBe(200);
        expect(result.length).toBe(bytes.length);   // fails today: chunk framing inflates it
        expect(result.sum).toBe(expectedSum);
      } finally { if (fs.existsSync(filePath)) fs.unlinkSync(filePath); }
    });

    test('SSE receives an update published by another session', async ({ page, browser }) => {
      await page.click('text=Comments etc.');
      await expect(page.locator('h2')).toContainText('Comments', { timeout: 35000 });
      await expect(page.getByTestId('sse-status')).toHaveText('Connected', { timeout: 15000 });
      await expect(page.getByTestId('sse-last-updated')).toHaveText('Never');

      const other = await openSecondSession(browser, forceTunnel);
      try {
        await postComment(other.page, `SSE update ${Date.now()}`);
        await expect(page.getByTestId('sse-last-updated')).not.toHaveText('Never', { timeout: 35000 });
      } finally { await other.context.close(); }
    });
  });
});
```

Note on `getByTestId('sse-status')`: the panel renders
`<strong>SSE Updates:</strong> {status()}`, so the `data-testid` goes on a
wrapping `<span>` around `{status()}` (§4.4), not on the outer `<div>`.

---

## §5 Complete list of touched files

| File | Change |
|---|---|
| `crates/router/src/route_handler/http.rs` | `service_relative_topic` + its use in `handle_messaging_sse`; 2 unit tests |
| `crates/coordinator_webrtc/templates/peer-proxy.js` | WS preamble scheme; `ChunkedDecoder` + 2 helpers; `handleSWRequest` header/body handling |
| `test-components/miniapp-demo1-wasm/client/src/SseLiveUpdates.tsx` | named-event listener; two `data-testid`s |
| `test-components/miniapp-demo1-wasm/client/src/LastUpdated.tsx` | send on open; three `data-testid`s |
| `crates/substrate/tests/e2e/global-setup.ts` | client+component build, asset tar, `kek inject`, second identity/registry/alias/deploy, two new env vars |
| `crates/substrate/tests/e2e/playwright.config.ts` | `testMatch` array |
| `crates/substrate/tests/e2e/tests/wasm-app.spec.ts` | **new**, 5 cases × 2 tunnel modes, own 90 s per-test timeout (D-A5-11) |
| `crates/substrate/tests/e2e/README.md` | second app in the diagram + a "Test Scenarios" subsection for the WASM suite |
| `crates/substrate/tests/miniapp_demo1_wasm_e2e.rs` | SSE test additionally asserts `event: comment-updates` (pins §4.1 from Rust) |
| `crates/substrate/tests/http_passthrough_e2e.rs` | same one-line assertion in `test_sse_receives_message_published_via_http` |
| `docs/planning/milestones/M06A-app-platform-surface/status.md` | A5 "What shipped" + evidence; overall line |
| `docs/planning/deferred-backlog.md` | rows in §7 below |

**No changes needed** to: `mise.toml` (the fixture is already in
`build:test-components`; `global-setup.ts` builds it again itself so the e2e
action stays self-contained), `.github/` (the pipeline already installs
`cargo-component` and the `wasm32-wasip2` target before the e2e action),
`global-teardown.ts`, `playwright-multihop.config.ts`, `routes.json`, the WASM
guest, `crates/core/src/test_constants.rs`.

---

## §6 Verification

Order matters — do §4.1-§4.4 and confirm the Rust suite is still clean before
touching the Playwright layer, so a browser failure is never ambiguous.

```bash
cargo +nightly fmt --all
cargo clippy --workspace --all-targets --all-features
mise run build:test-components
cargo test -p syneroym-router --lib
cargo test -p syneroym-substrate --test miniapp_demo1_wasm_e2e --test http_passthrough_e2e --test websocket_e2e
cargo test --workspace
mise run test:e2e
```

Per this repo's sandbox rules: run `cargo test --workspace` **with** the
sandbox on and re-run any flagged crate unsandboxed; run `mise run test:e2e`
**with the sandbox off** (it binds real ports and starts Chromium).

Expected e2e counts after this slice: `playwright test` → **18** (8 existing
`webrtc.spec.ts` + 10 new), `playwright test -c playwright-multihop.config.ts`
→ 4 unchanged.

Debugging aids if a browser case fails:

- `mise run test:e2e-ui`, or `npx playwright test wasm-app.spec.ts --headed`.
- The spec already forwards `page.on('console')`; `peer-proxy.js` mirrors every
  log into `window.logs`, readable with `page.evaluate(() => window.logs)`.
- Substrate stdout is prefixed `[Substrate]` in the setup output; run with
  `RUST_LOG=syneroym_router=debug` to see pipeline planning
  (`dispatch::log_pipeline`) — the fastest way to confirm F-A5-1/F-A5-5. The
  three error strings to recognise, all from `handle_raw_stream`/
  `handle_stream_protocol_request`: `"ServiceStage Unsupported is not
  supported for Raw transport"` means the WebSocket shim still sent `raw://`
  against the `http-native` endpoint (§4.2 not applied); `"missing or invalid
  \`dir\` query parameter"` means it sent `raw://` against an app-declared
  interface (§4.2 not applied *and* the alias is wrong); a local registry
  miss falling into community-registry/DHT resolution means the alias omitted
  `-i` entirely (F-A5-5).

---

## §7 Documentation and deferred-backlog

`status.md`: A5 row → Complete, an A5 "What shipped" section, and an evidence
section listing the commands above with real counts. Carry §1.1's
failure-matrix audit table across verbatim — criterion 9 closes with this
slice, so the milestone's own record needs the row-by-row mapping, not a
claim. State explicitly that exit criteria 3 and 4 are closed by A1/A4's Rust
tests rather than by Playwright (D-A5-8). While editing the file, correct the
A4 evidence line that reads "all 12 WebRTC multi-hop tests passed": the run is
8 WebRTC cases plus 4 multi-hop cases, under two separate configs (§8.5).

New `deferred-backlog.md` rows (§4, the M06A theme):

1. **The WebRTC proxy never tears down a tunnel when the page aborts a
   request.** `handleSWRequest` has no abort path, so an `EventSource.close()`
   or a cancelled fetch leaves the data channel / blind-tunnel WebSocket open
   until the page unloads. Target: TBD.
   `crates/coordinator_webrtc/templates/peer-proxy.js:854`.
2. **`ChunkedDecoder` ignores chunk extensions and trailer fields.** It parses
   the size before `;` and discards trailers wholesale. Nothing this substrate
   sends uses either. Target: TBD.
3. **The multi-hop Playwright suite still runs only against the native
   fixture** (D-A5-9, task.md's own stretch item). Target: M06C or later.
4. **The inbound WebSocket boundary has no failure-matrix rows of its own.**
   A4 plan §9.2 named four worth having — a frame-size cap, a guest trap
   inside `on-message`, a half-open connection, and an abrupt client
   disconnect mid-`on-message` — and A3 shipped tests for concurrency, the
   401 gate, teardown, echo and broadcast, but none of those four. task.md's
   matrix never listed them, so criterion 9 holds as written (§1.1); this row
   records the gap rather than leaving it in a completed slice's plan.
   Target: TBD. `crates/router/src/route_handler/http.rs:1691`.
5. **SSE reconnection (`Last-Event-ID`, `id:` fields) is not implemented.**
   `format_sse_frame` emits no `id:`, so a dropped SSE connection resumes with
   a gap. Surfaced by A5's browser path; not required by any exit criterion.
   Target: TBD.

No in-code `TODO`/`FIXME` markers are added by this slice.

---

## §8 Ambiguities and stale statements in the input documents

Flagged rather than guessed; each has a recommendation, and §3 records the one
this plan proceeds with.

1. **"Re-pointed at the demo app" (task.md, A5 row) is ambiguous** — it can be
   read as editing `webrtc.spec.ts` in place. That directly contradicts the
   non-goal "Retiring `miniapp-demo1-web`. It stays." **Resolved as D-A5-1**
   (new spec file, both fixtures keep running). If the intent was in-place
   replacement, this plan needs one change: delete `webrtc.spec.ts` instead of
   adding a file — but then task.md's own "a divergence between them is a
   finding" stops being testable.
2. **"Matching the native fixture" vs. exit criteria 7 and 8.** The native
   `WebSocket Echo and Broadcast` case is single-session and never asserts the
   echo at all; criteria 7 and 8 both demand "a different browser session".
   **Resolved as two-session tests** for the WS-broadcast and SSE cases, plus
   an explicit echo assertion (D-A5-10 makes it observable). This is a
   deliberate divergence from the native case's shape, in the direction the
   exit criteria require.
3. **A4 plan §8's "SSE WebRTC Proxy Spike … open integration point to be
   proven end-to-end by the Playwright browser suite in Slice A5" understates
   it.** It is not just unproven; it is broken in three specific ways
   (F-A5-1/2/3), two of which need router or coordinator changes, not test
   code. task.md's A5 scope line mentions no production-code work. **This
   plan adds it**; if A5 must stay test-only, the three fixes need their own
   slice and A5 cannot pass.
4. **Exit criterion 9 needed an audit, not an assertion.** The first draft of
   this plan said "already covered by A1–A4; A5 adds none". Reading the tests
   confirms it does hold, with one scoped-out half (matrix row 5's guest
   blocked inside a host call, which task.md itself excludes) — but the claim
   was unverified when written. §1.1 now carries the row-by-row mapping, and
   the WebSocket rows A4 wished for are filed as a backlog row rather than
   silently counted as covered.
5. **task.md's exit criterion 4 ("a repeat asset load returns 304") is not
   practically assertable from Playwright** — Chrome's own HTTP cache serves
   the second load without a network request, so the assertion would test the
   browser, not the substrate. A4's Rust test already asserts the 304 on the
   wire. **Resolved as D-A5-8.**
6. **`status.md`'s A4 evidence line is imprecise**: "`mise run test:e2e` #
   clean, all 12 WebRTC multi-hop tests passed". The suite is 8 WebRTC cases
   plus 4 multi-hop cases, run by two separate configs. Worth correcting while
   editing that file for A5.
7. **A4 plan §9's note on the WebRTC WebSocket shim ("The A3 101 handshake
   must answer exactly what that shim sends") checked the headers but not the
   preamble scheme.** The headers do match; the scheme does not (F-A5-1). The
   note is stale in the sense that it declares the interaction verified when
   only half of it was.
8. **`test-components/miniapp-demo1-wasm/routes.json` declares
   `/api/events` and the stream routes without `public`** (correct — A2's
   `validate_route` rejects `public: true` on non-guest/websocket targets), so
   they are reachable anonymously today only because the `messaging` SSE and
   `stream` targets have no caller gate. That is A4's recorded D-A4-5/F6 and
   an existing backlog row, not something A5 changes — but every browser case
   in this suite depends on it, since a WebRTC caller is anonymous.

---

## §9 Risks and fallbacks

| Risk | Signal | Fallback |
|---|---|---|
| `forceTunnel=true` + streaming bodies stresses the ECDH `Decryptor` path in a way nothing has before | SSE or download case fails only in the tunnel variant | Keep the tunnel variant for the three non-streaming cases; scope SSE/download to direct WebRTC and file a backlog row. Criterion 2 only requires direct WebRTC |
| Suite runtime grows (10 new cases, 2 extra browser contexts, one substrate) | CI e2e job time | Drop the tunnel variant for the two two-session cases first; they are the slowest and the least differentiated |
| `cargo component build` inside `global-setup.ts` is slow on a cold CI cache | Setup timeout | The pipeline job already runs `mise run build:test-components` before the e2e action, so the build is warm; the in-setup build is a no-op there and a convenience locally |
| The 256 KiB upload exceeds a data-channel message limit | Upload case fails only with `forceTunnel=false` | The proxy already writes the `File` stream in reader-sized pieces (~64 KiB); if it still trips, lower to 128 KiB — still multi-chunk |
