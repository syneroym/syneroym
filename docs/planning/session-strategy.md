# Antigravity Session Strategy

To successfully implement the Syneroym architecture, treat repository documents as durable memory and Antigravity windows as disposable workers. Do not rely on conversation history. 

This guide defines the strict workflow for navigating the Meta-Implementation Plan.

## 1. Recommended Document Structure

All strategic planning artifacts must live directly in the repository to guarantee cross-window context bootstrapping. Do not place these under `docs/archive` or use `scratch-notes` names.

```text
docs/planning/
├── meta-implementation-plan.md
├── traceability-matrix.md
├── decision-register.md
└── milestones/
    ├── M00-contract-decision/
    │   ├── task.md
    │   └── status.md
    ├── M01-local-app-model/
    │   ├── task.md
    │   └── status.md
    └── ...
docs/decisions/
├── 0001-sqlite-encryption.md
└── ...
```

## 2. Organize Windows by Milestones and Slices

Do not ask one window to implement an entire milestone. The phases remain requirement groupings; the milestones dictate the implementation order. 

We use three levels of granularity:
1. **Meta-plan:** One permanent roadmap for the whole program (`meta-implementation-plan.md`).
2. **Milestone plan:** A detailed `task.md` created before implementing that milestone.
3. **Implementation slice:** One Antigravity window per small, independently verifiable slice.

*Example slicing for Milestone 1:*
- M1A: Domain types and versioned manifest
- M1B: Pure manifest compiler and dependency graph
- M1C: Registry abstractions and logical resolver
- M1D: Deployment journal and recovery
- M1E: `roymctl` migration
- M1F: End-to-end acceptance tests

Each slice should be small enough to review as one coherent change.

---

## 3. The Window Workflow Rhythm

Follow this strict sequence for every milestone:

```text
Meta-plan 
  → Milestone Planning Window 
  → Slice Implementation Window 
  → Review/Fix Windows 
  → Milestone Closeout Window 
  → Next Milestone
```

### A. Milestone Planning Window (Read-Only initially)
Open a new window. Ask it to inspect the codebase and produce `task.md`.

**Prompt Template:**
> We are beginning Milestone 1 from `docs/planning/meta-implementation-plan.md`.
>
> Read AGENTS.md, the requirements and architecture documents, the traceability matrix, decision register, and current implementation.
>
> Create a detailed milestone plan at: `docs/planning/milestones/M01-local-app-model/task.md`
>
> Do not implement code yet. Include requirement-level traceability, dependency gates, ordered implementation slices, migration strategy, tests, non-goals, performance budgets, and measurable exit criteria. Identify unresolved decisions before planning around them.

*Review and approve that plan before proceeding to implementation.*

### B. Implementation Window (One per slice)
Open a fresh window for the specific slice to keep context focused and make failures easier to unwind.

**Prompt Template:**
> Implement only slice M1B from `docs/planning/milestones/M01-local-app-model/task.md`.
>
> Read the canonical project documents and inspect the current worktree before changing anything. Preserve unrelated changes. Do not commit or stage files.
>
> Implement the slice completely, add its tests, and update `task.md` and `status.md` with factual progress and verification evidence. Run the relevant tests for this slice and paste the passing output into `status.md`. Do not begin the next slice.

### C. Review Window (Read-Only)
Use a fresh window after every substantial slice to act as a reviewer.

**Prompt Template:**
> Review the implementation of slice M1B against its requirements and acceptance criteria. Diagnose correctness, security, concurrency, WASM-boundary, migration, and test-coverage issues.
>
> Do not modify code. Report actionable findings with file and line links, and output these findings as a checklist in a markdown artifact so the next implementation window can methodically address them.

*Use a subsequent implementation window to address any accepted findings.*

### D. Milestone Closeout Window (Read-Only)
When all slices are complete, open a final window to verify the milestone.

**Prompt Template:**
> Audit Milestone 1 for completion.
>
> Compare `task.md`, the traceability matrix, requirements, architecture, code, and tests. Run all required validation commands. Confirm every exit criterion with evidence. Update `status.md` and `traceability-matrix.md`.
>
> Do not mark the milestone complete if any requirement, test, decision, or migration task remains unresolved.

---

## 4. The "Program Window" Exception

You can retain **one** lightweight Antigravity window purely for roadmap-level coordination:
- Which milestone is active?
- What is blocked?
- Which ADR is unresolved?
- Is the next slice safe to start?
- Does new information require updating the meta-plan?

**Avoid implementing code in this window.** Its job is navigation and high-level tracking, not construction.

---

## 5. Rules to Prevent Session Drift

- **Read first:** Every window must begin by reading files from the repository.
- **No history reliance:** Never rely on another window’s conversation history. Context lives in the files.
- **Task vs Status:** `task.md` defines intended work; `status.md` records factual state.
- **Evidence-based:** Update the traceability matrix only when verifiable acceptance evidence exists.
- **No concurrent dependencies:** Do not run dependent slices concurrently. Parallelize only clearly independent research or non-overlapping crates.
- **ADRs over chat:** Record architectural decisions as ADRs in `docs/decisions/`, not as chat conclusions.
- **Clean handoffs:** End every implementation window with passing tests and a precise handoff to the next slice.
- **User control:** Keep commits and staging under your control, per `AGENTS.md`.
