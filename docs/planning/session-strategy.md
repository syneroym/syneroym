# Multi-Agent Session Strategy

To successfully implement the Syneroym architecture, treat repository documents as durable memory and agent sessions (Claude Code or Antigravity) as disposable workers. Do not rely on conversation history.

This guide defines the strict workflow for navigating the Meta-Implementation Plan.

## 1. Tool Roles

There is no fixed assignment of "Antigravity implements, Claude reviews." The one invariant that matters: **the reviewer for a slice must be a different tool than its implementer.** Which tool implements a given slice is a per-slice judgment call, not a program-wide default — and role reversal (Claude implements, Antigravity reviews, or vice versa) is expected and encouraged rather than an exception.

Starting heuristic:
- Slices dominated by the `cargo`/`clippy`/`mise` compile-test-fix loop lean toward whichever tool drives that loop more directly.
- Otherwise, default to alternating, and let evidence decide.

Record the assignment as fact, not preference: every slice's `status.md` must note which tool implemented it and which tool reviewed it. Periodically look back across slices at how many review findings and how much rework each implementer/tool pairing produced, and let *that* — not a priori assumption — settle where each tool ends up doing more implementation.

## 2. Recommended Document Structure

All strategic planning artifacts must live directly in the repository to guarantee cross-session context bootstrapping. Do not place these under `docs/archive` or use `scratch-notes` names.

```text
docs/planning/
├── meta-implementation-plan.md
├── traceability-matrix.md
└── milestones/
    ├── M01-local-app-model/
    │   ├── task.md
    │   └── status.md
    └── ...
docs/decisions/            # this is the decision register — one ADR per decision, no separate index file
├── 0001-sqlite-encryption.md
└── ...
```

M00 predates this task/status convention and lives as flat `m0-task.md` / `m0-status.md` files directly under `milestones/` — treat it as a legacy exception, not a pattern to replicate.

## 3. Organize Sessions by Milestones and Slices

Do not ask one session to implement an entire milestone. The phases remain requirement groupings; the milestones dictate the implementation order.

We use three levels of granularity:
1. **Meta-plan:** One permanent roadmap for the whole program (`meta-implementation-plan.md`).
2. **Milestone plan:** A detailed `task.md` created before implementing that milestone.
3. **Implementation slice:** One agent session per small, independently verifiable slice.

*Example slicing for Milestone 1 (illustrative only — see `meta-implementation-plan.md` for the actual milestone list and slice breakdown):*
- M1A: Domain types and versioned manifest
- M1B: Pure manifest compiler and dependency graph
- M1C: Registry abstractions and logical resolver
- M1D: Deployment journal and recovery
- M1E: `roymctl` migration
- M1F: End-to-end acceptance tests

Each slice should be small enough to review as one coherent change.

---

## 4. The Session Workflow Rhythm

Follow this strict sequence for every milestone:

```text
Meta-plan
  → Milestone Planning Session
  → Slice Implementation Session
  → Review/Fix Sessions (different tool than the implementer)
  → Milestone Closeout Session
  → Next Milestone
```

### A. Milestone Planning Session (Read-Only initially)
Open a new session, in either tool. Ask it to inspect the codebase and produce `task.md`.

**Prompt Template:**
> We are beginning Milestone 1 from `docs/planning/meta-implementation-plan.md`.
>
> Read AGENTS.md, the requirements and architecture documents, the traceability matrix, decision register, and current implementation.
>
> Create a detailed milestone plan at: `docs/planning/milestones/M01-local-app-model/task.md`
>
> Do not implement code yet. Include requirement-level traceability, dependency gates, ordered implementation slices, migration strategy, tests, non-goals, performance budgets, and measurable exit criteria. Identify unresolved decisions before planning around them.

*Review and approve that plan before proceeding to implementation.*

### B. Implementation Session (One per slice)
Open a fresh session for the specific slice, in whichever tool you've assigned (§1), to keep context focused and make failures easier to unwind. Leverage each tool's autonomous mode (e.g. `/goal`) to compile code, run tests, and fix errors until the slice is fully complete.

For complex slices — a genuine design decision, tricky invariants, a cross-boundary contract — have the *other* tool draft an implementation plan first, in its own session, before the assigned coder starts. Just paste that plan into the coder's session; no need to save it as a separate file. Skip this for mechanical slices where the approach is obvious.

**Prompt Template:**
> Implement only slice M1B from `docs/planning/milestones/M01-local-app-model/task.md`. Before writing any code, create an implementation plan and wait for my approval to proceed.
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
> Do not modify code. Report actionable findings with file and line links, and output these findings as a checklist in a markdown artifact so the next implementation session can methodically address them.

*Use a subsequent implementation session (same tool as the original implementer, or a new assignment per §1) to address any accepted findings.* You could use the following prompt template.
> Review the following review comments on implementation of slice xx of Mnn-xxx. Implement what you agree, and push back on or ask for justification of the others. Before writing code, show me your implementation plan and wait for my approval. Once approved, do not stop until the fixes are implemented and all tests pass.
>
> [Copy review comments below...]

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
- Does new information require updating the meta-plan?

**Avoid implementing code in this session.** Its job is navigation and high-level tracking, not construction.

Because it stays open across the whole program, periodically ask it to re-derive its understanding from the current files rather than trusting its own accumulated summary — treat its memory as a cache, not a source of truth.

---

## 6. Rules to Prevent Session Drift

- **Read first:** Every session must begin by reading files from the repository.
- **No history reliance:** Never rely on another session's conversation history. Context lives in the files.
- **Cross-tool review:** The reviewer for a slice must be a different tool than its implementer (§1). Record both in `status.md`.
- **Task vs Status:** `task.md` defines intended work; `status.md` records factual state.
- **Evidence-based:** Update the traceability matrix only when verifiable acceptance evidence exists.
- **No concurrent dependencies:** Do not run dependent slices concurrently. Parallelize only clearly independent research or non-overlapping crates.
- **Serialize shared-file edits:** Even for independent slices, do not let two sessions edit the same shared file (`traceability-matrix.md`, an ADR) concurrently — serialize those updates to avoid silent clobbers.
- **Bounded autonomy:** Cap autonomous implementation loops with a rough attempt or time budget. If a slice isn't converging, stop and escalate rather than letting it thrash.
- **ADRs over chat:** Record architectural decisions as ADRs in `docs/decisions/`, not as chat conclusions.
- **Clean handoffs:** End every implementation session with passing tests and a precise handoff to the next slice.
- **User control:** Keep commits and staging under your control, per `AGENTS.md`.
