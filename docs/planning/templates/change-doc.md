---
status: draft
living-docs-touched:
---
# <Feature name>

<!--
Copy this file to docs/planning/changes/<kebab-case-name>/change.md.
`status` is one of: draft, approved, in-progress, done, abandoned.
When status becomes `done`, list the living docs you updated under
`living-docs-touched`, one path per line, for example:
  - docs/system-architecture.md
Or write a single line `  - none: <reason>` if no living doc needed a change.
Delete these comments when you copy the file.
-->

## Problem

Who has the problem, and what happens today? Two or three short paragraphs.

## Scope and non-goals

- In scope:
- Not in scope:

## Acceptance criteria

Give each criterion an ID. Name the test that proves it when you write the test.

| ID | Criterion | Test |
| --- | --- | --- |
| AC-1 | | |

## Design

How the feature works, and how it fits into the current system. Link the living
doc for each existing component you rely on, and mark any claim you have not
checked against the code as `unverified`.

Rejected options and the reason for each. Link an ADR for any decision that is
hard to reverse.

## Open questions

- 

## Tasks

- [ ] 

## Deviations

Add one line each time the code or the plan differs from what this document
said. Write it at the moment you notice, not at the end.

## Close-out

Before merge, update the living docs (requirements, architecture, user guide)
so they describe the result. Then set `status: done` and fill in
`living-docs-touched`. Record anything you postponed in
[deferred-backlog.md](../../deferred-backlog.md).
