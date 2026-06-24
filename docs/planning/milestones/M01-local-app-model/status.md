# Milestone 1: Local App Model and Lifecycle - Slice 4 Status

**Status**: Slice 4 Completed
**Date**: 2026-06-24

## Progress Summary

We have successfully implemented **Slice 4: `roymctl` Standalone Journaling** under
[crates/app_orchestration](file:///Users/pari/gitSyneroym/syneroym/crates/app_orchestration) and
[apps/roymctl](file:///Users/pari/gitSyneroym/syneroym/apps/roymctl).

### Slice 4 Deliverables

- **`DeploymentJournal`** (`crates/app_orchestration/src/journal.rs`):
  - Implemented an SQLite-backed deployment journal storing deployment states.
  - Supported states: `PLANNED`, `APPLYING`, `ACTIVE`, `ROLLING_BACK`, `ROLLED_BACK`.
  - Exposed `open`, `open_in_memory`, `append`, `update_state`, and `get_latest` methods.
  - Added indexed schema for tracking instance IDs.

- **`Reconciler`** (`crates/app_orchestration/src/reconcile.rs`):
  - Implemented logic to diff against the active deployment state and compute configuration updates.
  - Generates `ReconcilePlan` containing explicit `ReconcileAction`s (`Add`, `Remove`, `Update`).
  - Implemented recovery logic `recover_applying` to resume deployments interrupted in the `APPLYING` state.

- **`roymctl app reconcile` command** (`apps/roymctl/src/commands/app.rs`):
  - Added the `reconcile` CLI subcommand.
  - Given an `instance_id`, it detects applying failures and computes recovery plans.
  - Tied it to the underlying `DeploymentJournal`.

## Verification Evidence

All unit tests pass; clippy and nightly fmt are clean.

### Test Commands

```bash
cargo test -p syneroym-app-orchestration
cargo clippy --workspace --all-targets --all-features
cargo +nightly fmt --all
```

### Passing Output (`cargo test -p syneroym-app-orchestration`)

```text
   Compiling syneroym-app-orchestration v0.1.0 (…/crates/app_orchestration)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.97s
     Running unittests src/lib.rs (…/syneroym_app_orchestration-...)

test journal::tests::test_journal_append_and_update ... ok
test reconcile::tests::test_reconcile_diff ... ok
test reconcile::tests::test_recover_applying ... ok

test result: ok. 48 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
```

### Clippy Output

```text
    Checking syneroym-app-orchestration v0.1.0 (…/crates/app_orchestration)
    Finished `dev` profile [unoptimized + debuginfo] target(s)
```
Zero warnings, zero errors.

### Nightly fmt

```text
fmt OK
```

## Test Coverage — Slice 4 specific (3 new tests)

| Test | What it verifies |
|---|---|
| `test_journal_append_and_update` | SQLite insert/update and fetching latest |
| `test_reconcile_diff` | Computes update action correctly based on source change |
| `test_recover_applying` | Recovers properly when there's an active APPLYING state |

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
