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
| 7 | Split roym_web tests/dual_build_parity.rs (7,146 lines) | done | (pending commit) | split into 7 test modules + helpers + fixtures, all <=1,600 lines, count 2553 |
| 8 | Split router src/proxy/tests.rs (2,927 lines) | not started | | |
| 9 | Split router tests/native_dispatch_identity.rs (2,877 lines) | not started | | |
| 10 | Split app_host_native tests/dual_build_parity.rs (2,441 lines) | not started | | |
| 11 | Enforcement: file-length limit for test files | not started | | |
| 12 | Enforcement: cap number of suppressions | not started | | |
| 13 | Wire in and document | not started | | |
| 14 | Final verification and pull request | not started | | |

## Running numbers
- Targeted allows added so far: 13 (4 in orchestration/tests/{rollback,assets}, 2 in app_supervisor/service/tests, 1 in router/proxy/tests.rs, 6 in roym_web/tests/dual_build_parity/{helpers,transaction,transaction_cards})
- Files still holding a file-level allow: 0 (all removed)
- Test count at last check: 2553 (matches baseline)

## Decisions made
- GEMINI.md is a symlink to AGENTS.md; Unit 13 updates GEMINI.md/AGENTS.md
- Unit 5: orchestration tests split into app_instance, assets, cert, deploy, fdae_policy, lifecycle, probes, proxy_queue, rollback, status, and helpers matching production module boundaries. All 177 tests preserved, none duplicated.
- Unit 6: app_supervisor service/tests.rs split into bindings, queue_worker, renewal, resident_loop, resolve, rotation, schedules, status, verbs, and helpers matching production module boundaries. All 216 tests preserved, none duplicated.
- Unit 7: roym_web tests/dual_build_parity.rs split into profile, catalog, conversation, wire_origin, directory, transaction, transaction_cards, along with shared helpers and fixtures. All 147 tests preserved, none duplicated.

## Test renames so far
- service::orchestration::tests::<name> -> service::orchestration::tests::{app_instance, assets, cert, deploy, fdae_policy, lifecycle, probes, proxy_queue, rollback, status}::<name> (177 tests moved into submodules, Unit 5)
- service::tests::<name> -> service::tests::{bindings, queue_worker, renewal, resident_loop, resolve, rotation, schedules, status, verbs}::<name> (216 tests moved into submodules, Unit 6)
- dual_build_parity::<name> -> dual_build_parity::{profile, catalog, conversation, wire_origin, directory, transaction, transaction_cards}::<name> (147 tests moved into submodules, Unit 7)

## Problems and open questions
(none)

## If you are picking this up
1. git fetch && git checkout chore/quality-test-file-limits
2. Read the units table above; continue at the first not-done unit.
3. Do not regenerate the baseline files.
