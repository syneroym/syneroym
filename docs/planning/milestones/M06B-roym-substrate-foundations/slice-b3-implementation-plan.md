# M06B Slice B3 — The Dual-Build Shim: Implementation Plan

> **Status**: draft for review. Nothing implemented yet.
>
> **Scope** (from [task.md](task.md)'s B3 row, `D-06B-3` and `D-06B-6`, and the
> experience spec's
> [Packaging](../../../roym-integrated-experience-spec.md#packaging-one-source-two-builds)
> section): one trait per host interface, two implementations —
> `wit-bindgen` guest bindings, and an in-process native shim linked into
> `syneroym-substrate` behind a Cargo feature. A fixture written once, built
> both ways, with **one** integration suite that runs against both and must
> produce identical results. Proven against `data-layer`, `blob-store`, and
> `messaging` — three interfaces that already exist.
>
> **What this slice does not build**: any new host capability. No WIT package
> is added. `syneroym:conversation` is B4. If this slice needs a new host
> function to succeed, that is a finding to report, not work to absorb.

Verified against the tree at commit `d00b802` on 2026-08-19. Every line number
below is from that tree. Claims marked **(probed)** were checked by compiling
throw-away code against the real crates, not by reading docs.

> **Revision 2 (review round 1).** Eighteen review points, all re-verified
> against the tree and all accepted. The three that changed the design rather
> than the wording:
>
> - **The parity harness was unrunnable as written.** It gave the two builds
>   different service ids, but the broker's namespaced topic
>   (`svc/<service_id>/<topic>`) is what reaches the delivery handler
>   ([engine.rs:1442](../../../../crates/sandbox_wasm/src/engine.rs#L1442)) —
>   so `read-inbox` would differ per build for a reason the shim cannot fix.
>   Replaced by two *fully independent* host stacks sharing **one** service id
>   (`D-B3-17`, §7.1).
> - **The shim must not write to `messaging_subscriptions`.**
>   [runtime.rs:972](../../../../crates/substrate/src/runtime.rs#L972) replays
>   every row of that table into the WASM engine on boot, for whatever service
>   id the row names. A native app's rows would be replayed into a sandbox with
>   no component to deliver to, and the native subscription would not be
>   restored at all (`D-B3-18`, §4.4).
> - **The extra `messaging` endpoint registration was a silent overwrite** of
>   the supervisor's — `EndpointRegistry::register` is a plain insert
>   ([local_registry.rs:177-189](../../../../crates/core/src/local_registry.rs#L177))
>   and CI runs both features. Removed; the native pump never goes through the
>   router (`D-B3-19`, §6.2).
>
> The rest tightened `NativeAppHost`'s ownership (`D-B3-20`), the fixture's
> service identity (`D-B3-21`), and lints (`D-B3-22`); added §7.5 (a test that
> proves the parity comparison can fail), §13 (**what B3 owes B4** — the
> constraint `D-B3-5` puts on where B4's conversation host implementation must
> live), §12.8–12.10, and four backlog rows; and removed the planning-doc ids
> from every code sketch, which violated
> [AGENTS.md](../../../../AGENTS.md)'s "No Planning-Doc References in Code".

---

## §0 What B1 and B2 hand to B3

1. **Nothing structural.** B1 (gateway person identity) and B2 (declared
   visibility) touch the gateway, the registry, and the deploy manifest. B3
   touches the guest/host capability boundary. The three slices were declared
   independent in [task.md](task.md) and they are: no file this plan edits was
   edited by B1 or B2, except `ServiceConfig` literals in tests, which now carry
   B2's `visibility: None` field (§9.3).
2. **One convention worth inheriting.** B2's plan opened with a findings
   section that corrected its own input documents before deciding anything.
   §1 and §12 here do the same; §12 is the list of things a reader should *not*
   trust in [task.md](task.md) and the experience spec as written.

---

## §1 Findings from reading the tree

### F1 — the guest bindings already exist as ordinary host-side Rust types, and nothing uses them

`crates/wit_interfaces` runs **`wit_bindgen::generate!`** (guest side) for
`data-layer` ([data_layer.rs](../../../../crates/wit_interfaces/src/data_layer.rs))
and `blob-store` ([blob_store.rs](../../../../crates/wit_interfaces/src/blob_store.rs)),
unconditionally for **both** targets, alongside the wasmtime host bindings in
[host.rs](../../../../crates/wit_interfaces/src/host.rs) which are
`#[cfg(not(target_arch = "wasm32"))]`
([lib.rs:5-16](../../../../crates/wit_interfaces/src/lib.rs#L5)).

This matters more than it looks:

- **(probed)** `syneroym_wit_interfaces::data_layer::syneroym::data_layer::store::{put, get, CollectionSchema, RecordWriteValue, DataLayerError, …}`
  resolves and compiles on the **host** target. `store::put` has type
  `fn(&str, &RecordWriteValue) -> Result<(), DataLayerError>`.
- **(probed)** `syneroym_wit_interfaces::blob_store::syneroym::blob_store::blob_store::{open_upload, open_download, BlobWriter, BlobReader, BlobError}`
  likewise. `BlobWriter::write(&self, &[u8])`, `BlobWriter::finish(&self)`
  (**by reference, not by value**), `BlobReader::read(&self, u32)`. Both
  handles are `Send`.
- **(probed)** wit-bindgen 0.57 emits, for every import,
  `#[cfg(not(target_arch = "wasm32"))] unsafe extern "C" fn … { unreachable!() }`
  next to the real `#[link(wasm_import_module = …)]` declaration
  (`wit-bindgen-rust`'s own `declare_import`, read from the vendored registry
  source, not from docs.rs). That is *why* the crate compiles on the host: the
  calls link, they just panic if anyone actually invokes them off-wasm.
- The workspace's `unsafe_code = "deny"`
  ([Cargo.toml:196](../../../../Cargo.toml#L196)) does **not** fire on this
  generated code — `syneroym-wit-interfaces` inherits `[lints] workspace = true`
  and builds clean today.
- **Nothing in `crates/` or `apps/` imports either module.** `grep` for
  `wit_interfaces::data_layer::` / `wit_interfaces::blob_store::` returns
  nothing. They are compiled and dead.

**Consequence.** B3 does not need to invent a third type vocabulary shared by
the two builds. The guest-generated types *are* that vocabulary, they already
compile on both targets, and this slice gives them their first consumer. The
precedent for using guest-generated types as plain host types is already set:
`syneroym-sdk` re-exports `DeployManifest`/`ServiceConfig`/`Visibility` from
`control_plane::exports::syneroym::control_plane::orchestrator`
([sdk/src/lib.rs:34-41](../../../../crates/sdk/src/lib.rs#L34)).

### F2 — there is no `messaging` guest module; adding one is safe

`crates/wit_interfaces/src` has no `messaging.rs`. **(probed)** adding

```rust
wit_bindgen::generate!({
    world: "messaging-guest",
    path: "wit/messaging/messaging.wit",
    additional_derives: [serde::Serialize, serde::Deserialize]
});
```

compiles cleanly on the host target, even though `messaging-guest` *exports*
`stream-types` and `guest-api` and no `export!` macro is invoked. The generated
`host_api::publish` has type `fn(&str, &[u8]) -> Result<(), MessagingError>`.

### F3 — `syneroym-sdk` cannot be the trait crate

[task.md](task.md)'s open design points leave this open ("a new one, or
`syneroym-sdk`"). It is not open. `syneroym-sdk` depends on `syneroym-router`,
`syneroym-app-orchestration`, `iroh`, `reqwest`, `tokio`, and `ed25519-dalek`
([sdk/Cargo.toml](../../../../crates/sdk/Cargo.toml)). None of that compiles to
`wasm32-wasip2`, and the trait crate must, because the WASM build of the app
depends on it. **New crates.**

### F4 — the host-capability semantics live in exactly one place, and it is reachable from outside `sandbox_wasm`

Every data-layer / blob-store / messaging behaviour a guest sees — the
`read_only` hard-denies, the `data-layer/admin` gate on `drop-collection` /
`execute-ddl` / `query-raw`, FDAE sieve resolution, CLS field masking, stage-4
after-step application, write attribution, DEK resolution, topic namespacing —
is implemented once, in `impl … for HostState`:

| Interface | Impl | Line |
|---|---|---|
| `syneroym:messaging/host-api` | `impl host_api::Host for HostState` | [host_capabilities.rs:364](../../../../crates/sandbox_wasm/src/host_capabilities.rs#L364) |
| `syneroym:data-layer/store` | `impl store::Host for HostState` | [:619](../../../../crates/sandbox_wasm/src/host_capabilities.rs#L619) |
| `syneroym:blob-store/blob-store` | `impl blob_store::Host for HostState` | [:1522](../../../../crates/sandbox_wasm/src/host_capabilities.rs#L1522) |
| `blob-writer` resource | `impl HostBlobWriter for HostState` | [:1603](../../../../crates/sandbox_wasm/src/host_capabilities.rs#L1603) |
| `blob-reader` resource | `impl HostBlobReader for HostState` | [:1645](../../../../crates/sandbox_wasm/src/host_capabilities.rs#L1645) |

`HostState`, `MessagingContext`, `StreamContext` and `empty_service_proxy` are
all `pub` and re-exported from the crate root
([lib.rs:10-15](../../../../crates/sandbox_wasm/src/lib.rs#L10)). `HostState::new`
([:159](../../../../crates/sandbox_wasm/src/host_capabilities.rs#L159)) takes
plain `Arc` handles plus a `CallerContext`; it builds its own `WasiCtx` and
`ResourceTable`. **Nothing in the data-layer or blob-store paths touches
wasmtime beyond `ResourceTable` and the `Resource<T>` handle type.**

The generated `Host` traits are public
(`syneroym_wit_interfaces::host::syneroym::{data_layer::store::Host, blob_store::blob_store::Host, messaging::host_api::Host}`)
and their methods are `fn … -> impl Future<Output = …> + Send where Self: Sized`
(wasmtime 46's `wit-bindgen` emits `Send` supertraits and `where Self: Sized`
bounds for async imports —
`wasmtime-internal-wit-bindgen-46.0.2/src/lib.rs:3190-3300`). So
`StoreHost::put(&mut host_state, collection, value).await` is callable from any
crate that depends on `syneroym-sandbox-wasm` and `syneroym-wit-interfaces`.

**Consequence.** The native shim does not have to reimplement anything. It can
be a thin adapter that owns a `HostState` and calls the same trait impls the
guest reaches. This makes exit criterion 13 ("the native shim and the WASM build
disagree on any interface → the shared suite fails") nearly unfalsifiable for
data-layer and blob-store, and concentrates all genuine risk in messaging (F5)
— which is the honest place for it.

### F5 — messaging's inbound half has no native path, and `register-stream-protocol` has no native meaning

Two of the four `host-api` functions are not build-neutral:

- `subscribe` ([:381](../../../../crates/sandbox_wasm/src/host_capabilities.rs#L381))
  persists the subscription and then calls
  `AppSandboxEngine::register_internal_subscription`
  ([engine.rs:1417](../../../../crates/sandbox_wasm/src/engine.rs#L1417)), which
  spawns a pump that calls `deliver_message`
  ([:1547](../../../../crates/sandbox_wasm/src/engine.rs#L1547)) — and that
  instantiates the **WASM component** and calls
  `syneroym:messaging/guest-api@0.1.0#handle-message` under
  `CallerContext::service_system`, retrying instantiation up to 4 times.
  A native app has no component to instantiate.
- `register_stream_protocol` ([:441](../../../../crates/sandbox_wasm/src/host_capabilities.rs#L441))
  registers `SubstrateEndpoint::WasmChannel { service_id }`, i.e. it hard-codes
  that the protocol handler is a WASM component.

`MqttBroker::subscribe(topic) -> (SubscriptionHandle, Receiver)`
([mqtt_broker/src/lib.rs:181](../../../../crates/mqtt_broker/src/lib.rs#L181)) is
generic, so a native pump is straightforward. `register-stream-protocol` is not,
and nothing in Roym's declared needs uses raw streams — see `D-B3-9`.

### F6 — guest state cannot live in guest memory, and the native build must obey the same rule

`messaging-pubsub-test`'s own world file says it plainly: *"every host
invocation gets a fresh `Store`/instance, so a plain Rust static wouldn't
survive"*
([test-components/messaging-pubsub-test/wit/world.wit](../../../../test-components/messaging-pubsub-test/wit/world.wit)),
and it persists received messages through `data-layer` instead. A native app
holding the same state in a struct field would pass a test the WASM build fails
— the exact class of bug exit criterion 13 exists to catch. `D-B3-12` makes
this a rule of the fixture, not an accident of how it was written.

### F7 — the JSON⇄WIT boundary only round-trips scalars, and errors are asymmetric

- Guest results: `wasm_results_to_json`
  ([conversions.rs:425](../../../../crates/sandbox_wasm/src/conversions.rs#L425))
  turns a guest `Err(payload)` into an `anyhow::Error`, which
  `dispatch_json_rpc_unfenced` then reports as JSON-RPC `-32603`
  ([dispatch.rs:262](../../../../crates/router/src/route_handler/dispatch.rs#L262)).
- Native results: `NativeService::dispatch` returns `RpcResult<NativeResponse>`
  ([rpc/src/native.rs:196](../../../../crates/rpc/src/native.rs#L196)), and
  `RpcError::code()` supplies `-32601`/`-32602`/`-32603`/custom
  ([rpc/src/lib.rs:54-83](../../../../crates/rpc/src/lib.rs#L54)).
- Parameter conversion supports `string`/`u32`/`bool` only, positionally or by
  name; `messaging-pubsub-test`'s world file documents the same limit.

**Consequence.** A fixture whose verbs return rich typed results, or that
signals failure by returning `Err`, would produce *legitimately different*
frames on the two builds and make "identical results" untestable. `D-B3-10` and
`D-B3-11` remove both problems by construction.

### F8 — native dispatch rejects an anonymous caller; WASM admits one

[dispatch.rs:209-212](../../../../crates/router/src/route_handler/dispatch.rs#L209)
rejects `caller: None` for native interfaces; the WASM arm forwards `None` into
`HostState.caller` deliberately (design §6.1.2, quoted in that comment). This is
a settled asymmetry of the *router*, not of the shim, but the parity suite must
never exercise it: both builds run under a real `CallerContext` (`D-B3-14`).

### F9 — a native service reaches the wire through two registrations, and the `supervisor` role is the working precedent

To be dispatchable, a native service needs both:

1. `native_dispatch.insert(id, Arc<dyn NativeService>)`
   ([runtime.rs:854](../../../../crates/substrate/src/runtime.rs#L854)), and
2. `endpoint_registry.register(node_service_id, "<interface>", SubstrateEndpoint::NativeHostChannel { service_id: id })`
   ([runtime.rs:668-690](../../../../crates/substrate/src/runtime.rs#L668)).

`plan_pipeline` then selects `ServiceStage::NativeService`
([dispatch.rs:321](../../../../crates/router/src/route_handler/dispatch.rs#L321)).
The whole supervisor role is gated behind the `supervisor` Cargo feature
([substrate/Cargo.toml:54](../../../../crates/substrate/Cargo.toml#L54)) with a
`#[cfg(not(feature))]` stub that fails loudly. B3 copies this shape exactly.

### F10 — CI runs `--all-features`, so a feature-gated test is a test that runs

`.github/actions/ci-build-and-test/action.yml` runs
`cargo test --workspace --all-targets --all-features`; `ci-lints` runs
`cargo clippy --workspace --all-targets --all-features -- -D warnings`. So code
behind a new substrate feature is compiled *and* tested in CI. The local gate in
[AGENTS.md](../../../../AGENTS.md) is plain `cargo test --workspace`, so §10's
checklist names the extra local command explicitly.

### F11 — test components are separate workspaces today, and one of them copies its WIT deps

All eleven WASM fixtures are in the root `exclude` list
([Cargo.toml:4-15](../../../../Cargo.toml#L4)) and built by
`cd test-components/<x> && cargo component build --release --target wasm32-wasip2`
([mise.toml:30-39](../../../../mise.toml#L30)), each with its own `Cargo.lock`
and its own `target/`. `test_constants` hard-codes those per-component target
paths ([test_constants.rs](../../../../crates/core/src/test_constants.rs)).
`data-layer-test/wit/deps/data-layer/data-layer.wit` is a **symlink** into
`crates/wit_interfaces/wit`; `miniapp-demo1-wasm/wit/deps/*` are **copies**,
which can and will drift. The new fixture symlinks (`D-B3-8`).

`miniapp-demo1-web` is the only test component that is a workspace member — and
it is a standalone TCP binary, not a `NativeService`. **There is no existing
example of one source tree producing both a component and an in-process native
service.** That is what B3 builds.

### F12 — the service id is part of every observable string, so the two builds must share one

`namespace_topic`/`namespace_topic_for_publish`
([mqtt_broker/src/lib.rs:70](../../../../crates/mqtt_broker/src/lib.rs#L70)) turn
`<topic>` into `svc/<service_id>/<topic>`, and it is the **namespaced** topic
that the broker hands back to the delivery pump and that
`deliver_message` passes to the guest as `handle-message`'s first argument
([engine.rs:1442](../../../../crates/sandbox_wasm/src/engine.rs#L1442),
[:1607](../../../../crates/sandbox_wasm/src/engine.rs#L1607)). The same id also
selects the service's SQLite store and the `data-layer/admin` resource
([host_capabilities.rs:1012](../../../../crates/sandbox_wasm/src/host_capabilities.rs#L1012)).
Any app that echoes a delivered topic — this fixture does, §5.3 — therefore
emits the service id, and two ids make two builds disagree on output for a
reason that is not the shim's. `D-B3-17`.

### F13 — `messaging_subscriptions` is replayed into the WASM engine for *every* row, at every boot

[runtime.rs:972](../../../../crates/substrate/src/runtime.rs#L972) calls
`replay_persisted_subscriptions`
([:1054](../../../../crates/substrate/src/runtime.rs#L1054)), which reads
`list_all_messaging_subscriptions()` and hands every `(service_id, topic)` row to
`AppSandboxEngine::register_internal_subscription` — unconditionally, for
whatever service id the row names, before any component is deployed or
restored (so a "skip if not deployed" guard would skip everything and regress
the feature it protects). A native app writing rows to that table would, after
a restart, get a broker subscription pumping into `deliver_message`, which
would spend four instantiation attempts per message on a service that has no
component, and the native subscription would not come back at all. `D-B3-18`.

### F14 — `EndpointRegistry::register` is a silent last-write-wins insert, and `(node_did, "messaging")` is taken

[local_registry.rs:177-189](../../../../crates/core/src/local_registry.rs#L177)
saves and then `insert`s into a `DashMap` keyed by `(service_id, interface_name)`
— no conflict check, no error. `setup_router` already registers
`(node_did, "messaging") -> NativeHostChannel { SUPERVISOR_DISPATCH_ID }`
([runtime.rs:686](../../../../crates/substrate/src/runtime.rs#L686)) behind a
long comment explaining why the supervisor needs that exact key. CI builds with
`--all-features`, so any second registrant of that key silently wins or loses
depending on ordering. `D-B3-19`.

### F15 — the route preamble separator is `|`, and the module's own doc comment says `.`

`PREAMBLE_SEPARATOR = "|"`
([preamble.rs:100](../../../../crates/router/src/preamble.rs#L100)), used by the
parser at [:285](../../../../crates/router/src/preamble.rs#L285). The module
header doc at [:7](../../../../crates/router/src/preamble.rs#L7) writes
`<scheme>://<interface>.<service_id>`, and so does the architecture prose in
[CLAUDE.md](../../../../CLAUDE.md)/[AGENTS.md](../../../../AGENTS.md). Tests
never notice because they build a `RoutePreamble` value or go through
`SyneroymClient::request`
([sdk/src/lib.rs:630](../../../../crates/sdk/src/lib.rs#L630)) rather than
writing the line by hand. §7.4 does the same, and §12.9 records the stale doc.

---

## §2 Decisions

| # | Decision | Why |
|---|---|---|
| **D-B3-1** | **Two new crates.** `crates/app_host/` → `syneroym-app-host` (trait definitions + the guest adapter). `crates/app_host_native/` → `syneroym-app-host-native` (the in-process shim). Neither is `syneroym-sdk`. | F3: `syneroym-sdk` cannot compile to `wasm32-wasip2`. Splitting the shim out keeps `syneroym-app-host` — the crate an app depends on — free of any substrate dependency, so a third-party app never path-depends on `syneroym-sandbox-wasm`. |
| **D-B3-2** | **The shared type vocabulary is the existing wit-bindgen *guest* types**, re-exported from `syneroym-app-host`. No third type set is defined. | F1. The alternative (neutral mirror types) means two conversions instead of one and a third place for a field to be forgotten. Mapping the wasmtime host bindings onto the guest types via `bindgen!`'s `with:` was considered and rejected: `with:`-mapped types must already implement `ComponentType`/`Lift`/`Lower`, which guest types do not. |
| **D-B3-3** | **The traits are async**, declared as `fn f(&self, …) -> impl Future<Output = …> + Send` (AFIT/RPITIT, no `async-trait`, no extra dependency). The guest adapter's impls are `async fn` bodies that never await. | The native side is genuinely async (SQLite via `tokio`, the broker, object storage). Making the trait sync would force the native shim to block a runtime worker (`block_in_place`/`futures::executor::block_on`) on every host call — a real hazard in the build the spec wants to be the *faster* one. The reverse cost is a ~25-line poll-once executor in the guest (`D-B3-4`), which is provably correct because no guest future can pend. |
| **D-B3-4** | **The guest drives futures with a poll-once executor that panics on `Pending`.** `syneroym-app-host::guest::block_on` polls once with a no-op waker; `Poll::Pending` is a fixture bug, not a runtime condition. | Every guest impl returns an already-complete future by construction (the underlying binding call is synchronous). A real executor would be dead weight and would hide the invariant instead of asserting it. |
| **D-B3-5** | **The native shim delegates to `HostState`'s existing `Host` impls.** It does not reimplement, and it does not call `syneroym-data-db` / `syneroym-data-blob` / `MqttBroker` for anything those impls already do. | F4. One implementation of the semantics, two callers. Any other choice creates a *third* adapter beside `HostState` and `SynSvcNativeService` and re-opens every gate, mask, and attribution rule for divergence. |
| **D-B3-6** | **A fresh `HostState` per invocation**, built from a long-lived factory that holds the providers, exactly mirroring the WASM path's fresh `Store` per invocation. Blob resource handles therefore do **not** survive across invocations on either build. | Parity by construction, including the unattractive parts. A shim with a longer-lived resource table would let a fixture open a `blob-writer` in one call and finish it in another — passing natively and failing on WASM. |
| **D-B3-7** | **Substrate Cargo feature `dual_build_fixture`** (default off) links the fixture's native build and registers it, following the `supervisor` role's shape exactly (F9), including a `#[cfg(not(feature))]` arm that fails with a named message. | The spec's packaging table states the native build is selected by a Cargo feature. `supervisor` proves the wiring; copying it means no new mechanism. |
| **D-B3-8** | **One fixture crate, a workspace member**, at `test-components/dual-build-fixture/` (`syneroym-test-dual-build-fixture`), `crate-type = ["cdylib", "rlib"]`, with `#[cfg(target_arch = "wasm32")]` guest wiring and `#[cfg(not(target_arch = "wasm32"))]` native wiring around one shared, target-independent core. WIT deps are **symlinks**. | This *is* "one source tree, two build targets". It also resolves a contradiction in the milestone doc — see `D-B3-15` and §12.1. Cfg-gating the `wit_bindgen::generate!`/`export!` block keeps the host build free of generated `unsafe` and of any doubt about `export!` off-wasm (F1 only proves *imports* compile off-wasm); the wasm build relaxes `unsafe_code` on that one module and keeps every other workspace lint (`D-B3-22`). |
| **D-B3-9** | **`register-stream-protocol` is out of B3's trait surface.** Everything else in the three interfaces is in. | F5: its only implementation registers a `WasmChannel` endpoint, so "the same call on the native build" has no meaning without first designing native raw-stream routing — a bigger question than this slice, and one nothing in R1–R4 needs. Recorded in [deferred-backlog.md](../../deferred-backlog.md) (§11). |
| **D-B3-10** | **The fixture's whole app surface is one verb**: `run: func(request: string) -> result<string, string>`, JSON in, JSON out. | F7: only `string`/`u32`/`bool` cross the JSON⇄WIT boundary, and a single verb means both builds share one dispatch function instead of two parallel sets of exports. It also makes "identical results" a byte comparison of two strings. |
| **D-B3-11** | **Application-level failures are reported inside the JSON payload** (`{"ok": …}` / `{"err": …}`), never as a WIT `Err` or an `RpcError`. The transport-level error shapes are asserted **per build**, in separate tests, and documented as a permitted difference. | F7: the two builds map failure onto the wire differently, and that difference is the router's, not the shim's. Forcing it into the parity comparison would make the suite fail for a reason the shim cannot fix. |
| **D-B3-12** | **The fixture keeps no state in process memory.** Everything observable across calls goes through `data-layer` or `blob-store`. | F6. |
| **D-B3-13** | **No `init`/`migrate` lifecycle hooks.** The fixture ensures its schema lazily, on first use, identically on both builds. | `invoke_lifecycle_hook` skips a component that does not export the hook ([engine.rs:1396](../../../../crates/sandbox_wasm/src/engine.rs#L1396)), so this costs nothing on the WASM side — and there is no native analogue of "the host called `init` at deploy time" to keep in step. `create-collection` is deliberately ungated ([host_capabilities.rs:642-651](../../../../crates/sandbox_wasm/src/host_capabilities.rs#L642)), so lazy ensure works under an ordinary caller on both builds. |
| **D-B3-14** | **The parity suite runs both builds under a real, identical `CallerContext`**, and asserts on the fixture's JSON payload — not on the router's frame. | F8: anonymous callers are treated differently by the router by design; that is out of scope here. |
| **D-B3-17** | **The parity harness gives the two builds one service id and two entirely separate host stacks** — separate `SqliteStorageProvider`, separate blob root, separate `MqttBroker`, separate `KeyStore`, separate temp dir. | F12. The service id is not an implementation detail the suite may vary: it is the store namespace, the topic namespace (`svc/<service_id>/<topic>`), *and* the admin-gate resource ([host_capabilities.rs:1012](../../../../crates/sandbox_wasm/src/host_capabilities.rs#L1012)). Two ids make the *delivered topic string* differ, which lands in the fixture's own output and fails `assert_eq!` for a reason no shim fix can reach. Two brokers are required alongside it, or one build's publish is delivered to the other's subscription. `MqttBroker` opens no listener ([mqtt_broker/src/lib.rs:118-144](../../../../crates/mqtt_broker/src/lib.rs#L118)), so a second instance in one process is free. |
| **D-B3-18** | **The shim does not persist subscriptions.** No `save_messaging_subscription` / `delete_messaging_subscription` call. Subscriptions live in the factory and die with it. Restart behaviour is therefore *not* at parity in B3, and says so in §7.3 and in the backlog. | F13. Writing to that table poisons a shared boot path this slice has no business changing, and the alternative — a native replay hook in `runtime.rs` — is new substrate machinery, not shim work. A stated gap beats a latent one. |
| **D-B3-19** | **The fixture registers exactly one endpoint**: its `test-driver` interface. No `messaging` endpoint. | F14. The supervisor already owns `(node_did, "messaging")`, `register` is a silent last-write-wins insert, and CI enables both features. The fixture has no use for it either way: its `subscribe` is app-initiated and its pump reads the broker directly, never the router. |
| **D-B3-20** | **`NativeAppHost` is a newtype over an `Arc`**, so the traits keep `&self` and a blob writer can hold its host. | A blob resource handle must outlive the `open_upload` call that produced it but not the invocation. A `&self`-borrowing writer would put a lifetime in `AppBlobStore::Writer`, which infects the trait — and therefore the *WASM* build, which has no such problem. Paying one `Arc` clone per upload is the smaller cost. |
| **D-B3-21** | **The fixture's service id is supplied by the embedder**, never a `const` in the fixture. It is a field on the factory and a parameter on the fixture's constructor. | F15. It selects the store namespace, the topic namespace, and the admin-gate resource. A `const` in the app would silently disagree with the id the substrate registered it under. |
| **D-B3-22** | **The fixture inherits workspace lints** (`[lints] workspace = true`), with a module-scoped `#[allow(unsafe_code)]` on the cfg-gated bindings module only. | Opting the crate out to dodge generated `unsafe` would also drop the deny-level `correctness`/`suspicious` groups from a crate that is about to grow B4's conversation logic. |
| **D-B3-15** | **`D-06B-6`'s "excluded from the workspace build graph" is amended, not silently overridden.** The fixture must be a workspace member for `syneroym-substrate` to link it. [task.md](task.md)'s `D-06B-6` row gets an edit noting this (§11). | Amending an input document in the open is the house rule (`D-06B-4` did the same to M05C's S4 row). |

---

## §3 `syneroym-app-host` — the traits

**New crate.** `crates/app_host/Cargo.toml`:

```toml
[package]
name = "syneroym-app-host"
version.workspace = true
edition.workspace = true
authors.workspace = true
license.workspace = true
repository.workspace = true

[lints]
workspace = true

[dependencies]
syneroym-wit-interfaces.workspace = true
```

No other dependency. It must stay this way — anything heavier is a dependency
the WASM build of every app pays for.

### 3.1 Re-exported types (`src/types.rs`)

```rust
//! The type vocabulary both builds share: the wit-bindgen *guest* types,
//! which compile for `wasm32-wasip2` and for the host alike (the host build
//! links them against stub imports it never calls).

pub mod data_layer {
    pub use syneroym_wit_interfaces::data_layer::syneroym::data_layer::store::{
        CollectionSchema, DataLayerError, IndexDefinition, IndexType, Mutation, PatchMutation,
        QueryOptions, QueryResult, RawQueryResult, RecordReadValue, RecordWriteValue, SqlValue,
    };
}

pub mod blob_store {
    pub use syneroym_wit_interfaces::blob_store::syneroym::blob_store::blob_store::BlobError;
}

pub mod messaging {
    pub use syneroym_wit_interfaces::messaging::syneroym::messaging::host_api::MessagingError;
}
```

`messaging` requires §9.1's new module in `syneroym-wit-interfaces`.

### 3.2 `src/lib.rs` — trait definitions

```rust
#![cfg_attr(target_arch = "wasm32", allow(clippy::future_not_send))]
//! One trait per host interface. Two implementations: `guest` (this crate,
//! `wasm32` only) over `wit-bindgen` bindings, and `syneroym-app-host-native`
//! over the substrate's own host-capability implementations. An app is
//! written once against these traits and built both ways.

use core::future::Future;

pub mod types;
#[cfg(target_arch = "wasm32")]
pub mod guest;

use types::{blob_store::BlobError, data_layer::*, messaging::MessagingError};

/// Everything an app may reach. One bound for an app to be generic over.
pub trait AppHost: AppDataLayer + AppBlobStore + AppMessaging + Send + Sync {}
impl<T> AppHost for T where T: AppDataLayer + AppBlobStore + AppMessaging + Send + Sync {}

/// Mirrors `syneroym:data-layer/store@0.1.0`, function for function.
pub trait AppDataLayer {
    fn create_collection(
        &self,
        schema: CollectionSchema,
    ) -> impl Future<Output = Result<(), DataLayerError>> + Send;

    fn drop_collection(&self, name: String)
        -> impl Future<Output = Result<(), DataLayerError>> + Send;

    fn put(
        &self,
        collection: String,
        value: RecordWriteValue,
    ) -> impl Future<Output = Result<(), DataLayerError>> + Send;

    fn patch(
        &self,
        collection: String,
        id: String,
        patch_json: Vec<u8>,
    ) -> impl Future<Output = Result<(), DataLayerError>> + Send;

    fn get(
        &self,
        collection: String,
        id: String,
    ) -> impl Future<Output = Result<Option<RecordReadValue>, DataLayerError>> + Send;

    fn query(
        &self,
        collection: String,
        opts: QueryOptions,
    ) -> impl Future<Output = Result<QueryResult, DataLayerError>> + Send;

    fn aggregate(
        &self,
        collection: String,
        pipeline: String,
    ) -> impl Future<Output = Result<RawQueryResult, DataLayerError>> + Send;

    fn delete(
        &self,
        collection: String,
        id: String,
    ) -> impl Future<Output = Result<(), DataLayerError>> + Send;

    fn delete_many(
        &self,
        collection: String,
        filter: String,
    ) -> impl Future<Output = Result<u64, DataLayerError>> + Send;

    fn batch_mutate(
        &self,
        collection: String,
        mutations: Vec<Mutation>,
    ) -> impl Future<Output = Result<(), DataLayerError>> + Send;

    fn execute_ddl(&self, sql: String)
        -> impl Future<Output = Result<(), DataLayerError>> + Send;

    fn query_raw(
        &self,
        sql: String,
        params: Vec<SqlValue>,
    ) -> impl Future<Output = Result<RawQueryResult, DataLayerError>> + Send;

    fn check_access(
        &self,
        collection: String,
        id: String,
        operation: String,
    ) -> impl Future<Output = Result<bool, DataLayerError>> + Send;
}

/// Mirrors `syneroym:blob-store/blob-store@0.1.0`. The two resources become
/// associated types: a guest holds a wit-bindgen handle, the native shim
/// holds a `ResourceTable` index plus a handle back to its host.
pub trait AppBlobStore {
    type Writer: AppBlobWriter;
    type Reader: AppBlobReader;

    fn put_blob(&self, data: Vec<u8>)
        -> impl Future<Output = Result<String, BlobError>> + Send;

    fn get_blob(&self, hash: String)
        -> impl Future<Output = Result<Vec<u8>, BlobError>> + Send;

    fn open_upload(&self) -> impl Future<Output = Result<Self::Writer, BlobError>> + Send;

    fn open_download(
        &self,
        hash: String,
        offset: u64,
    ) -> impl Future<Output = Result<Self::Reader, BlobError>> + Send;

    fn delete_blob(&self, hash: String)
        -> impl Future<Output = Result<(), BlobError>> + Send;

    fn signed_url(
        &self,
        hash: String,
        ttl_secs: u32,
    ) -> impl Future<Output = Result<String, BlobError>> + Send;
}

pub trait AppBlobWriter: Send {
    fn write(&mut self, chunk: Vec<u8>)
        -> impl Future<Output = Result<(), BlobError>> + Send;
    /// Consumes the writer: a finished upload cannot be written to again on
    /// either build (the host deletes its table entry; the guest's handle is
    /// dropped here rather than left dangling).
    fn finish(self) -> impl Future<Output = Result<String, BlobError>> + Send;
    fn abort(self) -> impl Future<Output = ()> + Send;
}

pub trait AppBlobReader: Send {
    fn read(&mut self, max_bytes: u32)
        -> impl Future<Output = Result<Vec<u8>, BlobError>> + Send;
}

/// Mirrors `syneroym:messaging/host-api@0.1.0`, minus
/// `register-stream-protocol`, whose only implementation registers a WASM
/// endpoint and so has no native counterpart.
pub trait AppMessaging {
    fn publish(&self, topic: String, payload: Vec<u8>)
        -> impl Future<Output = Result<(), MessagingError>> + Send;
    fn subscribe(&self, topic: String)
        -> impl Future<Output = Result<(), MessagingError>> + Send;
    fn unsubscribe(&self, topic: String)
        -> impl Future<Output = Result<(), MessagingError>> + Send;
}
```

Notes an implementer must not "simplify" away:

- `AppBlobWriter::finish` and `abort` take `self` **by value** even though the
  guest binding takes `&self` (F1). This is the one place the trait is
  deliberately *stricter* than one of its two implementations, so that a
  double-`finish` is a compile error rather than a build-dependent runtime
  error.
- Methods take owned arguments (`String`, `Vec<u8>`), not references. The guest
  adapter re-borrows for free; a borrowing signature would put a lifetime into
  every returned `impl Future` and infect the fixture.

### 3.3 `src/guest.rs` — the WASM implementation

```rust
#![cfg(target_arch = "wasm32")]
//! The `wit-bindgen` implementation of the traits. Every call here is
//! synchronous at the ABI, so every future returned is already complete --
//! see `block_on`.

use syneroym_wit_interfaces::{
    blob_store::syneroym::blob_store::blob_store as bs,
    data_layer::syneroym::data_layer::store as dl,
    messaging::syneroym::messaging::host_api as msg,
};

use crate::{AppBlobReader, AppBlobStore, AppBlobWriter, AppDataLayer, AppMessaging, types::*};

/// The app's handle to the host in the WASM build. Zero-sized: the component
/// model already binds the imports.
#[derive(Debug, Clone, Copy, Default)]
pub struct GuestHost;

impl AppDataLayer for GuestHost {
    async fn put(&self, collection: String, value: RecordWriteValue)
        -> Result<(), DataLayerError>
    {
        dl::put(&collection, &value)
    }
    // ... one line per function, same shape ...
}

impl AppBlobStore for GuestHost {
    type Writer = GuestBlobWriter;
    type Reader = GuestBlobReader;

    async fn open_upload(&self) -> Result<GuestBlobWriter, BlobError> {
        bs::open_upload().map(GuestBlobWriter)
    }
    // ...
}

pub struct GuestBlobWriter(bs::BlobWriter);

impl AppBlobWriter for GuestBlobWriter {
    async fn write(&mut self, chunk: Vec<u8>) -> Result<(), BlobError> {
        self.0.write(&chunk)
    }
    async fn finish(self) -> Result<String, BlobError> {
        self.0.finish()          // `self` dropped here: no second call possible
    }
    async fn abort(self) {
        self.0.abort();
    }
}

/// Drives an already-complete future to its value.
///
/// Correct only because every future this crate's guest implementations
/// return is complete on first poll: each wraps one synchronous component-
/// model call. A `Pending` here means an app awaited something that is not a
/// host call, which the WASM build cannot support -- so it panics loudly
/// rather than spinning.
pub fn block_on<F: core::future::Future>(fut: F) -> F::Output {
    use core::{
        pin::pin,
        task::{Context, Poll, RawWaker, RawWakerVTable, Waker},
    };
    const VTABLE: RawWakerVTable = RawWakerVTable::new(
        |_| RawWaker::new(core::ptr::null(), &VTABLE),
        |_| {}, |_| {}, |_| {},
    );
    // SAFETY: the vtable's operations are all no-ops on a null pointer.
    let waker = unsafe { Waker::from_raw(RawWaker::new(core::ptr::null(), &VTABLE)) };
    match pin!(fut).poll(&mut Context::from_waker(&waker)) {
        Poll::Ready(v) => v,
        Poll::Pending => panic!(
            "guest future pended: the WASM build can only await host calls, \
             which never pend"
        ),
    }
}
```

**Prefer `Waker::noop()` over the vtable above** — the pinned toolchain is
`stable` (1.96.0 as of this writing, `rust-toolchain.toml`), well past
`Waker::noop`'s stabilisation, so `block_on` can be written with no `unsafe` at
all and no `#[allow(unsafe_code)]` exception to the workspace lint. The vtable
form is shown only so the mechanism is legible; write the `noop` form.

---

## §4 `syneroym-app-host-native` — the shim

**New crate.** `crates/app_host_native/Cargo.toml`:

```toml
[package]
name = "syneroym-app-host-native"
# ... workspace inherits ...

[dependencies]
syneroym-app-host.workspace = true
syneroym-wit-interfaces.workspace = true
syneroym-sandbox-wasm.workspace = true
syneroym-data-db.workspace = true
syneroym-data-blob.workspace = true
syneroym-data-keystore.workspace = true
syneroym-mqtt-broker.workspace = true
syneroym-rpc.workspace = true
syneroym-core.workspace = true
syneroym-app-orchestration.workspace = true
tokio.workspace = true
tracing.workspace = true

[dev-dependencies]
syneroym-test-dual-build-fixture.workspace = true
tempfile.workspace = true
serde_json.workspace = true
```

Both new crates and the fixture are added to `[workspace.dependencies]` in the
root `Cargo.toml` (§9.2).

### 4.1 `NativeHostFactory` — long-lived

```rust
/// Everything the shim needs that outlives one call. One per native app
/// instance, held by whoever registered that app.
#[derive(Debug)]
pub struct NativeHostFactory {
    /// Supplied by the embedder, never defaulted: it selects the service's
    /// SQLite store, its broker topic namespace, and the resource its
    /// `data-layer/admin` gate checks against.
    service_id: String,
    key_store: Arc<KeyStore>,
    storage_provider: Arc<dyn StorageProvider>,
    blob_provider: Arc<dyn BlobProvider>,
    broker: Arc<MqttBroker>,
    endpoint_registry: EndpointRegistry,
    logical_resolver: Arc<LogicalResolver>,
    /// Live broker subscriptions, keyed by *namespaced* topic -- the native
    /// analogue of `AppSandboxEngine.subscriptions`.
    subscriptions: DashMap<String, SubscriptionHandle>,
    /// The app's inbound message entry point. `Weak`, not `Arc`: the app
    /// holds this factory, so a strong reference back is the same
    /// uncollectable cycle `HostState.service_proxy` and
    /// `ControlPlaneService.native_dispatch` already guard against.
    sink: OnceLock<Weak<dyn MessageSink>>,
}

/// The host -> app direction. The WASM build's equivalent is the exported
/// `syneroym:messaging/guest-api@0.1.0#handle-message`.
///
/// **Defined in `syneroym-app-host`, not here** -- it is part of the
/// app-facing contract (it is how an app *receives*), and the fixture must
/// not depend on the shim crate. `syneroym-app-host-native` re-exports it.
/// `async_trait` rather than AFIT because this one is used as
/// `dyn MessageSink`.
#[async_trait::async_trait]
pub trait MessageSink: Send + Sync + core::fmt::Debug {
    async fn handle_message(&self, topic: String, payload: Vec<u8>) -> Result<(), String>;
}
```

`sink` is a `OnceLock` set after construction, exactly as
`ControlPlaneService.service_proxy` is
([synsvc_native.rs](../../../../crates/control_plane/src/synsvc_native.rs)),
because the app and the factory are constructed in that order.

Methods:

```rust
impl NativeHostFactory {
    pub fn new(service_id: String, /* the other fields above */) -> Arc<Self>;
    pub fn set_sink(&self, sink: Weak<dyn MessageSink>);   // panics if set twice
    /// One host handle for one invocation: a fresh `HostState`, exactly as
    /// the sandbox builds a fresh `Store` per guest call.
    pub fn host_for(self: &Arc<Self>, caller: CallerContext) -> NativeAppHost;
    /// Drops every live subscription -- the explicit analogue of
    /// `AppSandboxEngine::unsubscribe_all`, which `ControlPlaneService::
    /// undeploy` calls
    /// (`crates/control_plane/src/service/orchestration.rs:2681`).
    ///
    /// A linked app has no undeploy path, so **nothing in the substrate
    /// calls this**; dropping the factory is the real teardown, and
    /// `SubscriptionHandle`'s own `Drop` unsubscribes. It exists for the
    /// parity suite, which tears one stack down while the process keeps
    /// running, and it is idempotent. Do not delete it as dead code without
    /// checking that caller.
    pub fn shutdown(&self);
}
```

### 4.2 `NativeAppHost` — per invocation

```rust
/// One invocation's handle to the host. A cheap `Arc` newtype, so a blob
/// resource returned by `open_upload` can hold its host without borrowing it
/// -- which is what keeps the trait methods on `&self` and keeps a lifetime
/// out of `AppBlobStore::Writer`.
#[derive(Debug, Clone)]
pub struct NativeAppHost(Arc<HostInner>);

#[derive(Debug)]
struct HostInner {
    factory: Arc<NativeHostFactory>,
    /// `tokio::sync::Mutex`, not `std`: the guarded calls are async, and the
    /// guard is held across them. `HostState` is `Send` (wasmtime requires it
    /// for async stores) but not `Sync`, which is exactly what a `Mutex`
    /// fixes.
    state: tokio::sync::Mutex<HostState>,
}
```

Construction (`NativeHostFactory::host_for`):

```rust
let state = HostState::new(
    self.service_id.clone(),
    None,                                  // max_memory_bytes: no wasm memory to bound
    self.key_store.clone(),
    self.storage_provider.clone(),
    self.blob_provider.clone(),
    caller,
    0,                                     // config_generation
    MessagingContext { broker: self.broker.clone(), engine: Weak::new() },
    StreamContext { registry: self.endpoint_registry.clone(), engine: Weak::new() },
    empty_service_proxy(),
    None,                                  // fdae_policy -- see D-B3-16 note below
    false,                                 // read_only
    empty_row_authorizer(),
    None,                                  // app_instance_id
    self.logical_resolver.clone(),
);
```

`MessagingContext.engine` is deliberately an empty `Weak`: the shim never
routes `subscribe` through `HostState` (4.4), so it is never upgraded. The same
holds for `StreamContext.engine`, since `register-stream-protocol` is out of
scope.

`read_only` is hardcoded `false` because its only producer is the stage-4 ABAC
after-step, which has no native path. That leaves the `read_only` hard-denies at
the head of nearly every host-capability function unexercised on both sides of
the parity suite — a real hole in F4's "the shim inherits every gate" argument.
Close it with a **shim unit test** (not a parity scenario, which cannot reach
it): build a `NativeAppHost` with `read_only: true` and assert `put`,
`delete`, `publish`, `put_blob` and `signed_url` all deny. The WASM side of the
same gate is already covered by the ABAC suite.

**`fdae_policy: None` is a scope boundary, not an oversight.** The WASM path
loads a deployed service's compiled policy; there is no deploy record for a
natively linked app, so a policy would have nowhere to come from in B3. The
parity suite therefore runs policy-absent on both sides. Recorded in
[deferred-backlog.md](../../deferred-backlog.md) (§11) — a native Roym under an
FDAE policy is an M06C question.

### 4.3 Data-layer and blob-store: pure delegation

```rust
use syneroym_wit_interfaces::host::syneroym::{
    blob_store::blob_store::{Host as HostBlobStore, HostBlobReader, HostBlobWriter},
    data_layer::store::Host as HostStore,
};

impl AppDataLayer for NativeAppHost {
    async fn put(&self, collection: String, value: app::RecordWriteValue)
        -> Result<(), app::DataLayerError>
    {
        let mut state = self.0.state.lock().await;
        HostStore::put(&mut *state, collection, convert::write_value_in(value))
            .await
            .map_err(convert::data_layer_error_out)
    }
    // ... one such body per function ...
}
```

Every method is this shape: lock, convert arguments *in*, call the `HostState`
impl, convert the result *out*. **No branching, no gates, no defaults.** A
reviewer should be able to check the whole impl by eye.

Blob resources:

```rust
#[derive(Debug)]
pub struct NativeBlobWriter {
    host: NativeAppHost,               // an Arc clone, not a borrow
    rep: u32,                          // the ResourceTable index open_upload produced
}

impl AppBlobWriter for NativeBlobWriter {
    async fn write(&mut self, chunk: Vec<u8>) -> Result<(), BlobError> {
        let mut state = self.host.0.state.lock().await;
        HostBlobWriter::write(&mut *state, Resource::new_own(self.rep), chunk).await
            .map_err(convert::blob_error_out)
    }
    async fn finish(self) -> Result<String, BlobError> {
        let mut state = self.host.0.state.lock().await;
        HostBlobWriter::finish(&mut *state, Resource::new_own(self.rep)).await
            .map_err(convert::blob_error_out)
    }
    async fn abort(self) {
        let mut state = self.host.0.state.lock().await;
        HostBlobWriter::abort(&mut *state, Resource::new_own(self.rep)).await;
    }
}
```

The `rep` (rather than a stored `Resource<T>`) is deliberate: `Resource<T>` is
not `Clone` in a way that survives being handed to a `&mut self` host method
twice, and the host methods take it by value. Storing the raw index and
rebuilding `Resource::new_own(rep)` per call is what `HostBlobWriter`'s own
implementation expects — `write` does `table.get_mut(&self_)` and `finish`/`abort`
do `table.delete(self_)`
([host_capabilities.rs:1603-1645](../../../../crates/sandbox_wasm/src/host_capabilities.rs#L1603)),
so the table, not the handle, owns lifetime. **Settle the exact spelling in
Step 1's spike** (it is the one place the shim touches wasmtime's resource ABI)
and pin it with a comment. If `Resource::new_own` turns out to be the wrong
constructor for a host-owned rep, the fallback is for the shim to keep its own
`HashMap<u32, Box<dyn UploadSession>>` the way `SynSvcNativeService` already does
([synsvc_native.rs:63-65](../../../../crates/control_plane/src/synsvc_native.rs#L63))
— at the cost of leaving `HostState`'s resource methods unexercised natively,
which §7.3 would then have to record.

### 4.4 Messaging: two of three delegate, one does not

```rust
impl AppMessaging for NativeAppHost {
    async fn publish(&self, topic: String, payload: Vec<u8>) -> Result<(), MessagingError> {
        let mut state = self.0.state.lock().await;
        HostMessaging::publish(&mut *state, topic, payload).await.map_err(convert::msg_error_out)
        // -> namespace_topic_for_publish + broker.publish, identical to the guest path
    }

    async fn subscribe(&self, topic: String) -> Result<(), MessagingError> {
        self.0.factory.subscribe(topic).await
    }

    async fn unsubscribe(&self, topic: String) -> Result<(), MessagingError> {
        self.0.factory.unsubscribe(topic).await
    }
}
```

`NativeHostFactory::subscribe` mirrors
`host_api::Host::subscribe` + `register_internal_subscription`
([host_capabilities.rs:381](../../../../crates/sandbox_wasm/src/host_capabilities.rs#L381),
[engine.rs:1417](../../../../crates/sandbox_wasm/src/engine.rs#L1417)) step for
step — **except that it does not persist the subscription** (`D-B3-18`, F13):

```
subscribe(topic):
    namespaced = namespace_topic(service_id, topic)          # same helper, same crate
    sink = self.sink.get().and_then(Weak::upgrade)
        or return Err(Internal("no message sink registered"))  # checked first, same order the
                                                               # guest path checks the engine
    if subscriptions.contains_key(namespaced): return Ok(())   # idempotent, same as the guest
    (handle, mut rx) = broker.subscribe(namespaced)?
    spawn:
        while let Some((topic, payload)) = rx.recv().await:
            let Some(sink) = weak_sink.upgrade() else { break }
            if let Err(e) = sink.handle_message(topic, payload).await:
                warn!(...)                                     # a failed delivery is logged and
                                                               # dropped, matching deliver_message
    subscriptions.insert(namespaced, handle)

unsubscribe(topic):
    namespaced = namespace_topic(service_id, topic)
    subscriptions.remove(namespaced)      # dropping SubscriptionHandle unsubscribes
    Ok(())
```

**Why no `save_messaging_subscription`.** The guest path persists so that
`replay_persisted_subscriptions` can restore the subscription at boot
([runtime.rs:972](../../../../crates/substrate/src/runtime.rs#L972)). That
replay hands **every** row to `AppSandboxEngine::register_internal_subscription`
regardless of which service id it names, and it runs before any component is
deployed or restored — so a row written by a natively linked app produces, after
every restart, a live broker subscription pumping into a sandbox that has no
component to deliver to (four instantiation attempts and a warning per message),
while the native subscription itself is not restored at all. Guarding the replay
on `is_deployed` ([engine.rs:936](../../../../crates/sandbox_wasm/src/engine.rs#L936))
does not work either: at line 972 nothing is deployed yet, so the guard would
skip every row and regress the feature it was meant to protect.

The consequence is stated, not hidden: **native subscriptions do not survive a
restart in B3**, §7.3 asserts the two builds' restart behaviour is a known
difference, and §11 carries a backlog row for the native replay hook that would
close it. That hook is new `runtime.rs` machinery, not shim work.

**Deliberate difference to document in the code**: `deliver_message` retries
component instantiation up to 4 times with a 50 ms backoff
([engine.rs:1549-1587](../../../../crates/sandbox_wasm/src/engine.rs#L1549)).
There is nothing to instantiate natively, so there is nothing to retry. The
retry exists to absorb wasmtime pool pressure, not to add delivery guarantees,
so its absence changes no observable contract. Say so in a comment at the pump.

### 4.5 Type conversions (`src/convert.rs`)

One function per direction per type, guest-vocabulary ⇄ host-vocabulary. Both
sides are generated from the same `.wit`, so every one is a field-for-field
copy. Full list:

| Guest type (`…::data_layer::syneroym::data_layer::store`) | Host type (`…::host::syneroym::data_layer::store`) | Direction needed |
|---|---|---|
| `IndexType`, `IndexDefinition`, `CollectionSchema` | same names | in |
| `RecordWriteValue`, `PatchMutation`, `Mutation` | same names | in |
| `QueryOptions`, `SqlValue` | same names | in |
| `RecordReadValue`, `QueryResult`, `RawQueryResult` | same names | out |
| `DataLayerError` | same name | out |
| `BlobError` (`…::blob_store::…`) | same name | out |
| `MessagingError` (`…::messaging::host_api`) | same name | out |

Add `src/convert.rs` unit tests that construct each type with distinct values
in every field and assert a round trip. **This is where a forgotten field
hides**, and it is cheap to fence off.

---

## §5 The fixture

**New workspace member** `test-components/dual-build-fixture/`, package
`syneroym-test-dual-build-fixture`.

### 5.1 `Cargo.toml`

```toml
[package]
name = "syneroym-test-dual-build-fixture"
version.workspace = true
edition.workspace = true

[lints]
workspace = true      # the generated `unsafe` is allowed at the bindings
                      # module only -- see src/guest.rs

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
syneroym-app-host.workspace = true
serde = { workspace = true, features = ["derive"] }
serde_json.workspace = true

[target.'cfg(target_arch = "wasm32")'.dependencies]
wit-bindgen.workspace = true

[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
syneroym-rpc.workspace = true
async-trait.workspace = true

[package.metadata.component.target.dependencies]
"syneroym:data-layer" = { path = "wit/deps/data-layer" }
"syneroym:blob-store" = { path = "wit/deps/blob-store" }
"syneroym:messaging"  = { path = "wit/deps/messaging" }
```

`wit/deps/<x>/<x>.wit` are **symlinks** into `crates/wit_interfaces/wit/`
(`D-B3-8`, F11):

```bash
ln -s ../../../../../crates/wit_interfaces/wit/data-layer/data-layer.wit \
      test-components/dual-build-fixture/wit/deps/data-layer/data-layer.wit
```

The fixture deliberately does **not** depend on `syneroym-app-host-native`. The
embedder picks the host implementation; the app never names it.

### 5.2 `wit/world.wit`

```wit
package syneroym-test:dual-build-fixture@0.1.0;

interface test-driver {
    /// The fixture's entire surface. `request` and the success payload are
    /// JSON documents; application-level failures are reported inside the
    /// success payload, so the WIT `Err` arm carries only fixture-internal
    /// faults (malformed request JSON) -- the two builds map a WIT `Err`
    /// onto the wire through different code, and this keeps that difference
    /// out of every ordinary result.
    run: func(request: string) -> result<string, string>;
}

world dual-build-fixture {
    import syneroym:data-layer/store@0.1.0;
    import syneroym:blob-store/blob-store@0.1.0;
    import syneroym:messaging/host-api@0.1.0;

    // Exported, not imported: `guest-api` uses `stream-cursor`/`stream-sink`
    // from `stream-types`, which are guest-implemented, so the world must
    // declare `stream-types` in the export direction even though this
    // fixture declines both stream exports. Same shape as
    // `messaging-pubsub-test`'s world.
    export syneroym:messaging/stream-types@0.1.0;
    export syneroym:messaging/guest-api@0.1.0;

    export test-driver;
}
```

### 5.3 `src/app.rs` — the shared core (target-independent)

```rust
//! The fixture's whole behaviour. Compiled unchanged into both builds; it
//! names no build-specific type and calls nothing but `syneroym-app-host`.

use syneroym_app_host::{AppBlobStore, AppBlobWriter, AppBlobReader, AppDataLayer,
                        AppHost, AppMessaging, types::data_layer::*};

const MESSAGES: &str = "messages";
const INBOX: &str = "inbox";

#[derive(serde::Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum Request {
    /// data-layer: ensure schema, write `count` rows, read them back.
    StoreMessages { count: u32 },
    /// data-layer: page through `messages` with an explicit limit.
    ReadMessages { limit: u32 },
    /// data-layer: a mutation the caller is not allowed to make
    /// (`execute-ddl` without `data-layer/admin`), to prove both builds deny
    /// identically.
    AdminDdl { sql: String },
    /// blob-store one-shot round trip.
    PutBlob { body: String },
    GetBlob { hash: String },
    /// blob-store streaming round trip through the resources.
    StreamBlob { chunks: Vec<String>, read_chunk: u32 },
    /// messaging: subscribe, then publish to self; the delivery lands in
    /// `inbox` via `handle_message`.
    SubscribeTopic { topic: String },
    PublishTopic { topic: String, payload: String },
    /// messaging: read what `handle_message` stored. Never in-process
    /// state: every WASM invocation gets a fresh instance, so a static
    /// would not survive between a delivery and this read.
    ReadInbox,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Response { Ok(serde_json::Value), Err(String) }

pub async fn run<H: AppHost>(host: &H, request: &str) -> Result<String, String> {
    let req: Request = serde_json::from_str(request)
        .map_err(|e| format!("malformed request: {e}"))?;   // the only WIT `Err`
    let response = match dispatch(host, req).await {
        Ok(v) => Response::Ok(v),
        Err(e) => Response::Err(e),
    };
    serde_json::to_string(&response).map_err(|e| e.to_string())
}

async fn dispatch<H: AppHost>(host: &H, req: Request) -> Result<serde_json::Value, String> {
    match req {
        Request::StoreMessages { count } => {
            ensure_collection(host, MESSAGES).await?;        // lazy: no init hook
            for i in 0..count {
                host.put(MESSAGES.into(), RecordWriteValue {
                    id: format!("m{i}"),
                    payload: format!(r#"{{"seq":{i}}}"#).into_bytes(),
                }).await.map_err(fmt_err)?;
            }
            let page = host.query(MESSAGES.into(),
                QueryOptions { filter: None, limit: Some(count), cursor: None })
                .await.map_err(fmt_err)?;
            Ok(json!({ "written": count, "read": page.records.len() }))
        }
        Request::StreamBlob { chunks, read_chunk } => {
            let mut w = host.open_upload().await.map_err(fmt_err)?;
            for c in &chunks { w.write(c.clone().into_bytes()).await.map_err(fmt_err)?; }
            let hash = w.finish().await.map_err(fmt_err)?;    // consumes `w`
            let mut r = host.open_download(hash.clone(), 0).await.map_err(fmt_err)?;
            let mut out = Vec::new();
            loop {
                let part = r.read(read_chunk).await.map_err(fmt_err)?;
                if part.is_empty() { break }
                out.extend_from_slice(&part);
            }
            Ok(json!({ "hash": hash, "bytes": out.len(),
                       "body": String::from_utf8_lossy(&out) }))
        }
        // ... one arm per Request variant ...
    }
}

/// Called by both builds when a subscribed message arrives -- from the
/// exported `guest-api::handle-message` on WASM, from the shim's broker pump
/// natively. Persists through `data-layer`, never in process memory.
///
/// `topic` arrives fully namespaced (`svc/<service_id>/<topic>`) on both
/// builds, and is stored verbatim.
pub async fn on_message<H: AppHost>(host: &H, topic: String, payload: Vec<u8>)
    -> Result<(), String>
{
    ensure_collection(host, INBOX).await?;
    host.put(INBOX.into(), RecordWriteValue {
        id: format!("{topic}:{}", stable_id(&payload)),
        payload: serde_json::to_vec(&json!({
            "topic": topic,
            "payload": String::from_utf8_lossy(&payload),
        })).map_err(|e| e.to_string())?,
    }).await.map_err(fmt_err)
}
```

`fmt_err` renders any host error as `format!("{e:?}")`. Both builds must render
identically — they do, because the error *types* are the same guest-generated
types on both sides (`D-B3-2`). **That is the single most valuable consequence
of `D-B3-2`**: without it, "permission denied" would stringify differently per
build and the suite would fail on cosmetics.

`ensure_collection` calls `create_collection` and treats an
already-exists error as success; check what `data_db` actually returns for a
repeat `create_collection` during Step 4 and pin the behaviour with a test on
both builds.

### 5.4 `src/guest.rs` — WASM wiring

```rust
#![cfg(target_arch = "wasm32")]

/// The only place generated `unsafe` enters this crate, so the workspace's
/// `unsafe_code = "deny"` is relaxed here and nowhere else -- the crate keeps
/// every other workspace lint, including the deny-level correctness and
/// suspicious groups.
#[allow(unsafe_code)]
mod bindings {
    wit_bindgen::generate!({
        path: "wit",
        world: "dual-build-fixture",
        with: {
            "syneroym:data-layer/store@0.1.0": generate,
            "syneroym:blob-store/blob-store@0.1.0": generate,
            "syneroym:messaging/host-api@0.1.0": generate,
            "syneroym:messaging/stream-types@0.1.0": generate,
        },
    });
    use super::Fixture;
    export!(Fixture);
}

struct Fixture;

impl bindings::exports::syneroym_test::dual_build_fixture::test_driver::Guest for Fixture {
    fn run(request: String) -> Result<String, String> {
        syneroym_app_host::guest::block_on(crate::app::run(
            &syneroym_app_host::guest::GuestHost, &request))
    }
}

impl bindings::exports::syneroym::messaging::guest_api::Guest for Fixture {
    fn handle_message(topic: String, payload: Vec<u8>) -> Result<(), String> {
        syneroym_app_host::guest::block_on(crate::app::on_message(
            &syneroym_app_host::guest::GuestHost, topic, payload))
    }
    fn handle_stream_request(..) -> Result<..> { Err("not supported".into()) }
    fn accept_stream_upload(..)  -> Result<..> { Err("not supported".into()) }
}

impl bindings::exports::syneroym::messaging::stream_types::Guest for Fixture {
    type StreamCursor = Never;    // declined, like messaging-pubsub-test
    type StreamSink   = Never;
}
```

**Open point for Step 4**: `syneroym-app-host`'s `guest` module declares the
imports by depending on `syneroym-wit-interfaces`' generated bindings, while the
fixture's own `generate!` declares the same imports again. Two `generate!`
invocations for the same interface in one linked artifact produce two sets of
`extern` declarations against the *same* `wasm_import_module`/`link_name`, which
the linker merges — this is expected to work and is how a library-plus-binary
guest is normally structured, but it is unproven in this tree. **Verify it in
Step 1 (§10) with a throw-away component before any other code is written.** If
it does not link, the fallback is to move `GuestHost` out of `syneroym-app-host`
and into the fixture (and, later, into Roym), which costs a duplicated adapter
per app but changes nothing else in this plan.

### 5.5 `src/native.rs` — in-process wiring

> **House rule for every sketch in this plan.** The decision ids (`D-B3-n`)
> belong to this document only. [AGENTS.md](../../../../AGENTS.md) forbids
> milestone/slice/decision references in code comments — ADR references are the
> only permitted kind. Where a sketch below explains *why*, it states the
> reason, not the id. Carry that discipline into the tree.

```rust
#![cfg(not(target_arch = "wasm32"))]

/// The fixture as a substrate-dispatchable native service. Generic over the
/// host so it names no shim type -- the embedder supplies one, along with the
/// service id it was registered under.
pub struct NativeFixture<H: AppHost + 'static> {
    service_id: String,
    host_for: Box<dyn Fn(CallerContext) -> H + Send + Sync>,
}

/// `NativeService` requires `Debug`, and a boxed closure has none. Hand-written
/// rather than derived; `missing_debug_implementations` is a workspace warning,
/// so every other public type here needs one too.
impl<H: AppHost + 'static> fmt::Debug for NativeFixture<H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeFixture").field("service_id", &self.service_id).finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl<H: AppHost + 'static> NativeService for NativeFixture<H> {
    async fn dispatch(&self, inv: NativeInvocation) -> RpcResult<NativeResponse> {
        if inv.method != "run" {
            return Err(RpcError::MethodNotFound(inv.method));
        }
        // Positional `["<json>"]` or named `{"request": "<json>"}` -- the same
        // two shapes `json_to_wasm_params` accepts on the WASM side, so one
        // client frame drives both builds.
        let request = extract_string_param(&inv.params, "request")
            .ok_or_else(|| RpcError::InvalidParams("expected one string param".into()))?;
        let host = (self.host_for)(inv.caller);
        match crate::app::run(&host, &request).await {
            Ok(payload) => Ok(NativeResponse { payload: Value::String(payload) }),
            Err(e) => Err(RpcError::InternalError(e)),   // mirrors the WASM `Err` arm's -32603
        }
    }
}

#[async_trait::async_trait]
impl<H: AppHost + 'static> MessageSink for NativeFixture<H> {
    async fn handle_message(&self, topic: String, payload: Vec<u8>) -> Result<(), String> {
        let host = (self.host_for)(CallerContext::service_system(&self.service_id));
        crate::app::on_message(&host, topic, payload).await
    }
}
```

`CallerContext::service_system` is the same identity the WASM delivery path
uses ([engine.rs:1562](../../../../crates/sandbox_wasm/src/engine.rs#L1562)) —
copy it, and cite the reason (an elevated caller here would let every delivered
message pass the `execute-ddl` admin gate).

The service id comes from `self.service_id`, set at construction by whoever
registered the fixture — **never a `const` in this crate**. The same id is what
the factory was built with; passing it twice from one place at the call site
(§6.2) is what keeps the store namespace, the topic namespace, and the
admin-gate resource in agreement.

`MessageSink` lives in `syneroym-app-host` (§4.1), not in the shim crate, so
this file compiles without the fixture ever depending on
`syneroym-app-host-native`.

---

## §6 Substrate wiring (`dual_build_fixture` feature)

### 6.1 `crates/substrate/Cargo.toml`

```toml
syneroym-app-host-native = { workspace = true, optional = true }
syneroym-test-dual-build-fixture = { workspace = true, optional = true }

[features]
# ... existing ...
dual_build_fixture = ["dep:syneroym-app-host-native", "dep:syneroym-test-dual-build-fixture"]
```

Not in `default`. `--all-features` picks it up, so CI compiles and tests it
(F10).

### 6.2 `crates/substrate/src/runtime.rs`

A new function beside `init_supervisor`
([:676](../../../../crates/substrate/src/runtime.rs#L676)), with the same
`#[cfg]` pair:

```rust
/// The reserved `native_dispatch` key the dual-build fixture registers
/// under. Same role `SUPERVISOR_DISPATCH_ID` plays for the supervisor.
const DUAL_BUILD_FIXTURE_DISPATCH_ID: &str = "dual-build-fixture";

#[cfg(feature = "dual_build_fixture")]
fn init_dual_build_fixture(
    shared: &SharedNodeHandles,
    blob_provider: Arc<dyn BlobProvider>,
    endpoint_registry: &EndpointRegistry,
    logical_resolver: Arc<LogicalResolver>,
) -> anyhow::Result<()> {
    let service_id = DUAL_BUILD_FIXTURE_DISPATCH_ID.to_string();
    let factory = NativeHostFactory::new(
        service_id.clone(),
        shared.key_store.clone(),
        shared.storage_provider.clone(),
        blob_provider,
        shared.messaging_broker.clone(),
        endpoint_registry.clone(),
        logical_resolver,
    );
    let f = factory.clone();
    let fixture =
        Arc::new(NativeFixture::new(service_id, move |caller| f.host_for(caller)));
    factory.set_sink(Arc::downgrade(&fixture) as Weak<dyn MessageSink>);
    shared.native_dispatch.insert(
        DUAL_BUILD_FIXTURE_DISPATCH_ID.to_string(),
        fixture as Arc<dyn NativeService>,
    );
    Ok(())
}
```

Call it from `setup_router` next to the supervisor block
([:665-690](../../../../crates/substrate/src/runtime.rs#L665)), and register
**exactly one** endpoint beside it:

```rust
endpoint_registry.register(service_id.into(), FIXTURE_INTERFACE.into(),
    SubstrateEndpoint::NativeHostChannel { service_id: DUAL_BUILD_FIXTURE_DISPATCH_ID.into() }).await?;
```

**Do not also register a `messaging` endpoint**, even though the supervisor
block right above does. `EndpointRegistry::register` is a silent
last-write-wins insert on `(service_id, interface_name)`
([local_registry.rs:177-189](../../../../crates/core/src/local_registry.rs#L177)),
`(node_did, "messaging")` already belongs to the supervisor
([runtime.rs:686](../../../../crates/substrate/src/runtime.rs#L686), with a
comment explaining exactly why it must), and CI builds `--all-features`, so both
would be live at once and one would quietly disappear. The fixture needs nothing
from that key: its `subscribe` is app-initiated and its pump reads the broker
directly, never through the router's messaging path.

`blob_provider` is currently moved into `ControlPlaneService::init`
([:1010](../../../../crates/substrate/src/runtime.rs#L1010)); add it to
`SharedNodeHandles` ([:762](../../../../crates/substrate/src/runtime.rs#L762))
as an `Arc` clone rather than threading a second parameter — that struct exists
for exactly this ("handles a post-router role needs, otherwise consumed by
`build_route_handler_deps`"). Same for `logical_resolver`.

`FIXTURE_INTERFACE` is `"syneroym-test:dual-build-fixture/test-driver@0.1.0"`,
exported from the fixture crate so the constant cannot drift between the
registration and the test.

---

## §7 The parity suite

**Location**: `crates/app_host_native/tests/dual_build_parity.rs` — the crate
that claims parity owns the proof, and it already depends on everything needed
(`syneroym-sandbox-wasm` for the WASM driver, the fixture as a dev-dependency
for the native one). It runs under plain `cargo test --workspace`, with **no**
feature flag, so the AGENTS.md gate covers it.

### 7.1 Shape

```rust
#[async_trait::async_trait]
trait Driver { async fn run(&self, request: &str) -> Result<String, String>; }

/// Drives the component through the real sandbox engine.
struct WasmDriver { engine: AppSandboxEngine, service_id: String }
/// Drives the same source, linked in, through the shim.
struct NativeDriver { fixture: Arc<NativeFixture<NativeAppHost>> }
```

`WasmDriver::run` builds a `JsonRpcRequest { method: "run", params: json!([request]) }`
and calls `engine.execute_wasm_json(&self.service_id, FIXTURE_INTERFACE, &req, Some(caller()))`,
then reads the returned `Value::String`.
`NativeDriver::run` builds the identical `NativeInvocation` and calls
`dispatch`. Both use the **same** `CallerContext` (`D-B3-14`) — a
`AuthLevel::Ucan` caller with a fixed DID and no capabilities, built by one
helper used by both drivers.

The scenario body is written once:

```rust
async fn scenarios<D: Driver>(d: &D) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    for (name, request) in SCENARIOS {          // a const table of (name, json)
        out.push((*name, d.run(request).await.unwrap_or_else(|e| format!("ERR:{e}"))));
    }
    out
}

#[tokio::test]
async fn both_builds_produce_identical_results() {
    let (wasm, native) = harness().await;       // one provider set, two service ids
    assert_eq!(scenarios(&wasm).await, scenarios(&native).await);
}
```

`SCENARIOS` covers, at minimum: `store-messages`, `read-messages`,
`admin-ddl` (denied on both), `put-blob`/`get-blob`, `stream-blob`,
`subscribe-topic` + `publish-topic` + `read-inbox`, and `get` of a missing id.

The messaging scenario needs a settle step: publish, then poll `read-inbox`
until non-empty or a timeout, on both builds. Use the existing
`crates/substrate/tests/common/retry.rs` pattern rather than a bare sleep.

### 7.1a The harness: two stacks, one service id

**The two builds must share a service id, and must therefore share nothing
else.** F12: the id is the store namespace, the broker topic namespace
(`svc/<service_id>/<topic>`), and the `data-layer/admin` resource — and the
*namespaced* topic is what reaches `handle-message` and lands in the fixture's
own `read-inbox` output. Two ids make the two builds print two different
strings for a reason no shim change can fix.

So the harness builds **two independent stacks**, each with its own
`TempDir`, and hands both the same `SERVICE_ID`:

| Per stack | Why not shared |
|---|---|
| `SqliteStorageProvider` (own `db_dir`) | same service id ⇒ same DB file ⇒ the two builds would read each other's rows |
| `ObjectStoreBlobProvider` (own root, or `in_memory`) | same, for blobs |
| `KeyStore` | derives per-service DEKs from the service id |
| `MqttBroker` | same id ⇒ same namespaced topic ⇒ one build's publish would be delivered to the other's subscription. `MqttBroker::new` opens no listener ([mqtt_broker/src/lib.rs:118-144](../../../../crates/mqtt_broker/src/lib.rs#L118)) — `v4`/`v5`/`ws` are all `None` — so a second in-process instance costs nothing and binds no port |

Everything else follows
[data_layer_integration.rs:28-58](../../../../crates/sandbox_wasm/tests/data_layer_integration.rs#L28).
The WASM stack additionally builds an `AppSandboxEngine` over its providers and
deploys the component under `SERVICE_ID`; the native stack builds a
`NativeHostFactory` over its own, with the same `SERVICE_ID`.

Teardown calls `NativeHostFactory::shutdown()` on the native stack and drops
both, so a test that follows does not inherit a live subscription.

### 7.2 Named per-build tests

The `assert_eq!` above tells you *that* the builds differ, not which is wrong.
Add, per build, the same handful of positive assertions (`store-messages`
writes 5 and reads 5; `stream-blob` round-trips its body; `admin-ddl` is
denied) so a failure names a build. These are cheap and are what a bisect
actually reads.

### 7.3 Permitted differences, asserted explicitly

One test per difference, so each is a decision on record rather than a gap:

- **transport error shape** — the WASM build's WIT `Err` becomes JSON-RPC
  `-32603` via `wasm_results_to_json`; the native build's `RpcError::InternalError`
  also codes `-32603`, but through a different path. Assert both, in one test,
  with a comment pointing at F7.
- **resource lifetime** — a `blob-writer` cannot outlive its invocation on
  either build (`D-B3-6`). Assert it natively (the WASM side cannot even express
  it, since the fixture has one verb).
- **subscription survival across restart** — the WASM build's subscription is
  persisted and replayed at boot
  ([runtime.rs:972](../../../../crates/substrate/src/runtime.rs#L972)); the
  native build's is not (`D-B3-18`, F13). Assert the *current* behaviour of each
  in one test, with a comment naming the backlog row that would close the gap.
  Asserting it is what stops the difference from being rediscovered as a bug.

### 7.5 A test that proves the comparison can fail

Comparing two result vectors proves nothing unless the comparison is known to
detect a difference. Row 13 of the failure matrix is B3's only matrix row, and
`assert_eq!(wasm, native)` passing is equally consistent with "the builds agree"
and "the harness compares two empty vectors".

Add a **mutant driver**:

```rust
/// Wraps another driver and corrupts one field of its result. Exists purely
/// to prove the parity comparison detects a divergence -- if this test ever
/// passes, the real comparison above is not comparing anything.
struct Mutant<D>(D);

#[async_trait::async_trait]
impl<D: Driver + Sync> Driver for Mutant<D> {
    async fn run(&self, request: &str) -> Result<String, String> {
        self.0.run(request).await.map(|s| s.replace("\"written\"", "\"wrote\""))
    }
}

#[tokio::test]
async fn the_parity_comparison_detects_a_divergence() {
    let (wasm, native) = harness().await;
    assert_ne!(scenarios(&wasm).await, scenarios(&Mutant(native)).await);
}
```

It must also assert the result vector is non-empty, so a harness that silently
produces nothing fails loudly rather than passing twice.

### 7.4 The feature-path test

`crates/substrate/tests/dual_build_fixture_e2e.rs`, `#![cfg(feature = "dual_build_fixture")]`,
one test: boot `SubstrateTestContext`, then

```rust
let response = ctx.substrate_client
    .request(FIXTURE_INTERFACE, "run", json!([r#"{"op":"store-messages","count":3}"#]))
    .await?;
```

and assert the payload. **Do not hand-write a preamble line.**
`SyneroymClient::request`
([sdk/src/lib.rs:630](../../../../crates/sdk/src/lib.rs#L630)) builds it, and the
separator is `|`, not `.` — `PREAMBLE_SEPARATOR`
([preamble.rs:100](../../../../crates/router/src/preamble.rs#L100)); the `.`
form in that module's own header doc and in the repo's architecture prose is
stale (F15, §12.9).

This test proves §6's registration, which the in-process suite does not touch.
It runs in CI under `--all-features` (F10) and locally via the command in §10's
checklist.

---

## §8 Every call site that changes

Exhaustive list of edits to existing files. Everything else in this plan is a
new file.

| File | Edit |
|---|---|
| [Cargo.toml](../../../../Cargo.toml) (root) | `[workspace.dependencies]`: add `syneroym-app-host`, `syneroym-app-host-native`, `syneroym-test-dual-build-fixture` (path `test-components/dual-build-fixture`). **Do not** add the fixture to `exclude` (`D-B3-8`). |
| [crates/wit_interfaces/src/lib.rs](../../../../crates/wit_interfaces/src/lib.rs#L5) | add `pub mod messaging;` beside `pub mod data_layer;` |
| `crates/wit_interfaces/src/messaging.rs` | **new**, F2's `generate!` block |
| [crates/substrate/Cargo.toml](../../../../crates/substrate/Cargo.toml#L44) | two optional deps + the `dual_build_fixture` feature (§6.1) |
| [crates/substrate/src/runtime.rs](../../../../crates/substrate/src/runtime.rs#L762) | `SharedNodeHandles` gains `blob_provider` and `logical_resolver`; `build_route_handler_deps` populates them ([:1024](../../../../crates/substrate/src/runtime.rs#L1024)); `setup_router` gains the cfg'd registration block (§6.2) |
| [crates/core/src/test_constants.rs](../../../../crates/core/src/test_constants.rs) | add `dual_build_fixture_wasm_path()` → **the workspace `target/`**, not a per-component one: `PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wasm32-wasip2/release/syneroym_test_dual_build_fixture.wasm")`; add `DUAL_BUILD_FIXTURE_INTERFACE` |
| [mise.toml](../../../../mise.toml#L30) | `build:test-components` gains a line **before** the existing loop: `cargo component build --release --target wasm32-wasip2 -p syneroym-test-dual-build-fixture` (run from the workspace root, not `cd`-ed) |
| [test-components/README.md](../../../../test-components/README.md) | one bullet for the new fixture, naming that it is the only member built both ways |
| [docs/planning/.../task.md](task.md) | `D-06B-6` row amended (`D-B3-15`); B3 row marked complete with links |
| [status.md](status.md) | B3 section (§11) |
| [docs/planning/deferred-backlog.md](../../deferred-backlog.md) | seven rows (§11) |

**Reviewed and deliberately left unchanged** — call sites this slice interacts
with but must not edit:

| File | Why it is in scope to *read*, and why it stays as it is |
|---|---|
| [runtime.rs:972](../../../../crates/substrate/src/runtime.rs#L972) (`replay_persisted_subscriptions`) | The shim's `subscribe` would feed this table if it persisted. It does not (`D-B3-18`), precisely so this boot path needs no change. Guarding the replay on `is_deployed` is *not* an option — nothing is deployed yet at line 972. |
| [runtime.rs:686](../../../../crates/substrate/src/runtime.rs#L686) (supervisor `messaging` endpoint) | The fixture must not re-register this key (`D-B3-19`). |
| [orchestration.rs:2681](../../../../crates/control_plane/src/service/orchestration.rs#L2681) (`unsubscribe_all` on undeploy) | The WASM analogue of `NativeHostFactory::shutdown`. A linked app has no undeploy, so nothing calls the native one from the substrate; `Drop` is the backstop. |
| [preamble.rs:7](../../../../crates/router/src/preamble.rs#L7) (stale `.` separator in the module doc) | A real doc bug this plan tripped over (F15). One-line fix; §12.9 says take it as a drive-by or file it, but do not let it silently stay wrong. |

No existing test needs editing: nothing this slice touches changes an existing
signature. If `SharedNodeHandles` gaining two fields breaks a construction site,
it is only [runtime.rs:1024](../../../../crates/substrate/src/runtime.rs#L1024)
— that struct is private to the module.

---

## §9 Build-graph details that will bite

### 9.1 `syneroym-wit-interfaces` gains a wasm-visible module

`pub mod messaging;` is unconditional, like `data_layer` and `blob_store`. It
must **not** go under the `#[cfg(not(target_arch = "wasm32"))]` group with
`host`/`http` ([lib.rs:13-16](../../../../crates/wit_interfaces/src/lib.rs#L13)),
or the guest adapter loses it.

### 9.2 The fixture as a workspace member: what changes

- `cargo build --workspace` now also builds a `cdylib` for the host. Harmless,
  and its host build contains no wit-bindgen code at all (`D-B3-8`'s cfg split).
- The wasm artifact lands in the **shared** `target/wasm32-wasip2/release/`,
  not in a per-component `target/`. `test_constants` must reflect that (§8).
  A `CARGO_TARGET_DIR` override breaks this path — as it already breaks every
  other `test_constants` helper, so this is not a new fragility, just a
  differently-shaped one. Note it in the helper's doc comment.
- The fixture resolves `wit-bindgen` **0.57** (the workspace pin,
  [Cargo.toml:169](../../../../Cargo.toml#L169)) rather than the `0.55.0` the
  excluded components pin in their own lockfiles. `cargo-component` 0.21.1 (CI's
  pinned version) must accept a 0.57-generated module. **Verify in Step 1.**
- **It is also the first WASM component built under the workspace release
  profile** ([Cargo.toml:24-30](../../../../Cargo.toml#L24)): `lto = true`,
  `codegen-units = 1`, `opt-level = "s"`, `panic = "abort"`, `strip = true`.
  Every excluded component builds under its own default profile today, so none
  of these has ever been exercised against `cargo component`'s
  module-to-component encode step. `panic = "abort"` in particular changes what
  the guest does on a trap. **Also a Step 1 check** — and if any of them turns
  out to break the encode, the fix is a `[profile.release.package.syneroym-test-dual-build-fixture]`
  override, not a change to the workspace profile.

### 9.3 `ServiceConfig` literals

Any new `DeployManifest` built in a test must carry every `ServiceConfig`
field, including B2's `visibility: None` — copy
[guest_http_e2e.rs:38-58](../../../../crates/substrate/tests/guest_http_e2e.rs#L38)
verbatim rather than an older example.

---

## §10 Order of work

Each step ends in a state where the tree builds and the suite that exists so
far passes. A step that cannot be finished is a step to report, not to skip.

**Step 1 — de-risk, before writing any real code (half a day).**
Throw-away spike, committed to nothing:
1. Add `crates/wit_interfaces/src/messaging.rs` + the `lib.rs` line. `cargo check -p syneroym-wit-interfaces`
   **and** `cargo check -p syneroym-wit-interfaces --target wasm32-wasip2`
   (the host half is already probed; the wasm half is what the guest build needs).
2. Build a two-line component that depends on **the real
   `syneroym-wit-interfaces`** — not a simplified stand-in — and that runs its
   own `generate!`+`export!` for the same imports.
   `cargo component build --release --target wasm32-wasip2`. **This settles
   §5.4's open point**, and it must use the real crate: that crate runs seven
   `generate!` invocations, several of them for worlds with *exports*
   (`control-plane`, `supervisor`), and the risk is at component encode/link
   time, which a one-interface stand-in would not reach. Build it **inside the
   workspace**, so the workspace release profile (§9.2) is exercised at the
   same time. If it fails, apply §5.4's fallback and adjust §3.3's home before
   Step 2.
3. In a scratch test inside `crates/sandbox_wasm`, build a `HostState` by hand
   and call `StoreHost::create_collection(&mut state, …).await` and
   `HostBlobStore::put_blob(&mut state, …).await`. **This settles F4's claim
   that the generated `Host` traits are callable from outside a wasmtime
   `Store`.** If it fails (an unexpected `WithStore`/`Accessor` variant, a
   `HasData` bound), fall back to `D-B3-5`'s named alternative: have
   `NativeAppHost` delegate to `SynSvcNativeService::dispatch` in process
   ([synsvc_native.rs](../../../../crates/control_plane/src/synsvc_native.rs)),
   converting via `serde_json`. That fallback keeps one implementation of the
   semantics and costs one JSON round trip per call; it changes §4.3 and
   nothing else.

**Step 2 — `syneroym-app-host`.** Traits, re-exports, `MessageSink`, the guest
module, `block_on`. Unit test: nothing to test yet beyond `cargo check` for both
targets (`cargo check -p syneroym-app-host --target wasm32-wasip2`).

**Step 3 — `syneroym-app-host-native`.** `convert.rs` first, with its round-trip
tests; then the factory, then `NativeAppHost`, then messaging. A unit test in
this crate that exercises `put`/`get`/`put_blob` against real providers and a
`tempfile` directory proves the shim before any fixture exists.

**Step 4 — the fixture, native build only.** `app.rs` + `native.rs`. Extend
Step 3's test to drive it through `NativeService::dispatch`. At the end of this
step the native half of exit criterion 1 is met and provable.

**Step 5 — the fixture's WASM build.** `wit/`, `guest.rs`, the `mise` line, the
`test_constants` helper. Deploy it through `AppSandboxEngine::deploy_wasm` in a
test and call one verb. Exit criterion 1 is now fully met.

**Step 6 — the parity suite** (§7.1–7.3). Expect real failures here; each one
is either a shim bug or a fixture that leaned on a build-specific behaviour.
Fix the former, rewrite the latter, and record any behaviour that genuinely
cannot be made identical in §7.3 with its reason.

**Step 7 — the substrate feature** (§6) and its e2e test (§7.4).

**Step 8 — the completion pass.** In this order — `cargo test --workspace`
needs the fixture's `.wasm` on disk, so the component build comes **first**, and
a clean checkout fails if it does not:

1. Import cleanup over every edited file (AGENTS.md's mandatory pass).
2. A grep for `D-B3`/`D-06B`/`B3` in the diff's code comments — AGENTS.md's
   "No Planning-Doc References in Code" (§5.5's house-rule note).
3. `cargo +nightly fmt --all`
4. `mise run build:test-components` — also proves the new `cargo component` line
5. `cargo clippy --workspace --all-targets --all-features`
6. `cargo test --workspace`
7. `cargo test -p syneroym-substrate --features dual_build_fixture --test dual_build_fixture_e2e`
   — **not covered by item 6**; CI's `--all-features` run covers it, the local
   gate does not (F10).
8. `cargo check -p syneroym-app-host --target wasm32-wasip2` — the trait crate's
   wasm half is not reachable from any host-target command.
9. `mise run test:e2e`
10. Docs (§11).

---

## §11 Documents and backlog owed

**[status.md](status.md)** — a B3 section in the same shape as B1's and B2's:
what shipped, per crate, with the decision ids; the parity suite's scenario
list; and the two commands a reader needs to reproduce it.

**[task.md](task.md)** — B3's row → Complete, with links; `D-06B-6`'s row
amended per `D-B3-15` ("in `test-components/`, a workspace member so the native
build can be linked; the WASM build is produced from the same crate by
`cargo component build --target wasm32-wasip2`").

**[deferred-backlog.md](../../deferred-backlog.md)** — seven new rows:

| Item | Reason | Target |
|---|---|---|
| `register-stream-protocol` has no native shim path | Its only implementation registers a `WasmChannel` endpoint (F5); a native raw-stream handler needs endpoint routing design that nothing in R1–R4 requires | TBD |
| **A natively linked app's subscriptions do not survive a restart** | `D-B3-18`/F13: the shim deliberately does not write to `messaging_subscriptions`, because `replay_persisted_subscriptions` ([runtime.rs:972](../../../../crates/substrate/src/runtime.rs#L972)) replays every row into the WASM engine regardless of service id and runs before anything is deployed. Closing it needs a native replay hook in `runtime.rs` — new substrate machinery, not shim work | M06C |
| A natively linked app runs with no FDAE policy | `HostState.fdae_policy` is loaded from a deploy record; a linked app has none (§4.2). Both builds run policy-absent in B3's suite, so the shim is untested under a policy | M06C |
| **The native shim links `wasmtime`** | `D-B3-5` buys semantic parity by delegating to `HostState`, which lives in `syneroym-sandbox-wasm`; `wasmtime::component::Resource` also appears in the shim's blob path. So a "native" build of Roym can never be a wasmtime-free build. Undoing it means extracting the host-capability impls into a wasmtime-free crate that both `HostState` and the shim call — a real refactor, worth doing only if a wasmtime-free build is ever actually wanted (§12.8) | TBD |
| **Three interfaces are shimmed; the rest of the guest-visible surface is not** | `D-06B-3` scopes B3 to `data-layer`/`blob-store`/`messaging`, but `D-06B-1` retires the Web entrypoint's native exemption, which eventually needs `syneroym:http` — both directions: the `incoming-handler`/`websocket-handler` guest exports *and* the `websocket.send` host import ([http.wit](../../../../crates/wit_interfaces/wit/http/http.wit)) — plus `app-config`, `vault`, and `proxy`/`saga`, which are the remaining imports of the `host-environment` world ([host.wit](../../../../crates/wit_interfaces/wit/host/host.wit)). Owner: M06C, which is the first consumer of any of them | M06C |
| `read_only` is unexercised on both parity builds | Its only producer is the stage-4 ABAC after-step, which has no native path, so the shim hardcodes `false` (§4.2). Covered by a shim unit test rather than by the parity suite; a native after-step would need its own design | TBD |
| The guest bindings' second consumer is the shim, not a real app | `syneroym-wit-interfaces`' `data_layer`/`blob_store`/`messaging` guest modules exist to serve `syneroym-app-host`; if M06C's Roym ends up generating its own, they go back to being dead code (F1) | M06C |

If Step 1's spike forces either fallback, add an eighth row naming which
fallback was taken and what would let it be undone. Same if §4.3's
`Resource::new_own` spelling has to fall back to a shim-owned session map.

---

## §12 Ambiguities and staleness in the input documents

Reported rather than guessed at, per the brief.

**12.1 `D-06B-6` contradicts D2/D3.** [task.md](task.md) says the fixture is
"in `test-components/`, **excluded from the workspace build graph**, built both
ways". A crate excluded from the workspace cannot be a dependency of
`syneroym-substrate`, and "built into the substrate behind a Cargo feature" is
half the definition of the slice. One of the two has to give. This plan keeps
the *requirement* (built both ways, linked in) and amends the *mechanism*
(`D-B3-8`, `D-B3-15`). **If the milestone owner intended the exclusion to be
load-bearing** — for example to keep `cargo build --workspace` free of any wasm
fixture — then the alternative is two manifests over one `src/` (`#[path]`
includes or a shared inner crate), which is uglier and weakens the "one source
tree" claim. Flagging rather than deciding unilaterally.

**12.2 The slice text does not say which direction the shim covers.** "One
trait per host interface" reads as app→host only. But `messaging`'s guest half
(`handle-message`) is host→app, and it is exactly where the two builds diverge
(F5). This plan covers both directions (`MessageSink`), which is a scope
*addition* relative to a literal reading. It is not optional: without it,
`messaging` is proven only in the direction that was never at risk.

**12.3 "Which crate owns the trait definitions is open" is not open.** F3
settles it against `syneroym-sdk` on facts, not preference.

**12.4 The packaging table's "lower call overhead" is unmeasured, and this
slice does not measure it.** With `D-B3-5`'s delegation the native path is
`Mutex` + `WasiCtx::builder().build()` + the same provider calls — no wasm
instantiation, no JSON. That is very likely faster, but "likely" is all this
slice will be able to say. If the claim needs to be load-bearing for M06C,
it needs a benchmark, and that benchmark belongs in this slice or in a row
of its own. Currently neither exists.

**12.5 The spec's Rust sketch says "make Roym generic over them".** This plan
makes the *functions* generic (`run<H: AppHost>(host: &H, …)`) rather than a
generic struct, because the native host handle is per-invocation (`D-B3-6`)
while a struct field would be per-instance. Equivalent in effect; worth noting
because a reader comparing against the spec's wording will see a difference.

**12.6 `ADR-0014`'s "Resource Mechanics" does not apply here.** [task.md](task.md)
raises it under B4's open design points, and a reader may reach for it when
seeing `blob-writer`/`blob-reader` in §3.2. It is about **guest-exported**
resources (`stream-cursor`/`stream-sink`) — the host calling into an instance
the guest returned. Blob resources are host-exported, the ordinary direction,
so §3.2's associated types are not a third pattern and do not touch that ADR.

**12.7 Nothing in the failure-and-security matrix except row 13 lands in B3.**
Rows 1–12 are B1/B4/B5 or already shipped. Exit criteria 1, 2, and 13 are B3's
whole share; criteria 3–10 are later slices. A reviewer checking B3 against the
milestone's exit list should expect exactly that.

**12.8 `D-B3-5` means the "native" build links wasmtime, and the packaging table
does not say so.** The spec's table contrasts "`wasm32-wasip2`, run by Wasmtime"
with "linked into `syneroym-substrate`, behind a Cargo feature … lower call
overhead". Delegating to `HostState` (which lives in `syneroym-sandbox-wasm`)
buys parity at the price of making that contrast partial: the native build has
no *guest* running under wasmtime, but the crate is still linked, and
`wasmtime::component::Resource` appears in the shim's own blob API. This is the
right trade for B3 — the alternative is a third implementation of every gate —
but it is a trade, it is not what the table implies, and the escape route
(extract the host-capability impls into a wasmtime-free crate that both callers
use) is recorded in §11 rather than assumed away.

**12.9 `preamble.rs`'s module doc contradicts its own parser.** The header at
[preamble.rs:7](../../../../crates/router/src/preamble.rs#L7) writes
`<scheme>://<interface>.<service_id>`; `PREAMBLE_SEPARATOR` is `"|"`
([:100](../../../../crates/router/src/preamble.rs#L100)) and the parser splits on
it ([:285](../../../../crates/router/src/preamble.rs#L285)). The same `.` form
appears in the repo's architecture prose in
[CLAUDE.md](../../../../CLAUDE.md)/[AGENTS.md](../../../../AGENTS.md). Nothing
breaks, because callers build a `RoutePreamble` value or go through
`SyneroymClient`, which is exactly why it has survived — the first draft of this
plan copied it into §7.4 and was wrong. Fix the one-line doc as a drive-by, or
file it; do not leave it unrecorded.

**12.10 The plan's own sketches were the first place the "no planning-doc
references in code" rule was broken.** Revision 1 carried `D-B3-n` ids inside
doc comments an implementer would copy verbatim. They are gone, and §5.5 carries
the rule explicitly, but it is worth naming as a hazard of this document's
format: a plan that shows code teaches its comment style too.

---

## §13 What B3 owes B4

**This is B3's chief output, and revision 1 left it unwritten.** B4 is gated on
B3 for a stated reason — *"so the largest new interface is designed against both
builds from the first line"* ([task.md](task.md)) — and `D-B3-5` turns that into
a hard constraint B4 must know before it writes its first WIT line:

1. **B4's `syneroym:conversation` host implementation must land as
   `impl … for HostState`** in `crates/sandbox_wasm/src/host_capabilities.rs`,
   beside `store::Host` and `blob_store::Host`. That is the only place the
   native shim can reach it from. An implementation that lives anywhere else —
   a free-standing `ConversationService`, a `NativeService`, a method on the
   engine — is reachable by the WASM build and invisible to the native one, and
   B4 would then need a second implementation, which is exactly the retrofit
   `D-06B-3` exists to prevent.
2. **Every function B4 adds needs a matching trait method in
   `syneroym-app-host`, a guest impl, and a shim delegation** — three small
   edits per function, mechanical, but they are part of "add a function", not a
   follow-up.
3. **Anything B4 needs in the host→app direction** (a delivery callback, a
   state-change notification) follows `MessageSink`'s shape (§4.1), not
   `handle-message`'s: a trait in `syneroym-app-host`, implemented by the app,
   driven by the engine on WASM and by the shim natively. This is the half that
   has no automatic parity, so it is the half to design first.
4. **B4's conversation types must be expressible in the guest vocabulary**
   (`D-B3-2`). If a type only makes sense as a wasmtime host type, both builds
   cannot share it.
5. **Resources are per-invocation on both builds** (`D-B3-6`). A conversation
   handle modelled as a WIT resource cannot hold state across calls on either
   build; if B4 wants a long-lived handle, that is a design question to settle
   against this constraint, not to discover in the shim.

---

## §14 What "done" means for B3

1. `cargo component build --release --target wasm32-wasip2 -p syneroym-test-dual-build-fixture`
   produces a component, and `cargo build -p syneroym-substrate --features dual_build_fixture`
   links the same crate's native build. (Exit criterion 1.)
2. `cargo test -p syneroym-app-host-native --test dual_build_parity` passes:
   `both_builds_produce_identical_results` over a non-empty result set, **and**
   `the_parity_comparison_detects_a_divergence` (§7.5). Without the second, the
   first is not evidence. (Exit criterion 2, failure-matrix row 13.)
3. No file under `test-components/dual-build-fixture/src/app.rs` names any
   substrate crate. (D3, checkable by `grep`.)
4. §10 Step 8's ten commands are clean, in that order.
5. No `D-B3`/`D-06B`/slice id appears in a code comment in the diff.
6. §11's documents are updated in the same change, including the seven backlog
   rows and `task.md`'s amended `D-06B-6`.
