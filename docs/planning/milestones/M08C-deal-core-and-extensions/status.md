# M08C status

*Reader: developers who build this sub-milestone.*

The plan and the slice order are in [task.md](task.md). This file records what is done, with evidence, and which tool did each step of each slice.

**Step** is one of: Not started, Plan written, Plan reviewed, Implemented, Code reviewed, Fixes done, Merged. "—" means the step has not happened yet.

| Slice | Step | Plan by | Plan review by | Implemented by | Code review by | Evidence |
| --- | --- | --- | --- | --- | --- | --- |
| C0 | Implemented | no plan (docs only) | no plan (docs only) | Claude Opus 5.5 | — | [ADR-0030](../../../decisions/0030-signed-message-origins.md), accepted by the owner 2026-11-16 |
| C1a | Merged | Claude Opus 5.5 | Claude Opus 5.5 (rounds 1, 3, 4, 5), Claude Sonnet 5.5 (round 2) | Gemini 3.8 Flash | Claude Opus 5.5 (rounds 1 to 6) | [PR #367](https://github.com/syneroym/syneroym/pull/367), merged as 3dfada58 (squash, PR #367). Code review round 6 on 7062a18f: 0 open, [review file](slice-c1-review.md). `mise run verify` with no `--skip`: all 16 gates passed (2026-11-17, on f44b0e72 plus the close-out docs). Sizes within limits: `transaction` +198,920 bytes (+13.1%), the other five within ±105 bytes ([task.md](task.md), section 1). Story 50 failed once in CI on f44b0e72 (Bob's receipt card not shown in 60 s); the rerun passed, it passed 4 of 4 locally, and the branch changes no delivery or Hub feed code. Owner reviewed the new Hindi messages. |
| C1b | Merged | Claude Opus 5.5 | Claude Opus 5.5 (rounds 1, 3, 4, 5), Claude Sonnet 5.5 (round 2) | Gemini 3.8 Flash | Claude Opus 5.5 (rounds 7, 8, 9) | [PR #368](https://github.com/syneroym/syneroym/pull/368), merged as ba5a5597 (squash). Code review round 9: 0 open, [review file](slice-c1-review.md). `mise run verify` with no `--skip`: all 16 gates passed (2026-11-18, on 14b96999; hub-ui: 46 files, 561 passed; nextest: 4,721 passed, 6 skipped across 157 binaries; e2e: 87 passed). Sizes within limits: `transaction` 1,833,741 bytes (+6.90% over `main` at `3dfada58` baseline of 1,715,448), `web` +32 bytes, the other four unchanged ([task.md](task.md), section 1). Hindi messages ready for owner review: `decline-title` (new), `messages-decline-note-placeholder` (changed), `messages-decline-note`, `quote-superseded`, `quote-expired`, `progress-fork`, `messages-waiting-for-other-side` (new). |
| C2a | Merged | Claude Opus 5.5 | Claude Sonnet 5.5 (round 1), Claude Opus 5.5 (rounds 2 to 4) | Gemini 3.8 Flash | Claude Opus 5.5 (rounds 1 to 3) | Merged as 581403ca (squash, PR #370); close-out together with C2b. Code review round 3 on 7edf78e3: 0 open, [review file](slice-c2-review.md). CI passed on 7edf78e3 (all jobs, Playwright E2E included). `mise run verify` with no `--skip` not yet run. Implementer's evidence for the round-2 fixes (R13 in cb6c82bf, R6 in 0a6c13b3, R7 in 0a1cbcc1, R8 in acdd35e0): targeted nextest on roym_core and roym_transaction: 406 passed (0 skipped); roym_web dual_build_parity: 260 passed (0 failed); agreement_lang parity: 2 passed; clippy clean with -D warnings; build:roym built cleanly; check-file-lengths, check-duplication (6.82%), and check-change-docs passed. Hindi messages ready for owner review: quote-size-line ("आकार: { $limit } KiB में से { $used } KiB"), quote-size-over ("(कार्ड सीमा से अधिक)"), quote-size-mailbox ("यह कोटेशन बड़ा है। दूसरे व्यक्ति को यह तभी मिलेगा जब दोनों डिवाइस ऑनलाइन हों।"). [Plan](slice-c2-implementation-plan.md). |
| C2b | Implemented | Claude Opus 5.5 | Claude Sonnet 5.5 (round 1), Claude Opus 5.5 (rounds 2 to 4) | Gemini 3.8 Flash | — | PR pending draft create; tests: roym_core presentation (10 passed), roym_transaction present (4 passed), method_contract (36 passed), substrate roym_disclosure_e2e (2 passed). Sizes within limits: transaction +100,016 bytes (+4.98%), other five components unchanged (+0) ([task.md](task.md), section 1). Same [plan](slice-c2-implementation-plan.md) as C2a. |
| C3a | Not started | — | — | — | — | — |
| C3b | Not started | — | — | — | — | — |
| C4 | Not started | — | — | — | — | — |
| C6 | Not started | — | — | — | — | — |
| C7 | Not started | — | — | — | — | — |
| C8 | Not started | — | — | — | — | — |

C5 (events to an extension service and deal grants) moved out of M08C on 2026-11-15 ([task.md](task.md), section 6, question 3). Its id is not reused.
