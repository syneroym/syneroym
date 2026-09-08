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
| G4 | e2e harness: move 25 files onto `tests/common` | not started | |
| G5 | Document rewrite (VISION, developer-guide, backlog, traceability, ADRs) | not started | |
| G6 | `clippy.toml` threshold ratchet + `cargo dupes check` + full-tree ref gate | not started | |
| G7 | Closing measurement: re-run the pass, commit `baseline-<date>/`, compare | not started | |

G5 depends on nothing and can run at any time, in parallel with everything else.
G6 and G7 come last.

## Per-crate work

For each crate, in this order: **comments → file split → long functions.**
Comments go first so the later structural diffs are not mixed with prose churn.

Counts are from the baseline. "Mechanical" is the subset where the citation is a
parenthetical that can be removed without rewriting the sentence.

| Crate | Comment blocks | mechanical | Files 500+ | Long fns | Comments | Split | Functions |
| --- | ---: | ---: | ---: | ---: | --- | --- | --- |
| `crates/app_supervisor` | 267 | 45 | 3 | 9 | in progress | | |
| `crates/control_plane` | 151 | 40 | 3 | 14 | | | |
| `crates/router` | 120 | 27 | 5 | 12 | | | |
| `crates/substrate` | 110 | 28 | 1 | 29 | | | |
| `crates/sdk` | 101 | 47 | 4 | 3 | | | |
| `crates/sandbox_wasm` | 90 | 16 | 3 | 6 | | | |
| `crates/core` | 75 | 37 | 3 | 0 | | | |
| `apps/roymctl` | 69 | 37 | 4 | 9 | | | |
| `crates/app_orchestration` | 49 | 22 | 2 | 2 | | | |
| `crates/async_queue` | 34 | 12 | 2 | 0 | | | |
| `crates/data_db` | 34 | 9 | 4 | 0 | | | |
| `crates/fdae` | 27 | 5 | 2 | 1 | | | |
| `crates/rpc` | 21 | 8 | 0 | 0 | | | |
| `crates/ucan` | 14 | 0 | 0 | 0 | | | |
| `crates/coordinator_iroh` | 7 | 2 | 1 | 4 | | | |
| `crates/coordinator_webrtc` | 7 | 2 | 1 | 0 | | | |
| `crates/data_keystore` | 7 | 2 | 0 | 0 | | | |
| `crates/data_blob` | 5 | 0 | 1 | 0 | | | |
| `crates/community_registry` | 5 | 0 | 0 | 0 | | | |
| `crates/mqtt_broker` | 5 | 0 | 0 | 0 | | | |
| `crates/conversation` | 4 | 0 | 4 | 6 | | | |
| `crates/roym_transaction` | 0 | 0 | 1 | 7 | n/a | | |
| `crates/roym_directory` | 0 | 0 | 1 | 4 | n/a | | |
| `crates/roym_web` | 0 | 0 | 0 | 3 | n/a | n/a | |
| `crates/auth` | 0 | 0 | 1 | 3 | n/a | | |
| `crates/roym_core` | 0 | 0 | 2 | 0 | n/a | | n/a |
| `crates/roym_catalog` | 0 | 0 | 1 | 1 | n/a | | |
| `crates/roym_profile` | 0 | 0 | 1 | 1 | n/a | | |
| `crates/roym_conversation` | 0 | 0 | 1 | 0 | n/a | | n/a |
| `crates/client_gateway` | 0 | 0 | 1 | 1 | n/a | | |
| `crates/app_host_native` | 1 | 0 | 1 | 1 | | | |
| `crates/sandbox_podman` | 1 | 1 | 1 | 1 | | | |
| `crates/identity` | 2 | 0 | 0 | 0 | | n/a | n/a |
| `crates/chunk_transfer` | 1 | 0 | 0 | 0 | | n/a | n/a |
| `crates/app_host` | 1 | 0 | 0 | 0 | | n/a | n/a |
| `crates/smoke-tests` | 0 | 0 | 0 | 1 | n/a | n/a | |
| `tests/perf` | 1 | 1 | 0 | 4 | | n/a | |
| `xtask/src` | 0 | 0 | 0 | 2 | n/a | n/a | |
| `test-components/*` | 13 | 1 | 1 | 1 | | | |
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
