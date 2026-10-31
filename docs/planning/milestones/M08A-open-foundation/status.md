# M08A status

*Reader: developers who build this sub-milestone.*

The plan and the slice order are in [task.md](task.md). This file records what is done, with evidence, and which tool did each step of each slice.

**Step** is one of: Not started, Plan written, Plan reviewed, Implemented, Code reviewed, Fixes done, Merged. "not recorded" means the tool was not written down when the step was done.

| Slice | Step | Plan by | Plan review by | Implemented by | Code review by | Evidence |
| --- | --- | --- | --- | --- | --- | --- |
| S0 | Merged | not recorded | not recorded | Gemini 3.8 Flash | not recorded | [crates/sandbox_wasm/tests/epoch_wait_credit.rs](../../../crates/sandbox_wasm/tests/epoch_wait_credit.rs), 20/20 runs passed, PR [#329](https://github.com/syneroym/syneroym/pull/329) |
| A1 | Merged | Claude Sonnet 5.5 | not recorded | Claude Sonnet 5.5 | not recorded | PR [#331](https://github.com/syneroym/syneroym/pull/331). The parity harness has 24 modules and 222 scenarios, all passing. `crates/roym_core/src/record/tests/wire_words.rs` and `crates/roym_web/ui/src/cards/wording.test.ts` keep the old words off the wire and off the cards. No review file was kept for A1. |
| A2a | Merged | Claude Sonnet 5.5 | Claude Opus 5.5 | Gemini 3.8 Flash | Claude Opus 5.5 | PR [#334](https://github.com/syneroym/syneroym/pull/334) (supersedes [#333](https://github.com/syneroym/syneroym/pull/333)). mise run verify passed (all 17 gates green, 2026-10-30). 63 method shapes, parity harness 24 modules / 222 scenarios pass, method_contract 11 tests pass, roym-core 221 tests pass, nextest 4171 tests pass. |
| A2b | Merged | Claude Opus 5.5 | Claude Opus 5.5 | Gemini 3.8 Flash | Gemini (round 1), Claude Opus 5.5 (round 2) | PR [#335](https://github.com/syneroym/syneroym/pull/335). CI [run 41178819995](https://github.com/syneroym/syneroym/actions/runs/41178819995) on 9733fb70 passed every job (2026-10-31); later commits change docs only (`check-change-docs` and `check-file-lengths` pass). 94 method shapes (25 catalog + 69 transaction; 157 total), parity harness 24 scenario modules plus deal_fixtures / 222 scenarios pass, method_contract contract tests pass (17 catalog + 41 transaction rows), 10 negative validation cases pass, roym_* e2e tests pass. |
| A2c | Implemented | Claude Opus 5.5 | Claude Sonnet 5.5 | Gemini 3.8 Flash | — | 38 method shapes (35 directory + 3 whoami; 195 total in workspace), parity harness 25 modules (20 scenario modules) / 224 scenarios pass, method_contract 31 tests pass (37 directory + 1 web row; unknown fields accepted on wire-reachable methods, refused on local-only; wire caller attribution on directory.publish verified), core/directory/catalog 242 tests pass. |
| A3 | Not started | — | — | — | — | — |
| A4 | Not started | — | — | — | — | — |
| A5 | Not started | — | — | — | — | — |
| A6 | Not started | — | — | — | — | — |
| A7 | Not started | — | — | — | — | — |
