# M08C status

*Reader: developers who build this sub-milestone.*

The plan and the slice order are in [task.md](task.md). This file records what is done, with evidence, and which tool did each step of each slice.

**Step** is one of: Not started, Plan written, Plan reviewed, Implemented, Code reviewed, Fixes done, Merged. "—" means the step has not happened yet.

| Slice | Step | Plan by | Plan review by | Implemented by | Code review by | Evidence |
| --- | --- | --- | --- | --- | --- | --- |
| C0 | Implemented | no plan (docs only) | no plan (docs only) | Claude Opus 5.5 | — | [ADR-0030](../../../decisions/0030-signed-message-origins.md), accepted by the owner 2026-11-16 |
| C1a | Plan reviewed | Claude Opus 5.5 | Claude Opus 5.5 (rounds 1, 3), Claude Sonnet 5.5 (round 2) | — | — | [Plan](slice-c1-implementation-plan.md) (one plan for C1a and C1b), 2026-11-15. Owner answers recorded (Q1 to Q6, 2026-11-16). Review rounds 1 to 3 items P1–P25 closed (25 fixed in e2a3898b, 0 open, [review](slice-c1-review.md)). Baseline component sizes in plan 2.2 point 10. |
| C1b | Plan reviewed | Claude Opus 5.5 | Claude Opus 5.5 (rounds 1, 3), Claude Sonnet 5.5 (round 2) | — | — | Same plan, part C1b. Starts after C1a merges. Owner answers recorded, review items P1–P25 closed (same review file, 0 open). |
| C2 | Not started | — | — | — | — | — |
| C3a | Not started | — | — | — | — | — |
| C3b | Not started | — | — | — | — | — |
| C4 | Not started | — | — | — | — | — |
| C6 | Not started | — | — | — | — | — |
| C7 | Not started | — | — | — | — | — |
| C8 | Not started | — | — | — | — | — |

C5 (events to an extension service and deal grants) moved out of M08C on 2026-11-15 ([task.md](task.md), section 6, question 3). Its id is not reused.
