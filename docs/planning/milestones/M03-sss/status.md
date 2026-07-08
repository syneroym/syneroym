# Milestone 3: Secure Stateful Services - Status Log

## Slice 0: Extract SQLite from `crates/core` (Completed)

We have successfully completed Slice 0. All SQLite dependencies and storage implementations have been extracted from `crates/core` into a new, dedicated `syneroym-data-layer` crate.

### Factual Verification Evidence

#### Workspace Tests (`cargo test --workspace`)
```text
test result: ok. 54 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 14.96s
```

#### E2E Playwright Tests (`mise run test:e2e`)
```text
  4 passed (19.3s)
```

#### Clippy Verification (`cargo clippy --workspace --all-targets --all-features`)
```text
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 10.55s
```

#### Formatting Verification (`cargo +nightly fmt --all --check`)
```text
    Formatting is fully compliant.
```

## Slice 1: Data-Layer and Vault WIT Interface Design (Completed)

We have successfully designed and implemented the WIT interface for both the data layer (`syneroym:data-layer`) and the secret vault (`syneroym:vault`). Guest Rust bindings have been regenerated successfully using the workspace standard `wit-bindgen::generate!`. Additionally, a placeholder crate `syneroym-data-layer` has been added, which exports the generated WIT types.

### Factual Verification Evidence

#### WASM WIT Target Build (`cargo build --target wasm32-wasip2 -p syneroym-bindings`)
```text
   Compiling syneroym-bindings v0.1.0 (/Users/pari/gitSyneroym/syneroym/crates/bindings)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.68s
```

#### Workspace Tests (`cargo test --workspace`)
```text
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s (syneroym_bindings doc-tests)
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 14.83s (syneroym_substrate tests)
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.08s (podman_lifecycle tests)
```

#### Clippy Verification (`cargo clippy --workspace --all-targets --all-features`)
```text
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 15.97s
    Clippy is fully compliant with 0 warnings.
```

#### Formatting Verification (`cargo +nightly fmt --all --check`)
```text
    Formatting is fully compliant.
```

#### E2E Playwright Tests (`mise run test:e2e`)
```text
  4 passed (19.3s)
```

## Slice 2A: Encrypted SQLite Isolation and Secret Vault (Completed)

We have successfully implemented Slice 2A:
1. **Memory Protection & Key Store**: Integrated memory-locking (`mlock`) re-exported from the identity crate. Implemented `KeyStore` supporting KEK injection, AES-256-GCM envelope encryption/decryption of DEKs, KEK rotation, and memory locking.
2. **Encrypted SQLite Storage Provider**: Created `SqliteStorageProvider` implementing transparent SQLCipher page-level encryption (`PRAGMA key`). Consolidated metadata schemas into the host's `endpoints.db` and per-service database state files (`state.db`), path traversal checks, and a secure `_vault` table for encrypted secrets.
3. **Control Plane Integration**: Extended `control-plane.wit` with the `security` management interface and updated `ControlPlaneService` to support native KEK injection, rotation, and secret registration via RPC. Slice 2A treats this as the local substrate management channel; full remote UCAN/FDAE authorization is deferred to M4.
4. **WASM Sandbox Vault Function**: Integrated `syneroym:host/vault` interface and registered `reveal` guest host function inside Wasmtime sandbox state, allowing secure retrieval of database vault secrets.
5. **Roymctl CLI Subcommands**: Wired clap commands for `roymctl kek inject`, `roymctl kek rotate`, and `roymctl secret set` using `SyneroymClient` RPC dispatch. `roymctl secret set <service-id> <key>` reads secret bytes from stdin.
6. **Data-Layer Boundary**: CRUD host functions remain deferred to Slice 3A. The current Slice 2A host wiring must return explicit errors for those calls rather than successful no-ops.

### Factual Verification Evidence

#### Workspace Tests (`cargo test --workspace`)
```text
running 2 tests
test engine::tests::test_wasm_quotas ... ok
test engine::tests::test_list_interfaces ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.17s

running 2 tests
test keys::tests::test_lock_memory ... ok
test keys::tests::test_lock_memory_large ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

running 3 tests
test key_store::tests::test_key_store_kek_rotation ... ok
test key_store::tests::test_key_store_lock_memory ... ok
test key_store::tests::test_key_store_operations ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s

running 3 tests
test sqlite::tests::test_encryption_key_required ... ok
test sqlite::tests::test_service_id_validation_and_path_traversal ... ok
test sqlite::tests::test_vault_write_and_reveal ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
```

#### E2E Playwright Tests (`mise run test:e2e`)
```text
  4 passed (18.7s)
```

#### Clippy and Formatting Verification
```text
Formatting and clippy checks pass cleanly across the workspace.
```

## Slice 3A: Data-Layer Host Functions (Completed)

We have successfully implemented Slice 3A:

1. **MongoDB-style filter compiler** (`crates/data-layer/src/filter.rs`): a pure,
   DB-free recursive-descent compiler from JSON filter documents to
   parameterized SQL `WHERE` clauses. Supports field equality, `$gt`/`$gte`/
   `$lt`/`$lte`/`$ne`, `$in`/`$nin`, `$regex` (compiled to `LIKE '%pattern%'`),
   `$and`/`$or`/`$not`, and dot-notation nested-field access. Both the JSON
   field *path* and the comparison *value* are bound as `?` parameters passed
   to `json_extract(payload, ?)` — no guest-supplied string is ever
   interpolated into SQL text. A depth-10 nesting guard and an unsupported-
   operator guard both return `data-layer-error::schema-violation`.
2. **Real CRUD/DDL host functions** (`crates/data-layer/src/sqlite.rs`,
   `crates/app_sandbox/src/engine.rs`): `create-collection`/`drop-collection`/
   `execute-ddl` (EXPLAIN-checked before execution)/`put` (upsert,
   `created_at` immutable after first write)/`patch` (RFC 7396 JSON
   merge-patch, implemented in Rust rather than depending on SQLite's
   `json_patch()` SQL function, since JSON1 availability isn't guaranteed with
   the `bundled-sqlcipher` rusqlite feature)/`get` (via the previously-unused
   deadpool reader pool)/`delete` (idempotent)/`delete-many` (returns affected
   row count)/`query` (cursor-paginated, hard-capped at
   `MAX_QUERY_PAGE_SIZE = 1000`)/`batch-mutate` (single transaction,
   `MAX_BATCH_SIZE = 200`, rolls back entirely on first failure). Mutating
   operations run through the existing single-writer actor; reads run through
   the reader pool for concurrency.
3. **Schema lifecycle hooks**: `AppSandboxEngine::deploy_wasm` now calls the
   new `StorageProvider::service_exists` check to decide whether a deploy is
   fresh (invokes the guest's `init()` export) or a re-deploy (invokes
   `migrate()`, with a `// TODO(M5)` noting the snapshot/rollback safety net
   is deferred). `get_wasm_func` was generalized to resolve root-level world
   exports (`interface_name: Option<&str>`), since `init`/`migrate` live
   directly on the `data-layer-guest` world, not inside a named interface.
   Lifecycle hook invocation skips gracefully (no error) for components that
   don't export `init`/`migrate` at all.
4. **Type-boundary conversion** (`crates/app_sandbox/src/data_layer_convert.rs`):
   `crates/bindings` generates two separate, structurally identical but
   distinct Rust type sets for `syneroym:data-layer/store` — one via
   `wasmtime::component::bindgen!` (used by `engine.rs`'s `Host` trait impl)
   and one via `wit_bindgen::generate!` (already used throughout
   `crates/data-layer` as `wit_store`, with zero `wasmtime` dependency). This
   module provides the mechanical field-by-field conversions between them.
5. **New test component** (`test-components/data-layer-test/`): imports
   `syneroym:data-layer/store@0.1.0`, exports `init`/`migrate` (root-level)
   and a `test-driver` interface (`run-crud-scenario`, `get-creator-id`) used
   by the new integration tests.
6. **Bug fix surfaced by this slice**: `SqliteStorageProvider`'s
   `SERVICE_ID_REGEX` (`^[a-zA-Z0-9_\-]{1,128}$`, inherited unchanged from
   Slice 2A) rejected colons, but real service ids are DIDs
   (`did:key:...`). Nothing called into the storage layer at deploy time
   before this slice, so the bug was latent; the new `service_exists` check
   made it immediately fatal for every WASM deploy. Fixed by extending the
   regex to `^[a-zA-Z0-9_:\-]{1,128}$` — colons are not a path separator on
   any Rust-supported OS, so this does not weaken the path-traversal guard.

### Factual Verification Evidence

#### Workspace Tests (`cargo test --workspace`)
```text
0 failures across the entire workspace, including:
- syneroym-data-layer: 50 passed (filter compiler, CRUD, batch, DDL-gating,
  SQL-injection, host-injected-field tests)
- syneroym-app-sandbox: 3 unit + 1 lifecycle-hooks integration (2 tests) +
  1 data-layer integration (1 test) = 6 passed
- syneroym-substrate tests/basic_lifecycle.rs: 3 passed (including the
  full WASM + TCP end-to-end scenario, confirming no regression from the
  SERVICE_ID_REGEX fix)
```

#### App-Sandbox Integration Tests
```text
running 1 test
test test_deploy_init_crud_creator_id_and_migrate ... ok

running 2 tests
test test_execute_ddl_denied_outside_lifecycle_context ... ok
test test_deploy_skips_lifecycle_hook_gracefully_for_component_without_it ... ok
```

#### Performance Budgets (`cargo bench -p syneroym-app-sandbox --bench data_layer_bench`)
```text
data_layer_put                  time: [24.7 µs 25.6 µs 26.3 µs]   (budget < 5 ms)
data_layer_get                  time: [17.7 µs 17.8 µs 17.8 µs]   (budget < 2 ms)
data_layer_query_100_eq_filter  time: [52.9 µs 53.0 µs 53.1 µs]   (budget < 20 ms)
data_layer_batch_mutate_50      time: [469  µs 485  µs 504  µs]   (budget < 30 ms)
data_layer_wasm_init_hook       time: [18.0 ms 18.2 ms 18.3 ms]   (budget < 200 ms)
data_layer_wasm_migrate_hook    time: [16.1 ms 16.9 ms 18.3 ms]   (budget < 200 ms)
```
All six measured operations are well within their M3A performance budgets.

#### WASM Target Build (`cargo build --target wasm32-wasip2 -p syneroym-bindings`)
```text
Finished `dev` profile [unoptimized + debuginfo] target(s)
```

#### E2E Playwright Tests (`mise run test:e2e`)
```text
  4 passed (18.7s)
```

#### Clippy and Formatting Verification
```text
cargo +nightly fmt --all -- --check: clean, zero diff
cargo clippy --workspace --all-targets --all-features: zero warnings
```

---

### Slice 4 (Service Configuration Delivery) Verification

#### `syneroym-app-sandbox` Config Unit Tests
```text
running 4 tests
test engine::tests::test_config_get_and_get_section ... ok
test engine::tests::test_config_isolation_and_generation_pinning ... ok
test engine::tests::test_wasm_quotas ... ok
test engine::tests::test_list_interfaces ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.18s
```

#### Full Workspace Integrity
```text
cargo test --workspace: All passing
cargo +nightly fmt --all -- --check: clean, zero diff
cargo clippy --workspace --all-targets --all-features: zero warnings
```

Slice 4 complete! Verified all tests pass.

---

## Slice 5 (M3B): Blob Object Service (Completed)

**Implemented by:** Claude Sonnet 5 (Claude Code), via an approved plan
(`/Users/pari/.claude/plans/valiant-jingling-tarjan.md`) reached after
several rounds of user clarification. Recorded here per AGENTS.md's
traceability requirement.

### Scope: larger than the `task.md` checklist, by explicit user direction

Discussion with the user before implementation surfaced two things beyond
the literal Slice 5 checklist:

1. **Streaming.** The blob interface needed to be stream-aware, not
   whole-buffer `list<u8>` in/out. Implemented as a hand-rolled WIT
   `resource` pair (`blob-writer`/`blob-reader`), chosen over
   `wasi:io/streams` or native `stream<u8>` because it's the lowest-risk
   option on the pinned wasmtime 46.0.1/wit-bindgen 0.57 toolchain (proven
   by the fact that `wasmtime-wasi` itself uses the identical mechanism for
   its own stream resources) and has a clean analogue on the native
   dispatch path (an explicit session id standing in for the resource
   handle). One-shot `put-blob`/`get-blob` were kept as thin wrappers over
   the same streaming primitives for small-payload convenience.
2. **Native (non-WASM) dispatch.** The user wants blob-store — and,
   retroactively, data-layer/vault/app-config, which turned out to have
   *no* native-callable path at all despite `crates/rpc`'s `NativeService`
   trait and `crates/router`'s dispatch machinery already existing for
   exactly this purpose (used only by `ControlPlaneService` until now) —
   reachable by a plain iroh client without deploying a WASM component.
   This is genuinely new cross-cutting infrastructure spanning
   `crates/rpc`, `crates/router`, and `crates/control_plane`, confirmed
   explicitly with the user as a scope expansion before implementation
   began (not something to silently fold into "Slice 5" without a record).

Raw HTTP GET passthrough (serving a signed URL or a static page directly
over HTTP, not JSON-RPC) was **deferred** at the user's own direction once
they understood it as a separate, general router feature — see
`task.md`'s "Deferred: HTTP Passthrough" entry.

### What was built

**`crates/blob-store/` (new crate):**
- `crypto.rs` — segmented streaming AEAD (`aead::stream` `StreamBE32` over
  AES-256-GCM, 256 KiB segments, HKDF-SHA256 per-blob subkeys from the
  service DEK) as incremental `BlobEncryptor`/`BlobDecryptor` types (fed
  arbitrary-sized chunks, not required to align to segment boundaries), plus
  HMAC-SHA256 `sign_url`/`verify_signed_url` deriving their key from the
  same DEK via a distinct HKDF `info` string (no new signing-key table, per
  the user's direction to reuse existing key material).
- `traits.rs` — `BlobProvider` (session-oriented: `open_upload`/
  `open_download` return `UploadSession`/`DownloadSession` trait objects;
  `put_blob`/`get_blob` are default-provided one-shot wrappers).
- `object_store_impl.rs` — `ObjectStoreBlobProvider`, backed by
  `Arc<dyn ObjectStore>`. Upload sessions buffer in memory (bounded by
  `max_blob_bytes`); download sessions stream via `GetResult::into_stream()`
  so an `offset` deep into a large encrypted blob doesn't force full-object
  buffering. Per-service aggregate quota is lazily loaded via one `list()`
  call per service on first touch, then tracked incrementally in memory.
  Path traversal is prevented structurally: `service_id`/`hash` are
  regex-validated (rejecting any `.`/`/` character) before any path is
  constructed, which is a stronger guarantee than a runtime
  `Path::join`+`starts_with` check alone (no TOCTOU symlink-race surface) —
  the `starts_with` check is still applied for the `LocalFileSystem` backend
  as defense in depth.
- `native_types.rs` — hand-authored JSON request/response shapes for the
  streaming methods' native-dispatch equivalent (opaque session ids standing
  in for WIT resource handles, which aren't JSON-representable).
- `HostUploadSession`/`HostDownloadSession` — concrete newtypes the WIT
  `blob-writer`/`blob-reader` resources are mapped to via `with:` in
  `crates/bindings/src/host.rs`'s `bindgen!` call (the `with:` key syntax is
  `"pkg:name/interface.resource-name"`, a dot before the resource name, not
  a slash — found by reading `wasmtime-wasi`'s own `bindings.rs` source
  after the naive slash-separated guess failed to compile).

**`crates/bindings/wit/blob-store/blob-store.wit`:** resource-based
interface as described above; symlinked into `host/deps/` the same way
`app-config`/`vault`/`data-layer` already are.

**`crates/app_sandbox/src/engine.rs`:** `HostState`/`AppSandboxEngine` gain
a `blob_provider: Arc<dyn BlobProvider>` field (threaded through the same
constructor path as `key_store`/`storage_provider`); `impl Host`,
`impl HostBlobWriter`, `impl HostBlobReader` for `HostState`, each
delegating into `BlobProvider`; both resource-trait impls include a
`drop()` handler that discards any in-flight session the guest never
explicitly `finish()`/`abort()`ed (implicit-abort safety net). Registered
in `build_wasm_linker`.

**`crates/data-layer/src/traits.rs`:** new
`StorageProvider::load_service_dek` method (`Ok(None)` when encryption is
disabled — a deliberate mode, not an error), factored out of the DEK
generate-or-load block already inlined in
`SqliteStorageProvider::open_service_db`, so blob-store and the native
data-layer dispatch can resolve a DEK without depending on `rusqlite`.

**`crates/core/src/config.rs`:** `StorageConfig.blob_store: BlobStoreConfig`
(`backend: Local|S3`, `local_root` defaulting to
`<app_local_data_dir>/blob_objects`, `s3: Option<S3BlobConfig>`,
`max_blob_bytes` default 100 MiB, `max_service_total_bytes: Option<u64>`).
Deliberately a distinctly-named field, not reusing the pre-existing
`storage.blobs_dir` — that field is the compiled-WASM-binary cache
(`crates/app_sandbox/src/engine.rs`), unrelated to this slice, and the name
collision would have been confusing.

**Native dispatch (`crates/rpc`, `crates/router`, `crates/control_plane`):**
- `syneroym_rpc::NativeDispatchRegistry` — `Arc<DashMap<String, Arc<dyn
  NativeService>>>` type alias (deliberately not a wrapper struct: `DashMap`'s
  own API already covers every call site).
- `RouteHandler::init` (`crates/router/src/route_handler.rs`) constructs the
  registry once and threads it — alongside a newly-constructed
  `blob_provider` — into both `AppSandboxEngine::init` and
  `ControlPlaneService::init`, mirroring exactly how `key_store`/
  `storage_provider` are already shared. `build_blob_provider` selects the
  `Local`/`S3` backend from config; `S3` requires the (off-by-default) `aws`
  cargo feature and fails fast with an actionable error otherwise.
- `SynSvcNativeService` (`crates/control_plane/src/synsvc_native.rs`) — one
  instance per deployed `service_id`, dispatching `data-layer`/`vault`/
  `app-config`/`blob-store` JSON-RPC calls onto the same
  `StorageProvider`/`ServiceStore`/`BlobProvider` traits the WASM `Host`
  impls already call (a second adapter, not a reimplementation). Does not
  depend on `syneroym-app-sandbox`, which is an optional feature of
  `control_plane` — native capability access must work without it.
- `ControlPlaneService::deploy`/`undeploy` register/deregister the 4
  native-capability interfaces and the dispatch entry for every deployed
  service, regardless of `service-type` (wasm/container/tcp), since these
  are host-provided capabilities orthogonal to execution model.

### Bug found and fixed during this slice

`ControlPlaneService::list()` derived a deployed service's `endpoint_type`
from whichever registered interface its internal loop encountered *first*.
That was safe when each service had exactly one registered interface; once
every deployed service also gained 4 native-capability interfaces, iteration
order became significant and `test_substrate_lifecycle_scenarios`
(`crates/substrate/tests/basic_lifecycle.rs`) caught a WASM-deployed
service's `endpoint_type` intermittently reporting `"native"` instead of
`"wasm"`. Fixed by excluding the 4 native-capability interface names from
`list()`'s enumeration entirely — they're host-provided plumbing, not part
of a service's own declared interface surface, and every deployed service
always has its real wasm/container/tcp endpoint registered separately.

### Deliberate deviations from ADR-0009 / the original task.md text

- **No live HTTP endpoint for `signed-url()`.** The function itself is
  implemented and tested (HMAC-SHA256, HKDF-derived key); no `GET
  /blobs/<hash>` route exists to resolve it. User-directed deferral — see
  `task.md`.
- **`object_store` pinned to 0.13.x, not the latest 0.14.0.** 0.14's `md-5`
  dependency requires stable `digest ^0.11.0`, which conflicts with the
  `digest 0.11.0-rc.10` pin already required by `iroh-base`'s pre-release
  `ed25519-dalek`/`pkcs8` chain (documented in the root `Cargo.toml` next to
  the pre-existing `sha2`/iroh pins). 0.13.x is the newest line that still
  resolves; revisit alongside those pins once iroh ships a stable release.
- **S3 backend gated behind a non-default `aws` cargo feature** on
  `syneroym-blob-store` (and forwarded through `syneroym-router`'s own
  `aws` feature), for the same digest-conflict reason — `object_store`'s
  `aws` feature pulls in the same `md-5`/`digest ^0.11.0` requirement.
  `BlobBackend::S3` still exists unconditionally in config; selecting it
  without the feature compiled in fails fast with an actionable error
  rather than silently falling back to `Local`.
- **Download-side `offset` is not a true segment-indexed seek for encrypted
  blobs.** It decrypts sequentially from segment 0 and discards plaintext
  before `offset` server-side — correct, but not optimal for a very late
  offset into a very large encrypted blob. The STREAM construction's
  counter-based nonce would allow true random-access decryption; left as a
  documented future optimization, not needed for correctness.

### Factual Verification Evidence

#### `syneroym-blob-store` (new crate, 29 tests)
```text
running 29 tests
test crypto::tests::sign_and_verify_url_round_trip ... ok
test crypto::tests::expired_url_is_rejected ... ok
test crypto::tests::round_trip_small_single_chunk ... ok
test crypto::tests::corrupted_byte_is_rejected ... ok
test crypto::tests::round_trip_empty_blob ... ok
test crypto::tests::tampered_signature_is_rejected ... ok
test object_store_impl::tests::abort_discards_partial_upload ... ok
test object_store_impl::tests::aggregate_quota_exceeded ... ok
test object_store_impl::tests::aggregate_quota_is_per_service ... ok
test object_store_impl::tests::delete_missing_blob_is_idempotent ... ok
test object_store_impl::tests::get_missing_blob_returns_not_found ... ok
test object_store_impl::tests::hash_path_traversal_rejected ... ok
test object_store_impl::tests::namespace_isolation_across_services ... ok
test object_store_impl::tests::delete_then_get_returns_not_found ... ok
test object_store_impl::tests::open_download_with_offset_returns_suffix_plaintext ... ok
test object_store_impl::tests::encrypted_bytes_at_rest_do_not_contain_plaintext ... ok
test object_store_impl::tests::service_id_path_traversal_rejected ... ok
test object_store_impl::tests::put_get_round_trip_unencrypted ... ok
test object_store_impl::tests::signed_url_for_existing_blob_succeeds ... ok
test object_store_impl::tests::signed_url_for_missing_blob_is_not_found ... ok
test object_store_impl::tests::single_blob_quota_exceeded_fails_fast ... ok
test object_store_impl::tests::single_blob_quota_exceeded_mid_upload ... ok
test object_store_impl::tests::wrong_dek_fails_integrity_check ... ok
test object_store_impl::tests::put_get_round_trip_encrypted ... ok
test crypto::tests::round_trip_exact_segment_boundary ... ok
test object_store_impl::tests::open_download_with_offset_returns_suffix_encrypted ... ok
test crypto::tests::reordered_segments_are_rejected ... ok
test crypto::tests::truncated_ciphertext_is_rejected ... ok
test crypto::tests::round_trip_multi_segment_arbitrary_chunking ... ok

test result: ok. 29 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

#### `syneroym-app-sandbox` (WASM host wiring: 4 unit + 5 new blob integration + 1 data-layer + 2 lifecycle-hooks)
```text
running 4 tests (unit)
test engine::tests::test_config_get_and_get_section ... ok
test engine::tests::test_config_isolation_and_generation_pinning ... ok
test engine::tests::test_wasm_quotas ... ok
test engine::tests::test_list_interfaces ... ok
test result: ok. 4 passed; 0 failed

running 5 tests (tests/blob_store_integration.rs -- new this slice)
test test_abort_discards_upload ... ok
test test_streaming_upload_and_download_via_resources ... ok
test test_cross_service_blob_isolation ... ok
test test_open_download_with_offset ... ok
test test_one_shot_put_get_delete_round_trip ... ok
test result: ok. 5 passed; 0 failed

running 1 test (tests/data_layer_integration.rs -- pre-existing, confirms no regression)
test test_deploy_init_crud_creator_id_and_migrate ... ok

running 2 tests (tests/lifecycle_hooks.rs -- pre-existing, confirms no regression)
test test_execute_ddl_denied_outside_lifecycle_context ... ok
test test_deploy_skips_lifecycle_hook_gracefully_for_component_without_it ... ok
```

#### `syneroym-data-layer` (54 tests: 50 pre-existing + 4 new `load_service_dek`)
```text
test result: ok. 54 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.06s
```
New: `test_load_service_dek_none_when_encryption_disabled`,
`test_load_service_dek_requires_kek_when_encryption_enabled`,
`test_load_service_dek_generates_then_reuses_same_dek`,
`test_load_service_dek_matches_open_service_db_dek`.

#### `syneroym-control-plane` (8 tests: 7 pre-existing + 1 new native-dispatch round trip)
```text
running 8 tests
test config_utils::tests::test_flatten_json_config ... ok
test service::tests::test_deploy_plan_path_traversal ... ok
test service::tests::test_deploy_plan_absolute_path ... ok
test service::tests::test_wit_adherence ... ok
test service::tests::test_security_dispatch_returns_sdk_statuses ... ok
test service::tests::test_native_dispatch_data_layer_and_blob_store_round_trip ... ok
test service::tests::test_deploy_config_schema_rejection ... ok
test service::tests::test_deploy_config_generation_rollback ... ok

test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.90-5.56s
```
`test_native_dispatch_data_layer_and_blob_store_round_trip` deploys a
TCP-type service (no WASM component involved at all), then drives
`data-layer` (`create-collection`/`put`/`get`), `blob-store` one-shot
(`put-blob`/`get-blob`), and the full `blob-store` streaming session
(`open-upload`/`write-chunk` ×2/`finish-upload`/`open-download`/
`read-chunk`) purely through `SynSvcNativeService::dispatch`, then confirms
`undeploy` removes the registration.

#### `syneroym-rpc` (9 tests, pre-existing, confirms `NativeDispatchRegistry` addition didn't regress anything)
```text
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

#### `syneroym-router` (15 tests, pre-existing, confirms `build_blob_provider`/native-dispatch wiring didn't regress anything)
```text
test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
```

#### `cargo build --target wasm32-wasip2 -p syneroym-bindings`
```text
Finished `dev` profile [unoptimized + debuginfo] target(s)
```
Confirms the new resource-based `blob-store` WIT doesn't break the
guest-only build.

#### `cargo test --workspace` (full run, sandbox disabled for iroh socket binding)
```text
0 failures across the entire workspace.
```

#### `cargo clippy --workspace --all-targets --all-features`
```text
Finished `dev` profile [unoptimized + debuginfo] target(s) in 38.64s
```
Zero warnings.

#### `cargo +nightly fmt --all -- --check`
```text
(no diff)
```

#### `mise run test:e2e`
```text
  4 passed (19.4s)
```
No regression from the M3A baseline.

### What was NOT done in this slice (tracked separately, not silently dropped)

- Live HTTP GET passthrough for signed URLs / static pages
  (`task.md`'s "Deferred: HTTP Passthrough").
- Performance budget measurements for `put-blob`/`get-blob` (the M3B
  performance budget table rows) — not yet benchmarked with `criterion`;
  functional correctness is fully covered above, but the row 1MB-local
  latency numbers from the M3B Performance Budgets table are still open.
- True resumable multipart uploads (persisted server-side offset across a
  reconnect) — `abort()` gives basic cleanup; see the "Deferred" note in
  `task.md`'s WIT interface section for why full resumability was judged
  disproportionate scope for this slice.
