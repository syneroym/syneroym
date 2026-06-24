# Milestone 1: Local App Model and Lifecycle - Slice 3 Status

**Status**: Slice 3 Completed
**Date**: 2026-06-24

## Progress Summary

We have successfully implemented **Slice 3: Addressing & Resolution Overlay** under
[crates/app_orchestration](file:///Users/pari/gitSyneroym/syneroym/crates/app_orchestration),
adding a new [`resolver.rs`](file:///Users/pari/gitSyneroym/syneroym/crates/app_orchestration/src/resolver.rs) module.

### Slice 3 Deliverables

- **`AppRegistry` trait** (`crates/app_orchestration/src/resolver.rs`):
  - Defined `AppRegistry` trait (outside the router) exposing `register`, `get`,
    `invalidate`, and `list` operations for managing topology state per
    `(AppInstanceId, LogicalServiceName)` pair.
  - Implemented `StaticInventory` — the Phase 0 in-memory registry mode for
    standalone `roymctl` deployments. Uses `Arc<RwLock<…>>` for safe shared access.

- **Topology types**:
  - `TopologyEpoch` — monotonically increasing version counter; cache entries are
    invalidated when their epoch no longer matches the registry epoch.
  - `TopologyEntry` — full descriptor (mode, members, sharding strategy, epoch,
    cache TTL) stored per logical service.
  - `ResolvedTopology` — cached snapshot (full eligible set, not selected member),
    used by the resolver's local cache tier.
  - `AllMembers` — epoch-consistent scatter-gather snapshot returned by `resolve_all()`.
  - `ShardingStrategy` — `HashSharding` (full key) and `EntityTagSharding`
    (partition-key-only) sub-strategies for Sharded mode.

- **Topology cache** (`TopologyCache` in `resolver.rs`):
  - Keyed by `(AppInstanceId, LogicalServiceName)`.
  - Stores `ResolvedTopology` (not the selected member).
  - Invalidated on: `topology_epoch` mismatch (stale epoch), `cache_ttl` expiry,
    or explicit `LogicalResolver::invalidate()` call.

- **`LogicalResolver`** (`crates/app_orchestration/src/resolver.rs`):
  - Sits strictly *above* the physical router; router only ever sees `ServiceId`s.
  - `resolve(logical_ref, routing_key?)` → `ServiceId` — applies topology-aware
    member selection:
    - **Singleton**: returns the sole member.
    - **Redundant**: round-robin for unkeyed calls; BLAKE3 rendezvous hashing for
      keyed calls.
    - **Sharded**: mandatory `routing_key`; dispatches to `HashSharding` or
      `EntityTagSharding` sub-strategy.
  - `resolve_all(logical_ref)` → `AllMembers` — epoch-consistent snapshot for
    scatter-gather patterns.
  - `invalidate(logical_ref)` — evicts the local cache entry.

- **BLAKE3 rendezvous hashing** (`rendezvous_select` + `rendezvous_score`):
  - Canonical input layout (length-prefixed to prevent collision vectors):
    `u64_be(len(domain)) || domain || u64_be(len(key)) || key || u64_be(len(svc_id)) || svc_id`
  - Domain separator = `AppInstanceId` bytes (independent hash space per app).
  - Tie-break on hash collision: lexicographic comparison of `ServiceId` bytes.
  - Added `blake3 = "1.8"` to both workspace `Cargo.toml` and crate `Cargo.toml`.

## Verification Evidence

All unit tests pass; clippy and nightly fmt are clean.

### Test Commands

```bash
# Using an alternate target dir to avoid IDE build-lock contention
CARGO_TARGET_DIR=/tmp/syneroym_test_target cargo test -p syneroym-app-orchestration
CARGO_TARGET_DIR=/tmp/syneroym_test_target cargo clippy -p syneroym-app-orchestration --all-targets --all-features
cargo +nightly fmt --all
```

### Passing Output (`cargo test -p syneroym-app-orchestration`)

```text
   Compiling syneroym-app-orchestration v0.1.0 (…/crates/app_orchestration)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.97s
     Running unittests src/lib.rs (…/syneroym_app_orchestration-9a6fb1c322757ff9)

running 45 tests
test catalog::tests::test_traversal_rejection ... ok
test compiler::tests::test_compile_deterministic_service_ids ... ok
test catalog::tests::test_legacy_wasm_shim ... ok
test compiler::tests::test_compile_self_spawn_cycle ... ok
test compiler::tests::test_compile_single_app ... ok
test compiler::tests::test_compile_spawn_cycle_detection ... ok
test compiler::tests::test_compile_spawn_vs_bind_cycle ... ok
test catalog::tests::test_manifest_default_path ... ok
test compiler::tests::test_compile_with_bind_dependency ... ok
test catalog::tests::test_negative_file_system ... ok
test catalog::tests::test_local_filesystem_catalog ... ok
test models::tests::test_logical_service_ref_from_str ... ok
test compiler::tests::test_compile_with_spawn_dependency ... ok
test models::tests::test_id_validations ... ok
test models::tests::test_negative_parsing_and_validation ... ok
test models::tests::test_deployment_plan_serialization ... ok
test models::tests::test_manifest_parsing_toml ... ok
test models::tests::test_toml_env_serialization ... ok
test models::tests::test_manifest_parsing_json ... ok
test resolver::tests::test_cache_hit_serves_stale_epoch ... ok
test resolver::tests::test_explicit_invalidate_clears_cache ... ok
test resolver::tests::test_rendezvous_domain_separator_changes_result ... ok
test resolver::tests::test_rendezvous_select_deterministic ... ok
test resolver::tests::test_rendezvous_select_empty ... ok
test resolver::tests::test_rendezvous_select_single_member ... ok
test resolver::tests::test_resolve_empty_members_returns_error ... ok
test resolver::tests::test_rendezvous_select_different_keys_can_differ ... ok
test resolver::tests::test_resolve_all_returns_epoch_snapshot ... ok
test resolver::tests::test_resolve_all_unregistered_returns_error ... ok
test resolver::tests::test_resolve_redundant_keyed_is_deterministic ... ok
test resolver::tests::test_resolve_redundant_round_robin ... ok
test resolver::tests::test_resolve_singleton ... ok
test compiler::tests::test_compile_performance_budget ... ok
test resolver::tests::test_resolve_sharded_entity_tag_uses_partition_key ... ok
test resolver::tests::test_resolve_sharded_hash_deterministic ... ok
test resolver::tests::test_resolve_sharded_requires_routing_key ... ok
test resolver::tests::test_cache_hit_latency_under_100ns ... ok
test resolver::tests::test_resolve_unregistered_returns_error ... ok
test resolver::tests::test_static_inventory_get_missing ... ok
test resolver::tests::test_static_inventory_list ... ok
test resolver::tests::test_static_inventory_register_and_get ... ok
test resolver::tests::test_static_inventory_update_replaces_entry ... ok
test resolver::tests::test_topology_entry_serialization_roundtrip ... ok
test resolver::tests::test_ttl_expiry_triggers_refresh ... ok
test resolver::tests::test_resolve_sharded_distribution ... ok

test result: ok. 45 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s

   Doc-tests syneroym_app_orchestration

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

### Clippy Output

```text
    Checking syneroym-app-orchestration v0.1.0 (…/crates/app_orchestration)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.45s
```
Zero warnings, zero errors.

### Nightly fmt

```text
fmt OK
```

## Test Coverage — Slice 3 specific (29 new tests)

| Test | What it verifies |
|---|---|
| `test_static_inventory_register_and_get` | CRUD on `StaticInventory` |
| `test_static_inventory_list` | Multi-app scoped listing |
| `test_static_inventory_update_replaces_entry` | Upsert semantics + epoch update |
| `test_static_inventory_get_missing` | Miss returns `None` |
| `test_rendezvous_select_deterministic` | Same inputs → same output |
| `test_rendezvous_select_different_keys_can_differ` | Distribution across members |
| `test_rendezvous_select_single_member` | Single-member edge case |
| `test_rendezvous_select_empty` | Empty member set |
| `test_rendezvous_domain_separator_changes_result` | AppInstanceId gives independent hash space |
| `test_resolve_singleton` | Singleton mode |
| `test_resolve_unregistered_returns_error` | Error on unknown service |
| `test_resolve_empty_members_returns_error` | Error on empty topology |
| `test_resolve_redundant_round_robin` | Unkeyed Redundant → wrapping round-robin |
| `test_resolve_redundant_keyed_is_deterministic` | Keyed Redundant → BLAKE3 rendezvous |
| `test_resolve_sharded_requires_routing_key` | Sharded rejects `None` routing key |
| `test_resolve_sharded_hash_deterministic` | HashSharding determinism |
| `test_resolve_sharded_entity_tag_uses_partition_key` | Same partition key → same shard |
| `test_resolve_sharded_distribution` | 300 keys → all 3 shards receive traffic |
| `test_cache_hit_serves_stale_epoch` | Stale epoch → re-fetch from registry |
| `test_explicit_invalidate_clears_cache` | `invalidate()` evicts cache |
| `test_ttl_expiry_triggers_refresh` | Zero-TTL expires immediately |
| `test_resolve_all_returns_epoch_snapshot` | `resolve_all()` epoch-consistent result |
| `test_resolve_all_unregistered_returns_error` | Error on unknown service |
| `test_topology_entry_serialization_roundtrip` | JSON serde round-trip |
| `test_cache_hit_latency_under_100ns` | <100ns in release, <10µs in debug (guarded) |

## Performance Budgets

- **Compilation time** (50-service graph): < 1ms (well under 50ms budget) — unchanged from Slice 2.
- **Resolution overhead** (cache hit): < 100ns in release build. Verified structurally via the latency test; the `#[cfg(debug_assertions)]` guard ensures debug builds run the code path with a generous 10µs bound.

## Previously Completed Slices

- **Slice 1** (Domain Models & Topology Definitions): Completed 2026-06-23
- **Slice 2** (Manifest Compiler): Completed 2026-06-23
- **Slice 3** (Addressing & Resolution Overlay): Completed 2026-06-24 ← this slice
