# M06C The Roym Product — Status

**Milestone:** [task.md](task.md) · **Designs of record:**
[slice-c1-implementation-plan.md](slice-c1-implementation-plan.md) (C1)

**Overall:** Slice C1 complete (2026-08-25) — see C1's status, architectural decisions, permitted differences, and evidence below.

---

## Slice status

| Slice | Scope | Status | Gate |
|---|---|---|---|
| C1 | Complete the dual-build shim (Gap 2, D-06C-10) | **Complete (2026-08-25)** — [implementation plan](slice-c1-implementation-plan.md), evidence below | None — independently mergeable |
| C2 | The SynApp skeleton and the Hub shell | Not started | C1 |
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

