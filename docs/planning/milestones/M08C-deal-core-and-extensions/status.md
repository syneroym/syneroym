# M08C status

*Reader: developers who build this sub-milestone.*

The plan and the slice order are in [task.md](task.md). This file records what is done, with evidence, and which tool did each step of each slice.

**Step** is one of: Not started, Plan written, Plan reviewed, Implemented, Code reviewed, Fixes done, Merged. "—" means the step has not happened yet.

| Slice | Step | Plan by | Plan review by | Implemented by | Code review by | Evidence |
| --- | --- | --- | --- | --- | --- | --- |
| C0 | Implemented | no plan (docs only) | no plan (docs only) | Claude Opus 5.5 | — | [ADR-0030](../../../decisions/0030-signed-message-origins.md), accepted by the owner 2026-11-16 |
| C1a | Fixes done | Claude Opus 5.5 | Claude Opus 5.5 (rounds 1, 3, 4, 5), Claude Sonnet 5.5 (round 2) | Gemini 3.8 Flash | Claude Opus 5.5 (round 1) | [PR #367](https://github.com/syneroym/syneroym/pull/367). Code review round 1 fixes complete (all 15 items closed: R1 in 1023868e; R2-R4, R7-R15 in 98bf5d3d; R6 in 4f03d8aa; R5 in 2b950f7a). Tests passing: 23 in roym-transaction, 290 in roym-core, 257 in dual_build_parity (incl. deal parity), 2 in roym_deal_e2e. Owner reviewed new Hindi messages. Clean clippy, fmt, xtask checks. |
| C1b | Plan reviewed | Claude Opus 5.5 | Claude Opus 5.5 (rounds 1, 3, 4, 5), Claude Sonnet 5.5 (round 2) | — | — | Same plan, part C1b. Starts after C1a merges. Owner answers recorded, review items P1–P26 closed (same review file, 0 open). |
| C2 | Not started | — | — | — | — | — |
| C3a | Not started | — | — | — | — | — |
| C3b | Not started | — | — | — | — | — |
| C4 | Not started | — | — | — | — | — |
| C6 | Not started | — | — | — | — | — |
| C7 | Not started | — | — | — | — | — |
| C8 | Not started | — | — | — | — | — |

C5 (events to an extension service and deal grants) moved out of M08C on 2026-11-15 ([task.md](task.md), section 6, question 3). Its id is not reused.
