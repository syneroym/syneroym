# ADR 0014: Host-Side QUIC Stream Protocol Routing (Slice 6B)

## Status

Accepted.

## Context

Slice 6B (`[PLT-DAP-06]`) adds generic bidirectional streaming to
`syneroym:messaging`: a WASM guest registers a named "stream protocol", and a
peer (another WASM service, a `SyneroymClient`, or `roymctl`) opens a direct
QUIC stream against it, either pulling from the guest (`stream-cursor`,
guest-as-source) or pushing into it (`stream-sink`, guest-as-sink).
`docs/planning/milestones/M03B-messaging/task.md`'s Dependency Gate 4 requires
this design note to exist, covering direction disambiguation, guest-implemented
resource mechanics, instance lifetime/quota handling, peer-kind symmetry, and
where the routing lives, before any Slice 6B code lands.

A day-0 spike (throwaway, scratchpad-only, not committed) confirmed the one
genuine unknown before this design was finalized: whether dynamic
`Val::Resource(ResourceAny)` invocation works for a guest-**exported** WIT
resource in wasmtime 46.0.1. It does — see "Resource Mechanics" below.

## Decision

### 1. Direction Disambiguation

A `dir=upload|download` query parameter on the `raw://` preamble.
`RoutePreamble` (`crates/router/src/preamble.rs`) already parses arbitrary
query params (`enc`, `pubkey`, `delegation`) before the stream goes fully raw;
`dir` is one more, parsed by `RoutePreamble::parse` and re-emitted by its
`Display` impl. `binary_json_rpc`/`from_http_path` preambles don't carry query
params at all and are unaffected.

The initial request/metadata payload (the download request bytes, or the
upload's metadata string) is read as **one** `syneroym_rpc::framing`
length-prefixed frame immediately after the preamble. After that single frame,
the stream reverts to true unframed raw bytes for chunk transfer, matching
`TransportStage::Raw`'s existing "zero framing" contract for everything else
that flows through it.

The router validates `dir=` **strictly and immediately**, before any WASM
instantiation: a stream that resolves to a stream-protocol route with `dir`
missing or not exactly `upload`/`download` is rejected at the router with a
clean, immediate stream close. This is deliberate — letting an invalid
direction surface later as a WASM-side failure would produce a confusing error
far from its actual cause.

### 2. Resource Mechanics

`stream-cursor` and `stream-sink` are **guest-implemented** resources — the
reverse of `blob-writer`/`blob-reader` (`crates/data_blob`), which are
**host**-implemented and mapped via `with:` in
`crates/wit_interfaces/src/host.rs`'s `bindgen!`. That `with:` mechanism only
applies to resources the *host* implements; it has no equivalent for a
guest-exported resource the host calls into.

**WIT direction is the load-bearing detail here.** In the component model,
whichever side *exports* an interface implements it. Since `stream-cursor`/
`stream-sink` must be guest-implemented, `stream-types` (the interface housing
them) is **exported** by `messaging-guest`, exactly mirroring how `guest-api`
is already exported there — not imported, the way `blob-store` is imported by
`blob-store-guest` (the host-implemented shape). Concretely:

```wit
world messaging-guest {
    import host-api;
    export stream-types;
    export guest-api;
}
```

and in `host.wit`'s `host-environment` world:

```wit
export syneroym:messaging/stream-types@0.1.0;
```

alongside the existing `export syneroym:messaging/guest-api@0.1.0;` line (not
alongside the `import ...blob-store...` line).

**Invocation mechanism**: dynamic invocation via `Val::Resource(ResourceAny)`,
generalizing the `get_wasm_func`/`Val`-construction pattern
`AppSandboxEngine::deliver_message`/`invoke_lifecycle_hook` already use for
plain functions, to resource methods. The day-0 spike proved this end to end
against the pinned `wasmtime = "46.0.1"`:

- A minimal guest world exported an interface with a resource
  (`stream-cursor`-shaped, one method) and a plain function returning an
  instance of it.
- Calling the plain export via `instance.get_export(None, "<interface>")` →
  `get_export(Some(iface_idx), "<fn>")` → `get_func` → `Func::call_async`
  yielded a `Val::Resource(ResourceAny)` in the results — the guest-exported
  resource surfaces to the host exactly like any other `Val` variant, no
  special extraction needed.
- Calling the resource's own method used the **same** `get_export`/`get_func`
  lookup, with the export name `[method]<resource-name>.<method-name>` (e.g.
  `[method]stream-cursor.next-chunk`) resolved against the *interface's*
  export index (not the plain-function path), and the `ResourceAny` passed as
  `Val::Resource(resource_any)` in the **first** position of the call args
  (the implicit receiver). This worked identically across repeated calls
  against the same resource instance, and `ResourceAny::resource_drop_async`
  completed cleanly afterward.
- One incidental finding: `Func::post_return_async` is a deprecated no-op on
  this wasmtime version (calling it is harmless but does nothing) — the
  production code added in this slice does not call it.

No new `with:` entry is needed in `crates/wit_interfaces/src/host.rs`'s
`bindgen!` for `stream-types`; `bindgen!` does not require one for an exported
resource the host only ever touches dynamically.

### 3. Instance Lifetime and Quota

Every existing invocation (`build_store_and_instantiate`) creates a **fresh**
`Store`/`Instance` per call — fine for stateless calls like `handle-message`,
but wrong for a stream: the guest returns a resource from
`handle-stream-request`/`accept-stream-upload`, and every subsequent
`next-chunk()`/`push-chunk()` call must run against the *same*
`Store`/`Instance` that resource lives in, or the `ResourceAny` is meaningless
(referencing a `Store` that error longer exists).

- **One dedicated Tokio task per open stream** owns a single long-lived
  `Store<HostState>` + `Instance`, obtained via a new
  `AppSandboxEngine::open_stream_instance` (parallel to, but distinct from,
  `build_store_and_instantiate`, which remains per-call for every other path).
- **Epoch deadline is re-armed before every `next-chunk`/`push-chunk` call**:
  `store.set_epoch_deadline(50)` (the same 5s-wall-clock budget every other
  invocation gets) is called again immediately before each chunk call, rather
  than once for the whole stream's lifetime. A long-running, actively
  progressing transfer therefore never spuriously traps between chunks; a
  guest that stalls mid-call still traps within one chunk's 5s budget.
- **Fuel is refilled per chunk** from the service's existing quota resolution
  (the same `max_instructions` lookup `build_store_and_instantiate` already
  performs), so a multi-chunk transfer isn't bounded by one call's worth of
  fuel.
- **A per-service cap on concurrent open stream instances** bounds memory,
  since each holds a live `Store` plus the guest's in-memory state:
  `StreamingConfig { max_concurrent_streams_per_service: u32 }` (default `8`)
  is added to `crates/core/src/config.rs`, threaded into `SubstrateConfig` as a
  new `[streaming]` section — mirroring `MessagingConfig`/`[mqtt]`'s existing
  precedent. Opening a new stream instance past the cap is rejected with a
  clean error, not a panic or unbounded queue.
- **Task tracking survives more than the `undeploy` path.** A bare
  `tokio::task::AbortHandle` does nothing on `Drop` — unlike Slice 6A's
  `SubscriptionHandle`, which actively unsubscribes on drop. The per-service
  `DashMap<String, Vec<AbortHandle>>` tracking open stream tasks is wrapped in
  a small `StreamRegistry` struct with its own `Drop` impl that walks and
  aborts every handle. `stop_wasm`/`ControlPlaneService::undeploy` still
  explicitly abort that service's handles (mirroring today's
  `unsubscribe_all`), but the `Drop` impl is the backstop for every other
  teardown path (e.g. the whole `AppSandboxEngine` being dropped at process
  shutdown) — relying solely on the `undeploy` path would leak running stream
  tasks there.
- **Cleanup must not panic on an already-trapped guest.** Every
  `ResourceAny::resource_drop_async` call made from a cleanup/abort path
  ignores its `Result` — a guest whose `Store` already trapped or panicked
  must not cause a panic in the host's own cleanup path.

### 4. Peer-Kind Symmetry

`register-stream-protocol` records `(service_id, protocol)` in
`EndpointRegistry` as a `WasmChannel { service_id }` entry (see "Where
Registration Lives" below). `raw://<protocol>|<service_id>` resolves through
the identical preamble → registry lookup → `plan_pipeline` →
`handle_raw_stream` path regardless of who opened the connection — a WASM
service, `SyneroymClient::connection()` (already returns
`Option<TransportConnection>` today, so no new client-side plumbing is
needed), or `roymctl`. This symmetry is only true once the new `plan_pipeline`
match arm below exists; before it, every `raw://` request against a
`WasmChannel` endpoint falls through to `ServiceStage::Unsupported`
regardless of initiator, so it is not "free" — it depends on item 5.

### 5. Where the Routing Lives

The integration point is `handle_raw_stream`'s `ServiceStage::WasmComponent`
arm in `crates/router/src/route_handler/io.rs` — today a `TODO(wRPC)`
placeholder that logs and drops the stream. It currently receives only
`&RoutePipeline`; it is extended to also take `&RoutePreamble`, since it needs
`preamble.interface` (the requested protocol name) and `preamble.dir`
(direction). Its one call site (`io.rs:171`) already has `preamble` in scope.

**A new `plan_pipeline` match arm is required** (`crates/router/src/
route_handler/dispatch.rs`):

```rust
(RouteProtocol::Raw, SubstrateEndpoint::WasmChannel { service_id }) =>
    (AdaptationStage::None, ServiceStage::WasmComponent { service_id: service_id.clone() }),
```

Today, `(RouteProtocol::Raw, WasmChannel)` has no arm and falls through to
`_ => (AdaptationStage::None, ServiceStage::Unsupported)`; the only existing
path that reaches `TransportStage::Raw` + `ServiceStage::WasmComponent` is via
`wrpc://` (matched via `(RouteProtocol::Wrpc, WasmChannel)`, then
transport force-overridden to `Raw`). Without this new arm, every `raw://`
stream-protocol request dead-ends regardless of everything else in this ADR.

New arm behavior once matched: validate `preamble.dir` (reject cleanly and
immediately if invalid, per item 1); read one framed initial payload
(`framing::read_frame`); call a new
`AppSandboxEngine::handle_stream_protocol_request` with direction + protocol
(`preamble.interface`) + peer-id + the framed payload + the split
reader/writer halves. That call spawns/awaits the pull loop
(`chunk_transfer::pull_until_eof`) for `download` or the push loop
(`push_until_eof`) for `upload`, built on the `GuestStreamCursor`/
`GuestStreamSink` wrappers (owning the long-lived `Store`/`Instance`/
`ResourceAny` from item 3). On guest decline (`Err` from
`handle-stream-request`/`accept-stream-upload`, or the export simply not
existing — checked via `get_wasm_func` returning `Err`, the same "not
exported" handling `deliver_message` already uses), the host closes the
stream immediately without creating a cursor/sink or reading any further
payload bytes.

`crates/coordinator_iroh`'s relay-forwarding path (the local-registry-miss
branch in `handle_stream`, which runs before `plan_pipeline`) is
preamble/transport-agnostic today — it forwards any preamble line and
blind-pipes bytes — and requires no change for this slice; confirmed by
reading it.

### 6. Peer-ID

`preamble.delegation.as_ref().map(|d| d.master_did.clone())` if the
initiator's connection carried a delegation certificate — the same field
Slice 6A's handshake verifier already reads — else a fallback constant. No new
identity-extraction plumbing is introduced.

### 7. Where Registration Lives

`register-stream-protocol` reuses `EndpointRegistry`
(`crates/core/src/local_registry.rs`) rather than a new
`stream_protocol_registrations` table, calling
`EndpointRegistry::register(service_id, protocol, WasmChannel{service_id})`
directly. `EndpointRegistry` already persists every `register()` call to
`endpoints.db` (`SqliteEndpointStorage`) and replays it via `load_from_db()`
at construction time, which happens in `setup_router`
(`crates/substrate/src/runtime.rs`) before the Iroh protocol handler starts
accepting connections — restart-replay is therefore already correct with no
new persistence code. `ControlPlaneService::undeploy` already iterates
`lookup_by_service` and removes every interface a service owns, so undeploy
cleanup also falls out with no new code.

**Caveat**: `SubstrateEndpoint::WasmChannel` carries no interface-kind tag, so
registry resolution alone cannot distinguish a stream-protocol registration
from an ordinary WIT interface a WASM component declared at deploy time
(`register_wasm_endpoints` uses the identical key shape). This is safe in
practice, not free of cost: a `raw://` request against a non-stream interface
name simply finds the guest doesn't export `handle-stream-request`/
`accept-stream-upload` and gets the graceful-decline behavior from item 5 —
**that decline path, not the registry's typing, is the actual safety net.**
Duplicate registration of the same `(service_id, protocol)` is idempotent,
last-write-wins, matching `EndpointRegistry::register`'s existing semantics —
no separate "already registered" error is introduced.

## Two Deliberate Deviations from `task.md`'s Literal Task List

1. **`handle-stream-request`/`accept-stream-upload` gain a `protocol: string`
   first parameter**, vs. task.md's literal `(peer-id, request-data)`/
   `(peer-id, metadata)`. `register-stream-protocol` lets one service register
   multiple named protocols, but there is only one `handle-stream-request`
   export per component; without the protocol name in the call, a guest that
   registers two protocols has no way to tell which one a given request is
   for. `preamble.interface` already carries the protocol name end-to-end
   through registry lookup → `plan_pipeline` → the raw-stream handler, so
   threading it into the guest call is a small, in-pattern addition. Same
   category of pre-approved fix as Slice 6A's `next-chunk` `result<>` wrapper
   (`task.md`'s "WIT Boundary Versioning" section: this WIT package isn't
   released externally, so fixing real gaps discovered while writing it fresh
   in the same slice costs nothing).
2. **No new `stream_protocol_registrations` table** — see "Where Registration
   Lives" above. Functionally equivalent to what `task.md` asks for (survives
   restart, cleaned up on undeploy), with less new code and reuse of
   already-tested infrastructure. Persists to a different file than Slice 6A's
   `messaging_subscriptions` (`endpoints.db` via `SqliteEndpointStorage`, not
   `substrate.db`) — noted here and in `status.md` so it isn't later read as
   an inconsistency.

## Consequences

**Positive:**
- No new top-level entry point or transport concept — reuses the existing
  `raw://` scheme, preamble, and `handle_raw_stream` integration point.
- `EndpointRegistry` reuse means restart-replay and undeploy-cleanup are
  already correct, not new surface to test from scratch.
- The chunk-transfer core (`ChunkSource`/`ChunkSink`) is shared with
  `blob-store`'s upload/download sessions (see `crates/chunk_transfer`),
  avoiding a second parallel chunking loop.

**Negative:**
- Every open stream holds a live `Store`/`Instance` for its duration — a
  meaningfully larger per-stream memory footprint than any other invocation
  path in this codebase, hence the explicit per-service cap.
- The registry-reuse deviation means a stream-protocol registration is not
  distinguishable from an ordinary declared interface by inspection alone;
  this is mitigated, not eliminated, by the graceful-decline path.

## Open Questions

- Whether a future need for guests to signal stream-level backpressure
  explicitly (beyond what QUIC's own flow control provides) requires a WIT
  change is left for a later slice if it arises.
