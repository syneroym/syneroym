# Milestone 2: Reliable, Operable Node

## Goal

Establish basic network transport robustness, cryptographic identity delegation,
and foundational operational mechanics before introducing stateful persistence in
M3. At the end of this milestone, a single node can be installed by an operator,
presents a stable delegated identity to peers, retries transient connection
failures gracefully without custom heartbeats, enforces runtime quotas in the
WASM sandbox, ships with a locked-down native TLS surface, and passes automated
smoke tests against the live coordinator at `syneroym.xyz`.

---

## Requirement IDs (Traceability)

| Requirement ID | Description | Sub-requirements targeted in M2 |
|---|---|---|
| `[TOP-ROB]` | Network & Connection Robustness | Retry with exponential backoff; reactive eviction on failure; QUIC idle timeout reliance |
| `[FND-IDT]` | Cryptographic Identity Primitives (Handshake slice only) | Master Key → Temporary Key delegation certificate; `verifyAndDeriveSharedSecret` handshake authorization; passive revocation via DHT array drop |
| `[FND-DEP]` | Deployment / Operations | Native TLS + rustls + certbot lifecycle; resource connection caps; cross-platform release pipeline; official Docker image; smoke tests against coordinator |
| `[FND-SEC]` | Substrate Security (runtime and memory slice only) | Wasmtime fuel metering quotas; OS `mlock` + `madvise(MADV_DONTDUMP)` for cryptographic key RAM; connection and payload limits at Iroh/QUIC boundary |
| `[PRD-OPS]` | Non-specialist install / recover | Operator-facing: guided install, plain-language health endpoint, SSH + `journalctl` diagnostic coverage for this milestone's features |

> **Out of scope in this milestone (deferred):**
> - `[FND-IDT]` extensions: Master Key export/recovery, Tier-1 fallback (ZK proof), Method B ZK plugin, device-bound consumer keys.
> - `[FND-SEC]` storage encryption (Envelope Encryption / DEK / KEK vault) — this is **M3A**.
> - `[FND-SEC]` hardware attestation (`substrate.attest`) — deferred to M7.
> - `[FND-SEC]` supply-chain binary signing — deferred to M7.
> - `[FND-CFG]` service configuration delivery — deferred to M3A.
> - `[PRD-OPS]` full admin surface (backup/restore, update/rollback, remote notifications) — deferred to M5/M7.

---

## Unresolved Decisions (Must Be Resolved Before Implementation)

These items are **blocking**. Implementation of the relevant slices must not
begin until each is resolved as an ADR in `docs/decisions/`.

### D-02-01 — Delegation Certificate Format

**Context:** The architecture specifies that the Master Key issues a
"Delegation Certificate" to a Temporary Key and that handshakes validate the
chain (section "Cryptographic Delegation (Method A)"). The identity crate
(`crates/identity`) already has `ControllerAgreement` — a mutual-signature
binding between a node DID and a controller DID. The codebase also has
`MasterAnchorPayload` in `crates/core/src/dht_registry.rs` from M1, which
lists authorized Temporary Key DIDs without encoding delegation expiry as a
verifiable certificate.

**Decision needed:** Is the `ControllerAgreement` (mutual two-party signature)
the delegation certificate for M2, or do we define a separate
`DelegationCertificate` (single-issuer, controller-only signed, with expiry) as
described by the architecture? What serialization format (CBOR vs JSON+z-base-32
vs a VC Data Model envelope) and what expiry field semantics?

**Impact:** Determines Slice 2 API surface and the exact bytes placed in the
`MasterAnchorPayload`'s `temporary_keys` array vs. a separate cert field.

---

### D-02-02 — Handshake Authorization Point

**Context:** The architecture describes an E2E ECDH handshake inside the
established stream (section "End-to-End (E2E) Encryption Handshake") where the
ephemeral keys are signed by permanent Ed25519 identity keys. The router crate
has a connection preamble (`crates/router/src/preamble.rs`). The Iroh transport
layer already provides QUIC TLS 1.3; the handshake described is an
*application-layer* E2E layer on top.

**Decision needed:** In M2, does the identity handshake authorize connections
at the level of the Iroh `RouteHandler` (inside
`crates/router/src/route_handler.rs`) checking the preamble's identity chain,
or does it sit at the `CoordinatorIroh` level as a stream interceptor? What
constitutes a "passed handshake" — verifying the delegation certificate chain is
sufficient, or also deriving a session key?

**Impact:** Determines whether a new `HandshakeVerifier` lives in
`crates/router` or in a new `crates/handshake` crate, and whether the E2E ECDH
key derivation (`verifyAndDeriveSharedSecret`) is implemented in M2 or deferred
to M4 (Universal Proxy / wRPC).

---

### D-02-03 — Retry Policy Ownership and Configuration Schema

**Context:** `[TOP-ROB]` specifies configurable retry counts (default 3) and
exponential backoff. Currently connection establishment is ad hoc in
`coordinator.rs` (30-attempt polling loop for registry registration). The
`SubstrateConfig` in `crates/core` does not yet expose a structured retry
policy.

**Decision needed:** Where does the canonical `RetryPolicy` struct live
(`crates/core/src/config.rs` or a dedicated `crates/retry` utility crate), and
what fields does it expose (`max_attempts: u8`, `initial_backoff_ms: u64`,
`backoff_multiplier: f64`, `max_backoff_ms: u64`)? Is the policy
per-coordinator-connection or per-outbound-request?

**Impact:** Determines the config schema extension in `SubstrateConfig` and the
API surface of the retry utility used by Slice 1 and Slice 3.

---

### D-02-04 — Docker Image Scope and Base Image

**Context:** `[FND-DEP]` specifies official Docker images pre-configured for
the community. The codebase has no Docker infrastructure yet.

**Decision needed:** Which binaries are containerized for M2
(`syneroym-substrate` only, or also `syneroym-coordinator`/
`syneroym-community-registry`)? Which base image (`distroless`, `debian-slim`,
`alpine`)? Multi-arch targets in M2 (`linux/amd64` only, or also
`linux/arm64`)?

**Impact:** Determines the GitHub Actions CI matrix and the Dockerfile(s) to
add.

---

### D-02-05 — WASM Fuel Quota Manifest Schema

**Context:** `[FND-SEC]` requires Wasmtime fuel metering for
`max_instructions` from the SynApp manifest. The `SynAppManifest` and
`ServiceManifest` types in `crates/app_orchestration` do not yet have resource
quota fields.

**Decision needed:** What field names and types? (`max_instructions:
Option<u64>`, `max_memory_bytes: Option<u64>`?) Are quotas per service or per
invocation? What is the default when unset (unlimited, or a conservative
substrate-global default)?

**Impact:** Determines the manifest schema change in
`crates/app_orchestration/src/models.rs` and the Wasmtime store
fuel-configuration code in `crates/app_sandbox/src/engine.rs`.

---

## Explicit Non-Goals

The following items are **explicitly excluded** from M2 and must not creep in:

- Encrypted SQLite databases, vault, or any persistent secret storage (M3A).
- Active Control Plane Mode (`roymctl` as a server SynApp) (M5).
- Multi-node clustering, WAL replication, or topology epochs (M7).
- Universal Proxy / wRPC cross-component calls (M4).
- Any DLN, UCAN, or FDAE access control (M4).
- Mobile-specific lifecycle (M10).
- AI concierge features (M9).
- `[FND-IDT]` Method B (ZK plugin, government ID binding) (deferred, post-M10).
- Full Syneroym Hub UI (M6).
- Litestream/WAL backup (M3A/M7).
- Hardware attestation API (`substrate.attest`) (M7).
- Supply-chain binary signing for release artifacts (M7).

---

## Dependency Gates

M2 may begin **only when**:

1. **M1 is fully closed.** All M1 exit criteria are verified and recorded in
   `docs/planning/milestones/M01-local-app-model/status.md`. ✅ (completed 2026-06-24)
2. **Decisions D-02-01 through D-02-05 are resolved** and written as ADRs in
   `docs/decisions/` before their respective slices begin implementation. ✅ (completed 2026-06-25)
3. The existing test suite (`cargo test --workspace`) passes cleanly on the branch
   that M2 slices are cut from, with zero clippy warnings.

---

## Current State Inventory

Understanding what exists vs. what needs to be built.

### Already Built (Relevant to M2)

| Crate | What Exists |
|---|---|
| `crates/identity` | `Identity` (Ed25519 keygen, sign, save/load from raw bytes); `derive_did_key`; `resolve_did_key`; `ControllerAgreement` + mutual-signature verification; `SubstrateIdentityState`; JSON canonicalization (RFC 8785); `sign_json` |
| `crates/core/src/dht_registry.rs` | `MasterAnchorPayload`, `SignedMasterAnchor`, `resolve_master_anchor`, `publish_master_anchor` (schema: `master_anchor_v1`) |
| `crates/coordinator_iroh` | `CoordinatorIroh`: Iroh endpoint + relay server + QUIC router + `/v1/info` HTTP endpoint + 30-attempt registry registration polling loop |
| `crates/router` | `RouteHandler` (coordinator + substrate modes), `preamble.rs` with `RoutePreamble`, `net_iroh.rs`, `net_webrtc.rs`, `connection_router.rs` |
| `crates/app_sandbox` | `WasmEngine` with Wasmtime component model, pooling allocator, WASM instance cache (`DashMap`), `HostState` per-request — **fuel metering is not yet configured** |

### Gaps to Close in M2

| Gap | Target Slice |
|---|---|
| No structured `RetryPolicy` config; coordinator registration uses a raw loop | Slice 1 |
| No retry + exponential backoff on Iroh connection establishment | Slice 1 |
| No QUIC idle timeout explicitly configured on Iroh `Endpoint`; `Endpoint.connect()` peer calls not retried | Slice 1 |
| `Identity` stores key as raw bytes in plaintext on disk; no `mlock` or `MADV_DONTDUMP` | Slice 2 |
| No `DelegationCertificate` — `MasterAnchorPayload` lists bare DIDs with no verifiable delegation expiry | Slice 2 |
| No handshake authorization in `RouteHandler` — preamble carries identity but chain is not verified | Slice 2 |
| No Wasmtime fuel metering enabled; quota fields absent from manifest schema | Slice 3 |
| No connection cap at Iroh/QUIC boundary | Slice 3 |
| No TLS certificate lifecycle management (certbot, `rustls` reload) | Slice 4 |
| No cross-platform release pipeline (GitHub Actions) | Slice 4 |
| No Docker image | Slice 4 |
| No automated smoke tests against `syneroym.xyz` | Slice 5 |

---

## Migration Strategy

### Schema Versioning

- `MasterAnchorPayload` from M1 uses schema `"master_anchor_v1"` with bare DID
  strings in `temporary_keys`. If D-02-01 resolves that delegation certs are
  embedded, the schema must bump to `"master_anchor_v2"`. The reader in
  `resolve_master_anchor` must gracefully handle v1 payloads (no cert field
  implies cert-less trust for existing records until they are re-published).

### Config Backward Compatibility

- `SubstrateConfig` TOML gains new optional sections (`retry`, `tls`,
  `connection_cap`). All new fields must be `#[serde(default)]`; existing config
  files without them must parse cleanly with documented safe defaults.

### Manifest Backward Compatibility

- `ServiceManifest` gains an optional `quota: ResourceQuota` field. Existing
  manifests without it are valid and receive the substrate-global default quota.
  The addition is a minor semver-compatible change to the manifest format.

### roymctl CLI

- If D-02-01 produces a `DelegationCertificate`, a new `roymctl identity`
  sub-command will be added to generate and publish it. This is strictly
  additive; existing commands are unchanged.

---

## Ordered Implementation Slices

### [x] Slice 1: Robust Transport — Retry and QUIC Idle Timeouts

**Requirement IDs:** `[TOP-ROB]`
**Blocking decision:** [D-02-03](../../../decisions/0003-retry-policy-ownership.md) is resolved.

#### Tasks
- [x] Define `RetryPolicy` struct in `crates/core/src/config.rs`:
  - Fields: `max_attempts: u8` (default 3), `initial_backoff_ms: u64` (default
    100), `backoff_multiplier: f64` (default 2.0), `max_backoff_ms: u64`
    (default 30_000).
  - Add `retry: RetryPolicy` to `SubstrateConfig` with `#[serde(default)]`.
- [x] Implement `retry_with_backoff<F, Fut, T, E>` utility in
  `crates/core/src/retry.rs`. Requirements:
  - No dependencies beyond `tokio::time`.
  - Respects all `RetryPolicy` fields.
  - Logs each retry at `warn!` with attempt number and error.
  - Applies ±10% jitter to avoid thundering herd.
  - Returns `Err` on the final attempt without further sleeping.
- [x] Replace the hardcoded 30-attempt polling loop in
  `coordinator.rs::register_in_global_registry` with `retry_with_backoff`.
- [x] Apply `retry_with_backoff` to **outbound peer connection attempts** in
  `crates/router/src/net_iroh.rs`: wrap `endpoint.connect(node_addr, alpn)`
  calls (not `endpoint.online().await`, which waits for network reachability
  and is not a peer-connection operation) with the retry utility.
- [x] Apply `retry_with_backoff` to `endpoint.online().await` in
  `ConnectionRouter::init_iroh` (line 104 of `connection_router.rs`) with an
  explicit timeout so a slow or unreachable relay does not block startup
  indefinitely.
- [x] **Configure QUIC idle timeouts** on the Iroh `Endpoint` builder in
  `build_iroh_endpoint` (`crates/router/src/net_iroh.rs`):
  - Add `idle_timeout_secs: u64` (default 30) to `CoordinatorIrohConfig` in
    `crates/core/src/config.rs`.
  - Pass it to the `Endpoint::builder` via the appropriate `TransportConfig`
    setter (e.g. `quinn::TransportConfig::max_idle_timeout`). Iroh's connection
    pool will then automatically evict peers that are idle beyond this window
    — no custom application-level dead-tracking needed.
  - Integration test: connect two Iroh endpoints; block traffic for longer than
    `idle_timeout_secs`; verify the connection is evicted by Iroh itself and the
    next `endpoint.connect()` re-establishes cleanly.
- [x] **Do not add custom dead-connection tracking** to
  `connection_router.rs`. The `iroh::Endpoint` and `iroh::Router` already manage
  the connection pool natively. The `[TOP-ROB]` spec mandates relying on Iroh's
  pooling rather than a custom application-level cache.
- [x] Unit tests for `retry_with_backoff`:
  - Retries exactly `max_attempts` times on persistent error.
  - Backoff grows exponentially within expected jitter band.
  - Succeeds on Nth retry if the function succeeds.

#### Acceptance Criteria
- Outbound `endpoint.connect()` failure triggers ≤ `max_attempts` retries with
  exponential backoff; no panic, no hang.
- QUIC idle timeout is explicitly configured; Iroh evicts stale connections
  natively without any custom heartbeat or dead-connection cache.
- No custom ping/pong heartbeat tasks are spawned.

---

### [ ] Slice 2: Cryptographic Identity Delegation and Handshake

**Requirement IDs:** `[FND-IDT]`
**Blocking decisions:** [D-02-01](../../../decisions/0001-delegation-certificate-format.md) and [D-02-02](../../../decisions/0002-handshake-authorization-point.md) are resolved.
**Depends on:** Slice 1 complete.

#### Tasks

**Memory Protection (prerequisite):**
- [ ] Add `zeroize` and `nix` (or `libc`) to `crates/identity` dependencies.
- [ ] Wrap `Identity.signing_key` with `ZeroizeOnDrop` semantics.
- [ ] Add `lock_memory(ptr, len)` helper in `crates/identity/src/keys.rs` that
  calls `mlock(2)` + `madvise(MADV_DONTDUMP)` on non-Windows targets (no-op
  on Windows with a compile-time `cfg` warning).
  - **`mlock` failure must degrade gracefully:** non-root users and Docker
    containers without the `IPC_LOCK` capability will have `mlock` fail with
    `EPERM`/`ENOMEM`. The helper must log a `warn!("`mlock` unavailable: {e};
    key will not be locked in RAM — ensure `--cap-add=IPC_LOCK` in production")`
    and continue rather than returning an error or panicking. The substrate must
    start and function correctly regardless of whether `mlock` succeeds.
- [ ] Call `lock_memory` in `Identity::from_bytes` and `Identity::generate`.
- [ ] Unit test: verify key bytes are zeroed after `Drop`.
- [ ] Unit test: verify substrate starts cleanly when `mlock` is unavailable
  (simulate by temporarily dropping `CAP_IPC_LOCK` in test, or by mocking).

**Delegation Certificate (resolve D-02-01 first):**
- [ ] Implement `DelegationCertificate` in `crates/identity/src/delegation.rs`:
  - Fields: `master_did`, `temporary_did`, `issued_at`, `expires_at`, `scope`
    (e.g., `"routing"`), `signature`.
  - `DelegationCertificate::issue(master: &Identity, temp_pubkey: VerifyingKey,
    expires_in_secs: u64) -> Result<Self>` — signs over canonicalized JSON
    payload (RFC 8785).
  - `DelegationCertificate::verify(master_did: &str) -> Result<()>` — resolves
    master pubkey from DID; verifies signature; checks `expires_at > now`.
  - `to_json() -> Result<String>` / `from_json(s: &str) -> Result<Self>`.

**MasterAnchorPayload Evolution:**
- [ ] Extend `MasterAnchorPayload` in `crates/core/src/dht_registry.rs`:
  - Bump to `schema: "master_anchor_v2"`.
  - Embed `DelegationCertificate` records (or a `temporary_keys_v2` field per
    D-02-01 resolution).
  - Reader gracefully falls back to `v1` (bare DID strings, no cert validation).
- [ ] Update `resolve_master_anchor` to validate delegation cert signatures and
  expiry for each key entry; reject expired or invalid certs.
- [ ] Update `publish_master_anchor` to accept `Vec<DelegationCertificate>`.

**Handshake Authorization (resolve D-02-02 first):**
- [ ] Add `HandshakeVerifier` in `crates/router/src/handshake.rs` (or new crate
  per D-02-02 resolution):
  - `verify_preamble(preamble: &RoutePreamble, resolver: &dyn MasterAnchorResolver)
    -> Result<VerifiedIdentity>` — confirms the source key is listed in the
    anchor's authorized temporary keys (with valid, unexpired delegation cert).
- [ ] Integrate `HandshakeVerifier` into `RouteHandler::on_connection` — invoked
  before processing the request payload. Failed handshake closes stream with
  `Unauthorized` preamble response.

**Tests:**
- [ ] Valid delegation cert verifies correctly.
- [ ] Expired cert fails validation.
- [ ] Mis-signed cert fails validation.
- [ ] Route handler rejects connections from non-authorized Temporary Keys.
- [ ] Passive revocation: publish anchor with key A, verify A succeeds; re-publish
  without A; verify A is now rejected.

#### Acceptance Criteria
- `Identity` zeroes key bytes on `Drop`; `mlock` is called on non-Windows.
- `DelegationCertificate` issued by Master Key is verifiable in < 1 ms.
- `RouteHandler` rejects expired/revoked/unknown Temporary Keys.
- Existing tests (M1) remain green — no regressions in `resolve_master_anchor`.

---

### [ ] Slice 3: Runtime Quotas and Connection Caps

**Requirement IDs:** `[FND-SEC]` (runtime quota and connection limit
sub-requirements)
**Blocking decision:** [D-02-05](../../../decisions/0005-wasm-fuel-quota-schema.md) is resolved.
**Can be developed concurrently with Slice 2.**

#### Tasks

**Manifest Schema — Resource Quotas:**
- [ ] Add `ResourceQuota` struct to `crates/app_orchestration/src/models.rs`:
  ```rust
  pub struct ResourceQuota {
      pub max_instructions: Option<u64>, // Wasmtime fuel units
      pub max_memory_bytes: Option<u64>, // bytes
  }
  ```
- [ ] Add `pub quota: Option<ResourceQuota>` to `ServiceManifest`.
- [ ] Add substrate-global defaults to `SubstrateConfig` (e.g.,
  `default_max_instructions: u64 = 10_000_000_000`,
  `default_max_memory_bytes: u64 = 268_435_456`).

**Wasmtime Fuel Metering:**
- [ ] Enable `config.consume_fuel(true)` in `crates/app_sandbox/src/engine.rs`.
- [ ] Set `store.set_fuel(quota.max_instructions)` before each invocation.
- [ ] Catch `wasmtime::Trap::OutOfFuel` and return structured `QuotaExceeded`
  error; log at `warn!`; do not panic.
- [ ] Unit test: WASM component that loops forever is deterministically trapped
  at fuel limit; Tokio runtime remains healthy.

**WASM Linear Memory Limit:**
- [ ] Wire `PoolingAllocationConfig::max_memory_size` from manifest quota.
- [ ] Unit test: WASM component that allocates beyond `max_memory_bytes` fails
  with `MemoryFault`; no substrate crash.

**Connection Cap at Iroh/QUIC Boundary:**
- [ ] Add `max_connections: usize` (default 500) to `CoordinatorIrohConfig`.
- [ ] Implement `AtomicUsize` connection counter in `CoordinatorIroh`.
- [ ] In `RouteHandler::on_connection`: check counter before accepting; reject
  with `ServiceUnavailable` if at cap. **Use a RAII drop-guard token**
  (e.g. `ConnectionSlot(Arc<AtomicUsize>)` that decrements in `Drop`) rather
  than a manual decrement call — this ensures the counter is always decremented
  even during unexpected task panics, client disconnects mid-stream, or
  early returns from error paths.
- [ ] Load test: 600 concurrent connection attempts against a 500-cap node;
  ≤ 500 accepted; 100 cleanly refused; counter returns exactly to the
  pre-test baseline after all connections close (no leaks).

#### Acceptance Criteria
- Looping WASM component deterministically trapped at fuel limit; substrate healthy.
- Over-allocating WASM component fails cleanly with `MemoryFault`.
- At `max_connections`, new connections cleanly refused; no OOM or panic.

---

### [ ] Slice 4: Native TLS, Release Pipeline, and Docker

**Requirement IDs:** `[FND-DEP]`
**Blocking decision:** [D-02-04](../../../decisions/0004-docker-image-scope.md) is resolved.
**Can be developed concurrently with Slices 2–3.**
**Depends on:** Slice 1 complete.

#### Tasks

**Native TLS with rustls and certbot lifecycle:**
- [ ] Add `tls` section to `SubstrateConfig`:
  ```toml
  [tls]
  cert_path = "/etc/letsencrypt/live/example.com/fullchain.pem"
  key_path  = "/etc/letsencrypt/live/example.com/privkey.pem"
  reload_on_sigusr1 = true
  ```
- [ ] Implement `TlsCertLoader` in `crates/core/src/tls.rs` using `rustls`:
  - Loads cert chain and private key from disk.
  - Supports hot-reload: listens for `SIGUSR1` (Unix) or file-watch to re-read
    cert files without process restart.
  - Exposes `Arc<ServerConfig>` behind `watch::Receiver` for zero-downtime
    rotation.
- [ ] Wire `TlsCertLoader` into the Axum HTTP/TLS server in the substrate entry
  point.
- [ ] Integration test: rotate certificate on disk; send `SIGUSR1`; verify
  subsequent TLS handshakes use new cert without dropped connections.

**Cross-Platform Release Pipeline:**
- [ ] Add `.github/workflows/release.yml` triggered on version tags (`v*.*.*`):
  - Builds `syneroym-substrate` and `roymctl` for:
    - `x86_64-unknown-linux-gnu`
    - `aarch64-unknown-linux-gnu` (cross-compiled via `cross`)
    - `x86_64-apple-darwin` + `aarch64-apple-darwin` (macOS universal via `lipo`)
    - `x86_64-pc-windows-msvc`
  - Uploads artifacts to GitHub Releases with SHA-256 checksums.
- [ ] Add `mise.toml` task `release:build` wrapping cross-compilation commands.
- [ ] CI must fail if binary size regresses > 10% vs. previous release.

**Official Docker Image (per D-02-04 resolution):**
- [ ] Add `Dockerfile` at workspace root (or `deploy/Dockerfile`) building a
  minimal image containing `syneroym-substrate` binary.
- [ ] Add `.github/workflows/docker.yml`:
  - Builds and pushes to `ghcr.io/syneroym/syneroym-substrate:{tag}` on release
    tags.
  - Pushes `ghcr.io/syneroym/syneroym-substrate:latest` on main.
  - Multi-arch: `linux/amd64` + `linux/arm64`.
- [ ] Add `deploy/docker-compose.community.yml` as a reference for community
  deployments pointing to `syneroym.xyz`.
- [ ] Document in `deploy/docker-compose.community.yml` that production
  deployments requiring strict memory-key protection must add
  `cap_add: [IPC_LOCK]` and `ulimits: memlock: -1` to the service definition,
  and explain the security trade-off when omitted.

#### Acceptance Criteria
- TLS cert rotates without process restart; existing connections undisturbed.
- Release pipeline produces signed binaries for ≥ 4 platforms on tag push.
- Docker image builds; `docker run syneroym-substrate --help` exits 0.
- Multi-arch Docker manifest present in GHCR.

---

### [ ] Slice 5: Smoke Tests and Operational Observability

**Requirement IDs:** `[FND-DEP]` (smoke tests), `[PRD-OPS]`
**Depends on:** Slices 1–4 complete; live coordinator at `syneroym.xyz` accessible.

#### Tasks

**Automated Smoke Tests:**
- [ ] Add `tests/smoke/` integration test binary (or `crates/smoke-tests`) that
  accepts `--coordinator-url` and `--registry-url` flags.
  - Test 1 (connectivity): Iroh connection to coordinator; `/v1/info` responds.
  - Test 2 (registry): register a test endpoint; look it up successfully.
  - Test 3 (master anchor): publish `MasterAnchorPayload` v2; resolve it.
  - Test 4 (retry): induce transient failure (wrong relay URL for first attempt);
    verify retry logic reconnects successfully.
  - Test 5 (quota): deploy fuel-limited WASM component; verify quota trap is
    clean.
- [ ] Add `mise run test:smoke` task.
- [ ] Add `.github/workflows/smoke.yml` running smoke tests against `syneroym.xyz`
  on release tags.

**Health Endpoint Extension:**
- [ ] Extend `/v1/info` in `coordinator_iroh` to expose a structured health
  object:
  ```json
  {
    "status": "healthy | degraded",
    "identity": { "did": "did:key:...", "controller_status": "verified" },
    "connections": { "active": 42, "cap": 500 },
    "tls": { "cert_expiry_days": 87 },
    "relay": { "online": true }
  }
  ```

**Documentation:**
- [ ] Update `docs/developer-guide.md` with M2 operational runbook:
  - How to generate a Master Key and issue a Delegation Certificate.
  - How to set up TLS with certbot, configure `syneroym.toml`, and hot-reload
    certs via `SIGUSR1`.
  - How to run `mise run test:smoke` against a local and remote deployment.

#### Acceptance Criteria
- `mise run test:smoke` passes against a local substrate.
- `mise run test:smoke --coordinator-url https://syneroym.xyz` passes against
  the live coordinator.
- `/v1/info` returns a structured health object conforming to the schema above.
- Developer guide M2 runbook is present and accurate.

---

## Tests and Runnable Reference Scenario

### Reference Scenario: Delegated Substrate Identity Handshake

1. Operator generates a Master Key (`did:key:...M`).
2. Operator generates a Temporary Key (`did:key:...T`), issues a
   `DelegationCertificate` signed by M authorizing T for 90 days.
3. Operator publishes the master anchor DHT record (`v2`) embedding the cert for
   T.
4. A peer substrate dials the operator's substrate using T as its routing `NodeId`.
5. The operator substrate's `RouteHandler` resolves the master anchor, validates
   the delegation cert for T (expiry + signature), and accepts the connection.
6. The peer substrate signs the `RoutePreamble` with T's key; signature verified.
7. `roymctl status` (or `curl /v1/info`) returns `"status": "healthy"` and
   `"controller_status": "verified"`.

### Failure / Security Tests

| Test | Expected Outcome |
|---|---|
| Connection with expired `DelegationCertificate` | `Unauthorized` rejection; peer receives explicit error |
| Connection from Temporary Key not in the master anchor array | `Unauthorized` rejection |
| Revoking Temp Key mid-session (anchor re-published without it) | New connections rejected; existing stream runs until natural close |
| WASM component exceeds fuel quota | Deterministic trap; substrate healthy; no panic |
| WASM component allocates memory beyond `max_memory_bytes` | Clean `MemoryFault`; substrate healthy |
| 600 concurrent Iroh connections against 500-cap node | ≤ 500 accepted; 100 cleanly refused; no OOM |
| Transient connection failure (1st attempt) | Retry succeeds on 2nd attempt within `initial_backoff_ms × 2` |
| All retries exhausted (simulated hard failure) | Structured error returned within `max_backoff_ms × max_attempts` |
| TLS certificate rotated on disk + SIGUSR1 sent | Next handshake uses new cert; existing connections undisturbed |

---

## Performance Budgets

| Metric | Budget | Measurement Method |
|---|---|---|
| Delegation certificate verification (single cert chain) | < 1 ms p99 | `criterion` micro-benchmark |
| Master anchor DHT resolution (cache hit) | < 5 ms p99 | Integration test with mock DHT |
| Master anchor DHT resolution (cache miss, network round-trip) | < 200 ms p99 | End-to-end test against `syneroym.xyz` |
| Iroh connection establishment (LAN, no retry) | < 100 ms p99 | Existing `bench:latency` suite |
| Retry overhead (3 retries, 100/200/400 ms backoff) | < 750 ms total elapsed | Integration test |
| Fuel quota trap overhead vs. un-metered invocation | < 5% latency regression | `bench:micro` WASM invocation bench |
| Connection cap check (`AtomicUsize` load) | < 100 ns | `criterion` micro-benchmark |
| `/v1/info` health endpoint response | < 10 ms p99 | Smoke test with `hyperfine` |
| Docker image size (uncompressed, single-arch) | < 80 MB | CI check in `docker.yml` |

---

## Measurable Exit Criteria

All of the following must be verified and recorded in `status.md` before M2 is
closed:

- [ ] `cargo +nightly fmt --all` passes with zero diff.
- [ ] `cargo clippy --workspace --all-targets --all-features` passes with zero
  warnings and zero errors.
- [ ] `cargo test --workspace` passes with all tests green.
- [ ] `mise run test:e2e` passes (existing e2e scenarios must not regress).
- [ ] `mise run test:smoke` passes against a local substrate deployment.
- [ ] `mise run test:smoke --coordinator-url https://syneroym.xyz` passes against
  the live coordinator (or equivalent staging environment).
- [ ] `wasm32-wasip2` compilation target remains unbroken:
  `cargo build --target wasm32-wasip2 -p syneroym-bindings`.
- [ ] Reference scenario (delegated handshake journey) executes end-to-end without
  error.
- [ ] All nine failure/security tests listed above produce the expected outcome.
- [ ] Docker image builds and `docker run syneroym-substrate --help` exits 0.
- [ ] Multi-arch Docker manifest (`linux/amd64`, `linux/arm64`) present in GHCR.
- [ ] Release pipeline workflow succeeds on a test tag.
- [ ] Performance budgets verified (criterion and smoke test output captured in
  `status.md`).
- [ ] Traceability matrix in `docs/planning/traceability-matrix.md` updated with
  M2 evidence for `[TOP-ROB]`, `[FND-IDT]`, `[FND-DEP]`, `[FND-SEC]`, and
  `[PRD-OPS]`.
- [ ] Decisions D-02-01 through D-02-05 resolved and written as ADRs in
  `docs/decisions/`.
