# Multi-Agent Session Strategy

To successfully implement the Syneroym architecture, treat repository documents as durable memory and agent sessions (Claude Code or Antigravity) as disposable workers. Do not rely on conversation history.

This guide defines the strict workflow for navigating the Meta-Implementation Plan.

## 1. Tool Roles

There is no fixed assignment of "Antigravity implements, Claude reviews." The one invariant that matters: **the reviewer for a slice must be a different tool than its implementer, and the reviewer of a plan must be a different tool than the author of that plan.** Which tool implements a given slice is a per-slice judgment call, not a program-wide default — and role reversal (Claude implements, Antigravity reviews, or vice versa) is expected and encouraged rather than an exception.

Starting heuristic:
- Slices dominated by the `cargo`/`clippy`/`mise` compile-test-fix loop lean toward whichever tool drives that loop more directly.
- Otherwise, default to alternating, and let evidence decide.

Record the assignment as fact, not preference: every slice's row in `status.md` must name the tool that wrote the plan, reviewed the plan, implemented the slice and reviewed the code. Periodically look back across slices at how many review findings and how much rework each implementer/tool pairing produced, and let *that* — not a priori assumption — settle where each tool ends up doing more implementation.

## 2. Recommended Document Structure

All strategic planning artifacts must live directly in the repository to guarantee cross-session context bootstrapping. Do not place these under `docs/archive` or use `scratch-notes` names.

```text
docs/planning/
├── roadmap.md                  # build order and status, one row per milestone
├── meta-implementation-plan.md # closed 2026-10-11, history only
├── traceability-matrix.md
├── changes/
│   └── <feature>/change.md     # one per feature
└── milestones/
    ├── M01-local-app-model/
    │   ├── task.md
    │   ├── status.md
    │   ├── slice-<id>-implementation-plan.md
    │   └── slice-<id>-review.md
    └── ...
docs/decisions/            # this is the decision register — one ADR per decision, no separate index file
├── 0001-sqlite-encryption.md
└── ...
```

M00 predates this task/status convention and lives as flat `m0-task.md` / `m0-status.md` files directly under `milestones/` — treat it as a legacy exception, not a pattern to replicate.

## 3. Organize Sessions by Milestones and Slices

Do not ask one session to implement an entire milestone. The phases remain requirement groupings; the milestones dictate the implementation order.

We use three levels of granularity:
1. **Roadmap:** One permanent index of milestones, in build order (`roadmap.md`). It grows by one row per milestone and holds no design.
2. **Milestone plan:** A detailed `task.md` created before implementing that milestone. Each feature in the milestone also has its own change doc under `changes/`.
3. **Implementation slice:** One agent session per small, independently verifiable slice.

*Example slicing for Milestone 1 (illustrative only — see `roadmap.md` for the actual milestone list, and each milestone's `task.md` for its slices):*
- M1A: Domain types and versioned manifest
- M1B: Pure manifest compiler and dependency graph
- M1C: Registry abstractions and logical resolver
- M1D: Deployment journal and recovery
- M1E: `roymctl` migration
- M1F: End-to-end acceptance tests

Each slice should be small enough to review as one coherent change.

### Slice steps and status values

Every slice has one row in the milestone's `status.md`. The row shows the step the slice has reached. The step is one of these, in order:

1. Not started
2. Plan written
3. Plan reviewed
4. Implemented
5. Code reviewed
6. Fixes done
7. Merged

Every row also records the tool for four jobs: plan, plan review, implementation and code review. Write "—" for a job that has no step yet. Write "not recorded" only for old work where nobody wrote the tool down.

### The review file

Each slice has one review file, `slice-<id>-review.md`, next to its plan. The file is the single place for review findings. Do not paste findings into chat.

- Plan review findings have ids P1, P2, … Code review findings have ids R1, R2, …
- A new review round is a new section. Its title has the kind of review, the round number, the tool and the date. Example: "Code review round 2 — <tool>, <date>". The reviewer writes its own tool name there.
- Ids are never renumbered. A new round continues the numbers.
- Every item has a status: `Open`, `Fixed (<sha>)`, `Rejected: <reason>` or `Deferred: <backlog link>`.
- The fix session changes the status of each item as it works.
- A later reviewer checks only the open items, and confirms the fixed ones.

---

## 4. The Session Workflow Rhythm

Follow this strict sequence for every milestone:

```text
Roadmap
  → Milestone Planning Session
  → Slice Implementation Session
  → Review/Fix Sessions (different tool than the implementer)
  → Milestone Closeout Session
  → Next Milestone
```

### A. Milestone Planning Session (Read-Only initially)
Open a new session, in either tool. Ask it to inspect the codebase and produce `task.md`.

**Prompt Template:**
> We are beginning Milestone 1 from `docs/planning/roadmap.md`.
>
> Read AGENTS.md, the requirements and architecture documents, the traceability matrix, decision register, and current implementation.
>
> Create a detailed milestone plan at: `docs/planning/milestones/M01-local-app-model/task.md`
>
> Do not implement code yet. Include requirement-level traceability, dependency gates, ordered implementation slices, migration strategy, tests, non-goals, performance budgets, and measurable exit criteria. Identify unresolved decisions before planning around them.

*Review and approve that plan before proceeding to implementation.*

At the start of a milestone:
- Refresh `task.md` against the current code, and create `status.md`.
- Write and review every change doc that the milestone depends on. Do this before the first slice plan.

### B. Implementation Session (One per slice)
Open a fresh session for the specific slice, in whichever tool you've assigned (§1), to keep context focused and make failures easier to unwind. Leverage each tool's autonomous mode (e.g. `/goal`) to compile code, run tests, and fix errors until the slice is fully complete.

For complex slices — a genuine design decision, tricky invariants, a cross-boundary contract — have the *other* tool draft an implementation plan first, in its own session, before the assigned coder starts. Just paste that plan into the coder's session; no need to save it as a separate file. Skip this for mechanical slices where the approach is obvious.

**Prompt Template:**
> Implement only slice M1B from `docs/planning/milestones/M01-local-app-model/task.md`. Before writing any code, create an implementation plan and wait for my approval to proceed.
>
> Write the plan so a different tool could execute it without your reasoning: step-by-step per file, exact function/type signatures, pseudo-code for non-trivial logic, explicit constraints — not shorthand only you would follow.
>
> [If applicable:] Here is an independent implementation plan for this slice from the other tool: [paste plan]. Fold in anything that improves correctness, robustness, or simplicity, and note what you rejected and why.
>
> Read the canonical project documents and inspect the current worktree before changing anything. Preserve unrelated changes. Do not commit or stage files.
>
> Implement the slice completely, add its tests, and update `task.md` and `status.md` with factual progress and verification evidence, including which tool implemented this slice. Run the relevant tests for this slice and paste the passing output into `status.md`. Do not stop until all tests and clippy checks pass for this slice. Do not begin the next slice.
>
> If you are not converging after a reasonable number of attempts, stop and report what's blocking you instead of continuing to thrash.

### C. Review Session (Read-Only, must be the other tool)
Use a fresh session, in the tool that did **not** implement this slice, after every substantial slice to act as a reviewer.

**Prompt Template:**
> Review the implementation of slice M1B against its requirements and acceptance criteria. Diagnose correctness, security, concurrency, WASM-boundary, migration, and test-coverage issues.
>
> Independently re-run the tests, clippy, and any acceptance commands yourself — do not rely on the pasted output in `status.md`.
>
> Do not modify code. Report actionable findings with file and line links, precise enough that a different tool can act on them directly without asking you to clarify, and write these findings as checklist items, with ids and status `Open`, in `slice-<id>-review.md` next to the slice plan. Add a new section for this round. Its title has your tool name and the date. Do not paste the findings only into chat.

*Use a subsequent implementation session (same tool as the original implementer, or a new assignment per §1) to address any accepted findings.* You could use the following prompt template.
> Read the open items in `slice-<id>-review.md` for slice xx of Mnn-xxx. Implement what you agree, and push back on or ask for justification of the others. Before writing code, show me your implementation plan and wait for my approval. Once approved, do not stop until the fixes are implemented and all tests pass. Update the status of each item in the review file as you work: `Fixed (<sha>)`, `Rejected: <reason>` or `Deferred: <backlog link>`.

### D. Milestone Closeout Session (Read-Only)
When all slices are complete, open a final session — ideally the tool that reviewed the majority of the milestone's slices, for a fresh audit — to verify the milestone.

**Prompt Template:**
> Audit Milestone 1 for completion.
>
> Compare `task.md`, the traceability matrix, requirements, architecture, code, and tests. Run all required validation commands. Confirm every exit criterion with evidence. Update `status.md` and `traceability-matrix.md`.
>
> Do not mark the milestone complete if any requirement, test, decision, or migration task remains unresolved.

---

## 5. The "Program Session" Exception

You can retain **one** lightweight session, in either tool, purely for roadmap-level coordination:
- Which milestone is active?
- What is blocked?
- Which ADR is unresolved?
- Is the next slice safe to start?
- Does new information require updating the roadmap?

**Avoid implementing code in this session.** Its job is navigation and high-level tracking, not construction.

Because it stays open across the whole program, periodically ask it to re-derive its understanding from the current files rather than trusting its own accumulated summary — treat its memory as a cache, not a source of truth.

---

## 6. Rules to Prevent Session Drift

- **Read first:** Every session must begin by reading files from the repository.
- **No history reliance:** Never rely on another session's conversation history. Context lives in the files.
- **Cross-tool review:** The reviewer for a slice must be a different tool than its implementer, and the reviewer of a plan must be a different tool than the plan author (§1). Record all four tools in `status.md`.
- **Write for the other tool:** Plans and review findings must be explicit enough for a different tool to execute or act on without the author's reasoning — step-by-step, exact signatures/schemas, pseudo-code for non-trivial logic — not shorthand only the author would follow.
- **Task vs Status:** `task.md` defines intended work; `status.md` records factual state.
- **Evidence-based:** Update the traceability matrix only when verifiable acceptance evidence exists.
- **No concurrent dependencies:** Do not run dependent slices concurrently. Parallelize only clearly independent research or non-overlapping crates.
- **Serialize shared-file edits:** Even for independent slices, do not let two sessions edit the same shared file (`traceability-matrix.md`, an ADR) concurrently — serialize those updates to avoid silent clobbers.
- **Bounded autonomy:** Cap autonomous implementation loops with a rough attempt or time budget. If a slice isn't converging, stop and escalate rather than letting it thrash.
- **ADRs over chat:** Record architectural decisions as ADRs in `docs/decisions/`, not as chat conclusions.
- **Clean handoffs:** End every implementation session with passing tests and a precise handoff to the next slice.
- **User control:** Keep commits and staging under your control, per `AGENTS.md`.
