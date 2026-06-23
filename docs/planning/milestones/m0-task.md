# Milestone 0: Contract and Decision Gate

**Goal:** Formalize the traceability matrix and create a baseline API migration plan for the existing codebase before major structural changes begin.

## Requirement IDs
- N/A (Meta-milestone focusing on planning and architecture decisions rather than implementation features).

## Explicit Non-Goals
- Writing code for structural changes.
- Finalizing the full Phase 1-10 traceability matrix (only the structure and first few core requirements are needed here).
- Resolving non-blocking architectural decisions.

## Dependency Gates
- None.

## Migration Impact
- Drafting the baseline migration plan for `roymctl` contracts. No actual migration execution in this milestone.

## Runnable Reference Scenario
- Not applicable for this planning milestone.

## Performance Budgets
- Not applicable.

## Measurable Exit Criteria
- `docs/planning/traceability-matrix.md` is created and the structure is validated with a few core requirements.
- Milestone-blocking ADRs (DLN Scope, SQLite Encryption mechanism, Manifest Versioning boundaries) are resolved and documented in the Decision Register.
- Baseline migration plan for `roymctl` is drafted and approved.
- All code repositories pass the standard suite:
  - `cargo +nightly fmt --all`
  - `cargo clippy --workspace --all-targets --all-features`
  - `cargo test --workspace`

## Decision Register

The following milestone-blocking architectural decisions need to be resolved:

### 1. DLN Scope Resolution
**Context:** Explicitly decide the target milestone for the Dynamic Ledger Network (DLN) and whether signed-interaction-receipts (and therefore robust `[P2P-REP]` reputation) are scheduled early or assigned to later milestones.
**Status:** Resolved. The full DLN and signed-interaction-receipts (`[P2P-REP]`) are explicitly deferred to Future Product Phases (Milestone 11+) as they are not needed for earlier planned capabilities.

### 2. SQLite Encryption Mechanism
**Context:** Build an ADR/feasibility prototype for the exact encrypted-SQLite mechanism to be used in M3 (Secure Stateful Services).
**Status:** Resolved. We will use `rusqlite` bundled with the `sqlcipher` feature. This maintains maximum stability and full support for required mainline extensions (like `sqlite-vec` and `FTS5`) at runtime. We accept the trade-off of increased build-pipeline complexity (managing C/OpenSSL cross-compilation) to ensure rock-solid runtime cross-platform compatibility without resorting to non-standard VFS hacks.

### 3. Manifest Versioning Boundaries
**Context:** Define the boundaries and structure for manifest versioning.
**Status:** Resolved. We will adopt a "Dual Versioning" (Helm-like) model. App Developers publish immutable `SynApp` blueprints with strict versioning. Providers deploy these using an independently versioned "Instance Configuration" (overlays for quotas, bindings). The orchestration engine compiles both into a strictly immutable `DeploymentPlan` for the active controller.

## Tasks
- [x] Scaffold Traceability Matrix (`docs/planning/traceability-matrix.md`).
- [x] Scaffold M0 Decision Register.
- [x] Resolve DLN Scope Resolution ADR.
- [x] Resolve SQLite Encryption Mechanism ADR.
- [x] Resolve Manifest Versioning Boundaries ADR.
- [x] Draft baseline migration plan for `roymctl`.
