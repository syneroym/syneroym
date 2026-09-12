# Code-quality round: progress

One row per unit of work. **Each session ticks its own row when its pull request
merges.** This file is how independent sessions know what is already done —
nothing else tracks it.

Baseline: [`baseline-2026-09-08/`](baseline-2026-09-08/), measured at `ef1be60`.
Tag `before-quality-round-2026-09-08` marks the starting point:

```bash
git diff before-quality-round-2026-09-08 main
```

## Global steps

| # | Step | Status | PR |
| --- | --- | --- | --- |
| G1 | Unused dependencies (`cargo shear --fix`) + machine-fixable lints | done | [#168](https://github.com/syneroym/syneroym/pull/168) |
| G2 | False-comment sweep: comments whose claims are no longer true | done | [#170](https://github.com/syneroym/syneroym/pull/170) |
| G3 | CI gate on planning refs **added in a diff** | done | [#169](https://github.com/syneroym/syneroym/pull/169) |
| G3b | Align the three ref-pattern sources; add the test-or-matrix-row family (gate union 1,564 → 1,713, +149 lines) | done | `chore/quality-align-ref-patterns` |
| G4 | e2e harness: move 25 files onto `tests/common` | done | [#176](https://github.com/syneroym/syneroym/pull/176) |
| G5 | Document rewrite (VISION, developer-guide, backlog, traceability, ADRs) | done | readability pass on `docs/quality-readability` worktree |
| G6 | `clippy.toml` threshold ratchet + `cargo dupes check` + full-tree ref gate | not started | |
| G7 | Closing measurement: re-run the pass, commit `baseline-<date>/`, compare | not started | |

G5 depends on nothing and can run at any time, in parallel with everything else.
G6 and G7 come last.

### G4 breakdown

`crates/substrate/tests/common` grew a `SubstrateNode` builder (multi-node,
restart, role knobs) plus `serial_guard`. The 25 files that carried their own
`struct Node` / `fn boot` move onto it, in these pull requests:

| PR | Files | Status |
| --- | --- | --- |
| 1 | harness (`common/node.rs`, `serial_guard`) + `cert_renewal_e2e`, `supervisor_alerts_e2e`, `topology_document_e2e` | done ([#173](https://github.com/syneroym/syneroym/pull/173)) |
| 2 | shared `common/fixtures.rs` + supervisor family: `supervisor_loop_e2e`, `supervisor_interface_e2e`, `scheduled_task_e2e`, `health_monitoring_e2e`, `tier1_endpoint_record_e2e`, `app_instance_identity_e2e`, `instance_identity_e2e` | done ([#174](https://github.com/syneroym/syneroym/pull/174)) |
| 3 | outbox / restart family: `durable_outbox_e2e`, `proxy_outbox_e2e`, `saga_e2e`, `reference_scenario_e2e`, `multi_substrate_placement_e2e`, `master_endpoint_record_e2e` | done ([#175](https://github.com/syneroym/syneroym/pull/175)) |
| 4 | conversation / roym family: `conversation_e2e`, `group_conversation_e2e`, `roym_conversation_e2e`, `roym_directory_e2e`, `roym_transaction_e2e`, `binding_push_e2e`, `federated_fdae_e2e` | done (`a608e74`..`0c51511` on `main`) |
| 5 | `gateway_hostname_e2e`, `substrate_ownership_e2e`; widen `no_ephemeral_port_literals.rs`; move the deferred-backlog port-ledger row to "Recently resolved"; tick G4 | done ([#176](https://github.com/syneroym/syneroym/pull/176)) |

PR 1 must merge before 2-5, which are then independent (disjoint file sets) and
merge in any order; PR 5 last, since it ticks G4.

## Per-crate work

For each crate, in this order: **comments → file split → long functions.**
Comments go first so the later structural diffs are not mixed with prose churn.

Counts are from the baseline. "Mechanical" is the subset where the citation is a
parenthetical that can be removed without rewriting the sentence.

| Crate | Comment blocks | mechanical | Files 500+ | Long fns | Comments | Split | Functions |
| --- | ---: | ---: | ---: | ---: | --- | --- | --- |
| `crates/app_supervisor` | 267 | 45 | 3 | 9 | done | done | done |
| `crates/control_plane` | 151 | 40 | 3 | 14 | done | done | |
| `crates/router` | 120 | 27 | 5 | 12 | done | done | |
| `crates/substrate` | 110 | 28 | 1 | 29 | done | done | |
| `crates/sdk` | 101 | 47 | 4 | 3 | done | done | |
| `crates/sandbox_wasm` | 90 | 16 | 3 | 6 | done | | |
| `crates/core` | 75 | 37 | 3 | 0 | done | done | |
| `apps/roymctl` | 69 | 37 | 4 | 9 | done | done | |
| `crates/app_orchestration` | 49 | 22 | 2 | 2 | done | done | |
| `crates/async_queue` | 34 | 12 | 2 | 0 | done | done | |
| `crates/data_db` | 34 | 9 | 4 | 0 | done | done | |
| `crates/fdae` | 27 | 5 | 2 | 1 | done | | |
| `crates/rpc` | 21 | 8 | 0 | 0 | done | | |
| `crates/ucan` | 14 | 0 | 0 | 0 | done | done | n/a |
| `crates/coordinator_iroh` | 7 | 2 | 1 | 4 | done | | |
| `crates/coordinator_webrtc` | 7 | 2 | 1 | 0 | done | | |
| `crates/data_keystore` | 7 | 2 | 0 | 0 | done | | |
| `crates/data_blob` | 5 | 0 | 1 | 0 | done | | |
| `crates/community_registry` | 5 | 0 | 0 | 0 | done | | |
| `crates/mqtt_broker` | 5 | 0 | 0 | 0 | done | | |
| `crates/conversation` | 4 | 0 | 4 | 6 | done | done | |
| `crates/roym_transaction` | 0 | 0 | 1 | 7 | n/a | | |
| `crates/roym_directory` | 0 | 0 | 1 | 4 | n/a | | |
| `crates/roym_web` | 0 | 0 | 0 | 3 | n/a | n/a | |
| `crates/auth` | 0 | 0 | 1 | 3 | n/a | | |
| `crates/roym_core` | 0 | 0 | 2 | 0 | n/a | | n/a |
| `crates/roym_catalog` | 0 | 0 | 1 | 1 | n/a | | |
| `crates/roym_profile` | 0 | 0 | 1 | 1 | n/a | | |
| `crates/roym_conversation` | 0 | 0 | 1 | 0 | n/a | | n/a |
| `crates/client_gateway` | 0 | 0 | 1 | 1 | n/a | | |
| `crates/app_host_native` | 1 | 0 | 1 | 1 | done | | |
| `crates/sandbox_podman` | 1 | 1 | 1 | 1 | done | | |
| `crates/identity` | 2 | 0 | 0 | 0 | done | n/a | n/a |
| `crates/chunk_transfer` | 1 | 0 | 0 | 0 | done | n/a | n/a |
| `crates/app_host` | 1 | 0 | 0 | 0 | done | n/a | n/a |
| `crates/smoke-tests` | 0 | 0 | 0 | 1 | n/a | n/a | |
| `tests/perf` | 1 | 1 | 0 | 4 | done | n/a | |
| `xtask/src` | 0 | 0 | 0 | 2 | n/a | n/a | |
| `test-components/*` | 13 | 1 | 1 | 1 | done | | |
| **total** | **1,222** | **342** | **55** | **125** | | | |

The four Roym service crates (`roym_transaction` 2,944 lines / 21 arms,
`roym_directory` 2,215 / 16, `roym_profile` 1,292 / 19 including the 993-line
`invoke`, `roym_catalog` 1,029 / 13) share one dispatch shape. Design the split
once and apply it four times, rather than four separate designs.

## Long-function ladder

Fix everything above the threshold, lower `too-many-lines-threshold` in
`clippy.toml`, repeat. The ladder is the proof of coverage, not the work plan —
the per-crate rows above are where the work happens.

| Threshold | Functions above it | Lines in them | Status |
| ---: | ---: | ---: | --- |
| 500 | 3 | 2,180 | not started |
| 400 | 6 | 3,530 | |
| 300 | 14 | 6,435 | |
| 250 | 21 | 8,392 | |
| 200 | 31 | 10,605 | |
| 150 | 58 | 15,301 | |
| 100 | 125 | 23,330 | |

## Rules for every session

1. One crate (or one step) per branch. Small pull requests.
2. All gates green before merge: `cargo +nightly fmt --all`,
   `cargo clippy --workspace --all-targets --all-features`,
   `cargo nextest run --workspace`, `cargo test --workspace --doc`,
   `cargo audit`, `cargo deny check licenses`, and `mise run test:e2e` when
   tests changed.
3. Refactors must not change behaviour. If a test needs editing, stop and say
   so in the pull request — that is a signal something real moved.
4. Tick your row here in the same pull request as the work.
5. Merge straight to `main`. There is no integration branch: every step is
   independently safe, and merging incrementally is what keeps `main` tested.
