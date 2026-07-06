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

This milestone is split into two sequential sub-milestones:

- **M3A — Structured State and Security:** Encrypted SQLite isolation (`oltp`/`olap` profiles), full
  data-layer WIT surface, vault integration, and configuration delivery.
- **M3B — Objects and Events:** Blob object service and Decentralized Pub/Sub
  (MQTT API overlay).

M3B may begin only after M3A exit criteria are fully met.

---

## Requirement IDs (Traceability)

| Requirement ID | Description | Sub-requirements targeted in M3 |
|---|---|---|
| `[PLT-DAT]` | Data Layer | Structured SQLite DBs per service (CRUD, batch, filters, aggregation, pagination, nested WIT serialisation, schema init DDL, `oltp`/`olap` build profiles); blob S3-compatible backend and signed HTTP access (M3B) |
| `[PLT-DAP]` | Distributed Data Topology | Logical Data Service interface definitions only (M3A) |
| `[TOP-ROB]` | Decentralized Pub/Sub | MQTT API overlay over QUIC with wildcard topics, retained messages, change notifications (M3B) |
| `[FND-SEC]` | Substrate Security (storage slice) | Envelope Encryption (DEK/KEK); secret vault inside encrypted SQLite; `mlock`-protected KEK RAM; DEK re-encryption on key rotation; production profiles default to encryption; opt-out produces persistent insecure-state warning |
| `[FND-CFG]` | Service Configuration Delivery | `syneroym:config/get` WASM host function; schema validation and defaults at deploy-time; Podman env-var and file-mount fallback; versioned immutable configuration generation; out-of-band secret rotation policy |

> **Out of scope in this milestone (deferred):**
> - `[FND-IAM]` FDAE access control and UCAN integration — **M4**.
> - `[PLT-DAT]` Universal Proxy / wRPC inter-component RPC — **M4**.
> - `[PLT-RED]` WAL replication, redundancy, HA failover — **M7**.
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

- **MQTT protocol abstraction over P2P log replication.** (Implemented initially using `rumqttd` in-process Tokio task for local state, pending full QUIC P2P overlay in M5).
- **Push-model delivery**: host invokes guest-exported `on-message(topic, payload)`.
- Wildcard topics (`+`, `#`) and retained messages **in scope**.
- Cross-service pub/sub **in scope** with topic namespace isolation
  (`svc/<service_id>/<user-topic>`).
- **Bounded channels** (default 1024 messages) with explicit backpressure;
  `publish` returns `pubsub-error::internal("broker channel full")` when saturated.
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
- `cr-sqlite` CRDT extensions (M7+).
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
5. Decisions D-03-04 and D-03-05 resolved before M3B Slices 5-6 begin.

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
| No `syneroym:config` WIT interface or host function | Slice 4 |
| No configuration generation pinning in `HostState` | Slice 4 |
| No schema validation at deploy-time for `custom-config` | Slice 4 |
| No production-mode encryption enforcement warning | Slice 2A |

### Gaps to Close in M3B

| Gap | Target Slice |
|---|---|
| No blob object service crate | Slice 5 |
| No `syneroym:blob-store` WIT interface | Slice 5 |
| No S3-compatible blob backend | Slice 5 |
| No embedded MQTT broker (`rumqttd`) | Slice 6 |
| No `syneroym:pubsub` WIT interface | Slice 6 |
| No MQTT wildcard topic or retained message support | Slice 6 |

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
`syneroym:config@0.1.0`, `syneroym:pubsub@0.1.0` in M3B) are added in M3.

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

### [ ] Slice 4: Service Configuration Delivery

**Requirement IDs:** `[FND-CFG]`
**ADR references:** [ADR-0008](../../../decisions/0008-config-host-function.md)
**Depends on:** Slice 1 complete. *(Can develop concurrently with Slices 2A-3A.)*

#### Tasks

**WIT Interface:**

- [ ] Create `crates/bindings/wit/config.wit`:
  - `interface config` with:
    - `get(key: string) -> result<option<string>, config-error>` — `Ok(None)` on missing key.
    - `get-section(prefix: string) -> result<list<tuple<string, string>>, config-error>`.
  - `variant config-error { internal(string) }` — no `not-found` variant; missing
    key is `Ok(None)`, not an error.
  - World: `world config-guest { import config; }`.

**Configuration Generation Store:**

- [ ] Add `config_generations` table to `substrate.db`:
  ```sql
  CREATE TABLE IF NOT EXISTS config_generations (
    service_id  TEXT    NOT NULL,
    generation  INTEGER NOT NULL,
    config_blob TEXT    NOT NULL,
    created_at  INTEGER NOT NULL,
    PRIMARY KEY (service_id, generation)
  );
  ```
- [ ] On `deploy`, flatten the service's `custom_config` from the manifest into
  a key-value JSON map, store as `generation = max(generation) + 1` (1 for first
  deploy), and set `HostState.config_generation = current_generation` for each
  new invocation.

**Host Function:**

- [ ] Wire `syneroym:config/get` and `config/get-section` in `engine.rs`:
  - Resolves `HostState.service_id` + `HostState.config_generation` → reads
    `config_generations` in `substrate.db` → returns value for `key`.
  - Configuration is immutable for the lifetime of a single invocation.

**Schema Validation:**

- [ ] If `ServiceManifest.config.schema_path` is set, validate `custom_config`
  against the JSON Schema using the `jsonschema` crate at deploy time; fail
  deployment with a structured error listing all violations.

**Podman Compatibility Mode:**

- [ ] For `container` service types, resolve the active generation and inject
  non-secret values as env vars into the Podman container spec. Secrets from
  vault are injected per `secret_mode` (`env` or `tmpfs`). Log `warn!` when
  `secret_mode = env`: `"Degraded secret isolation: secret injected as env var
  for Podman container <service_id>"`.

**Out-of-Band Rotation Policy:**

- [ ] Add `rotation_policy` to `ServiceManifest.config`:
  - `"restart-on-rotation"` (default): orchestrator queues graceful restart on
    out-of-band vault secret update.
  - `"none"`: no automatic restart.

**Tests:**

- [ ] Unit: `config/get` returns `Ok(Some(value))` for existing key.
- [ ] Unit: `config/get` on missing key returns `Ok(None)` (not `Err`).
- [ ] Unit: re-deploy bumps generation; in-flight invocations retain prior generation.
- [ ] Unit: JSON Schema validation rejects invalid `custom_config` at deploy time.
- [ ] Unit: Podman `secret_mode = env` emits degraded-isolation warning.
- [ ] Integration: two WASM components with different configs get isolated values.

#### Acceptance Criteria

- `config/get` returns `Ok(Some(value))` for present keys and `Ok(None)` for missing keys.
- New deploy bumps generation; in-flight invocations retain prior generation.
- JSON Schema validation fires at deploy time, not at runtime.
- Podman `secret_mode = env` emits persistent degraded-isolation warning.

---

### [ ] Slice 5 (M3B): Blob Object Service

**Requirement IDs:** `[PLT-DAT]` (object service sub-requirement)
**ADR references:** [ADR-0009](../../../decisions/0009-blob-storage-object-store.md)
**Depends on:** M3A exit criteria fully met.

#### Tasks

**WIT Interface:**

- [ ] Create `crates/bindings/wit/blob-store.wit`:
  - `interface blob-store` with `put-blob(data: list<u8>) -> result<string,
    blob-error>` (returns SHA-256 hex), `get-blob(hash: string) ->
    result<list<u8>, blob-error>`, `delete-blob(hash: string) ->
    result<_, blob-error>`, `signed-url(hash: string, ttl-secs: u32) ->
    result<string, blob-error>` (if in M3B scope per D-03-04).
  - `variant blob-error { not-found, quota-exceeded, internal(string) }`.

**Blob Service Crate:**

- [ ] Define generic `BlobProvider` trait in `crates/blob-store/src/traits.rs` to ensure the blob backend is pluggable.
- [ ] Implement `ObjectStoreBlobProvider` in `crates/blob-store/src/object_store_impl.rs` backed by `Arc<dyn ObjectStore>` from the `object_store` crate:
  - Backend switchable via config: `LocalFileSystem` for dev/tests;
    `AmazonS3` (or compatible S3 endpoint) for production.
  - Tests use `object_store::memory::InMemory`.
  - Blobs stored at path `<service_id>/<aa>/<remaining-62-hex-chars>`
    (two-level directory prefix for filesystem balance).
  - SHA-256 verified on write and on read (detect silent corruption).
  - Path traversal guards: `service_id` matches `^[a-zA-Z0-9_\-]{1,128}$`;
    `hash` must be exactly 64 lowercase hex chars (`^[0-9a-f]{64}$`).
  - Use `Path::join` + `starts_with` for path descendant verification;
    do **not** use `Path::canonicalize`.

**HTTP Serving** (if in scope per D-03-04):

- [ ] HMAC-SHA256 presigned URLs with configurable TTL. Substrate serves at
  `GET /blobs/<hash>?sig=<hmac>&exp=<unix-ts>`.

**Tests:**

- [ ] Unit: `put-blob` + `get-blob` round-trip verifies SHA-256.
- [ ] Unit: corrupted blob detected on read.
- [ ] Unit: path traversal via crafted hash or service ID rejected.
- [ ] Unit: blob quota exceeded returns structured error.
- [ ] Integration: two services cannot read each other's blobs (namespace isolation).
- [ ] Integration (if signed URL in scope): valid URL accepted; expired URL rejected.

#### Acceptance Criteria

- `put-blob`/`get-blob` round-trip verifies content integrity by SHA-256.
- Service blob namespaces are isolated.
- No path traversal possible via crafted input.

---

### [ ] Slice 6 (M3B): Embedded MQTT Broker

**Requirement IDs:** `[PLT-DAT]` (MQTT event service sub-requirement)
**ADR references:** [ADR-0010](../../../decisions/0010-mqtt-broker-rumqttd.md)
**Depends on:** Slice 5 complete.

#### Tasks

**WIT Interface:**

- [ ] Create `crates/bindings/wit/pubsub.wit`:
  - `interface pubsub` with `publish(topic: string, payload: list<u8>) ->
    result<_, pubsub-error>`, `subscribe(topic: string) -> result<_, pubsub-error>`,
    `unsubscribe(topic: string) -> result<_, pubsub-error>`.
  - `variant pubsub-error { permission-denied, internal(string) }`.
  - Delivery is push-model: host invokes the component's exported
    `on-message(topic: string, payload: list<u8>)` if declared.

**Broker Embedding:**

- [ ] Add `rumqttd` (latest stable; pin version with rationale comment per workspace
  convention) and `tokio-util` (if not already present) to `Cargo.toml`.
- [ ] Create `crates/mqtt-broker/` crate with `MqttBroker`:
  - Starts `rumqttd` as a Tokio background task bridged by a **bounded channel**
    (default capacity: 1024 messages, configurable via `[mqtt].channel_capacity`).
  - When the channel is full, `publish` returns
    `pubsub-error::internal("broker channel full: backpressure")` — never blocks
    the Wasmtime execution thread.
  - Exposes `publish(topic, payload)` and `subscribe(topic, sender)` async APIs.
  - Uses `CancellationToken` to cleanly terminate on `Drop` (applying the M2
    epoch timer audit lesson).
  - Topics are namespaced by the host: guest topic `t` published by service `s`
    becomes `svc/s/t` in `rumqttd`. Cross-service subscriptions require explicit
    `svc/<other_service_id>/...` topic strings.

**Host Function Wiring:**

- [ ] Wire `syneroym:pubsub/publish` in `engine.rs` → `MqttBroker::publish`.
- [ ] Wire delivery: broker message → host invokes component `on-message` export
  via Wasmtime invocation path (if the component declares that export).

**Wildcard and Retained Messages:**

- [ ] Verify `rumqttd` supports MQTT `+` and `#` wildcards; enable in config.
- [ ] Verify retained messages delivered to new subscribers after publish.

**Tests:**

- [ ] Unit: `publish` + `subscribe` on same topic delivers message.
- [ ] Unit: wildcard `sensors/+/temp` matches `sensors/room1/temp`.
- [ ] Unit: retained message delivered to subscriber joining after publish.
- [ ] Unit: `CancellationToken` terminates broker task on `Drop` (no leak).
- [ ] Integration: two WASM components in different services exchange a message.
- [ ] Integration: MQTT topic namespacing — service A cannot receive messages
  published in service B's namespace (without explicit opt-in).
- [ ] Integration: channel backpressure — when the bounded channel is saturated,
  `publish` returns the backpressure error without blocking or crashing the substrate.
- [ ] Unit: broker task terminates within 1 second of `CancellationToken` cancellation.

#### Acceptance Criteria

- `publish`/`subscribe` round-trip works within a single service.
- Wildcard topics and retained messages function correctly.
- Broker task terminates cleanly on substrate shutdown.
- Cross-service MQTT namespace isolation enforced.

---

## Reference Scenario: Encrypted Data Lifecycle (M3A)

1. Operator starts substrate with `encryption = true`. No KEK injected.
2. Operator deploys test WASM service `profile-store` with
   `custom_config = { db_name = "profiles" }`.
3. Deployment succeeds; configuration generation 1 stored.
4. Operator invokes `profile-store` — receives `StorageError::EncryptionKeyRequired`.
5. Operator injects KEK via `roymctl kek inject <service_id>` (reads from stdin).
6. Operator re-invokes `profile-store`. WASM `init()` runs: `create-collection`
   creates `profiles` table; `execute-ddl` adds an index.
7. Service puts 3 profile records; queries with `Eq` filter; receives correct results.
8. Service calls `config/get("db_name")` → returns `"profiles"`.
9. Operator sets secret: `roymctl secret set profile-store api_key` (stdin).
10. Service calls `vault/reveal("api_key")` → receives secret bytes; not logged.
11. Operator rotates KEK (`roymctl kek rotate`). All DEKs re-encrypted atomically.
    Service continues operating with existing DB.
12. `roymctl status` shows service healthy; encrypted DB confirmed in health output.

**M3B extension** (blob + MQTT):

13. Service stores a blob (`put-blob`); records SHA-256 hash in `profiles` collection.
14. Service publishes MQTT event `profiles/updated` with the record ID.
15. Second test service subscribed to `profiles/+` receives the event and reads
    the blob by hash.

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
| SQL injection via filter: `Eq("name", "'; DROP TABLE profiles; --")` | Bound as parameterised value; no injection; query returns 0 results |
| WASM guest sets `creator_id` in payload | Host overwrites field; guest value ignored |
| Blob exceeds service quota | `blob-error::quota-exceeded`; no substrate crash |
| Read blob with bit-flipped bytes (SHA-256 mismatch) | `blob-error::internal("integrity check failed")` |
| Service A publishes to service B's MQTT namespace | Delivery blocked by namespace isolation |
| KEK rotation while service is handling a request | Re-encryption completes; in-flight request uses cached DEK |

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

---

## Tests Summary

### Unit Tests (adjacent to implementation crates)

- `crates/key-store/src/tests.rs` — DEK generate/encrypt/decrypt/rotate/zeroize.
- `crates/data-layer/src/tests.rs` — CRUD, filter compilation, batch, pagination,
  DDL gating, `creator_id` injection, SQL injection resistance.
- `crates/blob-store/src/tests.rs` — SHA-256 integrity, path guards, namespace isolation.
- `crates/mqtt-broker/src/tests.rs` — publish/subscribe, wildcards, retained, cancellation.

### Integration Tests (`tests/integration/`)

- `encrypted_db.rs` — full lifecycle: deploy → inject KEK → init → CRUD → rotate KEK.
- `vault.rs` — set secret → reveal → key not found.
- `config.rs` — deploy with config → `config/get` → redeploy bumps generation.
- `blob_roundtrip.rs` — put blob → get blob → SHA-256 verified.
- `mqtt_exchange.rs` — two services publish/subscribe across MQTT.

### End-to-End Tests (extending `mise run test:e2e`)

- M3A reference scenario (steps 1-12) in a live substrate instance.
- M3B reference scenario (steps 13-15) in a live substrate instance.
- All failure/security tests produce documented outcomes.

---

## Measurable Exit Criteria

### M3A Exit Criteria

All of the following must be verified and recorded in `status.md`:

- [ ] `cargo +nightly fmt --all` passes with zero diff.
- [ ] `cargo clippy --workspace --all-targets --all-features` passes with zero
  warnings and zero errors.
- [ ] `cargo test --workspace` passes with all tests green.
- [ ] `mise run test:e2e` passes (existing e2e scenarios must not regress).
- [ ] `cargo build --target wasm32-wasip2 -p syneroym-bindings` exits 0.
- [ ] `syneroym:data-layer@0.1.0`, `syneroym:vault@0.1.0`, and
  `syneroym:config@0.1.0` WIT packages compile and generate valid Rust bindings.
- [ ] M3A reference scenario (steps 1-12) executes end-to-end without error.
- [ ] All M3A failure/security tests produce documented outcomes.
- [ ] Performance budgets for M3A metrics (rows 1-10 in table) verified;
  `criterion` output captured in `status.md`.
- [ ] DEK never appears in plaintext on disk; verified by hex dump of `substrate.db`.
- [ ] Decisions D-03-01, D-03-02, D-03-03 resolved as ADRs in `docs/decisions/`.
- [ ] Traceability matrix updated with M3A evidence for `[PLT-DAT]` (structured
  data), `[FND-SEC]` (storage encryption), and `[FND-CFG]`.

### M3B Exit Criteria (additional, after M3A is closed)

- [ ] `syneroym:pubsub@0.1.0` and `syneroym:blob-store@0.1.0` WIT packages
  compile and generate valid Rust bindings.
- [ ] M3B reference scenario (steps 13-15) executes without error.
- [ ] All M3B failure/security tests produce documented outcomes.
- [ ] Performance budgets for M3B metrics verified; output captured in `status.md`.
- [ ] Decisions D-03-04 and D-03-05 resolved as ADRs in `docs/decisions/`.
- [ ] Traceability matrix updated with M3B evidence for `[PLT-DAT]` (blob and
  MQTT sub-requirements).

---

## Open Questions (Non-Blocking; Track for M4 Planning)

1. **`cr-sqlite` CRDT extensions:** Architecture diagram names `cr-sqlite` as
   the storage backend. For M3, plain `rusqlite` (via `rusqlite-cipher`) is
   correct — CRDT support is only needed for M7+ multi-node. Confirm at M3
   closeout that `StorageManager`'s `ServiceDb` interface does not prevent a
   future swap to `cr-sqlite`.

2. **`AggregationPipeline` scope:** Requirements specify `$group`, `$having`,
   projections. Explicitly deferred to M4. Must appear as a gate item in the
   M4 milestone plan.

3. **FDAE `execute-ddl` elevation model:** `is_init_context` flag in `HostState`
   is a temporary M3 scaffold. In M4 this becomes a proper Admin UCAN capability.
   Add a `// TODO(M4): replace is_init_context with Admin UCAN check` comment
   at every usage site.

4. **Podman sandbox integration:** `crates/podman_sandbox` exists but its
   integration with data-layer configuration delivery in M3 is unclear. Verify
   at Slice 4 whether the Podman path calls `StorageManager` directly or only
   receives resolved config values from the orchestrator.

5. **Per-SynApp-Instance KEK:** Deferred to M4. The `KeyStore` API must have
   an extensibility point (e.g., optional `app_instance_id` scope parameter)
   from Day 1 so M4 can narrow the scope without a breaking change. Validate
   this at M3A closeout.
