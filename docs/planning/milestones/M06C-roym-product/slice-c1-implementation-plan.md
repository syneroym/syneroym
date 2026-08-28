# M06C Slice C1 — Complete the Dual-Build Shim: Implementation Plan

> **Scope, from [task.md](./task.md)'s slice table.** The four host
> interfaces Roym needs and `AppHost` does not have: `syneroym:proxy`
> (`call`/`enqueue`, both target shapes), `syneroym:http` inbound
> (`incoming-handler`, `websocket-handler`), `syneroym:app-config`, and
> `syneroym:vault` — each with a `wit-bindgen` guest implementation and an
> in-process native one, each proven by the existing
> `test-components/dual-build-fixture` built both ways. **No product code.**
>
> Plus the two things `task.md` hands C1 by name:
> the FDAE-policy / `RowAuthorizer` question for a linked native app, and
> the three M06C-targeted native-shim rows in
> [deferred-backlog.md](../../deferred-backlog.md) §5.
>
> Verified against the tree on **2026-08-25**, `main` at `394d0f4`. Every
> line reference below was read, not assumed.

---

## §0 What B3 handed C1, and what is missing

`syneroym-app-host` ([lib.rs:31-43](../../../../crates/app_host/src/lib.rs#L31))
defines:

```rust
pub trait AppHost: AppDataLayer + AppBlobStore + AppMessaging + AppConversation + Send + Sync {}
impl<T> AppHost for T where T: AppDataLayer + AppBlobStore + AppMessaging + AppConversation + Send + Sync {}
```

Two implementors, both in-tree:

| Implementor | Where | How it works |
|---|---|---|
| `GuestHost` (zero-sized) | [app_host/src/guest.rs:35](../../../../crates/app_host/src/guest.rs#L35), `cfg(target_arch = "wasm32")` | calls `syneroym-wit-interfaces`' pre-generated **guest import** bindings directly |
| `NativeAppHost` (`Arc<HostInner>`) | [app_host_native/src/host.rs:34](../../../../crates/app_host_native/src/host.rs#L34) | locks its per-invocation `HostState` and calls the **same `Host` trait impls** the wasm linker uses ([sandbox_wasm/src/host_capabilities.rs](../../../../crates/sandbox_wasm/src/host_capabilities.rs)) |

Two host→app sink traits sit beside `AppHost`, deliberately **not** in its
supertrait list because they are the receive direction:
`MessageSink` ([lib.rs:169](../../../../crates/app_host/src/lib.rs#L169)) and
`ConversationSink` ([lib.rs:243](../../../../crates/app_host/src/lib.rs#L243)).
Both use `#[async_trait]` because both are used as `dyn`.

What is missing, per interface:

| Interface | Direction | WIT | Guest bindings today | Native path today |
|---|---|---|---|---|
| `syneroym:vault/vault` | app → host | [vault.wit](../../../../crates/wit_interfaces/wit/host/deps/vault/vault.wit) | **yes** — `wit_interfaces::vault`, world `vault-guest` | none (but `vault::Host for HostState` exists and needs nothing new) |
| `syneroym:app-config/app-config` | app → host | [app-config.wit](../../../../crates/wit_interfaces/wit/app-config/app-config.wit) | **yes** — `wit_interfaces::app_config`, world `app-config-guest` | none; and `HostState.config_generation` is hardcoded `0` by the shim, so the interface would always answer empty |
| `syneroym:proxy/proxy` | app → host | [proxy.wit](../../../../crates/wit_interfaces/wit/proxy/proxy.wit) | **no guest module at all** | none; and `HostState.service_proxy` is `empty_service_proxy()`, `app_instance_id` is `None` |
| `syneroym:http/incoming-handler` + `websocket-handler` | host → app | [http.wit](../../../../crates/wit_interfaces/wit/http/http.wit) | export-side only, per component | **nothing anywhere in the tree** |
| `syneroym:http/websocket` (`send`) | app → host | same file | host bindgen only (`wit_interfaces::http`, world `http-host`) | none; the impl reaches `AppSandboxEngine.websocket_senders` through `MessagingContext.engine`, which the shim sets to `Weak::new()` |

The three hardcodes are in one function,
[`NativeHostFactory::host_with`](../../../../crates/app_host_native/src/factory.rs#L136):

```rust
let state = HostState::new(
    self.service_id.clone(),
    None,                              // max_memory_bytes
    …,
    caller,
    0,                                 // config_generation  <- app-config always empty
    MessagingContext { broker: …, engine: Weak::new() },   // <- websocket send unreachable
    StreamContext  { registry: …, engine: Weak::new() },
    empty_service_proxy(),             // <- proxy::call always "proxy unavailable"
    None,                              // fdae_policy
    read_only,
    syneroym_rpc::empty_row_authorizer(),
    None,                              // app_instance_id  <- dependency always unbound
    self.logical_resolver.clone(),
);
```

The WASM build derives all five per invocation, in
[`engine.rs:1245-1287`](../../../../crates/sandbox_wasm/src/engine.rs#L1245),
from things `NativeHostFactory` **already holds** (`storage_provider`,
`endpoint_registry`) or can be handed. That is the shape of most of this
slice: not new semantics, but giving the native factory the same five
inputs.

---

## §1 Findings from reading the tree

### F1 — three of the four interfaces are pure delegation; only `http` is new design

`proxy`, `app-config` and `vault` all already have a `Host for HostState`
impl that the wasm linker uses
([host_capabilities.rs:488, 698, 1228](../../../../crates/sandbox_wasm/src/host_capabilities.rs#L488)).
The native shim reaches those exact impls today for `data-layer`,
`blob-store`, `messaging` and `conversation`. Adding three more is the same
`lock the state, call the `Host` method, convert the error` pattern
`host.rs` repeats ~40 times already. **Nothing about their semantics is
being invented.**

`syneroym:http` inbound is different: it is an *export*, so there is no
`Host for HostState` to delegate to, and the router's only path to it
(`AppSandboxEngine::handle_guest_http_request`,
[engine.rs:2456](../../../../crates/sandbox_wasm/src/engine.rs#L2456)) goes
through wasmtime instantiation. This is the slice's real work.

### F2 — the guest proxy gate is enforced inside `HostState`, so the native build inherits it for free

`proxy::Host::call` always constructs
`CallOrigin::Guest { service_id: self.component_id.clone() }`
([host_capabilities.rs:1340](../../../../crates/sandbox_wasm/src/host_capabilities.rs#L1340)),
and `CallOrigin`'s own doc says it is "**Host-set, never guest-settable**"
([rpc/src/proxy.rs:44](../../../../crates/rpc/src/proxy.rs#L44)). Because the
native shim delegates *into* that same function, a natively linked app is
subject to `ProxyRouter::check_native_capability_gate` identically, with no
new enforcement code and no risk of the native build getting a weaker gate.
Same for the self-proxy caller-forwarding rule
([host_capabilities.rs:1327-1336](../../../../crates/sandbox_wasm/src/host_capabilities.rs#L1327)),
the `enqueue` idempotency-key refusals, and dependency resolution through
`LogicalResolver`.

**Consequence for the plan:** `AppProxy` must delegate, never re-implement.
A native `ProxyRequest` built by hand in the shim would be the exact
D3-style shortcut C1 exists to prevent.

### F3 — `saga` is a separate interface and is out of C1's scope

`wit/proxy/proxy.wit` declares two interfaces: `proxy` and `saga`. `task.md`
names only "`syneroym:proxy` (`call`/`enqueue`, both target shapes)".
`saga::Host for HostState` exists
([host_capabilities.rs:1492](../../../../crates/sandbox_wasm/src/host_capabilities.rs#L1492)),
so adding it later is mechanical. No Roym service in C2–C10's descriptions
drives a multi-service saga. C1 leaves it out and records the row.

### F4 — `http.wit`'s records live inside the *exported* interface, so the WIT-type-sharing trick used for `conversation` does not apply

The pattern B3 relies on — `app_host::types::conversation` re-exports the
**imported** `conversation` interface's types, and the fixture's exported
`guest-api` `use conversation.{message}` resolves to the same Rust type — works
because `conversation.wit` splits types (imported interface) from the
callback (`guest-api`, exported), confirmed at
[conversation.wit:181-186](../../../../crates/wit_interfaces/wit/conversation/conversation.wit#L181).
`messaging.wit` does the same with `stream-types`.

`http.wit` does **not**: `caller-auth`, `caller-identity`, `http-request` and
`http-response` are all declared inside `interface incoming-handler`, which
is the exported one. To share them the same way, C1 would have to split out
an `http-types` interface and import it — which changes the component type
of every existing HTTP guest component and adds a **types-only import
instance** to every component that imports it. Whether wasmtime's `Linker`
tolerates an unsatisfied types-only import instance is not established
anywhere in this tree.

There is already a hand-written host-side mirror of exactly these records:
[`syneroym_core::guest_http`](../../../../crates/core/src/guest_http.rs), whose
own doc comment says it mirrors the WIT "field for field, **in the same
order**". **Reusing that mirror instead of creating a second one is the
recommendation** (`D-C1-3` below).

### F5 — inbound HTTP reaches a service by `service_id` already; only the `guest` arm is WASM-only

[`route_handler/http.rs:896`](../../../../crates/router/src/route_handler/http.rs#L896)
resolves `(method, path)` against
`route_handler.inner.http_routes.get(&self.preamble.service_id)`, then
`dispatch_route` fans out on `route.target`
([http.rs:1006-1015](../../../../crates/router/src/route_handler/http.rs#L1006)):

```rust
"data-layer" => …,   // -> dispatch_native, i.e. NativeDispatchRegistry
"messaging"  => …,   // -> dispatch_native
"stream"     => …,
"guest"      => self.handle_guest_route(…),      // -> app_sandbox_engine ONLY
"websocket"  => self.handle_websocket_route(…),  // -> app_sandbox_engine ONLY
```

So the routing table, the preamble→service resolution, the caller
extraction, the 401-before-instantiation rule (`D-A2-7`), the body cap, the
header sanitising and the response framing are all already service-kind
agnostic. **Only the last hop is WASM-only.** A native path is a second
lookup at that hop, not a second HTTP stack.

`http_routes` is an `Arc<DashMap<String, Vec<HttpRoute>>>`
([core/src/http_routes.rs:62](../../../../crates/core/src/http_routes.rs#L62))
populated by `ControlPlaneService::deploy`. A linked native app has no
deploy, so C1's linking code inserts its routes directly.

### F6 — the WebSocket sender table lives inside `AppSandboxEngine` and is unreachable from the shim

`AppSandboxEngine.websocket_senders: Arc<DashMap<String, Arc<DashMap<String, WebSocketSender>>>>`
([engine.rs:77, 321](../../../../crates/sandbox_wasm/src/engine.rs#L77)). The
host-side `websocket::send` reads it via `self.messaging.engine.upgrade()`
([host_capabilities.rs:1861-1878](../../../../crates/sandbox_wasm/src/host_capabilities.rs#L1861)),
and the shim sets `engine: Weak::new()`, so a natively linked app calling
`send` gets `Err("Unknown connection ID")` — silently, and for the wrong
reason.

The router calls `register_websocket_sender` / `acquire_websocket_permit` /
`deregister_websocket_sender` / `handle_websocket_on_{open,message,close}`
on the engine ([http.rs:1706-1810](../../../../crates/router/src/route_handler/http.rs#L1706)).
Five engine methods, one table. Splitting the table out is the smaller and
safer change (`D-C1-6`).

### F7 — `apply_stage4` already fails closed with no `RowAuthorizer`, and only when a policy actually has ABAC permissions

[`rpc/src/fdae_abac.rs:228`](../../../../crates/rpc/src/fdae_abac.rs#L228):
`if sieve.abac_permissions.is_empty() { return Ok(rows…) }`, and
[:254](../../../../crates/rpc/src/fdae_abac.rs#L254):
`let Some(auth) = authorizer else { … return Err(AbacError::Unavailable(…)) }`.

So handing the native build a real FDAE policy is **safe today**: a policy
with no stage-4 after-step behaves identically on both builds, and a policy
*with* one fails closed on the native build rather than silently skipping
row authorization. That turns the backlog row's "must either build the
native policy path or say so loudly" into a small, testable change rather
than a design project (`D-C1-8`).

### F8 — everything the WASM build derives per invocation, the native factory can derive from what it already holds

| `HostState::new` arg | WASM build source | Available to `NativeHostFactory`? |
|---|---|---|
| `config_generation` | `storage_provider.get_latest_config_generation(service_id)` (**async**) | yes — it holds `storage_provider` |
| `app_instance_id` | `endpoint_registry.app_context_of(service_id)` (**sync**, [local_registry.rs:408](../../../../crates/core/src/local_registry.rs#L408)) | yes |
| `fdae_policy` | `storage_provider.load_fdae_policy(service_id)` (**async**) + a cache | yes |
| `service_proxy` | `engine.service_proxy: OnceLock<Weak<dyn ServiceProxy>>`, set post-construction | needs the same `OnceLock` on the factory |
| `row_authorizer` | `Weak<AppSandboxEngine>` coerced to `Weak<dyn RowAuthorizer>` | **no native implementation exists** — see F7 |

Two of the five are `async`. `NativeHostFactory::host_for` is `fn`, and the
fixture stores it as `Box<dyn Fn(CallerContext) -> H + Send + Sync>`
([dual-build-fixture/src/native.rs:24](../../../../test-components/dual-build-fixture/src/native.rs#L24)).
Making it `async fn` would force that closure to return a boxed future and
ripple into every future Roym service's native wiring. **Building
`HostState` lazily on first host call keeps the signature** (`D-C1-7`).

### F9 — a wasm32 consumer of `syneroym-app-host` links every enabled `wit_interfaces` feature's world

[`wit_interfaces/Cargo.toml`](../../../../crates/wit_interfaces/Cargo.toml)'s
own comment: each guest module is a separate `generate!` embedding a
`#[used]`-anchored component-type section, and a component carrying an
unsatisfied *export* requirement from an unrelated world fails to encode.
`app_host/Cargo.toml` therefore opts out of default features and enables
exactly four.

The three new imports are safe to add because **`vault-guest`,
`app-config-guest` and the new `proxy-import` are all import-only worlds** —
they add imports, and the substrate's linker satisfies all three
already ([`build_wasm_linker`, engine.rs:749-766](../../../../crates/sandbox_wasm/src/engine.rs#L749)
adds `vault`, `app_config`, `proxy`, `saga`). No export requirement is
added, which is the failure mode that comment describes.

This is a second, independent reason not to generate http guest bindings
(F4): an `http-types` import-only interface would add an import the linker
has nothing registered for.

### F10 — the fixture's WIT deps are symlinks, not copies

`test-components/dual-build-fixture/wit/deps/*/*.wit` are all symlinks back
into `crates/wit_interfaces/wit/`. Adding a dep is `ln -s`, and the fixture
can never drift from the canonical WIT. (B3's F11 noted a *different*
component copies its deps; this one does not.)

### F11 — the parity suite drives both builds below the router, and that is the right level for HTTP too

[`dual_build_parity.rs`](../../../../crates/app_host_native/tests/dual_build_parity.rs)'s
`WasmDriver` calls `engine.execute_wasm_json(...)` and `NativeDriver` calls
`fixture.dispatch(...)` — neither goes through hyper. The HTTP parity tests
should sit at the same level: `engine.handle_guest_http_request(...)` versus
the new native handler, comparing `HttpResponse` values. Standing up a real
listener would test the router, which M06A already covers
([`substrate/tests/guest_http_e2e.rs`](../../../../crates/substrate/tests/guest_http_e2e.rs)).

### F12 — `NativeHostFactory::subscribe` deliberately does not persist, and the reason does not apply to a linked app the way the backlog row implies

[factory.rs:170-186](../../../../crates/app_host_native/src/factory.rs#L170):
the substrate's `replay_persisted_subscriptions` hands every persisted row
to the WASM engine regardless of service id, so a native app's row would
produce wasted instantiation attempts after a restart. True. But a **linked**
app is linked in at every process start by definition — its subscriptions
can be re-established by the same code that links it, with no persistence at
all. The row is closable by construction once there is a linked app with a
startup path; the fixture has none today (`D-C1-9`).

### F13 — `HttpRoute.public` semantics matter for the native path

[core/src/http_routes.rs:35-56](../../../../crates/core/src/http_routes.rs#L35):
`public: true` does not merely relax reachability — with no caller to
substitute, the handler runs as `CallerContext::service_system`,
`AuthLevel::System`. `handle_guest_http_request` does exactly that
([engine.rs:2500](../../../../crates/sandbox_wasm/src/engine.rs#L2500)):
`let caller = caller.unwrap_or_else(|| CallerContext::service_system(service_id));`.
The native path **must** apply the same substitution, in the same place, or
an anonymous request would reach a native app with a different identity than
it reaches a WASM one — a straight failure of failure-matrix row 19.

---

## §2 Decisions

| # | Decision | Why |
|---|---|---|
| **D-C1-1** | **`syneroym:proxy` gets guest bindings via a new import-only world.** Add `world proxy-import { import proxy; }` to [proxy.wit](../../../../crates/wit_interfaces/wit/proxy/proxy.wit); add `wit_interfaces` feature `proxy` and module `src/proxy.rs` generating it. Mirrors `messaging-import` / `conversation-import` exactly. | It is the only one of the four with no guest module. An import-only world adds no export requirement (F9), and the substrate linker already registers `proxy` (F9). |
| **D-C1-2** | **`app-config` and `vault` reuse their existing worlds and modules unchanged.** Only new `syneroym-app-host` feature flags and trait impls. | `app-config-guest` and `vault-guest` are already import-only worlds with generated modules. Nothing to add on the WIT side. |
| **D-C1-3** | **HTTP inbound does not get WIT-generated shared types.** The shared vocabulary is plain Rust: `syneroym_core::guest_http`'s records **move to `syneroym-app-host::types::http`** and `core::guest_http` is deleted. `syneroym-core` keeps no HTTP mirror; `sandbox_wasm` and `router` depend on `syneroym-app-host` for these types instead. | F4 + F9: splitting `http.wit` would change every existing HTTP component's type and add a types-only import with no linker registration. Moving (not copying) the existing mirror keeps the number of mirrors at **two** (the WIT, and one Rust mirror) — exactly what it is today. A third mirror is what a naive "add types to app_host" would create. |
| **D-C1-4** | **`AppHost`'s supertrait list grows by four: `AppProxy`, `AppAppConfig`, `AppVault`, `AppWebSocket`.** HTTP *inbound* is **not** an `AppHost` supertrait — it is a sink trait beside `MessageSink`/`ConversationSink`: `HttpSink` and `WebSocketSink`. | `AppHost` is what the app calls; a handler is what the host calls. `task.md`'s migration note says "New traits on `AppHost` for … `http` inbound", which cannot be right for an export — flagged in §12. |
| **D-C1-5** | **The router reaches a natively linked app's HTTP through a new `NativeHttpRegistry`, checked in `handle_guest_route`/`handle_websocket_route` before the sandbox engine.** The trait `NativeHttpService` and the registry alias live in `syneroym-rpc`, beside `NativeService`/`NativeDispatchRegistry`; `syneroym-rpc` gains a dependency on `syneroym-app-host` for the record types. | Symmetric with `NativeDispatchRegistry`, which is how every *other* native route target already works (F5). `syneroym-rpc` is the crate `router`, `sandbox_wasm`, `app_host_native` and `substrate` all already share; `syneroym-app-host` depends only on `syneroym-wit-interfaces`, so there is no cycle (checked: `core → identity`; `rpc → ucan, fdae, identity`; neither depends on the other). |
| **D-C1-6** | **`AppSandboxEngine.websocket_senders` moves out into a shared `WebSocketSenderRegistry` owned by neither build.** It lives in `syneroym-rpc` next to `NativeHttpService`; `AppSandboxEngine` and `NativeHostFactory` each hold the same `Arc`, and `HostState` holds it directly instead of reaching it through `MessagingContext.engine`. | One `conn_id` namespace, one table, one `send` implementation for both builds. The alternative — a second table in the shim — means two places a connection can be looked up and a real chance the router registers into one while the app sends into the other. Also removes the last reason `HostState.messaging.engine` exists for non-messaging work. |
| **D-C1-7** | **`NativeHostFactory::host_for` stays synchronous; `HostState` is built lazily, once, on the first host call of an invocation.** `HostInner` holds `factory`, `caller`, `read_only` and a `tokio::sync::OnceCell<tokio::sync::Mutex<HostState>>`. | F8: `config_generation` and `fdae_policy` are async reads the WASM build does per invocation. Making `host_for` async would change `NativeFixture`'s `Box<dyn Fn(CallerContext) -> H>` into a boxed-future closure, and every Roym service in C2–C10 would inherit that shape. Laziness buys per-invocation freshness with zero call-site change outside `app_host_native`. |
| **D-C1-8** | **The native build honors an FDAE policy, and does not get a `RowAuthorizer`.** `NativeHostFactory` resolves `storage_provider.load_fdae_policy(service_id)` once (a linked app is never redeployed in-process) and passes it to every `HostState`. `row_authorizer` stays `empty_row_authorizer()`. A policy carrying ABAC permissions therefore **fails closed** with `AbacError::Unavailable` on the native build, proven by a test. | F7. This is the loud answer `task.md` asks for: the policy path is built, the stage-4 path is refused rather than skipped, and the difference is a named, tested permitted difference instead of a silent one. Building a native `RowAuthorizer` means designing a native stage-4 after-step (a second app entry point, its own read-only instance semantics, its own timeout) — real substrate work, not shim work, and nothing in R1–R4 needs it. |
| **D-C1-9** | **The native-subscription-replay backlog row is retargeted to C2, not closed and not left at C1.** C1 records *why*: a linked app is linked in at every process start, so its subscriptions are re-established by its own startup code rather than by replaying persisted rows — but the fixture has no startup path to prove it with, and inventing one is product-shaped work that belongs with the first real linked app. | Closing a row with no test is worse than moving it with a reason. `task.md`'s "Owed as slices land" permits "resolved **or restated** with what actually shipped". |
| **D-C1-10** | **`syneroym:proxy/saga` is out of scope and gets a backlog row.** | `task.md` names `call`/`enqueue` only (F3). `saga::Host for HostState` exists, so a later `AppSaga` is a mechanical addition; adding it now is untested surface. |
| **D-C1-11** | **The parity suite proves the four new interfaces at the same level it proves the existing four**: through the fixture's `run()` verb table plus, for `http`, a driver pair that calls `handle_guest_http_request` and the native `NativeHttpService` directly. No hyper listener in this suite. | F11. Router-level HTTP is already covered by `guest_http_e2e.rs`; duplicating it here would test the router twice and the shim once. |

---

## §3 `syneroym-wit-interfaces` — the one new binding module

### 3.1 `wit/proxy/proxy.wit` — append a world

At the end of the file, after `interface saga { … }`:

```wit
/// Import-only view of `proxy`, with no `saga` requirement. For bindgen
/// consumers that only need to *call* `proxy` (`syneroym-app-host`'s
/// shared guest bindings, reused by every component's own world) rather
/// than stand alone as a deployable component -- mirrors
/// `messaging-import` and `conversation-import`.
world proxy-import {
    import proxy;
}
```

Nothing else in the file changes. `interface proxy` and `interface saga`
keep their exact current text, so **no component type changes** and no
existing fixture needs rebuilding for this.

### 3.2 `crates/wit_interfaces/src/proxy.rs` — new file

```rust
//! Guest-side bindings for the Universal Proxy. `proxy-import`, not a
//! world of its own with `saga`: this module exists to be *called* by
//! other components' worlds, and pulling `saga` in would make its types a
//! requirement of every consumer that only wants `call`/`enqueue`.

wit_bindgen::generate!({
    world: "proxy-import",
    path: "wit/proxy/proxy.wit",
    additional_derives: [serde::Serialize, serde::Deserialize],
});
```

Generated path used downstream:
`syneroym_wit_interfaces::proxy::syneroym::proxy::proxy::{ call, enqueue, CallTarget, CallOptions, ProxyError, CalleeError }`.

### 3.3 `crates/wit_interfaces/src/lib.rs`

```rust
 #[cfg(feature = "messaging")]
 pub mod messaging;
+#[cfg(feature = "proxy")]
+pub mod proxy;
 #[cfg(feature = "supervisor")]
 pub mod supervisor;
```

Placed in alphabetical order with the rest.

### 3.4 `crates/wit_interfaces/Cargo.toml`

```toml
default = ["app-config", "blob-store", "control-plane", "conversation", "data-layer", "http", "messaging", "proxy", "supervisor", "vault"]
…
http = []
messaging = []
proxy = []
supervisor = []
```

`proxy` and `http` both join `default` so every existing host-only consumer
is unaffected (the same rule the file's own comment states). `http` gates
the **new** `src/http_guest.rs` module (§4.4's `websocket-import` world) —
not the existing `src/http_host.rs` below, which stays unconditional
(`cfg(not(wasm32))`) exactly as `src/http.rs` is today.

### 3.5 Rename `src/http.rs` → `src/http_host.rs` — **required, not optional**

§4.4 adds a second, guest-side module (`src/http_guest.rs`, gated by the new
`http` feature) that also speaks `syneroym:http`. Leaving the existing
host-side module at the bare name `http` would make `wit_interfaces::http`
ambiguous prose ("which one?") the moment both exist, and §6.3/§7.2 below
name the host-side module as `http_host` throughout — **this rename must
land in the same step as those sections, or their code samples do not
compile.** It also matches the existing `conversation` / `conversation_host`
naming split. Four call sites, all in `syneroym-sandbox-wasm`:

| File | Line | Change |
|---|---|---|
| `crates/wit_interfaces/src/lib.rs` | 26 | `pub mod http;` → `pub mod http_host;` |
| `crates/sandbox_wasm/src/engine.rs` | 37 | `wit_interfaces::http::…::FrameKind` → `wit_interfaces::http_host::…::FrameKind` |
| `crates/sandbox_wasm/src/engine.rs` | 760 | `wit_interfaces::http::…::websocket::add_to_linker` → `http_host::…` |
| `crates/sandbox_wasm/src/host_capabilities.rs` | 1861, 1866 | `impl …http::…::websocket::Host` → `http_host::…` |

---

## §4 `syneroym-app-host` — the traits

### 4.1 `Cargo.toml`

```toml
syneroym-wit-interfaces = { path = "../wit_interfaces", default-features = false, features = [
    "app-config",
    "blob-store",
    "conversation",
    "data-layer",
    "http",
    "messaging",
    "proxy",
    "vault",
] }
```

`"http"` is required here, not only on the fixture (§10.2) — it gates
`wit_interfaces::http_guest`, which `guest.rs`'s `AppWebSocket for GuestHost`
impl (§4.4) calls into directly. Omitting it here is a compile error in this
crate's own wasm32 build, not just the fixture's.

The existing comment above this dependency must be updated: it currently
says "this crate's guest impls (`guest.rs`) call only
`data-layer`/`blob-store`/`messaging`". Replace the enumerated list with
"the eight interfaces `guest.rs` implements", and keep the *reason*
(unrelated worlds' component-type sections) verbatim — it is still the
governing rule, and F9 is why the four additions are safe.

### 4.2 `src/types.rs` — three re-exports plus one hand-written module

Append:

```rust
pub mod proxy {
    pub use syneroym_wit_interfaces::proxy::syneroym::proxy::proxy::{
        CallOptions, CallTarget, CalleeError, ProxyError,
    };
}

pub mod app_config {
    pub use syneroym_wit_interfaces::app_config::syneroym::app_config::app_config::ConfigError;
}

pub mod vault {
    pub use syneroym_wit_interfaces::vault::syneroym::vault::vault::VaultError;
}
```

And a **new hand-written module**, `types::http`, holding the records moved
out of `syneroym_core::guest_http` (D-C1-3). Verbatim field order, because
`sandbox_wasm::http::request_to_val` builds a `Val::Record` positionally
from it:

```rust
/// Mirrors `syneroym:http/incoming-handler@0.1.0`'s records field for
/// field, **in the same order** -- the dynamic `Val::Record` the WASM
/// build marshals from these must match the declared field order.
///
/// Hand-written rather than WIT-generated, unlike every other module here:
/// these records are declared inside an interface a component *exports*,
/// so there is no import-direction interface to generate a shared guest
/// view from, and creating one would add a types-only import instance no
/// linker in the tree registers.
pub mod http {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum CallerAuth { Delegated, Ucan, SelfAsserted }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct CallerIdentity {
        pub did: String,
        pub auth: CallerAuth,
        pub app_instance: Option<String>,
    }

    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct HttpRequest {
        pub method: String,
        pub path: String,
        pub query: String,
        pub route: String,
        pub path_params: Vec<(String, String)>,
        pub headers: Vec<(String, String)>,
        pub body: Vec<u8>,
        pub caller: Option<CallerIdentity>,
    }

    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct HttpResponse {
        pub status: u16,
        pub headers: Vec<(String, String)>,
        pub body: Vec<u8>,
    }

    /// Mirrors `syneroym:http/websocket-types@0.1.0`'s `frame-kind`.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum FrameKind { Text, Binary }
}
```

`Default` on `HttpRequest` requires `CallerIdentity: Sized` only (it is
inside an `Option`), so the derive works as it does today on
`GuestHttpRequest`.

### 4.3 `src/lib.rs` — the trait definitions

**Supertrait list** (D-C1-4):

```rust
/// Everything an app may reach. One bound for an app to be generic over.
pub trait AppHost:
    AppDataLayer
    + AppBlobStore
    + AppMessaging
    + AppConversation
    + AppProxy
    + AppAppConfig
    + AppVault
    + AppWebSocket
    + Send
    + Sync
{
}
impl<T> AppHost for T where
    T: AppDataLayer
        + AppBlobStore
        + AppMessaging
        + AppConversation
        + AppProxy
        + AppAppConfig
        + AppVault
        + AppWebSocket
        + Send
        + Sync
{
}
```

**Four new app→host traits**, each mirroring its WIT function for function,
in the same `impl Future + Send` style as the existing four:

```rust
/// Mirrors `syneroym:proxy/proxy@0.1.0`, function for function. `saga` is
/// deliberately absent: no consumer drives a multi-service workflow, and
/// an untested trait is worse than a missing one.
pub trait AppProxy {
    fn call(
        &self,
        target: CallTarget,
        interface: String,
        method: String,
        params: String,
        options: Option<CallOptions>,
    ) -> impl Future<Output = Result<String, ProxyError>> + Send;

    fn enqueue(
        &self,
        target: CallTarget,
        interface: String,
        method: String,
        params: String,
        options: Option<CallOptions>,
    ) -> impl Future<Output = Result<(), ProxyError>> + Send;
}

/// Mirrors `syneroym:app-config/app-config@0.1.0`.
pub trait AppAppConfig {
    fn get(&self, key: String)
        -> impl Future<Output = Result<Option<String>, ConfigError>> + Send;
    fn get_section(
        &self,
        prefix: String,
    ) -> impl Future<Output = Result<Vec<(String, String)>, ConfigError>> + Send;
}

/// Mirrors `syneroym:vault/vault@0.1.0`. One function, and it stays one:
/// `D-06C-4` forbids using `reveal` to hand a signing key to an app.
pub trait AppVault {
    fn reveal(&self, key: String)
        -> impl Future<Output = Result<Vec<u8>, VaultError>> + Send;
}

/// Mirrors `syneroym:http/websocket@0.1.0` -- the *outbound* half of the
/// WebSocket surface (the app pushing a frame to a live connection). The
/// inbound half is `WebSocketSink`, below.
pub trait AppWebSocket {
    fn send(
        &self,
        conn: String,
        frame: Vec<u8>,
        kind: FrameKind,
    ) -> impl Future<Output = Result<(), String>> + Send;
}
```

**No `HttpSink`/`WebSocketSink` here.** An earlier draft of this plan put
two more host→app sink traits in this crate, directly after
`ConversationSink`, with no `CallerContext` parameter — and then, in §6,
wrote the native shim against a *different*, caller-carrying pair with a
different name (`CallerScopedHttpSink`). That fork is a real design error
in the earlier draft, not two valid options; it is resolved here by not
adding these two traits to `syneroym-app-host` at all. The reasoning:

`MessageSink`/`ConversationSink` (the two existing sinks) both hardcode
their caller on the native side —
`NativeFixture::handle_message`/`on_message` always builds their host as
`CallerContext::service_system(&self.service_id)`
([native.rs:74, 87](../../../../test-components/dual-build-fixture/src/native.rs#L74)),
because a message delivery or a conversation delivery is never attributed
to a specific request's caller; it is always the service acting as itself.
Nothing in either trait's signature needs a caller, so neither depends on
`syneroym-rpc::CallerContext`, and both stay in the wasm-portable crate.

**Inbound HTTP is not like that.** Its caller genuinely varies per request
— delegated, UCAN, self-asserted, or system-substituted only for a route
declared `public` (F13) — and the native build must build the *right*
`NativeAppHost` for *that* caller before running the handler, exactly as
`AppSandboxEngine::handle_guest_http_request` builds one fresh `HostState`
per request from the router-verified `CallerContext`
([engine.rs:2500](../../../../crates/sandbox_wasm/src/engine.rs#L2500)).
`request.caller` (the WIT's simplified `caller-identity` — a `did`, an
`auth` level, an `app-instance`) is not enough to reconstruct that: it
carries none of `CallerContext`'s `SessionContext`/capabilities/proof, and
the WASM build never needs it to, because its `HostState.caller` is baked
in at instantiation time, invisibly, before the guest's `handle-request`
export is even called. The native build has no instantiation step to hide
that in — the caller has to reach the shim through an explicit parameter
somewhere. `syneroym-app-host` must compile for `wasm32-wasip2`, so
`CallerContext` cannot live in a trait defined here.

**`HttpSink` and `WebSocketSink` are therefore defined in
`syneroym-app-host-native` instead — see §6.5 — with `CallerContext` as an
explicit parameter, and `NativeFixture` (native-only, §10.4) implements
them there.** `syneroym-app-host`'s `types::http` module (§4.2) still holds
the shared `HttpRequest`/`HttpResponse`/`FrameKind` records both builds use;
only the two *sink traits* move.

### 4.4 `src/guest.rs` — the WASM implementations

Add to the existing `use` block:

```rust
use syneroym_wit_interfaces::{
    app_config::syneroym::app_config::app_config as cfg,
    http_guest::syneroym::http::websocket as ws,
    proxy::syneroym::proxy::proxy as px,
    vault::syneroym::vault::vault as vlt,
};
```

**`http_host` (§3.5's renamed module) must never appear in `guest.rs`** — it
is `cfg(not(wasm32))` wasmtime bindgen, and `guest.rs` is `cfg(wasm32)`
only. `http_guest` (new, wit-bindgen, §4.4 below) is the module this file
actually imports.

Four impls, each one line per method, in the same shape as `AppMessaging`'s:

```rust
impl AppProxy for GuestHost {
    async fn call(
        &self,
        target: CallTarget,
        interface: String,
        method: String,
        params: String,
        options: Option<CallOptions>,
    ) -> Result<String, ProxyError> {
        px::call(&target, &interface, &method, &params, options.as_ref())
    }

    async fn enqueue(/* same params */) -> Result<(), ProxyError> {
        px::enqueue(&target, &interface, &method, &params, options.as_ref())
    }
}

impl AppAppConfig for GuestHost {
    async fn get(&self, key: String) -> Result<Option<String>, ConfigError> { cfg::get(&key) }
    async fn get_section(&self, prefix: String) -> Result<Vec<(String, String)>, ConfigError> {
        cfg::get_section(&prefix)
    }
}

impl AppVault for GuestHost {
    async fn reveal(&self, key: String) -> Result<Vec<u8>, VaultError> { vlt::reveal(&key) }
}
```

**`AppWebSocket for GuestHost` is the one that needs care.** `websocket` is
a *host import* declared in `http.wit`, and `wit_interfaces` has **no guest
module for it** — `src/http.rs` is `cfg(not(wasm32))`. Two options, and the
plan picks the second:

- (a) add an `http-guest-import` world and module, which reintroduces F9's
  types-only-import problem for `websocket-types`;
- (b) **the app's own `generate!` supplies it.** `websocket` has one
  function and its only type (`frame-kind`) already lives in a separate
  `websocket-types` interface, so a component that declares
  `import syneroym:http/websocket@0.1.0` in its own world gets it with
  `"syneroym:http/websocket@0.1.0": generate`, and `guest.rs` cannot
  implement `AppWebSocket for GuestHost` — the binding does not exist in
  this crate.

Under (b), `AppWebSocket` is implemented **not on `GuestHost` but on a
per-component adapter the app's `guest.rs` writes**, which breaks the
`AppHost` blanket bound for `GuestHost`.

> **This is a genuine fork in the design and is flagged, not guessed.**
> The clean resolution, and the plan's recommendation, is to **add
> `websocket` + `websocket-types` to `wit_interfaces` as a guest module**
> after all — `world websocket-import { import websocket; }` in `http.wit`.
> Unlike an `http-types` split (F4), this adds **no** types-only import:
> `websocket` has a real function, so the linker entry
> `build_wasm_linker` already registers
> ([engine.rs:760](../../../../crates/sandbox_wasm/src/engine.rs#L760))
> satisfies it, and `websocket-types` comes along as a `use`d dependency of
> a function-bearing interface, exactly like `stream-types` under
> `messaging`. `interface websocket`, `interface websocket-types` and
> `interface incoming-handler` are all left textually unchanged, so no
> existing component's type changes.
>
> Concretely: append to `http.wit`
> ```wit
> /// Import-only view of the host's WebSocket send capability, for
> /// bindgen consumers that push frames but do not define the handler.
> world websocket-import {
>     import websocket;
> }
> ```
> add `wit_interfaces` feature `http` + module `src/http_guest.rs`
> generating `websocket-import`, add `"http"` to `app_host`'s feature list,
> and implement `AppWebSocket for GuestHost` over
> `syneroym_wit_interfaces::http_guest::syneroym::http::websocket::send`.
> `FrameKind` conversion: `types::http::FrameKind` (hand-written, D-C1-3) ⇄
> the generated `websocket_types::FrameKind`, one two-arm `match` each way
> in `guest.rs` and one in `app_host_native::convert`.
>
> **If `websocket-import` fails to encode** for a reason this plan did not
> foresee, the fallback is to drop `AppWebSocket` from `AppHost`'s
> supertrait list and make it an optional trait an app implements per
> build — and to say so in `status.md` rather than absorbing it.

The concrete impl, once `http_guest` exists:

```rust
impl AppWebSocket for GuestHost {
    async fn send(&self, conn: String, frame: Vec<u8>, kind: FrameKind) -> Result<(), String> {
        ws::send(&conn, &frame, frame_kind_out(kind))
    }
}
```

where `frame_kind_out` converts `types::http::FrameKind` (D-C1-3's
hand-written mirror) into `http_guest::syneroym::http::websocket_types::FrameKind`
— a two-arm `match`, defined once in this file and reused by nothing else
(the native shim's own `convert.rs` defines its own copy, §6.4).

`block_on`'s doc comment ([guest.rs:290](../../../../crates/app_host/src/guest.rs#L290))
still holds for all of these: every new guest call is one synchronous
component-model call, complete on first poll.

---

## §5 `syneroym-rpc` — the native inbound-HTTP contract

### 5.1 `Cargo.toml`

```toml
syneroym-app-host.workspace = true
```

**Verified acyclic on 2026-08-25**, by reading each manifest:
`syneroym-app-host` → `syneroym-wit-interfaces` + `async-trait`;
`syneroym-wit-interfaces` → `serde`, `wit-bindgen`, and (non-wasm32)
`wasmtime` + `syneroym-data-blob`; `syneroym-data-blob` has **no `syneroym-*`
dependencies at all**. So nothing on that chain reaches back to
`syneroym-rpc`. Re-confirm with
`cargo tree -p syneroym-rpc -e normal --invert syneroym-app-host` after the
line is added; if it ever does cycle, the fallback is to put
`NativeHttpService` in `syneroym-core` instead and have `syneroym-core`
depend on `syneroym-app-host` (also acyclic — `syneroym-core →
syneroym-identity` only).

### 5.2 `crates/rpc/src/native_http.rs` — new file

```rust
//! The inbound-HTTP counterpart to [`NativeService`]: how a natively
//! linked app receives a request the router matched against its
//! `http_routes` table, and the WebSocket lifecycle that goes with it.
//!
//! Separate from `NativeService` rather than a method on it: an HTTP route
//! is not a JSON-RPC method, `NativeInvocation` has no place to put a body
//! or headers, and most native services (the control plane, the
//! supervisor) answer JSON-RPC and no HTTP at all.

use std::{fmt::Debug, sync::Arc};

use dashmap::DashMap;
use syneroym_app_host::types::http::{FrameKind, HttpRequest, HttpResponse};

use crate::CallerContext;

/// A natively linked app's inbound HTTP surface. Mirrors
/// `syneroym:http/incoming-handler` and `syneroym:http/websocket-handler`.
///
/// `caller` is the router-verified context, forwarded exactly as
/// `AppSandboxEngine::handle_guest_http_request` forwards it into
/// `HostState.caller`. `None` reaches here only for a route the deploy
/// declared `public`, and the implementation must substitute
/// `CallerContext::service_system(service_id)` -- the same substitution
/// the WASM path makes, in the same place, so an anonymous request is the
/// same principal on both builds.
#[async_trait::async_trait]
pub trait NativeHttpService: Send + Sync + Debug {
    async fn handle_request(
        &self,
        request: HttpRequest,
        caller: Option<CallerContext>,
    ) -> Result<HttpResponse, String>;

    async fn on_websocket_open(&self, conn: String, caller: Option<CallerContext>);
    async fn on_websocket_message(
        &self,
        conn: String,
        frame: Vec<u8>,
        kind: FrameKind,
        caller: Option<CallerContext>,
    );
    async fn on_websocket_close(&self, conn: String, caller: Option<CallerContext>);
}

/// Shared registry of natively linked HTTP surfaces, keyed by `service_id`
/// -- the `guest`/`websocket` route targets' analogue of
/// [`NativeDispatchRegistry`](crate::NativeDispatchRegistry), which covers
/// the `data-layer`/`messaging`/`stream` targets.
pub type NativeHttpRegistry = Arc<DashMap<String, Arc<dyn NativeHttpService>>>;
```

### 5.3 `crates/rpc/src/websocket_senders.rs` — new file (D-C1-6)

```rust
//! The live WebSocket connection table, shared by every build.
//!
//! Lived inside `AppSandboxEngine` until C1. Moved out because a natively
//! linked app must push frames onto the same connections the router
//! registered, and reaching them through `Weak<AppSandboxEngine>` gave a
//! native app a table it could never see into -- `send` failed as
//! "Unknown connection ID", which is the wrong answer for the wrong
//! reason.

use std::sync::Arc;

use dashmap::DashMap;
use syneroym_app_host::types::http::FrameKind;

pub type WebSocketSender = tokio::sync::mpsc::Sender<(Vec<u8>, FrameKind)>;
pub type WebSocketReceiver = tokio::sync::mpsc::Receiver<(Vec<u8>, FrameKind)>;

/// `service_id -> conn_id -> sender`.
#[derive(Debug, Default)]
pub struct WebSocketSenders {
    inner: DashMap<String, Arc<DashMap<String, WebSocketSender>>>,
}

impl WebSocketSenders {
    #[must_use]
    pub fn new() -> Arc<Self> { Arc::new(Self::default()) }

    /// Registers a connection and returns its receiving half. Channel
    /// depth stays at whatever `AppSandboxEngine::register_websocket_sender`
    /// used -- copy the literal across rather than changing it here.
    pub fn register(&self, service_id: &str, conn_id: &str) -> WebSocketReceiver { … }

    pub fn deregister(&self, service_id: &str, conn_id: &str) { … }

    pub fn forget_service(&self, service_id: &str) { … }

    /// `Err` distinguishes the two real failures the WIT's `result<_, string>`
    /// collapses: a `conn_id` this node never had, and one whose peer went
    /// away. Both become the same string at the WIT boundary; keeping them
    /// apart here is what lets a native app log the difference.
    pub async fn send(
        &self,
        service_id: &str,
        conn: &str,
        frame: Vec<u8>,
        kind: FrameKind,
    ) -> Result<(), String> { … }
}
```

### 5.4 `crates/rpc/src/lib.rs`

```rust
mod native_http;
mod websocket_senders;

pub use native_http::{NativeHttpRegistry, NativeHttpService};
pub use websocket_senders::{WebSocketReceiver, WebSocketSender, WebSocketSenders};
```

---

## §6 `syneroym-app-host-native` — the shim

### 6.1 `NativeHostFactory`: five new fields, three new setters

`crates/app_host_native/src/factory.rs`:

```rust
pub struct NativeHostFactory {
    service_id: String,
    key_store: Arc<KeyStore>,
    storage_provider: Arc<dyn StorageProvider>,
    blob_provider: Arc<dyn BlobProvider>,
    broker: Arc<MqttBroker>,
    endpoint_registry: EndpointRegistry,
    logical_resolver: Arc<syneroym_app_orchestration::LogicalResolver>,
    subscriptions: DashMap<String, SubscriptionHandle>,
    sink: OnceLock<Weak<dyn MessageSink>>,
    conversation: Arc<ConversationService>,
    conversation_sink: OnceLock<Weak<dyn ConversationSink>>,

    // -- new in C1 --

    /// The node's `ProxyRouter`, wired after `ConnectionRouter::init` the
    /// same post-construction way `AppSandboxEngine.service_proxy` and
    /// `ConversationService.set_service_proxy` are -- it does not exist
    /// when this factory is built. Unset means `proxy::Host::call`'s own
    /// `"proxy unavailable"` internal error, which is exactly what the
    /// WASM build answers in the same state.
    service_proxy: OnceLock<Weak<dyn ServiceProxy>>,

    /// The live WebSocket connection table, shared with the router and (in
    /// a mixed deployment) with `AppSandboxEngine`. One table, one
    /// `conn_id` namespace.
    websocket_senders: Arc<WebSocketSenders>,

    /// This app's compiled FDAE policy, resolved once. Once, not per call
    /// as the WASM build does: a linked app has no redeploy path in
    /// process, so there is no generation to race
    /// (`AppSandboxEngine::resolve_fdae_policy`'s generation guard exists
    /// for redeploy and has nothing to guard here).
    fdae_policy: tokio::sync::OnceCell<Option<Arc<Policy>>>,

    /// The app's inbound HTTP entry point, mirroring `sink`.
    http_sink: OnceLock<Weak<dyn HttpSink>>,
    /// The app's WebSocket lifecycle entry point, mirroring `sink`.
    websocket_sink: OnceLock<Weak<dyn WebSocketSink>>,
}
```

`NativeHostFactory::new` gains one parameter, `websocket_senders: Arc<WebSocketSenders>`,
and initialises the four new `OnceLock`/`OnceCell`s empty. It already has
`#[allow(clippy::too_many_arguments)]`.

Three new setters, each panicking on a second call exactly as `set_sink`
does:

```rust
pub fn set_service_proxy(&self, proxy: Weak<dyn ServiceProxy>) { … }
pub fn set_http_sink(&self, sink: Weak<dyn HttpSink>) { … }
pub fn set_websocket_sink(&self, sink: Weak<dyn WebSocketSink>) { … }
```

Plus two `pub(crate)` accessors mirroring `subscribe`'s own read of `sink`
([factory.rs:189](../../../../crates/app_host_native/src/factory.rs#L189)),
needed because `NativeHttpAdapter` (§6.5) lives in a different file
(`src/http.rs`) and these two fields are private:

```rust
pub(crate) fn http_sink(&self) -> Option<Arc<dyn HttpSink>> {
    self.http_sink.get().and_then(Weak::upgrade)
}
pub(crate) fn websocket_sink(&self) -> Option<Arc<dyn WebSocketSink>> {
    self.websocket_sink.get().and_then(Weak::upgrade)
}
```

Two new private async resolvers, each mirroring the WASM build's:

```rust
/// Mirrors `AppSandboxEngine::build_store_and_instantiate`'s own read
/// (`engine.rs:1245`), including its "log and fall back to 0" behaviour --
/// a storage blip must not become "this service has no configuration",
/// and it does not on the WASM build either.
pub(crate) async fn config_generation(&self) -> u64 {
    match self.storage_provider.get_latest_config_generation(&self.service_id).await {
        Ok(Some((g, _))) => g,
        Ok(None) => 0,
        Err(e) => { tracing::error!(service_id = %self.service_id, error = %e,
                    "failed to fetch config generation"); 0 }
    }
}

/// Resolved once (see the field's own doc). A parse failure is treated as
/// policy-absent and logged at `error!`, matching
/// `AppSandboxEngine::resolve_fdae_policy`'s own arm -- a policy that
/// cannot be parsed must not silently become a policy that permits
/// everything without saying so.
pub(crate) async fn fdae_policy(&self) -> Option<Arc<Policy>> {
    self.fdae_policy
        .get_or_init(|| async {
            match self.storage_provider.load_fdae_policy(&self.service_id).await {
                Ok(Some(doc)) => match syneroym_fdae::parse_and_validate(&doc) {
                    Ok(p) => Some(Arc::new(p)),
                    Err(e) => { tracing::error!(…, "FDAE policy failed to parse; \
                                treating as policy-absent"); None }
                },
                Ok(None) => None,
                Err(e) => { tracing::error!(…, "failed to load FDAE policy"); None }
            }
        })
        .await
        .clone()
}
```

> **Divergence from the WASM build, stated on purpose.** `resolve_fdae_policy`
> returns *uncached* on a storage error so the next call retries; the
> `OnceCell` above caches `None` permanently on that path. For a linked app
> that is the wrong trade — a blip at first call would disable FDAE for the
> process. **Fix:** use `tokio::sync::RwLock<Option<Option<Arc<Policy>>>>`
> and only memoize the `Ok(_)` arms, returning `None` uncached on `Err`.
> The `OnceCell` sketch above is shown because it reads better; the
> implementation must use the retrying shape. Assert it with a test that
> fails the first `load_fdae_policy` and succeeds the second.

### 6.2 `NativeAppHost`: lazy `HostState` (D-C1-7)

`crates/app_host_native/src/host.rs`:

```rust
#[derive(Debug)]
pub(crate) struct HostInner {
    pub(crate) factory: Arc<NativeHostFactory>,
    pub(crate) caller: CallerContext,
    pub(crate) read_only: bool,
    /// Built on first host call, not in `host_for`: two of its five
    /// per-invocation inputs (`config_generation`, `fdae_policy`) are
    /// async reads the WASM build also does per invocation, and making
    /// `host_for` async would push a boxed-future closure into every
    /// natively linked app's wiring.
    pub(crate) state: tokio::sync::OnceCell<tokio::sync::Mutex<HostState>>,
}

impl HostInner {
    async fn state(&self) -> tokio::sync::MutexGuard<'_, HostState> {
        self.state
            .get_or_init(|| async { tokio::sync::Mutex::new(self.build_state().await) })
            .await
            .lock()
            .await
    }

    async fn build_state(&self) -> HostState {
        let f = &self.factory;
        HostState::new(
            f.service_id.clone(),
            None,
            f.key_store.clone(),
            f.storage_provider.clone(),
            f.blob_provider.clone(),
            self.caller.clone(),
            f.config_generation().await,
            MessagingContext { broker: f.broker.clone(), engine: Weak::new() },
            StreamContext  { registry: f.endpoint_registry.clone(), engine: Weak::new() },
            f.service_proxy.get().cloned().unwrap_or_else(empty_service_proxy),
            f.fdae_policy().await,
            self.read_only,
            // No native `RowAuthorizer` (D-C1-8): a policy carrying ABAC
            // permissions fails closed in `apply_stage4` rather than
            // skipping row authorization. Tested, not assumed.
            syneroym_rpc::empty_row_authorizer(),
            f.endpoint_registry.app_context_of(&f.service_id).map(|(instance, _name)| instance),
            f.logical_resolver.clone(),
        )
        .with_conversation(Arc::downgrade(&f.conversation) as Weak<dyn ConversationHost>)
        .with_websocket_senders(f.websocket_senders.clone())   // new; see §7.2
    }
}
```

**Every `let mut state = self.0.state.lock().await;` in `host.rs` becomes
`let mut state = self.0.state().await;`** — 39 occurrences
(`grep -c 'state.lock().await' crates/app_host_native/src/host.rs`, verified
2026-08-25) across `AppDataLayer` (13), `AppBlobStore` (6),
`NativeBlobWriter`/`NativeBlobReader` (4, through `self.host.0`),
`AppMessaging` (3) and `AppConversation` (13). Purely mechanical.

Two call sites read `read_only` off the locked state and can now read it off
`HostInner` directly ([host.rs:315, 325](../../../../crates/app_host_native/src/host.rs#L315)):

```rust
-        if self.0.state.lock().await.read_only {
+        if self.0.read_only {
```

That also removes an unnecessary `HostState` construction on the
subscribe/unsubscribe denial path.

`factory.rs`'s `host_with` shrinks to:

```rust
pub(crate) fn host_with(self: &Arc<Self>, caller: CallerContext, read_only: bool) -> NativeAppHost {
    NativeAppHost::new(Arc::new(HostInner {
        factory: self.clone(),
        caller,
        read_only,
        state: tokio::sync::OnceCell::new(),
    }))
}
```

The `permitted_differences::each_native_invocation_gets_a_fresh_resource_table`
test ([dual_build_parity.rs:1041](../../../../crates/app_host_native/tests/dual_build_parity.rs#L1041))
still passes: each `host_for` still produces its own `HostInner` and
therefore its own `OnceCell`, hence its own `ResourceTable`. **Re-run it and
confirm — it is the test that would catch a shared-state mistake here.**

### 6.3 Four new trait impls in `host.rs`

Same delegation shape as the existing ones. New host-side imports:

```rust
use syneroym_wit_interfaces::host::syneroym::{
    app_config::app_config::Host as HostAppConfig,
    proxy::proxy::Host as HostProxy,
    vault::vault::Host as HostVault,
};
use syneroym_wit_interfaces::http_host::syneroym::http::websocket::Host as HostWebSocket;
```

(`Host` collides four ways already in this file, hence the aliases —
following the existing `HostStore`/`HostBlobStore`/`HostMessaging`/
`HostConversation` convention and its comment.)

```rust
impl AppProxy for NativeAppHost {
    async fn call(
        &self, target: CallTarget, interface: String, method: String,
        params: String, options: Option<CallOptions>,
    ) -> Result<String, ProxyError> {
        let mut state = self.0.state().await;
        HostProxy::call(
            &mut *state,
            convert::call_target_in(target),
            interface, method, params,
            options.map(convert::call_options_in),
        )
        .await
        .map_err(convert::proxy_error_out)
    }

    async fn enqueue(/* same */) -> Result<(), ProxyError> { /* HostProxy::enqueue, same shape */ }
}

impl AppAppConfig for NativeAppHost {
    async fn get(&self, key: String) -> Result<Option<String>, ConfigError> {
        let mut state = self.0.state().await;
        HostAppConfig::get(&mut *state, key).await.map_err(convert::config_error_out)
    }
    async fn get_section(&self, prefix: String) -> Result<Vec<(String, String)>, ConfigError> {
        let mut state = self.0.state().await;
        HostAppConfig::get_section(&mut *state, prefix).await.map_err(convert::config_error_out)
    }
}

impl AppVault for NativeAppHost {
    async fn reveal(&self, key: String) -> Result<Vec<u8>, VaultError> {
        let mut state = self.0.state().await;
        HostVault::reveal(&mut *state, key).await.map_err(convert::vault_error_out)
    }
}

impl AppWebSocket for NativeAppHost {
    async fn send(&self, conn: String, frame: Vec<u8>, kind: FrameKind) -> Result<(), String> {
        let mut state = self.0.state().await;
        HostWebSocket::send(&mut *state, conn, frame, convert::frame_kind_in(kind)).await
    }
}
```

### 6.4 `convert.rs` — seven new converters

Guest→host (`_in`) and host→guest (`_out`), field-for-field, following the
module's existing doc comment ("Both sides are generated from the same
`.wit`, so every one is a field-for-field copy"):

| Function | Shape |
|---|---|
| `call_target_in(GuestCallTarget) -> HostCallTarget` | 2-arm `match` (`Service`, `Dependency`) |
| `call_options_in(GuestCallOptions) -> HostCallOptions` | 5-field struct copy |
| `callee_error_out(HostCalleeError) -> GuestCalleeError` | 3-field struct copy |
| `proxy_error_out(HostProxyError) -> GuestProxyError` | 9-arm `match`; the `Callee` arm calls `callee_error_out` |
| `config_error_out(HostConfigError) -> GuestConfigError` | 1-arm `match` (`Internal`) |
| `vault_error_out(HostVaultError) -> GuestVaultError` | 3-arm `match` |
| `frame_kind_in` / `frame_kind_out` | 2-arm `match` each way, between `app_host::types::http::FrameKind` and the WIT one |

`ProxyError`'s nine variants, verbatim from
[proxy.wit:30-45](../../../../crates/wit_interfaces/wit/proxy/proxy.wit#L30):
`service-not-found`, `dependency-not-bound`, `unsupported-protocol`,
`unsupported-target`, `permission-denied`, `transport`, `timed-out`,
`callee`, `internal`. **Write the `match` exhaustively with no `_` arm** —
that is what makes a future WIT addition a compile error instead of a
silently mapped `Internal`.

### 6.5 `HttpSink`/`WebSocketSink` and the `NativeHttpService` adapter — new file `crates/app_host_native/src/http.rs`

**The two sink traits are defined in this crate, not in `syneroym-app-host`**
(§4.3 explains why: unlike `MessageSink`/`ConversationSink`, HTTP's caller
varies per request and must be threaded explicitly, and `CallerContext`
cannot live in a crate that compiles for `wasm32-wasip2`). This is the one
place the design forks from `MessageSink`'s shape on purpose, and it forks
exactly once, not twice:

```rust
/// The host -> app direction for inbound HTTP, native-only. Mirrors
/// `syneroym:http/incoming-handler@0.1.0#handle-request`'s contract word
/// for word (`Ok` is the response to send, including a deliberate
/// rejection as an ordinary 4xx `Ok`; `Err` means the handler itself
/// failed and becomes a 500). Takes `caller` explicitly, unlike
/// `MessageSink`/`ConversationSink`: those two always substitute
/// `CallerContext::service_system` on the native side because a delivery
/// is never attributed to a request's caller, but an HTTP caller
/// genuinely varies per call (F13), and `NativeFixture` needs the *real*
/// one to build the right `NativeAppHost` before running the handler --
/// exactly what `AppSandboxEngine::handle_guest_http_request` does by
/// building a fresh `HostState` per request.
#[async_trait::async_trait]
pub trait HttpSink: Send + Sync + core::fmt::Debug {
    async fn handle_request(
        &self,
        caller: CallerContext,
        request: HttpRequest,
    ) -> Result<HttpResponse, String>;
}

/// The host -> app direction for WebSocket lifecycle, native-only, same
/// reasoning as `HttpSink`. Every method returns `()`: the WIT declares no
/// return value, so a native handler must not be able to fail a frame the
/// WASM build could only log.
#[async_trait::async_trait]
pub trait WebSocketSink: Send + Sync + core::fmt::Debug {
    async fn on_open(&self, caller: CallerContext, conn: String);
    async fn on_message(&self, caller: CallerContext, conn: String, frame: Vec<u8>, kind: FrameKind);
    async fn on_close(&self, caller: CallerContext, conn: String);
}
```

The adapter joins `syneroym-rpc`'s router-facing trait to these, doing
**only** the caller substitution — never anything a sink implementor
should decide:

```rust
/// Bridges the router's `NativeHttpService` onto the app's own `HttpSink`/
/// `WebSocketSink`. Holds `Weak` handles for the same reason
/// `NativeHostFactory.sink` does: the app holds the factory, so a strong
/// reference back would be an uncollectable cycle.
#[derive(Debug)]
pub struct NativeHttpAdapter {
    factory: Arc<NativeHostFactory>,
}

#[async_trait::async_trait]
impl NativeHttpService for NativeHttpAdapter {
    async fn handle_request(
        &self,
        request: HttpRequest,
        caller: Option<CallerContext>,
    ) -> Result<HttpResponse, String> {
        // D-A2-7 / F13: the router only sends `None` for a route declared
        // `public`, and the WASM build substitutes the service itself
        // there (`engine.rs:2500`). Substituting the same value in the
        // same place is what keeps an anonymous request the same
        // principal on both builds.
        let caller = caller
            .unwrap_or_else(|| CallerContext::service_system(self.factory.service_id()));
        let Some(sink) = self.factory.http_sink() else {
            return Err("no HTTP sink registered for this native app".to_string());
        };
        sink.handle_request(caller, request).await
    }

    async fn on_websocket_open(&self, conn: String, caller: Option<CallerContext>) {
        let caller = caller.unwrap_or_else(|| CallerContext::service_system(self.factory.service_id()));
        if let Some(sink) = self.factory.websocket_sink() { sink.on_open(caller, conn).await; }
    }
    async fn on_websocket_message(&self, conn: String, frame: Vec<u8>,
                                  kind: FrameKind, caller: Option<CallerContext>) {
        let caller = caller.unwrap_or_else(|| CallerContext::service_system(self.factory.service_id()));
        if let Some(sink) = self.factory.websocket_sink() { sink.on_message(caller, conn, frame, kind).await; }
    }
    async fn on_websocket_close(&self, conn: String, caller: Option<CallerContext>) {
        let caller = caller.unwrap_or_else(|| CallerContext::service_system(self.factory.service_id()));
        if let Some(sink) = self.factory.websocket_sink() { sink.on_close(caller, conn).await; }
    }
}
```

`NativeFixture<H: AppHost>` (§10.4) implements both traits directly — no
second, differently named trait, no split between what the WASM build
implements and what the native build implements. The WASM build never
implements either trait at all: `guest.rs`'s `Fixture` implements the
wit-bindgen-generated `IncomingHandlerGuest`/`WebSocketHandlerGuest` traits
directly (§10.3), which is a different mechanism entirely and was never
going to share a trait with the native path — the same is already true
today for `MessageSink` (`GuestApiGuest` on one side, `MessageSink` on the
other) and needed no reconciling then either.

### 6.6 `lib.rs` re-exports

```rust
mod http;
pub use http::{HttpSink, NativeHttpAdapter, WebSocketSink};
pub use syneroym_app_host::{ConversationSink, MessageSink};
```

`HttpSink`/`WebSocketSink` are re-exported from **this** crate now, not
from `syneroym-app-host` — callers that used to write
`syneroym_app_host::{HttpSink, WebSocketSink}` (there are none yet; this is
new surface) write `syneroym_app_host_native::{HttpSink, WebSocketSink}`
instead. §9.2's `factory.set_http_sink(...)` and `factory.set_websocket_sink(...)`
calls are unaffected by this rename — only the `use` line that brings the
trait names into scope changes.

---

## §7 `syneroym-sandbox-wasm` — what moves and what does not

### 7.1 `core::guest_http` deletion (D-C1-3)

Delete [`crates/core/src/guest_http.rs`](../../../../crates/core/src/guest_http.rs)
and its `pub mod guest_http;` line in `crates/core/src/lib.rs`. Rename at
every use site:

| Old | New |
|---|---|
| `syneroym_core::guest_http::GuestHttpRequest` | `syneroym_app_host::types::http::HttpRequest` |
| `…::GuestHttpResponse` | `…::HttpResponse` |
| `…::GuestCallerIdentity` | `…::CallerIdentity` |
| `…::GuestCallerAuth` | `…::CallerAuth` |

Call sites (all of them; verified by grep on 2026-08-25):

| File | Lines |
|---|---|
| `crates/sandbox_wasm/src/http.rs` | 8-10 (import), 55, 91, 114, 125, 157-158 |
| `crates/sandbox_wasm/src/engine.rs` | 24 (import), 112, 2459 |
| `crates/sandbox_wasm/tests/guest_http_integration.rs` | 13, 103-104, 215 |
| `crates/router/src/route_handler/http.rs` | 54 (import), 519, 1541, 2330-2331, 2392 |

`crates/sandbox_wasm/Cargo.toml` and `crates/router/Cargo.toml` gain
`syneroym-app-host.workspace = true` (both already depend on
`syneroym-wit-interfaces`, so nothing new enters the build graph).

`sandbox_wasm/src/http.rs`'s doc comment says the `Val::Record` field order
"must stay in sync with `crates/wit_interfaces/wit/http/http.wit`" — keep
that sentence and add that the Rust mirror now lives in
`syneroym-app-host::types::http`.

### 7.2 WebSocket senders move out (D-C1-6)

`crates/sandbox_wasm/src/engine.rs`:

- **delete** `pub type WebSocketSender`, `pub type WebSocketReceiver`,
  `type WebSocketSenders` (lines 75-77) — re-export
  `syneroym_rpc::{WebSocketReceiver, WebSocketSender}` in their place so
  `router`'s existing imports keep resolving;
- `pub(crate) websocket_senders: Arc<WebSocketSenders>` (line 321) becomes
  `pub websocket_senders: OnceLock<Arc<syneroym_rpc::WebSocketSenders>>`,
  defaulted to `OnceLock::new()` at line 620 — **not** a new constructor
  parameter. `AppSandboxEngine::init` has **77 call sites**
  (`rg -c 'AppSandboxEngine::init' --type rust`, verified 2026-08-25: ~50 in
  `control_plane/src/service.rs` + `service/orchestration.rs` tests, ~12 in
  `router/tests/`, ~8 in `sandbox_wasm/tests|benches/`, plus `substrate`,
  `smoke-tests`, and `coordinator_iroh/tests/multi_hop_relay.rs`), and every
  one of them would need a new argument for a table only C1's own inbound
  HTTP path and `NativeHostFactory` care about. `AppSandboxEngine` already
  has this exact shape for `service_proxy`, `conversation`, and `self_weak`
  ([engine.rs:194-206](../../../../crates/sandbox_wasm/src/engine.rs#L194)):
  each is a `pub OnceLock<...>` the composition root fills in post-
  construction, read through `.get().cloned().unwrap_or_default()` at the
  per-invocation `HostState::new` call site. `websocket_senders` follows
  the identical pattern — the substrate's `build_route_handler_deps`
  builds the shared `Arc<WebSocketSenders>` once and calls
  `engine.websocket_senders.set(shared.clone()).expect(...)` immediately
  after `Arc::new`-wrapping the engine (the same point `self_weak.set` and
  `conversation.set` already happen).

  **The read is `get_or_init`, not `get().cloned().unwrap_or_default()` —
  this is not optional, and getting it wrong breaks an existing test.**
  `service_proxy`/`self_weak`/`conversation` can fall back to a *fresh*
  empty `Weak` on every read because an unset weak always fails to
  upgrade regardless of which fresh empty one it is — identity doesn't
  matter. `websocket_senders` is different: it is the actual owned table
  registrations live in, so every reader within one engine must see the
  *same* instance, or a registration written by one reader is invisible to
  another. `crates/sandbox_wasm/tests/websocket_integration.rs:95` calls
  `engine.register_websocket_sender(SERVICE_ID, conn_id)` — which never
  goes through `runtime.rs`'s `set` call, since this is a router-independent
  engine-only test — and then awaits `rx.recv()` after driving
  `handle_websocket_on_open`, which reaches the table through
  `HostState.websocket_senders` at a *different* call site. A fresh table
  per read would let `register` write into one throwaway and
  `handle_websocket_on_open`'s `send` write into another, and `rx.recv()`
  would hang forever. The correct read, everywhere this field is touched
  (the three engine methods below and the `HostState::new` call site
  alike), is `self.websocket_senders.get_or_init(syneroym_rpc::WebSocketSenders::new)`
  — lazily creates **one** private table on this engine's first touch and
  returns **`&Arc<WebSocketSenders>`** on every subsequent read, whether or
  not `runtime.rs` ever called `set`. This is what makes every pre-C1 test
  (which never calls `set`) keep working with zero edits: each such
  engine gets exactly one private table, not zero and not a fresh one per
  call. **At the `HostState::new`/`.with_websocket_senders(...)` call
  site this needs an explicit `.clone()`** —
  `self.websocket_senders.get_or_init(syneroym_rpc::WebSocketSenders::new).clone()`
  — because `HostState::new` takes an owned `Arc<WebSocketSenders>` and
  `OnceLock::get_or_init` returns a borrow, `&Arc<T>`. The three engine
  lifecycle methods below don't need the same `.clone()` written out:
  each calls a method on the `Arc` through auto-deref
  (`.get_or_init(...).register(...)`, etc.), so the borrow is enough there;
- `register_websocket_sender` (1518), `deregister_websocket_sender` (1530),
  `forget_websocket_senders` (1539) become one-line delegations through
  `self.websocket_senders.get_or_init(syneroym_rpc::WebSocketSenders::new)`
  (no `.clone()` needed here — see above). Keep them: the router calls them
  by name and they are the engine's own lifecycle surface.
- `pub use …http::…websocket_types::FrameKind` (line 37) becomes
  `pub use syneroym_app_host::types::http::FrameKind`, and the two
  `FrameKind` matches in `handle_websocket_on_message` (2628-2632) convert
  through `frame_kind_out` before building the `Val::Enum`.

`crates/sandbox_wasm/src/host_capabilities.rs`:

- `HostState` gains `websocket_senders: Arc<syneroym_rpc::WebSocketSenders>`,
  defaulted to an empty table in `HostState::new` and set by a
  `with_websocket_senders` builder method (the same shape
  `with_conversation` already uses at [line 350](../../../../crates/sandbox_wasm/src/host_capabilities.rs#L350),
  and for the same reason: it keeps `HostState::new`'s 15-argument
  signature from growing a 16th);
- `impl …websocket::Host for HostState::send` (1861-1878) stops upgrading
  `self.messaging.engine` and calls
  `self.websocket_senders.send(&self.component_id, &conn, frame, kind).await`
  instead. **Behaviour is unchanged for the WASM build** and becomes
  reachable for the native one.

Because `websocket_senders` is a post-construction setter, not a
constructor argument, **`AppSandboxEngine::init`'s signature does not
change**, and none of its 77 call sites need touching. Only two sites act:

| Site | Change |
|---|---|
| `crates/substrate/src/runtime.rs`, `build_route_handler_deps` | builds the shared `Arc<WebSocketSenders>`, wraps the freshly built engine in `Arc`, then `engine.websocket_senders.set(shared.clone()).expect("set once")` — same statement, same place, as the existing `engine.self_weak.set(...)` |
| `crates/app_host_native/tests/dual_build_parity.rs`, `build_wasm_stack` | same `set` call, so the parity harness's WASM stack and native stack (`NativeHostFactory::new`, §9.1) share one table per stack |

Every other existing `AppSandboxEngine::init` call site — every test in
`control_plane`, `router`, `sandbox_wasm`, `coordinator_iroh`, and
`smoke-tests` — needs **no edit**, because an unset `websocket_senders`
falls back to a private empty table exactly as it does today.

---

## §8 `syneroym-router` — the native HTTP hop

### 8.1 `RouteHandlerInner` / `RouteHandlerDeps`

`crates/router/src/route_handler.rs`:

```rust
pub struct RouteHandlerInner {
    …
    pub native_dispatch: NativeDispatchRegistry,
    /// The `guest`/`websocket` route targets' analogue of
    /// `native_dispatch`: a natively linked app's inbound HTTP surface.
    /// Checked before `app_sandbox_engine`, so a service that is linked in
    /// is answered by its own code rather than by "no deployed WASM
    /// component".
    pub native_http: NativeHttpRegistry,
    …
}
```

The same field on `RouteHandlerDeps`, and `RouteHandler::init` copies it
across. `RouteHandler::new_coordinator` (coordinator mode) gets an empty
`Arc::new(DashMap::new())`, matching how it handles `app_sandbox_engine:
None`.

### 8.2 `handle_guest_route` — one new branch, inserted at exactly one point

`crates/router/src/route_handler/http.rs`, in
[`handle_guest_route`](../../../../crates/router/src/route_handler/http.rs#L1467).
Everything before the `app_sandbox_engine` lookup stays where it is — the
`operation != "handle-request"` check, the `D-A2-7` 401, and
`guest_caller_identity` — because all three are service-kind agnostic and
must apply identically.

Restructure the middle of the function:

```
1. operation check                                    (unchanged)
2. D-A2-7 anonymous-on-non-public 401                 (unchanged)
3. caller_identity = guest_caller_identity(...)       (unchanged)
4. NEW: let native = self.route_handler.inner.native_http
             .get(&self.preamble.service_id).map(|e| e.value().clone());
   if native.is_none() {
       // existing engine-availability + is_deployed checks, verbatim
   }
5. body read, header sanitise, path_params, HttpRequest construction
                                                       (unchanged, moved up
                                                        so both paths share it)
6. match native {
       Some(svc) => match svc.handle_request(request, self.caller.clone()).await {
           Ok(response)  => Ok(build_guest_response(response)),
           Err(detail)   => { warn!(...); Ok(http_error(INTERNAL_SERVER_ERROR, detail)) }
       },
       None => /* existing app_sandbox_engine.handle_guest_http_request match, verbatim */
   }
```

**Deliberate differences from the WASM arm, and why each is right:**

- there is **no `GuestHttpFailure::Unavailable` / 503+`retry-after`** arm:
  that models wasmtime pool pressure and the per-service guest-HTTP
  admission semaphore, neither of which exists for in-process code. A
  natively linked app that wants to shed load returns a 503 itself, as an
  ordinary `Ok`;
- there is **no `NoHandler` arm**: registration in `native_http` *is* the
  handler's existence;
- `Err(detail)` maps to 500 with the message, matching the WIT's own
  "`Err` means the handler itself failed, and becomes a 500 carrying the
  message".

These belong in `status.md` as permitted differences, and the parity suite
asserts the *shared* behaviour (status, headers, body) rather than the
failure taxonomy.

`build_guest_response` ([http.rs:519](../../../../crates/router/src/route_handler/http.rs#L519))
is reused unchanged — it already caps the response body, validates the
status range and strips framing headers, and all three must apply to the
native build identically.

### 8.3 `handle_websocket_route`

Same shape, at three points:

- the `is_deployed` guard becomes "`native_http` has an entry **or** the
  engine has a deployed component";
- `acquire_websocket_permit` has no native analogue — for a native service
  skip it, and record the difference (a linked app owns its own
  concurrency);
- `register_websocket_sender` moves to the shared table (§7.2), so the same
  call works for both;
- the three `engine.handle_websocket_on_{open,message,close}` calls become a
  `match` on which side owns the service.

To keep the (already long) upgrade task readable, factor the three
dispatch points behind a small local enum:

```rust
enum WsTarget { Native(Arc<dyn NativeHttpService>), Wasm(Arc<AppSandboxEngine>) }

impl WsTarget {
    async fn on_open(&self, service_id: &str, conn: &str, caller: Option<CallerContext>) { … }
    async fn on_message(&self, …) { … }
    async fn on_close(&self, …) { … }
}
```

resolved once before `tokio::task::spawn` and moved into it.

---

## §9 `syneroym-substrate` — wiring

### 9.1 `SharedNodeHandles` gains three fields

`crates/substrate/src/runtime.rs`:

```rust
struct SharedNodeHandles {
    …
    /// The per-service HTTP route table. A linked native app has no deploy
    /// record, so nothing else would ever put its routes here.
    #[cfg_attr(not(feature = "dual_build_fixture"), allow(dead_code))]
    http_routes: HttpRouteRegistry,
    /// The `guest`/`websocket` route targets' native registry.
    native_http: NativeHttpRegistry,
    /// The shared live-WebSocket table (`AppSandboxEngine` holds the same
    /// `Arc`).
    websocket_senders: Arc<WebSocketSenders>,
}
```

`build_route_handler_deps` already builds `http_routes` at
[line 1143](../../../../crates/substrate/src/runtime.rs#L1143); it now also
builds `native_http` and `websocket_senders` there. `websocket_senders` is
**not** a new `AppSandboxEngine::init` argument (§7.2 corrects an earlier
draft of this plan on exactly that point) — instead, immediately after the
engine is `Arc`-wrapped, `build_route_handler_deps` calls
`engine.websocket_senders.set(websocket_senders.clone()).expect("set once")`,
the same statement shape as the existing `engine.self_weak.set(...)`. All
three (`http_routes`, `native_http`, `websocket_senders`) go into
`RouteHandlerDeps` and are cloned into `SharedNodeHandles`.

### 9.2 `init_dual_build_fixture` gains the new wiring

The function's existing `use` block
(`use syneroym_app_host_native::{MessageSink, NativeHostFactory};`,
[runtime.rs:995](../../../../crates/substrate/src/runtime.rs#L995)) grows to
`use syneroym_app_host_native::{HttpSink, MessageSink, NativeHostFactory, NativeHttpAdapter, WebSocketSink};`
— `HttpSink`/`WebSocketSink` come from `syneroym-app-host-native` (§6.6),
not `syneroym-app-host`.

```rust
    let factory = NativeHostFactory::new(
        service_id.clone(),
        shared.key_store.clone(),
        shared.storage_provider.clone(),
        shared.blob_provider.clone(),
        shared.messaging_broker.clone(),
        endpoint_registry.clone(),
        shared.logical_resolver.clone(),
        shared.conversation.clone(),
        shared.websocket_senders.clone(),          // new
    );
    let f = factory.clone();
    let fixture = Arc::new(NativeFixture::new(service_id.clone(), move |caller| f.host_for(caller)));
    factory.set_sink(…);
    factory.set_conversation_sink(…);
    factory.set_http_sink(Arc::downgrade(&fixture) as Weak<dyn HttpSink>);          // new
    factory.set_websocket_sink(Arc::downgrade(&fixture) as Weak<dyn WebSocketSink>); // new

    shared.native_dispatch.insert(DUAL_BUILD_FIXTURE_DISPATCH_ID.to_string(),
                                  fixture.clone() as Arc<dyn NativeService>);

    // New: the router's HTTP hop, and the route table a deploy would
    // otherwise have written. `public: false` throughout -- the fixture
    // has no access control of its own (see the feature's Cargo.toml
    // comment), and a public route would let an unauthenticated caller
    // reach it as the service itself (D-A2-7 / `HttpRoute.public`).
    shared.native_http.insert(
        DUAL_BUILD_FIXTURE_DISPATCH_ID.to_string(),
        Arc::new(NativeHttpAdapter::new(factory.clone())) as Arc<dyn NativeHttpService>,
    );
    shared.http_routes.insert(
        DUAL_BUILD_FIXTURE_DISPATCH_ID.to_string(),
        vec![
            HttpRoute { method: "POST".into(), path: "/run".into(),
                        target: "guest".into(), operation: "handle-request".into(),
                        collection: None, topic: None, protocol: None, public: false },
            HttpRoute { method: "GET".into(), path: "/ws".into(),
                        target: "websocket".into(), operation: "handle-upgrade".into(),
                        collection: None, topic: None, protocol: None, public: false },
        ],
    );

    // NEW, and load-bearing: without this, `resolve_route` never fires --
    // an inbound HTTP connection resolves its target through the
    // reserved `"http-native"` interface name *before* `http_routes` is
    // ever consulted (`route_handler/http.rs:199-200`: "a client connects
    // once with `http://http-native|<service_id>`", the service_id
    // resolved once per connection from that same reserved name). A
    // deployed service gets this automatically -- every one of
    // `NATIVE_CAPABILITY_INTERFACES` (`data-layer`, `vault`, `app-config`,
    // `blob-store`, `messaging`, `http-native`, `conversation`) is
    // registered for it by `register_wasm_endpoints`
    // (`control_plane/src/service/orchestration.rs:2119-2126`). A linked
    // app has no deploy, so this registration is C1's own to make, keyed
    // by the **fixture's own dispatch id**, not the node's -- a different
    // key from the existing `(node_service_id, FIXTURE_INTERFACE)`
    // registration a few lines above, which resolves JSON-RPC `run` calls,
    // not HTTP connections. The two do not collide with each other or
    // with the file's existing "exactly one endpoint" warning, which is
    // about a third, unrelated key: `(node_did, "messaging")`.
    endpoint_registry
        .register(
            DUAL_BUILD_FIXTURE_DISPATCH_ID.to_string(),
            "http-native".to_string(),
            SubstrateEndpoint::NativeHostChannel {
                service_id: DUAL_BUILD_FIXTURE_DISPATCH_ID.to_string(),
            },
        )
        .await?;
```

`HttpRoute` derives `Deserialize` only, so it must be constructed
field-by-field (no `serde_json::from_value` shortcut needed, but either is
fine).

### 9.3 The `ProxyRouter` hand-off

Immediately after
[`if let Some(proxy) = router.proxy() { shared.conversation.set_service_proxy(…) }`](../../../../crates/substrate/src/runtime.rs#L772),
the same block sets the fixture factory's proxy. The factory must therefore
outlive `init_dual_build_fixture` — change it to **return
`Option<Arc<NativeHostFactory>>`** rather than `()` and keep the handle:

```rust
    #[cfg(feature = "dual_build_fixture")]
    let fixture_factory = init_dual_build_fixture(&shared, &endpoint_registry, service_id).await?;

    let router = ConnectionRouter::init(…).await?;

    if let Some(proxy) = router.proxy() {
        shared.conversation.set_service_proxy(Arc::downgrade(&proxy) as Weak<dyn ServiceProxy>);
        #[cfg(feature = "dual_build_fixture")]
        fixture_factory.set_service_proxy(Arc::downgrade(&proxy) as Weak<dyn ServiceProxy>);
    }
```

This ordering — factory before router, proxy after — is the **same
two-phase wiring** `AppSandboxEngine.service_proxy` and
`ControlPlaneService.service_proxy` already use, and the reason is
identical: `ProxyRouter` does not exist until `ConnectionRouter::init`
returns.

---

## §10 The fixture

### 10.1 `wit/world.wit`

```wit
world dual-build-fixture {
    import syneroym:data-layer/store@0.1.0;
    import syneroym:blob-store/blob-store@0.1.0;
    import syneroym:messaging/host-api@0.1.0;
    import syneroym:conversation/conversation@0.1.0;
    import syneroym:proxy/proxy@0.1.0;
    import syneroym:app-config/app-config@0.1.0;
    import syneroym:vault/vault@0.1.0;
    import syneroym:http/websocket@0.1.0;

    export syneroym:messaging/stream-types@0.1.0;
    export syneroym:messaging/guest-api@0.1.0;
    export syneroym:conversation/guest-api@0.1.0;
    export syneroym:http/incoming-handler@0.1.0;
    export syneroym:http/websocket-handler@0.1.0;

    export test-driver;
}
```

### 10.2 New WIT dep symlinks

```bash
cd test-components/dual-build-fixture/wit/deps
mkdir -p proxy app-config vault http
ln -s ../../../../../crates/wit_interfaces/wit/proxy/proxy.wit               proxy/proxy.wit
ln -s ../../../../../crates/wit_interfaces/wit/app-config/app-config.wit     app-config/app-config.wit
ln -s ../../../../../crates/wit_interfaces/wit/host/deps/vault/vault.wit     vault/vault.wit
ln -s ../../../../../crates/wit_interfaces/wit/http/http.wit                 http/http.wit
```

and the matching `[package.metadata.component.target.dependencies]` entries
in `test-components/dual-build-fixture/Cargo.toml`:

```toml
"syneroym:proxy"      = { path = "wit/deps/proxy" }
"syneroym:app-config" = { path = "wit/deps/app-config" }
"syneroym:vault"      = { path = "wit/deps/vault" }
"syneroym:http"       = { path = "wit/deps/http" }
```

plus the four new `syneroym-wit-interfaces` features (`proxy`, `app-config`,
`vault`, `http`) on the wasm32 dependency.

### 10.3 `src/guest.rs` — remap the four new imports, generate the two new exports

```rust
        with: {
            "syneroym:data-layer/store@0.1.0":            syneroym_wit_interfaces::data_layer::syneroym::data_layer::store,
            "syneroym:blob-store/blob-store@0.1.0":       syneroym_wit_interfaces::blob_store::syneroym::blob_store::blob_store,
            "syneroym:messaging/host-api@0.1.0":          syneroym_wit_interfaces::messaging::syneroym::messaging::host_api,
            "syneroym:messaging/stream-types@0.1.0":      generate,
            "syneroym:conversation/conversation@0.1.0":   syneroym_wit_interfaces::conversation::syneroym::conversation::conversation,
            "syneroym:proxy/proxy@0.1.0":                 syneroym_wit_interfaces::proxy::syneroym::proxy::proxy,
            "syneroym:app-config/app-config@0.1.0":       syneroym_wit_interfaces::app_config::syneroym::app_config::app_config,
            "syneroym:vault/vault@0.1.0":                 syneroym_wit_interfaces::vault::syneroym::vault::vault,
            "syneroym:http/websocket@0.1.0":              syneroym_wit_interfaces::http_guest::syneroym::http::websocket,
            "syneroym:http/websocket-types@0.1.0":        generate,
            "syneroym:http/incoming-handler@0.1.0":       generate,
        },
```

and the two new `Guest` impls, converting the generated export types into
`syneroym_app_host::types::http`'s hand-written mirror (D-C1-3 — this
conversion is the price of not splitting `http.wit`, and it is ~30 lines
in one place):

```rust
impl IncomingHandlerGuest for Fixture {
    fn handle_request(request: bindings::exports::…::HttpRequest)
        -> Result<bindings::exports::…::HttpResponse, String>
    {
        let out = block_on(crate::app::handle_http(&GuestHost, http_request_in(request)))?;
        Ok(http_response_out(out))
    }
}

impl WebSocketHandlerGuest for Fixture {
    fn on_open(conn: String)    { block_on(crate::app::on_ws_open(&GuestHost, conn)); }
    fn on_message(conn: String, frame: Vec<u8>, kind: bindings::…::FrameKind) {
        block_on(crate::app::on_ws_message(&GuestHost, conn, frame, frame_kind_in(kind)));
    }
    fn on_close(conn: String)   { block_on(crate::app::on_ws_close(&GuestHost, conn)); }
}
```

`http_request_in` / `http_response_out` / `frame_kind_in` are private
helpers in this file, each a field-for-field copy. **A unit test in
`guest.rs` is impossible (wasm32-only), so the parity suite is what proves
they are right** — an HTTP scenario whose response echoes every field of the
request is the cheapest way to catch a transposed pair.

### 10.4 `src/native.rs` — two new impls on `NativeFixture<H>`

Both traits come from `syneroym_app_host_native` (§6.5), both take
`caller: CallerContext` directly — `NativeHttpAdapter` already resolved the
`None`-means-`public`-route substitution before calling either, so
`NativeFixture` never has to:

```rust
#[async_trait::async_trait]
impl<H: AppHost + 'static> syneroym_app_host_native::HttpSink for NativeFixture<H> {
    async fn handle_request(
        &self,
        caller: CallerContext,
        request: HttpRequest,
    ) -> Result<HttpResponse, String> {
        let host = (self.host_for)(caller);
        crate::app::handle_http(&host, request).await
    }
}

#[async_trait::async_trait]
impl<H: AppHost + 'static> syneroym_app_host_native::WebSocketSink for NativeFixture<H> {
    async fn on_open(&self, caller: CallerContext, conn: String) {
        // The WASM build's `handle_websocket_on_open` forwards the
        // upgrade's real caller and substitutes `service_system` only when
        // it is `None` (`engine.rs:2548`); `NativeHttpAdapter::on_websocket_open`
        // already applied that same substitution before this call, so
        // `caller` here is always the right identity to build the host
        // under -- never a second substitution.
        let host = (self.host_for)(caller);
        crate::app::on_ws_open(&host, conn).await;
    }
    async fn on_message(&self, caller: CallerContext, conn: String, frame: Vec<u8>, kind: FrameKind) {
        let host = (self.host_for)(caller);
        crate::app::on_ws_message(&host, conn, frame, kind).await;
    }
    async fn on_close(&self, caller: CallerContext, conn: String) {
        let host = (self.host_for)(caller);
        crate::app::on_ws_close(&host, conn).await;
    }
}
```

### 10.5 `src/app.rs` — new verbs and two new entry points

New `Request` variants (kebab-case `op` tags, no planning identifiers per
`D-06C-11`):

| `op` | Fields | What it proves |
|---|---|---|
| `proxy-call-self` | `interface`, `method`, `params` | `call-target::service(<own id>)`, the self-proxy caller-forwarding rule |
| `proxy-call-dependency` | `name`, `interface`, `method`, `params` | `call-target::dependency`, host-side `LogicalResolver` resolution |
| `proxy-call-unbound-dependency` | `name` | `DependencyNotBound`, byte-identical on both builds |
| `proxy-call-cross-service-native` | `target`, `interface`, `method`, `params` | `call-target::service(<a different DID>)` against a **native capability** interface (e.g. `data-layer`) of that other service — the shape `check_native_capability_gate` refuses. Not usable against `StubProxy` (§11.1/§11.3's own caveat); this op exists for §11.5's router-level test, which is the only place a real `ProxyRouter` runs |
| `proxy-enqueue` | `name`, `idempotency_key` | `enqueue` accepted with a key |
| `proxy-enqueue-no-key` | `name` | the `enqueue`-without-a-key refusal, verbatim message |
| `proxy-enqueue-empty-key` | `name` | the empty-key refusal |
| `read-config` | `key` | `app-config::get` |
| `read-config-section` | `prefix` | `app-config::get-section` |
| `reveal-secret` | `key` | `vault::reveal` — hit and `not-found` (§11.1: both stacks seed a secret via `ServiceStore::write_secret` before the scenario table runs) |
| `ws-send` | `conn`, `body` | `websocket::send` to a live conn and to an unknown one |

New entry points, target-independent, beside `on_message` /
`on_conversation_message`:

```rust
/// Echoes every field of the request back as JSON, plus a `seen` counter
/// persisted through `data-layer` -- never in-process state, the same rule
/// `INBOX` follows: a WASM `handle-request` gets a fresh instance, so a
/// static would not survive to the next call.
pub async fn handle_http<H: AppHost>(host: &H, req: HttpRequest) -> Result<HttpResponse, String>

pub async fn on_ws_open<H: AppHost>(host: &H, conn: String)
pub async fn on_ws_message<H: AppHost>(host: &H, conn: String, frame: Vec<u8>, kind: FrameKind)
pub async fn on_ws_close<H: AppHost>(host: &H, conn: String)
```

`handle_http` must switch on `req.path` the way `http-guest-test` does, with
at least: `/echo` (mirror every field), `/reject` (an ordinary `Ok` with a
4xx status), `/fail` (an `Err`), and `/whoami` (render `req.caller`) — those
four cover the WIT's entire stated contract.

The three WebSocket callbacks persist to a `ws_log` collection, read back by
a `read-ws-log` verb, for the same reason `CONV_INBOX` exists.

---

## §11 The parity suite

`crates/app_host_native/tests/dual_build_parity.rs`.

### 11.1 Harness changes

- `build_wasm_stack` / `build_native_stack` each build a **shared
  `Arc<WebSocketSenders>`** (one per stack, not one across both — the two
  stacks are independent nodes). On the WASM side this is
  `engine.websocket_senders.set(table.clone())` **after** `Arc::new`-wrapping
  the engine, not an `AppSandboxEngine::init` argument (§7.2). On the native
  side it is the new `NativeHostFactory::new` parameter (§6.1);
- `NativeHostFactory::new` gains one parameter (`websocket_senders`), which
  has **three** call sites, not two: `crates/substrate/src/runtime.rs`
  (§9.2), this harness's `build_native_stack` (§11.1), **and
  `crates/app_host_native/src/factory.rs:333`** — the crate's own
  `read_only_host_denies_every_mutating_and_egress_call` unit test, inside
  `#[cfg(test)] mod tests`. That call site needs the same new argument
  (`syneroym_rpc::WebSocketSenders::new()` is enough; the test never
  exercises `websocket::send`);
- both stacks get a real `LogicalResolver` (not `empty_resolver()`) with one
  registered `TopologyKey::local(app_instance, "sibling")` entry, and
  `endpoint_registry.set_app_context(SERVICE_ID, app_instance, "self")` so
  `app_context_of` returns `Some` and `call-target::dependency` resolves;
- both stacks get a `StubProxy: ServiceProxy` that answers a fixed
  `(interface, method)` with a deterministic value and everything else with
  `UnsupportedTarget`. Wired as `engine.service_proxy.set(…)` on the WASM
  side and `factory.set_service_proxy(…)` on the native side. **It must be
  the same stub type for both**, or the comparison proves nothing.
  **This stub does not exercise `check_native_capability_gate`** — see
  §11.5's `a_cross_service_call_by_did_is_gated_identically_on_both_builds`
  for the test that does, over the real router;
- both stacks write the same config generation
  (`storage_provider.save_config_generation(SERVICE_ID, r#"{"greeting":"hello","db.host":"x","db.port":"5432"}"#)`)
  so `read-config`/`read-config-section` have something to find. **Check
  `save_config_generation`'s exact signature before writing this** — it is
  on `StorageProvider` (`crates/data_db/src/traits.rs:60`);
- **both stacks seed one vault secret before running any scenario**, so
  `reveal-secret` can compare a real hit, not only `NotFound`. The writer
  path exists — `ServiceStore::write_secret`
  ([traits.rs:137](../../../../crates/data_db/src/traits.rs#L137)),
  implemented at
  [sqlite.rs:1991](../../../../crates/data_db/src/sqlite.rs#L1991) via
  `DbCommand::WriteSecret` → `INSERT OR REPLACE INTO _vault`
  ([sqlite.rs:1519](../../../../crates/data_db/src/sqlite.rs#L1519)) —
  and is already exercised end to end by
  `test_vault_write_and_reveal`
  ([sqlite.rs:2635](../../../../crates/data_db/src/sqlite.rs#L2635)):
  `SqliteStorageProvider::new(dir, true)` (encryption **on**) +
  `key_store.inject_kek([..; 32])` + `provider.open_service_db(...).await?.write_secret("api_key", secret).await?`.
  **This changes both stacks' storage-provider construction** from the
  existing harness's `SqliteStorageProvider::new(dir, false)`
  (`encryption_enabled = false`) to `..., true)` plus an injected KEK on
  each stack's own `KeyStore` — matching the setup
  `dual_build_fixture_e2e.rs` already uses for the router-level suite
  (§11.5), so C1 is not introducing a second encryption posture, only
  extending the in-process one to match it. **The `inject_kek` call has to
  land immediately after `KeyStore::new()`, before anything else touches
  `key_store`** — both `build_wasm_stack` and `build_native_stack` move
  `key_store` by value into their respective constructors
  (`AppSandboxEngine::init`,
  [dual_build_parity.rs:387](../../../../crates/app_host_native/tests/dual_build_parity.rs#L387);
  `NativeHostFactory::new`,
  [dual_build_parity.rs:433](../../../../crates/app_host_native/tests/dual_build_parity.rs#L433)),
  so an `inject_kek` written after either call would not compile (`key_store`
  already moved), and one squeezed in between the `.clone()` used for
  `test_conversation_service` and the move would work but reads as an
  afterthought — put it on the line directly after `Arc::new(KeyStore::new())`
  in both functions, before any clone. `verify_encryption_mode` only
  gates on a missing KEK when `encryption_enabled` is `true`
  ([sqlite.rs:1363-1376](../../../../crates/data_db/src/sqlite.rs#L1363)),
  so this also means `store-messages`/`store-messages`-adjacent data-layer
  scenarios now run under a real DEK on both builds, which they did not
  before — worth a quick look at whether any existing scenario's expected
  output assumed unencrypted storage (none should: the WIT boundary never
  exposes ciphertext, but confirm rather than assume);
- `Harness` gains `wasm_http: WasmHttpDriver`, `native_http: NativeHttpDriver`.

### 11.2 The HTTP driver pair

```rust
trait HttpDriver {
    async fn get(&self, path: &str, caller: Option<CallerContext>) -> HttpResponse;
}

struct WasmHttpDriver { engine: Arc<AppSandboxEngine> }
impl HttpDriver for WasmHttpDriver {
    async fn get(&self, path: &str, caller: Option<CallerContext>) -> HttpResponse {
        match self.engine.handle_guest_http_request(SERVICE_ID, &request(path), caller).await {
            Ok(GuestHttpOutcome::Response(r)) => r,
            other => panic!("wasm http driver: {other:?}"),
        }
    }
}

struct NativeHttpDriver { adapter: Arc<NativeHttpAdapter> }
impl HttpDriver for NativeHttpDriver {
    async fn get(&self, path: &str, caller: Option<CallerContext>) -> HttpResponse {
        self.adapter.handle_request(request(path), caller).await.expect("native http driver")
    }
}
```

`request(path)` builds one `HttpRequest` literal shared by both, so any
divergence is in the handling, never in the input.

### 11.3 New rows in the `SCENARIOS` byte-comparison table

Deterministic outputs only, matching the table's existing rule:

```rust
("proxy-call-self",              r#"{"op":"proxy-call-self","interface":"…","method":"…","params":"{}"}"#),
("proxy-call-dependency",        r#"{"op":"proxy-call-dependency","name":"sibling",…}"#),
("proxy-unbound-dependency",     r#"{"op":"proxy-call-unbound-dependency","name":"nope"}"#),
("proxy-enqueue-no-key",         r#"{"op":"proxy-enqueue-no-key","name":"sibling"}"#),
("proxy-enqueue-empty-key",      r#"{"op":"proxy-enqueue-empty-key","name":"sibling"}"#),
("read-config",                  r#"{"op":"read-config","key":"greeting"}"#),
("read-config-missing",          r#"{"op":"read-config","key":"absent"}"#),
("read-config-section",          r#"{"op":"read-config-section","prefix":"db"}"#),
// "known" is the key both stacks seed via `write_secret` before the
// harness runs any scenario (§11.1).
("reveal-secret",                r#"{"op":"reveal-secret","key":"known"}"#),
("reveal-secret-missing",        r#"{"op":"reveal-secret","key":"absent"}"#),
("ws-send-unknown-conn",         r#"{"op":"ws-send","conn":"nope","body":"hi"}"#),
```

`proxy-enqueue` with a real key is **not** in this table: it writes to a
durable outbox and its observable result depends on delivery timing. It gets
its own named test asserting only "accepted".

**What this table does and does not prove about `proxy`.** `proxy-call-self`
already exercises `call-target::service(<own id>)` and
`proxy-call-dependency` exercises `call-target::dependency` — that is
`task.md`'s "both target shapes" for *construction and resolution*. What
none of these scenarios can prove is `check_native_capability_gate`
([router/src/proxy.rs:589](../../../../crates/router/src/proxy.rs#L589))
denying a **cross-service** call to a **different** service's native
capability: `StubProxy` (§11.1) is not the real `ProxyRouter`, so the gate
never runs here regardless of which target shape is used. That is real
substrate-routing behavior, not shim behavior, and it needs the real router
— §11.5 covers it, not this table. Do not read a passing
`proxy-call-self`/`proxy-call-dependency` pair as evidence the gate is
enforced identically on both builds; only the router-level test is.

### 11.4 New named tests

| Test | Asserts |
|---|---|
| `both_builds_answer_an_http_request_identically` | `/echo` through both drivers → equal `HttpResponse` |
| `both_builds_render_the_same_caller_for_a_delegated_request` | `/whoami` with a `Delegated` caller → equal body |
| `both_builds_substitute_the_service_itself_for_an_anonymous_public_request` | `/whoami` with `caller: None` → both render `system:<SERVICE_ID>` (F13) |
| `a_guest_rejection_is_an_ok_with_a_4xx_on_both_builds` | `/reject` → `status == 403`, not an `Err` |
| `a_handler_failure_is_an_err_on_both_builds` | `/fail` → `Err` on both |
| `both_builds_deliver_websocket_frames_to_the_app` | register a conn in the shared table, drive `on_open`/`on_message`/`on_close`, read `read-ws-log` back → equal |
| `both_builds_push_a_frame_to_a_live_connection` | `ws-send` to a registered conn → the receiver sees it, on both |
| `a_dependency_resolves_to_the_same_target_on_both_builds` | `proxy-call-dependency` → equal payload, and the stub saw the same `target_service` |
| `an_enqueue_without_an_idempotency_key_is_refused_identically` | already in the table; keep the named version for a legible failure |
| `both_builds_read_the_same_config_generation` | write generation 2 after the harness is up, then `read-config` → both see the *new* value (this is the test D-C1-7's laziness exists for) |
| **`a_policy_with_abac_permissions_fails_closed_on_the_native_build`** | install an FDAE policy with a stage-4 after-step; the WASM build authorizes rows, the native build returns the `AbacError::Unavailable`-derived error. **Lives in `permitted_differences`, not in the parity table** (D-C1-8) |
| `a_transient_fdae_policy_load_failure_is_not_memoized` | first `load_fdae_policy` errors, second succeeds → the second `host_for` sees the policy (the retrying-shape requirement in §6.1) |

`the_parity_comparison_detects_a_divergence`'s `Mutant` currently rewrites
`"written"` → `"wrote"`, which only appears in `store-messages`. It keeps
working, and needs no change.

### 11.5 The router-level feature-path test

B3 shipped
[`crates/substrate/tests/dual_build_fixture_e2e.rs`](../../../../crates/substrate/tests/dual_build_fixture_e2e.rs)
— one `#[tokio::test]`,
`a_client_reaches_the_linked_native_fixture_through_the_router`, standing up
a real `SubstrateTestContext`, injecting a KEK, and driving
`FIXTURE_INTERFACE::run` through `SyneroymClient::request`. Its own doc
comment draws the line C1 must keep: *"The in-process
`dual_build_parity.rs` suite proves the shim itself; this proves the
registration."*

**Extend that file, do not add a second one.** One new test, over the
**same transport `guest_http_e2e.rs` uses for its inbound-HTTP tests** — a
raw Iroh QUIC bidi stream carrying an `http://http-native|<service_id>`
preamble, built with `RoutePreamble { transport: RouteTransport::Http,
protocol: RouteProtocol::JsonRpc, interface: "http-native".to_string(),
service_id: DUAL_BUILD_FIXTURE_DISPATCH_ID.to_string(), … }` followed by
raw HTTP/1.1 bytes over that stream — **not** the client-gateway port.
`guest_http_e2e.rs`'s own `open_http_stream` helper
([guest_http_e2e.rs:129](../../../../crates/substrate/tests/guest_http_e2e.rs#L129))
does exactly this today, against a *deployed* service; reuse it verbatim,
passing `DUAL_BUILD_FIXTURE_DISPATCH_ID` as `service_id`. (`guest_http_e2e.rs`
does have one gateway-port test, for the client-gateway hostname flow
specifically — that is a different, unrelated leg, out of scope here; see
§12's ambiguity note on the gateway.)

```rust
#[tokio::test]
async fn an_http_request_reaches_the_linked_native_fixture_through_the_router() {
    // Same setup + inject_kek as the existing test.
    // Open a raw Iroh bidi stream with `open_http_stream`'s preamble shape,
    // targeting DUAL_BUILD_FIXTURE_DISPATCH_ID over interface "http-native"
    // (the endpoint §9.2 now registers). Write a raw HTTP/1.1 GET /echo
    // request, read the response, assert the fixture's echo body -- i.e.
    // that `http_routes`, `native_http`, and the new "http-native" registry
    // entry were all populated and that `handle_guest_route`'s native
    // branch fired.
}
```

This is the one place in C1 where inbound HTTP is proven through the **real
router**, which is what makes §11.2's in-process drivers sufficient for
everything else.

A second new test in the same file answers §11.3's own caveat — proving
`check_native_capability_gate` denies a natively linked app's cross-service
call, over the real `ProxyRouter`, the same way
`crates/router/tests/proxy_dispatch.rs`'s existing
`guest_cross_service_native_capability_through_proxy_is_permission_denied`
([proxy_dispatch.rs:467](../../../../crates/router/tests/proxy_dispatch.rs#L467))
already proves it for a WASM component:

```rust
#[tokio::test]
async fn a_cross_service_call_by_did_is_gated_identically_on_both_builds() {
    // Same substrate setup as the existing test, plus a second deployed
    // (or linked) service to serve as the target. Drive the fixture's
    // `proxy-call-cross-service-native` op (target = the second service's
    // DID, interface = "data-layer"), through the router's real JSON-RPC
    // dispatch to the linked native fixture. Assert the response is a
    // `PermissionDenied`-shaped proxy error -- the same shape
    // `proxy_dispatch.rs`'s WASM-side test asserts (`message.contains
    // ("PermissionDenied")`), proving the gate applies identically to a
    // native `proxy::call` even though it is reached through
    // `NativeAppHost` rather than a wasmtime `Host` impl -- correct
    // because both delegate into `HostState`'s `proxy::Host::call`
    // (§6.3), which calls `service_proxy.invoke(req)`, and in a real
    // substrate both builds' `service_proxy` resolves to the *same*
    // `ProxyRouter` (§9.3's hand-off) -- `check_native_capability_gate`
    // itself lives in `ProxyRouter::invoke`
    // (`router/src/proxy.rs:911`), not in `HostState`, so this test is
    // what actually reaches it; §11.1's `StubProxy` never does.
}
```

---

## §12 Ambiguities and staleness in the input documents

Raised rather than guessed, per the brief.

1. **`task.md`'s migration note says `AppHost` grows an `http` inbound
   trait.** *"New traits on `AppHost` for `proxy`, `http` inbound,
   `app-config`, and `vault` (D-06C-10). This is a **breaking change to
   `syneroym-app-host`'s own trait bound**: `AppHost`'s supertrait list
   grows."* An inbound handler is a host→app **export**; `AppHost` is the
   bound an app is generic over for what it **calls**. Putting `HttpSink`
   in the supertrait list would require `GuestHost` (a zero-sized handle to
   the imports) to implement an app's own request handler, which is
   incoherent. **Resolved as `D-C1-4`**: `AppHost` grows four *call*
   traits; inbound HTTP becomes `HttpSink`/`WebSocketSink` beside
   `MessageSink`/`ConversationSink` — the shape B3 already established for
   exactly this direction. `task.md`'s migration bullet should be corrected
   when C1's `status.md` is written.

2. **`task.md` says the migration cost is "exactly two (`GuestHost`,
   `NativeAppHost`) plus one fixture".** The fixture is **generic over
   `H: AppHost`** and implements no `AppHost` sub-trait itself
   ([native.rs:22](../../../../test-components/dual-build-fixture/src/native.rs#L22)),
   so there are exactly **two** implementors, not three. The cost is
   smaller than stated, not larger. Minor, but the sentence is wrong.

3. **`task.md`'s Gap 2 lists `syneroym:http`'s inbound handlers under
   "traits `AppHost` does not have", alongside `proxy`/`app-config`/`vault`.**
   Same conflation as (1). The gap itself is real and correctly described in
   prose ("The WASM build exports it; the native build has no equivalent");
   only its placement in the trait list is wrong.

4. **`syneroym:http/websocket` (the outbound `send`) is named nowhere in
   `task.md`.** Gap 2 names `incoming-handler` / `websocket-handler` — the
   two exports — and stops. But a WebSocket handler that cannot push a frame
   is not a WebSocket surface, and `send` is a host *import* with no guest
   binding module and no native path (F6). C1 includes it (`AppWebSocket`,
   `D-C1-6`); flagging it because it is scope `task.md` did not budget for
   and it carries the `websocket_senders` move.

5. **Gap 2's claim that `AppHost` is
   `AppDataLayer + AppBlobStore + AppMessaging + AppConversation` cites
   `lib.rs:36-43`.** Correct as of `394d0f4` (the trait is at 31-43; the
   blanket impl at 39-43). No staleness — recorded because it was checked.

6. **`task.md`'s open design point *"How a natively linked Roym gets an FDAE
   policy and a `RowAuthorizer`"* frames these as one question.** They are
   two with very different sizes: the policy is a storage read the factory
   can already do (F8), while a `RowAuthorizer` is a second app entry point
   with its own read-only instance semantics and its own timeout. Splitting
   them (`D-C1-8`) is what makes the policy half shippable in C1.

7. **The backlog row for native subscription replay says closing it "needs a
   native replay hook in `syneroym-substrate`'s `runtime.rs`".** That is one
   solution; F12 argues a linked app does not need replay at all, because it
   is linked in at every boot. The row's *target* (C1) is wrong either way —
   there is no linked app with a startup path to prove either shape against.
   **`D-C1-9` retargets it to C2** and records the reasoning; if the
   implementing session disagrees, the alternative is to add a
   `NativeHostFactory::start()` hook in C1 with no consumer, which is
   speculative machinery.

8. **`task.md`'s reference scenario step 1 says "run one integration suite
   against both -- identical results", and exit criterion 1 repeats it.**
   In the tree this is already **two** suites with a deliberate split, stated
   in `dual_build_fixture_e2e.rs`'s own doc comment: the in-process
   `dual_build_parity.rs` proves the shim, and the router-level
   `dual_build_fixture_e2e.rs` proves the registration. C1 keeps that split
   (§11.5). Naming it so nobody reads "one integration suite" as a
   requirement to merge them, and so nobody reads C1's parity suite as
   discharging exit criterion 1 on its own — C2+ still owes a product-level
   suite for the Hub.

9. **Not stale, but worth stating:** `D-06C-10` says the shim grows its
   traits "in C1, before the first product service is written." Nothing in
   this plan writes product code, and `test-components/dual-build-fixture`
   is explicitly not product code. Confirmed against `D-06C-11` too: no
   `C1`, `M06C` or `R1` appears in any name this plan introduces.

10. **`call-target::dependency` has no production wiring for a linked app,
    and nothing in `task.md` names this.** `EndpointRegistry::set_app_context`
    and the binding rows `LogicalResolver::resolve` reads are written only
    by `ControlPlaneService::install_app_context`
    ([orchestration.rs:632](../../../../crates/control_plane/src/service/orchestration.rs#L632))
    on deploy. A linked app has no deploy. §11.1's parity harness and
    §9.2's `init_dual_build_fixture` both seed (or, for the fixture, simply
    never need) this by hand, in-process, which proves the *shim* resolves
    a dependency correctly once bound — it proves nothing about how a real
    linked app **gets** bound. `D-06C-10`'s own stated reason for doing
    `proxy` first is "every Roym service makes calls by dependency name" —
    which makes this exactly the gap that reason implies must be closed
    before C2 ships a second service, and nothing in `task.md`'s C1 row or
    open-design-points list mentions it. Given a backlog row rather than
    folded into C1's own scope (§14) — designing how a linked app's
    `depends_on` gets installed is product-shaped work belonging with the
    first linked app that needs two services talking to each other, not
    with a single-service fixture that only ever calls itself.

11. **The client-gateway hostname leg (ADR-0022 §7) is out of C1's
    scope, and `task.md` does not say so.** Exit criterion 2 ("the UI is
    served from one origin") runs through the client gateway, and the
    obstacle is smaller than a first read of `resolve_target` suggests but
    still real. Two host forms exist
    ([gateway.rs:597-598](../../../../crates/client_gateway/src/gateway.rs#L597)):
    an **app-scoped** one (`-a<app-did-hash>-s<service-name-hash>`), which
    genuinely does resolve through `AppHostResolver::resolve_app_host`
    ([sdk/src/topology.rs:372](../../../../crates/sdk/src/topology.rs#L372))
    against a signed, registered topology document; and an **unscoped**
    one (`s<service-did-hash>`), which the gateway's own comment says
    "needs no resolution" — it only needs a signed `EndpointInfo` published
    to the registry, exactly what
    `guest_http_e2e.rs`'s gateway-port test publishes by hand for a
    deployed WASM service
    ([guest_http_e2e.rs:341](../../../../crates/substrate/tests/guest_http_e2e.rs#L341)),
    independent of `deploy`'s own publication path. So the unscoped form is
    not blocked by missing topology-document machinery the way the
    app-scoped one is. **The actual obstacle is narrower and sharper: the
    fixture's own service id is the literal string `"dual-build-fixture"`,
    not a DID**, so it holds no key an `EndpointInfo` record could be
    signed under — there is nothing to publish, regardless of which host
    form is used. C1's answer to "how does the native build receive an
    inbound HTTP request" (§8) stops at the router's `http-native` hop,
    reachable directly by DID (§9.2's new endpoint registration, §11.5's
    test) but not through either gateway host form. `task.md`'s own "why
    C1 is not folded into C2" argument is precisely that native inbound
    HTTP is risky enough to deserve its own slice rather than being
    discovered inside a slice that also owes a working Hub — the gateway
    leg is exactly that kind of discovery, left for C2 by this plan
    (where a real Roym service, deployed under a real DID, removes the
    obstacle this fixture has no way around), and should be named as a C2
    dependency rather than silently assumed solved.

    **Split 2026-08-27 by
    [ADR-0024](../../../decisions/0024-client-gateway-identity-and-auth-service.md).**
    The gateway leg has two halves and they now land in different slices.
    The **hostname** half — resolving `s<hash>` / `-a…-s…` to a service — is
    unchanged by that ADR and stays C2's, exactly as this item describes.
    The **identity** half — what caller the gateway presents for a request on
    that hostname — moves to slice
    [C1.1](slice-c1.1-implementation-plan.md): the gateway becomes a dumb
    proxy with an `identity_mode`, and the person arrives as a verified
    `syneroym_session` cookie token rather than a preamble delegation the
    gateway minted. C1's own scope is untouched either way — it stops at the
    router's `http-native` hop in both readings.

---

## §13 Order of work

Each step compiles and its tests pass before the next begins.

| # | Step | Gate |
|---|---|---|
| 1 | `wit/proxy/proxy.wit` `+ world proxy-import`; `wit_interfaces` feature `proxy` + `src/proxy.rs`; **required** `http` → `http_host` rename (§3.5) | `cargo build -p syneroym-wit-interfaces --all-features`; `cargo build --workspace` |
| 2 | `wit/http/http.wit` `+ world websocket-import`; `wit_interfaces` feature `http` + `src/http_guest.rs` | same. **If this fails to encode, stop and take `§4.4`'s fallback before going further** |
| 3 | Move `core::guest_http` → `app_host::types::http`; update the 5 files in §7.1 | `cargo test -p syneroym-sandbox-wasm -p syneroym-router` |
| 4 | `app_host`: new features, `types::{proxy, app_config, vault, http}`, four new call traits (`AppProxy`, `AppAppConfig`, `AppVault`, `AppWebSocket`), `AppHost` bound. **No sinks here** — `HttpSink`/`WebSocketSink` are step 8's, in `app_host_native` | `cargo build -p syneroym-app-host`; `cargo build -p syneroym-app-host --target wasm32-wasip2` |
| 5 | `app_host/src/guest.rs`: four new impls | wasm32 build |
| 6 | `rpc`: `NativeHttpService`/`NativeHttpRegistry`, `WebSocketSenders` | `cargo build -p syneroym-rpc` |
| 7 | `sandbox_wasm`: `websocket_senders` moves out into `Arc<syneroym_rpc::WebSocketSenders>`; `HostState::with_websocket_senders`; `AppSandboxEngine.websocket_senders` becomes a `OnceLock` set post-construction (**no signature change**, no call-site edits beyond `runtime.rs` and the parity harness) | `cargo test -p syneroym-sandbox-wasm`; `cargo test -p syneroym-substrate` |
| 8 | `app_host_native`: lazy `HostState`, factory fields/setters, four trait impls, `convert.rs`, `NativeHttpAdapter` | `cargo test -p syneroym-app-host-native` (existing suite must still be green **before** any new scenario is added) |
| 9 | `router`: `native_http` field, the two new branches | `cargo test -p syneroym-router`; `cargo test -p syneroym-substrate --test guest_http_e2e` |
| 10 | `substrate`: `SharedNodeHandles`, `init_dual_build_fixture`, the proxy hand-off | `cargo build -p syneroym-substrate --all-features` |
| 11 | Fixture: WIT deps, world, `guest.rs`, `native.rs`, `app.rs` | `mise run build:test-components` |
| 12 | Parity suite: harness, drivers, scenarios, named tests | `cargo test -p syneroym-app-host-native` |
| 13 | Router-level test: extend `crates/substrate/tests/dual_build_fixture_e2e.rs` (§11.5) | `cargo test -p syneroym-substrate --features dual_build_fixture --test dual_build_fixture_e2e` |
| 14 | Docs and backlog (§14) | — |
| 15 | Full gate | `cargo +nightly fmt --all`, `cargo clippy --workspace --all-targets --all-features`, `cargo test --workspace`, `cargo audit`, `cargo deny check licenses`, `mise run test:e2e` |

**Step 3 is the riskiest cheap step and step 7 the riskiest expensive one.**
Do 3 as its own commit so a revert is clean.

---

## §14 Documents and backlog owed

Per `task.md`'s "Owed as slices land" table:

| Document | Edit |
|---|---|
| `docs/planning/milestones/M06C-roym-product/status.md` | **Created with C1 and not before.** Records: the eight interfaces as shipped; every permitted difference this plan names (no guest-HTTP admission semaphore natively, no `NoHandler`/`Unavailable` taxonomy, no WebSocket permit, no `RowAuthorizer`, FDAE stage-4 fails closed); the `HttpSink`/`WebSocketSink` location decision (§4.3/§6.5 — defined in `syneroym-app-host-native`, taking `CallerContext` directly, not in `syneroym-app-host`); that the `http` → `http_host` rename (§3.5, required) landed; and that the in-process parity harness now runs with encryption on and a KEK injected on both stacks (§11.1) |
| [deferred-backlog.md](../../deferred-backlog.md) §5, line 72 (**the master tracking row**) | *"The dual-build shim … covers only `data-layer`/`blob-store`/`messaging`"* — this is the row C1 exists to close, named nowhere else in this plan's own §14 draft until this pass. Update it to record what shipped: `http` (both directions), `app-config`, `vault`, `proxy` (`call`/`enqueue`) are now covered; only `syneroym:proxy/saga` remains, which gets its own row below (D-C1-10). Move the bulk of this row to "Recently resolved" and leave a narrower one for `saga` alone |
| [deferred-backlog.md](../../deferred-backlog.md) §5 | **Native FDAE policy row**: split — the policy half moves to "Recently resolved" with what shipped; the `RowAuthorizer` half stays open, retargeted `TBD`, with the pickup trigger *"a natively linked app needs a stage-4 ABAC after-step"* (D-C1-8). **Native subscription replay row**: retargeted `M06C C1` → `M06C C2`, with F12's reasoning added (D-C1-9). **`read_only` never exercised by the parity suite row**: unchanged — C1 does not close it |
| [deferred-backlog.md](../../deferred-backlog.md) §5, new row | **`syneroym:proxy/saga` has no native shim path** (D-C1-10). Target `TBD`, trigger *"a service drives a multi-service workflow"*, source `crates/app_host/src/lib.rs` (`AppProxy`) |
| [deferred-backlog.md](../../deferred-backlog.md) §5, new row | **A natively linked app has no per-service guest-HTTP admission control** — the WASM build bounds concurrent `handle-request` calls per service (`GUEST_HTTP_ADMISSION_TIMEOUT`, `max_concurrent_guest_http_per_service`); the native path has no equivalent and relies on the app to shed load itself. Target `TBD`, trigger *"a linked app is observed saturating the node"* |
| [deferred-backlog.md](../../deferred-backlog.md), new row | **A natively linked app has no `depends_on`/app-context binding in a real deployment** — `EndpointRegistry::set_app_context` and the binding rows `call-target::dependency` resolves against are written only by `ControlPlaneService::install_app_context` on deploy ([orchestration.rs:632](../../../../crates/control_plane/src/service/orchestration.rs#L632)); a linked app has no deploy, so in production `app_context_of` returns `None` and every dependency-named proxy call fails `DependencyNotBound`. C1's parity harness (§11.1) and `init_dual_build_fixture` (§9.2) do not seed this either — the fixture only ever calls itself. `D-06C-10`'s own reasoning for doing `proxy` first is that "every Roym service makes calls by dependency name" (Gap 2); this row is why C2 hits that on day one and needs its own answer, not an assumption that C1 already solved it. Target `M06C C2`, source `crates/control_plane/src/service/orchestration.rs` (`install_app_context`); `crates/app_host_native/src/factory.rs` |
| [task.md](./task.md) | Gap 2's trait list corrected per §12 (1)/(3): `http` inbound is a sink trait, not an `AppHost` supertrait. The "exactly two plus one fixture" count corrected to two |
| [CLAUDE.md](../../../../CLAUDE.md) / [AGENTS.md](../../../../AGENTS.md) | **No edit owed by C1.** The architecture section enumerates WIT interface names (`host`, `data-layer`, `blob-store`, `app-config`, `control-plane`, `vault`); C1 adds no new WIT *package*, only new worlds inside existing ones. C3 owes that edit, for the signing package |

---

## §15 What "done" means for C1

1. `syneroym-app-host::AppHost` bounds eight traits, and both implementors
   satisfy all eight.
2. `test-components/dual-build-fixture` builds as a `wasm32-wasip2`
   component **and** links into `syneroym-substrate` under the
   `dual_build_fixture` feature, exercising all eight.
3. `crates/app_host_native/tests/dual_build_parity.rs` compares both builds
   across every scenario in §11.3 and every named test in §11.4, and
   `the_parity_comparison_detects_a_divergence` still fails the mutant.
4. An inbound HTTP request routed by `syneroym-router` reaches a natively
   linked app and a WASM one through the same route table, the same caller
   extraction, the same 401 rule, and the same response validation — with
   only the last hop differing.
5. A natively linked app can make a `proxy::call` to a sibling by declared
   dependency name (§11.3/§11.4, in-process) and is refused by the same
   `check_native_capability_gate` a WASM component is when it addresses a
   different service by DID (§11.5's
   `a_cross_service_call_by_did_is_gated_identically_on_both_builds`, over
   the real router — the in-process parity suite's `StubProxy` does not
   exercise this gate at all, and no criterion here is claimed on its
   behalf).
6. A natively linked app running under an FDAE policy is subject to it, and
   a policy carrying ABAC permissions fails closed with a test to prove it.
7. Every permitted difference between the builds is named in `status.md` and
   asserted in `permitted_differences`, not left latent.
8. `cargo +nightly fmt --all`, `cargo clippy --workspace --all-targets
   --all-features`, `cargo test --workspace`, `cargo audit`,
   `cargo deny check licenses` and `mise run test:e2e` are clean.
9. No planning identifier appears in any name this slice introduces
   (`D-06C-11`), checked by grep.
10. The backlog edits in §14 are made, and `status.md` exists.
