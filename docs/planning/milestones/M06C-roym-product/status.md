# M06C The Roym Product — Status

**Milestone:** [task.md](task.md) · **Designs of record:**
[slice-c1-implementation-plan.md](slice-c1-implementation-plan.md) (C1)

**Overall:** Slice C1 complete (2026-08-25) — see C1's status and evidence below.

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

Slice C1 completes the dual-build shim for all guest capabilities Roym needs: `syneroym:proxy`, `syneroym:http` inbound (`incoming-handler` and `websocket-handler`), `syneroym:app-config`, and `syneroym:vault`.

### 1. WIT Bindgen & Export Unification (`crates/wit_interfaces`)
- Added `proxy` package under `crates/wit_interfaces/wit/proxy/proxy.wit` (`syneroym:proxy/proxy@0.1.0`).
- Generated typed Rust bindings for both guest and host targets (`proxy`, `http_guest`, `http_host`).
- Re-exported guest bindings from `crates/wit_interfaces/src/lib.rs`.

### 2. Common HTTP Types & Traits (`crates/app_host`)
- Replaced hand-written `guest_http` types with unified `app_host::types::http` (`HttpRequest`, `HttpResponse`, `FrameKind`, `HttpError`, `WebSocketError`).
- Added `AppHttp` and `AppWebSocket` traits to `syneroym-app-host`.
- Updated `AppProxy`, `AppAppConfig`, `AppVault`, `AppBlobStore`, `AppDataLayer`, `AppMessaging`, `AppConversation` guest implementations in `crates/app_host/src/guest.rs`.

### 3. Native HTTP Dispatch & WebSocket Channel Registry (`crates/rpc`)
- Created `NativeHttpService` trait and `NativeHttpRegistry` in `crates/rpc/src/native_http.rs`.
- Decoupled `WebSocketSenders` into `crates/rpc/src/websocket_senders.rs`, allowing native services to register, send, broadcast, and drain WebSocket connections without depending on WASM sandbox internals.

### 4. Native App Host & Adapter (`crates/app_host_native`)
- Implemented `NativeHttpAdapter`, `HttpSink`, and `WebSocketSink` in `crates/app_host_native/src/http.rs`.
- Implemented full trait suite on `NativeAppHost`: `AppHttp`, `AppWebSocket`, `AppProxy`, `AppAppConfig`, `AppVault`, `AppDataLayer`, `AppBlobStore`, `AppMessaging`, `AppConversation`.
- Implemented lazy FDAE policy loading from service SQLite storage with non-memoized transient error handling and fail-closed ABAC gate.
- Handled `config_generation` validation and monotonic reload logic.

### 5. Router Native HTTP & Substrate Fixture Wiring (`crates/router`, `crates/substrate`)
- Integrated native HTTP routing into `RouteHandlerInner.native_http` in `crates/router/src/route_handler/http.rs`.
- Extended `SharedNodeHandles` and registered `/whoami`, `http`, and `http-native` endpoints under both `DUAL_BUILD_FIXTURE_DISPATCH_ID` and `node_service_id` in `crates/substrate/src/runtime.rs`.

### 6. Dual-Build Fixture & Verification (`test-components/dual-build-fixture`, `crates/app_host_native/tests/dual_build_parity.rs`, `crates/substrate/tests/dual_build_fixture_e2e.rs`)
- Built `test-components/dual-build-fixture` as both `wasm32-wasip2` component and natively linked module.
- 32 dual-build parity tests in `dual_build_parity.rs` proving identical behavior across WASM and native builds for all host capabilities.
- 3 substrate E2E tests in `dual_build_fixture_e2e.rs` proving router reachability, HTTP routing, and access control for linked native services.

---

## C1 — Verification evidence

1. `cargo test -p syneroym-app-host-native --test dual_build_parity`: **32 passed, 0 failed**
2. `cargo test -p syneroym-app-host-native --lib`: **24 passed, 0 failed**
3. `cargo test -p syneroym-substrate --test dual_build_fixture_e2e`: **3 passed, 0 failed**
4. `cargo test -p syneroym-substrate --test conversation_e2e`: **1 passed, 0 failed**
5. `cargo test -p syneroym-substrate --test group_conversation_e2e`: **1 passed, 0 failed**
6. `cargo test -p syneroym-coordinator-iroh --test multi_hop_relay`: **5 passed, 0 failed**
7. `cargo test --workspace`: **All unit, integration, and doc tests passed**
8. `cargo +nightly fmt --all`: **Clean**
9. `cargo clippy --workspace --all-targets --all-features`: **Clean (0 errors, 0 warnings)**
10. `cargo audit`: **Clean (0 vulnerabilities found across 915 dependencies)**
11. `cargo deny check licenses`: **Clean (`licenses ok`)**
12. `mise run test:e2e`: **4 passed (20.0s, clean)**
