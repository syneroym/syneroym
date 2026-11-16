# M08C status

*Reader: developers who build this sub-milestone.*

The plan and the slice order are in [task.md](task.md). This file records what is done, with evidence, and which tool did each step of each slice.

**Step** is one of: Not started, Plan written, Plan reviewed, Implemented, Code reviewed, Fixes done, Merged. "—" means the step has not happened yet.

| Slice | Step | Plan by | Plan review by | Implemented by | Code review by | Evidence |
| --- | --- | --- | --- | --- | --- | --- |
| C0 | Implemented | no plan (docs only) | no plan (docs only) | Claude Opus 5.5 | — | [ADR-0030](../../../decisions/0030-signed-message-origins.md), accepted by the owner 2026-11-16 |
| C1a | Fixes done | Claude Opus 5.5 | Claude Opus 5.5 (rounds 1, 3, 4, 5), Claude Sonnet 5.5 (round 2) | Gemini 3.8 Flash | Claude Opus 5.5 (rounds 1, 2) | [PR #367](https://github.com/syneroym/syneroym/pull/367). Code review round 2 fixes done (c7d00dda, 33df19b9, 601ffb35, 94c36e85, 04402ed6): all 7 items fixed (R16, R17, R18, R19, R20, R13, R14), 0 open, [review file](slice-c1-review.md). 32 passed in syneroym-roym-transaction, 256 passed in dual_build_parity, 2 passed in roym_deal_e2e, 1 passed in sync_backup_shapes. Clean clippy, fmt, xtask checks (file lengths, duplication at 6.95%, change docs, lint suppressions at 38, module layout). |
| C1b | Plan reviewed | Claude Opus 5.5 | Claude Opus 5.5 (rounds 1, 3, 4, 5), Claude Sonnet 5.5 (round 2) | — | — | Same plan, part C1b. Starts after C1a merges. Owner answers recorded, review items P1–P26 closed (same review file, 0 open). |
| C2 | Not started | — | — | — | — | — |
| C3a | Not started | — | — | — | — | — |
| C3b | Not started | — | — | — | — | — |
| C4 | Not started | — | — | — | — | — |
| C6 | Not started | — | — | — | — | — |
| C7 | Not started | — | — | — | — | — |
| C8 | Not started | — | — | — | — | — |

C5 (events to an extension service and deal grants) moved out of M08C on 2026-11-15 ([task.md](task.md), section 6, question 3). Its id is not reused.
