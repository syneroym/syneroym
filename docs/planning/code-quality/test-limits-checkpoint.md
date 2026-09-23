# Test-limits task: checkpoint

Branch: chore/quality-test-file-limits
Branched from main at: 743f74a0
Baseline test count: 2553  (docs/planning/code-quality/test-limits/)

## Units

| Unit | What | Status | Commit | Notes |
|---|---|---|---|---|
| 1 | Baseline | done | 9efbe6ef | count 2553, branched at 743f74a0 |
| 2 | Suppressions A-F | done | 8ef99594 | 6 targeted allows (4 orchestration, 2 service/tests) |
| 3 | Suppressions G-R | done | 8ef99594 | 1 targeted allow: router/proxy/tests.rs:outbox_node |
| 4 | Suppressions S-Z, apps, tests, xtask | done | 8ef99594 | 0 targeted allows needed |
| 5 | Split control_plane orchestration/tests.rs (8,596 lines) | done | 63a2eb1d | split into 10 modules + helpers under tests/, all <1,500 lines, count 2553 |
| 6 | Split app_supervisor service/tests.rs (8,483 lines) | done | 981474ad | split into 9 modules + helpers under tests/, all <1,300 lines, count 2553 |
| 7 | Split roym_web tests/dual_build_parity.rs (7,146 lines) | done | f66619ca | split into 7 test modules + helpers + fixtures, all <=1,600 lines, count 2553 |
| 8 | Split router src/proxy/tests.rs (2,927 lines) | done | 03f04882 | split into 6 modules + helpers under proxy/tests/, all <620 lines, count 2553 |
| 9 | Split router tests/native_dispatch_identity.rs (2,877 lines) | done | d65b2168 | split into 5 modules + helpers under tests/native_dispatch_identity/, all <650 lines, count 2553 |
| 10 | Split app_host_native tests/dual_build_parity.rs (2,441 lines) | done | e02d8f1e | split into 6 modules + helpers under tests/dual_build_parity/, all <900 lines, count 2553 |
| 11 | Enforcement: file-length limit for test files | done | f95a5ecb | MAX_TEST_LINES=1800 ratchet added to xtask check-file-lengths, tested failure & pass |
| 12 | Enforcement: cap number of suppressions | done | da1f2c84 | MAX_TOO_MANY_LINES_SUPPRESSIONS=82 ratchet added, tested failure & pass |
| 13 | Wire in and document | not started | | |
| 14 | Final verification and pull request | not started | | |

## Running numbers
- Targeted allows added so far: 19 (4 in orchestration/tests/{rollback,assets}, 2 in app_supervisor/service/tests, 1 in router/proxy/tests.rs:outbox_node, 6 in roym_web/tests/dual_build_parity/{helpers,transaction,transaction_cards}, 4 in router/tests/native_dispatch_identity/{fdae_enforcement,cross_service_fetch}, 2 in app_host_native/tests/dual_build_parity/{permitted_differences,signing})
- Files still holding a file-level allow: 0 (all removed)
- Test count at last check: 2553 (matches baseline)

## Decisions made
- GEMINI.md is a symlink to AGENTS.md; Unit 13 updates GEMINI.md/AGENTS.md
- Unit 5: orchestration tests split into app_instance, assets, cert, deploy, fdae_policy, lifecycle, probes, proxy_queue, rollback, status, and helpers matching production module boundaries. All 177 tests preserved, none duplicated.
- Unit 6: app_supervisor service/tests.rs split into bindings, queue_worker, renewal, resident_loop, resolve, rotation, schedules, status, verbs, and helpers matching production module boundaries. All 216 tests preserved, none duplicated.
- Unit 7: roym_web tests/dual_build_parity.rs split into profile, catalog, conversation, wire_origin, directory, transaction, transaction_cards, along with shared helpers and fixtures. All 147 tests preserved, none duplicated.
- Unit 8: router src/proxy/tests.rs split into outbox, dead_letter, local_dispatch, remote_dispatch, sagas, saga_walk, and helpers. All 87 tests preserved, none duplicated.
- Unit 9: router tests/native_dispatch_identity.rs split into access_control, queries, fdae_enforcement, resolve_relation, cross_service_fetch, and helpers. All 39 tests preserved, none duplicated.
- Unit 10: app_host_native tests/dual_build_parity.rs split into conversation, data_layer, host_services, http, permitted_differences, signing, and helpers. All 40 tests preserved, none duplicated.

## Test renames so far
- service::orchestration::tests::<name> -> service::orchestration::tests::{app_instance, assets, cert, deploy, fdae_policy, lifecycle, probes, proxy_queue, rollback, status}::<name> (177 tests moved into submodules, Unit 5)
- service::tests::<name> -> service::tests::{bindings, queue_worker, renewal, resident_loop, resolve, rotation, schedules, status, verbs}::<name> (216 tests moved into submodules, Unit 6)
- dual_build_parity::<name> -> dual_build_parity::{profile, catalog, conversation, wire_origin, directory, transaction, transaction_cards}::<name> (147 tests moved into submodules, Unit 7)
- proxy::tests::<name> -> proxy::tests::{outbox, dead_letter, local_dispatch, remote_dispatch, sagas, saga_walk}::<name> (87 tests moved into submodules, Unit 8)
- native_dispatch_identity::<name> -> native_dispatch_identity::{access_control, queries, fdae_enforcement, resolve_relation, cross_service_fetch}::<name> (39 tests moved into submodules, Unit 9)
- dual_build_parity::<name> -> dual_build_parity::{conversation, data_layer, host_services, http, signing}::<name> (36 tests moved into submodules; 4 permitted_differences tests retained their module path, Unit 10)

## Verification outputs
### Unit 11 (file-length limit for test files)
- Failure with 1900-line test file (`crates/substrate/tests/test_length_ratchet_failure.rs`):
```
Checking source file lengths...
ERROR: crates/substrate/tests/test_length_ratchet_failure.rs: 1900 lines (maximum allowed for test files is 1800)
Error: File length check failed with 1 violation(s)
```
- Pass after removing test file:
```
Checking source file lengths...
All 408 production files (<= 800 lines) and 177 test files (<= 1800 lines) adhere to length limits.
```

### Unit 12 (cap number of suppressions)
- Failure with extra suppression added (`crates/core/src/lib.rs`):
```
Checking clippy::too_many_lines suppressions (max 82)...
Found 83 clippy::too_many_lines suppression(s) across tracked files.
ERROR: too_many_lines suppressions (83) exceed maximum allowed (82):
  crates/core/src/lib.rs:1
  ...
Error: Lint suppressions check failed: 83 exceeds maximum of 82
```
- Pass after removing extra suppression:
```
Checking clippy::too_many_lines suppressions (max 82)...
Found 82 clippy::too_many_lines suppression(s) across tracked files.
All clippy::too_many_lines suppressions are within the limit (82 <= 82).
```

## Problems and open questions
(none)

## If you are picking this up
1. git fetch && git checkout chore/quality-test-file-limits
2. Read the units table above; continue at the first not-done unit.
3. Do not regenerate the baseline files.
