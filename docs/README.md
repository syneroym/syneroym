# Documentation map

Syneroym docs have four kinds. Each kind has a different lifetime. Do not mix them.

| Kind | Answers | Lifetime | Where |
| --- | --- | --- | --- |
| Living docs | How does the system work **now**? | Always current | See the table below |
| Change docs | What do we plan to change, and why? | Temporary, kept as history after the work | `docs/planning/changes/<name>/change.md`, `docs/planning/milestones/` |
| ADRs | Why did we decide this? | Permanent, one decision each | `docs/decisions/` |
| Backlog | What did we postpone? | Running list | `docs/planning/deferred-backlog.md` |

## Living docs

| Topic | File |
| --- | --- |
| Vision (stable, not a description of the current system) | [VISION.md](VISION.md) |
| Requirements | [system-requirements-spec.md](system-requirements-spec.md) |
| Requirement status and evidence | [planning/traceability-matrix.md](planning/traceability-matrix.md) |
| Architecture | [system-architecture.md](system-architecture.md) |
| Developer guide | [developer-guide.md](developer-guide.md) |
| Terms | [TERMINOLOGY.md](TERMINOLOGY.md) |

These files are being reorganized into smaller files, one per capability, each
with a short index. Until that finishes, update the file in the table above.

## Rules

1. **Living docs describe the result, not the plan.** Write "the router does X".
   Do not write "we added X in milestone Y" and do not link a change doc. Link
   ADRs only.
2. **A change doc is the workspace for one feature.** Start from
   [planning/templates/change-doc.md](planning/templates/change-doc.md). Keep the
   spec, the design and the list of deviations in it.
3. **Write deviations while you work.** When the code differs from the plan, add
   a line to `## Deviations` at that moment.
4. **Close out before merge.** Update the living docs, then set
   `status: done` and list the files in `living-docs-touched`.
   `cargo xtask check-change-docs` checks this.
5. **Small changes skip the change doc.** A bug fix or a config option needs only
   the living-doc edit in the same PR.
6. **Milestone folders keep their own format.** Files under
   `docs/planning/milestones/` use `task.md` and `status.md`, as described in
   [planning/session-strategy.md](planning/session-strategy.md). Only new
   features under `docs/planning/changes/` use the change doc format.
