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
| Native subscriber's stream dies without a clean unsubscribe | Subscription dropped, not leaked — `test_native_subscriber_receives_push_delivery_and_close_unsubscribes` (`crates/substrate`), via the read-EOF mechanism described above rather than a literal `SendStream::stopped()` call |

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

## Slice 6B: Bidirectional Streaming (Not Started)

## Slice 7: HTTP Passthrough (Not Started)
