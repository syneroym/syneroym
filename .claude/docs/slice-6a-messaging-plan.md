# Handoff: Implement Slice 6A (Messaging WIT + Embedded Pub/Sub Broker)

> **For the fresh session picking this up**: this document is fully
> self-contained — it was written after a prior session's exploration +
> planning ran out of context budget before implementation started. The
> plan below was reviewed (including one external review pass) and is
> **approved to implement directly** — you do not need to re-propose it or
> get plan re-approval, but you should still sanity-check the facts below
> against the live repo before trusting them (file:line references can
> drift). Read `docs/planning/milestones/M03B-messaging/task.md` (lines
> 144-470 are Slice 6A) and `docs/decisions/0010-mqtt-broker-rumqttd.md`
> yourself first — this doc summarizes and resolves ambiguities in them,
> it doesn't replace them.

## Task (verbatim intent from the user)

Implement **only Slice 6A** from
`docs/planning/milestones/M03B-messaging/task.md`. Do not start Slice 6B
or Slice 7. Read the canonical project documents and inspect the current
worktree before changing anything. Preserve unrelated changes. **Do not
commit or stage files** (repo convention: `AGENTS.md` forbids
staging/committing on `main`; regardless, this task should leave commits
to the user). Implement the slice completely, add its tests, and update
`task.md` and `status.md` in the M03B-messaging milestone folder with
factual progress and verification evidence, **including which tool
implemented this slice** (state: implemented by Claude Code, Sonnet 5, or
whichever model is actually running — update this line to be accurate).
Run the relevant tests for this slice and paste the passing output into
`status.md`. Do not stop until all tests and clippy checks pass for this
slice. If not converging after a reasonable number of attempts, stop and
report what's blocking instead of thrashing.

**Branch**: the repo starts clean on `main`. Create and switch to a
feature branch `slice-6a-messaging` off `main` before making any edits (do
not edit in place on `main`). No commits/staging on this branch either —
just a clean place to work.

**AGENTS.md task**: also add a short new rule to `AGENTS.md` (under
"Project & Rust Specifics") codifying the crate-naming convention: new
crates go under `crates/<snake_case_name>/` with Cargo package name
`syneroym-<kebab-case-name>` (directory uses underscores, package name
uses hyphens) — even when a planning doc's prose uses a hyphenated
directory name (e.g. `task.md` literally says `crates/mqtt-broker/`, which
is wrong per the repo's own convention; the real crate name must be
`crates/mqtt_broker`). This was flagged by the user specifically so it
isn't missed again.

## Why this milestone/slice exists (context)

M03B-messaging was split out of `M03-sss/task.md` on 2026-07-09 (see
`task.md`'s provenance note at the top) to carry the pub/sub + streaming +
HTTP-bridge tail of M3 with room for detailed planning. Slice 6A is gated
on M03-sss Slice 5 (blob store) being complete — it is (see
`docs/planning/milestones/M03-sss/status.md`). ADR-0010 (+ 2 amendments)
is the accepted design authority for the broker itself.

Slice 6A delivers: an embedded `rumqttd` MQTT broker; the
`syneroym:messaging@0.1.0` WIT package (pub/sub half only — no streaming,
that's Slice 6B); native (non-WASM) dispatch for `publish`; and a
genuinely new push-delivery mechanism for both WASM guests
(`guest-api::handle-message`) and plain `SyneroymClient` callers
(`subscribe` over a live connection, since there's no existing precedent
for a long-lived push channel in this codebase's native-dispatch layer).

## Repo state assumed by this plan (verify before trusting)

- Crate rename commit `61962d5` already landed. 21 crates under `crates/`,
  all snake_case dirs / `syneroym-<kebab>` package names — **verified**
  via `grep '^name' crates/*/Cargo.toml`, every single one follows this
  (e.g. `crates/data_blob` → `syneroym-data-blob`, `crates/sandbox_wasm` →
  `syneroym-sandbox-wasm`). No `mqtt_broker` crate exists yet.
- M03-sss Slice 5 (blob store, native dispatch machinery) is complete —
  this is what Slice 6A's native dispatch and `HostState` wiring build on
  top of.
- Nothing in `docs/planning/milestones/M03B-messaging/status.md` is
  started yet ("Slice 6A: ... (Not Started)").

## Verified `rumqttd` facts (from docs.rs, NOT from ADR-0010's prose — the ADR predates any real integration attempt)

Latest published version: **0.20.0**. Nothing pinned in this repo yet.

- `rumqttd::Broker::new(config: Config) -> Broker`
- `Broker::link(&self, client_id: &str) -> Result<(LinkTx, LinkRx), LinkError>` — in-process, no network needed.
- `Broker::start(&mut self) -> Result<(), Error>` — **synchronous, not async.** rumqttd's router runs a blocking loop. Must be driven via `tokio::task::spawn_blocking(move || broker.start())`, not `tokio::spawn(async {...})`. This corrects ADR-0010's looser "background Tokio task" phrasing.
- `rumqttd::Config { id: usize, router: RouterConfig, v4: Option<..>, v5: Option<..>, ws: Option<..>, cluster: Option<..>, console: Option<..>, bridge: Option<..>, prometheus: Option<..>, metrics: Option<..> }` — leave everything but `id`/`router` as `None` → **no network listener is ever bound**. Directly satisfies ADR-0010's Finding A5 ("no `[mqtt] bind_addr`") for free.
- `rumqttd::RouterConfig { max_connections, max_outgoing_packet_count, max_segment_size, max_segment_count, custom_segment: Option<..>, initialized_filters: Option<Vec<Filter>>, shared_subscriptions_strategy: Strategy }` — no explicit retained-message toggle; retention is protocol-level, not config-gated.
- `LinkTx::publish<S: Into<Bytes>, V: Into<Bytes>>(&mut self, topic: S, payload: V) -> Result<usize, LinkError>` (+ `try_publish` non-blocking variant) — **no retain parameter on either**.
- `LinkTx::subscribe<S: Into<String>>(&mut self, filter: S) -> Result<usize, LinkError>` / `unsubscribe` (+ `try_` variants) — arbitrary MQTT filter string, wildcards (`+`/`#`) pass through as-is (empirically confirm in the spike).
- `LinkTx::send(&mut self, data: rumqttd::protocol::Packet) -> Result<usize, LinkError>` (async) — raw-packet escape hatch, needed for retained publishes.
- `rumqttd::protocol::Publish { retain: bool, topic: Bytes, payload: Bytes }`, constructor `Publish::new(topic, payload, retain: bool)`. **Retained publish = `link_tx.send(Packet::Publish(Publish::new(topic, payload, true))).await`** — this is the only way to retain a message; `publish()`/`try_publish()` cannot.
- `LinkRx::next(&mut self) -> Result<Option<Notification>, LinkError>` (async) — the receive-loop primitive to use. **The exact `Notification` enum variant/field names for a forwarded publish were NOT confirmed from docs** — read them from the compiler or `cargo doc --open` locally during the spike; do not guess.
- **Biggest unresolved risk, and the spike's #1 priority**: no explicit "stop the router" method was found on `Broker` in the docs fetched. Whether dropping all `LinkTx`/`LinkRx` handles causes `start()`'s blocking loop to return (so the `spawn_blocking` thread joins cleanly) is unconfirmed. If no clean stop exists, fall back to: a detached blocking thread where `MqttBroker::drop` only guarantees no *new* links are created (accept the OS thread may not join on shutdown) — and adjust the "CancellationToken terminates broker task within 1s" test to reflect whatever is actually true, rather than asserting something false.

## Step 0 — Day-1 Spike (do first, per task.md's own sequencing)

Add `rumqttd` to a throwaway test binary/example (not the real crate yet) and empirically confirm, against 0.20.0:
1. **Priority 1**: how to cleanly stop `Broker::start()`'s blocking loop.
2. Retained messages: publish via raw `Packet::Publish{retain:true}`, then open a *new* `Broker::link()` client and subscribe — confirm the retained message arrives to a subscriber that joined after the publish.
3. `+`/`#` wildcard subscriptions match through `LinkTx::subscribe`.
4. Read the real `Notification` enum for exact variant/field names of a forwarded publish.

**Record findings in `docs/planning/milestones/M03B-messaging/status.md` before writing `crates/mqtt_broker`'s real implementation.** If the stop-mechanism question resolves unfavorably, document the accepted fallback there too.

## Step 1 — Cargo / mise / AGENTS.md

- Add `rumqttd = "0.20"` and `tokio-util` to `[workspace.dependencies]` in root `Cargo.toml`. Add a pin-rationale comment for `rumqttd` following the existing `sha2`/`object_store`/`iroh` comment convention in that file (even with no live conflict yet — task.md/ADR-0010 ask for this pattern explicitly).
- New crate: **`crates/mqtt_broker`** (package `syneroym-mqtt-broker`) — snake_case dir, kebab package name, matching all 21 existing crates. **Deliberately deviates from task.md's literal `crates/mqtt-broker/` text** — record this deviation in `status.md`.
- No `mise.toml` changes (no new external tool).
- Add the AGENTS.md rule described above under "Task".

## Step 2 — `crates/mqtt_broker` crate

Files: `Cargo.toml`, `src/lib.rs`, `src/tests.rs`.

`MqttBroker`:
- Holds the `rumqttd::Broker` (ownership shape depends on spike findings — likely moved into the `spawn_blocking` closure, with `LinkTx`/`LinkRx` handles retained on the `MqttBroker` side for the host's own use), a `tokio_util::sync::CancellationToken`, and `channel_capacity: usize` (from config).
- `MqttBroker::new(config: MqttBrokerConfig) -> Result<Self>` — builds `rumqttd::Config` with only `id`/`router` set, spawns the router via `spawn_blocking`.
- `publish(&self, topic: &str, payload: Vec<u8>) -> Result<(), MessagingError>` — via a shared internal "host" link, call `try_publish`/`publish`; map backpressure/full conditions to `MessagingError::Internal("broker channel full: backpressure")`. Exact enforcement point (rumqttd's own channel vs. our bounded `mpsc` wrapper) is guided by the spike — the ADR's bound applies to the host↔rumqttd bridge, and each `subscribe`'s own forwarding channel is separately bounded by `channel_capacity`.
- `subscribe(&self, topic_filter: &str) -> Result<(SubscriptionHandle, mpsc::Receiver<(String, Vec<u8>)>), MessagingError>` — fresh `Broker::link(unique_client_id)`, subscribe the filter, spawn an internal forwarding task looping `link_rx.next().await` → bounded `mpsc::Sender` (capacity = `channel_capacity`). Returns the `Receiver` + a `SubscriptionHandle` whose `Drop` unsubscribes and aborts the forwarding task.
- Retained-publish path (`send(Packet::Publish{retain:true,..})`) — internal/test-only scaffolding; Slice 6A's WIT `publish` has no `retain` parameter (ADR-0010's Finding A4 explicitly rejects adding one), so this is not part of the guest-facing surface. Confirm against spike output whether it needs to be `pub(crate)` or just used inline in the retained-message test.
- `fn namespace_topic(service_id: &str, topic: &str) -> String` — pure function: if `topic` starts with `"svc/"`, use literally; else prefix `svc/<service_id>/`. Per ADR-0010's namespace-isolation rule.
- `Drop for MqttBroker` cancels the `CancellationToken`.

`src/tests.rs` (task.md's Slice 6A unit tests, 1:1 — see task.md lines 410-418):
1. `publish` + `subscribe` same topic delivers message (via `MqttBroker`'s own API, not through Wasmtime).
2. Wildcard `sensors/+/temp` matches `sensors/room1/temp`.
3. Retained message delivered to a subscriber joining after publish.
4. `CancellationToken` terminates broker task within 1s (shape depends on Step 0 finding).
5. Topic disambiguation: `svc/`-prefixed stays literal; anything else gets `svc/<caller>/` prefix (pure-function test, no broker needed).

## Step 3 — WIT interface

New file `crates/wit_interfaces/wit/messaging/messaging.wit`:

```wit
package syneroym:messaging@0.1.0;

interface host-api {
    variant messaging-error { permission-denied, internal(string) }
    publish: func(topic: string, payload: list<u8>) -> result<_, messaging-error>;
    subscribe: func(topic: string) -> result<_, messaging-error>;
    unsubscribe: func(topic: string) -> result<_, messaging-error>;
}

interface guest-api {
    handle-message: func(topic: string, payload: list<u8>) -> result<_, string>;
}

world messaging-guest {
    import host-api;
    export guest-api;
}
```

(Matches `crates/wit_interfaces/wit/data-layer/data-layer.wit`'s style:
one `-error` variant, per-interface funcs, `<domain>-guest` world. **No**
`stream-types`/streaming surface — that's Slice 6B, per task.md's "WIT
Boundary Versioning" section.)

- Symlink `crates/wit_interfaces/wit/host/deps/messaging -> ../../messaging` (directory symlink — matches the `app-config`/`blob-store` pattern; `vault` is the one outlier with no top-level dir, don't copy that shape).
- `crates/wit_interfaces/wit/host/host.wit`: add `import syneroym:messaging/host-api@0.1.0;` and `export syneroym:messaging/guest-api@0.1.0;` to `world host-environment` (which currently imports `context`, `vault`, `data-layer/store`, `app-config`, `blob-store`, and exports `app`).
- `crates/wit_interfaces/src/host.rs` — the `wasmtime::component::bindgen!` invocation (currently `imports: { default: async }`, `exports: { default: async }`, `with:` mapping only `blob-store`'s two resource types). **No new `with:` entries needed** — messaging's WIT this slice has no `resource` types.
- Possibly a new `crates/wit_interfaces/src/messaging.rs` (standalone `bindgen!`, mirroring `data_layer.rs`/`blob_store.rs`) — first check whether `data_layer.rs`'s module is actually consumed by anything besides guest-side test-component codegen before assuming this file is needed; native-dispatch DTOs should most likely be hand-written structs instead (matching blob-store's `native_types.rs` precedent), not reused bindgen types.

## Step 4 — `crates/sandbox_wasm/src/engine.rs` (1414 lines as of this writing)

Key existing facts to re-verify on read:
- `HostState` struct (`engine.rs:63-75`): `wasi`, `table`, `component_id`, `request_ctx`, `memory_limits`, `key_store: Arc<KeyStore>`, `storage_provider: Arc<dyn StorageProvider>`, `blob_provider: Arc<dyn BlobProvider>`, `is_init_context`, `config_generation`. Constructed via `HostState::new(...)` (`engine.rs:90-119`, 7 positional args, already `#[allow(clippy::too_many_arguments)]`).
- `AppSandboxEngine` struct (`engine.rs:542-554`): `blobs_dir`, `engine`, `linker`, `components: DashMap<...>`, `default_max_instructions`, `default_max_memory_bytes`, `_shutdown_tx: Option<oneshot::Sender<()>>`, `pub key_store`, `pub storage_provider`, `pub blob_provider`.
- `AppSandboxEngine::init(config: &SubstrateConfig, endpoints: Vec<...>, key_store: Arc<KeyStore>, storage_provider: Arc<dyn StorageProvider>, blob_provider: Arc<dyn BlobProvider>) -> anyhow::Result<Self>` (`engine.rs:583-589`). Callers to update: `crates/router/src/route_handler.rs:155-164`, `crates/control_plane/src/service.rs` test module (9+ call sites), `crates/sandbox_wasm/tests/*.rs` (3 files), `crates/sandbox_wasm/benches/*`.
- `build_wasm_linker` (`engine.rs:687-696`) registers each interface's `add_to_linker` on `Linker<HostState>`.
- `build_store_and_instantiate` (`engine.rs:919-986`) is the single choke point constructing every `HostState`, setting the epoch deadline (`store.set_epoch_deadline(50)` = 5s wall-clock, `epoch_deadline_trap()`) and fuel budget.
- `invoke_lifecycle_hook` (`engine.rs:1008-1032`) is the closest existing template for a new message-delivery helper, but note the differences (nested interface export lookup via the `Some(interface_name)` branch of `get_wasm_func` at `engine.rs:742-776`, non-empty `Val` args, no existing `Val::List` helper).
- `crates/sandbox_wasm/src/conversions.rs`'s `json_to_wasm_params` only handles `Type::String`/`Type::U32`/`Type::Bool` — cannot build the `list<u8>` payload arg, confirming the need for a hand-built `Val` construction path instead of routing through the JSON conversion helper.
- `Host` trait impls (`vault::Host`, `app_config::Host`, `store::Host`, `blob_store::Host`+resources) all clone what they need out of `&mut self` **before** any `.await` (`engine.rs:167-175` explains why: `HostState` embeds a non-`Sync` `WasiCtx`, so holding a borrow across an await breaks `Send`).

Changes:
- `HostState`: add a `messaging: MessagingContext` field bundling `broker: Arc<MqttBroker>`, `engine: Weak<AppSandboxEngine>`, service_id (reuse existing `component_id`), and `storage_provider` (already present) — avoids scattering raw fields across the already-long `HostState::new` arg list.
- `AppSandboxEngine`: add `pub messaging_broker: Arc<MqttBroker>` field, and `self_weak: OnceLock<Weak<AppSandboxEngine>>` (**not** `Arc::new_cyclic` — `init` is `async fn` with `.await` throughout, and `new_cyclic`'s closure must be fully synchronous). Wiring: `init(...)` keeps returning `Result<Self>` unchanged; the caller (`RouteHandler::init`, Step 9) does:
  ```rust
  let engine = Arc::new(AppSandboxEngine::init(...).await?);
  engine.self_weak.set(Arc::downgrade(&engine)).expect("set once");
  ```
  immediately after construction, before the engine is used for anything else. This lets a live `subscribe()` call spawn a forwarding task that can reach back into the engine long after the originating `Store`/`HostState` is gone (every WASM invocation gets a fresh `Store`, so nothing else can hold a durable "invoke this service's `handle-message`" handle).
  - `AppSandboxEngine::init(...)` gains a 6th positional arg: `messaging_broker: Arc<MqttBroker>` — update every call site listed above.
  - New `subscriptions: DashMap<(String, String), SubscriptionHandle>` field, keyed `(service_id, namespaced_topic)`.
  - New `AppSandboxEngine::deliver_message(&self, service_id: &str, topic: &str, payload: Vec<u8>)` — modeled on `invoke_lifecycle_hook` but: (a) looks up the export via the interface-scoped `Some("guest-api")` branch of `get_wasm_func`, not the root-level branch `invoke_lifecycle_hook` uses; (b) builds args directly as `&[Val::String(topic.into()), Val::List(payload.iter().map(|b| Val::U8(*b)).collect())]`; (c) if the component doesn't export `guest-api::handle-message`, silently return (message discarded, per ADR-0010); (d) goes through the normal `build_store_and_instantiate` (automatic epoch/fuel setup, no special-casing needed); (e) runs on its own spawned Tokio task, separate from the broker's own delivery loop.
  - New `AppSandboxEngine::register_internal_subscription(&self, service_id: &str, namespaced_topic: &str) -> Result<()>` — the core in-memory subscribe logic, extracted onto `AppSandboxEngine` itself (NOT left inline inside the `Host::subscribe` impl) because `HostState` is ephemeral and does not exist at substrate startup, when replay (Step 9) needs to run this same logic. Calls `broker.subscribe(...)`, spawns the forwarding task (`self_weak.get().and_then(Weak::upgrade)` → `deliver_message` per message; task exits when the receiver closes), inserts the `SubscriptionHandle` into `subscriptions`. Both `Host::subscribe` (live call) and startup replay call this same method — only the DB write differs (live call writes the row first; replay skips it since the row's already there).
  - New `AppSandboxEngine::unsubscribe_all(&self, service_id: &str)` — drops every `subscriptions` entry for that service (called from `undeploy` cleanup, Step 6).
  - `impl host_api::Host for HostState` (confirm exact generated trait path once WIT compiles) — thin wrappers only, no forwarding-task logic of their own:
    - `publish`: namespace the topic, delegate to `self.messaging.broker.publish(...)`, map errors to `messaging-error`.
    - `subscribe`: namespace the topic, persist the row via `storage_provider`, then `engine.register_internal_subscription(service_id, &namespaced_topic)` via the weak engine handle. Clone everything out of `&mut self` before any `.await`.
    - `unsubscribe`: namespace the topic, delete the DB row, remove+drop the matching `subscriptions` entry (triggers broker-side unsubscribe via `SubscriptionHandle::Drop`).
- `build_wasm_linker`: add `host_api::add_to_linker::<_, HasSelf<HostState>>(&mut linker, |state| state)?;` (guest-exported `guest-api` is not linked — looked up per-instance via `get_export`, same as `init`/`migrate`).

## Step 5 — `crates/data_db` (substrate.db migration + StorageProvider)

`crates/data_db/src/sqlite.rs` migrations are inline SQL gated by one shared `schema_version` table + `const SUBSTRATE_SCHEMA_VERSION: &str = "m3a";`, all run unconditionally every startup inside one transaction (`CREATE TABLE IF NOT EXISTS`, idempotent) via `run_m3a_migration`.

- Bump `SUBSTRATE_SCHEMA_VERSION` to `"m3b"`.
- Add `run_m3b_migration(&Connection) -> rusqlite::Result<()>` creating:
  ```sql
  CREATE TABLE IF NOT EXISTS messaging_subscriptions (
    service_id TEXT NOT NULL,
    topic      TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (service_id, topic)
  );
  ```
- Call `Self::run_m3b_migration(&tx)?;` right after the existing `run_m3a_migration` call inside `SqliteStorageProvider::new`.
- Update the existing schema-version assertion test (around `sqlite.rs:1205`) to expect `"m3b"`.
- New `StorageProvider` trait methods (exact naming up to you):
  - `save_messaging_subscription(&self, service_id: &str, topic: &str) -> Result<()>`
  - `delete_messaging_subscription(&self, service_id: &str, topic: &str) -> Result<()>`
  - `delete_all_messaging_subscriptions_for_service(&self, service_id: &str) -> Result<()>`
  - `list_all_messaging_subscriptions(&self) -> Result<Vec<(String, String)>>`
  - Implement on `SqliteStorageProvider`; check for any other `StorageProvider` impl (e.g. an in-memory test double) that needs a matching implementation to keep things compiling.

## Step 6 — `crates/control_plane`

- `crates/control_plane/src/synsvc_native.rs`: `SynSvcNativeService` struct gains `messaging_broker: Arc<MqttBroker>` field + constructor param. `dispatch_vault` (`synsvc_native.rs:295-315`) and `dispatch_app_config` (`synsvc_native.rs:319-370`) **already exist and work but have zero dedicated round-trip tests anywhere in the repo** (confirmed via repo-wide grep) — task.md explicitly wants these tests added in this slice. Add `dispatch_messaging` (publish only — subscribe/unsubscribe do NOT go through this request/response trait, see Step 7), following `dispatch_blob_store`'s (`synsvc_native.rs:374-511`) param-parsing/error-mapping style (inline `#[derive(Deserialize)] struct Req`, a domain error-mapping helper, `to_payload(...)`). Add the `"messaging" => self.dispatch_messaging(invocation).await,` arm to `dispatch`.
- `crates/control_plane/src/service.rs`:
  - `NATIVE_CAPABILITY_INTERFACES` (`service.rs:50`, currently `[&str; 4] = ["data-layer", "vault", "app-config", "blob-store"]`) → `[&str; 5]`, append `"messaging"`. This alone makes the existing generic registration loop in `deploy` (`service.rs:471-...`) register messaging's `EndpointRegistry` interface for every deployed service — no separate code path needed. **Important**: forgetting this reproduces the exact `endpoint_type` iteration-order flake M03-sss Slice 5 already hit once (see `service.rs`'s `list()` — it excludes the native-capability interface names from its own enumeration for exactly this reason).
  - `ControlPlaneService` gains `messaging_broker: Arc<MqttBroker>`, threaded into `init(...)`, forwarded to `AppSandboxEngine::init` and `SynSvcNativeService::new`.
  - `undeploy` (`service.rs:510-565`, currently: removes registry cert file, removes every registered interface best-effort, stops/removes WASM or container sandbox, `native_dispatch.remove(&service_id)`, always `Ok(())`): add a new best-effort step (matching the file's warn-on-failure style) calling `storage_provider.delete_all_messaging_subscriptions_for_service(&service_id)` and `app_sandbox_engine.unsubscribe_all(&service_id)` (if a WASM engine is present) — this cleanup class (a long-lived stateful subsystem needing an explicit "forget this service" signal) has no precedent in the current `undeploy`, since every other native capability is pure request/response.
  - Test module: add the missing `vault`/`app-config` native-dispatch round-trip tests, plus a new `messaging/publish` native-dispatch round trip, all following `test_native_dispatch_data_layer_and_blob_store_round_trip`'s exact template (`service.rs` around line 1102-1273: deploy a TCP-type/non-WASM service, drive `SynSvcNativeService::dispatch` directly via `NativeInvocation`, assert on `NativeResponse.payload`, `undeploy`, assert `native_dispatch.get(&service_id).is_none()`).

## Step 7 — Native push delivery (`crates/rpc`, `crates/router`, `crates/sdk`)

The one piece of genuinely new router infrastructure (ADR-0010/task.md's Finding A2 — `NativeService::dispatch`'s strict one-request-one-response shape cannot express a subscription).

Facts: `crates/rpc/src/native.rs`'s `NativeService::dispatch(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse>` — one call, one response, no trait changes needed (special-casing happens entirely at the router layer, before this is ever invoked for `subscribe`). `NativeDispatchRegistry = Arc<DashMap<String, Arc<dyn NativeService>>>` keyed by `service_id`. `crates/rpc/src/framing.rs` has reusable `write_frame`/`read_frame` (u32-length-prefixed, works over any `AsyncWrite`/`AsyncRead`). `JsonRpcRequest`/`JsonRpcResponse` (`crates/rpc/src/types.rs:9-27`) already support `id: Option<Value>` = `None` for notification frames.

`crates/router/src/route_handler/dispatch.rs`'s `handle_json_rpc_loop` (`dispatch.rs:172-201`) currently owns `writer: &mut W` for the connection's lifetime and returns the moment `framing::read_frame` sees an empty/EOF frame, calling `dispatch_json_rpc_once` (`dispatch.rs:31-109`) once per frame. Its call site (`crates/router/src/route_handler/io.rs:171-174`, inside `handle_stream`'s `TransportStage::Binary` arm) already owns both `reader`/`writer` as locals split from an owned `OwnedStream` — so switching the subscribe path to take **owned** `W` (not `&mut W`) and move it into a spawned task is a mechanical signature change, not a borrow-checker fight. `plan_pipeline` (`dispatch.rs:113-165`) needs **no changes** — a `messaging/subscribe` call already resolves to the same `(RouteProtocol::JsonRpc, SubstrateEndpoint::NativeHostChannel{service_id})` shape as any other native-capability call; the special-casing is purely based on parsed `interface`+`method`, at the loop/dispatch-once layer.

Plan:
- Read one frame via `framing::read_frame`; parse enough (`preamble.interface`, and the frame body's `method` field) to check if it's `messaging` + `subscribe`. If not, fall through into the existing generic loop using this frame as its first iteration (i.e. `handle_json_rpc_loop` gains an "first frame already read" entry point, rather than re-implementing frame reading twice).
- If it is a subscribe request: add a new method, e.g. `RouteHandler::handle_messaging_subscribe(reader, writer: W, preamble, service_id, topic)`, which: writes the subscribe ack frame, then loops forever `tokio::select!`ing between (a) the broker-fed `mpsc::Receiver<(String, Vec<u8>)>` — writing each message as a `{"method":"messaging/message","params":{"topic":...,"payload":...},"id":null}` notification via `framing::write_frame` — and (b) the `SendStream`'s own `stopped()` signal (Iroh QUIC send-half close/error, the "peer went away" signal). Whichever fires first: drop the `SubscriptionHandle` and return. `handle_json_rpc_loop`'s existing generic path is left completely unchanged for every other native-capability method.

`crates/sdk/src/lib.rs`: `SyneroymClient` has `pub fn connection(&self) -> Option<TransportConnection>` (`lib.rs:208-211`, cheap clone) and an existing `request_raw` (`lib.rs:228-259`) that opens a bi-stream, writes preamble + one frame, then **immediately calls `send.finish()`** before reading the response — do **not** reuse this as-is for `subscribe` (finishing the send side would let the router's reader hit EOF and tear the whole loop down before any notification arrives). There's also `passthrough_with_conn` (`lib.rs:410-444`) as a precedent for "open a raw bi-stream and hand off to a background-driven loop instead of request/response," though it bridges to a `TcpStream` via `copy_bidirectional`, not an mpsc-fed read loop.

New method: `SyneroymClient::subscribe(&self, interface: &str, topic: &str) -> Result<MessageStream>` — opens its own `conn.open_bi()`, writes preamble + a `subscribe` `JsonRpcRequest` frame, does **not** call `send.finish()`, reads the ack frame synchronously to confirm success, then spawns a background task looping `framing::read_frame` on `recv`, parsing each as a `messaging/message` notification and forwarding `(topic, payload)` through a `tokio::sync::mpsc::Receiver` wrapped by `MessageStream` (a plain `.recv().await`-exposing wrapper is fine; check whether `futures`/`tokio_stream` is already a workspace dep before adding a `Stream` impl). Dropping `MessageStream` must drop the send half (triggering the router's `stopped()`-based cleanup) — verify Iroh's QUIC send-stream `Drop` already resets the stream, rather than needing an explicit `finish()`/`stop()` call in `MessageStream::drop`.

## Step 8 — Config

`crates/core/src/config.rs`: `SubstrateConfig` has no `[mqtt]`-like section today. Add `pub mqtt: MessagingConfig` directly on `SubstrateConfig` (it's a core, always-on capability, not an optional deployment role like `RolesConfig`'s members). `MessagingConfig { channel_capacity: u64 }` defaulting to 1024 — flat struct (`RetryPolicy`-style, `config.rs:710-726`), no `bind_addr` field (ADR-0010's aspirational `[mqtt] bind_addr` is explicitly dropped per Finding A5). Add a `test_messaging_config_defaults` test alongside the existing `test_blob_store_config_defaults` (`config.rs:767-775`).

## Step 9 — Runtime wiring

Facts: `crates/substrate/src/runtime.rs`'s `RuntimeServices` only holds feature-gated *optional* top-level roles (`community_registry`, `coordinator`, `client_gateway`) raced in one `tokio::select!`. The router/app-sandbox/control-plane stack is constructed separately inside `RouteHandler::init` (`crates/router/src/route_handler.rs:142-231`), not `runtime.rs`. Messaging is a core per-node capability (like data-layer/blob-store), always present — so:

- `RouteHandler::init`: construct `MqttBroker::new(config.mqtt)` once, thread `Arc<MqttBroker>` into `AppSandboxEngine::init(...)` and `ControlPlaneService::init(...)` (alongside `key_store`/`storage_provider`/`blob_provider`), store it on `RouteHandlerInner` (`route_handler.rs:77-88`, which currently holds `native_dispatch: NativeDispatchRegistry` and `app_sandbox_engine: Option<Arc<AppSandboxEngine>>`). Immediately after wrapping the engine in `Arc::new(...)`, call `engine.self_weak.set(Arc::downgrade(&engine))` (Step 4).
- **Startup replay (Finding A1)**: immediately after both `MqttBroker` and `AppSandboxEngine` (as an `Arc`, with `self_weak` set) exist, call `storage_provider.list_all_messaging_subscriptions()` and for each `(service_id, topic)` row call `engine.register_internal_subscription(&service_id, &topic)` (same method `Host::subscribe` calls; skip the DB write since the row's already there) — before the router starts accepting connections.
- **No new `RuntimeServices` field.** The broker's own background task lifecycle (blocking router thread + `CancellationToken`) is self-contained, mirroring `AppSandboxEngine`'s existing epoch-timer task pattern (`engine.rs:646-659`, a `oneshot::Sender`-based shutdown signal) — not raced in `runtime.rs`'s top-level `tokio::select!`.
- `RouteHandler`/`ConnectionRouter`'s existing `shutdown()` path needs to cascade into dropping/cancelling `MqttBroker` so its `Drop` runs during graceful shutdown, not just process exit.

## Step 10 — Test fixture: `messaging-pubsub-test` WASM component

Template: `test-components/data-layer-test/` (`Cargo.toml` with `wit-bindgen = "0.55.0"`, `crate-type = ["cdylib"]`; `wit/world.wit` importing a host interface and exporting `init`/`migrate`/a `test-driver` interface; `src/lib.rs` with a `wit_bindgen::generate!` macro in a `mod bindings` block). Root `Cargo.toml` has an `exclude = [...]` list these test-component dirs must be added to (they're standalone build targets, not workspace members).

New `test-components/messaging-pubsub-test/`: same shape, `wit/world.wit` importing `syneroym:messaging/host-api@0.1.0` and exporting `syneroym:messaging/guest-api@0.1.0` (`handle-message`) plus a small `test-driver` interface (e.g. `subscribe-to(topic: string)` / `get-received-messages() -> list<tuple<string, list<u8>>>`) so tests can assert on delivery. Add its dir to root `Cargo.toml`'s `exclude` list. Add a `messaging_pubsub_test_wasm_path()` helper to `crates/core/src/test_constants.rs` (mirroring `data_layer_test_wasm_path()`), and use the same "skip with an eprintln message if the wasm artifact isn't pre-built" pattern (`let Ok(wasm_bytes) = fs::read(path) else { eprintln!("Skipping..."); return; };`) in whatever integration test consumes it — a pre-build step (`cargo build --target wasm32-wasip2 --release` inside the component's dir) is required before this test can actually exercise anything; document that in the test's skip message.

## Step 11 — Integration & E2E tests (task.md's checklist, 1:1 — see task.md lines 405-452)

- **`crates/sandbox_wasm/tests/messaging_integration.rs`** (new, mirrors `data_layer_integration.rs`): two deployed WASM components (`messaging-pubsub-test`) in different services exchange a message guest-to-guest via the fully-qualified cross-service topic (reference scenario steps 14-15 — task.md's Finding B1 fixed the scenario to use the fully-qualified `svc/<first-service-did>/profiles/+` form, since a bare `profiles/+` subscribe from a different service would resolve to the *subscriber's own* namespace and never see the publish).
- **`crates/control_plane/src/service.rs`** test module additions:
  - Native dispatch round trip: `messaging/publish`, plus the previously-missing `vault`/`app-config` round trips (Step 6).
  - Substrate restart replays `messaging_subscriptions`: construct engine/broker twice against the same `storage_provider`/DB, assert a previously-subscribed guest still receives a post-"restart" publish.
  - `undeploy` removes a service's subscriptions; a publish afterward is not delivered to (and doesn't error on) the undeployed service.
  - Namespace isolation: service A cannot receive service B's messages without the explicit `svc/<other>/...` opt-in.
  - Channel backpressure: saturate the bounded channel, assert `publish` returns the backpressure error without blocking/crashing.
- **`crates/substrate/tests/messaging_client_e2e.rs`** (new, mirrors `basic_lifecycle.rs`'s `assert_cmd`-driven process-spawning style): a real `SyneroymClient` calls `subscribe` on a topic over a live substrate/Iroh connection; a second connection (or WASM guest) publishes; assert the first client's `MessageStream` receives it. Also: a native subscriber that closes its stream stops receiving messages (proves close-as-unsubscribe, not just that subscribe works).
- Performance budget checks (plain timing assertions inside relevant integration tests — not Criterion, which is a separate `mise run bench:*` concern not gated by this slice): native-subscriber delivery <5ms p99; guest `handle-message` delivery <25ms p99 (per task.md's Finding B4 split — the guest path is more expensive because of fresh-Store-per-delivery instantiation cost, ~16-18ms measured in M03-sss Slice 3A's own benchmarks). Capture actual measured numbers in `status.md`.
- **Test scope discipline**: task.md explicitly says "kept to 1-2 basic-path tests per API, not exhaustive variation coverage" (task.md line 407) — match this bar, don't over-build coverage beyond task.md's own checklist.
- **All Slice 6A failure/security test rows** (task.md's table, lines 852-864, rows relevant to 6A: namespace isolation, no unauthenticated listener, restart replay, backpressure) must produce documented outcomes — cross-reference against the tests above; add anything missing.

## Step 12 — Docs (required before calling this slice done)

- `docs/planning/milestones/M03B-messaging/task.md`: check off every completed Slice 6A checkbox, including the Day-1 Spike items and the "Measurable Exit Criteria" list (task.md lines 918-936). Leave Slice 6B/7 checkboxes untouched.
- `docs/planning/milestones/M03B-messaging/status.md`: replace "Slice 6A: ... (Not Started)" with a factual completion record, mirroring `M03-sss/status.md`'s existing style for its own Slice 5 entry — include:
  - What was built (crate/file list).
  - The day-1 spike's actual findings (especially how the broker-stop-mechanism question resolved).
  - The `crates/mqtt_broker` vs. task.md's literal `crates/mqtt-broker` naming deviation, and why.
  - Any other interim/behavioral decisions made where task.md left something open (e.g. exact `StorageProvider` method names, whether a `MessagingContext` struct vs. loose fields).
  - All failure/security test outcomes.
  - The two performance-budget measurements with real numbers.
  - Full passing output of `cargo fmt --check`/`clippy`/`test --workspace`/`mise run test:e2e`.
  - **Tool attribution**: state plainly which coding tool/model implemented this slice.

## Verification checklist (must all be green before declaring done)

1. `cargo +nightly fmt --all` — zero diff.
2. `cargo clippy --workspace --all-targets --all-features` — zero warnings/errors.
3. `cargo test --workspace` — all green, including every new test in Step 11.
4. `mise run test:e2e` — no regression to existing e2e scenarios.
5. Re-read task.md's Slice 6A "Measurable Exit Criteria" (lines 918-936) and check each one against actual evidence (test name + assertion), not assumption.
6. Confirm `git status` shows the feature branch, not `main`, and that nothing has been staged/committed.

## Explicit non-goals (do not do these in this session)

- Do not implement Slice 6B (bidirectional streaming) or Slice 7 (HTTP passthrough) — not even scaffolding for them.
- Do not write the Slice 6B design note/ADR (that's a Slice 6B dependency-gate task, not this one).
- Do not commit or stage any files.
- Do not add a `retain` parameter to the guest-facing `publish` WIT function (ADR-0010 Finding A4 explicitly rejects this for Slice 6A).
