# Milestone 3B: Messaging, Streaming, and HTTP Bridge - Status Log

> This milestone was split out of `M03-sss` on 2026-07-09 before any of
> Slices 6A/6B/7 were implemented — see the provenance note at the top of
> `task.md`. No completed-slice history to carry over; this log starts
> fresh.

## Slice 6A: Messaging WIT and Embedded Pub/Sub Broker (Complete)

**Implemented by:** Claude Code, Sonnet 5 (`claude-sonnet-5`).

### What was built

- **`crates/mqtt_broker`** (package `syneroym-mqtt-broker`) — new crate wrapping
  `rumqttd` 0.20.0: `MqttBroker` (`new`/`publish`/`subscribe`), `SubscriptionHandle`
  (unsubscribes + stops its forwarding task on `Drop`), `MessagingError`, and the
  pure `namespace_topic` function. 7 unit tests in `src/tests.rs`.
- **`crates/wit_interfaces/wit/messaging/messaging.wit`** — `syneroym:messaging@0.1.0`
  package: `host-api` (`publish`/`subscribe`/`unsubscribe`, `messaging-error` variant)
  and `guest-api` (`handle-message`), `world messaging-guest`. Symlinked into
  `wit/host/deps/messaging` (directory symlink, matching the `app-config`/`blob-store`
  pattern) and wired into `host-environment`'s imports/exports in `host.wit`.
  No standalone `crates/wit_interfaces/src/messaging.rs` bindgen module was added —
  `data_layer.rs`/`blob_store.rs`-style standalone modules turned out to be unused
  outside their own crate (confirmed via repo-wide grep); native-dispatch DTOs are
  hand-written instead, matching `syneroym-data-blob`'s `native_types.rs` precedent.
- **`crates/sandbox_wasm/src/engine.rs`** — `MessagingContext` (bundles
  `broker: Arc<MqttBroker>` and `engine: Weak<AppSandboxEngine>`) added as a new
  `HostState`/`AppSandboxEngine::init` field/param; `impl host_api::Host for HostState`
  (publish/subscribe/unsubscribe); `AppSandboxEngine::{self_weak, subscriptions,
  register_internal_subscription, unsubscribe_all, deliver_message}`; `build_wasm_linker`
  registers `host_api::add_to_linker`.
- **`crates/data_db`** — schema bumped to `"m3b"`; new `messaging_subscriptions` table;
  four new `StorageProvider` methods (`save_messaging_subscription`,
  `delete_messaging_subscription`, `delete_all_messaging_subscriptions_for_service`,
  `list_all_messaging_subscriptions`), implemented on `SqliteStorageProvider` (the only
  implementor).
- **`crates/control_plane`** — `SynSvcNativeService::dispatch_messaging` (publish only);
  `NATIVE_CAPABILITY_INTERFACES` extended to 5 (added `"messaging"`);
  `ControlPlaneService` gained a `messaging_broker` field, threaded through `init`;
  `undeploy` now also clears persisted + live messaging subscriptions for the
  undeployed service.
- **`crates/router/src/route_handler/dispatch.rs`** — `handle_binary_stream` (reads the
  first frame, special-cases `messaging`+`subscribe`) and `handle_messaging_subscribe`
  (acks, then loops forwarding broker messages as `messaging/message` notifications
  until the client's send-side closes or the broker channel closes); `RouteHandlerInner`
  gained a `messaging_broker` field.
- **`crates/sdk`** — `SyneroymClient::subscribe(interface, topic) -> Result<MessageStream>`
  (opens a bidi stream, does *not* finish the send side) and `MessageStream`
  (`.recv()`, `.stop()` — closes just the send half so `.recv()` keeps working and
  observes the resulting close).
- **`crates/core/src/config.rs`** — `MessagingConfig { channel_capacity: u64 }` (default
  1024), `SubstrateConfig.mqtt`.
- **`crates/router/src/route_handler.rs`** — `RouteHandler::init` constructs the
  `MqttBroker`, sets `AppSandboxEngine::self_weak` immediately after `Arc::new`, replays
  every persisted `messaging_subscriptions` row before the router starts accepting
  connections, and threads the broker into `AppSandboxEngine`/`ControlPlaneService`.
- **`test-components/messaging-pubsub-test`** — new WASM test fixture (imports
  `messaging/host-api` + `data-layer/store`, exports `messaging/guest-api` +
  a `test-driver` interface: `subscribe-to`/`publish-to`/`get-received-messages`).
  Persists received messages via `data-layer` (JSON-encoded), not in-guest memory,
  because every host invocation gets a fresh `Store`/instance.
- **`AGENTS.md`** — new rule codifying the crate-naming convention
  (`crates/<snake_case>/` dir, `syneroym-<kebab-case>` package name), called out
  explicitly since `task.md`'s own prose says `crates/mqtt-broker/`.

### Day-1 spike findings (`rumqttd` 0.20.0)

Spiked in a throwaway crate under the session scratchpad (not committed). Findings,
some of which **correct assumptions in this doc's original plan**:

1. **`Broker::new(config)` already starts the router**, as a dedicated native OS
   thread (`std::thread::Builder`), as a side effect of construction. `Broker::link()`
   works immediately — `Broker::start()` is never called and is not needed.
2. **`Broker::start()` must never be called for this deployment shape.** It only
   starts network listeners (v4/v5/ws) and immediately returns
   `Err(Error::Config(...))` if none are configured — exactly ADR-0010's "no
   `bind_addr`" intent, but it means `MqttBroker` must not call `start()` at all,
   not merely leave listener config `None`. This also means the plan's assumption
   that `start()` needs driving via `tokio::task::spawn_blocking` was wrong — no
   `spawn_blocking` is used anywhere in `MqttBroker`.
3. **No clean stop mechanism exists**, confirmed by reading the full `rumqttd`
   source: the `Router` struct holds a permanent internal clone of its own
   event-channel sender (used to reissue links), so the channel can never observe
   "all senders dropped," and no `Shutdown` `Event` variant exists anywhere in the
   crate. **Resolution (the accepted fallback the plan flagged as possible):**
   `MqttBroker::drop` cancels a `CancellationToken` that only governs *this crate's
   own* Tokio tasks (each subscription's forwarding task); the underlying `rumqttd`
   router OS thread is an accepted, harmless leak (parked on a blocking channel
   `recv`, zero CPU use) until process exit. `dropping_broker_terminates_subscription_forwarding_tasks_within_one_second`
   tests exactly this — the forwarding task's closure, not the OS thread's.
4. **Retained messages confirmed working**: `LinkTx::publish`/`try_publish` indeed
   take no `retain` parameter; retaining requires the raw-packet escape hatch —
   `LinkTx::send(Packet::Publish(Publish::new(topic, payload, true), None)).await`.
   A subscriber that joins *after* a retaining publish receives it.
5. **Wildcard `+` subscriptions confirmed working** through `LinkTx::subscribe`.
6. **`Notification::Forward(Forward { publish: Publish { topic, payload, retain, .. }, .. })`**
   is the exact shape for a delivered publish; the router first emits
   `Notification::DeviceAck(Ack::SubAck(..))` immediately after `subscribe()`, so
   consumers must skip non-`Forward` notifications.
7. **The publish-side "channel" ADR-0010 describes is rumqttd's own internal
   event channel, fixed at capacity 1000** (`bounded(1000)` in `Router::new`, not
   exposed via any public config field) — **not** our own `channel_capacity`
   config value. `channel_capacity` (default 1024) governs each *subscription's*
   own forwarding `mpsc` channel instead. Empirically, a tight loop of un-drained
   publishes reliably triggers `try_publish`'s backpressure error at roughly
   message #1000-1012 (confirmed in
   `publish_returns_backpressure_error_when_channel_saturated`, 20k iterations in
   ~40ms). This is the interim decision task.md anticipated needing ("exact
   enforcement point... guided by the spike").

### Other interim decisions

- **`crates/mqtt_broker` vs. task.md's literal `crates/mqtt-broker`**: deliberate
  deviation, per the new AGENTS.md rule — directory names are always snake_case,
  even when planning-doc prose uses a hyphen.
- **`SendStream::stopped()` vs. read-EOF detection for native-subscriber cleanup**:
  ADR-0010/task.md's design specifically names iroh's `SendStream::stopped()` for
  detecting a dead/gone-away subscriber. The actual router-layer code
  (`handle_binary_stream`/`handle_messaging_subscribe`) is generic over
  `W: AsyncWrite + Unpin + Send` (the already-boxed `OwnedStream`, used for both
  Iroh and plain-TCP-backed streams elsewhere in the router), so it has no
  Iroh-specific type to call `.stopped()` on. Implemented instead: a dedicated
  background task loops `framing::read_frame` on the client's read half until
  EOF/error, then signals a `oneshot` the main forwarding loop selects on. This
  is deliberately **not raced per-message-iteration** against the broker
  `mpsc::Receiver` (which would risk losing a partially-read frame on
  cancellation) — it runs to completion on its own task instead. Functionally
  equivalent for the "subscriber went away" case
  `test_native_subscriber_receives_push_delivery_and_close_unsubscribes` exercises
  (drop/`.stop()` the client's send half → server-side cleanup fires), though not
  a literal `SendStream::stopped()` call.
- **`AppSandboxEngine::deliver_message` interface-name bug found and fixed during
  integration testing**: `get_wasm_func`'s `interface_name` argument must be the WIT
  package-qualified string (`"syneroym:messaging/guest-api@0.1.0"`, matching how
  `GREETER_INTERFACE_NAME`/`TEST_DRIVER_INTERFACE` are defined elsewhere in the
  codebase), not the short interface name (`"guest-api"`) — the initial
  implementation used the short name and silently discarded every delivery until
  `test_guest_to_guest_cross_service_message_delivery` caught it.
- **`syneroym:data-layer/store::put` requires a JSON payload** (validated at the
  host boundary) — discovered while building the `messaging-pubsub-test` fixture,
  whose first draft packed `(topic, payload)` into a raw length-delimited byte
  string. `received_messages` rows are JSON-encoded instead
  (`{"topic": ..., "payload": ...}`, payload as UTF-8 text — the fixture only ever
  sends UTF-8 text payloads).
- **`test-driver`'s own WIT surface intentionally avoids `list<u8>`/tuple types**:
  `crates/sandbox_wasm/src/conversions.rs`'s generic JSON-RPC parameter/result
  conversion (used by `execute_wasm`, which every integration test drives guest
  calls through) only supports `string`/`u32`/`bool`. `publish-to` therefore takes
  `payload: string` (not `list<u8>`) and `get-received-messages` returns a single
  delimited `string`, not `list<tuple<...>>`. This is unrelated to and does not
  weaken `guest-api::handle-message`'s real `list<u8>` contract, which the host
  invokes through a separate, direct `Val`-construction path
  (`AppSandboxEngine::deliver_message`) with no such limitation.
- **Backpressure test placement**: task.md's checklist lists channel backpressure
  under Slice 6A's *integration* tests, but the only real bounded/fallible resource
  on the publish path is `crates/mqtt_broker`'s own host link (see spike finding
  7 above), so the deterministic test for it (`publish_returns_backpressure_error_when_channel_saturated`)
  lives in `crates/mqtt_broker/src/tests.rs` rather than being duplicated at the
  `control_plane` native-dispatch layer, where routing ~1000+ JSON-RPC round trips
  through the full dispatch stack would be far slower without proving anything new.
- Two pre-existing, unrelated bugs found and fixed while working in this area:
  a broken symlink in `test-components/data-layer-test/wit/deps/data-layer/data-layer.wit`
  left over from the `crates/bindings` → `crates/wit_interfaces` rename (commit
  `61962d5`), and stray `.claude/.cc-writes` bookkeeping directories that had
  landed inside `wit/deps/` trees and broke `wit-parser` (removed; harmless
  outside a WIT-parsed directory).

### Failure / security test outcomes (task.md's table, Slice 6A rows)

| Test | Outcome |
|---|---|
| Service A publishes to service B's MQTT namespace | Blocked — `test_messaging_namespace_isolation` (`crates/control_plane`) |
| A process speaks raw MQTT to the broker's port | N/A confirmed — `no_network_listener_is_bound` (`crates/mqtt_broker`) binds `127.0.0.1:1883` successfully right after constructing a broker |
| Substrate restarts with active subscriptions | Replayed from `substrate.db`, no manual re-subscribe — `test_messaging_subscriptions_survive_restart_replay` (`crates/control_plane`) |
| Native subscriber's stream dies without a clean unsubscribe | Subscription dropped, not leaked — `test_native_subscriber_receives_push_delivery_and_close_unsubscribes` (`crates/substrate`), via both the read-EOF mechanism described above and a raced `SendStream::stopped()` (QUIC `STOP_SENDING`) signal on the Iroh transport — see `crates/router/src/stop_signal.rs` |

(The remaining rows in that table are Slice 6B/7 concerns — QUIC stream
protocols, `stream-cursor`/`stream-sink`, signed-URL HTTP — out of scope here.)

### Reference scenario steps 14-15

Covered by `test_guest_to_guest_cross_service_message_delivery`
(`crates/sandbox_wasm/tests/messaging_integration.rs`): service A publishes
`orders/new` (host-namespaced to `svc/messaging-svc-a/orders/new`); service B
subscribes to the fully-qualified topic and receives it via
`guest-api::handle-message`. (Step 15's "then reads the blob from step 13 by
hash" is an M03-sss Slice 5 blob-store detail, already covered by that slice's
own tests — not re-exercised here.)

Retained-message support ("A4" decision) is broker-internal/plumbing-only in
Slice 6A: `retained_message_delivered_to_late_subscriber`
(`crates/mqtt_broker/src/tests.rs`) proves the mechanism works via the raw-
packet escape hatch, but the production `publish()` path never sets the
retain flag and the WIT surface has no `retain` parameter by design, so
there is no guest/native-facing trigger yet.

### Performance budgets (measured)

| Metric | Budget | Measured (p99, n=20) | Test |
|---|---|---|---|
| MQTT `publish` → native-subscriber delivery | < 5 ms | **3.46 ms** | `test_native_subscriber_receives_push_delivery_and_close_unsubscribes` (`crates/substrate/tests/messaging_client_e2e.rs`) |
| MQTT `publish` → guest `handle-message` delivery | < 25 ms | **5.16 ms** | `test_guest_delivery_latency_budget` (`crates/sandbox_wasm/tests/messaging_integration.rs`) |

### Verification

All run from a clean `slice-6a-messaging` branch off `main`:

```
cargo +nightly fmt --all -- --check         # zero diff
cargo clippy --workspace --all-targets --all-features   # zero warnings, zero errors
cargo test --workspace                      # 28 test binaries, all green, 0 failures
mise run test:e2e                           # 4 passed (19.3s) — no regression
```

New tests added this slice (all passing): 7 in `crates/mqtt_broker/src/tests.rs`;
2 in `crates/sandbox_wasm/tests/messaging_integration.rs`; 4 in
`crates/control_plane/src/service.rs` (`test_native_dispatch_data_layer_and_blob_store_round_trip`
extended with vault/app-config/messaging round trips, plus 3 new dedicated
messaging tests); 1 in `crates/substrate/tests/messaging_client_e2e.rs` (covers
both basic-path delivery and close-as-unsubscribe, plus the native-subscriber
performance budget).

## Slice 6B: Bidirectional Streaming (Complete)

**Implemented by:** Claude Code, Sonnet 5 (`claude-sonnet-5`).

### What was built

- **`docs/decisions/0014-quic-stream-protocol-routing.md`** — the required
  design note (Dependency Gate 4), covering direction disambiguation,
  guest-implemented resource mechanics, instance lifetime/quota, peer-kind
  symmetry, and where the routing lives.
- **`crates/chunk_transfer`** (package `syneroym-chunk-transfer`) — new crate:
  `ChunkSource`/`ChunkSink` traits, `pull_until_eof`/`push_until_eof` shared
  host-side loops, 5 unit tests including a `Box<dyn ChunkSink>` object-safety
  proof. `crates/data_blob/src/chunk_transfer.rs` implements
  `ChunkSource`/`ChunkSink` for `Box<dyn DownloadSession>`/`Box<dyn
  UploadSession>`, sharing this core with `blob-store`'s own upload/download
  sessions rather than maintaining a second chunking loop (task.md's
  "Consolidate chunk-transfer core").
- **`crates/wit_interfaces/wit/messaging/messaging.wit`** —
  `register-stream-protocol` added to `host-api`; new `stream-types` interface
  with `stream-cursor` (guest-as-source, `next-chunk`) and `stream-sink`
  (guest-as-sink, `push-chunk`/`finalize`) resources, both **guest-exported**
  (the reverse of every other resource in this codebase — see "Day-0 spike
  finding" below); `handle-stream-request`/`accept-stream-upload` added to
  `guest-api`, both taking a `protocol: string` first parameter (ADR
  deviation 1). `host.wit` exports `stream-types`.
- **`crates/sandbox_wasm/src/stream.rs`** (new) — `StreamContext`,
  `StreamRegistry` (per-service concurrency cap, `AbortHandle` tracking, and a
  `Drop` impl that aborts every tracked stream as the backstop for every other
  teardown path), `GuestStreamCursor`/`GuestStreamSink` (dynamic
  `Val::Resource`-based resource method calls, epoch/fuel re-arm per chunk
  call), `call_handle_stream_request`/`call_accept_stream_upload`.
- **`crates/sandbox_wasm/src/engine.rs`** — `HostState.streaming` field,
  `register_stream_protocol` `host_api` impl (delegates to
  `EndpointRegistry::register`, ADR deviation 2 — see below);
  `AppSandboxEngine::{endpoint_registry, stream_registry,
  max_concurrent_streams_per_service}`; `open_stream_instance`,
  `handle_stream_protocol_request` (spawns a dedicated task per stream,
  tracked in `StreamRegistry`, awaited synchronously by the caller),
  `run_stream_protocol_request` (resolves the guest export, drives
  `pull_until_eof`/`push_until_eof`, explicitly `writer.shutdown()`s on every
  exit path — see "Bug found and fixed" below), `abort_streams` (wired into
  `stop_wasm`). All `AppSandboxEngine::init`/`HostState::new` call sites
  across the workspace updated for the new params.
- **`crates/router`** — `RoutePreamble.dir: Option<StreamDirection>` parses
  `?dir=upload|download` leniently (`crates/router/src/preamble.rs`; strict
  validation happens at the use site per the ADR); `dispatch.rs` gained the
  `(RouteProtocol::Raw, WasmChannel) -> WasmComponent` pipeline arm;
  `route_handler/io.rs`'s `handle_raw_stream` validates `dir` strictly, reads
  one framed initial payload, and calls
  `AppSandboxEngine::handle_stream_protocol_request`.
- **`test-components/stream-test`** (package `syneroym-test-stream`) — new
  WASM fixture: guest-as-source (deterministic download payload derived from
  `peer_id`+`request_data`, chunked 8 bytes at a time) and guest-as-sink
  (accumulates pushed chunks, commits to `data-layer` as JSON on `finalize`).
  Supports test sentinels: `request_data`/`metadata == "reject"` declines;
  `metadata == "fail-after-first-chunk"` makes `push-chunk` fail from the 2nd
  call onward (abort-without-finalize coverage).
- **`crates/rpc/src/dispatch_registry.rs`** — new
  `WeakNativeDispatchRegistry` type alias, and updated module docs explaining
  the reference-cycle hazard `ControlPlaneService` must avoid (see "Bug found
  and fixed" below).

### Day-0 spike finding

A throwaway spike (scratchpad-only, not committed) confirmed the one genuine
unknown before the design note could be finalized: dynamic
`Val::Resource(ResourceAny)` invocation **does** work for a guest-exported WIT
resource in wasmtime 46.0.1. This ruled out needing typed `bindgen!` export
bindings (uncertain toolchain support) in favor of the same
`get_wasm_func`/`Val`-construction pattern `invoke_lifecycle_hook` already
uses elsewhere, generalized to resource methods.

### Two deliberate ADR deviations from `task.md`'s literal task list

1. **`handle-stream-request`/`accept-stream-upload` gain a `protocol: string`
   first parameter**, vs. task.md's literal `(peer-id, request-data)`. A
   service can `register-stream-protocol` more than once, but there is only
   one `handle-stream-request` export per component — without the protocol
   name in the call, a guest registering two protocols can't tell them apart.
   `preamble.interface` already carries the protocol name end-to-end, so
   threading it into the guest call is a small, in-pattern addition (same
   category as Slice 6A's `next-chunk` `result<>` fix).
2. **No new `stream_protocol_registrations` table.** `register-stream-protocol`
   reuses `EndpointRegistry` (already used for ordinary WIT interface
   declarations), which already persists to `endpoints.db` and replays on
   restart before the router starts accepting connections — restart-replay
   and undeploy-cleanup are correct with zero new persistence code. Trade-off:
   a stream-protocol registration isn't distinguishable from an ordinary
   declared interface by registry inspection alone; the graceful-decline path
   (a guest that doesn't export the stream handler) is the actual safety net,
   not the registry's typing.

### Known environment workaround (will bite the next slice's fixtures too)

`cargo component build` is broken in this environment for any test component
with a `wit/deps/` directory (fails with "package not found", reproducible
even on pre-existing, untouched fixtures like `data-layer-test` — not caused
by this work). Workaround used for `test-components/stream-test`: build the
fixture directly with
`CARGO_TARGET_DIR=<fixture>/target cargo build --manifest-path <fixture>/Cargo.toml --release --target wasm32-wasip2`
(never `cd` into the fixture dir — the session's bookkeeping hook can drop a
`.claude/.cc-writes` directory into a `wit/deps/` tree it's `cd`'d into,
which breaks `wit-parser`). Rust's `wasm32-wasip2` target compiles straight
to a valid component with no `cargo-component` post-processing needed —
verified via `wasm-tools component wit`.

### Bugs found and fixed

- **Missing `writer.shutdown()` on the download path.** Neither
  `pull_until_eof` nor `push_until_eof` shuts down the destination writer.
  `run_stream_protocol_request` now explicitly calls `writer.shutdown().await`
  on every exit path (guest decline, success, error) — without it, a real
  QUIC peer reading the download direction to EOF hung forever since nothing
  ever signalled FIN.
- **`Arc` reference cycle hanging graceful shutdown indefinitely** — found
  while root-causing a hang in `stream_client_e2e.rs`'s real-client test:
  after a full, successful test run (both directions verified byte-exact) and
  a clean-looking `syneroym_substrate::runtime` shutdown sequence (every
  component logged its own shutdown, ending in `"shutdown complete"`), the
  test process never exited — `cargo test` hung indefinitely post-teardown.
  Root-caused via `sample`(1) (macOS's stack sampler; `lldb -p` attach is
  blocked by this sandbox) plus a temporary `Weak`-based strong-count monitor:
  `SqliteStorageProvider`'s `Arc` strong count sat stable at 4 forever, never
  decreasing, which blocked `run_writer_loop`'s `spawn_blocking` thread inside
  `blocking_recv()` (waiting for its channel's last `Sender` to drop), which
  in turn is exactly what `tokio::runtime::Runtime`'s `Drop` impl blocks on
  after every async task has already been torn down — so the leak could not
  be an ordinary un-aborted task (tokio force-drops those before touching the
  blocking pool). The actual cause: `ControlPlaneService` held a **strong**
  `NativeDispatchRegistry` (`Arc<DashMap<String, Arc<dyn NativeService>>>`)
  as a field, but `ControlPlaneService` itself is inserted into that same
  `DashMap` (`RouteHandler::init`'s `register_native_service` call, keyed by
  the substrate's own `service_id`) — a two-node `Arc` cycle
  (`registry -> Arc<ControlPlaneService> -> registry`) that reference counting
  can never collect, independent of how correctly every task/router/QUIC
  layer shuts down (all of which were verified correct in the process of
  ruling them out). **Fix:** `ControlPlaneService.native_dispatch` is now a
  `Weak` (`syneroym_rpc::WeakNativeDispatchRegistry`), upgraded at each of its
  two use sites (`insert` on deploy, `remove` on undeploy); `RouteHandlerInner`
  keeps the one strong clone, matching this codebase's existing
  `Weak`-backreference convention (`AppSandboxEngine::self_weak`,
  `MessagingContext.engine`). This is a genuine production bug, not a
  test-only artifact — the real `syneroym-substrate` binary would hang the
  same way on any graceful shutdown after a data-layer-using service had ever
  been deployed. Existing `control_plane` unit tests that constructed
  `ControlPlaneService` with an inline, unretained
  `NativeDispatchRegistry::default()` needed updating to keep their own
  strong clone alive (mirroring `RouteHandlerInner`'s real ownership), for
  the same reason the production code needs one.

### Failure / security test outcomes (task.md's table, Slice 6B rows)

| Test | Outcome |
|---|---|
| Peer opens a stream against an unregistered protocol namespace | Rejected cleanly, no panic/hang — `test_unregistered_stream_protocol_rejected_cleanly` (`crates/substrate`) |
| Guest declines a `handle-stream-request` | Stream closed without invoking `next-chunk` — `test_download_declined_by_guest_closes_stream_without_bytes` (`crates/sandbox_wasm`) |
| Guest declines an `accept-stream-upload` | Incoming stream closed, no `stream-sink` created, no bytes read — `test_upload_declined_by_guest_leaves_no_stored_content` (`crates/sandbox_wasm`) |
| `push-chunk` returns `Err` mid-upload | Upload aborted, `finalize()` never called, no leaked state — `test_upload_push_chunk_failure_aborts_without_finalize` (`crates/sandbox_wasm`) + `push_until_eof_aborts_without_finalize_on_push_failure` (`crates/chunk_transfer`, shared-core level) |
| `stream-cursor.next-chunk()` returns `Err` mid-download | Covered at the shared-core level only — `pull_until_eof_propagates_source_error` (`crates/chunk_transfer`) proves the generic pull loop stops and propagates on any `ChunkSource` error; **not separately exercised as a WASM-guest-returns-`Err`-mid-download integration test** — noted honestly rather than over-claimed |
| Long-running stream exceeds the default epoch deadline while progressing | No spurious trap — `test_long_running_stream_does_not_trap_on_epoch_deadline` (`crates/sandbox_wasm`) |
| Substrate restarts with active stream-protocol registrations | Replayed via `EndpointRegistry`, no manual re-registration — `test_stream_protocol_registration_survives_restart_replay` (`crates/sandbox_wasm`) |
| Cross-service stream namespace isolation | A peer cannot address another service's registered protocol — `test_cross_service_stream_protocol_isolation` (`crates/sandbox_wasm`) |
| `undeploy` cleans up a service's stream-protocol registrations | `test_stream_protocol_undeploy_removes_registration` (`crates/control_plane`) |

### Reference scenario steps 16-18

Not separately exercised as their own dedicated scenario test — the
underlying mechanics are proven via other tests instead, the same way Slice
6A's status.md handled an analogous gap. Step 16 (register) and step 17
(download-direction pull to EOF) are covered by
`test_download_direction_end_to_end`; step 18 (upload-direction push +
`finalize` + read-back) is covered by
`test_upload_direction_end_to_end_commits_content` (both in
`crates/sandbox_wasm/tests/stream_integration.rs`). Neither test literally
chains through a prior blob-store step's blob hash the way the scenario's
prose describes — the streamed payloads are the fixture's own deterministic
content, not a blob fetched from Slice 5's `blob-store`. Chaining the two
mechanisms together was judged to add scenario-realism without proving new
code paths (both `blob-store` and this slice's streaming are independently,
thoroughly tested), so it wasn't added as a separate test.

### Performance budgets (measured)

| Metric | Budget | Measured (average, n=53 chunks) | Test |
|---|---|---|---|
| `stream-cursor.next-chunk()` round trip | < 5 ms p99 | **27.7 µs** | `test_next_chunk_and_push_chunk_latency_budget` (`crates/sandbox_wasm/tests/stream_integration.rs`; asserted in-test at a 15 ms average threshold — 3x the p99 budget — for CI-runner headroom, matching Slice 6A's own budget-test margin) |

`push-chunk` round trip is not separately budget-tested — the upload-direction
integration tests (`test_upload_direction_end_to_end_commits_content`) don't
assert latency, only correctness; the shared `next-chunk`/`push-chunk` code
path (`crates/chunk_transfer`) means the measured `next-chunk` number is
representative of both directions' per-call host/guest round-trip cost, but
this is noted rather than presented as if `push-chunk` had its own dedicated
measurement.

### Real-client end-to-end test: resolved, not a known issue

`test_real_client_opens_direct_stream_both_directions`
(`crates/substrate/tests/stream_client_e2e.rs`) initially exhibited the
post-teardown hang described above during development. It is **not** a known
issue left in the suite — the root cause (the `Arc` cycle) is fixed, and the
test now passes cleanly alongside
`test_unregistered_stream_protocol_rejected_cleanly` in ~21s total.

### Verification

All run from the `slice-6b-streaming` branch:

```
cargo +nightly fmt --all -- --check
# zero diff

cargo clippy --workspace --all-targets --all-features
# zero warnings, zero errors (43 crates checked)

cargo test --workspace
# 43 test binaries, 298 tests passed, 0 failed
# includes stream_client_e2e's 2 tests (both real-QUIC-client
# directions + unregistered-protocol rejection), 20.18s, no hang

mise run test:e2e
# 4 passed (20.2s) — no regression
```

New tests added this slice (all passing): 5 in `crates/chunk_transfer/src/lib.rs`;
10 in `crates/sandbox_wasm/tests/stream_integration.rs`; 1 in
`crates/control_plane/src/service.rs`
(`test_stream_protocol_undeploy_removes_registration`); 2 in
`crates/substrate/tests/stream_client_e2e.rs`.

## Slice 7: HTTP Passthrough (Not Started)
