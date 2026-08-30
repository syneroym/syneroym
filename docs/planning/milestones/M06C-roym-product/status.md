# M06C The Roym Product — Status

**Milestone:** [task.md](task.md) · **Designs of record:**
[slice-c1-implementation-plan.md](slice-c1-implementation-plan.md) (C1),
[slice-c1.1-implementation-plan.md](slice-c1.1-implementation-plan.md) (C1.1,
under [ADR-0024](../../../decisions/0024-client-gateway-identity-and-auth-service.md)),
[slice-c2-implementation-plan.md](slice-c2-implementation-plan.md) (C2)

**Overall:** Slices C1 (2026-08-25), C1.1 (2026-08-28), and C2 (2026-08-29) complete. C1.1, added by ADR-0024, makes the client gateway a dumb proxy with an `identity_mode` and moves the person session onto a node auth service; C2 builds the six-service Roym SynApp skeleton and the Hub shell on top of that model. The held `feat/m06c-slice-c2` branch (built before C1.1) was ported onto it rather than rebased commit-by-commit.

---

## Slice status

| Slice | Scope | Status | Gate |
|---|---|---|---|
| C1 | Complete the dual-build shim (Gap 2, D-06C-10) | **Complete (2026-08-25)** — [implementation plan](slice-c1-implementation-plan.md), evidence below | None — independently mergeable |
| C1.1 | The node auth service and the dumb client gateway (ADR-0024) | **Complete (2026-08-28)** — [implementation plan](slice-c1.1-implementation-plan.md), evidence below | C1 |
| C2 | The SynApp skeleton and the Hub shell | **Complete (2026-08-29)** — [implementation plan](slice-c2-implementation-plan.md), evidence below | C1.1 |
| C3 | Signed records: host signing interface and envelope | Not started | C1.1 |
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
11. `mise run test:e2e`: **23 passed (clean — 19 passed default config + 4 passed multi-hop config)**

---

## C1.1 — What shipped

Slice C1.1 implements the node auth service and simplifies the client gateway to a dumb reverse proxy per [ADR-0024](../../../decisions/0024-client-gateway-identity-and-auth-service.md).

### 1. New Auth Service Crate (`crates/auth`, package `syneroym-auth`)
- `SessionToken` minting, parsing, and verification against node auth service public key / DID (D-06C-1.1-1).
- Session tokens are `CapabilityToken` instances with empty `capabilities` and `proofs` (who, not what).
- Native HTTP service implementing endpoints:
  - `POST /_syneroym/session/challenge`: issues random single-use nonces with configurable TTL.
  - `POST /_syneroym/session/login`: supports `delegated-key` and `local` login methods; returns `200 OK` with JSON session grant + `Set-Cookie: syneroym_session=...; HttpOnly; SameSite=Lax; Path=/`.
  - `GET /_syneroym/session/methods`: returns list of enabled login methods.
  - `GET /_syneroym/session/whoami`: inspects `Cookie` or `Authorization: Bearer` and returns caller DID and auth facts.
  - `POST /_syneroym/session/logout`: revokes session token and clears cookie (`Max-Age=0`).
  - `POST /_syneroym/session/refresh`: extends valid unexpired session.

### 2. Client Gateway Refactoring (`crates/client_gateway`)
- Removed `SessionStore`, in-memory session tracking, and pre-login credential minting (D-06C-1.1-2).
- Added `IdentityMode` configuration (`Open`, `Login`, `Fixed`).
- Proxies `/_syneroym/session/*` to local `"auth"` native service without requiring `Host` header.
- Returns `404 Not Found` for unknown reserved `/_syneroym/*` endpoints.
- Proxies raw `Cookie` and `Authorization` headers untouched to target services.
- Added optional `connection_auth_gate` in login mode.

### 3. CLI Session Commands (`apps/roymctl`)
- `roymctl session delegate`: creates temporary keypair and signs `DelegationCertificate` under person master key with `session-auth` scope.
- `roymctl session login`: supports `delegated-key` (using `--session-key-file`) and `local` (using `--identity`).
- `roymctl session whoami`, `status`, `refresh`, `logout`: sends `Cookie` and `Authorization: Bearer` tokens.

### 4. Router & Substrate Integration (`crates/router`, `crates/substrate`)
- Substrate registers `AuthService` in `NativeHttpRegistry` and registers public routes in `HttpRouteRegistry`.
- Router extracts `syneroym_session` token from `Cookie` or `Authorization: Bearer` on guest HTTP requests, verifies against trusted native auth service, and populates `CallerIdentity { did: person_did, auth: CallerAuth::Delegated }`.
- Deleted deprecated `crates/ucan/src/normalize.rs` per D-06C-1.1-9.

---

## C1.1 — Verification evidence

1. `cargo test -p syneroym-auth`: **3 passed, 0 failed** (unit + integration tests)
2. `cargo test -p syneroym-client-gateway`: **4 passed, 0 failed**
3. `cargo test -p roymctl`: **89 passed, 0 failed**
4. `cargo test -p syneroym-router`: **265 passed, 0 failed**
5. `cargo test -p syneroym-substrate --test gateway_session_e2e`: **20 passed, 0 failed**
6. `cargo test -p syneroym-substrate --test guest_http_e2e`: **14 passed, 0 failed**
7. `cargo test -p syneroym-substrate --test basic_lifecycle`: **3 passed, 0 failed**
8. `cargo test --workspace`: **All tests passed**
9. `cargo +nightly fmt --all`: **Clean**
10. `cargo clippy --workspace --all-targets --all-features`: **Clean (0 errors, 0 warnings)**
11. `cargo audit`: **Clean (0 vulnerabilities)**
12. `cargo deny check licenses`: **Clean (`licenses ok`)**
13. `mise run test:e2e`: **23 passed (clean — 19 passed default config + 4 passed multi-hop config)**

---

## C2 — What shipped

Slice C2 stands up the Roym SynApp skeleton (seven crates: `syneroym-roym-core` plus one per service — `web`, `profile`, `conversation`, `catalog`, `transaction`, `directory`) and the Hub shell. The held `feat/m06c-slice-c2` branch was built before C1.1; its server-side login half (a gateway-owned `SessionStore`, `POST /_syneroym/session/login-local`, `GET /_syneroym/session/identities`, `roles.client_gateway.person_identities_dir`) was dropped and the browser half rewritten against C1.1's auth service.

### 1. The SynApp manifest and services (`crates/roym_core/app/roym.toml`, `crates/roym_*`)

Each service exports one WIT interface — `invoke(request: string) -> result<string, string>` plus `status()` — carrying a JSON-RPC-shaped envelope (`syneroym_roym_core::envelope`) rather than a WIT function per verb. `web` is the only inbound HTTP surface (`/rpc`, `/health` public, `/ws`, and the static UI bundle) and the only service with `depends_on`; the other five answer `<name>.ping` only, proving reachability through the manifest's real dependency bindings. Both builds pass one parity suite (`crates/roym_web/tests/dual_build_parity.rs`), and behind the `roym` Cargo feature the six services link natively into `syneroym-substrate` through `init_roym`.

Declared visibility (`roym.toml`): `web` `internal`, `profile` `private`, `conversation`/`catalog`/`transaction` `public`, `directory` `public` + `topology_visibility = "open"`.

### 2. The Hub shell (`crates/roym_web/ui`)

A TypeScript/Vite single-page app. Login is `delegated-key` only (`GET /_syneroym/session/methods` drives the screen): the person runs `roymctl session delegate`, the Hub imports the resulting `session-key.json` into IndexedDB as a non-extractable WebCrypto Ed25519 key, signs the auth service's challenge (the challenge returns the exact canonical assertion string, so the browser never canonicalizes JSON), and posts the login. A session bar shows the logged-in DID; a Home screen round-trips `profile.ping` over `/rpc`; a seven-card gallery plus an unknown-type fallback exercises the renderer. Packed as `bundle.tar.gz` (gitignored, `mise run build:roym-ui`) and deployed as `web`'s asset bundle.

### 3. What C2 discharges, and what it does not

**Discharges:** the six services exist and are addressable both ways (dependency binding and gateway hostname); a request from a logged-in person's browser reaches `web` and comes back; `web`'s `session.whoami` reports that person's DID, because after C1.1 the connection router verifies the session cookie and hands `web` the person's DID as `HttpRequest.caller` (no guest-side crypto).

**Does not discharge:** a **sibling** never learns who is asking. `syneroym_roym_core::envelope::Request` carries no caller field — a sibling cannot verify a claim forwarded in the payload, and no host-attested channel threads the caller past `web`'s own dispatch. C4/C5 need this and inherit the gap (deferred-backlog, targeted at M6's cross-service caller-identity spec). The native build installs no instance certificates and no per-service admission control (permitted differences, carried from C1). WebCrypto Ed25519 has no JS fallback for older browsers. `local` login is a `roymctl`/config convenience, not a browser method.

### 4. Answers to the plan's open items

- **C1.1 §11 question 13** (what identity a service receives): resolved in code. The router's `resolve_effective_session_caller` (`crates/router/src/route_handler/http.rs`) verifies the `syneroym_session` cookie / `Authorization: Bearer` for gateway-origin requests and produces `CallerContext { caller_did: person_did, auth: Delegated }`, surfaced to a guest as `CallerIdentity { did: person_did, auth: CallerAuth::Delegated }`. So `D-C2-4`'s worry ("caller is the gateway node key") does not apply to the merged C1.1 — `web`'s `session.whoami` reads `request.caller` and gets the person.
- **`GET /` on a WASM-deployed Hub returned 500** — fixed in `crates/router/src/route_handler/io.rs`: an HTTP request with an empty interface routes to `http-native` when the service declares guest routes or an asset bundle *and* actually has an `http-native` channel (a deployed service does; a node-level native service such as the auth service does not, and keeps resolving through its own interface).

---

## C2 — Verification evidence

1. `cargo test -p syneroym-roym-core`: **5 passed, 0 failed**
2. `cargo test -p syneroym-roym-web --test dual_build_parity`: **10 passed, 0 failed**
3. `cargo test -p syneroym-substrate --test roym_app_e2e`: **2 passed, 0 failed**
4. `cargo test -p syneroym-auth`: **4 passed, 0 failed** (incl. the browser-signable challenge assertion)
5. `cargo test -p syneroym-substrate --test gateway_session_e2e`: **20 passed, 0 failed** (C1.1's suite, unchanged by C2)
6. `cd crates/roym_web/ui && npm test`: **9 passed** (incl. the z32 vector test against Rust output)
7. `cargo xtask check-roym-deps`: **clean**
8. `cargo test --workspace`: **0 failed** (run with the sandbox off — under the sandbox the substrate e2e tests fail on real binds / iroh and `cert_renewal_e2e` spins forever; this is an environment constraint, not the change)
9. `cargo +nightly fmt --all`: **clean**
10. `cargo clippy --workspace --all-targets --all-features`: **clean**
11. `cargo audit`: **clean** · `cargo deny check licenses`: **`licenses ok`**
12. `mise run test:e2e`: **23 passed (default config) + 4 passed (multi-hop)** — the four new `roym-hub.spec.ts` cases (delegated-key login, session persistence, card gallery, card safety) all pass


