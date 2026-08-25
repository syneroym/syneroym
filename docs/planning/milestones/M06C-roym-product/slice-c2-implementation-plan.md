# M06C Slice C2 — The SynApp Skeleton and the Hub Shell: Implementation Plan

> **Scope, from [task.md](task.md)'s slice table.** One `SynAppManifest` with
> six services (`web`, `conversation`, `profile`, `catalog`, `transaction`,
> `directory`); sibling wiring by `depends_on` + `call-target::dependency`;
> `topology_visibility` / `visibility` declared so a foreign caller can
> resolve what it should; the Web entrypoint as an ordinary WASM component
> (`D-06B-1`) serving the UI bundle and forwarding JSON-RPC from one origin,
> with no business logic in it; the Hub shell — a person logs in through the
> gateway session and the guest sees *them* — plus the card renderer's fixed
> templates and its unknown-type fallback (`D-06C-3`).
>
> **Depends on:** C1 (Complete 2026-08-25).
> **Blocks:** C4 and C5 (both need the skeleton to hang product code on).
> **Runs in parallel with:** C3 — the two do not read each other
> (`task.md`, "Dependency shape").
>
> **This plan writes no product behaviour.** Every service it creates is a
> skeleton: one WIT interface, a request router with no verbs of its own,
> and its own storage namespace. Records, listings, conversations, cards'
> *contents*, search — all of that is C3–C10. What C2 owes is that the six
> services exist, are addressable both ways, and that a request from a
> logged-in person's browser reaches one of them and comes back.

---

## §0 What C1 handed C2, and what is still missing

C1 closed Gap 2. `AppHost` now bounds eight traits
([lib.rs:36-60](../../../../crates/app_host/src/lib.rs#L36)) — `AppDataLayer`,
`AppBlobStore`, `AppMessaging`, `AppConversation`, `AppProxy`,
`AppAppConfig`, `AppVault`, `AppWebSocket` — and inbound HTTP arrives as the
sink traits `HttpSink` / `WebSocketSink`
([app_host_native/src/http.rs](../../../../crates/app_host_native/src/http.rs)),
reached by the router through `NativeHttpRegistry`
([rpc/src/native_http.rs](../../../../crates/rpc/src/native_http.rs)).
`test-components/dual-build-fixture` proves all eight through both builds.

Five things C2 needs are **not** in the tree, and each is a section below:

| # | Missing | Section |
|---|---|---|
| 1 | Any product crate at all (`task.md` Gap 8) | §3–§7 |
| 2 | A checked-in `SynAppManifest` — the tree has none; every existing manifest is built in Rust inside a test | §9 |
| 3 | A way for a **natively linked** app to get an app context and dependency bindings, so `call-target::dependency` resolves (backlog row, targeted `M06C C2`; C1 §12 (10)) | §11.3 |
| 4 | A browser login flow for a person session (backlog §7 row, targeted `M06C C2`) | §10 |
| 5 | The client-gateway hostname leg for the app's own front door — C1 §12 (11) left it explicitly to C2 | §9.4, §12.4 |

---

## §1 Findings from reading the tree

Verified 2026-08-25 against `6a6aa99`. Each is load-bearing for a decision
in §2.

### F1 — a proxied call to a **WASM** callee loses the caller; to a **native** callee it does not

`ProxyRouter::invoke_local` passes `req.caller.clone()` into
`NativeInvocation` for a `NativeHostChannel` target
([proxy.rs:673-695](../../../../crates/router/src/proxy.rs#L673)), but passes
`None` for a `WasmChannel` target
([proxy.rs:716-736](../../../../crates/router/src/proxy.rs#L716)), and
`prepare_wasm_execution` then substitutes
`CallerContext::service_system(service_id)` — the **callee's own** id
([engine.rs:1372](../../../../crates/sandbox_wasm/src/engine.rs#L1372)).

So for the identical sibling call:

| Build | What the callee's `CallerContext.caller_did` is |
|---|---|
| WASM | `system:<callee's own service id>` |
| Native | `system:<caller's service id>` |

Both are `AuthLevel::System`. Neither carries the person. This is a real
dual-build divergence in a code path *every* Roym service uses, and it is
the single most important finding in this plan. It is **not** a shim bug —
both sides go through `HostState`, which is shared; the divergence is in
`ProxyRouter`, above the shim. §2 `D-C2-4` decides what Roym does about it,
and §15 records the backlog row.

### F2 — a guest's own outbound proxy call never forwards its caller either

`host_capabilities.rs`'s `call` sets
`caller = if target_service == self.component_id { self.caller } else { CallerContext::service_system(&self.component_id) }`
([host_capabilities.rs:1350-1356](../../../../crates/sandbox_wasm/src/host_capabilities.rs#L1350)),
with the comment *"Real cross-service caller-delegation is B1/UCAN, not yet
built."* Identical on both builds (one `HostState`). Therefore **the Web
entrypoint cannot forward the person's identity through the proxy's own
identity channel**, on either build. If a sibling is to know who is asking,
the app must carry it in the payload. This is what makes `D-C2-4`
unavoidable rather than a preference.

### F3 — `public: true` on an HTTP route also produces an `AuthLevel::System` caller

`HttpRoute.public`'s own doc says so
([http_routes.rs:34-52](../../../../crates/core/src/http_routes.rs#L34)):
with no caller to substitute, the handler runs as
`CallerContext::service_system`, `AuthLevel::System`. So "the caller is
`System`" is only a sound proxy for "this call came from a sibling in my own
app" if the service declares **no** `http_routes` at all. Only `web` may
declare routes (§9.2), and that is a checkable invariant, not a convention.

### F4 — static assets are served by the host, never by the app, on either build

`try_handle_asset` reads `RouteHandlerInner.assets` (an `AssetRegistry`
keyed by `service_id`) and streams the blob
([http.rs:829](../../../../crates/router/src/route_handler/http.rs#L829)),
placed **before** route resolution and without instantiating anything
(M06A `D-06A-1`). `unpack_asset_bundle` is the only reader of the archive
([control_plane/src/assets.rs:62](../../../../crates/control_plane/src/assets.rs#L62)),
takes a gzip tar plus a `BlobProvider`, and returns an `AssetManifest`.

Consequence: **the UI bundle is not part of either build of the app.** It
never crosses the WIT boundary. The parity suite therefore owes it nothing,
and the native wiring can register the same bundle by calling
`unpack_asset_bundle` itself at startup (§11.4). This removes what looked
like C2's hardest dual-build problem.

### F5 — `AssetRegistry` and the `StaticInventory` are not on `SharedNodeHandles`

`build_route_handler_deps` creates `assets`
([runtime.rs:1271](../../../../crates/substrate/src/runtime.rs#L1271)) and
`app_registry`
([runtime.rs:1223](../../../../crates/substrate/src/runtime.rs#L1223)) as
locals, moves `assets` into `RouteHandlerDeps` and `app_registry` into the
`LogicalResolver`, and neither reaches `SharedNodeHandles`
([runtime.rs:837-873](../../../../crates/substrate/src/runtime.rs#L837)).
The native wiring needs both. Two new fields, same shape as the five
`dual_build_fixture` already added.

### F6 — the endpoint registry is authoritative for local dispatch, published or not

`invoke_local` is chosen from `EndpointRegistry`, and its doc calls the
registry *"authoritative for services hosted on this node"*
([proxy.rs:662](../../../../crates/router/src/proxy.rs#L662)). So a service
declaring `visibility = private` (no endpoint record published anywhere) is
still reachable by its siblings on the same node. `visibility` governs
publication and therefore *cross-node* reach only. This is what lets
`profile` stay `private` (§9.3) while `web` still calls it.

### F7 — `visibility = public`/`internal` requires a registry certificate at deploy

[ADR-0018:229](../../../decisions/0018-service-record-visibility.md) —
*"`visibility` is `public`/`internal` but no certificate → fail the deploy"*,
and `internal` means *"registered with this substrate's registry only"*
(line 209). Five of Roym's six services need one (§9.3), so the WASM deploy
**must** run `roymctl app deploy --mint-masters --registry-url …`. A deploy
without those flags fails, and that is the correct failure, not a hurdle to
route around.

### F8 — the client gateway's unscoped host form needs only a published endpoint record

`resolve_target`'s unscoped arm returns `(lookup_alias, interface)` with no
resolution at all
([gateway.rs:605](../../../../crates/client_gateway/src/gateway.rs#L605));
`guest_http_e2e.rs:341` reaches a deployed WASM service through
`s<short_hash(service_did)>.localhost:<gw port>` after publishing an
`EndpointInfo` by hand. The app-scoped `-a…-s…` form additionally needs
`AppHostResolver` against a signed topology document plus either
`supervisor/resolve` or `topology_visibility = open`. C1 §12 (11) said the
fixture could use neither because its service id is the literal string
`"dual-build-fixture"`. Roym's `web` service has a real minted DID, so the
**unscoped form works with no new machinery** — this is the answer to the
question C1 left open.

### F9 — person login needs three primitives the browser should not have

`SessionStore::login` requires (1) an Ed25519 signature over
`gateway_session_assertion(node_did, nonce, person_did)` in RFC-8785
canonical JSON, z-base-32 encoded; (2) a `DelegationCertificate` from the
person to *this* node with `scope = "routing"`; (3) the person's master
anchor already resolvable in the registry, else 409
([session.rs:160-240](../../../../crates/client_gateway/src/session.rs#L160)).
`derive_did_key` is `did:key:h` + z32 of `0xed01 || pubkey`
([substrate.rs:142](../../../../crates/identity/src/substrate.rs#L142)).

The spec's client contract states flatly: *"The UI holds no user private
key."* So the browser cannot be the signer. `D-C2-6` follows.

### F10 — no checked-in `SynAppManifest` exists, and `source` resolves against the client's cwd

Every manifest in the tree is built in Rust inside a test. The two path
rules differ and are documented as differing: `ServiceConfig.source`
resolves against the **client process's working directory**
([models.rs:468-488](../../../../crates/app_orchestration/src/models.rs#L468)),
while `AssetBundle.archive` resolves against the **manifest's own
directory** for `supervisor submit`
([models.rs:589-596](../../../../crates/app_orchestration/src/models.rs#L589)).
C2 introduces the tree's first checked-in manifest and must live with both
rules rather than pick one.

### F11 — the fixture is the working template for a dual-build crate

`test-components/dual-build-fixture` is a **workspace member** with
`crate-type = ["cdylib", "rlib"]`, target-gated dependencies
(`wit-bindgen` + `syneroym-wit-interfaces` on `wasm32`; `syneroym-rpc` +
`syneroym-app-host-native` + `async-trait` elsewhere), `wit/deps/*/*.wit`
symlinked back to `crates/wit_interfaces/wit/`, and a three-file split:
`app.rs` (target-independent behaviour), `guest.rs` (`generate!` + exports),
`native.rs` (generic `NativeFixture<H: AppHost>` implementing
`NativeService` / `MessageSink` / `ConversationSink` / `HttpSink` /
`WebSocketSink`). Every Roym service crate copies this shape exactly. Note
C1 `F9`: the `syneroym-wit-interfaces` dependency must enable **only** the
features that crate's own `with:` remap references, or the component links
worlds it never uses.

### F12 — `SynAppManifest::validate` detects dependency cycles

[models.rs:718-772](../../../../crates/app_orchestration/src/models.rs#L718).
`web → {conversation, profile, catalog, transaction, directory}` with no
back-edges is legal. A sibling must never declare `depends_on = ["web"]`.

### F13 — a WASM service's callable interfaces come from the manifest, not the component

`register_wasm_endpoints` registers one `WasmChannel` endpoint per name in
`ServiceConfig.interfaces`
([orchestration.rs:606-620](../../../../crates/control_plane/src/service/orchestration.rs#L606)).
So each Roym service must list its own WIT interface name in the manifest's
`interfaces` array, and `proxy.call`'s `interface` argument must match it
byte for byte. One shared constant per service, in `roym-core`, is what
keeps the manifest, the caller, and the native registration from drifting.

### F14 — `HealthCheck::Rpc` is available and free

`RpcProbe { interface, method, timeout_ms }`, valid for `wasm` services, and
*"any non-error return is a pass"*
([models.rs:395-431](../../../../crates/app_orchestration/src/models.rs#L395)).
If every service exports a nullary `status` function, every service gets a
readiness probe for one manifest line.

---

## §2 Decisions

| # | Decision | Why |
|---|---|---|
| **D-C2-1** | **Seven crates: `crates/roym_core/` (`syneroym-roym-core`) plus one per service — `roym_web`, `roym_conversation`, `roym_profile`, `roym_catalog`, `roym_transaction`, `roym_directory`, packaged `syneroym-roym-<name>`.** All are ordinary workspace members (`crates/*` is already a glob), built as components with `cargo component build --target wasm32-wasip2 -p <pkg>` exactly as the fixture is. | `D-06C-9` states the rule and the naming, including that the `syneroym-roym-` prefix is a known cost. Per-service crates because each is separately deployed and separately certified. `roym_core` is `D-06C-9`'s "one crate holds the shared record, card, and dual-build wiring". |
| **D-C2-2** | **Each service exports exactly one WIT interface with exactly two functions: `invoke: func(request: string) -> result<string, string>` and `status: func() -> result<string, string>`.** Package `syneroym-roym:<service>@0.1.0`, interface `api`. `request` is a JSON document; the JSON-RPC method name lives *inside* it, not in the WIT. | The alternative — one WIT function per product method — freezes signatures C4–C10 have not designed yet, and makes every new method a WIT change plus a redeploy of the interface list (F13). The string-in/string-out shape is the one the dual-build fixture already proved through both builds. The cost is real and named: **the sibling boundary is not typed by WIT**, so a malformed inner call is an application error, not a dispatch error. Backlog row in §15, trigger *"a sibling boundary needs typed WIT parameters"*. `status` exists so F14's probe costs one manifest line. |
| **D-C2-3** | **The Web entrypoint holds a static routing table from JSON-RPC method prefix to (dependency name, interface constant), and nothing else.** No defaulting, no wildcard, no fallthrough: an unmatched prefix is JSON-RPC `-32601`. The table lives in `roym_core` and a unit test asserts it is total over the declared prefix set and unambiguous. | The spec's rule 4 — *"The Web entrypoint holds no business logic. It serves files and forwards calls."* A table with a default arm is a decision; a table without one is a lookup. Putting it in `roym_core` rather than `roym_web` is what lets the test compare it against the manifest's `depends_on` list. |
| **D-C2-4** | **The person's identity travels to a sibling as an explicit `caller` envelope inside `invoke`'s `request` JSON, and a sibling accepts that envelope only when its own `CallerContext.auth == System`. Under any other auth level the envelope is ignored and the request is attributed to the wire caller's own DID.** The envelope carries `person_did`, `auth` (the `caller-auth` the entrypoint itself observed), and `origin` (the entrypoint's own service id, for logging only — see F1). | F2 makes this unavoidable: neither build forwards a caller through the proxy, so if the envelope is not carried, no Roym service except `web` can ever know who is asking, and exit criterion 3 is unmeetable. The `System` gate is sound because `CallerContext::service_system` is substrate-injected, carries `proof: None`, and therefore cannot cross a wire hop or be constructed from wire input — **provided** the service declares no `public: true` HTTP route (F3), which `D-C2-5` makes an invariant. The gate reads identically on both builds even though F1's divergence means the two see different `caller_did` values, because it tests `auth`, not `caller_did`. |
| **D-C2-5** | **Only `web` declares `http_routes`, and only `web` declares an `assets` bundle. A test asserts the other five declare neither.** | Two independent reasons that happen to agree: it is what "one origin" means (spec, Client contract), and it is what makes `D-C2-4`'s `System` gate sound (F3). Written as one invariant with one test so neither reason can be relaxed without noticing the other. |
| **D-C2-6** | **Browser login is a gateway-side local ceremony for the first release, structured so a browser-held WebAuthn/passkey signer can be added later as a second `SessionRoute` without changing the session wire format.** A new reserved endpoint `POST /_syneroym/session/login-local` takes `{"identity": "<name>"}`, loads that person key from a newly configured `roles.client_gateway.person_identities_dir`, refreshes the master anchor, runs the existing challenge/assert/delegate/login steps in-process, and returns the same `LoginResponse` + `Set-Cookie` the remote path already returns. `GET /_syneroym/session/identities` lists the available names and **no key material**. Absent config disables both with 404. | F9: the spec forbids the UI holding a private key, so the browser cannot be the signer for the first release, and the three primitives (z-base-32, RFC-8785, `did:key:h`) would otherwise have to be reimplemented in TypeScript and kept byte-compatible with Rust forever. The gateway binds `127.0.0.1` and D8 puts the browser on the same machine, so this adds no exposure that did not already exist: any local process that can call this endpoint can already read the same key file and run `roymctl session login`. That equivalence is the whole security argument and is stated in the endpoint's own doc comment. **Deferred, not rejected: a browser-held WebAuthn/passkey signer.** Confirmed addable later without a breaking change here — `LoginResponse`, the cookie, and `whoami` are unchanged either way; only the *login initiation* half (a third `SessionRoute`, e.g. `WebauthnLogin`, verifying an authenticator assertion instead of a bare ed25519 signature) would be new. It needs three things C2 does not build and must not accidentally foreclose: (1) **CORS on the community registry** (`crates/community_registry/src/registry.rs` — no CORS layer exists on any of its four Axum routes today), because a browser holding its own key publishes its own anchor directly to the registry rather than through the gateway, which is a cross-origin call the gateway-side ceremony never makes; (2) a **did:key scheme extension**, since `derive_did_key`/`resolve_did_key` hardcode the ed25519 multicodec (`0xed01`, [substrate.rs:142-166](../../../../crates/identity/src/substrate.rs#L142)) and reject anything else — a WebAuthn credential using ES256 (P-256, COSE alg `-7`) needs a second multicodec branch (P-256 is `0x1200`) threaded through `resolve_did_key` and `verify_json_signature`; an authenticator using EdDSA (COSE alg `-8`) needs none, so this is a per-authenticator gap, not a universal one; (3) a **WebAuthn assertion verifier** (challenge, `clientDataJSON`, `authenticatorData`, signature) — a different verification shape than `DelegationCertificate::verify`'s single z32 signature, so it is new code, not a reuse. None of this is C2's to build; C2's job is only to not paint it into a corner, which the two-`SessionRoute` shape (§10) achieves. Backlog rows for both gaps in §15. |
| **D-C2-7** | **Identity *creation* stays in C4. C2's login endpoint operates only on an identity that already exists on disk.** When the directory is empty the Hub says so and names `roymctl identity create`. | R1 row 1 (device-bound identity, encrypted backup, import) is C4's, by `task.md`'s own slice table. C2 owes the *ceremony*, which is what the backlog row targeted at C2 actually asks for. |
| **D-C2-8** | **The card renderer ships in the UI bundle as data-driven templates, and the canonical `(type, version)` set lives in `roym_core`. A unit test in `roym_core` reads the UI's own registry file and fails if the two lists differ.** | `D-06C-3` fixes seven types plus a fallback and requires the rule to be decided once. The producer is Rust (C7/C8) and the consumer is TypeScript, so "decided once" needs a mechanism, not a promise. Reading the TS file from a Rust test is ugly and cheap; a code generator is neither. |
| **D-C2-9** | **The native build registers the UI bundle by calling `unpack_asset_bundle` at startup against a path from config (`roles.roym.ui_bundle_path`), not by `include_bytes!`.** Absent path = no assets served natively; the JSON-RPC API is unaffected and the parity suite still passes. | F4: assets never cross the WIT boundary, so this is host wiring, not app code. `include_bytes!` would make `cargo build -p syneroym-substrate --features roym` depend on npm having run, which would break `cargo build --workspace` for everyone. |
| **D-C2-10** | **The native build's app context and dependency bindings are installed by the wiring function itself**, writing `EndpointRegistry::set_app_context` + `save_binding` and registering `TopologyKey::local(instance, name) -> TopologyEntry` into the same `StaticInventory` the node's `LogicalResolver` holds. This closes the backlog row targeted at `M06C C2`. | C1 §12 (10): `install_app_context` runs only on deploy, and a linked app has no deploy, so in production `app_context_of` returns `None` and every dependency-named proxy call fails `DependencyNotBound`. The wiring function is the linked app's deploy. Doing it anywhere else means a second place that knows Roym's service topology. |
| **D-C2-11** | **The native build installs no instance certificates.** Sibling calls are local and do not need one; `enqueue` and cross-node `call` from the native build are refused, named as a permitted difference, and given a backlog row. | The certificate path (`roymctl identity certify-instance`, master minting, anchor publication) is a deploy-time ceremony with no in-process equivalent, and nothing in C2 needs it — the six services all live on one node in the native build. C9 is the slice that stands up three installations, and it uses the WASM build for them. Naming it beats discovering it in C5. |
| **D-C2-12** | **Two suites, matching C1's split (C1 §12 (8)).** `crates/roym_web/tests/dual_build_parity.rs` drives the whole app through the entrypoint on both builds, in process, with a test `ServiceProxy` that dispatches sibling calls into whichever build is under test. `crates/substrate/tests/roym_app_e2e.rs` proves registration, the gateway hostname leg, the person session, and the routes — router-level, WASM build only. | Duplicating the router in the parity suite would test the router twice and the app once. The gateway leg cannot be proven in process at all. |
| **D-C2-13** | **`roym_directory` is a full sibling in the manifest and in `depends_on`, even though a consumer reaches a *foreign* directory by DID.** `web → directory` by `call-target::dependency` is how the SynOrg owner administers their own directory; a consumer's chosen directories are `call-target::service(<did>)` at runtime. C2's suites exercise **both** shapes. | The spec's own service-boundaries section says Roym needs both, and `D-06C-10`'s stated reason for doing `proxy` first was that every Roym service makes dependency calls. If C2 exercised only one shape, the other would first be exercised in C6, on a foreign node, which is the worst place to find a defect in it. `D-06C-6a` (the Directory is optional) is untouched: nothing in C2 makes any flow require it. |

---

## §3 Crate layout and workspace wiring

### 3.1 New directories

```
crates/roym_core/            syneroym-roym-core
crates/roym_web/             syneroym-roym-web
crates/roym_conversation/    syneroym-roym-conversation
crates/roym_profile/         syneroym-roym-profile
crates/roym_catalog/         syneroym-roym-catalog
crates/roym_transaction/     syneroym-roym-transaction
crates/roym_directory/       syneroym-roym-directory
```

`crates/*` is already a workspace member glob (root `Cargo.toml:3`), so no
`members` edit is needed. Do **not** add any of them to `exclude`: like the
dual-build fixture, they must be linkable into `syneroym-substrate`, so
their host-target builds belong in the shared `target/`.

### 3.2 Root `Cargo.toml` — `[workspace.dependencies]`

Append, after the existing `syneroym-*` entries:

```toml
syneroym-roym-core = { path = "crates/roym_core" }
syneroym-roym-web = { path = "crates/roym_web" }
syneroym-roym-conversation = { path = "crates/roym_conversation" }
syneroym-roym-profile = { path = "crates/roym_profile" }
syneroym-roym-catalog = { path = "crates/roym_catalog" }
syneroym-roym-transaction = { path = "crates/roym_transaction" }
syneroym-roym-directory = { path = "crates/roym_directory" }
```

### 3.3 Per-service `Cargo.toml` — the exact shape

Copy `test-components/dual-build-fixture/Cargo.toml` (F11) and change three
things: the package name, the `wit_interfaces` feature list (enable **only**
what this crate's `with:` remap names — C1 `F9`), and add
`syneroym-roym-core.workspace = true`. For example,
`crates/roym_profile/Cargo.toml`:

```toml
[package]
name = "syneroym-roym-profile"
version.workspace = true
edition.workspace = true
license.workspace = true

[lints]
workspace = true

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
syneroym-app-host.workspace = true
syneroym-roym-core.workspace = true
serde = { workspace = true, features = ["derive"] }
serde_json.workspace = true

[target.'cfg(target_arch = "wasm32")'.dependencies]
wit-bindgen.workspace = true
syneroym-wit-interfaces = { path = "../wit_interfaces", default-features = false, features = [
    "data-layer",
] }

[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
syneroym-rpc.workspace = true
syneroym-app-host-native.workspace = true
async-trait.workspace = true

[package.metadata.component.target.dependencies]
"syneroym:data-layer" = { path = "wit/deps/data-layer" }
```

Per-service feature lists for C2 (each service imports only what its
skeleton actually touches; C4–C10 widen these as they add behaviour):

| Crate | `wit_interfaces` features | `wit/deps` symlinks |
|---|---|---|
| `roym_web` | `http`, `proxy` | `http`, `proxy` |
| `roym_conversation` | `data-layer`, `conversation` | `data-layer`, `conversation` |
| `roym_profile` | `data-layer` | `data-layer` |
| `roym_catalog` | `data-layer` | `data-layer` |
| `roym_transaction` | `data-layer` | `data-layer` |
| `roym_directory` | `data-layer` | `data-layer` |

`roym_core` is target-independent: `serde`, `serde_json`, and
`syneroym-app-host` (for the trait bounds and the `types::http` vocabulary)
only. It declares **no** `wit-bindgen`, no `[lib] crate-type`, and no
`[package.metadata.component]` — it is never a component itself.

### 3.4 `wit/deps` symlinks

For each service crate, per the table above, exactly as the fixture does
(C1 `D-C1-10.2`, verified: they are symlinks, not copies):

```bash
mkdir -p crates/roym_web/wit/deps/http crates/roym_web/wit/deps/proxy
ln -s ../../../../wit_interfaces/wit/http/http.wit   crates/roym_web/wit/deps/http/http.wit
ln -s ../../../../wit_interfaces/wit/proxy/proxy.wit crates/roym_web/wit/deps/proxy/proxy.wit
```

Note the depth differs from the fixture's (`crates/roym_web/wit/deps/http/`
is four levels below `crates/`, where `test-components/dual-build-fixture/`
was five). Verify each link resolves before committing:
`for f in $(find crates/roym_* -type l); do test -e "$f" || echo "BROKEN $f"; done`.

### 3.5 `mise.toml` — a new build task

```toml
[tasks."build:roym-ui"]
description = "Build the Roym Hub UI bundle and pack it for the asset bundle"
run = """
(cd crates/roym_web/ui && (npm ci || npm install) && npm run build)
tar -czf crates/roym_web/ui/bundle.tar.gz -C crates/roym_web/ui/dist .
"""

[tasks."build:roym"]
description = "Build every Roym service as a wasm32-wasip2 component"
depends = ["build:roym-ui"]
run = """
for pkg in web conversation profile catalog transaction directory; do
  cargo component build --release --target wasm32-wasip2 -p "syneroym-roym-$pkg"
done
"""
```

Add `"build:roym"` to `test:rust`'s `depends` list beside
`"build:test-components"`. Add `crates/roym_web/ui/node_modules/`,
`crates/roym_web/ui/dist/`, and `crates/roym_web/ui/bundle.tar.gz` to
`.gitignore` — the bundle is a build product, and the deploy step builds it.

---

## §4 The WIT surface

### 4.1 One file per service crate: `crates/roym_<name>/wit/world.wit`

Shown for `profile`; the other four siblings are identical with the package
name changed.

```wit
package syneroym-roym:profile@0.1.0;

/// One service of the Roym SynApp. The JSON-RPC method name and its
/// parameters travel inside `request`, not in this signature: the method
/// set grows with the product, and a WIT function per method would make
/// every added verb a change to the deployed interface list.
///
/// `request` is a JSON object: `{ "method": string, "params": any,
/// "caller": { "person-did": string|null, "auth": string, "origin":
/// string } }`. The success value is a JSON object `{ "result": any }`;
/// an application-level refusal is `{ "error": { "code": number,
/// "message": string } }` inside the success arm, so the `Err` arm below
/// carries only faults this service could not describe -- unparseable
/// request JSON, and nothing else.
interface api {
    invoke: func(request: string) -> result<string, string>;

    /// Liveness plus the service's own schema version, as a JSON object.
    /// Any non-error return is a readiness pass; the manifest points its
    /// `health_check` here.
    status: func() -> result<string, string>;
}

world profile {
    import syneroym:data-layer/store@0.1.0;
    export api;
}
```

`roym_web`'s world differs:

```wit
package syneroym-roym:web@0.1.0;

interface api {
    invoke: func(request: string) -> result<string, string>;
    status: func() -> result<string, string>;
}

world web {
    import syneroym:proxy/proxy@0.1.0;
    import syneroym:http/websocket@0.1.0;

    export syneroym:http/incoming-handler@0.1.0;
    export syneroym:http/websocket-handler@0.1.0;
    export api;
}
```

`web` exports `api` too, so the parity suite can drive it without an HTTP
stack — the same reason the fixture exports `test-driver`. Note that there
is **no `roymctl svc call` verb** in the tree (`SvcCommands` is `Deploy`,
`Remove`, `Restart`, `ProxyOutbox`, `ProxyDeadLetters`, `ProxyReplay`,
`Sagas`, `SagaCompensate`, `EndpointInfo`), so this export is not reachable
from the CLI today — see §14 (10).

### 4.2 Interface-name constants (F13)

`crates/roym_core/src/services.rs`:

```rust
/// The logical service names this app declares, and the WIT interface each
/// one answers on. The manifest's `interfaces` array, `proxy.call`'s
/// `interface` argument, and the native registration all read these -- one
/// definition, so a rename cannot land in two of the three.
pub const WEB: Service = Service {
    name: "web",
    interface: "syneroym-roym:web/api@0.1.0",
};
pub const CONVERSATION: Service = Service {
    name: "conversation",
    interface: "syneroym-roym:conversation/api@0.1.0",
};
// ... profile, catalog, transaction, directory

pub struct Service {
    pub name: &'static str,
    pub interface: &'static str,
}

/// Every service, in manifest order. `web` first.
pub const ALL: [Service; 6] = [WEB, CONVERSATION, PROFILE, CATALOG, TRANSACTION, DIRECTORY];

/// The five `web` declares `depends_on` (everything but itself).
pub const SIBLINGS: [Service; 5] = [CONVERSATION, PROFILE, CATALOG, TRANSACTION, DIRECTORY];
```

---

## §5 `syneroym-roym-core` — the shared crate

Five modules. None of them is product behaviour.

### 5.1 `src/services.rs`

§4.2 above.

### 5.2 `src/envelope.rs` — the `invoke` request/response vocabulary

```rust
/// What the Web entrypoint observed about the caller, forwarded to a
/// sibling. Never trusted on its own -- see `CallerEnvelope::accept`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CallerEnvelope {
    /// The person's master DID, when a person session was active. `None`
    /// means the entrypoint saw no person -- an anonymous or node-identity
    /// request.
    pub person_did: Option<String>,
    /// How the entrypoint's own inbound request was authenticated, as the
    /// `syneroym:http` `caller-auth` enum spells it: "delegated", "ucan",
    /// or "self-asserted". Copied, never inferred.
    pub auth: String,
    /// The forwarding service's own id. Diagnostic only: it is not proof
    /// of anything, because a proxied call carries no caller identity on
    /// either build (see the module doc).
    pub origin: String,
}

/// One `invoke` request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
    pub caller: CallerEnvelope,
}

/// One `invoke` response. Exactly one of `result`/`error` is present.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response { /* result: Option<Value>, error: Option<RpcError> */ }
```

The module doc carries `D-C2-4`'s reasoning verbatim — that a proxied call
forwards no caller on either build (F1, F2), that this envelope is the
substitute, and that it is admissible only under `AuthLevel::System`.

`CallerEnvelope::accept`:

```
fn accept(envelope, observed: &CallerContext) -> Principal
    if observed.auth == AuthLevel::System:
        // Only in-node dispatch produces System. `service_system` sets
        // proof: None, so it cannot cross a wire hop, and no service but
        // `web` declares an http_routes entry (D-C2-5), which is the other
        // way System is produced.
        return Principal::from_envelope(envelope)
    // A direct caller: attribute to who they actually are, and drop the
    // envelope entirely rather than merging the two.
    return Principal::direct(observed.caller_did, observed.auth)
```

`Principal` is `{ did: Option<String>, auth: Auth, trusted: bool }`. It is
what C4–C10's authorization reads. C2 uses it only to answer `whoami`.

### 5.3 `src/router.rs` — the method-prefix table (`D-C2-3`)

```rust
/// JSON-RPC method prefix -> the sibling that owns it. The spec's own API
/// column, made executable. No default arm: an unlisted prefix is
/// `-32601`, because "which service owns this?" is a question with a
/// written answer or no answer.
const ROUTES: &[(&str, Service)] = &[
    ("conversation.", CONVERSATION),
    ("profile.",      PROFILE),
    ("contacts.",     PROFILE),
    ("block.",        PROFILE),
    ("listing.",      CATALOG),
    ("availability.", CATALOG),
    ("request.",      TRANSACTION),
    ("quote.",        TRANSACTION),
    ("agreement.",    TRANSACTION),
    ("receipt.",      TRANSACTION),
    ("directory.",    DIRECTORY),
];

pub fn route(method: &str) -> Option<Service>;
```

`session.whoami` is **not** in the table: `web` answers it itself from the
inbound `caller`, with no sibling call. That is the one exception and it is
listed explicitly in `route`'s doc, not implied by a missing entry.

Unit tests: (a) every prefix maps to a service in `SIBLINGS`; (b) no prefix
is a prefix of another; (c) an unlisted method returns `None`; (d) the set
of services reachable from the table equals `SIBLINGS`, so a sibling that is
in the manifest but unreachable from the API fails the build.

### 5.4 `src/card.rs` — the fixed type set (`D-06C-3`, `D-C2-8`)

```rust
/// The seven card types of the first release, and the version each one is
/// rendered at today. Fixed: a card of an unlisted type, or a listed type
/// at an unlisted version, renders as the neutral unknown block.
pub const CARD_TYPES: &[(&str, u32)] = &[
    ("request", 1),
    ("quote", 1),
    ("agreement-receipt", 1),
    ("booking-progress", 1),
    ("payment-request", 1),
    ("payment-acknowledgement", 1),
    ("fulfilment-receipt", 1),
];
```

Plus the drift test (`D-C2-8`):

```
#[test] fn the_ui_card_registry_matches_this_crate
    read ../roym_web/ui/src/cards/registry.ts
    parse the `export const CARD_TYPES: [string, number][] = [...]` literal
    assert the parsed pairs equal CARD_TYPES, in the same order
```

Parse with a small hand-written scan for `["<name>", <n>]` pairs between the
first `[` and the matching `]` — not a TS parser, and a malformed file fails
the test rather than being skipped.

### 5.5 `src/dual_build.rs` — the wiring both builds share

Two things every service crate needs and none should write twice:

```rust
/// Parses an `invoke` request string, dispatches it through the app's own
/// handler, and encodes the response. The `Err` arm is reserved for a
/// request that could not be parsed at all.
pub async fn handle_invoke<H, F, Fut>(host: &H, request: &str, f: F)
    -> Result<String, String>
where H: AppHost, F: FnOnce(&H, Request) -> Fut, Fut: Future<Output = Response>;

/// The same two parameter shapes `json_to_wasm_params` accepts on the WASM
/// side -- positional `["<json>"]` or named `{"request": "<json>"}` -- so
/// one client frame drives both builds. Lifted verbatim from the fixture's
/// `extract_request_param`.
pub fn extract_request_param(params: &serde_json::Value) -> Option<String>;
```

Plus a generic `NativeApi<H: AppHost>` adapter implementing
`syneroym_rpc::NativeService` over an app-supplied
`fn(&H, Request) -> Response`, so each service's `native.rs` is roughly ten
lines. Gate it `#[cfg(not(target_arch = "wasm32"))]` — `roym_core` must
still compile for `wasm32`, where `syneroym-rpc` is absent.

---

## §6 The five sibling services

Each of `roym_conversation`, `roym_profile`, `roym_catalog`,
`roym_transaction`, `roym_directory` is the same four files.

### 6.1 `src/lib.rs`

```rust
pub mod app;
#[cfg(target_arch = "wasm32")]
mod guest;
#[cfg(not(target_arch = "wasm32"))]
pub mod native;
```

Identical to the fixture's (`test-components/dual-build-fixture/src/lib.rs`).

### 6.2 `src/app.rs` — target-independent

```rust
/// This service's own schema version. Bumped by whichever slice changes
/// what this service stores; read by `status` and by nothing else in C2.
pub const SCHEMA_VERSION: u32 = 1;

pub async fn status<H: AppHost>(_host: &H) -> Result<String, String> {
    Ok(json!({
        "service": services::PROFILE.name,
        "schema_version": SCHEMA_VERSION,
    }).to_string())
}

pub async fn invoke<H: AppHost>(host: &H, req: Request, observed: &CallerContext)
    -> Response
{
    let principal = req.caller.accept(observed);
    match req.method.as_str() {
        // C2 declares no product verbs. `whoami` exists so the shared
        // suite can prove, on both builds, that the person the gateway
        // authenticated is the person this service sees.
        "profile.whoami" => Response::ok(json!({
            "did": principal.did,
            "auth": principal.auth.as_str(),
            "trusted": principal.trusted,
        })),
        other => Response::method_not_found(other),
    }
}
```

`observed` is the `CallerContext` the host gave this invocation. On the
native build it arrives as `NativeInvocation.caller`. **On the WASM build a
guest cannot read its own `CallerContext`** — there is no host import for
it. So the guest passes a synthesized `CallerContext` that names only what
the guest can know. See §6.3 for exactly what, and §14 (3) for why this is a
flagged asymmetry rather than a hidden one.

### 6.3 `src/guest.rs` — WASM wiring

Copy the fixture's `guest.rs` header verbatim (the `#[allow(unsafe_code)]`
`mod bindings` block, and the doc comment explaining why a second
`generate!` over the same imports cannot encode). For `profile`:

```rust
mod bindings {
    wit_bindgen::generate!({
        path: "wit",
        world: "profile",
        with: {
            "syneroym:data-layer/store@0.1.0":
                syneroym_wit_interfaces::data_layer::syneroym::data_layer::store,
        },
    });
    use super::Profile;
    export!(Profile);
}

struct Profile;

impl ApiGuest for Profile {
    fn invoke(request: String) -> Result<String, String> {
        block_on(roym_core::dual_build::handle_invoke(
            &GuestHost, &request,
            |h, req| crate::app::invoke(h, req, &guest_caller()),
        ))
    }
    fn status() -> Result<String, String> {
        block_on(crate::app::status(&GuestHost))
    }
}
```

`guest_caller()` returns `CallerContext::service_system(<this service's
name>)` — i.e. `auth: System`. That is *exactly* what the WASM host would
have synthesized for a proxied call anyway (F1), so the envelope gate
behaves identically on both builds. It is a synthesized value and the
function's doc comment says so in one sentence: a WASM guest has no import
that reveals its own caller, so the only honest thing it can assert is the
auth level the host already guarantees for a proxied call.

> **This is the load-bearing subtlety of the whole slice.** If the WASM
> guest synthesized anything *other* than `System`, the envelope would be
> rejected on the WASM build and accepted on the native build, and every
> identity test would pass one way and fail the other. §12.2's first
> scenario is written to fail loudly if that ever changes.

`CallerContext` is a `syneroym-rpc` type and `syneroym-rpc` is not a
`wasm32` dependency. So `roym_core` must define its own minimal
`ObservedCaller { auth: Auth }` rather than reuse `CallerContext`, and the
native side converts. Two lines, and it keeps `roym_core` free of
`syneroym-rpc` on the guest target. **Do this**; do not add `syneroym-rpc`
to the wasm32 dependency set.

### 6.4 `src/native.rs` — in-process wiring

```rust
pub struct NativeProfile<H: AppHost + 'static> {
    service_id: String,
    host_for: Box<dyn Fn(CallerContext) -> H + Send + Sync>,
}
// hand-written Debug, exactly as NativeFixture has (a boxed closure has none)

#[async_trait::async_trait]
impl<H: AppHost + 'static> NativeService for NativeProfile<H> {
    async fn dispatch(&self, inv: NativeInvocation) -> RpcResult<NativeResponse> {
        let observed = ObservedCaller::from(&inv.caller);
        let host = (self.host_for)(inv.caller);
        match inv.method.as_str() {
            "invoke" => { /* extract_request_param, app::invoke, wrap */ }
            "status" => { /* app::status */ }
            other => Err(RpcError::MethodNotFound(other.to_string())),
        }
    }
}
```

No `MessageSink` / `ConversationSink` / `HttpSink` / `WebSocketSink` impls
in C2 for the five siblings — none of them receives anything yet.
`roym_conversation` gains `ConversationSink` in C5, not here.

---

## §7 `roym_web` — the Web entrypoint

Same four files, plus the HTTP surface.

### 7.1 `src/app.rs`

Three entry points, all target-independent.

**`handle_http`** — the only public door.

```
fn handle_http(host, request: HttpRequest) -> Result<HttpResponse, String>
    match (request.method.as_str(), request.route.as_str()):
      ("POST", "/rpc")  -> rpc(host, request)
      ("GET",  "/health") -> 200, {"status":"ok"}
      _ -> 404 with a JSON body     // unreachable: the router matched a
                                    // declared route to get here, so this
                                    // arm is a defect check, not a path
```

**`rpc`** — the forwarder, and the whole of `D-C2-3`.

```
fn rpc(host, request: HttpRequest) -> HttpResponse
  1. body must parse as a JSON-RPC 2.0 Request object; else -32700/-32600
  2. envelope = CallerEnvelope {
         person_did: request.caller.filter(|c| c.auth == Delegated).map(did),
         auth:       request.caller.map(auth_str).unwrap_or("none"),
         origin:     <this service's own id, from app-config; see below>,
     }
     // person_did is Some ONLY for `delegated`. `self-asserted` on a
     // gateway-proxied request is the NODE's key, identical for every
     // visitor (http.wit's own words) -- treating it as a person would
     // make every anonymous visitor the same person.
  3. if method == "session.whoami": answer here, no sibling call
  4. service = roym_core::router::route(method) else -32601
  5. payload = Request { method, params, caller: envelope } as JSON string
  6. out = host.call(
         CallTarget::Dependency(service.name.into()),
         service.interface.into(),
         "invoke".into(),
         json!([payload]).to_string(),
         Some(CallOptions { idempotent: false, ..default }),
     )
  7. map ProxyError -> a JSON-RPC error object:
       DependencyNotBound -> -32001 "service not available"
       Timeout            -> -32002 "service did not answer in time"
       Callee{code,msg}   -> pass the callee's own code and message through
       everything else    -> -32603, with the variant name, never the detail
  8. 200 with the JSON-RPC Response. A JSON-RPC *error* is still HTTP 200 --
     the transport succeeded.
```

Step 2's `origin` comes from `host.get("service.id".into())` via
`AppAppConfig` if the deploy sets it, and falls back to the constant
`"web"`. It is diagnostic only (`D-C2-4`), so a fallback is honest here in a
way it would not be for anything authorizing.

**`invoke`** — `web`'s own `api`, for a non-HTTP caller. Same body as `rpc`
minus the HTTP framing, so the parity suite and any future CLI verb drive
identical code. Exit criterion 2's *"a second client drives the same flow
through the same API with no UI involved"* is discharged by a plain HTTP
script against `POST /rpc` (§12.3 item 6) plus this being the same
function, not a parallel one.

**`on_ws_open` / `on_ws_message` / `on_ws_close`** — C2 registers the
connection id in its own in-memory set and echoes nothing. The route is
declared so C5 has a live socket to push into; C2 proves the lifecycle
fires on both builds and nothing more.

### 7.2 `src/guest.rs`

The fixture's `guest.rs` is the template, including its `caller_auth_in` /
`caller_identity_in` / `http_request_in` / `http_response_out` /
`frame_kind_in` converters — **copy them, do not re-derive them.** They exist
because `http.wit`'s records live inside an *exported* interface, so there
is no shared generated type (C1 `F4`/`D-C1-3`). `world: "web"`, `with:` maps
`syneroym:proxy/proxy@0.1.0` and `syneroym:http/websocket@0.1.0` onto
`syneroym-app-host`'s bindings and `generate`s `websocket-types`,
`incoming-handler`, `websocket-handler`.

### 7.3 `src/native.rs`

`NativeWeb<H: AppHost>` implementing **four** traits:
`syneroym_rpc::NativeService` (for `api`), `syneroym_app_host_native::HttpSink`,
`syneroym_app_host_native::WebSocketSink`. Bodies delegate to `app.rs`
exactly as `NativeFixture`'s do
([native.rs:93-129](../../../../test-components/dual-build-fixture/src/native.rs#L93)).

---

## §8 The Hub UI bundle

`crates/roym_web/ui/`, a Vite + TypeScript project, following
`test-components/miniapp-demo1-wasm/client/` (the only browser build in the
tree). Output `dist/`, packed to `bundle.tar.gz` by §3.5's task.

### 8.1 What the shell contains

| Screen | Behaviour |
|---|---|
| Login | `GET /_syneroym/session/identities` → a picker. On choose, `POST /_syneroym/session/login-local`, then reload. Empty list → the message from `D-C2-7` naming `roymctl identity create`. |
| Session bar | `GET /_syneroym/session/whoami` on load. A 401 renders **"the substrate restarted — log in again"** as an ordinary state, never an error banner (`task.md`'s carried-forward gateway limit; failure matrix row 17). |
| Home | Calls `session.whoami` and one sibling's `whoami` through `POST /rpc`, and shows both DIDs. This is the shell's proof that the person reaches a service, and C4 replaces it with the real profile screen. |
| Card gallery | Renders one sample of each of the seven types plus the unknown-type fallback, from a local fixture file. Not reachable from the Home screen; it exists so the renderer and its tests have a subject before C7 produces a real card. |

### 8.2 The card renderer — `ui/src/cards/`

- `registry.ts` — `export const CARD_TYPES: [string, number][] = [...]`, the
  seven pairs, in `roym_core`'s order. This is the file §5.4's Rust test
  reads.
- `templates/<type>.ts` — one per type. Each takes the parsed card object
  and returns a DOM node built with `document.createElement` and
  `textContent`. **No `innerHTML`, no `insertAdjacentHTML`, no
  `dangerouslySetInnerHTML`, no `<template>` string interpolation, anywhere
  under `ui/src/`.** An ESLint rule (`no-restricted-properties` on
  `innerHTML`/`outerHTML`, `no-restricted-syntax` for
  `insertAdjacentHTML`) enforces it, and `npm run build` runs the lint.
- `unknown.ts` — the fallback: a neutral block naming the type and version
  and saying this client does not understand it. Reached for an unlisted
  type **and** for a listed type at an unlisted version (`D-06C-3`).
- `link.ts` — the one place a card's URL is rendered. Shows the URL in full
  as text, wrapped in an `<a>` with `rel="noopener noreferrer"` and no
  `target`, and **never** as an `<img>`, `<iframe>`, `<link>`, or any
  attribute a browser fetches. Nothing prefetches, resolves, or navigates.

### 8.3 Content-Security-Policy

The entrypoint's asset responses cannot set headers (the host serves them),
so the policy ships as a `<meta http-equiv="Content-Security-Policy">` in
`index.html`:

```
default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:;
connect-src 'self'; form-action 'none'; base-uri 'none'; frame-ancestors 'none'
```

`connect-src 'self'` is what makes the one-origin rule enforced by the
browser and not only by convention, and it is why `default-src 'none'` is
worth the strictness: a card that somehow got a URL into a fetchable
position would be blocked by the browser as well as by §8.2's renderer.
Confirm the meta form is honoured for `connect-src` in the target browsers
during §13 step 9; if a header proves necessary, that is a router change and
belongs in its own commit.

---

## §9 The `SynAppManifest`

### 9.1 Location and path rules (F10)

`crates/roym_core/app/roym.toml`. `roym_core` because the manifest is the
app as a whole, not any one service, and `roym_core` is already the crate
that owns cross-service facts (§4.2, §5.3).

Both path rules apply and differ, so state both in a comment at the top of
the file:

- `source` resolves against the **client process's cwd**, so every `source`
  is written relative to the workspace root
  (`target/wasm32-wasip2/release/syneroym_roym_web.wasm`), and the deploy
  command must be run from the workspace root.
- `assets.archive` resolves against the **manifest's own directory** for
  `roymctl supervisor submit`, so it is written
  `../../roym_web/ui/bundle.tar.gz`.

Verify the second empirically in §13 step 8 against the actual command used;
if `roymctl app deploy`'s mapper uses cwd for `archive` too, change it to
the workspace-relative form and record which command was used, rather than
carrying a rule that is right for one path and wrong for the other.

### 9.2 The services

```toml
id = "syneroym:roym"
version = "0.1.0"
description = "Roym"

[services.web]
service_type = "wasm"
source = "target/wasm32-wasip2/release/syneroym_roym_web.wasm"
interfaces = ["syneroym-roym:web/api@0.1.0"]
depends_on = ["conversation", "profile", "catalog", "transaction", "directory"]
visibility = "internal"
custom_config = '''{"http_routes":[
  {"method":"POST","path":"/rpc",   "target":"guest",    "operation":"handle-request","public":false},
  {"method":"GET", "path":"/health","target":"guest",    "operation":"handle-request","public":true},
  {"method":"GET", "path":"/ws",    "target":"websocket","operation":"handle-upgrade","public":false}
]}'''

[services.web.assets]
archive = "../../roym_web/ui/bundle.tar.gz"
visibility = "public"

[services.web.health_check.rpc]
interface = "syneroym-roym:web/api@0.1.0"
method = "status"
```

and, for each of the five siblings (shown for `catalog`):

```toml
[services.catalog]
service_type = "wasm"
source = "target/wasm32-wasip2/release/syneroym_roym_catalog.wasm"
interfaces = ["syneroym-roym:catalog/api@0.1.0"]
visibility = "public"

[services.catalog.health_check.rpc]
interface = "syneroym-roym:catalog/api@0.1.0"
method = "status"
```

No sibling declares `custom_config` or `assets` (`D-C2-5`). No sibling
declares `depends_on` — `web → siblings` is the only edge, so F12's cycle
check is satisfied by construction.

`/health` is `public: true` and every other route is not. That is
deliberate and is the one place `D-C2-5`'s reasoning needs care: `public`
routes produce a `System` caller (F3), but this is `web`, which is the
service that *sends* envelopes and never accepts one, so the gate `D-C2-4`
protects is not reachable through it. Say so in the manifest comment.

### 9.3 Visibility, and what each value buys (`task.md`'s C2 row)

| Service | `visibility` | `topology_visibility` | Why |
|---|---|---|---|
| `web` | `internal` | `restricted` | Reached only by the local client gateway, which needs a published endpoint record to resolve the unscoped host form (F8). `internal` publishes to this substrate's registry and no further (ADR-0018 §209). Nothing outside this install addresses it. |
| `conversation` | `public` | `restricted` | A peer on another installation addresses it **by DID**, from a listing or a contact entry — an endpoint record, never a topology document. |
| `catalog` | `public` | `restricted` | Same: a consumer reaches a provider's catalog by the DID a listing carried. |
| `transaction` | `public` | `restricted` | Same, and it is the provider's single writer (spec rule 1). |
| `profile` | `private` | `restricted` | Nothing outside this install ever addresses it: it holds the person's *own* profile, contacts, and block list. Publishing it would publish the existence of a person for no consumer. This is also the subject of failure-matrix row 18's negative half — "a service that declares neither stays cleanly refused". |
| `directory` | `public` | `open` | The one service a stranger resolves by **topology document** with no pre-installed token — failure-matrix row 18's positive half, and `D-B2-15`'s named case. |

F7: `public`/`internal` require a certificate, so the deploy is
`roymctl app deploy roym crates/roym_core/app/roym.toml --mint-masters
--registry-url <url>`, run from the workspace root. A deploy without those
flags fails at `web`, and the C2 status note records that as expected.

### 9.4 The Hub's URL (F8, closing C1 §12 (11))

`http://s<short_hash(<web service DID>)>.localhost:7960/`.

The `web` service's DID is minted at deploy, so the URL is per-install. Add
a line to the deploy step in the developer guide showing how to compute it
(`roymctl app deploy` already prints each service's resolved master DID;
`short_hash` is `syneroym_core::util::short_hash`). The app-scoped
`-a…-s…` form also works once `topology_visibility`/`resolve_ucan` are in
place, and is not used in C2: it needs a credential the unscoped form does
not, and buys nothing here.

---

## §10 The gateway: browser person login (`D-C2-6`)

### 10.1 `crates/core/src/config.rs`

`ClientGatewayRole` gains one field, defaulting to absent:

```rust
    /// Directory of person identity key files (`<name>.key`), the same
    /// layout `roymctl` uses under its own `--dir`. Present enables the
    /// local login endpoints below; absent leaves them 404, which is what
    /// every configuration written before this field means.
    ///
    /// A local login signs with a key on this machine. That is not new
    /// exposure: any process that can reach this loopback endpoint can
    /// already read the same file and run `roymctl session login`. Do not
    /// point this at a directory reachable by a less-trusted local user.
    #[serde(default)]
    pub person_identities_dir: Option<PathBuf>,
```

Update `ClientGatewayRole::default()` and `config.sample.toml` (a commented
example plus the sentence above).

### 10.2 `crates/client_gateway/src/session.rs`

Two new `SessionRoute` variants and two `classify` arms:

```rust
pub enum SessionRoute { Challenge, Login, Logout, Whoami, Identities, LoginLocal, Unknown }
```
```rust
    ("GET",  "/_syneroym/session/identities")  => Session(SessionRoute::Identities),
    ("POST", "/_syneroym/session/login-local") => Session(SessionRoute::LoginLocal),
```

Both are under `GATEWAY_RESERVED_PATH_PREFIX`, so `classify`'s existing
guard already keeps them from ever being proxied to a guest — the property
`gateway_session_e2e.rs` already pins.

New request/response types beside the existing ones:

```rust
#[derive(Deserialize)] pub struct LocalLoginRequest { pub identity: String }
#[derive(Serialize)]   pub struct IdentitiesResponse { pub identities: Vec<String> }
```

New free function, unit-testable without a socket:

```rust
/// Every `<name>.key` in `dir`, sorted, names only. Returns an empty list
/// for a missing or unreadable directory -- a login attempt against a name
/// that is not there fails with the same 404 either way, so distinguishing
/// them here would leak whether a path exists.
pub fn list_person_identities(dir: &Path) -> Vec<String>;
```

`list_person_identities` refuses any name containing a path separator, `..`,
or a NUL, and the login path re-checks the chosen name against the listed
set rather than joining caller-supplied text onto `dir`. Two guards, because
the second one is the one that actually holds if the first is ever moved.

### 10.3 `crates/client_gateway/src/gateway.rs`

`GatewayState` gains `person_identities_dir: Option<PathBuf>` and
`registry_url` is already present. `ClientGateway::init` reads the new
config field.

`handle_session_request` gains two arms:

```
SessionRoute::Identities:
    let Some(dir) = state.person_identities_dir else -> 404
        {"error":"local person identities are not configured"}
    200 { "identities": list_person_identities(dir) }

SessionRoute::LoginLocal:
    let Some(dir) = state.person_identities_dir else -> 404 (same body)
    parse LocalLoginRequest; malformed -> 400
    if !list_person_identities(dir).contains(&req.identity) -> 404
        {"error":"no such local identity"}
    let person = Identity::load_from_path(dir.join(format!("{name}.key")))
        // io error -> 500, message names the file, not its contents
    let person_did = substrate::derive_did_key(&person.public_key())

    // Same five steps `roymctl session login` performs, in the same order,
    // with no network hop to this gateway because we are it.
    let ch   = state.sessions.issue_challenge()
    let node = substrate::resolve_did_key(&ch.node_did)
    let cert = DelegationCertificate::issue(
                   &person, node, session_ttl_secs, SCOPE_ROUTING)
    let sig  = person.sign_json(&assertion_value(&ch.node_did, &ch.nonce, &person_did))

    // Publish the anchor BEFORE login, or step 4 of SessionStore::login
    // answers 409 AnchorUnresolvable. Failure here is reported, not
    // swallowed: a session that cannot be used is worse than a refusal.
    if let Some(url) = &state.registry_url_if_configured() {
        RegistryClient::new(dht, Some(url)).refresh_master_anchor(&person).await
            -> on error, 502 {"error":"could not publish this person's anchor"}
    }

    match state.sessions.login(&LoginRequest{person_did, nonce: ch.nonce, signature: sig,
                                             delegation: cert},
                               state.anchor_lookup.as_ref()).await {
        Ok(grant)  => 200 + the SAME Set-Cookie the remote Login arm builds
        Err(e)     => e.http_status(), {"error": e.message()}
    }
```

**Factor the `Set-Cookie` construction and the 200 body out of the existing
`SessionRoute::Login` arm into one helper and call it from both.** Two
places building a session cookie is exactly how one of them ends up without
`HttpOnly`.

`expires_hours` is deliberately not a request parameter: the delegation is
issued for `session_ttl_secs`, and `SessionStore::login` already takes the
earlier of that and its own ceiling. A caller-chosen lifetime would let a
local page mint a longer-lived delegation than the operator configured.

### 10.4 Tests (`crates/substrate/tests/gateway_session_e2e.rs`)

Extend the existing file rather than starting a new one — it already boots
a substrate with a gateway and has the helpers.

1. `identities` with no configured dir → 404, and `login-local` likewise.
2. `identities` lists exactly the `.key` files present, names only, and the
   response body contains no key bytes.
3. `login-local` for a listed identity → 200, a `Set-Cookie` with
   `HttpOnly` and `SameSite=Strict`, and a following `whoami` with that
   cookie reports that person's DID with `auth = "delegated"`.
4. A proxied request under that cookie reaches a guest as
   `caller-auth = delegated` with the person's DID — the same assertion the
   existing cookie test makes, through the new door.
5. `login-local` for an unlisted name → 404; for `../something` → 404 and
   **no** file outside `dir` is opened.
6. Restart the substrate → `whoami` is 401 (sessions are in-memory by
   design), and `login-local` works again immediately.

---

## §11 `syneroym-substrate` — native wiring

### 11.1 `Cargo.toml`

```toml
# Links the Roym SynApp's native build in as six `NativeService`s plus one
# inbound HTTP surface, proving M06C exit criterion 1 (built both ways from
# one source tree). Off by default. Unlike `dual_build_fixture`, this is a
# real product feature, not test scaffolding -- but it registers services
# with no deploy record, so a node running it must not also deploy the WASM
# build of the same app (the router warns; see `handle_guest_route`).
roym = [
    "dep:syneroym-roym-core",
    "dep:syneroym-roym-web",
    "dep:syneroym-roym-conversation",
    "dep:syneroym-roym-profile",
    "dep:syneroym-roym-catalog",
    "dep:syneroym-roym-transaction",
    "dep:syneroym-roym-directory",
]
```

with the seven as `optional = true` workspace dependencies.

### 11.2 `SharedNodeHandles` — two new fields (F5)

```rust
    /// The static asset table. A linked native app has no deploy record,
    /// so nothing else would put its UI bundle here.
    #[cfg_attr(not(feature = "roym"), allow(dead_code))]
    assets: AssetRegistry,
    /// The in-memory topology inventory the node's own `LogicalResolver`
    /// reads. A linked app installs its own dependency bindings here
    /// (`install_app_context`'s in-process equivalent).
    #[cfg_attr(not(feature = "roym"), allow(dead_code))]
    app_registry: Arc<StaticInventory>,
```

Set both in the `SharedNodeHandles { … }` literal at
[runtime.rs:1363](../../../../crates/substrate/src/runtime.rs#L1363) from
the locals already in scope (`assets.clone()`, `app_registry.clone()`); note
`assets` currently moves into `RouteHandlerDeps` at line 1389, so clone it
before.

Also extend the five existing `#[cfg_attr(not(feature = "dual_build_fixture"), …)]`
attributes on `blob_provider`, `logical_resolver`, `http_routes`,
`native_http`, `websocket_senders` to
`#[cfg_attr(all(not(feature = "dual_build_fixture"), not(feature = "roym")), allow(dead_code))]`
— `roym` uses every one of them.

### 11.3 `init_roym` — the new wiring function

Modelled on `init_dual_build_fixture`
([runtime.rs:1003-1148](../../../../crates/substrate/src/runtime.rs#L1003)),
with three things the fixture never needed: six factories instead of one,
the app-context/binding installation (`D-C2-10`), and the asset bundle
(`D-C2-9`).

```
#[cfg(feature = "roym")]
const ROYM_APP_INSTANCE: &str = "roym";

/// A linked service's dispatch key: "roym-<logical name>". Not a DID --
/// a linked app mints no identity (D-C2-11) -- and not the bare logical
/// name, which would collide with any deployed service that happened to
/// be called `profile`.
fn roym_dispatch_id(name: &str) -> String { format!("roym-{name}") }

async fn init_roym(shared, endpoint_registry, node_service_id, config)
    -> Result<Vec<Arc<NativeHostFactory>>>
{
  factories = []
  // 1. One factory + one NativeService per service.
  for svc in roym_core::services::ALL:
      let id = roym_dispatch_id(svc.name)
      let factory = NativeHostFactory::new(
          id.clone(), shared.key_store, shared.storage_provider,
          shared.blob_provider, shared.messaging_broker,
          endpoint_registry.clone(), shared.logical_resolver,
          shared.conversation, shared.websocket_senders)
      let f = factory.clone()
      let service = <the crate's Native* type>::new(id.clone(),
                        move |caller| f.host_for(caller))
      shared.native_dispatch.insert(id.clone(), service.clone() as Arc<dyn NativeService>)
      endpoint_registry.register(id.clone(), svc.interface.to_string(),
          SubstrateEndpoint::NativeHostChannel { service_id: id.clone() }).await?
      factories.push(factory)

  // 2. `web` alone gets the HTTP surface (D-C2-5).
  let web_id = roym_dispatch_id("web")
  factory_web.set_http_sink(downgrade(web) as Weak<dyn HttpSink>)
  factory_web.set_websocket_sink(downgrade(web) as Weak<dyn WebSocketSink>)
  let adapter = Arc::new(NativeHttpAdapter::new(factory_web.clone(),
                    downgrade(web), downgrade(web)))
  shared.native_http.insert(web_id.clone(), adapter as Arc<dyn NativeHttpService>)
  shared.http_routes.insert(web_id.clone(), roym_http_routes())
  endpoint_registry.register(web_id.clone(), "http-native".into(),
      NativeHostChannel { service_id: web_id.clone() }).await?

  // 3. App context and dependency bindings (D-C2-10). This is
  //    `ControlPlaneService::install_app_context`'s in-process twin, and
  //    the two must stay the same shape: a binding row that only one of
  //    them writes is a difference between the two builds by another name.
  for svc in ALL:
      endpoint_registry.set_app_context(
          roym_dispatch_id(svc.name), ROYM_APP_INSTANCE.into(), svc.name.into()).await?
  for dep in SIBLINGS:
      let entry = TopologyEntry {
          mode: TopologyMode::Singleton,
          members: vec![ServiceId::new(roym_dispatch_id(dep.name))],
          sharding_strategy: None,
          epoch: TopologyEpoch(1),
          cache_ttl: Duration::from_secs(60),
          not_after: None,
      }
      shared.app_registry.register(
          TopologyKey::local(AppInstanceId::new(ROYM_APP_INSTANCE),
                             LogicalServiceName::new(dep.name)), entry.clone())
      endpoint_registry.save_binding(&web_id, ROYM_APP_INSTANCE, dep.name,
                                     &serde_json::to_string(&entry)?).await?

  // 4. The UI bundle (D-C2-9), best-effort and loud on failure.
  if let Some(path) = config.roles.roym.as_ref().and_then(|r| r.ui_bundle_path.as_ref()):
      match fs::read(path) {
        Ok(archive) =>
          let mut written = BTreeSet::new()
          let manifest = unpack_asset_bundle(&web_id, &archive, None,
                             &roym_http_routes(), &shared.blob_provider,
                             dek_for(&web_id), &mut written).await
          shared.assets.insert(web_id.clone(), ServiceAssets {
              manifest: Arc::new(manifest), public: true, manifest_hash })
        Err(e) => warn!(?path, %e,
            "Roym UI bundle could not be read; the API is unaffected and \
             the Hub will not be served from this node")
      }
  else: info!("no roym.ui_bundle_path configured; serving the API without the Hub")

  Ok(factories)
}
```

`save_binding` writes only `web`'s five rows: no sibling declares a
dependency (§9.2), and writing a row nothing reads would make the linked
app's context differ from the deployed app's.

**The `dek_for` / `manifest_hash` details.** `unpack_asset_bundle`'s
existing deploy call site
([orchestration.rs](../../../../crates/control_plane/src/service/orchestration.rs))
already resolves both — read it and reuse the same derivation rather than
inventing a second one; if the DEK lookup turns out to be private to the
control plane, register the bundle through a small `pub(crate)` helper there
instead of duplicating the key derivation in `runtime.rs`. Do not guess at
it.

### 11.4 Call sites in `run` / `setup_router`

Beside the existing `dual_build_fixture` lines at
[runtime.rs:757-778](../../../../crates/substrate/src/runtime.rs#L757):

```rust
    #[cfg(feature = "roym")]
    let roym_factories = init_roym(&shared, &endpoint_registry, service_id, config).await?;

    let router = ConnectionRouter::init(…).await?;

    if let Some(proxy) = router.proxy() {
        shared.conversation.set_service_proxy(…);
        #[cfg(feature = "dual_build_fixture")]
        if let Some(factory) = fixture_factory { factory.set_service_proxy(…); }
        #[cfg(feature = "roym")]
        for factory in &roym_factories {
            factory.set_service_proxy(Arc::downgrade(&proxy) as Weak<dyn ServiceProxy>);
        }
    }
```

Same two-phase wiring and same reason as C1 §9.3: `ProxyRouter` does not
exist until `ConnectionRouter::init` returns. Every one of the six factories
needs it — `web` to make calls, and the five siblings because C4–C10 will.

### 11.5 New config role

`crates/core/src/config.rs`, beside the other role structs:

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RoymRole {
    /// Path to the Hub's gzip-tar UI bundle. Absent serves the API with no
    /// UI (D-C2-9). Read once at startup; changing it needs a restart.
    pub ui_bundle_path: Option<PathBuf>,
}
```

`RolesConfig` ([config.rs:380](../../../../crates/core/src/config.rs#L380))
gains `pub roym: Option<RoymRole>`. Gate the field
`#[cfg(feature = …)]`-free — a config key for a feature this binary was not
built with should be ignored, not rejected, exactly as the other roles are.

---

## §12 Tests

### 12.1 What each suite is for (`D-C2-12`)

| Suite | Level | Proves |
|---|---|---|
| `crates/roym_core/src/**` unit tests | in crate | the routing table (§5.3), the card set drift guard (§5.4), envelope acceptance (§5.2) |
| `crates/roym_web/tests/dual_build_parity.rs` | in process, both builds | every §12.2 scenario, byte-identical |
| `crates/substrate/tests/roym_app_e2e.rs` | router + gateway, WASM only | registration, routes, the Hub URL, the person session end to end, visibility |
| `crates/substrate/tests/gateway_session_e2e.rs` | gateway | §10.4's six cases |
| `crates/substrate/tests/e2e/tests/roym-hub.spec.ts` | real browser | login, the card gallery, and matrix row 4 |

### 12.2 The parity scenarios

Structure: copy `crates/app_host_native/tests/dual_build_parity.rs`'s
`Driver` trait, `SCENARIOS` byte-comparison table, and
`the_parity_comparison_detects_a_divergence` mutant test wholesale. The
`WasmDriver` deploys all six components into one `AppSandboxEngine`; the
`NativeDriver` builds six `NativeHostFactory`s. Both are handed a test
`ServiceProxy` that resolves `CallTarget::Dependency(name)` and
`CallTarget::Service(id)` into the same build's dispatcher — the one place
the two harnesses differ, and the reason a real `ProxyRouter` is not used
(it would drag the router into a shim-level suite).

| # | Scenario | Assertion |
|---|---|---|
| 1 | `web.invoke` with `method: "profile.whoami"` and a `delegated` envelope naming `did:key:hAlice` | The response names `did:key:hAlice`, `trusted: true`. **Identical on both builds** — this is the test F1's divergence would break if `guest_caller()` (§6.3) ever stopped returning `System`. |
| 2 | The same, with `auth: "self-asserted"` in the envelope | `person_did` is `null` and `trusted` is false: a self-asserted gateway request is the node's key, not a person. |
| 3 | A sibling reached **directly** (not through `web`) with a caller whose `auth` is `Ucan` and an envelope claiming `did:key:hMallory` | The envelope is ignored; the response names the Ucan caller's own DID. `D-C2-4`'s gate, from the attacker's side. |
| 4 | `web.invoke`, `method: "nope.thing"` | JSON-RPC `-32601`, and **no** proxy call is made (the test proxy records zero invocations). |
| 5 | `web.invoke` routed to a dependency the test proxy refuses with `DependencyNotBound` | `-32001`, with a message that names neither the DID nor the dependency's internals. |
| 6 | `handle_http` `POST /rpc` with the same body as scenario 1 | HTTP 200 and a body byte-identical to scenario 1's. This is what proves `rpc` and `invoke` are one function (§7.1). |
| 7 | `handle_http` `POST /rpc` with a malformed body | HTTP 200, JSON-RPC `-32700`. Never a 500 — a bad request is not a handler fault. |
| 8 | Each of the six services' `status` | A JSON object naming that service and `schema_version: 1`. Six rows, one per service. |
| 9 | WebSocket `on_open` → `on_message` → `on_close` against `web` | The lifecycle fires in order and the connection id is registered then removed. |
| 10 | `web.invoke` with `method: "directory.<x>"` addressed by `CallTarget::Service("did:key:hForeign")` instead of by dependency | Reaches the same handler. `D-C2-13`'s second target shape, on both builds. |

### 12.3 `crates/substrate/tests/roym_app_e2e.rs`

Boot one substrate with a registry and a gateway; deploy the six components
with `--mint-masters`; then:

1. `GET /` on the `web` service's own DID returns the Hub's `index.html`
   from the asset bundle, with the CSP meta present in the body.
2. `POST /rpc` with `session.whoami` under a person session cookie returns
   the person's DID with `auth: "delegated"` — **exit criterion 3**.
3. The same request through the gateway's unscoped host form
   `s<hash>.localhost:<gw port>` reaches the same handler — §9.4, and the
   answer to C1 §12 (11).
4. `POST /rpc` with `profile.whoami` under the same cookie returns the same
   person DID — the person reached a *sibling*, which is what `D-C2-4`
   exists for.
5. A caller on an unaffiliated identity resolves `directory`'s topology
   document with no pre-installed token, and the same call for `profile` is
   cleanly refused — **failure matrix row 18**, both halves. Model on
   `crates/substrate/tests/service_visibility_e2e.rs`.
6. A plain HTTP client (`reqwest` in the test, and a documented `curl`
   recipe in the developer guide) drives `POST /rpc` with
   `Authorization: Bearer <token from roymctl session token>` and gets the
   same `session.whoami` answer with no browser — **exit criterion 2's
   second-client half**. Deliberately not a `roymctl` verb: none exists
   (§14 (10)), and adding one is not C2's scope.
7. A second local process with no cookie sees `self-asserted:<node did>`,
   not the person — the property `gateway_session_e2e.rs` already pins,
   re-asserted through Roym's own door.

### 12.4 `crates/substrate/tests/e2e/tests/roym-hub.spec.ts`

Extend `global-setup.ts` to deploy Roym and export `ROYM_HUB_URL`. Four
tests:

1. Login: the identity picker lists the seeded person, choosing it logs in,
   and the session bar shows that person's DID.
2. Restart handling: kill and restart the substrate, reload → the Hub shows
   "log in again" as an ordinary state, not an error — **failure matrix
   row 17**.
3. Card gallery: all seven types render their own template, and an eighth
   fixture with `type: "not-a-real-type"` and a ninth with
   `type: "quote", version: 99` both render the neutral unknown block naming
   the type — `D-06C-3` and **matrix row 4**.
4. Card safety: a fixture whose fields contain `<img src=x onerror=…>`, a
   `<script>` tag, and a `javascript:` URL produces **zero** console errors,
   **zero** network requests to any origin other than the Hub's own
   (asserted with `page.on('request')`), and the markup appears as visible
   literal text. This is **exit criterion 10**, and it is a browser test
   because it is a claim about a browser.

---

## §13 Order of work

Each step compiles and its own tests pass before the next begins.

| # | Step | Gate |
|---|---|---|
| 1 | `roym_core`: crate, `services.rs`, `envelope.rs`, `router.rs`, `card.rs` (without the drift test), `dual_build.rs` | `cargo test -p syneroym-roym-core`; `cargo build -p syneroym-roym-core --target wasm32-wasip2` |
| 2 | `roym_profile` end to end (WIT, `app.rs`, `guest.rs`, `native.rs`, symlinks, `Cargo.toml`) — **one** sibling, complete, before the other four | `cargo component build --target wasm32-wasip2 -p syneroym-roym-profile`; `cargo build -p syneroym-roym-profile` |
| 3 | The other four siblings, by copying step 2 | same, per crate |
| 4 | `roym_web`: WIT, `app.rs`, `guest.rs`, `native.rs` | both builds |
| 5 | `crates/roym_web/tests/dual_build_parity.rs`, scenarios 1–10 | `cargo test -p syneroym-roym-web` |
| 6 | Gateway: config field, `session.rs` routes, `gateway.rs` arms, the cookie helper extraction (§10) | `cargo test -p syneroym-client-gateway`; `cargo test -p syneroym-substrate --test gateway_session_e2e` |
| 7 | `syneroym-substrate`: `roym` feature, `SharedNodeHandles` fields, `init_roym`, `RoymRole` | `cargo build -p syneroym-substrate --features roym`; `cargo build -p syneroym-substrate --all-features` |
| 8 | The manifest (§9) and the `mise` tasks (§3.5); deploy it by hand once and record the exact command and the resolved Hub URL | `roymctl app deploy …` succeeds; `curl` reaches `/health` |
| 9 | The UI bundle (§8), including the ESLint rule and the CSP meta; then `roym_core`'s card drift test | `mise run build:roym-ui`; `cargo test -p syneroym-roym-core` |
| 10 | `crates/substrate/tests/roym_app_e2e.rs` | `cargo test -p syneroym-substrate --test roym_app_e2e` |
| 11 | `roym-hub.spec.ts` and the `global-setup.ts` deploy | `mise run test:e2e` |
| 12 | Docs and backlog (§15) | — |
| 13 | Full gate | `cargo +nightly fmt --all`, `cargo clippy --workspace --all-targets --all-features`, `cargo test --workspace`, `cargo audit`, `cargo deny check licenses`, `mise run test:e2e` |

**Step 2 is the one to get right slowly.** Four crates are copied from it,
and every mistake in the WIT/`generate!`/symlink triangle is copied four
times. **Step 8 is the riskiest**, because it is the first time the two path
rules (F10), the certificate requirement (F7), and the hostname form (F8)
are exercised together, and none of them is provable before then.

---

## §14 Ambiguities and staleness in the input documents

Raised rather than guessed.

1. **`task.md`'s C2 row says the entrypoint "forwards JSON-RPC" but the tree
   forwards no caller identity with it.** F1 and F2 together mean the
   person's DID cannot reach a sibling through any existing channel on
   either build. Neither `task.md` nor the spec names this, and exit
   criterion 3 (*"Roym's services see **that person's** DID with
   `caller-auth = delegated`"*) reads as though it were solved. Resolved as
   `D-C2-4` — an application-level envelope admitted only under an
   `AuthLevel::System` caller. **This is the largest single design item in
   C2 and it was not in the slice's budget.** If the implementing session
   disagrees, the alternative is real cross-service caller delegation
   (ADR-0015 UCAN), which is substrate work and belongs behind a decision,
   not inside a product slice.

2. **`ProxyRouter::invoke_local` behaves differently for a WASM callee and a
   native callee** (F1) — the WASM arm passes `None` and gets
   `service_system(<callee>)`, the native arm passes the caller's own
   `service_system(<caller>)`. By failure-matrix row 19's own rule
   (*"Any interface behaves differently on the WASM build and the native
   build → the shared suite fails"*) this is a defect. C2 routes around it
   rather than fixing it: `D-C2-4`'s gate reads `auth`, which is `System` on
   both. Fixing it means changing what a proxied WASM callee sees as its
   caller, which could change any deployed FDAE policy keyed on
   `system:<own id>` — too large to fold into a skeleton slice. **Backlog
   row, §15.**

3. **A WASM guest cannot read its own `CallerContext`** (§6.3). There is no
   host import for it, so the guest synthesizes `System`. That is *correct*
   for every path C2 has — a guest is reached either through the proxy
   (where the host would have synthesized exactly that) or through
   `web`'s HTTP handler (where the real caller arrives in `HttpRequest.caller`
   instead). It stops being correct the moment a sibling is reachable
   directly from the wire *and* wants to authorize on the caller — which is
   C4's problem, not C2's. Named here so C4 does not inherit it silently.

4. **`task.md` says the Web entrypoint forwards to "the four services below"**
   (the spec's own Client contract diagram shows four), while the C2 slice
   row names **six** services including `directory`. Resolved as `D-C2-13`:
   six in the manifest, five in `depends_on`, because the SynOrg owner's own
   Hub administers the local directory while a consumer's chosen directories
   are runtime DIDs. The spec's diagram is describing the *consumer's* Hub
   and is not wrong; it is just not the whole set. No spec edit is needed,
   but the C2 status note should say which reading was taken.

5. **The spec says "The bundle is embedded in the entrypoint for the first
   release, so it versions with the app automatically"**, and separately
   that serving it from `blob-store` is *"a later convenience"*. M06A A1
   made those the same thing: `assets.archive` ships with the deploy (so it
   versions with the app) *and* is served from blob storage (so the
   component is never instantiated). The spec sentence predates A1 and now
   reads as a distinction that no longer exists. C2 uses `assets`. **Spec
   edit owed** — one sentence, §15.

6. **`task.md`'s C2 row says "sibling wiring by `depends_on` +
   `call-target::dependency`" and stops there.** It does not say how a
   *natively linked* app gets those bindings, which C1 §12 (10) had already
   identified and given a backlog row targeted at C2. `D-C2-10` closes it.
   Flagging that the slice row understates its own scope by one wiring
   function.

7. **Nothing in `task.md` says who builds the UI toolchain.** The Hub needs
   a JS build, an npm dependency tree, and a lint rule, none of which exist
   in `crates/`. The only precedent is
   `test-components/miniapp-demo1-wasm/client/`. C2 follows it and adds
   `build:roym-ui` to `mise.toml` (§3.5). Naming it because it is real work
   the slice row does not mention, and because it adds a second npm project
   to the repo's dependency surface (`cargo audit` does not cover it; an
   `npm audit` step is **not** added by this plan and is left as an open
   question for the operator).

8. **`D-06C-3` fixes seven card types but not their version numbers.** The
   table's "Signed by" column names producers, and every type is at version
   1 by construction because none exists yet. C2 writes `1` for all seven
   (§5.4) and the fallback path is what handles a future bump. Recording
   the assumption rather than treating an absent number as `1` by silence.

9. **`task.md`'s "Owed as slices land" says C2 must record whether the
   entrypoint needed any exemption from D2/D3.** It does not: `roym_web` is
   an ordinary component and its native build makes no host-crate call.
   Confirmed by construction — the crate's `[target.'cfg(not(wasm32))']`
   dependencies are `syneroym-rpc`, `syneroym-app-host-native`, and
   `async-trait`, and none of `syneroym-identity`, `syneroym-data-db`,
   `syneroym-data-blob`, `syneroym-conversation` appears. **Add a
   workspace-level test asserting exactly that** for all seven Roym crates,
   so a future slice cannot add one without failing a build.

10. **Exit criterion 2 names `roymctl` as a possible second client, and no
    such verb exists.** `SvcCommands` has no `call`
    ([svc.rs](../../../../apps/roymctl/src/commands/svc.rs)): its verbs are
    `Deploy`, `Remove`, `Restart`, `ProxyOutbox`, `ProxyDeadLetters`,
    `ProxyReplay`, `Sagas`, `SagaCompensate`, `EndpointInfo`. The criterion
    says *"a script **or** `roymctl`"*, so a plain HTTP script satisfies it
    and C2 uses one (§12.3 item 6). Naming it so nobody reads the criterion
    as requiring a CLI verb, and so the gap is visible when a later slice
    wants one. **No backlog row** — the criterion is already satisfiable,
    and a verb nothing needs is speculative surface.

11. **`D-06C-11` and the `roym` Cargo feature.** `roym` is a product name,
    not a planning identifier, so the feature name, the crate names, the
    `roym-` dispatch-id prefix, and `ROYM_APP_INSTANCE` are all compliant.
    Confirmed by reading, and the grep exit criterion 12 asks for
    (`R1|R2|R3|R4|C[0-9]|M06`) matches nothing this plan introduces.

---

## §15 Documents and backlog owed

| Document | Edit |
|---|---|
| `docs/planning/milestones/M06C-roym-product/status.md` | C2's section: the six services and their visibility table (§9.3); `D-C2-4`'s envelope rule and why it exists; the F1 divergence, named as a permitted difference C2 routes around rather than fixes; `D-C2-11`'s no-instance-certificate limit; the resolved Hub URL form; and the §14 (4) reading of "four services" vs six. Update the slice table's C2 row |
| [deferred-backlog.md](../../deferred-backlog.md) §5 | **Retarget** *"A natively linked app's message subscriptions do not survive a process restart"* (`M06C C2`). C2 does not close it: none of the six services subscribes to anything yet. Move to `M06C C5` (`roym_conversation` is its first consumer) with that reason, or restate the C1 `F12` argument — a linked app re-subscribes from its own startup path — now that `init_roym` **is** that startup path. Prefer the second: `init_roym` is where a `NativeHostFactory::start()` hook would go, and C2 is the first slice with a reason to want one |
| [deferred-backlog.md](../../deferred-backlog.md) — the `M06C C2` app-context row | **Move to "Recently resolved"** with what shipped: `init_roym` writes `set_app_context` and `save_binding` for the linked build, and registers the same `TopologyEntry`s the deploy path registers (`D-C2-10`) |
| [deferred-backlog.md](../../deferred-backlog.md) §7, browser-login row | **Move to "Recently resolved"**, recording `D-C2-6` — a gateway-side local login, not a browser-held key — and the security argument that makes it equivalent to `roymctl session login`. Note that WebAuthn/passkey is still not built and open a **narrower** row for it, target `TBD`, trigger *"a person's key must live somewhere the node cannot read"* |
| [deferred-backlog.md](../../deferred-backlog.md) §3, new row | **A proxied call reaches a WASM callee with a different caller than a native callee** — `ProxyRouter::invoke_local` passes `None` for `WasmChannel` (synthesizing `service_system(<callee>)`) and the caller's own context for `NativeHostChannel` (`crates/router/src/proxy.rs:673,716`). Both are `AuthLevel::System`, so nothing in C2 depends on the difference (`D-C2-4` gates on `auth`), but it is a failure-matrix row 19 case. Target `TBD`, trigger *"a service authorizes on the identity of the sibling that called it"* |
| [deferred-backlog.md](../../deferred-backlog.md) §5, new row | **No cross-service caller delegation** — neither build forwards a caller through `syneroym:proxy` (`host_capabilities.rs:1350`), so an app must carry the person in its own payload (`D-C2-4`). Target `TBD`, trigger *"a Roym service must authorize a person it did not receive over HTTP"*; source ADR-0015 |
| [deferred-backlog.md](../../deferred-backlog.md) §5, new row | **The Roym sibling boundary is not typed by WIT** — one `invoke: func(string) -> result<string,string>` per service (`D-C2-2`), so a malformed inner call is an application error rather than a dispatch error. Target `TBD`, trigger *"a sibling boundary needs typed WIT parameters"* |
| [deferred-backlog.md](../../deferred-backlog.md) §8, new row | **A natively linked Roym holds no instance certificate** (`D-C2-11`), so `enqueue` and cross-node `call` are refused on that build. Target `TBD`, trigger *"the native build must reach another installation"* |
| [deferred-backlog.md](../../deferred-backlog.md) §11 | Any `TODO`/`FIXME` this slice leaves — expected: one in `roym_web`'s `origin` fallback (§7.1) and one in each sibling's `invoke` where the verb table is empty. Each needs a matching row |
| [roym-integrated-experience-spec.md](../../../roym-integrated-experience-spec.md) | §14 (5): the Client-contract bullet *"The bundle is embedded in the entrypoint … Serving it from `blob-store` instead would allow a UI update without redeploying the service; that is a later convenience"* is stale — M06A A1 made shipping-with-the-deploy and serving-from-blobs the same mechanism. One sentence, with a pointer to `assets` |
| [task.md](./task.md) | Gap 2 recorded as **closed** (owed by C2 per the "Owed as slices land" table). Add the §14 (4) clarification that C2's `depends_on` set is five, not four |
| [docs/developer-guide.md](../../../developer-guide.md) | The deploy command, the resolved Hub URL form (§9.4), and the new `roles.client_gateway.person_identities_dir` / `roles.roym.ui_bundle_path` keys |
| [CLAUDE.md](../../../../CLAUDE.md) / [AGENTS.md](../../../../AGENTS.md) | The architecture section's component list gains the Roym crates in one sentence. **No** WIT-interface-list edit: C2 adds `syneroym-roym:*` packages, which are the app's own, not host interfaces. C3 owes that edit for the signing package |

---

## §16 What "done" means for C2

1. Seven crates exist under `crates/`, named per `D-06C-9`, and each of the
   six services builds **both** as a `wasm32-wasip2` component and as a
   host-target library linked into `syneroym-substrate` behind the `roym`
   feature.
2. No Roym crate depends on `syneroym-identity`, `syneroym-data-db`,
   `syneroym-data-blob`, `syneroym-conversation`, or any other host crate on
   either target — D3, asserted by a test, not by reading (§14 (9)).
3. `crates/roym_core/app/roym.toml` deploys, all six services reach
   `running`, and each answers its `status` probe.
4. `crates/roym_web/tests/dual_build_parity.rs` compares both builds across
   all ten scenarios in §12.2 with byte-identical results, and the mutant
   test still detects an injected divergence.
5. A person logs in **from the browser**, and `POST /rpc` with
   `profile.whoami` returns that person's DID with `auth = "delegated"` —
   through the client gateway, from one origin, on the WASM build.
6. A caller on an unaffiliated installation resolves `directory` with no
   pre-installed token, and the same call for `profile` is cleanly refused.
7. The Hub renders all seven card types plus the unknown-type fallback, and
   a browser test proves that a card carrying markup, script, or a URL
   results in no execution, no insertion, and no fetch to any other origin.
8. Gap 2 is recorded closed, and no exemption from D2 or D3 was needed for
   the entrypoint (§14 (9)) — or, if one was, it is written down as a
   `D-06B-1` regression rather than absorbed.
9. `cargo +nightly fmt --all`, `cargo clippy --workspace --all-targets
   --all-features`, `cargo test --workspace`, `cargo audit`,
   `cargo deny check licenses`, and `mise run test:e2e` are clean.
