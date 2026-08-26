# M06C The Roym Product — Status

**Milestone:** [task.md](task.md) · **Designs of record:**
[slice-c1-implementation-plan.md](slice-c1-implementation-plan.md) (C1),
[slice-c2-implementation-plan.md](slice-c2-implementation-plan.md) (C2)

**Overall:** Slice C2 complete — blocked on verification — see C1's and C2's status, architectural decisions, permitted differences, and evidence below.

---

## Slice status

| Slice | Scope | Status | Gate |
|---|---|---|---|
| C1 | Complete the dual-build shim (Gap 2, D-06C-10) | **Complete (2026-08-25)** — [implementation plan](slice-c1-implementation-plan.md), evidence below | None — independently mergeable |
| C2 | The SynApp skeleton and the Hub shell | **Complete — blocked on verification** — [implementation plan](slice-c2-implementation-plan.md), evidence below; blocked on the two [deferred-backlog](../../deferred-backlog.md#10-product-surfaces--ux) rows for `GET /` on a WASM-deployed Hub and `roymctl app deploy`'s CLI timeout | C1 |
| C3 | Signed records: host signing interface and envelope | Not started | C1 |
| C4 | Identity, profile, contacts, and safety (R1 rows 1 and 6) | Not started | C3 |
| C5 | Catalog and conversation in the product (R1 rows 2 and 3) | Not started | C4 |
| C6 | Directory: the search half (R1 row 5) | Not started | C5 |
| C7 | A need becomes an offer, and the card contract (R1 row 4) | Not started | C5, C6 |
| C8 | The transaction vertical (R2, all five rows) | Not started | C7 |
| C9 | Cross-installation trust (R3, all three rows) | Not started | C8 |
| C10 | Private group chat in the product (R4, all five rows) | Not started | C5, C9 |

---

## C1 — What shipped

Slice C1 completes the dual-build shim for the guest capabilities Roym requires: `syneroym:proxy` (`call`/`enqueue`), `syneroym:http` inbound (`incoming-handler` and `websocket-handler`), `syneroym:app-config`, and `syneroym:vault`.

### 1. WIT Bindgen & Interface Bindings (`crates/wit_interfaces`, `crates/app_host`)
- Added `proxy` world to `crates/wit_interfaces/wit/proxy/proxy.wit` (`syneroym:proxy/proxy@0.1.0`) with typed guest and host bindings.
- Generated typed bindings for `http_host` (renamed to avoid collisions with guest `http`) and guest HTTP/WebSocket interfaces.
- Standardized common HTTP/WebSocket types in `syneroym_app_host::types::http` (`HttpRequest`, `HttpResponse`, `FrameKind`, `CallerAuth`).
- Implemented `AppProxy`, `AppAppConfig`, and `AppVault` guest bridges in `crates/app_host/src/guest.rs`.

### 2. Native HTTP Sinks & Dispatch (`crates/rpc`, `crates/app_host_native`)
- Defined `HttpSink` and `WebSocketSink` in `crates/app_host_native/src/http.rs` (D-C1-4: placed in `app_host_native` because guest code implements WIT exports rather than host traits).
- Created `NativeHttpService` trait and `NativeHttpRegistry` in `crates/rpc/src/native_http.rs`.
- Implemented `NativeHttpAdapter` wrapping `HttpSink` and `WebSocketSink` into `NativeHttpService`.
- Decoupled `WebSocketSenders` into `crates/rpc/src/websocket_senders.rs` as a shared table across `RouteHandlerInner`, `AppSandboxEngine`, and `NativeAppHost`.

### 3. Native App Host Implementation (`crates/app_host_native`)
- Implemented full trait suite on `NativeAppHost`: `AppProxy`, `AppAppConfig`, `AppVault`, `AppDataLayer`, `AppBlobStore`, `AppMessaging`, `AppConversation`, `AppWebSocket`.
- Implemented lazy FDAE policy loading in `NativeHostFactory` with `fdae_policy_generation` atomic concurrency guard, transient load error resilience, and fail-closed handling for ABAC policies.
- Implemented per-invocation config reads via `get_latest_config_generation` against the service store.

### 4. Router Integration & Substrate Fixture (`crates/router`, `crates/substrate`)
- Integrated native HTTP routing in `crates/router/src/route_handler/http.rs`, dispatching via `native_http` and issuing warnings if a native HTTP service shadows a deployed WASM service.
- Registered `/whoami`, `/ws`, and `/run` HTTP routes and native services under `DUAL_BUILD_FIXTURE_DISPATCH_ID` and `node_service_id` in `crates/substrate/src/runtime.rs`.

### 5. Architectural Decisions & Test Harness
- `HttpSink`/`WebSocketSink` location: Defined in `app_host_native` rather than `app_host` (D-C1-4) since guests export WIT functions directly.
- WIT package naming: `http_host` naming avoids clashes with guest `http` definitions.
- Test parity harness: `crates/app_host_native/tests/dual_build_parity.rs` runs with real AES-GCM encrypted SQLite storage provider and injected KEK.

---

## §14 Permitted Differences (WASM vs Native Build)

As specified in §14 of the implementation plan, the following structural differences between the WASM component host and the native in-process shim are intentional and accepted:

1. **Guest HTTP Admission Limiting:** WASM execution acquires per-service concurrency permits (`guest_http_permits`); native HTTP dispatches directly as asynchronous Tokio tasks.
2. **HTTP Failure Taxonomy:** WASM bridge classifies failures into `Unavailable`, `Declined`, or `Trap`; native handlers return `Result<HttpResponse, String>`.
3. **WebSocket Permits:** WASM bridge enforces `guest_websocket_permits`; native WebSocket connections register directly in the shared `WebSocketSenders` table.
4. **Row Authorizer (Stage-4 ABAC):** WASM host instantiates guest `syneroym:data-layer/authorizer` for stage-4 post-query filtering; native host uses `empty_row_authorizer()` and fails closed for policies with ABAC rules.
5. **Subscription Replay:** WASM engine replays stored subscriptions from `StorageProvider` on startup; native app subscription replay is deferred to C2 (`D-C1-9`).
6. **Isolated Resource Tables:** WASM execution manages Wasmtime store-bound resource handles; native host allocates fresh resource tables per invocation (with handle IDs starting at `rep 0`), ensuring isolation without cross-invocation handle reuse.

---

## C1 — Verification evidence

1. `cargo test -p syneroym-app-host-native --test dual_build_parity`: **34 passed, 0 failed**
2. `cargo test -p syneroym-substrate --features dual_build_fixture --test dual_build_fixture_e2e`: **4 passed, 0 failed**
3. `cargo test -p syneroym-coordinator-iroh --test multi_hop_relay`: **5 passed, 0 failed**
4. `cargo test -p syneroym-substrate --test conversation_e2e`: **1 passed, 0 failed**
5. `cargo test -p syneroym-substrate --test group_conversation_e2e`: **1 passed, 0 failed**
6. `cargo test --workspace`: **All tests passed**
7. `cargo +nightly fmt --all`: **Clean**
8. `cargo clippy --workspace --all-targets --all-features`: **Clean (0 errors, 0 warnings)**
9. `cargo audit`: **Clean (0 vulnerabilities)**
10. `cargo deny check licenses`: **Clean (`licenses ok`)**
11. `mise run test:e2e`: **4 passed (clean)**

---

## C2 — What shipped

Slice C2 stands up the Roym SynApp skeleton (six native-linked services: `web`, `profile`, `conversation`, `catalog`, `transaction`, `directory`) and the Hub shell — the first browser-facing surface for a Roym installation, reached through the client gateway's local login flow.

### 1. The SynApp manifest and native services (`crates/roym_core/app/roym.toml`, `crates/roym_{web,profile,conversation,catalog,transaction,directory}`)

Each service exports one WIT interface, `invoke(request: string) -> result<string, string>` plus `status()`, carrying a JSON-RPC-shaped envelope (`syneroym_roym_core::envelope`) rather than a WIT function per product verb. `web` is the only one with an inbound HTTP surface (`/rpc`, `/health`, `/ws`, plus the static UI bundle) and the only one that declares `depends_on`; every other service answers `<name>.ping` only, proving reachability through the manifest's real dependency bindings, not identity.

Declared visibility (`roym.toml`):

| Service | `visibility` | `topology_visibility` | Reachable from |
|---|---|---|---|
| `web` | `internal` | (default) | Gateway only (`/rpc`, `/health`, `/ws`; `/health` is the one `public: true` route) |
| `profile` | `private` | (default) | `web` only, as a bound dependency |
| `conversation` | `public` | (default) | Any verified caller with a resolve grant |
| `catalog` | `public` | (default) | Any verified caller with a resolve grant |
| `transaction` | `public` | (default) | Any verified caller with a resolve grant |
| `directory` | `public` | `open` | Any verified caller, no grant needed (topology-document resolution) |

### 2. The Hub shell (`crates/roym_web/ui`)

A minimal TypeScript/Vite single-page app: a person-identity picker backed by `GET /_syneroym/session/identities` and `POST /_syneroym/session/login-local`, a session bar showing the logged-in DID, a Home screen that round-trips `profile.ping` over `POST /rpc`, and a seven-card gallery (`request`, `quote`, `agreement-receipt`, `booking-progress`, `payment-request`, `payment-acknowledgement`, `fulfilment-receipt`) plus an unknown-type fallback. Packed as `bundle.tar.gz` (gitignored, rebuilt by `mise run build:roym-ui`) and deployed as the `web` service's asset bundle.

### 3. Both Hub URL forms (ADR-0022 §7 grammar)

A deployed Roym Hub is reachable through the client gateway on two hostname forms, both resolving to the same `web` service:

- **Bare service form**: `<nickname>-s<service-did-hash>[-i<interface-hash>].<domain>` — used when `web` is deployed standalone (`roymctl svc deploy`).
- **App-instance form**: `<nickname>-a<app-did-hash>-s<service-name-hash>[-i<interface-hash>].<domain>` — used when `web` is deployed as part of the `roym` app instance (`roymctl app deploy`), naming the app and the logical service name (`web`) rather than a raw service DID.

An omitted `-i` segment resolves to `web`'s one app-declared interface (`http-native`).

### 4. Permitted differences (D-C2-4, F1, D-C2-11)

1. **No forwarded caller identity (D-C2-4).** `syneroym_roym_core::envelope::Request` carries no caller field: a sibling has no sound way to verify a caller claim forwarded to it inside the JSON-RPC payload, so nothing in the envelope claims to carry one. Every C2 response is correct but anonymous from a sibling's point of view (see the deferred-backlog row on this).
2. **WASM-vs-native caller divergence (F1).** The self-proxy caller-forwarding exception (a component calling its own service forwards the real caller) is evaluated against `HostState.component_id`, which is populated differently per build path. Identical for every case C2's parity suite drives; not proven identical for every future caller shape (see the deferred-backlog row).
3. **Native Hub reachable only at the node's own address (D-C2-11).** `init_roym`'s six services are native-linked into the one substrate process; there is no separate network address per service the way a remote WASM deploy might imply — every one of them is reached at this node's own gateway/router address, under its own DID.

---

## C2 — Verification evidence

1. `cargo test -p syneroym-roym-web --test dual_build_parity`: **11 passed, 0 failed**
2. `cargo test -p syneroym-roym-core`: **5 passed, 0 failed**
3. `cargo test -p syneroym-substrate --test gateway_session_e2e`: **19 passed, 0 failed**
4. `cargo test -p syneroym-substrate --test roym_app_e2e`: `test_roym_app_e2e_lifecycle`'s first assertion fails — see the [deferred-backlog row](../../deferred-backlog.md#10-product-surfaces--ux) on `GET /` returning a 500 on a WASM-deployed Hub; the lifecycle test's remaining steps and this pass's `deploy_roym_app`-based tests are unaffected
5. `cargo +nightly fmt --all`: **Clean**
6. `cargo clippy --workspace --all-targets --all-features`: **Clean**
7. `cargo audit`: **Clean (0 vulnerabilities)**
8. `cargo deny check licenses`: **Clean (`licenses ok`)**
9. `cargo xtask check-roym-deps`: **Clean, and confirmed to reject a planted violation**
10. `mise run test:e2e`: **Not run** — `global-setup.ts`'s `roymctl app deploy` step times out deploying the six Roym services; see the [deferred-backlog row](../../deferred-backlog.md#10-product-surfaces--ux) on that CLI-driven deploy timeout

