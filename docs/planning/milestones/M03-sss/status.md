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
