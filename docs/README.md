# Documentation map

Syneroym docs have five kinds. Each kind has a different lifetime. Do not mix them.

| Kind | Answers | Lifetime | Where |
| --- | --- | --- | --- |
| Living docs | How does the system work **now**? | Always current | See the table below |
| Change docs | What do we plan to change, and why? | Temporary, kept as history after the work | `docs/planning/changes/<name>/change.md`, `docs/planning/milestones/` |
| ADRs | Why did we decide this? | Permanent, one decision each | `docs/decisions/` |
| Backlog | What did we postpone? | Running list | `docs/planning/deferred-backlog.md` |
| Ideas | What might we build one day? | Until promoted or rejected | `docs/ideas/` |

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

## Planning docs

These are shared project history, not private notes. You do not need them to
understand the current system. The living docs are the source for that.

| Doc | What it is | How to use it |
| --- | --- | --- |
| [decisions/](decisions/) (ADRs) | The permanent record of one decision and its reason | Read them. Link them from anywhere. |
| [planning/meta-implementation-plan.md](planning/meta-implementation-plan.md) | Roadmap and order of the large pieces of work | Read it to see what is planned. |
| [planning/milestones/](planning/milestones/) | Plans and progress logs (`task.md`, `status.md`, slice plans) for large efforts built from many features. The structure varies between folders, because it grew over time. | History. Do not copy the format. Do not trust `status.md` for the current state of the system. |
| [planning/changes/](planning/changes/) | The change doc for one feature | Use the [template](planning/templates/change-doc.md). |
| [planning/deferred-backlog.md](planning/deferred-backlog.md) | What we postponed, with the reason | Update it when you postpone something. |
| [planning/session-strategy.md](planning/session-strategy.md) | How agent sessions are organized | Read it before planning a milestone. |

**One feature** gets a change doc under `planning/changes/`.
**A group of features planned together** gets a milestone folder as an umbrella.
It holds the scope and the order of the work. Each feature in it still gets its
own change doc. Existing milestone folders stay as they are.

## Rules

1. **Living docs describe the result, not the plan.** Write "the router does X".
   Do not write "we added X in milestone Y" and do not link a change doc. Link
   ADRs only.
2. **A change doc is the workspace for one feature.** Start from
   [planning/templates/change-doc.md](planning/templates/change-doc.md). Keep the
   spec, the design and the list of deviations in it. See *Planning docs* above
   for when a feature belongs under a milestone.
3. **Write deviations while you work.** When the code differs from the plan, add
   a line to `## Deviations` at that moment.
4. **Close out before merge.** Update the living docs, then set
   `status: done` and list the files in `living-docs-touched`.
   `cargo xtask check-change-docs` checks this.
5. **Small changes skip the change doc.** A bug fix or a config option needs only
   the living-doc edit in the same PR.
6. **Ideas are not authoritative.** Notes in `docs/ideas/` are not requirements
   or designs. Do not implement them. Living docs and code never link to them: an
   idea appears in a living doc only after it is promoted and built. See
   [ideas/README.md](ideas/README.md).
