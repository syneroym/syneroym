# Milestone 6A: App Platform Surface (M06A-app-platform-surface)

> **Provenance.** Split out of Milestone 6 on 2026-08-13 while reviewing
> [roym-integrated-experience-spec.md](../../../roym-integrated-experience-spec.md)
> against the tree. M6 was first split two ways (foundation / product); scoping
> the foundation half found two unrelated clusters inside it, so it is three:
> **M06A** (this — the HTTP surface an app needs to be a web app), **M06B**
> (Roym's own substrate foundations: durable messaging, person identity at the
> gateway, service visibility), and **M06C** (the Roym product itself). Same
> treatment M5 got, for the same reason.
>
> **What this milestone is.** Today a SynApp can be a web app only by *not*
> being a WASM component. `test-components/miniapp-demo1-web` — the fixture the
> whole Playwright suite runs against — is a native axum binary deployed as a
> TCP service, and M04A's status records that it "fails to build under
> `wasm32-wasip2`". This milestone closes the three gaps that force that, then
> proves it by building the same app as a real WASM component and running the
> same tests against it.
>
> **What it is deliberately not.** Not `wasi:http` — a component still does not
> get a raw HTTP server, it gets a request handed to a function, and a
> WebSocket whose lifetime the host owns. Not the Roym product, and not Roym's
> messaging or identity work, which is M06B.

## Goal

By the end of M06A, a SynApp deployed as a pure WASM component serves a
single-page web application — static assets, REST endpoints backed by guest
logic, large uploads and downloads, and live updates in both directions (SSE
and WebSocket) — reached through the client gateway by an ordinary browser,
with the Playwright suite covering each part.

---

## Why this comes before the Roym product

Three things, in order of how much each de-risks what follows.

1. **It removes the only non-WASM piece of Roym.** The experience spec
   currently carries a Web entrypoint service that is native-only and is the
   sole part of Roym exempt from its one-source/two-builds rule
   ([D2/D3](../../../roym-integrated-experience-spec.md#decisions-of-record)).
   That exemption exists purely because a component cannot serve HTTP. Close
   the gap here and the exemption goes away instead of hardening into
   precedent.
2. **It exercises the primitives with a consumer that is cheap to throw away.**
   Every gap this finds is a gap Roym would have found later, with product code
   built on top of it.
3. **It gives the foundation a runnable exit gate.** A foundation milestone
   usually ends on "the interfaces exist". This one ends on "every browser test
   that passes against the native app passes against the WASM one."

---

## The gaps, verified against the tree

**Gap 1 — inbound HTTP cannot reach guest logic.** `dispatch_route`
([http.rs:583](../../../../crates/router/src/route_handler/http.rs#L583)) has
exactly three targets: `data-layer`, `messaging`, `stream`. The first two call
the native capability directly *on the service's behalf* — the guest is never
invoked, so a route can store a comment but cannot validate it, stamp it, or
react to it. Only `stream` reaches the guest, and only for `accept-upload`
([http.rs:809](../../../../crates/router/src/route_handler/http.rs#L809)).

This is a gap in one direction only. A guest calling *out* to `data-layer`,
`messaging`, `blob-store`, `app-config`, `vault`, or `proxy` has always worked;
those are its imports. What is missing is HTTP coming *in* to guest code as an
ordinary request/response.

**Gap 2 — a deploy cannot carry a site.** `artifact-source::binary` carries WASM
bytes, `document-source::inline` carries a text document, and ADR-0019 §2 gave
Podman volumes per-file content — but nothing carries a bundle of static assets
for a WASM service, and nothing serves one. The one existing HTTP path to blob
bytes, `GET /blobs/<hash>?svc=&exp=&sig=`
([http.rs:883](../../../../crates/router/src/route_handler/http.rs#L883)), is
deliberately hash-addressed, HMAC-signed, and expiring — the opposite of what a
public website needs.

**Gap 3 — no inbound WebSocket, so one native-fixture test has no WASM
counterpart.** Nothing anywhere handles an `Upgrade: websocket` request; the
only `tokio-tungstenite` use is the WebRTC signalling *client* dialling out.
This is narrower than it looks: the client gateway is already
WebSocket-transparent, reading headers from the first request only and then
handing the whole socket to one iroh stream for the connection's lifetime
([gateway.rs:256-263](../../../../crates/client_gateway/src/gateway.rs#L256)),
and `tokio-tungstenite` is already a dependency of `syneroym-router`. What is
missing is the upgrade handshake in the HTTP bridge and a guest boundary for a
connection the host owns — see `D-06A-2` and slice A3.

The shaping constraint on that boundary is `dispatch_epoch_timeout_secs`,
which defaults to **5 seconds** ([config.rs:449](../../../../crates/core/src/config.rs#L449))
and bounds every guest entry point. A guest can therefore never hold a
connection open; the host owns lifetime and calls the guest in short bursts.
That is exactly the shape `stream-cursor`/`stream-sink` already have, which is
why this is a wiring problem rather than a new execution model.

---

## Decisions

> **On identifiers.** `D-06A-n` are milestone-level decisions, matching the
> house `D-<scope>-<n>` shape. Bare `A1`–`A5` are **slices**, matching M05A's
> `A0`–`A7`. The two namespaces are separate; a reference to "A1" always means
> the slice.

| # | Decision | Why |
|---|---|---|
| D-06A-1 | **Static assets bypass the sandbox entirely.** The router resolves a path to a blob and streams it. The component is never instantiated. | A cold SPA load is tens of asset requests; none of them is a decision the guest should make. Content addressing also gives correct `ETag`/`Cache-Control` for free. |
| D-06A-2 | ~~**Live updates use SSE, not WebSocket.**~~ **Revised 2026-08-14: build both.** SSE stays, and inbound WebSocket is added as slice A3. The demo app exposes both, so each is exercised end to end. | **The original reasoning was weak and the cost estimate was wrong.** "WebSocket is HTTP-specific" is not a reason — the honest framing is that bidirectional streaming to a guest already exists substrate-side (ADR-0014 `raw://` routing, `stream-cursor`/`stream-sink`), and WebSocket is the *browser-edge compatibility shim* for the one client that cannot open a raw QUIC stream. Three things make it far cheaper than assumed: `tokio-tungstenite` is **already a dependency of `syneroym-router`**; the client gateway is already WebSocket-transparent, handing the whole socket to one iroh stream after reading the first request's headers ([gateway.rs:256-263](../../../../crates/client_gateway/src/gateway.rs#L256)); and A2 has since established the `syneroym:http` package this extends. Leaving it out would also have broken this milestone's own premise — the native fixture tests WebSocket, and *"a divergence between them is a finding"*. |
| D-06A-3 | **Public readability is a declared property of the service, not a flag on the handler.** | The existing blob GET is signed and scoped on purpose. "Skip the HMAC" bolted onto it would be a security regression by accident. This is the same question ADR-0018 and [ADR-0022](../../../decisions/0022-two-tier-logical-service-discovery.md) §5 already need answered, so it is answered once. |
| D-06A-4 | **The demo app is a fixture, not a product.** It lives beside `miniapp-demo1-web` in `test-components/` and stays excluded from the workspace build graph. | Its job is to fail loudly when a primitive is missing. |

---

## Explicit non-goals

- **`wasi:http`.** A component gets a request handed to a function, not a
  socket. Adding the full WASI HTTP world is a much larger commitment and
  nothing here needs it.
- **WebTransport and HTTP/3 datagrams.** Narrower browser support, a larger
  lift, and QUIC-native apps already have raw streams (ADR-0014). WebSocket
  (slice A3) covers the browser edge; this would be a second way to do it.
- **Retiring `miniapp-demo1-web`.** It stays. Two fixtures exercising the same
  behaviour through different execution models is the point — a divergence
  between them is a finding.
- **The `svc deploy --volume` file-materialisation CLI gap.** Container-path
  work, already its own backlog row, untouched here.
- **Roym's durable messaging interface, person identity at the gateway, and
  service-record visibility.** M06B.

---

## Dependency gates

| Depends on | State |
|---|---|
| M3B HTTP bridge (`http://http-native\|<service_id>`, per-service `http_routes`) | Shipped |
| M3B stream protocols (`register-stream-protocol`, `stream-cursor`/`stream-sink`) | Shipped |
| Blob store + storage provider + service DEK | Shipped |
| M04A caller identity through native dispatch (ADR-0016) | Shipped |
| Playwright harness, global setup/teardown, WebRTC + multi-hop configs | Shipped |

Nothing here is gated on M06B.

---

## Slices

| # | Scope | Gate |
|---|---|---|
| **A1** | **Blob-backed static serving.** Deploy carries an asset bundle; unpack into blobs; persist a path → hash manifest; router serves it without touching the sandbox — exact-path serving plus a trailing-slash directory index, not SPA history fallback (that's A4's problem); content type from the path; `ETag`/`Cache-Control` from the content hash; declared public readability (D-06A-3) | **Complete** |
| **A2** | **Guest HTTP route target.** A fourth `dispatch_route` target that hands method, path, params, headers, and body to the component and turns its return into an HTTP response; error and status mapping; body size caps; behaviour within the existing 5s `dispatch_epoch_timeout_secs` bound; a deploy-time export check (a declared `guest` route whose component doesn't export the handler fails the deploy) and a non-`Wasm`-service refusal, mirroring A1's asset-bundle gates; the route is authenticated by default, reachable by an anonymous caller only when explicitly declared `public` (D-A2-7) | **Complete** |
| **A3** | **Inbound WebSocket.** `Upgrade` detection and the 101 handshake in the HTTP bridge; frame codec via the router's existing `tokio-tungstenite`; a `websocket-handler` interface extending `syneroym:http` with guest exports (`on-open`/`on-message`/`on-close`) plus the host side that owns connection lifetime. Unicast reply by host import; **broadcast reuses the pub-sub broker** rather than inventing a second fan-out (D-06A-2) | **Complete** |
| **A4** | **The demo app.** A WASM component providing the same functionality as `miniapp-demo1-web`: UI bundle via A1, REST endpoints via A2, upload/download via the existing `stream` target, and live updates **both ways — over SSE and over WebSocket**, so each is exercised end to end | **Complete** |
| **A5** | **The Playwright suite.** The four cases from `webrtc.spec.ts` re-pointed at the demo app — including echo/broadcast over a real WebSocket, matching the native fixture — plus a fifth covering the SSE path | A4 |

**Slices A1 and A2 are independent** and can run in parallel; both are complete.
A3 needs A2's `syneroym:http` package to extend. A4 is the first thing needing
all three.

> **Why A3 precedes the demo app.** Pairing both directions of a long-lived
> connection at the guest boundary is the one genuine unknown in the WebSocket
> estimate, and it is a *design* question — what the guest-side API looks like.
> Settling it while the demo app is still a blank page costs less than building
> the app against SSE and retrofitting. A4 is the proof slice; running it once,
> against everything, is the point.

### Open design points for the slice plans

- **How a guest pushes on a WebSocket it does not own** (slice A3). The 5s
  `dispatch_epoch_timeout_secs` bounds every guest entry point, so a guest can
  never hold a connection open — the host owns lifetime and calls the guest in
  short bursts. That settles the *inbound* direction (`on-message` per frame),
  but not the outbound one, where a guest must be able to send without having
  just been asked. Two mechanisms look right and A3's plan should confirm both:
  a host import for **unicast** (`send(conn-id, frame)` — the echo case), and
  **broadcast reusing the existing pub-sub broker**, where a connection
  subscribes to a messaging topic at open and any ordinary `messaging.publish`
  fans out to it. The second is what makes "all HTTP server functionality
  through existing primitives" true rather than aspirational: broadcast stops
  being a new mechanism and becomes pub-sub with a different egress.

- **Where the path → hash manifest lives.** `http_routes` rides in
  `custom_config` JSON, which suits a handful of entries and not a few hundred
  files. **Resolved by slice A1's plan (D-A1-2):** deploy writes the manifest
  as its own blob and puts that hash in the config-equivalent `asset-bundle`
  record — but only the writing half. Router-side loading is a cache
  (`AssetRegistry`, populated by `deploy()`/cleared by `undeploy()`), not a
  boot-time restore from the stored manifest blob; nothing in this tree reads
  it back after a restart. Recorded as a deliberate scope line, not an
  oversight — see [deferred-backlog.md](../../planning/deferred-backlog.md)
  §4.
- **What a guest HTTP handler looks like in WIT.** A dedicated
  `handle-request`-style export, or a JSON-RPC method invoked by convention.
  The first is clearer at the boundary; the second adds no WIT surface.
  **Resolved by slice A2's plan (D-A2-1):** a dedicated
  `syneroym:http/incoming-handler@0.1.0` export, `handle-request` — reached by
  calling the sandbox engine directly (`AppSandboxEngine::
  handle_guest_http_request`), not through `dispatch_json_rpc_once`, since an
  `http-native` connection always resolves to a `NativeService` pipeline and
  can never reach a guest that way. No M06C document exists in the tree yet to
  check the "affects M06C's Web entrypoint" note against.
- **Whether the demo app needs its own DEK-scoped bundle.** Static assets are
  public by D-06A-3, so encrypting them at rest buys nothing against the
  stated threat model but keeps one storage path instead of two. **Resolved
  by slice A1's plan (D-A1-3):** assets use the service DEK like every other
  blob — one storage path, not two.

---

## Migration impact

- **A new `http_routes` target value.** Additive. An existing route table names
  only the three current targets and keeps working; the unknown-target arm
  already returns a clean 500 rather than misrouting.
- **A new deploy-time artifact kind.** Additive, and absent means today's
  behaviour — a service with no asset bundle serves no static paths.
- **A1 does add WIT**, correcting this section's earlier claim that only A2
  might: a `visibility` enum, an `asset-bundle` record, and
  `service-config.assets: option<asset-bundle>` (all additive — an existing
  manifest with no `assets` field keeps deploying unchanged). **A2 does too**:
  a new standalone `syneroym:http@0.1.0` package (`incoming-handler`, two
  records, one enum) — additive, and deliberately *not* added to the
  `host-environment` world, so a component that doesn't export it deploys
  exactly as before. `HttpRoute` also gains a `public: bool` field
  (`#[serde(default)]`, defaulting `false`), additive for the same reason.
- **No wire-format change** to endpoint records, topology documents, or gateway
  hostnames.

---

## Reference scenario (runnable)

```
1. Build the demo component for wasm32-wasip2 and its client bundle
2. Deploy it as a single WASM SynSvc, asset bundle carried in the deploy call
3. Browser opens the service's gateway hostname
4. GET /                    -> index.html from blobs, sandbox never instantiated
5. GET /assets/*            -> same path; second load is a 304 from the ETag
6. POST /api/comments       -> guest validates and writes via data-layer
7. GET /api/comments        -> guest reads and shapes the response
8. POST /api/upload (large) -> existing stream target, guest sink
9. GET /api/download (large)-> existing stream protocol, guest cursor
10. SSE /api/events         -> guest publishes; browser receives live updates
```

Step 4 is the one to watch: if the sandbox is instantiated, slice A1 is wrong.

---

## Failure and security matrix

| # | Case | Expected |
|---|---|---|
| 1 | Asset bundle contains `../` or an absolute path | Rejected at deploy, before any blob is written |
| 2 | Asset bundle exceeds the size cap | Rejected at deploy with a clear error, no partial state. Slice A1's plan (D-A1-5) settles this as three caps, not one: a cheap `MAX_ASSET_BUNDLE_BYTES` (2 MiB) early guard, `MAX_ASSET_UNPACKED_BYTES` (64 MiB) against a decompression bomb, and the *authoritative* check — the combined `encoded(component) + encoded(bundle) + envelope` fitting the 16 MiB RPC frame, checked client-side, since both expand ~3.57× as JSON integer arrays and share one frame with the component binary |
| 3 | Request for a path absent from the manifest | 405 (falls through to the JSON-RPC bridge's uniform non-`POST` rejection — the same status any other unmatched `GET`/`HEAD` gets, asset or not), no blob lookup, no sandbox instantiation |
| 4 | A request tries to read another service's assets | Refused — the manifest is per service, resolved from the connection's own `service_id`, matching the existing blob GET's `svc` check |
| 5 | Guest route handler traps or exceeds its epoch bound | 500 with a structured error; the connection stays usable; no partial response body. **A2 covers the guest-*wasm-execution* half only** — the epoch deadline does not interrupt a guest blocked inside a host call, and no guest-reachable host function in this tree blocks unboundedly today, so that half is deliberately untested rather than tested badly (backlog row owed, see [deferred-backlog.md](../../planning/deferred-backlog.md)) |
| 6 | Guest returns a malformed or oversized response | Bounded and rejected, not streamed to the client. One word of precision: the host cannot bound the *allocation* — a guest's `list<u8>` return is fully materialised in host memory before its size is knowable; what `MAX_GUEST_RESPONSE_BODY_BYTES` bounds is what gets **sent**, and the allocation bound is the guest's own `max_memory_bytes` store limiter |
| 7 | A service that never declared public assets is asked for one | Same 405 as row 3, not 403 — absence and refusal look the same from outside |
| 8 | Many concurrent SSE subscribers on one service | Bounded; exhausting them degrades that service, not the node. The principle is general, not SSE-specific: A2's `D-A2-11` applies it to guest HTTP concurrency too, bounding it per service with a 503 past a fixed admission wait rather than the wasmtime pool's own hard instantiation failure |

---

## Measurable exit criteria

1. The demo app deploys as a **single WASM component** — no container, no TCP
   service, no native helper binary.
2. All five Playwright cases pass against it in the direct WebRTC
   configuration — the four from `webrtc.spec.ts`, plus the SSE case.
3. `GET /` and every asset request completes **without instantiating the
   component**, shown by a test that asserts on instantiation count, not by
   inspection.
4. A repeat asset load returns 304 from the `ETag`.
5. A POST that the guest rejects returns the guest's own status and message —
   proving guest logic is genuinely in the request path and the route is not
   silently declarative.
6. Upload and download of a file large enough to require multiple chunks
   complete and round-trip byte-identical.
7. An SSE subscriber receives an update published by a different browser
   session.
8. **A WebSocket client echoes a frame off the guest, and a frame published by
   a different browser session is broadcast to it** — the same behaviour the
   native fixture's `WebSocket Echo and Broadcast` case tests, so the two
   fixtures no longer diverge.
9. Every row of the failure and security matrix has a test.
10. `cargo +nightly fmt --all`, `cargo clippy --workspace --all-targets
   --all-features`, `cargo test --workspace`, and `mise run test:e2e` are clean.

**Stretch, not required:** the same suite passing in the multi-hop
configuration. `multi-hop.spec.ts` covers static and REST but not upload or
SSE, so a pass there widens coverage rather than repeating it.
