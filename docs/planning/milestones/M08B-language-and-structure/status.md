# M08B status

*Reader: developers who build this sub-milestone.*

The plan and the slice order are in [task.md](task.md). This file records what is done, with evidence, and which tool did each step of each slice.

**Step** is one of: Not started, Plan written, Plan reviewed, Implemented, Code reviewed, Fixes done, Merged. "—" means the step has not happened yet.

| Slice | Step | Plan by | Plan review by | Implemented by | Code review by | Evidence |
| --- | --- | --- | --- | --- | --- | --- |
| S0 | Merged | Claude Opus 5.5 | Claude Sonnet 5.5 | Claude Sonnet 5.5 | Claude Opus 5.5 | [PR #348](https://github.com/syneroym/syneroym/pull/348); full CI green on `b1757a66` ([run 42060861373](https://github.com/syneroym/syneroym/actions/runs/42060861373)), later commits docs only. Docs only, no automated tests; `spelling-pairs.tsv` check: 200 pairs per language, 0 errors, 17 conflicts for the owner. Reviews: plan 12 items, code 5 items, all closed ([review](slice-s0-review.md)) |
| B1a | Code reviewed | Claude Opus 5.5 | Claude Sonnet 5.5 | Claude Sonnet 5.5 | Claude Opus 5.5 | [Plan](slice-b1a-implementation-plan.md); owner answered Q1 to Q6 (all (a), 2026-11-06). Plan review round 1: 12 items (0 blocker, 3 major, 9 minor), all fixed in `f10a0b15`. Plan review round 2: all 12 confirmed fixed; 2 new items (0 blocker, 0 major, 2 minor), both fixed in `96e0120e`. Plan review round 3: both confirmed fixed; 1 new item (0 blocker, 0 major, 1 minor), fixed in `78069aca`. Plan review round 4: confirmed; 0 open ([review](slice-b1a-review.md)). Implemented: Hub unit tests 226 passed in 27 files; `roym-hub.spec.ts` 36 passed; `mise run demo` 6 passed (story 40 included); the two Rust wording tests pass and fail when a sentence changes; [PR #349](https://github.com/syneroym/syneroym/pull/349). Full `mise run verify` not run yet (the slice close step runs it); CI runs the same suites as separate jobs. Code review round 1 at `92c7d11d`: CI green; 7 items (0 blocker, 1 major, 6 minor), all fixed in `2befd1b8`, `97e04d14`, `0614ab55`, `485d19e7`, `96cd887b` (Hub unit tests 230 passed in 27 files; the `hub-ui` gate passed inside `cargo xtask verify`). Code review round 2 at `ebd1ece8`: all 7 confirmed fixed; 0 new; **0 open** ([review](slice-b1a-review.md)). CI on `ebd1ece8`: every finished job passes; Test 3/6 and 4/6 were still running |
| B1b | Not started | — | — | — | — | — |
| B2 | Not started | — | — | — | — | — |
| B3a | Not started | — | — | — | — | — |
| B3b | Not started | — | — | — | — | — |
| B4a | Not started | — | — | — | — | — |
| B4b | Not started | — | — | — | — | — |
| B5 | Not started | — | — | — | — | — |
| B6 | Not started | — | — | — | — | — |
