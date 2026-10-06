---
status: in-progress
living-docs-touched:
---
# Living docs baseline

Reader: a developer who plans or does the docs round.

## Problem

The architecture doc ([system-architecture.md](../../../system-architecture.md))
has about 2,300 lines. It was written in stages, and the code has changed
since. We do not know which statements are still true. A reader who trusts the
doc can believe that a feature works when it does not.

The requirements, developer guide and traceability matrix may have the same
problem. The doc is also too big to review in one pass, and it has no end user
guide.

## Scope and non-goals

The whole docs round has four steps. This change doc tracks all of them.

1. **Audit.** Check every factual claim in a section against the code. Record
   the result in a report. The first audit is a pilot on one section.
2. **Split.** Move each section into small files, one per capability, each with
   a short index. This follows the plan in [docs/README.md](../../../README.md).
3. **Fix.** Correct the wrong statements. Mark planned work as `planned`. Remove
   milestone and slice IDs from living docs.
4. **User guide.** Write a guide for end users. It says what a person can do and
   how, with no internals.

- In scope now: the pilot audit of the "Layer 2 — Substrate Runtime" section.
  The report is [audit-architecture-layer2.md](audit-architecture-layer2.md).
- Not in scope now: editing the architecture doc, editing any code, and the
  later steps above.

## Acceptance criteria

| ID | Criterion | Test |
| --- | --- | --- |
| AC-1 | Every claim in the pilot section has one verdict. Every MATCHES verdict has a `file:line` citation. | Review of the report |
| AC-2 | The report lists structural problems of the whole file and the questions that need the user. | Review of the report |
| AC-3 | This change doc passes the change-doc check. | `cargo xtask check-change-docs` |

## Design

The audit uses only the code and the tests as proof. Planning docs under
`docs/planning/milestones` are not proof. ADRs help to understand intent.

Each claim gets one verdict: MATCHES, DIVERGES, NOT BUILT, STALE or UNCLEAR.
A claim with no citation counts as UNCLEAR.

Default rule for DIVERGES and NOT BUILT: if the code does it, the code is right
and the doc needs a fix. If the code does not do it and the doc clearly means
future work, the proposed status is `planned`. The user decides only when the
intent is unclear.

## Open questions

See section 4 of [audit-architecture-layer2.md](audit-architecture-layer2.md).

## Tasks

- [x] Audit "Layer 2 — Substrate Runtime" (pilot).
- [ ] Audit the other sections of the architecture doc.
- [ ] Audit the requirements, developer guide and traceability matrix.
- [ ] Split the architecture doc into capability files.
- [ ] Fix the wrong statements found by the audits.
- [ ] Write the end user guide.

## Deviations

- The pilot report carries 61 claims for about 140 doc lines, which is more than the
  "one claim per statement" plan expected, because the diagram nodes and the
  table rows each count as a claim.

## Close-out

Before merge of the last step, update the living docs so they describe the
result. Then set `status: done` and fill in `living-docs-touched`. Record
anything postponed in [deferred-backlog.md](../../deferred-backlog.md).
