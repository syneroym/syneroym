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
| 5 | Split control_plane orchestration/tests.rs (8,596 lines) | not started | | |
| 6 | Split app_supervisor service/tests.rs (8,483 lines) | not started | | |
| 7 | Split roym_web tests/dual_build_parity.rs (7,146 lines) | not started | | |
| 8 | Split router src/proxy/tests.rs (2,927 lines) | not started | | |
| 9 | Split router tests/native_dispatch_identity.rs (2,877 lines) | not started | | |
| 10 | Split app_host_native tests/dual_build_parity.rs (2,441 lines) | not started | | |
| 11 | Enforcement: file-length limit for test files | not started | | |
| 12 | Enforcement: cap number of suppressions | not started | | |
| 13 | Wire in and document | not started | | |
| 14 | Final verification and pull request | not started | | |

## Running numbers
- Targeted allows added so far: 7 (4 in orchestration/tests.rs, 2 in service/tests.rs, 1 in router/proxy/tests.rs)
- Files still holding a file-level allow: 0 (all removed)
- Test count at last check: 2553 (pending re-verification after suppression changes)

## Decisions made
- GEMINI.md is a symlink to AGENTS.md; Unit 13 updates GEMINI.md/AGENTS.md

## Test renames so far
(none yet)

## Problems and open questions
(none)

## If you are picking this up
1. git fetch && git checkout chore/quality-test-file-limits
2. Read the units table above; continue at the first not-done unit.
3. Do not regenerate the baseline files.
