# Milestone 3B: Messaging, Streaming, and HTTP Bridge (M03B-messaging)

> **Provenance.** This milestone was split out of `docs/planning/milestones/M03-sss/task.md`
> on 2026-07-09, after detailed pre-implementation planning for Slices 6A, 6B,
> and 7 surfaced enough scope, new-infrastructure design, and open questions
> that keeping them inside the already-large M03-sss `task.md`/`status.md`
> (which also carries the fully-shipped M3A + Slice 5 record) was becoming
> unmanageable. Nothing here is a new requirement — it is the unimplemented
> tail of M3 (the pub/sub half of M3B, plus all of M3C), carried over with
> the slice numbers (**Slice 6A**, **Slice 6B**, **Slice 7**) and
> sub-milestone labels (**M3B**, **M3C**) unchanged from the original
> planning, so every existing cross-reference (ADR-0010, system-architecture.md,
> meta-implementation-plan.md) that already says "Slice 6A" or "M3C" still
> means exactly the same thing — only the file it lives in has moved.
>
> **What stayed in `M03-sss`:** Slices 0-5, including the M3B blob-object
> service (Slice 5), which is fully shipped and verified — see
> `docs/planning/milestones/M03-sss/status.md`. M03-sss's own reference
> scenario ends at step 12 (M3A) plus step 13 (blob store, M3B/Slice 5). This
> document's reference scenario continues from step 13 as a precondition.
>
> **Pre-implementation review.** Before any slice below is implemented, a
> detailed review of the original Slice 6A/6B/7 plan (as it stood in
> M03-sss) surfaced eight architecture-level findings and about a dozen
> smaller consistency errors. All of them are incorporated into the task
> lists below; §"Review Findings Incorporated" at the end of this document
> records what changed and why, for traceability. One explicit user
> decision shapes several of these: **this WIT surface is not released to
> the outside world**, so breaking changes across Slice 6A → 6B are
> acceptable and the plan no longer pays a stability tax to avoid them.

## Goal

Deliver the pub/sub half of `syneroym:messaging` with an embedded broker and
native (non-WASM) dispatch (Slice 6A); generic bidirectional streaming,
guest-as-source and guest-as-sink (Slice 6B); and an HTTP-verb-based bridge
onto data-layer, blob-store, and messaging, including an SSE subscription
path (Slice 7).

---

## Requirement IDs (Traceability)

| Requirement ID | Description | Scope in this milestone |
|---|---|---|
| `[PLT-DAP-04]` | Decentralized Pub/Sub | MQTT-shaped API via in-process `rumqttd`, wildcard topics, retained messages, cross-service pub/sub with namespace isolation, native (non-WASM) dispatch, push delivery to a live `SyneroymClient` (Slice 6A). Iroh QUIC log-replication overlay for broker durability remains deferred to M7 (`[PLT-RED]`) — unchanged from ADR-0010 Amendment 2; not this milestone's concern. **(Note: M7 will explicitly consolidate WAL replication, MQTT topic-log replication, and blob replication under a single unified log-shipping stream primitive, endorsing the 'everything is a stream' durability pattern.)** |
| `[PLT-DAP-06]` | Generic Bidirectional Streaming | `stream-cursor` (guest-as-source pull) and `stream-sink` (guest-as-sink push) resources, `register-stream-protocol`/`handle-stream-request`/`accept-stream-upload` flows, host-side QUIC stream acceptance and routing (Slice 6B). |
| `[PLT-DAT]` | Data Layer (HTTP-facing sub-requirement) | HTTP verb/path routing onto `data-layer`/`blob-store`/`messaging` native dispatch, signed-URL blob serving over a live HTTP endpoint, chunked upload onto `stream-sink` (Slice 7). |

---

## Resolved Decisions (ADR References)

- [ADR-0010](../../../decisions/0010-mqtt-broker-rumqttd.md) (D-03-05) — broker
  deployment, delivery model, topic namespacing, backpressure, cancellation.
  Still governs Slice 6A in full; this document's Slice 6A tasks are the
  concrete implementation of that ADR plus the findings in
  §"Review Findings Incorporated".
- **New design note/ADR required before Slice 6B implementation begins**
  (carried over from the original M3C dependency gate) — documents
  host-side QUIC stream acceptance/routing, which has no prior ADR or
  precedent in this codebase. See Slice 6B's "Design Note Requirements"
  subsection for exactly what it must specify.

---

## Dependency Gates

This milestone may begin **only when**:

1. `docs/planning/milestones/M03-sss/status.md` records Slice 5 (M3B blob
   object service) as complete, with `cargo test --workspace`, clippy, and
   `mise run test:e2e` all green on that branch.
2. Decision D-03-05 is resolved (already true — [ADR-0010](../../../decisions/0010-mqtt-broker-rumqttd.md)).

**Slice 6B gate (additional):**

3. Slice 6A exit criteria (below) are met and recorded in this milestone's
   `status.md`.
4. The design note/ADR for host-side QUIC stream acceptance/routing is
   recorded in `docs/decisions/` before Slice 6B implementation begins.

**Slice 7 gate (additional):**

5. Slice 6B exit criteria are met and recorded in this milestone's `status.md`
   — Slice 7's chunked-upload bridge depends directly on `stream-sink`, and
   its SSE bridge depends on the push-delivery mechanism Slice 6A built.

---

## Gaps to Close

| Gap | Target Slice |
|---|---|
| No embedded MQTT broker (`rumqttd`) | Slice 6A |
| No `syneroym:messaging` WIT interface (pub/sub half) | Slice 6A |
| No MQTT wildcard topic or retained message support | Slice 6A |
| No native (non-WASM) dispatch for `messaging` — `data-layer`/`blob-store`/`vault`/`app-config` have it since M03-sss Slice 5; `messaging` doesn't yet, and `vault`/`app-config` dispatch exists but has no dedicated round-trip test either (this milestone should close that gap alongside adding `messaging`'s) | Slice 6A |
| No push-delivery path to a non-WASM (`SyneroymClient`) subscriber — no existing `NativeService` call can hold a stream open across multiple emitted messages | Slice 6A |
| No persistence or restart-recovery for MQTT subscriptions or stream-protocol registrations (see Finding A1) | Slice 6A, 6B |
| No `syneroym:messaging` bidirectional streaming, guest-as-source (`stream-cursor`, `handle-stream-request`) | Slice 6B |
| No `syneroym:messaging` bidirectional streaming, guest-as-sink (`stream-sink`, `accept-stream-upload`) | Slice 6B |
| No `register-stream-protocol` host-side routing table | Slice 6B |
| No host-side QUIC stream acceptance/routing for peer-initiated streams (either direction) | Slice 6B |
| No guest-implemented (as opposed to host-implemented) WIT resource precedent — `stream-cursor`/`stream-sink` are the first (see Finding A6) | Slice 6B |
| No HTTP verb/path routing bridged onto native dispatch (`crates/router/src/route_handler/http.rs` is JSON-RPC-over-POST only, and its HTTP responses are fully-buffered `Full<Bytes>`, which also blocks SSE and large streamed blob `GET`s) | Slice 7 |
| No chunked-upload-to-`stream-sink` bridge in the HTTP router | Slice 7 |
| No live HTTP endpoint resolving `signed-url()` (the function itself shipped in M03-sss Slice 5; no route serves it) | Slice 7 |

---

## WIT Boundary Versioning (Revised Strategy)

The original M03-sss planning declared the *entire* `syneroym:messaging@0.1.0`
surface — including the Slice 6B streaming resources and
`register-stream-protocol` — inside Slice 6A's WIT file, unimplemented, so
that no later slice would need a breaking WIT change. **That strategy is
dropped for this milestone** (explicit user decision: this WIT package is
never released outside this repository, so a breaking addition between
Slice 6A and Slice 6B costs nothing and buys real simplification — see
Finding A3).

Concretely:

- **Slice 6A** declares only `host-api::publish`/`subscribe`/`unsubscribe`
  and `guest-api::handle-message`. No `stream-types` interface, no
  `register-stream-protocol`, no streaming guest exports, and no "not yet
  implemented" placeholder machinery for them.
- **Slice 6B** adds `register-stream-protocol` to `host-api`, adds a new
  `stream-types` interface (`stream-cursor`, `stream-sink` resources), and
  adds `handle-stream-request`/`accept-stream-upload` to `guest-api` — a
  normal additive-and-occasionally-breaking WIT change, exactly like every
  other slice-to-slice WIT evolution in this codebase (e.g. `data-layer.wit`
  gained fields across Slices 1→3A without a stability pact).
- A consequence: components written against Slice 6A's `guest-api` that only
  care about pub/sub do **not** need to stub out streaming exports they
  don't use — WIT export granularity is per-interface, and `guest-api` in
  Slice 6A simply doesn't have streaming methods on it yet.
- `syneroym:messaging@0.1.0`'s version number does not need to bump between
  6A and 6B; this is pre-release, in-repo-only surface.

---

## Slice 6A: Messaging WIT and Embedded Pub/Sub Broker

**Requirement IDs:** `[PLT-DAP-04]`, `[PLT-DAT]` (MQTT event service sub-requirement)
**ADR references:** [ADR-0010](../../../decisions/0010-mqtt-broker-rumqttd.md)
**Depends on:** Dependency Gate 1-2 above (M03-sss Slice 5 closed; D-03-05 resolved).

### Day-1 Spike (do this first, before committing to the broker integration shape)

- [x] Add `rumqttd` behind a throwaway `examples/` or test binary and prove,
  against the actual pinned version, that:
  - The in-process `Broker::link(client_id)` API (`LinkTx`/`LinkRx`) works
    without a network listener bound (see "Broker Embedding" below — no
    `[mqtt] bind_addr` is planned; confirm `rumqttd` doesn't require one).
  - Retained messages are delivered to a subscriber that joins *after* the
    retaining publish. This has had bug-fix churn upstream; verify against
    the exact pinned version rather than trusting the crate's advertised
    feature list.
  - `LinkTx::publish(topic, payload)` has **no retain parameter** in the
    current `rumqttd` API — confirm whether retaining a message requires
    constructing a raw MQTT `Publish` packet via `LinkTx::send()` instead,
    and record the concrete approach here before writing broker-wrapper code
    against it.
  - `+`/`#` wildcard subscriptions match as expected through the link API.
- [x] Record the spike's findings (working code path, any surprises) in this
  milestone's `status.md` before proceeding — this de-risks the rest of the
  slice instead of discovering broker API gaps mid-integration.

### WIT Interface

- [x] Create `crates/wit_interfaces/wit/messaging/messaging.wit`, package
  `syneroym:messaging@0.1.0`:
  - `interface host-api` (host-imported, guest-triggered):
    - `publish(topic: string, payload: list<u8>) -> result<_, messaging-error>`
    - `subscribe(topic: string) -> result<_, messaging-error>`
    - `unsubscribe(topic: string) -> result<_, messaging-error>`
  - `interface guest-api` (guest-exported, host-triggered):
    - `handle-message(topic: string, payload: list<u8>) -> result<_, string>`
  - `variant messaging-error { permission-denied, internal(string) }` —
    matches [ADR-0010](../../../decisions/0010-mqtt-broker-rumqttd.md)'s
    `pubsub-error` variant (the original Slice 6A draft used
    `result<_, string>` throughout, drifting from the ADR; fixed here so
    callers can distinguish backpressure/internal failures from permission
    errors programmatically).
  - World: `world messaging-guest { import host-api; export guest-api; }`.
  - Delivery remains push-model: host invokes the component's exported
    `guest-api::handle-message` if declared; if not declared, the
    subscription is registered but messages are silently discarded (per
    ADR-0010).
  - No `stream-types`, `register-stream-protocol`, `handle-stream-request`,
    or `accept-stream-upload` in this slice — see "WIT Boundary Versioning"
    above.

### Broker Embedding

- [x] Add `rumqttd` (pin the exact version proven in the day-1 spike, with
  the pinning rationale as a comment per workspace convention) and
  `tokio-util` (if not already present) to `Cargo.toml`.
- [x] Create `crates/mqtt-broker/` crate with `MqttBroker`:
  - Starts `rumqttd`'s router as a Tokio background task; the substrate
    process talks to it exclusively through `Broker::link()` in-process
    links. **No `[mqtt] bind_addr` / network listener is configured** — see
    Finding A5. If a real MQTT-protocol listener is wanted later (e.g. for
    external MQTT tooling to observe substrate topics), it must be a
    separate, explicitly opt-in decision with its own auth story, not a
    default of this slice.
  - Exposes `publish(topic, payload) -> Result<()>` and
    `subscribe(topic, sender: mpsc::Sender<(String, Vec<u8>)>) -> Result<SubscriptionHandle>`
    async APIs, where `SubscriptionHandle`'s `Drop` unsubscribes.
  - Bridges the host-facing API to the broker's internal channel with a
    **bounded channel** (default capacity: 1024 messages, configurable via
    `[mqtt].channel_capacity`). When full, `publish` returns
    `messaging-error::internal("broker channel full: backpressure")` —
    never blocks the Wasmtime execution thread.
  - Uses `CancellationToken` to cleanly terminate the broker task on `Drop`
    (applying the M2 epoch-timer-audit lesson, per ADR-0010).
  - Topics are namespaced by the host: guest/native-caller topic `t`
    published by service `s` becomes `svc/<s>/t` internally. A subscribe
    call whose topic string does **not** start with `svc/` is treated as
    "my own namespace" and prefixed; a subscribe call starting with `svc/`
    is taken literally (cross-service, opt-in). This disambiguation rule
    was implicit in the original ADR text and is made explicit here because
    the reference scenario below depends on getting it right (see Finding B1).

### Subscription and Registration Persistence (Finding A1)

> `docs/system-architecture.md` §2 already asserts "Subscriptions are
> persisted by the host" for the guest-delivery model — this was stated in
> architecture but never operationalized as a task in the original Slice 6A
> plan. Under the current WASM execution model
> (`AppSandboxEngine::build_store_and_instantiate`,
> `crates/sandbox_wasm/src/engine.rs`), every invocation gets a **fresh**
> `Store`/`Instance` — there is no long-lived guest instance for the broker
> to hold a subscription against. Without persistence, every substrate
> restart silently drops all subscriptions with no guest code path ever
> re-invoked to restore them (there is no `on-start` lifecycle hook).

- [x] Add a `messaging_subscriptions` table to `substrate.db`:
  ```sql
  CREATE TABLE IF NOT EXISTS messaging_subscriptions (
    service_id TEXT NOT NULL,
    topic      TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (service_id, topic)
  );
  ```
- [x] `host-api::subscribe`/`unsubscribe` (guest path) write/delete rows here
  in addition to registering with `MqttBroker`.
- [x] On substrate startup, after `MqttBroker` is constructed, replay every
  row in `messaging_subscriptions` into the broker so guest subscriptions
  survive a restart. (A native caller's subscription is inherently
  connection-scoped — see Slice 6A "Native Push Delivery" below — and is
  *not* persisted or replayed; only guest-delivery subscriptions are.)
- [x] `ControlPlaneService::undeploy` removes all `messaging_subscriptions`
  rows for the undeployed `service_id` and unsubscribes them from the live
  broker. (This was missing from the original plan entirely — undeploy only
  deregistered the `EndpointRegistry` interface.)

### Host Function Wiring

- [x] Wire `syneroym:messaging/host-api.publish`/`subscribe`/`unsubscribe`
  in `engine.rs` → `MqttBroker`.
- [x] Wire delivery: broker message → host invokes the deployed component's
  `guest-api::handle-message` export, **if declared**, via a new direct
  `Val`-construction invocation helper modeled on the existing
  `invoke_lifecycle_hook` (`crates/sandbox_wasm/src/engine.rs`) — **not**
  through `execute_wasm`'s JSON-parameter path. `json_to_wasm_params`
  (`crates/sandbox_wasm/src/conversions.rs`) only handles `String`/`U32`/
  `Bool` today; `handle-message`'s `payload: list<u8>` parameter is
  unsupported by it, and routing binary payloads through a JSON
  intermediate would be both lossy-prone and slower than constructing
  `Val::String(topic)`/`Val::List(payload.iter().map(|b| Val::U8(*b))...)`
  directly, the way `invoke_lifecycle_hook` already builds its (empty)
  argument list directly. Runs on a Tokio task separate from the broker's
  own delivery loop (per ADR-0010), instantiating a fresh Store per
  delivery (see the performance-budget note below on why this doesn't hit
  a 5ms target).
- [x] Re-arm each delivery instance's epoch deadline and fuel budget exactly
  as `build_store_and_instantiate` already does for every other invocation
  — no special-casing needed here since each `handle-message` delivery is
  its own fresh Store/instance (same reasoning as any other WASM call), but
  call this out explicitly since Slice 6B's long-lived stream instances
  will need different handling (see Slice 6B's "Quota Handling" section).

### Native (Non-WASM) Dispatch

> Mirrors the pattern established in M03-sss Slice 5 for
> `data-layer`/`vault`/`app-config`/`blob-store`: every deployed service
> (regardless of `service-type` — wasm/container/tcp) gets a
> native-capability interface so an external caller doesn't need a WASM
> component in the loop.

- [x] Extend `crates/control_plane/src/synsvc_native.rs`'s
  `SynSvcNativeService` with `dispatch_messaging`, handling
  `messaging/publish` (request/response — fits the existing `NativeService`
  trait unchanged) onto the same `MqttBroker` the WASM `Host` impl uses.
  `messaging/subscribe`/`unsubscribe` do **not** go through this path — see
  "Native Push Delivery" below, which requires router-level changes
  `NativeService::dispatch`'s single-request/single-response shape cannot
  express.
- [x] `ControlPlaneService::deploy`/`undeploy`: register/deregister the
  `messaging` native-capability `EndpointRegistry` interface alongside the
  existing four for every deployed service.
- [x] Add `"messaging"` to `NATIVE_CAPABILITY_INTERFACES` in
  `crates/control_plane/src/service.rs` (currently a fixed `[&str; 4]` —
  see Finding B3). Forgetting this reproduces the exact `endpoint_type`
  flake Slice 5 already hit and fixed once (`ControlPlaneService::list()`
  picking `endpoint_type` from whichever interface iterates first).
- [x] Add the missing round-trip tests for `vault` and `app-config` native
  dispatch while touching this file — both currently have dispatch code but
  no dedicated test, a gap the original plan flagged but didn't schedule.

### Native Push Delivery (`SyneroymClient`) — Design (Finding A2)

> A plain (non-WASM) caller has no export the host can invoke the way it
> invokes a WASM guest's `handle-message`, so `subscribe` needs a delivery
> path with no analogue elsewhere in the router. This is more than "keep
> the stream open" — three concrete problems in the existing code need a
> real design, not a one-line task:
>
> 1. **`NativeService::dispatch`** (`crates/rpc/src/native.rs`) takes a
>    `NativeInvocation` and returns one `NativeResponse` — it has no handle
>    to the underlying QUIC stream at all. The router must special-case
>    `messaging/subscribe` *before* falling into the generic
>    `dispatch_json_rpc_once` path (`crates/router/src/route_handler/dispatch.rs`),
>    not extend the `NativeService` trait for one method.
>  2. **The existing binary-stream loop exits on EOF and hands the writer
>     back.** `handle_json_rpc_loop` (`dispatch.rs`) owns `writer: &mut W`
>     for its lifetime and returns when `framing::read_frame` sees EOF.
>     `SyneroymClient::request_raw` (`crates/sdk/src/lib.rs`) calls
>     `send.finish()` right after writing its request — exactly the pattern
>     a naive `subscribe()` client would reuse, which would make the
>     server-side reader hit EOF and the whole stream handler return,
>     killing the subscription before the first message arrives. The
>     subscribe path must **not** reuse `request_raw` as-is: the client
>     writes its `subscribe` request but deliberately does **not** finish
>     the send side, and the server-side handler for this one method must
>     move the writer into a dedicated long-lived task instead of returning
>     from `handle_stream` when the read side goes quiet.
>  3. **One writer, fed by a channel.** The subscribe ack and every
>     subsequent notification share the same `SendStream`; don't hand the
>     raw write-half to `MqttBroker::subscribe` as the `sender` directly
>     (a `SendStream` isn't `Send + Sync`-cloneable the way a broker fan-out
>     sender needs to be). Instead: `MqttBroker::subscribe` takes an
>     `mpsc::Sender<(String, Vec<u8>)>`; a dedicated writer task owns the
>     `SendStream`, reads off the paired `mpsc::Receiver`, and calls
>     `framing::write_frame` with a JSON-RPC notification frame
>     (`method: "messaging/message"`, `params: {topic, payload}`, `id: None`
>     — this serializes fine against the existing `JsonRpcRequest`/response
>     types, verified: `id: Option<Value>` already supports `None`).
>
> **Lifecycle and cleanup, decided here (not left open):**
> - **Unsubscribe = close the stream.** A native subscriber has no separate
>   stream to send an `unsubscribe` request on that would correlate to the
>   original `subscribe` call. The router-side handler for this method
>   treats client-initiated stream closure as the unsubscribe signal and
>   drops the `SubscriptionHandle`.
> - **Dead-subscriber cleanup.** The writer task selects between the
>   `mpsc::Receiver` and the QUIC stream's own close/error signal
>   (`SendStream::stopped()` on the Iroh/QUIC send half); on either firing,
>   it drops the `SubscriptionHandle`, unsubscribing from the broker instead
>   of leaking a registration that will only be discovered on the next
>   failed write.
> - **Backpressure.** The per-subscriber `mpsc` channel is bounded (reuse
>   `[mqtt].channel_capacity`); a subscriber that can't keep up gets
>   messages dropped past the bound rather than blocking the broker's
>   delivery loop — document this as "at-most-once, best-effort under load"
>   for native subscribers in the WIT/ADR text, since it differs from the
>   guest-delivery path's own backpressure story (ADR-0010's bounded
>   *publish*-side channel).

- [x] Implement the design above: router-level special-casing for
  `messaging/subscribe` on the native-dispatch path, writer task, mpsc
  bridge, `SendStream::stopped()`-driven cleanup.
- [x] Client side: add `SyneroymClient::subscribe(interface, topic) ->
  Result<MessageStream>` in `crates/sdk/src/lib.rs` — opens a bidirectional
  stream (not via `request_raw`, for the EOF reason above), sends the
  `subscribe` request, does **not** finish the send side, then loops
  `framing::read_frame` on a background task, forwarding each parsed
  notification through a `tokio::sync::mpsc::Receiver` (or equivalent
  `Stream` impl) the caller reads from. Dropping the returned
  `MessageStream` should close the send side, triggering server-side
  cleanup per the unsubscribe-by-close rule above.

### Wildcard and Retained Messages

- [x] Wire `+`/`#` wildcard subscriptions through to `rumqttd`'s router (no
  substrate-side wildcard logic needed if the day-1 spike confirms the link
  API passes them through as MQTT topic filters).
- [x] Wire retained-message support per the day-1 spike's findings (raw
  `Publish` packet construction via `LinkTx::send()` if `LinkTx::publish`
  indeed has no retain parameter, as expected).

### No Unauthenticated Broker Listener (Finding A5)

- [x] Confirm (and add a regression test if feasible) that no TCP port is
  opened by `MqttBroker` beyond the substrate's own Iroh/HTTP/gateway
  surfaces — namespace isolation is enforced entirely by the host-side
  topic-prefixing wrapper in "Broker Embedding" above, and a bare MQTT
  listener would let any local process bypass it by speaking MQTT directly
  to `rumqttd` and subscribing to `svc/#`.

### Tests

(kept to 1-2 basic-path tests per API, not exhaustive variation coverage —
see Slice 7's note on the same principle)

- [x] Unit: `publish` + `subscribe` on same topic delivers message (via
  `MqttBroker`'s own API, not through Wasmtime).
- [x] Unit: wildcard `sensors/+/temp` matches `sensors/room1/temp`.
- [x] Unit: retained message delivered to subscriber joining after publish.
- [x] Unit: `CancellationToken` terminates broker task on `Drop` within 1
  second (no leak).
- [x] Unit: subscribe-topic disambiguation — a topic starting with `svc/` is
  taken literally; any other topic is prefixed with the caller's own
  `svc/<service_id>/`.
- [x] Integration: two WASM components in different services exchange a
  message (guest-to-guest), using the fully-qualified cross-service topic.
- [x] Integration: substrate restart replays `messaging_subscriptions` and a
  previously-subscribed guest still receives a post-restart publish.
- [x] Integration: `undeploy` removes a service's subscriptions; a publish
  to that topic after undeploy is not delivered to (and doesn't error on)
  the now-undeployed service.
- [x] Integration (native dispatch, in-process): deploy a service,
  `publish` via `SynSvcNativeService` with no WASM component involved —
  mirrors M03-sss Slice 5's
  `test_native_dispatch_data_layer_and_blob_store_round_trip`. Include the
  previously-missing `vault`/`app-config` round-trip tests here too.
- [x] Integration (real client, end-to-end, 1 test): a `SyneroymClient`
  connects over a live substrate/Iroh connection, calls
  `SyneroymClient::subscribe` on a topic, then `publish`es to that same
  topic from a second connection (or a WASM guest) and asserts the first
  client's `MessageStream` receives it. First end-to-end test in the repo
  to exercise a native-capability interface through a real `SyneroymClient`
  connection (existing e2e coverage only reaches the toy `greeter`
  interface), and the first to exercise push delivery to a non-WASM caller.
- [x] Integration: a native subscriber that closes its stream is
  unsubscribed and stops receiving messages (proves the close-as-unsubscribe
  rule, not just that subscribe works).
- [x] Integration: MQTT topic namespacing — service A cannot receive
  messages published in service B's namespace without the explicit
  fully-qualified `svc/<other>/...` opt-in.
- [x] Integration: channel backpressure — when the bounded channel is
  saturated, `publish` returns the backpressure error without blocking or
  crashing the substrate.
- [x] New test fixture (Finding B5): a `messaging-pubsub-test` WASM
  component under `test-components/`, exporting `guest-api::handle-message`
  and a small `test-driver` interface (record received messages so
  integration tests can assert on delivery), mirroring
  `test-components/data-layer-test/`'s structure.

### Acceptance Criteria

- `publish`/`subscribe` round-trip works within a single service.
- Wildcard topics and retained messages function correctly.
- Broker task terminates cleanly on substrate shutdown.
- Cross-service MQTT namespace isolation enforced; no unauthenticated
  network listener exists for the broker.
- Guest subscriptions and their `messaging_subscriptions` rows survive a
  substrate restart; `undeploy` cleans them up.
- `syneroym:messaging@0.1.0` WIT compiles with pub/sub fully wired; no
  streaming surface is declared yet (see "WIT Boundary Versioning").
- `messaging/publish` is dispatchable natively (non-WASM) for every
  deployed service; `messaging/subscribe` delivers real push notifications
  to a live `SyneroymClient` over its existing connection, proven by one
  real `SyneroymClient`-over-the-wire test.

---

## Slice 6B: Bidirectional Streaming

**Requirement IDs:** `[PLT-DAP-06]`
**ADR references:** New design note/ADR required before implementation (Dependency Gate 4).
**Depends on:** Slice 6A complete (Dependency Gate 3).

### Design Note Requirements (must exist in `docs/decisions/` before any code here)

The note must specify, at minimum:

1. **Direction disambiguation.** How the host distinguishes a download
   request (route to `handle-stream-request`) from an upload push (route to
   `accept-stream-upload`) on the same registered protocol namespace — e.g.
   a direction flag or small header in the peer's initial frame. Decided
   and documented once, not per-implementation.
2. **Guest-implemented resource mechanics (Finding A6).** `stream-cursor`
   and `stream-sink` are **guest-implemented** WIT resources — the host
   calls methods *on* an instance the guest returned, which is the reverse
   of every existing resource in this codebase. `blob-writer`/`blob-reader`
   (`crates/data_blob`, M03-sss Slice 5) are **host-implemented**
   resources the guest calls into via `with:` in
   `crates/wit_interfaces/src/host.rs`'s `bindgen!` — a real but partial
   precedent (it proves custom resources work with the pinned
   wasmtime/wit-bindgen toolchain at all) but not a template for this
   direction. The note must pick and justify one of:
   - Typed `bindgen!` *export* bindings for the resource methods (if the
     pinned wasmtime/wit-bindgen version supports exported resources
     cleanly), or
   - Dynamic invocation via `Val::Resource` handles and
     `[method]stream-cursor.next-chunk`-style export name lookup (the
     `get_wasm_func`/`Val`-construction pattern already used by
     `invoke_lifecycle_hook`, generalized to resource methods).
3. **Instance lifetime and quota handling (Finding A7).** Every existing
   invocation gets a fresh `Store` with a fixed epoch deadline
   (`store.set_epoch_deadline(50)`, i.e. 5s wall-clock — see
   `AppSandboxEngine::build_store_and_instantiate`,
   `crates/sandbox_wasm/src/engine.rs`) and one fixed fuel budget. A
   multi-minute file transfer held open across many `next-chunk`/
   `push-chunk` calls on the *same* instance will trap on epoch deadline
   and/or exhaust fuel mid-stream unless the note specifies: whether the
   epoch deadline is re-armed per chunk call (and how, given
   `increment_epoch()` is currently a periodic global tick — see
   `AppSandboxEngine::init`), a fuel policy for long-lived streams (refill
   per chunk vs. a streaming-specific exemption/budget), a cap on
   concurrent open stream instances per service (each one holds a live
   `Store`/instance and its guest-side state in memory), and what happens
   to in-flight streams on `stop_wasm`/undeploy. Note that while streaming
   is universal below the WASM boundary, the sandbox lifecycle forces these
   explicit long-lived exemptions or push-model callbacks across it.
4. **Peer-kind symmetry.** Confirm that `register-stream-protocol` accepts
   streams from either a WASM-hosted service or a plain
   `SyneroymClient`/`roymctl` peer with no special-casing (the QUIC stream
   itself is the initiator's channel either way) — expected to fall out of
   the routing design for free; the note should state why.
5. **Where the routing lives.** Confirmed by this review: the natural
   integration point is `handle_raw_stream`'s `ServiceStage::WasmComponent`
   arm in `crates/router/src/route_handler/io.rs` (today's `TODO(wRPC)`
   placeholder that logs and drops the stream) plus a preamble
   scheme/protocol addition — not a new top-level entry point. State
   whether any `crates/coordinator_iroh` changes are also needed for the
   relay-forwarding path (`handle_stream`'s community-registry-miss branch
   already blind-pipes unrecognized-locally streams to the next hop; this
   should compose without changes, but confirm).

### Tasks (once the design note is accepted)

**Stream Protocol Registration:**

- [ ] Add `register-stream-protocol(protocol: string) -> result<_, string>`
  to `host-api`. Wire in `engine.rs`: records `(service_id, protocol)` in a
  host-side routing table.
- [ ] Persist registrations the same way as Slice 6A's
  `messaging_subscriptions` (Finding A1 applies identically here — a
  `stream_protocol_registrations` table, replayed on startup, cleaned up on
  undeploy). Decide and document whether duplicate registration for the
  same service is idempotent or a structured error.

**Inbound Stream Routing (new host infrastructure):**

- [ ] Implement acceptance of peer-initiated QUIC streams against a
  registered protocol namespace per the design note.
- [ ] Route an accepted download-request stream's initial request payload
  to `guest-api::handle-stream-request(peer-id, request-data)`, and an
  accepted upload stream's initial metadata to
  `guest-api::accept-stream-upload(peer-id, metadata)`.

**Guest-as-Source (Host Pull Loop):**

- [ ] Add `stream-types` interface with
  `resource stream-cursor { next-chunk: func() -> result<option<list<u8>>, string>; }`
  — note the `result<...>` wrapper (the original draft's
  `option<list<u8>>` had no way to signal guest-side failure distinct from
  clean EOF; fixed here since this WIT is being written fresh in this
  slice anyway, per "WIT Boundary Versioning").
- [ ] Add `handle-stream-request(peer-id: string, request-data: list<u8>) ->
  result<stream-cursor, string>` to `guest-api`.
- [ ] Implement the guest-side resource per the design note's mechanics
  decision, and the host-side pull loop: a Tokio task that calls
  `next-chunk()` in a loop, transmits each chunk over the Iroh QUIC stream,
  and stops on `Ok(None)` (EOF) or `Err` (abort), closing the stream and
  dropping the resource handle either way.
- [ ] Apply backpressure consistent with the QUIC transport's own flow
  control where available. This is a distinct, simpler interface from any
  future Arrow/DataFusion-style data streaming primitive — no record
  batches here.

**Guest-as-Sink (Host Push Loop):**

- [ ] Add `resource stream-sink { push-chunk: func(data: list<u8>) -> result<_, string>; finalize: func() -> result<_, string>; }`
  to `stream-types`.
- [ ] Add `accept-stream-upload(peer-id: string, metadata: string) ->
  result<stream-sink, string>` to `guest-api`.
- [ ] Implement the guest-side resource and the host-side push loop: a
  Tokio task reads chunks off the incoming Iroh QUIC stream and
  synchronously calls `push-chunk(data)` for each one; when the QUIC stream
  closes (peer signals EOF), the host calls `finalize()` so the guest can
  commit its write (e.g. flush a `data-layer`/`blob-store` write session)
  and release state.
- [ ] `push-chunk` returning `Err` aborts the upload: the host stops
  reading from the QUIC stream, does **not** call `finalize`, and
  resets/closes the stream so the peer observes a clean failure rather than
  a hang.
- [ ] A guest that declines the upload (`Err` from `accept-stream-upload`)
  causes the host to close the incoming QUIC stream immediately, without
  creating a `stream-sink` or reading any payload bytes.
- [ ] **Consolidate chunk-transfer core:** Share the host-side push/pull loop
  implementation between these Slice 6B stream resources and `blob-store`'s
  `UploadSession`/`DownloadSession`. Do not maintain two parallel chunking
  loops for what is fundamentally the same "push `Vec<u8>` chunks until EOF" mechanism.

### Tests

- [ ] Unit: `register-stream-protocol` records the namespace; duplicate
  registration behavior matches the design note's decision.
- [ ] Unit: `stream-cursor.next-chunk()` returning `Ok(None)` closes the
  QUIC stream and drops host-side state (no leak); returning `Err` also
  closes the stream and drops state (aborts, doesn't hang).
- [ ] Unit: `stream-sink.finalize()` is called exactly once, only after the
  QUIC stream closes cleanly (not on abort).
- [ ] Unit: `push-chunk` returning `Err` aborts the upload without invoking
  `finalize`; host-side state is dropped (no leak).
- [ ] Unit: a long-lived stream instance's epoch deadline is re-armed
  across chunk calls per the design note's quota policy (prove a
  multi-chunk transfer exceeding the default 5s single-invocation deadline
  does not spuriously trap).
- [ ] Integration: substrate restart replays `stream_protocol_registrations`.
- [ ] Integration: two WASM components in different services exchange a
  file-transfer-style byte stream end to end via `handle-stream-request`/
  `stream-cursor` (mirrors the design note's worked example).
- [ ] Integration: two WASM components in different services exchange a
  file-transfer-style byte stream end to end via `accept-stream-upload`/
  `stream-sink` (upload direction).
- [ ] Integration: a guest that does not export `handle-stream-request`
  causes the host to reject an inbound download request cleanly (no panic,
  no hang); a guest that does not export `accept-stream-upload` does the
  same for an inbound upload.
- [ ] Integration: cross-service stream namespace isolation — a peer cannot
  address another service's registered protocol without going through the
  substrate's routing.
- [ ] Integration (real client, end-to-end, 1 test covering both
  directions): a `SyneroymClient`/`roymctl` peer (not a WASM-hosted
  service) opens a QUIC stream directly against a deployed WASM service's
  registered protocol, for download and upload — proving the stream
  initiator doesn't need to be another service. No new client-side
  plumbing needed, unlike Slice 6A's `subscribe` — a stream initiator just
  opens a raw bidirectional stream via `SyneroymClient::connection()`,
  already exposed today.
- [ ] New test fixtures (Finding B5): guest-as-source and guest-as-sink
  WASM test components under `test-components/` (may be one component
  exporting both, or two) exercising `handle-stream-request`/
  `accept-stream-upload`.

### Acceptance Criteria

- `register-stream-protocol` → peer opens matching QUIC stream →
  `handle-stream-request` is invoked → returned `stream-cursor` is pulled by
  the host until EOF → QUIC stream closes cleanly, end to end.
- `register-stream-protocol` → peer opens matching QUIC stream to upload →
  `accept-stream-upload` is invoked → returned `stream-sink` is pushed into
  by the host until the peer closes the stream → `finalize()` is called
  exactly once → QUIC stream closes cleanly, end to end.
- No panics or hangs when a guest declines a stream request/upload or fails
  to export `handle-stream-request`/`accept-stream-upload`.
- Both directions work identically whether the initiating peer is another
  WASM-hosted service or a plain `SyneroymClient`/`roymctl` connection —
  verified by at least one test per direction using a real client.
- An aborted upload (`push-chunk` error) never calls `finalize` and leaves
  no dangling host-side state.
- Stream protocol registrations survive a substrate restart; `undeploy`
  cleans them up.
- No trap from epoch/fuel exhaustion on a legitimately long-running,
  actively-progressing stream.

---

## Slice 7: HTTP Passthrough

**Requirement IDs:** `[PLT-DAT]`, `[PLT-DAP-04]`, `[PLT-DAP-06]` (HTTP-facing translation of data-layer, blob-store, and messaging)
**Depends on:** Slice 6B complete (Dependency Gate 5) — needs both pub/sub and streaming.

### Architecture (Corrected — Finding A8)

The original M03-sss draft of this slice claimed `crates/client_gateway` "is
the only component that terminates a real browser/JS SSE or WebSocket
connection" and that no substrate node ever speaks HTTP/SSE to a browser
directly. **That is not what the code does today.** Reading
`crates/client_gateway/src/gateway.rs`: `handle_connection` parses only
enough of the HTTP request (via `httparse`) to read the `Host` header and
pick a target service, then calls
`SyneroymClient::passthrough_with_conn`, which does a raw
`tokio::io::copy_bidirectional` between the inbound TCP socket and the Iroh
QUIC stream. The gateway is a **byte tunnel that sniffs one header**, not
an HTTP server. The component that actually parses and serves HTTP is the
**target node's own hyper server**,
`RouteHandler::handle_http_stream`/`HttpHandler` in
`crates/router/src/route_handler/http.rs` — it runs on whichever substrate
node hosts the target service, fed bytes that arrived over an Iroh-tunneled
stream from the gateway (or from anywhere else; `handle_http_stream` is
generic over any `AsyncRead + AsyncWrite`).

**Consequence for this slice's design:** implement SSE at the target node's
hyper server (`route_handler/http.rs`), not in `client_gateway`. This
reuses the existing byte tunnel unchanged (the gateway keeps doing exactly
what it does today — sniff `Host`, pipe bytes) and means the SSE endpoint
gets `MqttBroker::subscribe` in-process, on the same node, for free — no
new cross-node plumbing. It also means the **same streaming-response-body
work Slice 7 needs for SSE is required anyway** for large blob `GET`s and
chunked uploads, since `HttpHandler`'s responses are currently fully
buffered (`Response<Full<Bytes>>`) — one piece of new infrastructure serves
three of this slice's tasks, not three separate ones.

**Scope note:** WebSocket upgrade is **out of scope** for this slice. The
original draft's "SSE (or WebSocket upgrade)" parenthetical was unexamined
scope; SSE alone (a plain long-lived `GET` with
`Content-Type: text/event-stream`) covers the messaging-subscription use
case and is far simpler to implement on top of a raw byte tunnel than a
WebSocket handshake. Revisit WebSocket as a follow-up if a concrete need
arises.

### Interim Security Note (Finding B9)

M4's IAM/UCAN access control is not yet built. Until then, this slice
exposes `data-layer::put`/`patch` and `messaging::publish` over plain HTTP
from any process that can reach the gateway's loopback port (`127.0.0.1`
only, per `client_gateway`'s existing bind — see `gateway.rs`'s `run()`).
This is judged acceptable for M3C because the gateway is not exposed beyond
localhost by default, but it should be **stated explicitly** as an interim
posture rather than left implicit, so it's visible when M4 IAM planning
happens and so nobody mistakes "works over HTTP" for "safe to expose
publicly" before M4 lands. No code task follows from this — it's a
decision to record in `status.md` at slice close.

### HTTP Verb / Path Routing (target node, `crates/router/src/route_handler/http.rs`)

- [ ] Extend `HttpHandler` to support **streaming response bodies**
  (replace or augment `Response<Full<Bytes>>` with a streaming body type
  hyper supports, e.g. `http_body_util::StreamBody`), the shared
  prerequisite for SSE, large blob `GET`, and chunked upload below.
- [ ] Define **where routes are declared** (open design question the
  original plan left unaddressed — Finding B8): the simplest option
  consistent with this codebase's existing `ServiceManifest`-driven
  configuration is a new optional `[services.<id>.http]` route table in the
  manifest (method + path pattern → target interface + operation), rather
  than a global substrate-wide policy; decide and document at
  implementation time, but it must be per-service, not global, since
  different services expose different data-layer collections/messaging
  topics.
- [ ] Define the `data-layer-error`/`blob-error`/`messaging-error` → HTTP
  status code mapping once, in one place (e.g.
  `permission-denied` → 403, `collection-not-found`/`not-found` → 404,
  `schema-violation` → 400, `quota-exceeded` → 429, `internal` → 500),
  reused by every route below instead of each handler inventing its own.
- [ ] `GET` → `data-layer::get`/`query` (DB access via REST-like
  conventions) or `blob-store::get-blob`/streamed `blob-reader` (signed-URL
  blob serving, static file access) depending on route configuration.
- [ ] `POST`/`PUT` (small body) → `data-layer::put`/`patch` or
  `messaging::publish`.
- [ ] `PUT`/chunked upload (large body) → `messaging::accept-stream-upload`/
  `stream-sink`: the router treats the HTTP body as an inbound stream,
  translating chunked-transfer-encoding reads into `push-chunk` calls and
  end-of-body into `finalize()`. Where the upload target is specifically a
  blob (not a guest-defined sink), the router may instead call
  `blob-store`'s existing `blob-writer` directly — decide per-route via the
  same route configuration as above, not a global policy.
- [ ] `GET /blobs/<hash>?svc=<service_id>&exp=<unix-ts>&sig=<hmac>` resolves
  the existing `signed_url`/`verify_signed_url` logic
  (`crates/data_blob/src/crypto.rs`) — note the query parameter set
  includes `svc` (the original draft's HTTP task description omitted it;
  `sign_url`'s actual output format is
  `blobs/<hash>?svc=<service_id>&exp=<exp>&sig=<sig_hex>`, and `svc` is
  required to resolve the per-service DEK-derived HMAC key — see Finding B2).

### SSE Bridge (target node, same `route_handler/http.rs`)

- [ ] `GET` with `Accept: text/event-stream` on a route mapped to a
  messaging topic: the handler calls `MqttBroker::subscribe` in-process
  (the same broker API Slice 6A's native push-delivery path uses,
  reused directly — no new subscription mechanism), and re-emits each
  received message as an SSE `data:` frame for the life of the HTTP
  connection, using the streaming response body from the task above.

### Tests

- [ ] Integration: `GET /blobs/<hash>?svc=...&exp=...&sig=...` resolves the
  signed-URL logic end to end over a live HTTP endpoint — closes the gap
  explicitly left open in M03-sss Slice 5's "HTTP Serving" deferral.
- [ ] Integration: `GET` static file passthrough serves blob content with
  correct `Content-Type`/`Content-Length`.
- [ ] Integration: `POST` JSON body passthrough performs a `data-layer::put`
  and returns the resulting record.
- [ ] Integration: SSE `GET` receives messages published via
  `messaging::publish` from another connection.
- [ ] Integration: chunked `PUT` upload round-trips through
  `accept-stream-upload`/`stream-sink` with content integrity verified end
  to end (HTTP client → router → guest `finalize()`).
- [ ] Integration: a guest that declines the upload (`Err` from
  `accept-stream-upload`) surfaces as a structured `4xx`, not a hung
  connection or an unexplained `5xx`.
- [ ] Integration: malformed or oversized request rejected with a
  structured HTTP error, not a panic.
- [ ] Integration: the error-mapping table above is exercised for at least
  one case per source error type (`permission-denied`, `not-found`,
  `schema-violation`, `quota-exceeded`, `internal`).

### Acceptance Criteria

- Signed-URL blob serving and static file access work over plain HTTP
  `GET`, closing the deferral from M03-sss Slice 5.
- `data-layer` and `messaging` are reachable over HTTP using conventional
  verbs, without requiring a JSON-RPC envelope.
- Chunked HTTP upload is wired end-to-end onto `stream-sink`/
  `accept-stream-upload` (or `blob-store`'s `blob-writer` for blob-typed
  routes).
- SSE subscription works end to end through the target node's hyper
  server, reusing `client_gateway`'s existing byte-tunnel unchanged.
- No regression to the existing JSON-RPC-over-POST native-dispatch path.
- The interim HTTP-write security posture is recorded in `status.md`.

---

## Reference Scenario: Messaging, Streaming, and HTTP (continues from M03-sss step 13)

> M03-sss's own reference scenario ends at step 13 (blob store, Slice 5).
> This scenario continues with the same numbering.

**M3B extension** (messaging pub/sub):

14. Service publishes messaging event `profiles/updated` with the record ID
    (`syneroym:messaging` `host-api::publish`).
15. A second test service subscribes to the **fully qualified** topic
    `svc/<first-service-did>/profiles/+` (per the namespacing rule in Slice
    6A — a bare `profiles/+` subscribe would resolve to the *subscriber's
    own* namespace and never see the first service's publish; the original
    scenario draft had this bug — see Finding B1) and receives the event
    via `guest-api::handle-message`, then reads the blob from step 13 by
    hash.

**M3C extension** (bidirectional streaming + HTTP):

16. Service registers a stream protocol (`host-api::register-stream-protocol("file-transfer")`).
17. A second test service opens a direct stream and requests the blob from
    step 13 by hash; the first service's `handle-stream-request` returns a
    `stream-cursor` that the host pulls until EOF, delivering the blob
    bytes over a direct QUIC stream (not through `data-layer`/`blob-store`).
18. A third test service uploads a new file to the first service by opening
    a direct stream on the same `"file-transfer"` namespace; the first
    service's `accept-stream-upload` returns a `stream-sink`, the host
    pushes the uploaded bytes via `push-chunk`, and `finalize()` commits the
    file — verified by the first service reading it back afterward.
19. An external HTTP client performs
    `GET /blobs/<hash>?svc=...&exp=...&sig=...` against the signed URL from
    step 13 and receives the raw blob bytes; a second HTTP client opens an
    SSE connection and receives the `profiles/updated` event from step 14
    pushed live; a third HTTP client performs a chunked `PUT` upload that is
    bridged onto `accept-stream-upload`/`stream-sink` the same way as step 18.

---

## Failure and Security Tests

| Test | Expected Outcome |
|---|---|
| Service A publishes to service B's MQTT namespace | Delivery blocked by namespace isolation |
| A process on the local machine speaks raw MQTT to the broker's port | N/A — no such port exists (see Finding A5); if this changes later, this row must be revisited |
| Substrate restarts with active subscriptions/stream registrations | All are replayed from `substrate.db`; no manual re-subscribe needed |
| Peer opens a stream against an unregistered protocol namespace | Host rejects the stream cleanly; no panic, no hang |
| Guest declines a `handle-stream-request` (returns `Err`) | Host closes the QUIC stream without invoking `next-chunk` |
| Guest declines an `accept-stream-upload` (returns `Err`) | Host closes the incoming QUIC stream without creating a `stream-sink` or reading payload bytes |
| `push-chunk` returns `Err` mid-upload | Upload aborted; `finalize()` never called; host-side state dropped; peer observes a clean failure, not a hang |
| `stream-cursor.next-chunk()` returns `Err` mid-download | Host stops pulling, closes the stream cleanly; no leaked instance |
| A long-running stream exceeds the default single-invocation epoch deadline while still making progress | No spurious trap — quota policy from the Slice 6B design note re-arms the deadline |
| HTTP request with tampered or expired signed-URL query params | `4xx` structured error; blob not served |
| Native subscriber's QUIC stream dies without a clean unsubscribe | Broker-side subscription is dropped via `SendStream::stopped()`, not leaked |

---

## Performance Budgets

| Metric | Budget | Measurement Method | Note |
|---|---|---|---|
| MQTT `publish` → native-subscriber delivery (same process, no guest invocation) | < 5 ms p99 | Integration test | Broker → `mpsc` → writer task path only. |
| MQTT `publish` → guest `handle-message` delivery (same process) | < 25 ms p99 | Integration test | Split from the native-subscriber budget (Finding B4) — includes a fresh `Store`/instance per delivery; M03-sss Slice 3A's own bench measured ~16-18 ms just for lifecycle-hook instantiation, so a 5 ms target here was never achievable once guest delivery goes through Wasmtime. |
| `stream-cursor.next-chunk()` round trip (host pull, same process) | < 5 ms p99 | Integration test | |
| `stream-sink.push-chunk()` round trip (host push, same process) | < 5 ms p99 | Integration test | |
| HTTP `GET` signed-URL blob serve (1 MB) | < 100 ms p99 | Integration test | |
| HTTP chunked `PUT` upload (1 MB, via `stream-sink`) | < 150 ms p99 | Integration test | |

---

## Tests Summary

### Unit Tests (adjacent to implementation crates)

- `crates/mqtt-broker/src/tests.rs` — publish/subscribe, wildcards,
  retained, cancellation, subscribe-topic disambiguation.
- `crates/mqtt-broker/src/tests.rs` (Slice 6B additions) or a new
  `crates/messaging-stream/src/tests.rs` — `register-stream-protocol`,
  `stream-cursor` lifecycle/EOF/error handling, `stream-sink`
  `push-chunk`/`finalize` lifecycle and abort-on-error handling.

### Integration Tests (`crates/*/tests/`)

- `mqtt_exchange.rs` — two WASM services publish/subscribe across MQTT
  (guest-to-guest), plus native-dispatch (in-process) `publish` with no
  WASM component involved, plus restart-replay and undeploy-cleanup cases.
- `messaging_client_e2e.rs` (in `crates/substrate/tests/`) — a real
  `SyneroymClient` calls `SyneroymClient::subscribe` on a topic over the
  wire, then a publish (from a second connection or a WASM guest) is
  asserted to arrive on the client's `MessageStream`.
- `stream_exchange.rs` — two services exchange a file-transfer-style byte
  stream via `handle-stream-request`/`stream-cursor` (download) and via
  `accept-stream-upload`/`stream-sink` (upload); repeated with a
  `SyneroymClient`/`roymctl` peer as the stream initiator instead of a
  second WASM service, both directions.
- `http_passthrough.rs` — signed-URL blob GET, JSON POST to data-layer, SSE
  subscription to messaging, chunked upload round trip via `stream-sink`.

### End-to-End Tests (extending `mise run test:e2e`)

- Reference scenario steps 14-19 in a live substrate instance.
- All failure/security tests produce documented outcomes.

---

## Measurable Exit Criteria

### Slice 6A Exit Criteria (M3B messaging)

- [x] `cargo +nightly fmt --all` passes with zero diff.
- [x] `cargo clippy --workspace --all-targets --all-features` passes with
  zero warnings and zero errors.
- [x] `cargo test --workspace` passes with all tests green.
- [x] `mise run test:e2e` passes (existing e2e scenarios must not regress).
- [x] `syneroym:messaging@0.1.0` (pub/sub surface only) WIT compiles and
  generates valid Rust bindings.
- [x] `messaging` is dispatchable natively (non-WASM) via `SynSvcNativeService`
  for every deployed service, with a passing `SyneroymClient`-over-the-wire
  subscribe test — not just an in-process dispatch-registry test.
- [x] Guest subscriptions survive a substrate restart; `undeploy` cleans up
  subscriptions.
- [x] No unauthenticated broker network listener exists.
- [x] Reference scenario steps 14-15 execute without error.
- [x] All Slice 6A failure/security test rows produce documented outcomes.
- [x] Performance budgets for the two MQTT delivery rows verified; output
  captured in `status.md`.

### Slice 6B Exit Criteria

- [ ] Design note/ADR for host-side QUIC stream acceptance/routing recorded
  in `docs/decisions/` before Slice 6B implementation began (Dependency
  Gate 4).
- [ ] `syneroym:messaging@0.1.0` streaming surface — both guest-as-source
  (`stream-cursor`, `handle-stream-request`) and guest-as-sink
  (`stream-sink`, `accept-stream-upload`), plus `register-stream-protocol`
  — is fully wired; WIT package compiles.
- [ ] Guest-as-sink upload direction is implemented and documented.
- [ ] Stream protocol registrations survive a substrate restart; `undeploy`
  cleans them up.
- [ ] Long-running streams don't spuriously trap on epoch/fuel exhaustion
  per the design note's quota policy.
- [ ] Reference scenario steps 16-18 execute end-to-end without error.
- [ ] All Slice 6B failure/security test rows produce documented outcomes.
- [ ] Performance budgets for Slice 6B metrics verified; output captured in
  `status.md`.
- [ ] `cargo +nightly fmt --all`, `cargo clippy --workspace --all-targets
  --all-features`, `cargo test --workspace`, and `mise run test:e2e` all
  pass.

### Slice 7 Exit Criteria

- [ ] `crates/router/src/route_handler/http.rs` serves `GET`/`POST`/streaming
  `PUT` against `data-layer`, `blob-store`, and `messaging` without a
  JSON-RPC envelope, alongside the existing JSON-RPC-over-POST path
  (no regression).
- [ ] SSE subscription is served from the target node's hyper server (not
  `client_gateway`), reusing `client_gateway`'s byte tunnel unchanged.
- [ ] Signed-URL blob GET works over a live HTTP endpoint, closing the
  M03-sss Slice 5 deferral.
- [ ] Chunked upload is wired end-to-end onto `stream-sink`/
  `accept-stream-upload` or `blob-store`'s `blob-writer`.
- [ ] Interim HTTP-write security posture recorded in `status.md`.
- [ ] Reference scenario step 19 executes end-to-end without error.
- [ ] All Slice 7 failure/security test rows produce documented outcomes.
- [ ] Performance budgets for Slice 7 metrics verified; output captured in
  `status.md`.
- [ ] `cargo +nightly fmt --all`, `cargo clippy --workspace --all-targets
  --all-features`, `cargo test --workspace`, and `mise run test:e2e` all
  pass.
- [ ] Traceability matrix updated with evidence for `[PLT-DAP-06]`
  (bidirectional streaming) and the HTTP-facing sub-requirements of
  `[PLT-DAT]`/`[PLT-DAP-04]`.

---

## Review Findings Incorporated

A detailed pre-implementation review of the original Slice 6A/6B/7 plan (as
it stood inside `M03-sss/task.md`) surfaced the following. Recorded here for
traceability, per this project's convention of documenting scope
decisions rather than silently absorbing them.

**Incorporated as designed above:**
- **A1** — Subscription/stream-protocol persistence and restart-recovery
  were architecturally asserted (`system-architecture.md` §2: "Subscriptions
  are persisted by the host") but never operationalized as a task. Added
  `messaging_subscriptions`/`stream_protocol_registrations` tables,
  startup replay, and undeploy cleanup to both slices.
- **A2** — Native push delivery can't be expressed by the existing
  `NativeService::dispatch` single-request/response shape, and naively
  reusing `SyneroymClient::request_raw`'s `send.finish()` pattern would
  kill the subscription immediately. Added a full design (router
  special-casing, writer task, mpsc bridge, close-as-unsubscribe,
  `SendStream::stopped()`-driven cleanup) to Slice 6A.
- **A5** — A real MQTT network listener would let any local process bypass
  the host-enforced namespace isolation entirely. Dropped the `[mqtt]
  bind_addr` listener from scope; broker is reachable only via in-process
  links.
- **A6** — `stream-cursor`/`stream-sink` are guest-implemented resources,
  the reverse of the `blob-writer`/`blob-reader` precedent the original
  plan cited; corrected the framing and made the mechanics question an
  explicit, required part of the Slice 6B design note. Also added a
  result-wrapped `next-chunk` and a direct-`Val` invocation helper task for
  `handle-message` (bypassing `conversions.rs`'s `String`/`U32`/`Bool`-only
  limitation).
- **A7** — Long-lived stream instances collide with the fixed
  epoch-deadline/fuel-per-invocation model. Made quota handling an explicit,
  required part of the Slice 6B design note, plus a dedicated test.
- **A8** — Corrected the factual claim that `client_gateway` terminates
  SSE/WebSocket connections; it's a byte tunnel, and the target node's
  hyper server is the actual HTTP terminator. Moved the SSE implementation
  there, which also unlocks streaming responses for large blob `GET`s and
  chunked uploads as a shared prerequisite. Scoped WebSocket out.
- **B1** — Fixed the reference scenario's step 15: subscribing to a bare
  `profiles/+` from a different service resolves to the *subscriber's own*
  namespace under the stated namespacing rule and would never see the
  first service's publish. Scenario now uses the fully-qualified topic.
- **B2** — Corrected the signed-URL query parameter set to include `svc`,
  matching `sign_url`'s actual output format
  (`crates/data_blob/src/crypto.rs`).
- **B3** — Added the `NATIVE_CAPABILITY_INTERFACES` extension as an
  explicit task, since forgetting it reproduces the exact `endpoint_type`
  flake M03-sss Slice 5 already hit and fixed once.
- **B4** — Split the MQTT delivery performance budget into a
  native-subscriber row (achievable at < 5 ms) and a guest-`handle-message`
  row (< 25 ms, reflecting real per-invocation instantiation cost measured
  in M03-sss Slice 3A's own benchmarks).
- **B5** — Added explicit test-fixture tasks for new WASM test components
  (pub/sub, stream source, stream sink) alongside each slice's test list.
- **B6** — Folded the missing `vault`/`app-config` native-dispatch
  round-trip tests into Slice 6A's native dispatch work.
- **B8** — Added an explicit route-configuration design task (per-service
  manifest section, not global policy) and a single shared error-to-HTTP-
  status mapping table to Slice 7.
- **B9** — Added an explicit interim-security note for HTTP-facing writes
  pre-M4-IAM, to be recorded rather than left implicit.
- **ADR-0010 alignment** — Slice 6A's WIT now uses a structured
  `messaging-error` variant matching ADR-0010's `pubsub-error`, instead of
  `result<_, string>` throughout.

**Considered and explicitly not carried forward, given breaking WIT changes
between slices are acceptable (this package isn't released externally):**
- **A3 (as originally proposed: split `guest-api` into two interfaces in
  6A to avoid forcing streaming stubs)** — resolved more simply: Slice 6A
  just doesn't declare the streaming surface at all. See "WIT Boundary
  Versioning" above. Same underlying concern, cheaper fix, and it also
  removes the "declared but returns not-yet-implemented" placeholder
  machinery and its tests entirely.
- **A4 (as originally proposed: add a `retain` flag to `publish` now, and
  fix `next-chunk`'s missing error channel now, specifically to avoid a
  future breaking change)** — the urgency was entirely about not breaking
  an already-shipped WIT surface later. Since Slice 6A's WIT isn't shipped
  externally and Slice 6B's WIT is written fresh in this document anyway
  (see "WIT Boundary Versioning"), both fixes are simply made directly in
  their owning slice's first draft (`next-chunk`'s `result` wrapper is in
  Slice 6B's WIT above) with no separate "add now to avoid a break later"
  task. A `retain` parameter on `publish` is deliberately **not** added in
  Slice 6A, since retained-message *support* is delivered via the broker
  wrapper (see the day-1 spike), not via an API parameter guests set per
  call in this design (`rumqttd`'s own `LinkTx::publish` has no retain
  parameter either — see the spike). If a future need arises for guests to
  control retention per-message, add the parameter then; it's a one-line
  addition, not a redesign.
