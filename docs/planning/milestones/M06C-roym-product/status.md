# M06C The Roym Product — Status

**Milestone:** [task.md](task.md) · **Designs of record:**
[slice-c1-implementation-plan.md](slice-c1-implementation-plan.md) (C1),
[slice-c1.1-implementation-plan.md](slice-c1.1-implementation-plan.md) (C1.1,
under [ADR-0024](../../../decisions/0024-client-gateway-identity-and-auth-service.md)),
[slice-c2-implementation-plan.md](slice-c2-implementation-plan.md) (C2)

**Overall:** Slices C1 (2026-08-25), C1.1 (2026-08-28), C2 (2026-08-29), C3 (2026-08-31), C4 (2026-09-01), C5 (2026-09-03), and **C6 (2026-09-05, partial — see its own section)** complete or landed. C1.1, added by ADR-0024, makes the client gateway a dumb proxy with an `identity_mode` and moves the person session onto a node auth service; C2 builds the six-service Roym SynApp skeleton and the Hub shell on top of that model; C3 provides the host record-signing capability interface (`syneroym:signing`), canonical JSON record envelope format, verification, and tri-state revocation checking; C4 gives `profile` real product state (profile, contacts, block, report, contact rate limits), an owner-only authorization gate on `web`, the certificate lifecycle C3 required as a hard prerequisite, and an encrypted identity backup/restore; C5 adds the versioned signed listing schema (`catalog`), Roym's own copy of every message plus a block-enforcing inbox (`conversation`), the `syneroym:invocation` host interface with a local-only admission rule on every service, and the two `depends_on` edges those callers traverse; C6 adds the `directory` service's server and client halves (SynOrg settings/roster, provider-initiated publication, search over a derived projection, and a consumer's own directory list/fan-out/merge), the first wire-reachable Roym verbs, and the directory-side publication limiter that closes `[PRD-SAF]` — with the full Hub UI, the three-substrate e2e, and part of the plan's own 51-scenario parity matrix explicitly not built in this pass (see C6's "What C6 did not build" below).

---

## Slice status

| Slice | Scope | Status | Gate |
|---|---|---|---|
| C1 | Complete the dual-build shim (Gap 2, D-06C-10) | **Complete (2026-08-25)** — [implementation plan](slice-c1-implementation-plan.md), evidence below | None — independently mergeable |
| C1.1 | The node auth service and the dumb client gateway (ADR-0024) | **Complete (2026-08-28)** — [implementation plan](slice-c1.1-implementation-plan.md), evidence below | C1 |
| C2 | The SynApp skeleton and the Hub shell | **Complete (2026-08-29)** — [implementation plan](slice-c2-implementation-plan.md), evidence below | C1.1 |
| C3 | Signed records: host signing interface and envelope | **Complete (2026-08-31)** — [implementation plan](slice-c3-implementation-plan.md), evidence below | C1.1 |
| C4 | Identity, profile, contacts, and safety (R1 rows 1 and 6) | **Complete (2026-09-01)** — [implementation plan](slice-c4-implementation-plan.md), evidence below | C3 |
| C5 | Catalog and conversation in the product (R1 rows 2 and 3) | **Complete (2026-09-03)** — [implementation plan](slice-c5-implementation-plan.md), evidence below | C4 |
| C6 | Directory: the search half (R1 row 5) | **Partial (2026-09-05)** — core service, admission rule, roymctl, and 34 parity scenarios shipped; Hub UI and the three-substrate e2e were not built. See its own section and "What C6 did not build" below | C5 |
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
10. **The wire origin has no production producer on the native build, and the answer for a local call is identical on both (C5):** a natively linked Roym service is registered only in the local endpoint registry, is never published, and an inbound stream naming `did:key:roym-*` fails the handshake because no private key exists for it — so `NativeHostFactory::host_for` (meaning *local*) is the only origin anything in the substrate produces. `host_for_wire` exists, is unit-tested, and its only caller is the parity harness's wire driver, which is what makes scenarios 67–71 a real two-build comparison. **Not permitted:** any difference on the *local* path — both builds answer `internal` for a local call whatever the caller's auth level, because the parity driver hands a verified delegated caller to a purely local drive on both stacks; an auth-reading native mapping would have failed every existing native scenario with `-32013`. Scenario 70 is that regression guard.
11. **The inbound conversation notification mechanism differs and the store contents do not (C5):** WASM instantiates the component and calls its `guest-api` export with a 4-attempt retry; the native build calls a `Weak<dyn ConversationSink>` once, with no retry — B3's own precedent for `MessageSink`, restated because C5 is the first slice where a Roym service is on the receiving end. The parity suite compares the `messages` and `refused_messages` rows afterwards, never the timing.
12. **Guest wall-clocks stay unsynchronized (item 7, extended by C5):** §13 of the C5 plan lists six volatile fields (`stored_at_secs`, `opened_at_secs`, `updated_at_secs`, `deleted_at_secs`, `last_activity_ms`, `sender_timestamp_ms`) rather than C4's four, and the host `message-id` is normalized positionally in `sort_key` order rather than stripped — stripping would stop the suite noticing two messages merged into one row. The signed `listing` envelope stays compared byte-for-byte (its timestamp is the pinned `RecordClock`).

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
10. **Guest-HTTP / websocket invocation origin** (added in the C5 review follow-up). The WASM build now reports `verified`/`anonymous` for a guest-HTTP or websocket request off the wire (`InstanceOptions::from_wire()`); the native shim's `HttpSink`/`WebSocketSink` are still built from `host_for` and report `internal`. Unobservable today — `web`'s `incoming-handler` gates on its session cookie, not the invocation origin. Tracked as a backlog row (§3), targeted C6, and asserted WASM-side by `a_guest_http_request_reports_a_wire_origin_on_the_wasm_build`.

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

## C5 — What shipped

Slice C5 turns `catalog` and `conversation` from `ping`-only stubs into
the product's offer and inbox surfaces, adds the `syneroym:invocation`
host interface and a local-only admission rule on every Roym service, and
gives the person a signed versioned listing schema plus Roym's own
searchable, exportable, deletable copy of every message. Delivered in
four work orders on `feat/m06c-slice-c5`: WO1 (Rust core, plan steps
1–9), WO2 (37 cross-build parity scenarios, step 10), WO3 (the
two-substrate e2e and the Hub, steps 11–12), WO4 (`roym address`, the
full gate, this section and the backlog, steps 13–14).

### The new app-facing surface

`syneroym:invocation@0.1.0` is a **new host interface** — the tenth
`AppHost` supertrait, and the only host surface outside inbound HTTP that
says anything about who is calling. It is additive and is **not** in the
default `host-environment` world: a component that does not import it
deploys exactly as before. It is the capability the milestone preamble
means by "where a capability is genuinely missing, this document names it
as a gap and gives it a slice" — without it, `conversation.history` is
readable by anyone holding an address the product hands out on purpose.
No new ADR (plan §15): it adds no wire format and changes no record
envelope.

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
   host's own return value, never optimistically (`D-C5-10`);
   `delete-message` tombstones the local row, keeps it as the deletion
   record, and asks the peer for an outgoing message; `search` is an
   escaped `$regex` over `utf8` bodies (`F9`).
6. **Manifest** (plan §8, `D-C5-9`): `conversation` and `catalog` each
   declare `depends_on = ["profile"]`. `init_roym` persists the two
   bindings and registers the native conversation inbox sink.
7. **`roymctl`** (plan §9): `roym enrol-signing` and `roym
   signing-status` cover `profile`, `catalog` and `conversation` — one
   certificate per service — and exit non-zero if any service fails.
   `roym address` (WO4) reads `svc list` and prints this installation's
   own Roym Conversation service id (to paste into `profile.set` as
   `conversation_address`) and the Hub's gateway host. It invents no
   resolution path; `F14` is why it exists.
8. **Cross-build parity** (plan §11.2, WO2): 37 scenarios (37–73) on
   `crates/roym_web/tests/dual_build_parity.rs`, each asserting on a
   value only the verb's own handler produces. Scenario 73 is the guard
   that fails if any verb named in 37–72 answers `-32601` or `-32013`
   through the local path; scenario 70 is `F17`'s regression guard (a
   local call carrying a verified delegated caller is admitted on both
   builds). Harness changes: `conversation` is bound on both stacks,
   scenario 5's unbound dependency moves to `transaction` (`D-C5-12`),
   the native `conversation` factory gets its sink and the WASM engine
   its notifier, a wire driver (`execute_wasm_json_from_wire` /
   `host_for_wire`) proves the `-32013` refusal on both builds,
   `normalize_message_ids` rewrites the non-reproducible host message id
   positionally, and `strip_volatile` drops the six wall-clock row
   fields.
9. **Two-substrate e2e** (plan §11.3, WO3):
   `crates/substrate/tests/roym_conversation_e2e.rs` runs the reference
   scenario over two independent substrates, each running the full Roym
   SynApp under its own owner — `profile.set` carrying each side's
   conversation address, a contact imported from a verified profile
   envelope, `conversation.open` resolved through contacts, a message
   `pending` from the host's own answer then `delivered` with the same id
   on both sides, a blocked sender's message that reaches no conversation
   and no search result, a signed listing verified with no directory
   anywhere, conversation and catalog export/import round-trips
   (including a tampered-bundle refusal), and a deletion request the peer
   honours only for a message its sender authored.
10. **The Hub** (plan §10, WO3): Messages and Listings tabs on the
    three-state shell, plus the Safety report form and first-contact
    limit editor and a Backup tab showing the three service bundles
    separately. Every stranger-influenced value is a text node.
    `rpc.ts` maps `-32013` to a `NotLocal` error rendered as *"this
    installation refused a request that did not come from you"*.

### Failure-and-security matrix rows C5 closes

| Row | How |
|---|---|
| **1** (a forged or absent listing signature) | `listing.verify` returns `verified: false` with a reason; parity 47; the Hub renders it as unknown, never as trusted. |
| **3** (no Directory anywhere) | The R1 half: e2e step 11 completes the find-and-engage path by direct link with no directory deployed. R2's half stays C8's. |
| **11** (a blocked sender) | Fully, for R1 row 6: parity 57/58 and e2e step 10 — never in a conversation, never in a search, never counted, and the product does not claim the sender was prevented. |
| **12** (flooding) | The publication half now has a caller (parity 43) with a stated exemption for withdrawal (parity 44); the contact half is C4's, re-exercised at the inbox by parity 59. |
| **13** (import reproduces what was exported) | For the conversation sections (parity 63–65, e2e step 12) **and the catalog sections** (parity 49–51, e2e step 13) — the latter is R1 row 2's own acceptance test. |
| **16-adjacent** (a message that never settles) | e2e step 15 (single-node): `pending` while the window is open, `failed` after it, with the host's own reason; parity 56 covers `failed` without waiting on a clock. |
| **17** (restart mid-flow) | e2e steps 6/9, run as a **single-node `#[ignore]`d test** (see below). |
| **19** (build divergence) | 37 new parity scenarios, including 67/68 (the wire refusal on both builds) and 70 (`F17`'s guard). |

**A row C5 explicitly does *not* close: matrix row 12's "refusal is
visible to the sender" for an *inbound* refusal.** The host stores and
acknowledges an inbound message before Roym's `on_message` runs, so there
is nothing left to refuse to the sender; §6.3 of the plan says why, and
`deferred-backlog.md` §5's "no inbound admit/reject hook" row is where it
lives. C5 states this rather than quietly claiming the row.

Three backlog rows C5 **restates rather than resolves**: the
native-subscription-replay row (§5 — Roym subscribes to no messaging
topic even now, so C5 supplies no consumer to close it against), the
`depends_on`-not-enforced row (§3 — C5 declares the two edges it
traverses but the enforcing binding check is still absent), and the
publication half of `[PRD-SAF]` (§10 — the catalog-side caller ships, the
directory-side one stays C6's).

### `D-C5-4` — no manifest `visibility` value changed

With `admit::require_internal` in force on every service, `visibility` is
a discoverability choice, not an authorization control. No service was
narrowed or widened. This is said out loud so a reader does not conclude
from a diff that a manifest field is protecting the API — the origin
check is.

### Deviations from the plan

- **§6.2** says roym `src/bindings.rs` is checked in. In the tree it is
  gitignored for every roym crate and regenerated by `cargo component` /
  `mise run build:roym`; followed the tree.
- **§4.4 / §6** non-text message bodies: added a std-only `encode_body`
  (base64) to `roym_core::conversation` rather than a `base64` crate
  dependency, which `cargo xtask check-roym-deps` forbids on a roym
  service crate. Attachments are out of scope for the first release, so
  every stored body takes the `Utf8` path today.
- **WO2 — the pinned `RecordClock`.** Plan §13 pins the signing clock to
  `Fixed(F)`. `Fixed(2_000_000_000)` (year 2033) sat years past the
  5-year certificate ceiling, so no delegated signature could clear both
  its certificate window and the verifier's 300 s skew bound, and no
  listing could be signed or verified. The finding: a `Fixed` clock in
  the parity harness must sit a small step *ahead* of wall-now, not years
  ahead. Fixed there; the signed `listing` envelope is still compared
  byte-for-byte.
- **WO2 — the parity scenario table.** Two departures from plan §11.2's
  table, recorded in WO2's notes: scenario 8's `expected_schema_version`
  map now carries `catalog` and `conversation` at 2 alongside `profile`
  (planned, but the map was the change); and the `messages` collection is
  created on first use inside `put_message` (mirroring `load_message`) —
  the inbox store path reached it without one and every delivered message
  was lost to `CollectionNotFound` until this landed.
- **WO2 / D-C5-10 — `conversation.history` reconciliation.** As shipped it
  read the host only for `pending` rows. The review (C5-3) noted this
  leaves `failed → delivered` unreachable when the notification is missed
  (a retry that succeeds while the substrate is down), so a delivered
  message can read `failed` forever. Now every row that is not yet
  `delivered` and not deleted is re-read, but a `failed` row moves *only*
  to `delivered` — never back to `pending` — keeping the anti-regression
  property the original narrowing protected. Still bounded by messages not
  yet delivered.
- **WO3 — the two-substrate e2e carries the reference scenario *without*
  a substrate restart.** `roym_conversation_e2e.rs` runs reference steps
  1–5, 7–8 and 10–14. Step 15 (a message settles `failed` with the host's
  reason) is its own single-node test. Steps 6/9 (a `pending` message and
  its body survive a substrate restart) were a single-node test,
  `a_pending_message_and_its_body_survive_a_substrate_restart`, marked
  `#[ignore]` while the redeploy after a restart deduped into a no-op and
  left `POST /rpc` unrouted. **Partly resolved in the review follow-up**
  (see below): the dedup now has a `full_deploy_completed` guard so the
  redeploy on `resume()` does real work again and the test is
  un-`#[ignore]`d — but a bare substrate restart with *no* redeploy still
  leaves the route tables empty (backlog §8, boot-time rehydration). The
  export/import steps (12, 13) re-import against the running substrate; the
  wipe-and-restore variant is parity 49–51 / 63–65.
- **WO3 additions not in the plan's verb tables.** `listing.list` now
  returns `title` (parsed from the stored signed envelope; a row whose
  envelope will not parse lists with an empty title), which the Hub's
  Listings tab shows. `roym_conversation`'s `on_delivery_state` and
  `history`'s reconciliation read the host's failure reason back from the
  outbox — the WIT `delivery-state` enum carries no reason, so a message
  that settled `failed` otherwise showed no `last_error` in Roym's copy.
  Both are in `roym-integrated-experience-spec.md`'s Catalog and
  Conversation API columns now.
- **WO3 — `roym-hub.spec.ts`.** Plan §11.4 lists six browser intents;
  they ship as five tests — the "pending message shows pending" and the
  "delete dialog wording" cases share the open + send setup and were
  merged into one.

### C5 — Review follow-up (2026-09-04)

A code review of the slice raised 19 findings. Incorporated:

- **Blocker (partial) — `POST /rpc` unrouted after a substrate restart.**
  Root cause was *not* `maybe_rewrite_http_native_interface` (route
  resolution reads `http_routes` directly). The three route tables
  (`native_dispatch`, `http_routes`, `assets`) are process-local and start
  empty on every boot; the sandbox warm-up restores only the WASM
  instance. A redeploy after a restart then deduped — matching persisted
  `manifest_hash`, warmed `Running` instance — and never reached the table
  re-registration. Fixed with a `full_deploy_completed` guard on the dedup
  in `deploy_with_context`: a dedicated per-service map, written after
  every route-table write and cleared by `undeploy`, is the witness that
  *this* process ran the full deploy, so a fresh boot's redeploy always
  falls through. `a_pending_message_and_its_body_survive_a_substrate_
  restart` un-`#[ignore]`d (its `resume()` redeploys). Same fix clears the
  "Native service not found" symptom for native-capability calls after a
  restart. **Not covered:** a bare restart with no redeploy — nothing in
  `runtime.rs` re-applies a deployment plan on boot, so the route tables
  stay empty until some deploy runs. New backlog row (§8) for boot-time
  rehydration from persisted deploy facts; targeted at the M6 substrate
  work, where substrate-lifecycle design gaps belong.
- **C5-1 — wire ingress reported `internal`.** `handle_guest_http_request`,
  the raw-stream instance, and the three websocket handlers built their
  instance with `InvocationOrigin::Local`, so `invocation.caller()` would
  answer `internal` for a request off the wire — a trap for the first
  C6/C7 service that puts `require_internal` in an `incoming-handler`. All
  five now use `InstanceOptions::from_wire()`. Not exploitable in C5.
- **C5-2 — a transient inbox fault dropped the message.** `on_message` now
  returns `Err` on a storage/proxy fault (WASM retries, native warns) and
  `Ok` only for a deliberate refusal. The gap-reconciliation half (read
  `AppConversation::history` and backfill) is a new backlog row.
- **C5-3 — a delivered message could read `failed` forever** (above).
- **C5-4 / C5-5 — the shared sort key was not shared.** An outgoing row's
  `author` was a synthetic `self:<conv-id>` and its `sender_timestamp_ms`
  a locally recomputed whole-second value. Both are now taken from the
  host's own record of the message (`host_message`), which is what the
  peer stores, so both transcripts compute the same
  `(sender-timestamp, author, id)` order.
- **C5-6 — the listing editor assumed two decimal places for every
  currency.** `toMinorUnits` now scales by `currencyMinorExponent(code)`
  (the full exponent-0 and exponent-3 ISO-4217 sets; two otherwise), so a
  JPY price is no longer signed at 100× and a KWD price no longer 10×
  low.
- **C5-9(c) — publication row id from a counter.** Two concurrent
  `listing.set` calls read the same `version_count` and wrote the same
  `{listing_id}:{count}` publication id, so two versions cost one unit of
  the flood budget. The id is now `{listing_id}:{record_id}` (unique per
  signed version).
- **C5-10 — wire-refusal coverage was 2 of 6 services.** Parity scenarios
  67/68 now loop `WIRE_REFUSED_VERBS` over all six.
- **N-1** listing-history comment corrected; **N-5** `listing.set` with no
  `status` now carries the prior version's status forward instead of
  silently re-activating a withdrawn listing.

Deferred with a backlog row (§ links in `deferred-backlog.md`): C5-2's
history gap-reconciliation, C5-7 (read verbs scan the whole collection),
C5-8 (`catalog.export` omits `listing_history` / `publications` /
`settings`), C5-9(a)(b) (unfenced read-modify-write on counts), C5-11
(`publications` never pruned), N-2/3/4/6/7/8.

**Verification pass (V-1…V-4), same day:**

- **V-1** — the Hub UI bundle was stale when the first local verification
  ran (`cargo component build` was run directly, not `mise run
  build:roym`, so `build:roym-ui` never repacked `bundle.tar.gz`). Bundle
  rebuilt. `bundle.tar.gz` is gitignored and CI's `global-setup.ts` runs
  `npm run build && npm run pack` fresh, so nothing committed was wrong —
  but any local e2e result before the rebuild was against the old
  `toMinorUnits`.
- **V-2** — the blocker fix covers the *redeploy-after-restart* path only.
  A bare restart with no redeploy still leaves the route tables empty
  (nothing in `runtime.rs` re-applies a plan on boot). The "Recently
  resolved" backlog row was reworded to say so, and a new open §8 row
  covers boot-time rehydration, targeted at the M6 substrate spec. The
  WO3 deviation bullet above is corrected to "partly resolved".
- **V-3** — `send`'s fallback (host record absent) no longer restores the
  synthetic `self:` author; it reads this installation's real conversation
  address from `profile.get` (`own_conversation_address`). The comment is
  corrected. The timestamp fallback stays a local clock — the only way
  `host_message` is `None` for a just-sent row is an instantly-delivered
  synthetic peer, and there is no host timestamp to read in that case.
- **V-4** — the guard is `full_deploy_completed` in code; the four doc /
  comment sites that named `routes_registered_this_process` (a draft name)
  are fixed. Added `a_redeploy_in_a_fresh_process_reinstalls_even_when_the_
  manifest_is_unchanged` (a second `ControlPlaneService` over one storage
  dir) and `a_guest_http_request_reports_a_wire_origin_on_the_wasm_build`
  (drives `/origin` on the dual-build fixture).
- **V-5** — the Playwright `global-setup.ts` ran a plain `npm install`
  twice inside the suite's one `globalTimeout` (300 s); on a
  network-restricted host each took ~3 min doing registry round trips
  while reporting "up to date", exhausting the budget before the
  substrate started. Both call sites (and the multihop setup's one) now
  pass `--prefer-offline --no-audit --no-fund`, so a warm cache costs
  ~100 ms.
- Smaller: `on_message` gained a `load_message` idempotency guard so
  C5-2's retry cannot re-increment `message_count` on every attempt;
  parity scenario 69 was folded into 68; the C5-6 Rust-side home and the
  native guest-HTTP origin mismatch each got a backlog row.

### C5 — Verification evidence

1. `cargo test -p syneroym-roym-core`: **64 passed, 0 failed** — `admit`,
   `area`, `listing`, `conversation`, `backup` and `router` unit tests.
2. `cargo test -p syneroym-app-host-native --test dual_build_parity`:
   **39 passed, 0 failed** — includes
   `caller_origin_is_identical_on_both_builds` (local `internal` / wire
   `verified` / wire `anonymous`).
3. `cargo test -p syneroym-sandbox-wasm --lib
   invocation_caller_origin_mapping`: **1 passed** — the origin mapping
   across all five `AuthLevel` arms, local and wire.
4. `cargo test -p syneroym-roym-web --test dual_build_parity`: **74
   passed, 0 failed** — 37 pre-C5 scenarios plus the 37 C5 scenarios
   (37–73), identical on both builds, including guard scenario 73 and
   regression guard 70.
5. `cargo test -p syneroym-substrate --test roym_conversation_e2e`: **2
   passed, 1 ignored** (the two-substrate flow and the single-node
   `failed`-settles test pass; the single-node restart-survival test is
   `#[ignore]`d — see Deviations).
6. `cargo test -p syneroym-substrate --test roym_app_e2e`: **passed, 0
   failed** — one added step signs a listing and verifies its envelope.
7. `cargo test -p roymctl`: **77 + 17 passed, 0 failed** (three new
   `find_roym_service` unit tests for `roym address`).
8. `cargo xtask check-roym-deps`: **Clean** — `roym_core` on its
   allowlist, no roym service crate pulled in anything new.
9. Planning-identifier grep over `crates/roym_*`, `crates/roym_core/app/`,
   `crates/wit_interfaces/wit/invocation/` and every file this slice
   touched (all four work orders): **no `M0[0-9]`, `\bR[1-4]\b`,
   `\bC[0-9]`, `D-C[0-9]`, `D-0[0-9]`, or `Slice ` in any name or
   comment.** Nine WO2/WO3 slips fixed (parity section headers and notes,
   a listing doc comment, a Hub editor comment, two e2e step comments).
   Pre-existing earlier-milestone references in files C5 only lightly
   touched (`// ---- C1 new verbs ----` in the dual-build fixture,
   `D-06C-4` in `app_host/src/lib.rs`, the M04A/M05A comments in
   `engine.rs` / `dispatch.rs` / `runtime.rs`) are untouched and out of
   C5's scope, the same call C4 made for `synsvc_native.rs`.
10. `cargo +nightly fmt --all`: **Clean**.
11. `cargo clippy --workspace --all-targets --all-features`: **Clean**.
12. `cargo test --workspace` (2026-09-03, sandbox off): **exit 0, 0
    failures** across 151 test binaries. The known pre-existing flake
    `scheduled_task_e2e::a_supervisor_restart_skips_the_ticks_it_missed`
    (iroh network-path abandonment, unrelated to C5) passed in this run.
    The `#[ignore]`d `a_pending_message_and_its_body_survive_a_substrate_restart`
    did not run, as expected.
13. `cargo audit`: **Clean (0 vulnerabilities)** · `cargo deny check
    licenses`: **`licenses ok`**.
14. `mise run test:e2e` (2026-09-03): **31 default + 4 multi-hop
    passed**, including `roym-hub.spec.ts`'s 13 cases (8 from C2/C4 plus
    the 5 covering §11.4's six intents). `mise run test:roym-ui` (vitest):
    **27 passed** across 4 files.

---

## C6 — What shipped, and what did not

**Read this section before trusting the "Partial" status above at face
value.** The slice plan ([slice-c6-implementation-plan.md](slice-c6-implementation-plan.md))
specifies six work orders. WO1-WO4 (the vocabulary, the server half, the
client half, and a slice of the parity suite) are built, tested, and
verified below. WO5 (the native shim's wire-origin fix) was skipped —
the plan itself calls it severable. **WO6 (the Hub UI, `roymctl`'s own
polish, and the three-substrate e2e) was only partly built**: `roymctl
roym directory` exists and works; the Hub's Directory/SynOrg tabs and the
three-substrate `roym_directory_e2e.rs` do not exist. This is recorded
here in full rather than summarized away, because R1 row 5's acceptance
test has a visual half ("missing evidence shows as unknown, never as
positive", *rendered*) that nothing in this pass proves.

### D-C6-1 — the C6 row and Gap 7 are corrected, not followed

The milestone's own Gap 7 said the Directory could build FTS5 and R\*Tree
tables through `execute-ddl`/`query-raw` with no new host interface. That
conclusion does not survive reading the tree: both verbs require
`data-layer/admin`, whose only producer is the deploy-time lifecycle hook;
no Roym component exports `init`/`migrate`; the native build has no
lifecycle path at all; and no owner-rooted UCAN chain can carry the
ability (ADR-0015/0016's own boundary). **C6 therefore builds no FTS5
table, no R\*Tree table, and issues no `execute-ddl`/`query-raw` call
anywhere** — a grep over `crates/roym_directory` confirms it. Search is
built on the existing MongoDB-style filter DSL over a purpose-built
projection collection (`search_index`), which the DSL's `$regex` and
`$and` operators cover completely for category tokens, free text, and
bounding-box intersection. `task.md`'s own Gap 7 text is corrected below
rather than left to contradict this section.

### What landed

1. **`roym_core::admit`'s wire-exception table** (`WireRule`, `Caller`,
   `admit`). `require_internal` is untouched and keeps its five other
   callers; `directory` alone moves to `admit::admit` with a three-method
   table: `directory.search` and `directory.info` are `Open` (any caller,
   identified or not — reading something published on purpose costs
   nothing to leave open); `directory.publish` is `VerifiedOnly`. Every
   other Roym verb, on every service, is unaffected and stays refused
   over the wire (parity 106b, 68).
2. **`roym_core::area`'s exact intersection functions** — `boxes_intersect`,
   `areas_intersect` (`None` for any pairing touching a `Named` area),
   `labels_match` — layered on the existing over-covering `bounding_box`
   sieve. A geometric search refines every sieve candidate exactly before
   it is returned (parity 88/89).
3. **`roym_core::listing::verify_envelope` + `ListingVerdict`** — the one
   verification body `catalog.listing.verify` and the directory's own
   `publish`/`query-source` all call now, so a stranger's listing is never
   verified twice by two copies of logic that could quietly disagree.
   Gains `revocation_status` (`"good"`/`"unknown"` — `RevocationStatus`
   carries no serde impl of its own, so the wire shape is the word, not
   the enum), which `catalog.listing.verify`'s response did not carry
   before. N-2's caps (unbounded `PaymentTerms`/`ProductDetail`/
   `ServiceDetail` fields, `Area::Named::label`) land in the same pass.
4. **`roym_core::directory`** — the shared vocabulary: `SynOrgSettings`
   (+ validation), `SearchQuery`/`SearchHit`/`AreaMatch`/`SourceError`, and
   every derived constant with its own build-time assertion (`
   source_timeout_fits_inside_the_dispatch_epoch`,
   `client_concurrency_stays_below_guest_http_admission`) rather than a
   number trusted on faith. `normalize_text`/`normalize_category` are the
   one normalization both the write side (the projection) and the query
   side share, because `compile_regex` emits `LIKE` with no `ESCAPE`
   clause — a wildcard can only be removed, never escaped.
5. **`roym_directory`'s server half** (`crates/roym_directory/src/app.rs`):
   `directory.settings`/`set-settings`, `directory.info` (no roster —
   `member.*` stays local-only), `member.add`/`remove`/`list`,
   `directory.publish` (verified-caller-only; verifies the envelope;
   refuses a `draft`; runs the publication limiter keyed on the envelope's
   **issuer**, never the connection; prunes the limiter ledger and, per the
   SynOrg's own `retention_secs`, `publications`/`search_index` together;
   deletes every stale index row for a listing before writing its
   replacement, closing the republish-with-fewer-areas leak the plan calls
   out by name), `directory.unpublish`, `directory.publications`,
   `directory.search` (filter compiled from category/text/`open_to`/
   `booking_mode`/bounding-box; the sieve refined exactly per candidate;
   one hit per `listing_id`; the response carries no verdict field at
   all), `directory.limits`/`set-limits`, `directory.reindex`,
   `directory.export`/`import` (five bundle sections:
   `synorg`/`members`/`publications`/`publication_log`/`sources` — bare
   nouns, matching every existing section name).
6. **`roym_directory`'s client half**: `directory.add-source` (probes once
   with `directory.info`; a probe that succeeds but answers `null` is
   reported, not silently swallowed), `remove-source`, `sources`,
   `directory.probe-info` (one `directory.info` call with no persistence,
   for `roymctl roym directory info` — not in the plan's own verb table,
   added because a stranger should be able to read a directory's rules
   without adding it as a source first), `start-run` (mints a run id from
   the current `RUNS` row count rather than a process-global counter —
   `std::process::id()` traps on `wasm32-wasip2`, and a `static
   AtomicU64` would drift out of step between the wasm build's
   per-instantiation memory and the native build's process-lifetime
   memory, which is exactly the divergence a parity suite exists to
   catch; caught here by parity 97, fixed before it reached the plan's own
   two-directory harness gap below), `query-source` (one proxy call, one
   dispatch; refuses a `source` not in this person's own sources and a
   `run_id` this node did not mint; verifies every hit on this node,
   stores at most `MAX_STORED_PER_SOURCE` verified and `MAX_REFUSED_
   RESULTS` refused rows), `merge` (per-source share then round-robin
   across sources in DID order; keeps the newest signed version per
   `listing_id`; returns projections, never envelopes), `run-envelope`
   (fetches the one envelope a person actually opens).
7. **`directory.publish-to-source`** — the one verb that reads a signed
   envelope from `catalog` (through a new `directory → catalog`
   `depends_on` edge) and sends it to a chosen source. `directory` is now
   the only Roym service with `CallTarget::Service` in its own source and
   the only one with a wire-reachable verb; nothing else in the product
   talks to a stranger's node in either direction.
8. **The manifest and native wiring**: `directory` gains `depends_on =
   ["catalog"]` (no `visibility`/`topology_visibility` change — both were
   already correct for a wire-reachable, publicly discoverable service);
   `init_roym` persists the matching binding; `router.rs` gains the
   `member.` prefix and its dependency test grows to the third edge.
9. **`roymctl roym directory`**: `sources`, `add`/`remove`, `find` (mints
   a run, drives `query-source` per source with `tokio::task::JoinSet` in
   batches of `max_concurrency` — the node's own number, never a client
   guess — then `merge`; prints verified hits with issuer/age/both
   unknowns, and refused evidence and source errors as their own blocks),
   `publish`, `info` (drives the new `probe-info` verb), `serve` (writes
   `SynOrgSettings` from a rules file — journey step S2), and `member
   add`/`remove`/`list`.
10. **`roym_catalog`'s inherited C5-7 fix**: `listing.list` filters by
    status at the host when asked; `listing.history` filters on
    `payload.listing_id` (a dotted path into the stored envelope's JSON,
    not a top-level field) instead of parsing every history row to find
    matches; `publication_secs_in_window` filters on `at_secs` and the
    same call site now prunes rows outside the window, closing
    `deferred-backlog.md`'s "publications never pruned" row for the
    catalog side the same way `directory.publish` closes it for the
    directory side.

### A defect this slice found and fixed: `uuidish()`'s reliance on
process identity

`start_run`'s first implementation minted a run id from
`std::process::id()`. That traps on `wasm32-wasip2` — a component has no
real process id — which parity scenario 97 caught immediately as a wasm
trap (`unreachable` instruction) rather than a silent divergence. The
fix replaced it with a process-global atomic counter, which parity then
caught as a *second*, quieter bug: the wasm build's counter lives in
wasm linear memory that is fresh per component instantiation inside one
test harness, while the native build's `static` lives for the life of
the whole test binary process and accumulates across every test that
calls `start_run` — so the two builds' run ids matched only by accident,
whenever a test happened to be the first in the process to touch either
counter. The actual fix reads the current `RUNS` row count from storage
instead: reproducible per build because each build's storage starts
identically empty per harness, and — unlike a Rust `static` — the right
shape for a real substrate, which may host or restart sandboxes within
one long-lived process. Recorded here because it is exactly the class of
bug the parity suite exists to catch, and it was caught twice, by two
different symptoms, before the constant it was measured against was even
looked up.

### Failure-and-security matrix rows C6 closes

| Row | How |
|---|---|
| **1** (a forged or absent listing signature) | For the directory path: parity 81 (`directory.publish` refuses a tampered envelope with the verdict's own reason). The **consumer-receiving-a-forgery** half (a forged hit reaching a search result, kept and marked rather than dropped or trusted) is **not** proven in this pass — it needs the two-directory harness noted below, since a single directory's own `publish` already refuses forgeries at the door. |
| **2** (a directory asserts a credential is valid) | **Structurally, for R1's half**: parity 93 asserts a directory's own search answer carries no verdict field at all — `verified`, `revocation_status`, `credential` are all absent — so there is nothing for the product to (mis)trust from the directory's side. The client's own verdict (parity 79, 94) carries `revocation_status: "unknown"` and `credential: "unknown"`, the honest values in R1. The credential *content* half stays C9's. |
| **3** (no Directory deployed anywhere) | Not re-proven at e2e level in this pass (the three-substrate e2e was not built); the two-substrate suites this slice left untouched (`roym_conversation_e2e.rs`, `roym_app_e2e.rs`) still pass, so the R1 rows 1-4 path by direct link is intact, but R1 row 5's own "optional by construction" claim (`D-06C-6a`) is proven only at verb level here (parity 101: a run with zero sources succeeds with zero hits, never an error). |
| **12** (flooding) | The publication half is now complete: the directory-side caller of `safety::admit_publication` ships, keyed on the issuer (parity 83), the withdrawal exemption holds (parity 84), a `draft` is refused before it can consume budget (parity 84c), and the ledger prunes itself. `[PRD-SAF]` is fixed at both call sites (`catalog`, `directory`) against the one function. |
| **18** (an unaffiliated caller resolves the Directory) | Proven at the wire-admission level: `directory.search`/`info` admit an anonymous wire caller (parity 94, 76) and every other verb on every service still answers `-32013` (parity 106b, and the pre-existing 67/68 unchanged). **Not** proven with a real cross-substrate registry resolution (`topology_visibility = "open"` / `supervisor/resolve`) — `deferred-backlog.md`'s existing row on that gap is unchanged, not closed, by this slice. |
| **19** (build divergence) | 31 new parity scenarios (74-115, some plan numbers merged or renumbered against what actually needed separate coverage; 110-115 added by the code-review follow-up below), all passing identically on both builds, including the wire admission table in both directions and the confused-deputy invariant (parity 109: no `WIRE_REACHABLE` method makes a proxy call). |

Rows C6 explicitly does **not** close: the credential half of row 2 and
the revocation half of row 15 (both C9's, by `D-06C-6`'s R1/R3 split);
the forged-hit-reaches-a-consumer half of row 1 (needs the two-directory
harness below); and row 3's cross-installation half (needs the
three-substrate e2e below).

### What C6 did not build

Recorded here in full, once, rather than scattered as a hedge on every
claim above. Each has its own row in `deferred-backlog.md` §11 with a
pickup trigger.

1. **The Hub has no Directory or SynOrg tab.** No person can add a
   source, run a search, read a result's source/age/unknowns, see refused
   evidence rendered distinctly, create a SynOrg, edit its publication
   limit, or publish a listing to a directory, from the browser. This is
   the single largest gap: R1 row 5's acceptance test has a rendered half
   ("missing evidence shows as unknown, never as positive") that only a
   browser can prove, and nothing in this pass drives one.
   `roymctl roym directory` is a full, working second client that drives
   the same JSON-RPC verbs the Hub would — exit criterion 2's "a second
   client drives the same flow through the same API with no UI involved"
   is met — but it is not a substitute for the Hub existing.
2. **No `crates/substrate/tests/roym_directory_e2e.rs`.** The plan's
   three-substrate reference scenario (a SynOrg owner, a provider
   publishing to it, a consumer finding it, the no-directory regression
   run twice, a certificate-dependency failure over a real transport, the
   loop at `MAX_SOURCES` over real transports) does not exist. The
   existing two-substrate suites (`roym_conversation_e2e.rs`,
   `roym_app_e2e.rs`) are unmodified and still pass in full.
3. **No second, independently-stored directory in the parity harness.**
   The plan's `did:key:hForeignWire`/`hForeignWire2` scaffolding, needed
   to prove two directories genuinely disagreeing about a version
   (`versions_differ`) and a directory crowding a page with forged *or*
   genuinely signed recent listings (`D-C6-18`'s per-source share), was
   not built. What the new scenarios do prove — the admission table,
   `publish`/`search`/`settings`/roster, and the client verbs' own shape
   and validity checks — is real and passes on both builds; two-directory
   merge behaviour is unverified.
4. **WO5 was not attempted** (severable by the plan's own design): the
   native shim's guest-HTTP/websocket sinks are not wired through
   `host_for_wire`, so that permitted difference is carried forward
   unchanged rather than closed.
5. **`roymctl roym directory`'s `find`, `serve`, and `member` subcommands
   have no automated test of their own** — they are exercised only by
   hand against the same verbs the Rust suites cover directly. A
   `roymctl` CLI-argument test (mirroring the existing
   `apps/roymctl/tests/cli_args.rs` gap already in the backlog) was not
   added.

### Permitted differences added to §14 (WASM vs native)

13. **The guest dispatch epoch (WASM) versus none (native) cannot show,
    because the fan-out loop was kept out of the guest entirely.** WASM
    arms a 5s wall-clock dispatch budget that burns while suspended in a
    host call; the native shim arms nothing. Every C6 dispatch on both
    builds makes at most one bounded proxy call (`query-source`), so the
    budget is never approached on either build. Parity 97 is one such
    call, asserted as a real round trip through the proxy path shared with
    every other cross-service call in the product.
14. **Guest-HTTP admission (WASM, 4 concurrent, 503 after a 2s wait) has
    no native counterpart**, which is exactly the limit
    `MAX_CLIENT_CONCURRENCY` (3) is derived from. Not a divergence for a
    client that honours the `max_concurrency` `start-run` returns, since
    such a client never approaches the WASM-only limit on either stack.
    `roymctl find` and (when it exists) the Hub are both required to
    honour it; nothing enforces that requirement but review, per the new
    backlog row above.
15. **The wire origin still has no production producer on the native
    build.** A natively linked `directory` is registered only in the
    local endpoint registry and never published, so `host_for_wire`'s
    only caller stays the parity harness. A native deployment of Roym
    therefore cannot serve a foreign consumer's search or publish to a
    foreign directory — both now stated together in `deferred-backlog.md`'s
    instance-certificate row, since the outbound half (no certificate to
    present) and the inbound half (no wire-origin producer) are the same
    underlying limit seen from two directions.

---

### C6 — Code review follow-up (2026-09-05)

An independent code review of the Rust core (the admission table,
`publish()`, `search()`, `query_source()`, `merge()`) found 15 issues,
concentrated on exactly what a slice adding the product's first
wire-reachable verbs should be scrutinized hardest for: input a stranger
fully controls, and the rate limiter and freshness checks around a
durable write. All 15 were incorporated; none were pushed back on. In
order of what an anonymous stranger can reach first:

1. **`search()` never validated the caller-supplied `Area`.** An extreme
   `radius_m` drove `bounding_box`/`areas_intersect` into `i64`/`u64`
   overflow (a debug-build panic, a release-build wrong answer). Fixed by
   calling `Area::validate()` in `search()` before any arithmetic, and —
   belt and braces, since the arithmetic itself should not depend on every
   caller validating first — making `bounding_box` and the circle-circle
   radius sum in `areas_intersect` saturating rather than wrapping/
   panicking. Two new `roym_core::area` unit tests and parity scenario 110.
2. **`publish()` stored a stranger's bytes even on a node running no
   SynOrg.** `directory.info` already answers `null` for such a node;
   `directory.publish` now refuses with the same reasoning, before ever
   touching storage. Parity scenario 112.
3. **The publication limiter was keyed on the envelope's `issuer`, not
   the verified caller.** Since `directory.search` serves envelopes back
   verbatim, this let a caller either mint a fresh budget by rotating
   issuer keys, or exhaust a *stranger's* budget by replaying their own
   signed envelope under a connection the attacker controls. Re-keyed on
   `published_by` — the identity the router actually verified for the
   connection — everywhere: `publication_secs_in_window`'s filter, the
   `publication_log` row shape, and its index.
4. **Republishing accepted an older or replayed envelope with no
   freshness check**, including on the withdrawal path (replaying an old
   `withdrawn` envelope could delete a provider's current live listing
   for free). Fixed with a check against the stored row: refuse unless
   the incoming envelope is strictly newer, *or* its own `supersedes`
   names the stored `record_id`. The `or` is load-bearing, not
   cosmetic — the parity harness pins the signing clock, so two
   legitimate versions signed in the same test run tie exactly on
   `issued_at_secs`, and only `supersedes` tells a real edit apart from a
   replay. This required threading `supersedes` through
   `roym_core::listing::ListingVerdict`, which did not carry it before.
   Parity scenario 115 proves both halves in one scenario: a same-second
   edit is accepted, and replaying the superseded envelope afterward is
   refused.
5. **`query.categories` and `query.text` had no cap** on the anonymous
   search path, despite the same limits existing and being enforced
   everywhere else. Added `roym_core::directory::MAX_QUERY_TEXT_LEN` and
   a `MAX_CATEGORIES` check in `search()`. Parity scenario 111.
6. **`search()` full-scanned `publications` once per hit** (up to 50 full
   collection scans per anonymous-reachable request) where a direct
   `get_json` by `record_id` — which the index row already carries —
   does the same job in one read.
7. **A local `directory.publish` could never succeed.** The router routes
   the whole `directory.` prefix to this service as an owner method, but
   the handler refused anything that was not `Caller::Verified` — and a
   local dispatch legitimately arrives `Caller::Internal` by the
   admission rule's own design (a local caller is trusted for where it
   came from, whatever the wire table says). Decided, rather than
   mechanically patched: a local publish now uses this installation's own
   recorded owner as `published_by`, read from the host and never from a
   caller-supplied value. Parity scenario 113.
8. **The search index stored the directory's own receive time as
   `issued_at_secs`**, so a stale listing re-served today outranked a
   genuinely newer one, silently contradicting `roym_core::directory`'s
   own documented meaning of the two fields. `build_index_rows` now takes
   the signed `issued_at_secs` and the directory's `received_at_secs` as
   two separate parameters, threaded correctly from both `publish()` and
   `reindex()`.
9. **`merge()` could list one source twice in a hit's `sources[]`.** The
   cross-source union pass compared against a `kept` row that the same
   loop was mutating, so a source already recorded could be recorded
   again once `kept` moved to a different source mid-loop. Restructured
   into two passes: round-robin selection decides *which* listings are
   included (unchanged), then a separate pass gathers every source's row
   for each selected listing and computes the winner and the source list
   once, from data that does not change under it.
10. **`query_source` verified every hit a source returned before applying
    the stored cap**, so a source answering with far more hits than it
    could ever have stored got all of them signature-checked in guest
    memory — exactly the dispatch-epoch budget the timeout constants were
    sized against. Now truncates to `MAX_STORED_PER_SOURCE +
    MAX_REFUSED_RESULTS` before verifying, not after.
11. **`search_runs` keys collided across sources.** Two directories
    serving the same signed envelope (same `record_id`) would have the
    second `query-source` call's row silently overwrite the first's,
    undercounting `sources[]` in the merged hit. `source` is now part of
    the key; `run-envelope`'s lookup (which only ever took `record_id`,
    an API shape kept as-is) now scans this one run's rows for a matching
    `record_id` instead of doing an exact-key `get`.
12. **A single directory could fill every refused-evidence slot**, since
    the list was sorted by `(source, listing_id)` — both values a forger
    or a hostile directory controls — rather than round-robined the way
    verified hits are. Fixed with the same per-source round-robin
    `merge()`'s verified path already used.
13. **`import()` never rebuilt `search_index`.** A restored node answered
    zero hits for listings it demonstrably held until an owner happened
    to run `directory.reindex` by hand. `reindex`'s body is now a shared
    `rebuild_search_index` helper, called automatically at the end of
    `import()`.
14. **`open_to`/`booking_mode`/`status` were indexed via `{:?}`
    (`Debug`), not their declared `#[serde(rename_all = "kebab-case")]`
    spelling.** Every multi-word variant (`existing-customers`, etc.)
    indexed under a string nothing else in the product produces, so a
    query for the documented value matched nothing. Fixed with a small
    `serde_str` helper used at both call sites. Parity scenario 114.
15. **The rate-limiter's read-then-write was not atomic**, and the
    ledger row was written last, after several other awaited operations
    — the widest window available for two concurrent publishes to both
    read the same prior state and both be admitted. The data layer
    offers no compare-and-swap this call could use instead, so the fix
    narrows rather than eliminates the window: the ledger row is now
    written immediately on admission, before the prune/replace work that
    used to sit between the decision and the record of it.

Six new parity scenarios (110–115) were added for the findings with a
clean, harness-reachable repro; findings 9 and 11 (the merge/storage
fixes) do not have a dedicated regression test, because reproducing them
needs two genuinely independent directories with separate stores — the
same two-directory parity-harness gap already recorded above and in
`deferred-backlog.md` §11. Both were verified by re-tracing the fixed
code by hand against the exact sequence the review described, and by the
existing 105-scenario suite continuing to pass (which would have caught
a `merge()` regression against a single source, just not the two-source
case the finding was about).

**One residual noticed while fixing finding 4, not in the original 15:**
withdrawal deletes the `PublicationRow` entirely rather than keeping a
tombstone, so once a listing is withdrawn there is no stored
`issued_at_secs` left to compare a later publish against — a stale,
pre-withdrawal envelope replayed *after* a legitimate withdrawal would be
accepted as if new, since the freshness check has nothing to refuse it
against. Not fixed in this pass (it is a storage-shape change —
keeping a withdrawn row as its own anchor — adjacent to but outside the
15 reported findings); recorded as its own backlog row rather than
folded silently into finding 4's fix.

**Correction to finding 3's own writeup, raised on a second review pass.**
Keying the publication limiter on `published_by` closes the half that
mattered — a stranger can no longer exhaust a *different* provider's
budget by replaying their signed envelope — but it does not, by itself,
give `published_by` any cost to mint. `crates/router/src/route_handler/
http.rs:486-488` assigns `AuthLevel::Delegated` to every verified
preamble, including an unchallenged node-DID pubkey with no certificate
behind it, so a wire caller's `published_by` proves key possession and
nothing else — exactly as cheap to generate as the envelope issuer key
it replaces. The fix binds the rate limit to the correct party; it does
not make that party expensive to become many of. Real resistance to that
needs a membership credential, which is C9's, by `D-06C-6`'s own R1/R3
split. Failure-matrix row 12 (flooding) is closed for the specific
attack the row and the findings both named — exhausting *someone else's*
budget, or resetting your own by rotating a signing key — not for an
attacker willing to mint a fresh identity per attempt.

### C6 — Second code review pass (2026-09-05)

A second, independent pass over the same fixes found four more issues —
three real gaps the first round's fixes opened or left standing, one a
precision correction to the first round's own writeup (above). All four
fixed, no pushback:

16. **`DIRECTORY_SCHEMA_VERSION` stayed at 2** after finding 4's fix added
    `PublicationRow::issued_at_secs` as a required field with no serde
    default. `import()`'s version gate compares against this constant, so
    a bundle exported before that change would be *accepted* and then
    fail to deserialize row by row — `rebuild_search_index` silently
    skipping each one. The no-migrations-pre-release policy makes
    changing the row shape in place correct; it does not make a stale
    version gate correct. Bumped to 3, with the parity harness's
    `expected_schema_version` map (`crates/roym_web/tests/
    dual_build_parity.rs`) updated to match and given its own arm rather
    than sharing one with `profile`/`catalog`/`conversation`.
17. **`search()`'s finding-6 fix (a direct `get_json` instead of a
    collection scan) turned one unparseable row into a hard failure of
    the whole request.** The replaced code skipped a row it could not
    parse; the replacement's `Err` arm returned `Response::internal_error`
    for the *entire* anonymous-reachable search. Now both `Ok(None)` and
    `Err(_)` drop just that one hit, matching the old behaviour's
    robustness.
18. **`merge()`'s per-source list was still not deduplicated by
    `listing_id`.** The reported cause (a `kept` row mutated mid-loop)
    was fixed, but nothing stopped two rows for the *same* listing
    landing in one source's own list in the first place — not from one
    `query-source` call (a directory's own `search()` already collapses
    to one hit per listing), but from two calls for the same `(run_id,
    source)`, e.g. a client retry, each storing a different, genuinely
    signed version under a different `record_id` (and therefore a
    different `search_runs` key, per finding 11's own fix). A comment
    claimed this "cannot happen... by construction"; the construction did
    not exist. Added: dedupe by `listing_id` within each source, keeping
    the newer row, before the existing sort-and-truncate. Parity scenario
    116 reproduces the exact two-call sequence.
19. **`run_envelope`'s scan matched on `record_id` alone, with no guard
    against a refused row.** Its own comment justified the match by
    `record_id` being content-derived from the envelope — true only for a
    *verified* row; a refused row's `record_id` is whatever the source
    claimed, unverified, so a hostile source could set one to collide
    with a genuine record. The genuine row won only because `did:` sorts
    before `refused#` under the collection's own `id` order — a lexical
    accident, not a guarantee. Added `&& !row.refused` to the match.

Parity scenario 117 was added alongside these — not itself one of the
four findings, but written while fixing finding 16 because no
`directory.export`/`import` scenario existed at all until now; it also
exercises finding 13's fix (import reindexing) and the resulting search
in one pass. `dual_build_parity` now carries 107 scenarios (74-117),
all passing on both builds; the full gate was re-run in full afterward.

**A genuinely flaky test, found by the full-workspace re-run itself, not
by either review.** `cargo test --workspace` failed twice
(`scenario_83`, `scenario_86`) with `assert_eq!(w, n)` mismatches on
`received_at_secs` differing by exactly one second between the wasm and
native builds. Both fields are each build's own wall clock
(`directory.search`'s `answered_at_secs`/per-hit `received_at_secs`, and
`retry_after_secs`, computed from two such reads) -- documented
elsewhere in this file as unsynchronized between builds, and already
handled for other services by `strip_volatile` before comparison. Most
of the 33 new scenarios compared raw responses carrying one of these
fields directly, which happens to pass on almost every run (the two
builds' clocks agree to the second far more often than not) and fails
exactly as rarely as that assumption is wrong -- which is what makes a
flaky test worse than a deterministic failure: it passed cleanly through
every verification run in this document until this one. Fixed at every
call site that compares a raw `directory.*` response pair (18 scenarios,
26 call sites): `strip_volatile` gained `received_at_secs`,
`answered_at_secs`, `retry_after_secs`, `age_secs`, and `last_ok_secs`
(a `directory.sources` probe timestamp missed on the first pass), and a
`stripped(&v) -> Value` borrowing wrapper lets a call site write
`assert_eq!(stripped(&w), stripped(&n))` without giving up `w`/`n` for
the assertions that follow. Verified with 25 back-to-back runs each of
`scenario_83` and `scenario_86` (50 runs, 0 failures) in addition to the
full suite re-run.

## C6 — Verification evidence

1. `cargo test -p syneroym-roym-core --lib`: **90 passed, 0 failed** —
   `admit`'s wire-exception table, `area`'s exact intersection functions,
   `listing::verify_envelope`'s verdicts (accept/tamper/issuer-mismatch),
   and `directory`'s settings validation, text/category normalization, and
   the two derived-constant build-time assertions.
2. `cargo build` / `cargo clippy --all-targets --all-features` on
   `syneroym-roym-directory`, `syneroym-roym-catalog`, `syneroym-roym-web`
   (tests included), `roymctl`, and `syneroym-substrate --features
   roym,dual_build_fixture`: **all clean, 0 warnings** (three `expect()`
   call sites in `roym_directory`'s non-test code were rewritten to
   returned errors during this pass rather than left as warnings).
3. `cargo test -p syneroym-roym-web --test dual_build_parity`: **107
   passed, 0 failed** (rerun in full after every implementation fix and
   both code-review passes landed) — the 73 pre-existing scenarios
   (unchanged behaviour, includes one merged plan-numbered pair) plus 33
   new functions covering scenarios 74-117 (`scenario_88_89` covers two
   plan-numbered cases in one function; 110-117 are the code-review
   follow-ups' own regression scenarios, across both passes). All six
   `wasm32-wasip2` Roym components were rebuilt with `cargo component
   build --release --target wasm32-wasip2` before each run; two real bugs
   were caught and fixed in the implementation pass (`directory`'s own
   `SCHEMA_VERSION` bump needed the harness's `expected_schema_version`
   map updated, and `start_run`'s process-derived run id both trapped the
   wasm build and produced non-reproducible ids across builds), plus the
   19 findings from both code-review passes, detailed above.
4. `cargo xtask check-roym-deps`: **Clean.**
5. Planning-identifier grep over every file this slice touched or added
   (`crates/roym_core/src/{admit,area,listing,router,backup,directory}.rs`,
   `crates/roym_directory/src/app.rs`, `crates/roym_catalog/src/app.rs`,
   `apps/roymctl/src/commands/roym.rs`, `crates/roym_core/app/roym.toml`,
   `crates/roym_web/tests/dual_build_parity.rs`): **no `M0[0-9]`,
   `\bR[1-4]\b`, `\bC[0-9]`, `D-C[0-9]`, `D-0[0-9]`, or `Slice ` in any
   name or comment.** Six slips found and fixed in this pass's own new
   code (three `D-C6-*` references, two `C5-7` references, one `(C6)`
   section header in the test file); pre-existing earlier-milestone
   references in `crates/substrate/src/runtime.rs` (M05A/M05B) are
   untouched and out of this slice's scope, the same call C4 and C5 made
   for their own lightly-touched files.
6. `cargo +nightly fmt --all`: **clean** (`-- --check` reports no diff).
7. `cargo clippy --workspace --all-targets --all-features`: **clean, 0
   warnings.**
8. `cargo test --workspace` (sandbox off, per the repository's own
   sandbox note), re-run after both code-review passes: **2475 passed, 0
   failed** across 151 test binaries — including `dual_build_parity`'s 107
   (again), `roym_conversation_e2e.rs`, and `roym_app_e2e.rs` (both
   unmodified by this slice and both still green).
9. `cargo audit` (re-run): **clean (0 vulnerabilities)**.
10. `cargo deny check licenses` (re-run): **clean (`licenses ok`)**.
11. `mise run test:e2e` (re-run): **31 passed (default config, 1.2m) + 4
    passed (multi-hop, 19.4s)** — identical counts to C5's own baseline;
    this slice added no new Playwright cases (the Hub UI gap above), so
    this run proves no regression rather than new browser coverage.

**What this evidence does, and does not, prove.** It proves the Rust
core — the admission rule, the server half, the client half, the manifest
wiring, and `roymctl` — is correct and behaves identically on both
builds, to the depth the 33 new parity scenarios reach, and that nothing
else in the workspace (the two-substrate e2e suites, the Playwright
suite) regressed. It does **not** prove R1 row 5's acceptance
test end to end: that needs the Hub UI (item 1 above) and, for the
cross-installation half, the three-substrate e2e (item 2 above). Both are
named, not hidden, in "What C6 did not build."

