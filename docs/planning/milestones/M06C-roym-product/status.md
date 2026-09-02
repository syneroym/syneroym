# M06C The Roym Product — Status

**Milestone:** [task.md](task.md) · **Designs of record:**
[slice-c1-implementation-plan.md](slice-c1-implementation-plan.md) (C1),
[slice-c1.1-implementation-plan.md](slice-c1.1-implementation-plan.md) (C1.1,
under [ADR-0024](../../../decisions/0024-client-gateway-identity-and-auth-service.md)),
[slice-c2-implementation-plan.md](slice-c2-implementation-plan.md) (C2)

**Overall:** Slices C1 (2026-08-25), C1.1 (2026-08-28), C2 (2026-08-29), C3 (2026-08-31), and C4 (2026-09-01) complete. C1.1, added by ADR-0024, makes the client gateway a dumb proxy with an `identity_mode` and moves the person session onto a node auth service; C2 builds the six-service Roym SynApp skeleton and the Hub shell on top of that model; C3 provides the host record-signing capability interface (`syneroym:signing`), canonical JSON record envelope format, verification, and tri-state revocation checking; C4 gives `profile` real product state (profile, contacts, block, report, contact rate limits), an owner-only authorization gate on `web`, the certificate lifecycle C3 required as a hard prerequisite, and an encrypted identity backup/restore.

---

## Slice status

| Slice | Scope | Status | Gate |
|---|---|---|---|
| C1 | Complete the dual-build shim (Gap 2, D-06C-10) | **Complete (2026-08-25)** — [implementation plan](slice-c1-implementation-plan.md), evidence below | None — independently mergeable |
| C1.1 | The node auth service and the dumb client gateway (ADR-0024) | **Complete (2026-08-28)** — [implementation plan](slice-c1.1-implementation-plan.md), evidence below | C1 |
| C2 | The SynApp skeleton and the Hub shell | **Complete (2026-08-29)** — [implementation plan](slice-c2-implementation-plan.md), evidence below | C1.1 |
| C3 | Signed records: host signing interface and envelope | **Complete (2026-08-31)** — [implementation plan](slice-c3-implementation-plan.md), evidence below | C1.1 |
| C4 | Identity, profile, contacts, and safety (R1 rows 1 and 6) | **Complete (2026-09-01)** — [implementation plan](slice-c4-implementation-plan.md), evidence below | C3 |
| C5 | Catalog and conversation in the product (R1 rows 2 and 3) | **In progress** — Rust core landed on `feat/m06c-slice-c5` (plan steps 1–9); cross-build parity, two-substrate e2e, Hub UI, and doc/backlog updates (steps 10–14) outstanding | C4 |
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
7. **Guest wall-clocks are not synchronized (C4):** the two builds each read a real wall clock that has moved between runs. The parity suite compares the host-stamped signed envelope byte-for-byte and every other artifact through `strip_volatile()`.
8. **`owner_did` must be set for the native build (C4):** a natively linked Roym has no deploy record, so `owner_of` is `None` unless `[roles.roym] owner_did` is configured; without it every `Owner`-classified method answers `-32012`. The parity harness sets it explicitly on both stacks.
9. **The native-dispatch privileged-capability gate has no native counterpart (C4):** a natively linked service has no `SynSvcNativeService`, so there is no external native-dispatch path into its `signing` / `vault` for the gate to close. Both builds' guest path reaches `HostState` directly.

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

---

## C3 — What shipped

Slice C3 implements the host record signing interface (`syneroym:signing`), canonical record envelope JSON encoding/verification, and tri-state revocation checking.

### 1. Identity & Record Envelope Core (`crates/identity`, `crates/signed_record`)
- Added `SCOPE_RECORD_SIGNING = "record-signing"` scope constant in `syneroym-identity`.
- Created new `syneroym-signed-record` crate with canonical JSON draft/envelope serialization, z32 SHA256 record ID computation, Ed25519 signature verification, delegation certificate scope/expiry validation, and tri-state revocation checking (26 unit tests).

### 2. Host Interface & Sandbox (`crates/wit_interfaces`, `crates/app_host`, `crates/sandbox_wasm`, `crates/app_host_native`)
- Added WIT interface `syneroym:signing/signing@0.1.0` in `crates/wit_interfaces/wit/signing/signing.wit`.
- Generated typed host/guest bindings in `crates/wit_interfaces/src/signing.rs` and `signing_host.rs` behind feature `"signing"`.
- Defined `AppSigning` trait in `crates/app_host`.
- Integrated `signing::Host` in `crates/sandbox_wasm` and `AppSigning` on `NativeAppHost` in `crates/app_host_native`.

### 3. Control Plane Dispatch & Substrate Wiring (`crates/control_plane`, `crates/substrate`)
- Threaded `NodeRecordSigner` into `crates/control_plane`'s JSON-RPC dispatch, supporting `signing/identity` and `signing/sign-record`.
- Wired `NodeRecordSigner` into `syneroym-substrate` runtime startup and dual-build fixture initializers.

### 4. Roym Integration, SDK & CLI (`crates/roym_core`, `crates/sdk`, `apps/roymctl`)
- Added `RECORD_TYPES` table and `is_known_record` in `crates/roym_core/src/record.rs`.
- Added `"syneroym-signed-record"` to `xtask` allowed target-independent dependencies allowlist.
- Added `signing_identity` and `certify_record_signing` in `crates/sdk`.
- Added `roymctl identity certify-signing` CLI command.

---

## C3 — Verification evidence

1. `cargo test -p syneroym-signed-record`: **27 passed, 0 failed**
2. `cargo test -p syneroym-identity`: **50 passed, 0 failed**
3. `cargo test -p syneroym-app-host-native --test dual_build_parity`: **38 passed, 0 failed**
4. `cargo test -p syneroym-substrate --test record_signing_e2e`: **1 passed, 0 failed**
5. `cargo check -p roymctl`: **Clean**
6. `cargo xtask check-roym-deps`: **Clean**
7. `cargo +nightly fmt --all`: **Clean**
8. `cargo clippy --workspace --all-targets --all-features`: **Clean**
9. `cargo test --workspace`: **All tests passed cleanly**
10. `cargo audit`: **Clean (0 vulnerabilities)**
11. `cargo deny check licenses`: **Clean (`licenses ok`)**
12. `mise run test:e2e`: **27 passed (clean — 23 single-node + 4 multi-hop)**

---

## C4 — What shipped

Slice C4 turns the Roym `profile` service from a `ping`-only stub into the product's identity, profile, contacts, and safety surface; adds an owner-only authorization gate to `web`; delivers the record-signing certificate lifecycle C3 named as its hard prerequisite; and adds an encrypted, transportable identity backup.

### 1. Content hashing and certificate primitives (`crates/signed_record`)
- Exported `content_digest(prefix, value)` — z-base-32 SHA-256 over key-sorted canonical JSON — as the single content-hash definition. `Envelope::record_id` is refactored to call it; its existing stability tests pass unchanged.
- Re-exported `DelegationCertificate` and `canonicalize_json_value` so a guest checks a certificate and hashes a document with the same code the host runs. The `wasm32-wasip2` build still links (nothing here can produce a signature).

### 2. `roym_core` product primitives (`crates/roym_core`)
- New modules: `clock` (the one wall-clock read, passed down as `now_secs: u64`), `person` (`ProfilePayload` v1 and `is_did_key`, closing Gap 5's person→conversation-address mapping on the product side), `safety` (`admit_first_contact` / `admit_publication` / `ContactLimits` / `PublicationLimits` as pure arithmetic, fully unit-tested), `backup` (`Bundle` / `BundleManifest` / `SectionDigest` with per-section content digests and `check_integrity`), and `signing` (the per-service certificate store, `CertificateStatus`, `person_principal`, `owner_did`, and the shared `handle_certificate_verb`).
- `record.rs` corrected to the `(type, version)` shape mirroring `card::CARD_TYPES`, `profile` added as the tenth record type, and the planning-identifier doc comment removed.
- `router.rs` gains a `MethodAuth` column (`Public` / `Owner`, no default arm) beside the routing table, plus `method_auth()` and `PUBLIC_METHODS` (`profile.policy`).

### 3. `roym_profile` — the product surface (`crates/roym_profile`)
- Eight collections created idempotently on first use; `SCHEMA_VERSION` 1 → 2.
- Verb families: `profile.*` (`get`, `set` — the one flow that signs a `profile` record and supersedes the prior one — `policy`, `export`, `import`, `signing-status`, `install-signing-certificate`, `ping`), `contacts.*` (`list`, `get`, `upsert` with an optional verified `profile_envelope`, `remove`, `resolve-address`, `admit-first-contact`, `limits`, `set-limits`), `block.*` (`add`, `remove`, `list`, `check`), `report.*` (`create` with a content-derived `report_id`, `list`, `get`, `withdraw`).

### 4. `roym_web` — the authorization gate (`crates/roym_web`)
- One `admit()` helper on both the HTTP `rpc` path and the proxied `invoke` path: `Owner` methods require a verified delegated session whose subject equals `AppSigning::signing_identity().owner_did`. Codes `-32010` (not signed in), `-32011` (not the owner), `-32012` (no recorded owner). `session.whoami` and `GET /health` stay reachable with no session.

### 5. Encrypted identity backup (`crates/identity`)
- New non-default `backup` feature (enabled by `roymctl` only, keeps `aes-gcm` out of the `wasm32-wasip2` build). `backup.rs`: HKDF-SHA256 → AES-256-GCM under a randomly generated 32-byte recovery key, AAD binding the DID and header, and a DID-derivation check on import.

### 6. `roymctl` (`apps/roymctl`)
- `identity export` / `identity import` (recovery-key-based, refuses to overwrite an existing key file).
- New `roym` command group: `roym enrol-signing` (asks the service for its own signing identity per `D-C4-5`, mints a `record-signing` delegation against it with the deployer's master key, and installs it) and `roym signing-status`.

### 7. External-caller gate on native dispatch (`crates/control_plane`)
- `admit_privileged_capability()` on `SynSvcNativeService`: `signing/sign-record` and `vault/reveal` are admitted only for the service's own `system:<service_id>` identity or its recorded owner. `signing/identity` stays open (public identifiers only). `record_signing_e2e.rs` step 8's assertion inverts from "succeeds" to "refused".

### 8. The Hub (`crates/roym_web/ui`)
- New `rpc.ts` (one `call()` mapping `-3201x` onto typed errors) and five screens (`setup`, `profile`, `contacts`, `safety`, `backup`). `main.ts` becomes a three-state shell: not signed in → login; signed in but not enrolled → setup; signed in and enrolled → tab bar, with the card gallery behind a "Components" tab.

### 9. What C4 did **not** close
- **R1 row 6 (a blocked sender never reaches the inbox):** C4 ships the block list, the decision function `contacts.admit-first-contact`, and the `D-06C-8` wording. The enforcement point is Roym's own inbox, which does not exist until C5. Row 6's acceptance gate closes in C5.
- **R1 row 1 (restore reproduces identity *and history*):** C4 proves restore for the sections it owns (`profile`, `contacts`, `blocks`, `reports`). Conversation history is C5's; the bundle format is fixed here so C5 adds sections rather than reshaping it.
- **The publication-limit half of `[PRD-SAF]`:** `safety::admit_publication` ships as a pure rule with no caller. C5 (catalog) and C6 (directory) call it.
- **A browser-only path to signing enrolment:** a delegate cannot re-delegate, so enrolment needs the deployer's master key on a shell. New backlog row.
- **Wire-side authorization on `catalog` / `conversation` / `directory`'s `api.invoke`:** the gate lives in `web` and covers only its ingress. New backlog row targeted at C5.

### 10. Permitted differences added to §14 (WASM vs native)
7. **Guest clocks are not synchronized** between the two builds — a property of wall clocks, not the shim. The parity suite compares the signed envelope (host-stamped, pinnable) byte-for-byte and every other artifact through `strip_volatile()`.
8. **`owner_did` must be set for the native build.** A natively linked Roym has no deploy record; without `[roles.roym] owner_did` every `Owner` method answers `-32012`. The parity harness sets it explicitly on both stacks.
9. **§7's native-dispatch gate has no native-build counterpart** and that is not a divergence: a natively linked service has no `SynSvcNativeService`, so there was never an external native-dispatch path into its `signing` / `vault` to close.

---

## C4 — Verification evidence

1. `cargo test -p syneroym-roym-core`: **28 passed, 0 failed**
2. `cargo test -p syneroym-signed-record`: **27 passed, 0 failed**
3. `cargo test -p syneroym-identity --all-features`: **56 passed, 0 failed**
4. `cargo test -p syneroym-roym-web --test dual_build_parity`: **36 passed, 0 failed** (35 scenarios: 10 from C1–C3, 25 new for C4 — certificate lifecycle, `profile.set` byte-identity, contacts, block, report, export/import, and the `web` authorization gate on both paths)
5. `cargo test -p syneroym-app-host-native --test dual_build_parity`: **36 passed, 0 failed**
6. `cargo test -p syneroym-substrate --test roym_identity_e2e`: **1 passed, 0 failed** — one 12-step scenario against a live substrate and gateway: the real enrolment ceremony; `profile.set` refused before enrolment then producing a verifying envelope after; owner-only refusal against two real sessions; `stranger` refused at `install-signing-certificate` and at `signing/sign-record`; export/import round-trip; `identity export` → `import` into a fresh directory → enrol from the restored key alone; and an operator-key mint attempt refused.
7. `cargo test -p syneroym-substrate --test roym_app_e2e`: **passed, 0 failed** (harness change only — person and deployer are now the same DID)
8. `cargo test -p syneroym-substrate --test record_signing_e2e`: **1 passed, 0 failed** (step 8 inverted)
9. `cargo xtask check-roym-deps`: **Clean**
10. Planning-identifier grep over `crates/roym_*`, `crates/roym_core/app/`, the new `roym_core` / `identity` / `roymctl` files, and every line C4 added elsewhere: **no `M0[0-9]`, `\bR[1-4]\b`, `\bC[0-9]`, `D-C[0-9]`, `D-0[0-9]`, or `Slice ` in any name or comment**. `F13`'s two existing slips (`record.rs`, `roym.toml`) are fixed. (Pre-existing planning references in `synsvc_native.rs` comments from earlier milestones are untouched and out of C4's scope.)
11. `cargo +nightly fmt --all`: **Clean**
12. `cargo clippy --workspace --all-targets --all-features`: **Clean**
13. `cargo test --workspace`: **2231 passed, 0 failed on C4-related crates.** One pre-existing flake in `syneroym-substrate::scheduled_task_e2e::a_supervisor_restart_skips_the_ticks_it_missed` (iroh network-path abandonment, unrelated to C4) — **passes in isolation** (`finished in 141.64s`).
14. `cargo audit`: **Clean (0 vulnerabilities)**
15. `cargo deny check licenses`: **Clean (`licenses ok`)**
16. `mise run test:e2e`: **27 passed (clean)** — includes all 8 `roym-hub.spec.ts` C4 browser cases (delegated login, card gallery + safety, profile save showing `rec_…`, contact add, block, app-data bundle export).

---

## C5 — In progress (Rust core, plan steps 1–9)

Slice C5's Rust side is implemented on `feat/m06c-slice-c5`. The 37
cross-build parity scenarios, the two-substrate e2e, the Hub UI, and the
document/backlog updates ([plan](slice-c5-implementation-plan.md) steps
10–14) are a second work order.

### What landed

1. **`syneroym:invocation` — a new host interface** (plan §3). One
   function tells a component whether the call it is handling arrived
   from inside this node (`internal`), from a verified party over the
   wire (`verified(did)`), or from the wire with no identity
   (`anonymous`). A local dispatch is `internal` whatever identity the
   caller carries; the auth level is read only on the wire path. Threaded
   through `HostState` by a second engine entry point
   (`execute_wasm_json_from_wire`) so the wire path is greppable and no
   existing call site changes; `dispatch.rs`'s one wire site switches.
   The native shim carries the origin by which factory constructor built
   the host (`host_for` vs `host_for_wire`). `AppHost`'s supertrait list
   grows from nine to ten.
2. **`admit::require_internal`** (plan §4.1, §7, `D-C5-3`) — the single
   admission rule, called as the first statement of every service's
   `invoke`. A method reached over the wire (not through a local
   dispatch) answers `-32013`. `api.status` is the one export
   deliberately left open. `web`'s own `MethodAuth` owner-session gate on
   `POST /rpc` is unchanged and is still what protects the product from
   the browser side. `D-C5-4`: no manifest `visibility` value changes —
   with `require_internal` in force, visibility is a discoverability
   choice, not an authorization control.
3. **`roym_core` vocabulary** (plan §4): `listing` (one record type, a
   required core plus seven optional named blocks, `ListingPayload::
   validate` with a per-rule error set, `derive_listing_id`
   content-derived from issuer+slug, `slug_from_title`); `area`
   (micro-degree Bbox/Circle/Named, an over-covering circle→box
   projection, all integers — `D-C5-6`); `conversation`
   (`MessageRow`/`ConversationRow`/`StoredState` for Roym's own copy, the
   `(sender-timestamp, author, id)` sort key, the reserved
   `application/vnd.roym.deletion-request+json` content type with a
   strict parser, a std-only `encode_body`); four new `backup` section
   names; `RECORD_LISTING`; a `catalog.` routing prefix.
4. **`roym_catalog`** (SCHEMA_VERSION 1 → 2, plan §5): `listing.set`
   signs a `listing` record with a stable content-derived id, supersedes
   the prior version, and is counted by `safety::admit_publication`; the
   conversation address is filled from the person's own `profile` record
   when omitted, through the declared `catalog → profile` edge.
   `listing.withdraw` runs the same flow with `status = withdrawn` and
   consumes no publication budget (`D-C5-19`). `listing.verify` checks a
   stranger's envelope locally and answers inside a success envelope.
   `availability.*` stores clock-free slots. `catalog.export` /
   `catalog.import` round-trip listings and availability with the schema
   version preserved and every listing envelope re-verified.
   `handle_certificate_verb` is mounted (`D-C5-18`).
5. **`roym_conversation`** (SCHEMA_VERSION 1 → 2, plan §6): the world
   exports `guest-api`; the guest and native sinks route `on-message` /
   `on-delivery-state` into one target-independent inbox. `on_message`
   enforces the block list on every message and the first-contact rate
   limit only for a peer this node holds no conversation with, records a
   refused message in a separate bodiless `refused_messages` collection
   (`D-C5-11`), drops a non-direct conversation as `unsupported-kind`,
   and honours an inbound deletion request only for a message its sender
   authored here (`D-C5-20`). Verbs: `conversation.open/list/send/
   history/delivery-status/outbox/retry/delete-message/search/export/
   import` plus the certificate verbs. `send` stores `pending` from the
   host's own return value, never optimistically (`D-C5-10`); `history`
   re-reads delivery status for every non-`delivered` row;
   `delete-message` tombstones the local row, keeps it as the deletion
   record, and asks the peer for an outgoing message; `search` is an
   escaped `$regex` over `utf8` bodies (`F9`).
6. **Manifest** (plan §8, `D-C5-9`): `conversation` and `catalog` each
   declare `depends_on = ["profile"]`. `init_roym` persists the two
   bindings and registers the native conversation inbox sink.
7. **`roymctl`** (plan §9): `roym enrol-signing` and `roym
   signing-status` now cover `profile`, `catalog` and `conversation` —
   one certificate per service — and exit non-zero if any service fails.
   `roym address` is deferred to the second work order.

### Deviations from the plan

- **§6.2** says roym `src/bindings.rs` is checked in. In the tree it is
  gitignored for every roym crate and regenerated by `cargo component` /
  `mise run build:roym`; followed the tree.
- **§4.4 / §6** non-text message bodies: added a std-only `encode_body`
  (base64) to `roym_core::conversation` rather than a `base64` crate
  dependency, which `cargo xtask check-roym-deps` forbids on a roym
  service crate. Attachments are out of scope for the first release, so
  every stored body takes the `Utf8` path today.

### C5 — Verification evidence (steps 1–9)

1. `cargo test -p syneroym-roym-core`: **63 passed, 0 failed** — `admit`,
   `area`, `listing`, `conversation`, `backup` and `router` unit tests.
2. `cargo test -p syneroym-app-host-native --test dual_build_parity`:
   **39 passed, 0 failed** — includes the new
   `caller_origin_is_identical_on_both_builds` (local `internal` / wire
   `verified` / wire `anonymous`) and a `caller-origin` scenario in the
   byte-comparison table.
3. `cargo test -p syneroym-sandbox-wasm --lib
   invocation_caller_origin_mapping`: **1 passed** — the origin mapping
   across all five `AuthLevel` arms, local and wire.
4. `cargo test -p syneroym-roym-web --test dual_build_parity`: **37
   passed, 0 failed** (existing scenarios only; the 37 C5 scenarios are
   the second work order — scenario 8's `schema_version` map is bumped).
5. `cargo test -p roymctl`: **74 + 17 passed, 0 failed**.
6. `cargo xtask check-roym-deps`: **Clean**.
7. `cargo +nightly fmt --all`: **Clean**.
8. `cargo clippy --workspace --all-targets --all-features`: **Clean**.
9. `cargo test --workspace`: **exit 0, 0 failures** across ~150 test
   binaries (2026-09-02, sandbox off).
10. `cargo audit`: **Clean (0 vulnerabilities)**.
11. `cargo deny check licenses`: **Clean (`licenses ok`)**.
12. `mise run test:e2e`: **not yet run** — the browser cases are part of
    the second work order.

