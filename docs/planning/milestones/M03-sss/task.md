# Milestone 3: Secure Stateful Services (M03-sss)

> [!NOTE]
> All five blocking decisions (D-03-01 through D-03-05) are **resolved**. ADRs
> are recorded in `docs/decisions/` as ADR-0006 through ADR-0010. Slices may
> proceed in order once the M3 dependency gate is cleared.

## Goal

Introduce the foundational data layer and secret management infrastructure. By the
end of this milestone, a deployed `SynSvc` can provision an isolated, encrypted
SQLite database (via SQLCipher); perform schema-initialised CRUD and batch
operations through typed WIT host functions with a MongoDB-style query API;
store and retrieve secrets from an in-database vault; receive versioned
configuration through a typed host function; and — in 3B — store blobs via the
`object_store` unified backend and exchange asynchronous events via an embedded
`rumqttd` MQTT broker.

This milestone is split into three sequential sub-milestones:

- **M3A — Structured State and Security:** Encrypted SQLite isolation (`oltp`/`olap` profiles), full
  data-layer WIT surface, vault integration, and configuration delivery.
- **M3B — Objects and Events:** Blob object service and the pub/sub half of
  `syneroym:messaging` (MQTT API overlay).
- **M3C — Unified Messaging Streams and HTTP Bridge:** Bidirectional streaming
  half of `syneroym:messaging`, then an HTTP passthrough bridge onto the
  native-dispatch surface (data-layer, blob-store, messaging).

M3B may begin only after M3A exit criteria are fully met. M3C may begin only
after M3B exit criteria are fully met.

> **Note:** `syneroym:messaging` was formerly planned as `syneroym:pubsub`.
> It is renamed and split across M3B (pub/sub) and M3C (bidirectional
> streaming) so the WIT package shape is stable from Day 1 without a breaking
> v0.2 rename later. HTTP Passthrough — previously tracked as an open item at
> the end of M3B — now lives in M3C because it depends on the streaming half,
> not just pub/sub. See [meta-implementation-plan.md](../../meta-implementation-plan.md#milestone-3c-unified-messaging-streams-and-http-bridge).

---

## Requirement IDs (Traceability)

| Requirement ID | Description | Sub-requirements targeted in M3 |
|---|---|---|
| `[PLT-DAT]` | Data Layer | Structured SQLite DBs per service (CRUD, batch, filters, pagination, nested WIT serialisation via JSON payloads, schema init DDL, `oltp`/`olap` build profiles; `AggregationPipeline` deferred to M4); blob S3-compatible backend and signed HTTP access (M3B) |
| `[PLT-DAP]` | Distributed Data Topology | Pluggable `StorageProvider`/`ServiceStore` trait foundations only (Slice 2A); logical data-service routing deferred to M5 |
| `[PLT-DAP-04]` | Decentralized Pub/Sub | MQTT API via in-process `rumqttd` with wildcard topics, retained messages, change notifications (M3B, as the pub/sub half of `syneroym:messaging`); Iroh QUIC log-replication overlay deferred to M7 (alongside `[PLT-RED]` DB and blob replication, as the same replication primitive) |
| `[PLT-DAP-06]` | Generic Bidirectional Streaming | `syneroym:messaging` `stream-cursor` (guest-as-source pull) and `stream-sink` (guest-as-sink push) resources, `register-stream-protocol`/`handle-stream-request`/`accept-stream-upload` flows, and the HTTP passthrough bridge built on top of them (M3C) |
| `[FND-SEC]` | Substrate Security (storage slice) | Envelope Encryption (DEK/KEK); secret vault inside encrypted SQLite; `mlock`-protected KEK RAM; DEK re-encryption on key rotation; DEK-encrypted blob content (M3B); production profiles default to encryption; opt-out produces persistent insecure-state warning |
| `[FND-CFG]` | Service Configuration Delivery | `syneroym:app-config/get` WASM host function; schema validation and defaults at deploy-time; Podman env-var and file-mount fallback; versioned immutable configuration generation; out-of-band secret rotation policy |

> **Out of scope in this milestone (deferred):**
> - `[FND-IAM]` FDAE access control and UCAN integration — **M4**.
> - `[PLT-DAT]` Universal Proxy / wRPC inter-component RPC — **M4**.
> - `[PLT-RED]` WAL replication, redundancy, HA failover — **M7**. Includes
>   the decentralized pub/sub QUIC log-replication overlay (`[PLT-DAP-04]`,
>   moved here from an earlier M5 placement — see ADR-0010 Amendment 2) and
>   peer-to-peer blob replication for non-S3 deployments. This is a
>   redundancy/failover concern only — cross-node *access* to
>   `syneroym:messaging` and `blob-store` already works via the same
>   RPC/native-dispatch routing as any cross-node host-function call (e.g.
>   `data-layer`), available from Slice 5/6A onward, not gated on M7.
> - `[PLT-ASY]` Outbox queue, DLQ, long-running tasks, saga compensations — **M5**.
> - `[LFC-VER]` SQLite snapshot/rollback on upgrade — **M5**.
> - `[FND-IDT]` Master Key export/recovery, ZK proof plugin — deferred post-M10.
> - `[FND-SEC]` Hardware attestation (`substrate.attest`) — **M7**.
> - `[FND-SEC]` Supply-chain binary signing — **M7**.
> - `[ADV-OBS]` Advanced observability metric pipeline — **M7**.
> - Active Control Plane / Server SynApp mode — **M5**.
> - Multi-node clustering, topology epochs — **M7**.

---

## Resolved Decisions (ADR References)

All five blocking decisions are resolved. See the linked ADRs for full rationale
and consequences. A summary is provided here for planning reference.

### D-03-01 — Envelope Encryption ✅ → [ADR-0006](../../../decisions/0006-sqlite-encryption-sqlcipher.md)

- **SQLCipher** via `rusqlite-cipher` (transparent page-level encryption; WAL mode supported).
- DEK: 32-byte random key, AES-256-GCM encrypted with the KEK, stored in `substrate.db`.
- Raw hex key passed to `PRAGMA key`; no PBKDF2 (bypassed by using raw key, not passphrase).
- **M3: Substrate-global KEK.** One `roymctl kek inject` call unlocks all service DEKs.
- **M4 gate: Per-SynApp-Instance KEK.** Must be added to the M4 milestone plan as an
  explicit gate item (requires M4 IAM/UCAN for authorised per-app injection).
  Per-service KEK scoping is the eventual target in a later milestone.
- DB open time budget **removed from M3**; to be established in the M4 ADR alongside
  per-app KEK, measured on Tier 1 hardware (Raspberry Pi 4).
- Auto-unseal (AWS KMS) is out of scope for the substrate; deployer scripts handle this externally.

### D-03-02 — Data-Layer WIT Interface ✅ → [ADR-0007](../../../decisions/0007-data-layer-wit-interface.md)

- **MongoDB-style JSON query string** at the WIT boundary (`filter: option<string>`).
- `get` returns `option<record-value>` — `Ok(None)` on missing record (not an error).
- `query` returns empty list on no results.
- `AggregationPipeline` (`$group`, `$having`, projections) **deferred to M4** (tracked as gate).
- `execute-ddl` is available in two lifecycle hooks:
  - `init()` — called on **first deploy only** (fresh DB). Schema creation + seed data.
  - `migrate()` — called on **re-deploy** (existing DB). Additive and destructive DDL allowed.
  - Both hooks execute with `is_init_context = true`.
- Host injects `creator_id`, `created_at`, and `updated_at` — guest values silently discarded.

### D-03-03 — Config Host Function ✅ → [ADR-0008](../../../decisions/0008-config-host-function.md)

- `get(key) -> result<option<string>, config-error>` — missing key returns `Ok(None)`.
- `get-section(prefix) -> result<list<tuple<string, string>>, config-error>`.
- Generation **pinned at invocation start** (immutable per-invocation).
- Resolved from `config_generations` table in `substrate.db`.

### D-03-04 — Blob Storage Backend ✅ → [ADR-0009](../../../decisions/0009-blob-storage-object-store.md)

- **`object_store` crate** (Apache Arrow) as the unified backend from Day 1.
- Local filesystem for dev/tests; S3-compatible (MinIO, Tigris, R2) for production via config.
- SHA-256 content addressing; integrity verified on both write and read.
- Signed URLs via HMAC-SHA256 presigner; public HTTP serving deferred.
- Path traversal guard: `service_id` regex + `hash` 64-char hex; `Path::join` +
  `starts_with` (not `canonicalize`).

### D-03-05 — Decentralized Pub/Sub (MQTT API) ✅ → [ADR-0010](../../../decisions/0010-mqtt-broker-rumqttd.md)

> **Amendment (see ADR-0010):** the WIT package originally described below as
> standalone `syneroym:pubsub` is now the pub/sub half of the broader
> `syneroym:messaging` package (Slice 6A); the exported delivery function is
> `guest-api::handle-message`, not `on-message`. Decision content below is
> otherwise unchanged.

- **MQTT protocol abstraction over P2P log replication.** (Implemented initially using `rumqttd` in-process Tokio task for local state, pending full QUIC P2P overlay in M7 — moved from an earlier M5 placement once it was recognized as sharing a replication primitive with `[PLT-RED]` DB/blob replication; see ADR-0010 Amendment 2).
- **Push-model delivery**: host invokes guest-exported handler (`guest-api::handle-message(topic, payload)`).
- Wildcard topics (`+`, `#`) and retained messages **in scope**.
- Cross-service pub/sub **in scope** with topic namespace isolation
  (`svc/<service_id>/<user-topic>`).
- **Bounded channels** (default 1024 messages) with explicit backpressure;
  `publish` returns a structured internal error ("broker channel full") when saturated.
- `CancellationToken` ensures clean broker task termination on substrate shutdown.

---

## Items Tracked for M4 Planning

The following items were deferred from M3 and **must appear as gate items in the
M4 milestone plan**:

- **Per-SynApp-Instance KEK** (D-03-01): Narrowing KEK scope to per-app requires
  M4 UCAN/IAM enforcement. The M3 `KeyStore` API must be designed to accommodate
  this narrowing without interface breaks.
- **`AggregationPipeline`** (D-03-02): `$group`, `$having`, projections deferred
  from the data-layer WIT surface.
- **DB open time performance budget** (D-03-01): Establish with per-app KEK
  measurement on Raspberry Pi 4 hardware.

---

## Explicit Non-Goals

The following must **not** creep into M3:

- FDAE / UCAN access control on data-layer calls (M4). `creator_id` is injected
  but no ReBAC policies are enforced in M3.
- Per-SynApp-Instance KEK (M4 — requires IAM).
- Universal Proxy / wRPC cross-component calls (M4).
- AggregationPipeline in the data-layer WIT (M4).
- WAL replication to a secondary node (M7).
- Active Control Plane / Server SynApp mode (M5).
- SQLite snapshot/rollback on upgrade (`LFC-VER`) (M5 — `migrate()` in M3 has
  no snapshot safety net; developers are responsible for safe DDL).
- Hardware attestation (M7) and supply-chain signing (M7).
- Saga compensations and DLQ (M5).
- AI concierge features (M9) or mobile lifecycle (M10).
- Litestream integration (M7).
- Distributed cron lease scheduling (M5).
- Public (unsigned) blob HTTP serving.
- Full MongoDB operator compatibility (only the operators listed in ADR-0007 are implemented).

---

## Dependency Gates

M3 may begin **only when**:

1. **M2 is fully closed.** All M2 exit criteria verified and recorded in
   `docs/planning/milestones/M02-reliable-node/status.md`. ✅ (completed 2026-06-29)
2. **Decisions D-03-01 through D-03-05 are resolved** and written as ADRs in
   `docs/decisions/` before their respective slices begin.
3. `cargo test --workspace` passes cleanly with zero clippy warnings on the M3 branch.

**M3B gate (additional):**

4. M3A exit criteria verified and recorded in `status.md` before any M3B slice begins.
5. Decisions D-03-04 and D-03-05 resolved before M3B Slices 5-6A begin.

**M3C gate (additional):**

6. M3B exit criteria verified and recorded in `status.md` before any M3C slice begins.
7. A design note or ADR for host-side QUIC stream acceptance/routing (the
   piece `handle-stream-request` delivery depends on, not covered by
   D-03-01 through D-03-05) is recorded in `docs/decisions/` before Slice
   6B implementation begins.

---

## Current State Inventory

### Already Built (Relevant to M3)

| Crate / File | What Exists |
|---|---|
| `crates/app_sandbox/src/engine.rs` | `WasmEngine`, `HostState` (WASI ctx + store limits); fuel metering; epoch interruption; quota trap handling; `syneroym:host/context` import wired |
| `crates/bindings/wit/host.wit` | Placeholder `syneroym:host` package with `context::get-test-context` and `app::run` exports |
| `crates/bindings/wit/control-plane.wit` | Full deploy/undeploy/list WIT; `resource-quota` record already present |
| `crates/app_orchestration/src/models.rs` | `SynAppManifest`, `ServiceManifest`, `ResourceQuota`, semver validation |
| `crates/core/src/config.rs` | `SubstrateConfig` with `RetryPolicy`, `TlsConfig`, `default_max_instructions`, `default_max_memory_bytes` |
| `Cargo.toml` | `rusqlite = { version = "0.40", features = ["bundled"] }`, `aes-gcm = "0.10"`, `rand = "0.9"` in workspace deps |
| `crates/identity/src/delegation.rs` | `DelegationCertificate` with `mlock` memory protection (from M2) |

### Gaps to Close in M3A

| Gap | Target Slice |
|---|---|
| No `syneroym:data-layer` WIT package or interface | Slice 1 |
| No `syneroym:vault` WIT package or interface | Slice 1 |
| No per-service SQLite database isolation | Slice 2A |
| No envelope encryption for SQLite (DEK/KEK) | Slice 2A |
| No secret vault in encrypted DB | Slice 2A |
| No `syneroym:vault/reveal` host function | Slice 2A |
| No data-layer host functions (`put`, `get`, `query`, etc.) wired in Wasmtime | Slice 3A |
| No schema init DDL execution hook (`execute-ddl` in `init()`) | Slice 3A |
| No `syneroym:app-config` WIT interface or host function | Slice 4 |
| No configuration generation pinning in `HostState` | Slice 4 |
| No schema validation at deploy-time for `custom-config` | Slice 4 |
| No production-mode encryption enforcement warning | Slice 2A |

### Gaps to Close in M3B

| Gap | Target Slice |
|---|---|
| No blob object service crate | Slice 5 |
| No `syneroym:blob-store` WIT interface | Slice 5 |
| No S3-compatible blob backend | Slice 5 |
| No embedded MQTT broker (`rumqttd`) | Slice 6A |
| No `syneroym:messaging` WIT interface (pub/sub half) | Slice 6A |
| No MQTT wildcard topic or retained message support | Slice 6A |

### Gaps to Close in M3C

| Gap | Target Slice |
|---|---|
| No `syneroym:messaging` bidirectional streaming, guest-as-source (`stream-cursor`, `handle-stream-request`) | Slice 6B |
| No `syneroym:messaging` bidirectional streaming, guest-as-sink (`stream-sink`, `accept-stream-upload`) | Slice 6B |
| No `register-stream-protocol` host-side routing table | Slice 6B |
| No host-side QUIC stream acceptance/routing for peer-initiated streams (either direction) | Slice 6B |
| No HTTP verb/path routing bridged onto native dispatch (`crates/router/src/route_handler/http.rs` is JSON-RPC-over-POST only) | Slice 7 |
| No chunked-upload-to-`stream-sink` bridge in the HTTP router | Slice 7 |

---

## Migration Strategy

### No Existing Service Databases

There are no deployed `SynSvc` databases from prior milestones. No data
migration is required from M1 or M2 state.

### `substrate.db` Schema Extension

The substrate's internal SQLite database must gain new tables:

- `dek_store` — per-service encrypted DEK blobs — added in Slice 2A.
- `config_generations` — active configuration generations per service — added
  in Slice 4.

These tables are added with `CREATE TABLE IF NOT EXISTS`. A `schema_version`
table tracks the current schema, bumped to `"m3a"` in Slice 2A. On startup,
the substrate detects an older version and runs migration SQL automatically.
No data loss is possible because these are new tables.

### `SubstrateConfig` Extension

New optional TOML sections (all `#[serde(default)]`; existing configs parse cleanly):

```toml
[storage]
blobs_dir = "/var/lib/syneroym/blobs"   # M3B
encryption = true                        # M3A default
# insecure_no_encryption_warning emitted when encryption = false in non-dev profile

[mqtt]                                   # M3B
bind_addr = "127.0.0.1:1883"
```

### `ServiceManifest` Extension

Additive optional fields (existing manifests remain valid):

```toml
[services.my-svc.storage]
encrypted = true   # default true

[services.my-svc.config]
schema_path = "config-schema.json"  # optional JSON Schema for deploy-time validation
rotation_policy = "restart-on-rotation"  # or "none"
```

### WIT Boundary Versioning

New WIT packages (`syneroym:data-layer@0.1.0`, `syneroym:vault@0.1.0`,
`syneroym:app-config@0.1.0`, `syneroym:blob-store@0.1.0`, and
`syneroym:messaging@0.1.0`) are added in M3. `syneroym:messaging` is
introduced in M3B with only its pub/sub surface (`host-api::publish`/
`subscribe`, `guest-api::handle-message`) wired; its `stream-types`
(`stream-cursor` and `stream-sink` resources) and `register-stream-protocol`/
`handle-stream-request`/`accept-stream-upload` surface are declared in the
same WIT file for interface stability but only wired up in M3C, so no
breaking v0.2 rename is needed later. (Formerly planned as a standalone
`syneroym:pubsub@0.1.0` package — renamed for the reasons above.)

Slice 2A intentionally extends the host/control-plane boundary only where
needed for storage encryption and vault bootstrap:

- `syneroym:host` imports `syneroym:vault/vault@0.1.0` and
  `syneroym:data-layer/store@0.1.0`.
- `syneroym:control-plane` exposes a native `security` management interface
  for KEK injection, KEK rotation, and vault secret registration.
- CRUD data-layer calls remain a Slice 3A surface. If linked before Slice 3A,
  they must fail explicitly instead of returning successful no-ops.

The `wasm32-wasip2` compilation target must remain unbroken after every slice.

---

## Ordered Implementation Slices

### [x] Slice 0: Extract SQLite from `crates/core`

**Context:** `crates/core/src/storage.rs` currently contains `SqliteEndpointStorage` and a direct `rusqlite` dependency. This violates the goal of keeping storage implementations pluggable and pollutes the core crate with a specific database driver.

**Tasks:**

- [x] Move `SqliteEndpointStorage` out of `crates/core/src/storage.rs`. Keep the `EndpointStorage` trait and `MockStorage` in `crates/core`.
- [x] Relocate `SqliteEndpointStorage` to `crates/data-layer/src/registry_store.rs` (or a similar dedicated storage crate).
- [x] Remove the `rusqlite` dependency from `crates/core/Cargo.toml`.
- [x] Update the substrate entrypoint to instantiate the relocated `SqliteEndpointStorage` and inject it as `Arc<dyn EndpointStorage>` into the `SubstrateConfig` or registry initialization.

**Acceptance Criteria:**
- `crates/core` has no `rusqlite` dependency.
- Tests pass and `EndpointRegistry` continues to function using the injected SQLite store.

---

### [x] Slice 1: Data-Layer and Vault WIT Interface Design

**Requirement IDs:** `[PLT-DAT]`, `[FND-SEC]` (vault WIT boundary)
**ADR references:** [ADR-0007](../../../decisions/0007-data-layer-wit-interface.md)
**Depends on:** M2 fully closed.

This slice is purely additive WIT authoring and Rust type generation. No host
function implementation yet.

#### Tasks

- [x] Create `crates/bindings/wit/data-layer.wit`:
  - Package: `package syneroym:data-layer@0.1.0;`
  - `interface store` with:
    - `record collection-schema` — name, list of `index-definition` records (for field name and index type: string, numeric, boolean).
    - `record record-write-value` — `id: string`, `payload: list<u8>` (JSON bytes).
    - `record record-read-value` — `id: string`, `payload: list<u8>` (JSON bytes), `creator-id: string`, `created-at: u64` (Unix ms), `updated-at: u64`.
    - `record query-options` — `filter: option<string>` (MongoDB-style JSON query document), `limit: option<u32>`, `cursor: option<string>`.
    - `record query-result` — `records: list<record-read-value>`, `next-cursor: option<string>`.
    - `variant mutation` — `put(record-write-value)`, `patch(tuple<string, list<u8>>)` (id, patch-json), `delete(string)`.
    - `variant data-layer-error` — `permission-denied`, `collection-not-found`, `schema-violation(string)`, `quota-exceeded`, `internal(string)`.
      Note: **no `not-found` variant** — missing records are represented as `option<record-read-value>` returns, not errors.
    - Functions:
      - `create-collection(schema: collection-schema) -> result<_, data-layer-error>`
      - `drop-collection(name: string) -> result<_, data-layer-error>`
      - `put(collection: string, value: record-write-value) -> result<_, data-layer-error>`
      - `patch(collection: string, id: string, patch-json: list<u8>) -> result<_, data-layer-error>`
      - `get(collection: string, id: string) -> result<option<record-read-value>, data-layer-error>`
      - `query(collection: string, opts: query-options) -> result<query-result, data-layer-error>`
      - `delete(collection: string, id: string) -> result<_, data-layer-error>`
      - `delete-many(collection: string, filter: string) -> result<u64, data-layer-error>`
      - `batch-mutate(collection: string, mutations: list<mutation>) -> result<_, data-layer-error>`
      - `execute-ddl(sql: string) -> result<_, data-layer-error>` (admin-only; gated to lifecycle context)
  - World: `world data-layer-guest { import store; }`.
  - Also add guest lifecycle exports to the world:
    - `init: func() -> result<_, string>` (called on first deploy — fresh DB)
    - `migrate: func() -> result<_, string>` (called on re-deploy — existing DB)

- [x] Create `crates/bindings/wit/vault.wit`:
  - `interface vault` — `reveal(key: string) -> result<list<u8>, vault-error>`.
  - `variant vault-error { not-found, permission-denied, internal(string) }`.
  - World: `world vault-guest { import vault; }`.

- [x] Update `crates/bindings/build.rs` to include the new WIT files. (Note: Shifted to `wit-bindgen::generate!` workspace macro standard instead of a custom `build.rs`).

- [x] Regenerate bindings; verify `cargo build --target wasm32-wasip2 -p syneroym-bindings` exits 0.

- [x] Add `crates/data-layer/` crate with placeholder `DataLayerService` struct (no DB logic yet) and re-export the generated WIT types.

#### Acceptance Criteria

- `syneroym:data-layer@0.1.0` and `syneroym:vault@0.1.0` WIT compile cleanly.
- `cargo build --target wasm32-wasip2 -p syneroym-bindings` exits 0.
- Zero new clippy warnings.

---

### [x] Slice 2A: Encrypted SQLite Isolation and Secret Vault

**Requirement IDs:** `[FND-SEC]` (storage encryption), `[PLT-DAT]` (DB isolation)
**ADR references:** [ADR-0006](../../../decisions/0006-sqlite-encryption-sqlcipher.md)
**Depends on:** Slice 1 complete.

#### Tasks

**Envelope Encryption Infrastructure:**

- [x] Add `crates/key-store/` crate with `KeyStore`:
  - Holds an AES-256 KEK in `mlock`'d + `MADV_DONTDUMP` memory (reuse
    `lock_memory` helper from M2 `crates/identity/src/keys.rs`).
  - `inject_kek(kek: [u8; 32]) -> Result<()>` — substrate management
    interface only; refused after first injection without re-auth. Full remote
    UCAN/FDAE authorization for this management channel is deferred to M4.
  - `generate_dek(service_id: &str) -> Result<[u8; 32]>` — random DEK,
    AES-256-GCM encrypted with KEK, stored in `substrate.db`.
  - `load_dek(service_id: &str) -> Result<[u8; 32]>` — decrypts from DB.
  - `rotate_kek(new_kek: [u8; 32]) -> Result<()>` — atomically re-encrypts all
    DEKs in a single `substrate.db` transaction; zeroes old KEK on success.
  - All key material uses `ZeroizeOnDrop`.
  - **Design constraint:** The `inject_kek` / `load_dek` interface must accept a
    `scope` extensibility point (e.g., an optional `app_instance_id: Option<&str>`
    parameter) so that M4 per-SynApp-Instance KEK can be introduced without
    breaking the `KeyStore` interface.

- [x] Add `dek_store` and `schema_version` tables to `substrate.db`:
  ```sql
  CREATE TABLE IF NOT EXISTS schema_version (version TEXT NOT NULL);
  CREATE TABLE IF NOT EXISTS dek_store (
    service_id    TEXT PRIMARY KEY,
    encrypted_dek BLOB NOT NULL,
    nonce         BLOB NOT NULL,
    created_at    INTEGER NOT NULL
  );
  ```

- [x] Define generic storage traits in `crates/data-layer/src/traits.rs` to ensure the storage backend is pluggable:
  - `trait StorageProvider`: provides `open_service_db(service_id, key_store) -> Result<Box<dyn ServiceStore>>`.
  - `trait ServiceStore`: defines the CRUD, batch, and DDL operations.
- [x] Implement build profiles: `syneroym-oltp` and `syneroym-olap` (both currently backed by standard SQLite). Ensure Cargo feature gate structure is in place. (DuckDB integration is explicitly deferred to future backlog).
- [x] Implement `SqliteStorageProvider` in `crates/data-layer/src/sqlite.rs`:
  - Implements `StorageProvider`. Manages per-service SQLite files at `<data_dir>/<service_id>/state.db`.
  - Path traversal guard: `service_id` must match `^[a-zA-Z0-9_\-]{1,128}$`;
    use `Path::join` + `starts_with` to confirm the resolved path is strictly a
    descendant of `<data_dir>`. Do **not** use `Path::canonicalize` — it requires
    the path to exist and behaves differently across OSes.
  - `open_service_db` resolves DEK, opens/creates the service DB, and passes the
    raw hex DEK to SQLCipher via `PRAGMA key = "x'<hex>'";`.
- [x] Implement `SqliteServiceStore` in `crates/data-layer/src/sqlite.rs`:
  - Implements `ServiceStore`. Wraps a `deadpool-sqlite` reader pool + single-writer `mpsc`
    channel (Actor/Pool concurrency model per architecture).

- [x] Production enforcement:
  - `encryption = true` (default) + no KEK at `open_service_db` → `error!` +
    `Err(StorageError::EncryptionKeyRequired)`.
  - `encryption = false` in non-dev profile → `warn!` on every startup:
    `"INSECURE: storage encryption is disabled. Only use in development profiles."`.

**Secret Vault:**

- [x] `_vault` table in each `state.db`:
  ```sql
  CREATE TABLE IF NOT EXISTS _vault (
    key        TEXT PRIMARY KEY,
    ciphertext BLOB NOT NULL,
    nonce      BLOB NOT NULL,
    updated_at INTEGER NOT NULL
  );
  ```
  Vault values encrypted with the service's own DEK (DEK itself encrypted by KEK).

- [x] Wire `syneroym:vault/reveal` host function in `engine.rs`:
  - `HostState.service_id` → `KeyStore::load_dek` → AES-256-GCM decrypt
    vault row (using the `ServiceStore`) → return `list<u8>` to guest.
  - Secret bytes must not appear in any log output at any level.

- [x] Add `roymctl secret set <service-id> <key>` command reading value from
  stdin (never from CLI args) and writing to vault via substrate management API.

**Tests:**

- [x] Unit: `KeyStore` generate/encrypt/decrypt DEK round-trip.
- [x] Unit: `rotate_kek` re-encrypts all DEKs; old KEK bytes zeroed on drop.
- [x] Unit: path traversal guard rejects `"../../etc/passwd"` and `"../x"`.
- [x] Unit: `vault/reveal` returns correct secret; `not-found` for unknown key.
- [x] Integration: production profile, no KEK → `EncryptionKeyRequired`.
- [x] Integration: dev profile, encryption disabled → insecure warning in logs.
- [x] Integration: inject KEK → DB opens; data survives substrate restart.

#### Acceptance Criteria

- Each service has an isolated SQLite file; no cross-service data access.
- DEK stored encrypted in `substrate.db`; plaintext DEK never written to disk.
- KEK held in `mlock`'d RAM, zeroed on drop.
- All path traversal negative vectors rejected with structured `Err`, no panics.
- Production insecure warning present in logs when encryption disabled.
- `vault/reveal` delivers secret bytes; secret is never logged.

---

### [x] Slice 3A: Data-Layer Host Functions

**Requirement IDs:** `[PLT-DAT]` (CRUD, batch, schema init)
**ADR references:** [ADR-0007](../../../decisions/0007-data-layer-wit-interface.md)
**Depends on:** Slice 2A complete.

#### Tasks

**Schema Lifecycle Hooks:**

- [x] Add `init()` lifecycle call in the WASM deployment path in `engine.rs`:
  - Invoked on **first deploy only** (fresh database).
  - Sets `HostState.is_init_context = true`.
  - Guest uses `execute-ddl` for `CREATE TABLE`, `CREATE INDEX`, seed inserts.
  - DDL validated with `EXPLAIN <sql>` before execution (syntax check, no mutation).

- [x] Add `migrate()` lifecycle call in the WASM re-deployment path in `engine.rs`:
  - Invoked on **re-deploy** (existing database; `init()` is skipped).
  - Also sets `HostState.is_init_context = true`.
  - Guest uses `execute-ddl` for `ALTER TABLE ADD COLUMN`, new `CREATE INDEX`,
    and data transformations.
  - Full snapshot/rollback safety net is **deferred to M5** `[LFC-VER]`; document
    this constraint in a `// TODO(M5)` comment at the call site.
  - `execute-ddl` forwards SQL only when `is_init_context`; returns
    `data-layer-error::permission-denied` from normal invocation context.

**Collection Lifecycle:**

- [x] `create-collection(schema)` — generates `CREATE TABLE IF NOT EXISTS <name>
  (id TEXT PRIMARY KEY, payload JSON NOT NULL, creator_id TEXT NOT NULL,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL)` plus
  `CREATE INDEX IF NOT EXISTS` for each indexed field via `json_extract`.

- [x] `drop-collection(name)` — `DROP TABLE IF EXISTS <name>` via single writer.

**CRUD Operations:**

- [x] `put` (upsert), `patch` (JSON merge-patch, implemented in Rust per RFC
  7396 rather than SQLite's `json_patch()` — see status.md), `get`,
  `delete`, `delete-many`.

- [x] `query` with:
  - MongoDB-style JSON filter document parsed from the `filter: option<string>`
    field in `query-options`.
  - Operators supported: equality, `$gt`/`$gte`/`$lt`/`$lte`/`$ne`,
    `$in`/`$nin`, `$regex` (compiled to LIKE), `$and`/`$or`/`$not`, dot-notation
    path access (`json_extract`).
  - All extracted values bound as `?` parameters — no string interpolation.
  - Maximum nesting depth guard: 10 levels; deeper documents return
    `data-layer-error::schema-violation("query document too deeply nested")`.
  - Unsupported operators return `data-layer-error::schema-violation("unsupported
    operator: <op>")`.
  - Cursor pagination using `id > ?` ordering.

- [x] `batch-mutate` — all mutations in a single `BEGIN`/`COMMIT` transaction
  through the single writer; rollback entire transaction on any failure.

**Host-Injected Fields:**

- [x] `creator_id` is set from `HostState.component_id`; cannot be overridden
  by the WASM guest (the host always overwrites the field before write).
- [x] `created_at` is set by the host clock (Unix ms) on first `put`; immutable thereafter.
- [x] `updated_at` is set by the host clock (Unix ms) on every `put` or `patch`;
  guest-supplied value is silently discarded.

**Tests:**

- [x] Unit: `put`/`get`/`patch` correctness.
- [x] Unit: `get` returns `Ok(None)` for missing record (not `Err`).
- [x] Unit: `query` with various MongoDB operators (`$gt`, `$in`, `$regex`,
  `$and`, dot-notation).
- [x] Unit: `query` returns empty list when no records match (not `Err`).
- [x] Unit: cursor pagination returns correct disjoint pages.
- [x] Unit: `batch-mutate` rolls back all mutations on one failure.
- [x] Unit: `execute-ddl` succeeds in `init()` and `migrate()` contexts;
  returns `permission-denied` from normal invocation context.
- [x] Unit: `delete-many` returns correct affected row count.
- [x] Unit: unsupported MongoDB operator returns structured `schema-violation` error.
- [x] Unit: SQL injection attempt via filter JSON value is safely bound; no injection.
- [x] Unit: filter document nested > 10 levels returns `schema-violation`.
- [x] Unit: `updated_at` is host-injected; guest-supplied value is discarded.
- [x] Integration: test WASM component calls `create-collection`, puts 100
  records, queries all, verifies count.
- [x] Integration: `creator_id` from guest is overridden by host value (verified
  as host-supplied, since `record-write-value` has no `creator_id` field for a
  guest to set in the first place — see status.md).
- [x] Integration: `migrate()` called on re-deploy adds a new column; existing
  records accessible after migration.

#### Acceptance Criteria

- All CRUD and `batch-mutate` operations correct against isolated encrypted DB.
- MongoDB filter compilation never interpolates untrusted guest input.
- `execute-ddl` strictly gated to `init()` or `migrate()` context.
- `creator_id`, `created_at`, and `updated_at` are substrate-injected; guest values ignored.
- `get` returns `Ok(None)` for missing records; `query` returns empty list — never `Err(not-found)`.
- `migrate()` is invoked on re-deploy; `init()` only on first deploy.

---

### [x] Slice 4: Service Configuration Delivery

**Requirement IDs:** `[FND-CFG]`
**ADR references:** [ADR-0008](../../../decisions/0008-config-host-function.md)
**Depends on:** Slice 1 complete. *(Can develop concurrently with Slices 2A-3A.)*

#### Tasks

**WIT Interface:**

+ [x] Create `crates/bindings/wit/app-config.wit`:
  - `interface app-config` with:
    - `get(key: string) -> result<option<string>, config-error>` — `Ok(None)` on missing key.
    - `get-section(prefix: string) -> result<list<tuple<string, string>>, config-error>`.
  - `variant config-error { internal(string) }` — no `not-found` variant; missing
    keys return `Ok(None)`.
  - World: `world app-config-guest { import app-config; }`.

**Configuration Generation Store:**

+ [x] Add `config_generations` table to `substrate.db`:
  ```sql
  CREATE TABLE IF NOT EXISTS config_generations (
    service_id  TEXT    NOT NULL,
    generation  INTEGER NOT NULL,
    config_blob TEXT    NOT NULL,
    created_at  INTEGER NOT NULL,
    PRIMARY KEY (service_id, generation)
  );
  ```
+ [x] On `deploy` (in `control_plane::service`), flatten the service's `custom_config` from the manifest into
  a key-value JSON map, store as `generation = max(generation) + 1` (1 for first
  deploy) via `storage_provider`.

**Host Function:**

+ [x] Wire `syneroym:app-config/get` and `syneroym:app-config/get-section` in `engine.rs`:
  - `build_store_and_instantiate` resolves the active generation dynamically per-invocation via `storage_provider.get_latest_config_generation()`.
  - `get`/`get-section` resolves `HostState.service_id` + `HostState.config_generation` → reads
    `config_generations` in `substrate.db` → returns value for `key`. If no generation exists, returns `Ok(None)` or `Ok(vec![])`.
  - Configuration is immutable for the lifetime of a single invocation.

**Schema Validation:**

+ [x] If `ServiceManifest.config.schema_path` is set, validate `custom_config`
  against the JSON Schema using the `jsonschema` crate at deploy time (inside `control_plane::service::deploy`); fail
  deployment with a structured error listing all violations before saving the new generation.

**Podman Compatibility Mode:**

+ [x] For `container` service types, resolve the active generation and inject
  non-secret values as env vars into the Podman container spec.

**Out-of-Band Rotation Policy:**

+ [ ] Add `rotation_policy` to `ServiceConfig` (mapped to `rotation-policy` WIT variant):
  - `"restart-on-rotation"` (default): orchestrator queues graceful restart on
    out-of-band vault secret update.
  - `"none"`: no automatic restart.

**Tests:**

+ [x] Unit: `config/get` returns `Ok(Some(value))` for existing key.
+ [x] Unit: `config/get` on missing key returns `Ok(None)` (not `Err`).
+ [x] Unit: re-deploy bumps generation; in-flight invocations retain prior generation.
+ [x] Unit: JSON Schema validation rejects invalid `custom_config` at deploy time.
+ [ ] Unit: Podman injects configuration correctly into env vars.
+ [x] Integration: two WASM components with different configs get isolated values.

#### Acceptance Criteria

- `config/get` returns `Ok(Some(value))` for present keys and `Ok(None)` for missing keys.
- New deploy bumps generation; in-flight invocations retain prior generation.
- JSON Schema validation fires at deploy time, not at runtime.
- Podman defaults to tmpfs for secret isolation.

---

### [x] Slice 5 (M3B): Blob Object Service

**Requirement IDs:** `[PLT-DAT]` (object service sub-requirement)
**ADR references:** [ADR-0009](../../../decisions/0009-blob-storage-object-store.md)
**Depends on:** M3A exit criteria fully met.

> **Scope note (user-directed, recorded here and in `status.md`):** this
> slice's actual scope grew beyond the checklist below during implementation
> planning: (1) the WIT interface is streaming-first (a hand-rolled
> `blob-writer`/`blob-reader` resource pair), with one-shot `put-blob`/
> `get-blob` kept as thin convenience wrappers over the same streaming
> primitives, not the sole interface; (2) **native (non-WASM) JSON-RPC
> dispatch** was added for data-layer, vault, app-config, *and* blob-store —
> this retroactively closes a Slice 3A gap (data-layer had no native-callable
> path at all) and is new infrastructure spanning `crates/rpc`,
> `crates/router`, and `crates/control_plane`, not just `crates/blob-store`.
> Slice 3A's own checklist/acceptance criteria above are intentionally left
> unmodified — this is additive infrastructure, not a redo. See "Additional
> Slice 5 Tasks (Native Dispatch)" below.
> HTTP serving (the "HTTP Serving" section below) was explicitly deferred —
> see that section for why.

#### Tasks

**WIT Interface:**

+ [x] Create `crates/bindings/wit/blob-store/blob-store.wit`:
  - `interface blob-store` with a streaming-first shape: `resource
    blob-writer { write, finish, abort }`, `resource blob-reader { read }`,
    `open-upload() -> result<blob-writer, blob-error>`, `open-download(hash,
    offset: u64) -> result<blob-reader, blob-error>` (the `offset` param is
    an addition beyond the original spec, for ranged/resumed reads) — plus
    one-shot `put-blob(data: list<u8>) -> result<string, blob-error>` and
    `get-blob(hash: string) -> result<list<u8>, blob-error>` as thin
    wrappers over the same streaming primitives, `delete-blob(hash: string)
    -> result<_, blob-error>`, `signed-url(hash: string, ttl-secs: u32) ->
    result<string, blob-error>`.
  - `variant blob-error { not-found, quota-exceeded, internal(string) }`.
  - Host-side resources are mapped via `with:` in
    `crates/bindings/src/host.rs`'s `bindgen!` call to concrete
    `syneroym_blob_store::{HostUploadSession, HostDownloadSession}` newtypes
    — the first custom (non-WASI) resource type in this codebase's own WIT
    interfaces; see `status.md` for how this was verified.

**Blob Service Crate:**

+ [x] Define generic `BlobProvider` trait in `crates/blob-store/src/traits.rs`
  (session-oriented: `open_upload`/`open_download` return
  `UploadSession`/`DownloadSession` trait objects, with `put_blob`/`get_blob`
  as default-provided one-shot wrappers) to ensure the blob backend is pluggable.
+ [x] Implement `ObjectStoreBlobProvider` in `crates/blob-store/src/object_store_impl.rs` backed by `Arc<dyn ObjectStore>` from the `object_store` crate:
  - Backend switchable via config: `LocalFileSystem` for dev/tests;
    `AmazonS3` for production, gated behind a non-default `aws` cargo
    feature (see the `object_store`/`digest` version-pin comment in the
    root `Cargo.toml` — `object_store` 0.14's `md-5` dependency requires
    stable `digest ^0.11.0`, conflicting with the `digest 0.11.0-rc.10` pin
    already required by `iroh-base`'s pre-release `ed25519-dalek`/`pkcs8`
    chain; pinned to `object_store 0.13.x`, the newest line that still
    resolves, with `aws` off by default until that pin is lifted).
  - Tests use `object_store::memory::InMemory` (via the crate's own
    `ObjectStoreBlobProvider::in_memory` helper).
  - Blobs stored at path `<service_id>/<aa>/<remaining-62-hex-chars>`
    (two-level directory prefix for filesystem balance).
  - SHA-256 verified on write and on read (detect silent corruption).
  - Path traversal guards: `service_id` matches `^[a-zA-Z0-9_:\-]{1,128}$`
    (colon added — real `service_id`s are DIDs, matching the Slice 3A
    `SqliteStorageProvider` fix); `hash` must be exactly 64 lowercase hex
    chars (`^[0-9a-f]{64}$`).
  - Use `Path::join` + `starts_with` for path descendant verification
    (`LocalFileSystem` backend only, defense in depth on top of the regex
    guards above, which make traversal structurally impossible from
    validated input); do **not** use `Path::canonicalize`.
  - Upload sessions buffer in memory (bounded by `max_blob_bytes`); download
    sessions stream via `object_store`'s `GetResult::into_stream()`
    (never `.bytes()`), so an `offset` deep into a large blob doesn't force
    full-object buffering.
  - Per-service aggregate quota (`max_service_total_bytes`, optional) is
    lazily loaded via one `list()` per service on first touch, then
    maintained incrementally in memory.

+ [x] Blob encryption at rest (`[FND-SEC]`): when `encryption = true`, encrypt
  blob content before handing bytes to the `object_store` backend using
  segmented streaming AEAD — `aead::stream` `StreamBE32` over AES-256-GCM,
  256 KiB segments, per-blob subkey derived via HKDF-SHA256 from the service
  DEK (random 32-byte salt in the ciphertext header) — and decrypt on read.
  SHA-256 content addressing and integrity verification apply to the
  plaintext. Encryption mode is a per-service deployment property (reuses
  the existing `storage.encryption` flag via the new
  `StorageProvider::load_service_dek` trait method — no new config toggle);
  no magic sniffing on read. See
  [ADR-0009 Amendment 1](../../../decisions/0009-blob-storage-object-store.md).

**HTTP Serving** (if in scope per D-03-04):

+ [ ] HMAC-SHA256 presigned URLs with configurable TTL. Substrate serves at
  `GET /blobs/<hash>?sig=<hmac>&exp=<unix-ts>`.
  **Deferred, user-confirmed.** `signed-url()` itself *is* implemented
  (HMAC-SHA256, HKDF-derived from the service DEK) and unit-tested
  end-to-end (`crates/blob-store/src/crypto.rs`'s `sign_url`/
  `verify_signed_url`), but no live HTTP endpoint resolves the URL — the
  user's direction was that blob access should work like the data layer,
  through the WIT/native JSON-RPC surface, not a raw HTTP interface. A
  generic HTTP-verb/path-routing bridge in `crates/router` is tracked as a
  follow-up (see "Deferred: HTTP Passthrough" below), not built this slice.

**Tests:**

+ [x] Unit: `put-blob` + `get-blob` round-trip verifies SHA-256.
+ [x] Unit: with encryption enabled, bytes at rest in the backend are
  ciphertext (plaintext not found); round-trip decrypts correctly.
+ [x] Unit: truncated ciphertext (missing final segment) rejected via the
  STREAM last-block flag; reordered segments rejected via the nonce counter.
+ [x] Unit: corrupted blob detected on read.
+ [x] Unit: path traversal via crafted hash or service ID rejected.
+ [x] Unit: blob quota exceeded returns structured error (both single-blob
  fail-fast-mid-upload and aggregate per-service cases).
+ [x] Integration: two services cannot read each other's blobs (namespace isolation).
+ [x] Integration (WIT-resource level, not live HTTP): `signed_url`/
  `verify_signed_url` valid URL accepted; expired URL and tampered
  signature rejected (see HTTP Serving deferral note above for why there's
  no live-endpoint variant of this test).
+ [x] Integration: WASM-path put/get/delete/streaming round trip through the
  real `Host`/`HostBlobWriter`/`HostBlobReader` resource wiring
  (`crates/app_sandbox/tests/blob_store_integration.rs`).
+ [x] Integration: native (non-WASM) JSON-RPC round trip — deploy, dispatch
  data-layer and blob-store (one-shot and full streaming session) calls
  with no WASM component involved, undeploy removes the registration
  (`crates/control_plane/src/service.rs`'s
  `test_native_dispatch_data_layer_and_blob_store_round_trip`).

#### Acceptance Criteria

- `put-blob`/`get-blob` round-trip verifies content integrity by SHA-256.
- With `encryption = true`, blob content at rest is DEK-encrypted; plaintext
  never reaches the storage backend.
- Service blob namespaces are isolated.
- No path traversal possible via crafted input.

---

### [x] Additional Slice 5 Tasks (Native Dispatch — user-directed scope expansion)

Not in the original checklist; added during Slice 5 planning at the user's
explicit direction (see `status.md` for the full rationale). Tracked here so
the traceability record reflects what was actually built.

+ [x] `crates/rpc`: `NativeDispatchRegistry` type alias
  (`Arc<DashMap<String, Arc<dyn NativeService>>>`) shared between
  `RouteHandler` and `ControlPlaneService`.
+ [x] `crates/control_plane/src/synsvc_native.rs`: `SynSvcNativeService`,
  one instance per deployed `service_id`, dispatching `data-layer`/`vault`/
  `app-config`/`blob-store` JSON-RPC calls onto the same
  `StorageProvider`/`ServiceStore`/`BlobProvider` traits the WASM `Host`
  impls in `engine.rs` use — a second adapter, not a reimplementation.
+ [x] `ControlPlaneService::deploy`/`undeploy`: register/deregister the 4
  native-capability `EndpointRegistry` interfaces plus the `native_dispatch`
  entry for every deployed service, regardless of `service-type`
  (wasm/container/tcp).
+ [x] Bug found and fixed during this work: `ControlPlaneService::list()`
  picked `endpoint_type` from whichever registered interface was iterated
  first, which broke (`"native"` instead of `"wasm"`) once every deployed
  service also had 4 native-capability interfaces registered. Fixed by
  excluding those 4 interfaces from `list()`'s enumeration entirely — see
  `crates/substrate/tests/basic_lifecycle.rs`'s
  `test_substrate_lifecycle_scenarios`, which caught this.

### [ ] Slice 6A (M3B): Messaging WIT and Embedded Pub/Sub Broker

**Requirement IDs:** `[PLT-DAP-04]`, `[PLT-DAT]` (MQTT event service sub-requirement)
**ADR references:** [ADR-0010](../../../decisions/0010-mqtt-broker-rumqttd.md) (see amendment for the `syneroym:messaging` rename)
**Depends on:** Slice 5 complete.

> Scope note: this slice implements only the pub/sub half of
> `syneroym:messaging`. The bidirectional-streaming half — both directions,
> guest-as-source (`stream-cursor`, `handle-stream-request`) and
> guest-as-sink (`stream-sink`, `accept-stream-upload`) — plus
> `register-stream-protocol`, is declared in the same WIT file for interface
> stability but wired up in Slice 6B (M3C), which needs new host-side QUIC
> stream routing this slice does not build. `syneroym:messaging` was
> formerly planned as a standalone `syneroym:pubsub` package — renamed so
> pub/sub and streaming can share one WIT boundary from Day 1 without a
> breaking v0.2 rename later.

#### Tasks

**WIT Interface:**

+ [ ] Create `crates/bindings/wit/messaging/messaging.wit`, package
  `syneroym:messaging@0.1.0`, split into three interfaces:
  - `interface host-api` (host-imported, guest-triggered):
    `publish(topic: string, payload: list<u8>) -> result<_, string>`,
    `subscribe(topic: string) -> result<_, string>`,
    `unsubscribe(topic: string) -> result<_, string>`,
    `register-stream-protocol(protocol: string) -> result<_, string>`
    (signature declared this slice; unimplemented — see Slice 6B).
  - `interface stream-types`:
    `resource stream-cursor { next-chunk: func() -> option<list<u8>>; }`
    (guest-as-source, pull direction) and
    `resource stream-sink { push-chunk: func(data: list<u8>) -> result<_,
    string>; finalize: func() -> result<_, string>; }` (guest-as-sink, push
    direction). Both declared this slice; unimplemented — see Slice 6B.
  - `interface guest-api` (guest-exported, host-triggered):
    `handle-message(topic: string, payload: list<u8>) -> result<_, string>`
    (this slice's delivery path — replaces the earlier `on-message` naming
    from ADR-0010's original text), `handle-stream-request(peer-id: string,
    request-data: list<u8>) -> result<stream-cursor, string>` (guest-as-source),
    and `accept-stream-upload(peer-id: string, metadata: string) ->
    result<stream-sink, string>` (guest-as-sink). The latter two declared
    this slice; unimplemented — see Slice 6B.
  - World: `world messaging-guest { import host-api; export guest-api; }`.
  - Delivery remains push-model: host invokes the component's exported
    `guest-api::handle-message` if declared; if not declared, the
    subscription is registered but messages are silently discarded (same
    behavior as ADR-0010, renamed export).

**Broker Embedding:**

+ [ ] Add `rumqttd` (latest stable; pin version with rationale comment per workspace
  convention) and `tokio-util` (if not already present) to `Cargo.toml`.
+ [ ] Create `crates/mqtt-broker/` crate with `MqttBroker`:
  - Starts `rumqttd` as a Tokio background task bridged by a **bounded channel**
    (default capacity: 1024 messages, configurable via `[mqtt].channel_capacity`).
  - When the channel is full, `publish` returns
    `Err("broker channel full: backpressure")` — never blocks
    the Wasmtime execution thread.
  - Exposes `publish(topic, payload)` and `subscribe(topic, sender)` async APIs.
  - Uses `CancellationToken` to cleanly terminate on `Drop` (applying the M2
    epoch timer audit lesson).
  - Topics are namespaced by the host: guest topic `t` published by service `s`
    becomes `svc/s/t` in `rumqttd`. Cross-service subscriptions require explicit
    `svc/<other_service_id>/...` topic strings.

**Host Function Wiring:**

+ [ ] Wire `syneroym:messaging/host-api.publish` in `engine.rs` → `MqttBroker::publish`.
+ [ ] Wire delivery: broker message → host invokes component `guest-api::handle-message`
  export via Wasmtime invocation path (if the component declares that export).

**Wildcard and Retained Messages:**

+ [ ] Verify `rumqttd` supports MQTT `+` and `#` wildcards; enable in config.
+ [ ] Verify retained messages delivered to new subscribers after publish.

**Tests:**

+ [ ] Unit: `publish` + `subscribe` on same topic delivers message.
+ [ ] Unit: wildcard `sensors/+/temp` matches `sensors/room1/temp`.
+ [ ] Unit: retained message delivered to subscriber joining after publish.
+ [ ] Unit: `CancellationToken` terminates broker task on `Drop` (no leak).
+ [ ] Integration: two WASM components in different services exchange a message.
+ [ ] Integration: MQTT topic namespacing — service A cannot receive messages
  published in service B's namespace (without explicit opt-in).
+ [ ] Integration: channel backpressure — when the bounded channel is saturated,
  `publish` returns the backpressure error without blocking or crashing the substrate.
+ [ ] Unit: broker task terminates within 1 second of `CancellationToken` cancellation.

#### Acceptance Criteria

- `publish`/`subscribe` round-trip works within a single service.
- Wildcard topics and retained messages function correctly.
- Broker task terminates cleanly on substrate shutdown.
- Cross-service MQTT namespace isolation enforced.
- `syneroym:messaging@0.1.0` WIT compiles with both `host-api`/`guest-api`
  pub/sub functions wired and the streaming resource/functions declared
  but unimplemented (returning a structured "not yet implemented" error if
  called), so Slice 6B is purely additive.

---

### [ ] Slice 6B (M3C): Bidirectional Streaming

**Requirement IDs:** `[PLT-DAP-06]`
**ADR references:** New design note/ADR required before implementation (see M3C dependency gate); documents host-side QUIC stream acceptance and routing.
**Depends on:** Slice 6A complete; M3B exit criteria met.

#### Tasks

**Stream Protocol Registration:**

+ [ ] Wire `syneroym:messaging/host-api.register-stream-protocol` in
  `engine.rs`: records `(service_id, protocol)` in a host-side routing
  table so inbound peer streams on that namespace can be dispatched to the
  right guest instance.

**Inbound Stream Routing (new host infrastructure):**

+ [ ] Design and implement acceptance of peer-initiated QUIC streams
  against a registered protocol namespace (likely `crates/coordinator_iroh`
  and/or `crates/router`) — this is new infrastructure with no existing
  precedent in the codebase; requires the design note/ADR from the M3C
  dependency gate before implementation starts.
+ [ ] The design note must specify how the host distinguishes a download
  request (route to `handle-stream-request`) from an upload push (route to
  `accept-stream-upload`) on the same registered protocol namespace — e.g. a
  direction flag or small header in the peer's initial frame, decided and
  documented once, not per-slice.
+ [ ] Route an accepted download-request stream's initial request payload to
  `guest-api::handle-stream-request(peer-id, request-data)`, and an accepted
  upload stream's initial metadata to `guest-api::accept-stream-upload
  (peer-id, metadata)`, through the normal Wasmtime invocation path.

**Stateful Iterator — Guest as Source (Host Pull Loop):**

+ [ ] Implement the `stream-cursor` resource on the guest side (bindings
  generation) and the host-side pull loop: a Tokio task that calls
  `next-chunk()` in a loop, transmits each chunk over the Iroh QUIC stream,
  and stops on `none` (EOF), closing the stream and dropping the resource
  handle. Model this on the `blob-writer`/`blob-reader` resource pattern
  from Slice 5 (`crates/blob-store`), the first precedent for a custom
  (non-WASI) resource type in this codebase's WIT interfaces.
+ [ ] Apply backpressure consistent with `[PLT-DAP-05]`'s flow-control
  approach where the QUIC transport allows it, but note this is a distinct,
  simpler interface from `syneroym:data/stream` (no Arrow record batches,
  no DataFusion integration) — see architecture doc §2 vs §4 disambiguation.

**Stateful Sink — Guest as Sink (Host Push Loop):**

+ [ ] Implement the `stream-sink` resource on the guest side (bindings
  generation) and the host-side push loop: a Tokio task reads chunks off the
  incoming Iroh QUIC stream and synchronously calls `push-chunk(data)` for
  each one; when the QUIC stream closes (peer signals EOF), the host calls
  `finalize()` so the guest can commit its write (e.g. flush a
  `data-layer`/`blob-store` write session) and release state.
+ [ ] `push-chunk` returning `Err` aborts the upload: the host stops reading
  from the QUIC stream, does **not** call `finalize`, and resets/closes the
  stream so the peer observes a clean failure rather than a hang.
+ [ ] A guest that declines the upload (`Err` from `accept-stream-upload`)
  causes the host to close the incoming QUIC stream immediately, without
  creating a `stream-sink` or reading any payload bytes.

**Tests:**

+ [ ] Unit: `register-stream-protocol` records the namespace; duplicate
  registration for the same service is idempotent or returns a structured
  error (decide and document in the design note).
+ [ ] Unit: `stream-cursor.next-chunk()` returning `none` closes the QUIC
  stream and drops host-side state (no leak).
+ [ ] Unit: `stream-sink.finalize()` is called exactly once, only after the
  QUIC stream closes cleanly (not on abort).
+ [ ] Unit: `push-chunk` returning `Err` aborts the upload without invoking
  `finalize`; host-side state is dropped (no leak).
+ [ ] Integration: two WASM components in different services exchange a
  file-transfer-style byte stream end to end via `handle-stream-request`/
  `stream-cursor` (mirrors the design note's worked example).
+ [ ] Integration: two WASM components in different services exchange a
  file-transfer-style byte stream end to end via `accept-stream-upload`/
  `stream-sink` (upload direction).
+ [ ] Integration: a guest that does not export `handle-stream-request`
  causes the host to reject an inbound download request cleanly (no panic,
  no hang); a guest that does not export `accept-stream-upload` does the
  same for an inbound upload.
+ [ ] Integration: cross-service stream namespace isolation — a peer cannot
  address another service's registered protocol without going through the
  substrate's routing.

#### Acceptance Criteria

- `register-stream-protocol` → peer opens matching QUIC stream →
  `handle-stream-request` is invoked → returned `stream-cursor` is pulled by
  the host until EOF → QUIC stream closes cleanly, end to end.
- `register-stream-protocol` → peer opens matching QUIC stream to upload →
  `accept-stream-upload` is invoked → returned `stream-sink` is pushed into
  by the host until the peer closes the stream → `finalize()` is called
  exactly once → QUIC stream closes cleanly, end to end.
- No panics or hangs when a guest declines a stream request/upload or fails
  to export `handle-stream-request`/`accept-stream-upload`.
- An aborted upload (`push-chunk` error) never calls `finalize` and leaves
  no dangling host-side state.
- Streaming and pub/sub share the `syneroym:messaging@0.1.0` package with no
  breaking change to the Slice 6A surface.

---

### [ ] Slice 7 (M3C): HTTP Passthrough

**Requirement IDs:** `[PLT-DAT]`, `[PLT-DAP-04]`, `[PLT-DAP-06]` (HTTP-facing translation of data-layer, blob-store, and messaging)
**Depends on:** Slice 6B complete (needs both pub/sub and streaming, not pub/sub alone — this is why it moved to M3C instead of shipping as a follow-up inside M3B).

> Formerly tracked as "Deferred: HTTP Passthrough" at the end of M3B Slice 5.
> The user's original direction: build this "soon" as its own follow-up once
> native-dispatch plumbing (Slice 5) existed. It is now sequenced after
> messaging (6A+6B) rather than right after Slice 5, because HTTP access
> needs to translate onto pub/sub-style subscription (SSE/long-poll) and
> streaming uploads/downloads, not just request/response CRUD and blob
> get/put.

#### Tasks

**HTTP Verb / Path Routing:**

+ [ ] Extend `crates/router/src/route_handler/http.rs` (currently
  JSON-RPC-over-POST only) to translate HTTP method + path + query + body
  into a native or WASM call, and stream the response back as raw bytes
  rather than JSON-RPC envelope, covering at minimum:
  - `GET` → `data-layer::get`/`query` (DB access via REST-like conventions)
    or `blob-store::get-blob`/streamed `blob-reader` (signed-URL blob
    serving, static file access) depending on route configuration.
  - `POST`/`PUT` (small body) → `data-layer::put`/`patch` or
    `messaging::publish`.
  - `PUT`/chunked upload (large body) → `messaging::accept-stream-upload`/
    `stream-sink` (Slice 6B): the router treats the HTTP body as an inbound
    stream, translating chunked-transfer-encoding reads into `push-chunk`
    calls and end-of-body into `finalize()`. `stream-sink` resolved the
    guest-as-sink gap in Slice 6B, so this task is direct wiring, not new
    resource design. Where the upload target is specifically a blob (not a
    guest-defined sink), the router may instead call `blob-store`'s
    existing `blob-writer` directly — decide per-route via configuration,
    not a global policy.
  - `GET` with `Accept: text/event-stream` (or WebSocket upgrade) →
    `messaging::subscribe`, bridging push-model `handle-message` delivery
    onto SSE frames or WebSocket messages for the life of the connection.

**Tests:**

+ [ ] Integration: `GET /blobs/<hash>?sig=<hmac>&exp=<unix-ts>` resolves the
  previously-implemented `signed-url`/`verify_signed_url` logic
  (`crates/blob-store/src/crypto.rs`) end to end over a live HTTP endpoint —
  closes the gap explicitly left open in Slice 5's "HTTP Serving" section.
+ [ ] Integration: `GET` static file passthrough serves blob content with
  correct `Content-Type`/`Content-Length`.
+ [ ] Integration: `POST` JSON body passthrough performs a `data-layer::put`
  and returns the resulting record.
+ [ ] Integration: SSE/long-poll `GET` receives messages published via
  `messaging::publish` from another connection.
+ [ ] Integration: chunked `PUT` upload round-trips through
  `accept-stream-upload`/`stream-sink` with content integrity verified end
  to end (HTTP client → router → guest `finalize()`).
+ [ ] Integration: a guest that declines the upload (`Err` from
  `accept-stream-upload`) surfaces as a structured HTTP error (e.g. `4xx`),
  not a hung connection or a `5xx` with no explanation.
+ [ ] Integration: malformed or oversized request rejected with a structured
  HTTP error, not a panic.

#### Acceptance Criteria

- Signed-URL blob serving and static file access work over plain HTTP `GET`,
  closing the deferral from Slice 5.
- `data-layer` and `messaging` are reachable over HTTP using conventional
  verbs, without requiring a JSON-RPC envelope.
- Chunked HTTP upload is wired end-to-end onto `stream-sink`/
  `accept-stream-upload` (or `blob-store`'s `blob-writer` for blob-typed
  routes), with no open design question left for this direction.
- No regression to the existing JSON-RPC-over-POST native-dispatch path.

---

## Reference Scenario: Encrypted Data Lifecycle (M3A)

1. Operator starts substrate with `encryption = true`. No KEK injected.
2. Operator deploys test WASM service `profile-store` with
   `custom_config = { db_name = "profiles" }`.
3. Deployment succeeds; configuration generation 1 stored.
4. Operator invokes `profile-store` — receives `StorageError::EncryptionKeyRequired`.
5. Operator injects the substrate-global KEK via `roymctl kek inject` (reads
   from stdin; one injection unlocks all service DEKs per ADR-0006).
6. Operator re-invokes `profile-store`. WASM `init()` runs: `create-collection`
   creates `profiles` table; `execute-ddl` adds an index.
7. Service puts 3 profile records; queries with an equality filter
   (`{"name": "..."}`); receives correct results.
8. Service calls `config/get("db_name")` → returns `"profiles"`.
9. Operator sets secret: `roymctl secret set profile-store api_key` (stdin).
10. Service calls `vault/reveal("api_key")` → receives secret bytes; not logged.
11. Operator rotates KEK (`roymctl kek rotate`). All DEKs re-encrypted atomically.
    Service continues operating with existing DB.
12. `roymctl status` shows service healthy; encrypted DB confirmed in health output.

**M3B extension** (blob + messaging pub/sub):

13. Service stores a blob (`put-blob`); records SHA-256 hash in `profiles` collection.
14. Service publishes messaging event `profiles/updated` with the record ID
    (`syneroym:messaging` `host-api::publish`).
15. Second test service subscribed to `profiles/+` receives the event via
    `guest-api::handle-message` and reads the blob by hash.

**M3C extension** (bidirectional streaming + HTTP):

16. Service registers a stream protocol (`host-api::register-stream-protocol("file-transfer")`).
17. A second test service opens a direct stream and requests the blob from
    step 13 by hash; the first service's `handle-stream-request` returns a
    `stream-cursor` that the host pulls until EOF, delivering the blob bytes
    over a direct QUIC stream (not through `data-layer`/`blob-store`).
18. A third test service uploads a new file to the first service by opening
    a direct stream on the same `"file-transfer"` namespace; the first
    service's `accept-stream-upload` returns a `stream-sink`, the host
    pushes the uploaded bytes via `push-chunk`, and `finalize()` commits the
    file — verified by the first service reading it back afterward.
19. An external HTTP client performs `GET /blobs/<hash>?sig=...` against the
    signed URL from step 13 and receives the raw blob bytes; a second HTTP
    client opens an SSE connection and receives the `profiles/updated` event
    from step 14 pushed live; a third HTTP client performs a chunked `PUT`
    upload that is bridged onto `accept-stream-upload`/`stream-sink` the
    same way as step 18.

---

## Failure and Security Tests

| Test | Expected Outcome |
|---|---|
| Production profile, no KEK, first `open_service_db` | `StorageError::EncryptionKeyRequired`; DB not opened |
| `encryption = false` in non-dev profile | Persistent `⚠️ INSECURE` warning in logs on every restart |
| `service_id = "../../etc"` | Rejected before DB path formed; no file system access |
| Blob `hash = "../secret.txt"` | Rejected; must be 64-char lowercase hex |
| `execute-ddl("DROP TABLE _vault")` from normal invocation | `data-layer-error::permission-denied` |
| `vault/reveal` on non-existent key | `vault-error::not-found` |
| SQL injection via filter: `{"name": "'; DROP TABLE profiles; --"}` | Bound as parameterised value; no injection; query returns 0 results |
| WASM guest sets `creator_id` in payload | Host overwrites field; guest value ignored |
| Blob exceeds service quota | `blob-error::quota-exceeded`; no substrate crash |
| Read blob with bit-flipped bytes | `blob-error::internal("integrity check failed")` (AEAD tag failure when encrypted; plaintext SHA-256 mismatch otherwise) |
| Service A publishes to service B's MQTT namespace | Delivery blocked by namespace isolation |
| KEK rotation while service is handling a request | Re-encryption completes; in-flight request uses cached DEK |
| Peer opens a stream against an unregistered protocol namespace | Host rejects the stream cleanly; no panic, no hang |
| Guest declines a `handle-stream-request` (returns `Err`) | Host closes the QUIC stream without invoking `next-chunk` |
| Guest declines an `accept-stream-upload` (returns `Err`) | Host closes the incoming QUIC stream without creating a `stream-sink` or reading payload bytes |
| `push-chunk` returns `Err` mid-upload | Upload aborted; `finalize()` never called; host-side state dropped; peer observes a clean failure, not a hang |
| HTTP request with tampered or expired signed-URL query params | `401`/`403`-equivalent structured error; blob not served |

---

## Performance Budgets

| Metric | Budget | Measurement Method |
|---|---|---|
| `put` (single record, encrypted DB, SQLCipher) | < 5 ms p99 | `criterion` integration bench |
| `get` (single record, cache-warm reader pool) | < 2 ms p99 | `criterion` integration bench |
| `query` (100 records, MongoDB `$eq` filter, encrypted DB) | < 20 ms p99 | `criterion` integration bench |
| `batch-mutate` (50 mutations, single transaction) | < 30 ms p99 | `criterion` integration bench |
| SQLCipher overhead vs. plaintext for `put`/`get` | < 10% latency regression | A/B bench with encryption on/off |
| `vault/reveal` (single secret) | < 2 ms p99 | `criterion` micro-bench |
| KEK rotation (re-encrypt 100 DEKs) | < 500 ms total | Integration test |
| `config/get` (cache-warm, pinned generation) | < 1 ms p99 | `criterion` micro-bench |
| Service DB open time (DEK load + SQLCipher open) | **No budget in M3** — see ADR-0006; budget deferred to M4 | — |
| WASM `init()` hook (10-table DDL) | < 200 ms | Integration test |
| WASM `migrate()` hook (5 ALTER TABLE statements) | < 200 ms | Integration test |
| `put-blob` (1 MB, `object_store` local backend) | < 100 ms p99 | Integration test |
| `get-blob` (1 MB, local cache hit) | < 50 ms p99 | Integration test |
| MQTT `publish` to `subscribe` delivery (same process) | < 5 ms p99 | Integration test |
| `stream-cursor.next-chunk()` round trip (host pull, same process) | < 5 ms p99 | Integration test |
| `stream-sink.push-chunk()` round trip (host push, same process) | < 5 ms p99 | Integration test |
| HTTP `GET` signed-URL blob serve (1 MB) | < 100 ms p99 | Integration test |
| HTTP chunked `PUT` upload (1 MB, via `stream-sink`) | < 150 ms p99 | Integration test |

---

## Tests Summary

### Unit Tests (adjacent to implementation crates)

- `crates/key-store/src/tests.rs` — DEK generate/encrypt/decrypt/rotate/zeroize.
- `crates/data-layer/src/tests.rs` — CRUD, filter compilation, batch, pagination,
  DDL gating, `creator_id` injection, SQL injection resistance.
- `crates/blob-store/src/tests.rs` — SHA-256 integrity, path guards, namespace isolation.
- `crates/mqtt-broker/src/tests.rs` — publish/subscribe, wildcards, retained, cancellation.
- `crates/mqtt-broker/src/tests.rs` (M3C additions) or a new
  `crates/messaging-stream/src/tests.rs` — `register-stream-protocol`,
  `stream-cursor` lifecycle/EOF handling, `stream-sink`
  `push-chunk`/`finalize` lifecycle and abort-on-error handling.

### Integration Tests (`tests/integration/`)

- `encrypted_db.rs` — full lifecycle: deploy → inject KEK → init → CRUD → rotate KEK.
- `vault.rs` — set secret → reveal → key not found.
- `config.rs` — deploy with config → `config/get` → redeploy bumps generation.
- `blob_roundtrip.rs` — put blob → get blob → SHA-256 verified.
- `mqtt_exchange.rs` — two services publish/subscribe across MQTT.
- `stream_exchange.rs` (M3C) — two services exchange a file-transfer-style
  byte stream via `handle-stream-request`/`stream-cursor` (download) and via
  `accept-stream-upload`/`stream-sink` (upload).
- `http_passthrough.rs` (M3C) — signed-URL blob GET, JSON POST to
  data-layer, SSE subscription to messaging, chunked upload round trip via
  `stream-sink`.

### End-to-End Tests (extending `mise run test:e2e`)

- M3A reference scenario (steps 1-12) in a live substrate instance.
- M3B reference scenario (steps 13-15) in a live substrate instance.
- M3C reference scenario (steps 16-19) in a live substrate instance.
- All failure/security tests produce documented outcomes.

---

## Measurable Exit Criteria

### M3A Exit Criteria

All of the following must be verified and recorded in `status.md`:

+ [ ] `cargo +nightly fmt --all` passes with zero diff.
+ [ ] `cargo clippy --workspace --all-targets --all-features` passes with zero
  warnings and zero errors.
+ [ ] `cargo test --workspace` passes with all tests green.
+ [ ] `mise run test:e2e` passes (existing e2e scenarios must not regress).
+ [ ] `cargo build --target wasm32-wasip2 -p syneroym-bindings` exits 0.
+ [ ] `syneroym:data-layer@0.1.0`, `syneroym:vault@0.1.0`, and
  `syneroym:app-config@0.1.0` WIT packages compile and generate valid Rust bindings.
+ [ ] M3A reference scenario (steps 1-12) executes end-to-end without error.
+ [ ] All M3A failure/security tests produce documented outcomes.
+ [ ] Performance budgets for M3A metrics (rows 1-10 in table) verified;
  `criterion` output captured in `status.md`.
+ [ ] DEK never appears in plaintext on disk; verified by hex dump of `substrate.db`.
+ [ ] Decisions D-03-01, D-03-02, D-03-03 resolved as ADRs in `docs/decisions/`.
+ [ ] Traceability matrix updated with M3A evidence for `[PLT-DAT]` (structured
  data), `[FND-SEC]` (storage encryption), and `[FND-CFG]`.

### M3B Exit Criteria (additional, after M3A is closed)

+ [ ] `syneroym:messaging@0.1.0` (pub/sub surface wired; streaming surface
  declared only) and `syneroym:blob-store@0.1.0` WIT packages compile and
  generate valid Rust bindings.
+ [ ] M3B reference scenario (steps 13-15) executes without error.
+ [ ] All M3B failure/security tests produce documented outcomes.
+ [ ] Performance budgets for M3B metrics verified; output captured in `status.md`.
+ [ ] Decisions D-03-04 and D-03-05 resolved as ADRs in `docs/decisions/`.
+ [ ] Traceability matrix updated with M3B evidence for `[PLT-DAT]` (blob and
  MQTT sub-requirements).

### M3C Exit Criteria (additional, after M3B is closed)

+ [ ] `syneroym:messaging@0.1.0` streaming surface — both guest-as-source
  (`stream-cursor`, `handle-stream-request`) and guest-as-sink
  (`stream-sink`, `accept-stream-upload`), plus
  `register-stream-protocol` — is fully wired; WIT package compiles with no
  breaking change to the Slice 6A pub/sub surface.
+ [ ] Design note/ADR for host-side QUIC stream acceptance/routing recorded
  in `docs/decisions/` before Slice 6B was implemented (per the M3C
  dependency gate).
+ [ ] `crates/router/src/route_handler/http.rs` serves `GET`/`POST`/streaming
  `PUT` against `data-layer`, `blob-store`, and `messaging` without a
  JSON-RPC envelope, alongside the existing JSON-RPC-over-POST path
  (no regression).
+ [ ] Guest-as-sink upload direction is implemented and documented (not left
  as an open question).
+ [ ] M3C reference scenario (steps 16-19) executes end-to-end without error.
+ [ ] All M3C failure/security tests produce documented outcomes.
+ [ ] Performance budgets for M3C metrics verified; output captured in `status.md`.
+ [ ] `cargo +nightly fmt --all`, `cargo clippy --workspace --all-targets
  --all-features`, `cargo test --workspace`, and `mise run test:e2e` all
  pass on the M3C branch.
+ [ ] Traceability matrix updated with M3C evidence for `[PLT-DAP-06]`
  (bidirectional streaming) and the HTTP-facing sub-requirements of
  `[PLT-DAT]`/`[PLT-DAP-04]`.

---

## Open Questions (Non-Blocking; Track for M4 Planning)

1. **`AggregationPipeline` scope:** Requirements specify `$group`, `$having`,
   projections. Explicitly deferred to M4. Must appear as a gate item in the
   M4 milestone plan. Design should translate MongoDB aggregation-pipeline
   stages onto SQLite constructs (`GROUP BY`, `HAVING`, views), not invent
   parallel syntax — see [ADR-0007](../../../decisions/0007-data-layer-wit-interface.md).

1a. **Privileged raw-SQL escape hatch:** [ADR-0011](../../../decisions/0011-privileged-raw-sql-query.md)
   (Proposed) specifies a `query-raw` host function for trusted contexts that
   need SQL expressivity beyond the JSON filter DSL, gated the same way as
   `execute-ddl`. Not in M3A scope; must appear as a gate item alongside
   `AggregationPipeline` in the M4 (or M3B) milestone plan.

2. **FDAE `execute-ddl` elevation model:** `is_init_context` flag in `HostState`
   is a temporary M3 scaffold. In M4 this becomes a proper Admin UCAN capability.
   Add a `// TODO(M4): replace is_init_context with Admin UCAN check` comment
   at every usage site.

3. **Podman sandbox integration:** `crates/podman_sandbox` exists but its
   integration with data-layer configuration delivery in M3 is unclear. Verify
   at Slice 4 whether the Podman path calls `StorageManager` directly or only
   receives resolved config values from the orchestrator.

4. **Per-SynApp-Instance KEK:** Deferred to M4. The `KeyStore` API must have
   an extensibility point (e.g., optional `app_instance_id` scope parameter)
   from Day 1 so M4 can narrow the scope without a breaking change. Validate
   this at M3A closeout.
