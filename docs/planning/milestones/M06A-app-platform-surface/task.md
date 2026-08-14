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
> `wasm32-wasip2`". This milestone closes the two gaps that force that, then
> proves it by building the same app as a real WASM component and running the
> same tests against it.
>
> **What it is deliberately not.** Not `wasi:http` — a component still does not
> get a raw HTTP server, it gets a request handed to a function. Not WebSocket
> support. Not the Roym product, and not Roym's messaging or identity work,
> which is M06B.

## Goal

By the end of M06A, a SynApp deployed as a pure WASM component serves a
single-page web application — static assets, REST endpoints backed by guest
logic, large uploads and downloads, and live server-to-client updates — reached
through the client gateway by an ordinary browser, with the Playwright suite
covering each part.

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
   usually ends on "the interfaces exist". This one ends on "the same four
   browser tests that pass against the native app pass against the WASM one."

---

## The two gaps, verified against the tree

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

---

## Decisions

> **On identifiers.** `D-06A-n` are milestone-level decisions, matching the
> house `D-<scope>-<n>` shape. Bare `A1`–`A4` are **slices**, matching M05A's
> `A0`–`A7`. The two namespaces are separate; a reference to "A1" always means
> the slice.

| # | Decision | Why |
|---|---|---|
| D-06A-1 | **Static assets bypass the sandbox entirely.** The router resolves a path to a blob and streams it. The component is never instantiated. | A cold SPA load is tens of asset requests; none of them is a decision the guest should make. Content addressing also gives correct `ETag`/`Cache-Control` for free. |
| D-06A-2 | **Live updates use SSE, not WebSocket.** The demo's echo/broadcast test is rewritten over `subscribe-sse`, which is already wired through the bridge. | There is no inbound WebSocket support anywhere — the only `tokio-tungstenite` use is the WebRTC signalling *client* dialling out. Adding an upgrade path is real router work that serves nothing later: Roym's own live needs (typing indicators, read receipts, new-message nudges) are all server-to-client. |
| D-06A-3 | **Public readability is a declared property of the service, not a flag on the handler.** | The existing blob GET is signed and scoped on purpose. "Skip the HMAC" bolted onto it would be a security regression by accident. This is the same question ADR-0018 and [ADR-0022](../../../decisions/0022-two-tier-logical-service-discovery.md) §5 already need answered, so it is answered once. |
| D-06A-4 | **The demo app is a fixture, not a product.** It lives beside `miniapp-demo1-web` in `test-components/` and stays excluded from the workspace build graph. | Its job is to fail loudly when a primitive is missing. |

---

## Explicit non-goals

- **`wasi:http`.** A component gets a request handed to a function, not a
  socket. Adding the full WASI HTTP world is a much larger commitment and
  nothing here needs it.
- **Inbound WebSocket upgrade.** See D-06A-2. Tracked in the backlog if it is ever
  wanted.
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
| **A1** | **Blob-backed static serving.** Deploy carries an asset bundle; unpack into blobs; persist a path → hash manifest; router serves it without touching the sandbox — exact-path serving plus a trailing-slash directory index, not SPA history fallback (that's A3's problem); content type from the path; `ETag`/`Cache-Control` from the content hash; declared public readability (D-06A-3) | **Complete** |
| **A2** | **Guest HTTP route target.** A fourth `dispatch_route` target that hands method, path, params, headers, and body to the component and turns its return into an HTTP response; error and status mapping; body size caps; behaviour within the existing 5s `dispatch_epoch_timeout_secs` bound | — |
| **A3** | **The demo app.** A WASM component providing the same functionality as `miniapp-demo1-web`: UI bundle via A1, REST endpoints via A2, upload/download via the existing `stream` target, live updates via SSE | A1 **and** A2 |
| **A4** | **The Playwright suite.** The four cases from `webrtc.spec.ts` re-pointed at the demo app, with the echo/broadcast case rewritten over SSE | A3 |

**Slices A1 and A2 are independent** and can run in parallel. Both are single-concern
router changes with their own tests; A3 is the first thing that needs both.

### Open design points for the slice plans

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
  The first is clearer at the boundary; the second adds no WIT surface. Slice
  A2's plan decides, and the choice affects M06C's Web entrypoint.
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
  manifest with no `assets` field keeps deploying unchanged). Slice A2's plan
  may add further WIT surface if it chooses a dedicated guest export.
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
| 3 | Request for a path absent from the manifest | 404, no blob lookup, no sandbox instantiation |
| 4 | A request tries to read another service's assets | Refused — the manifest is per service, resolved from the connection's own `service_id`, matching the existing blob GET's `svc` check |
| 5 | Guest route handler traps or exceeds its epoch bound | 500 with a structured error; the connection stays usable; no partial response body |
| 6 | Guest returns a malformed or oversized response | Bounded and rejected, not streamed to the client |
| 7 | A service that never declared public assets is asked for one | 404, not 403 — absence and refusal look the same from outside |
| 8 | Many concurrent SSE subscribers on one service | Bounded; exhausting them degrades that service, not the node |

---

## Measurable exit criteria

1. The demo app deploys as a **single WASM component** — no container, no TCP
   service, no native helper binary.
2. All four Playwright cases pass against it in the direct WebRTC
   configuration.
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
8. Every row of the failure and security matrix has a test.
9. `cargo +nightly fmt --all`, `cargo clippy --workspace --all-targets
   --all-features`, `cargo test --workspace`, and `mise run test:e2e` are clean.

**Stretch, not required:** the same suite passing in the multi-hop
configuration. `multi-hop.spec.ts` covers static and REST but not upload or
SSE, so a pass there widens coverage rather than repeating it.
