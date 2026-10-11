# Roadmap

*Reader: developers and the project owner, when they choose or plan the next
large piece of work.*

This file gives the build order of the large pieces of work (milestones) and
the status of each one. It is an index. It has no design and no task lists.

- A **milestone** is a group of features planned together. Its plan lives in
  its own folder, `docs/planning/milestones/<ID>/` (`task.md` for the plan,
  `status.md` for progress). Each feature in it has its own change doc under
  `docs/planning/changes/`. See [../README.md](../README.md).
- Before 2026-10-11, the roadmap and the milestone sketches lived in one file,
  [meta-implementation-plan.md](meta-implementation-plan.md). That file is now
  closed. It is kept as history, and the "sketched" rows below link to their
  sketch in it.

## How this file changes

- **Add one row** when a new milestone is proposed. Put it at its place in the
  order. Do not write its design here. Write it in the milestone folder.
- **Change the status** of a row when the work moves. Link the folder once it
  exists.
- **The row order is the build order.** The ID does not give the order. IDs
  keep the `M<number><letter>` form so that existing links and backlog targets
  (for example "M7" or "M11+") stay valid. A new milestone between two others
  takes a letter, for example `M06D` after `M06C`.

Status values:

| Status | Meaning |
| --- | --- |
| done | All slices merged. The folder's `status.md` has the evidence. |
| active | Slices are being built. |
| planning | The milestone folder or its change docs are being written. |
| sketched | Only a sketch exists, in the closed meta plan. Plan it in a folder before you build it. |
| deferred | Planned for later on purpose. The row says why. |

## Principles

These hold for every milestone.

1. **Inside out.** Build the local primitives (routing, security, data) before
   multi-node features and applications.
2. **A real app at every step.** Each milestone grows a reference SynApp (today
   Roym) end to end, so we do not build test-only facades.
3. **No bypass of trust boundaries.** A call that crosses a `SynSvc` trust
   boundary always goes through identity and authorization checks. Components
   composed statically into one `SynSvc` count as one boundary.
4. **One planner.** `crates/app_orchestration` compiles a manifest into a
   `DeploymentPlan`. `roymctl` and the substrate only act on that plan. They do
   not plan on their own.
5. **Tooling and observability grow with each milestone**, not at the end.

How sessions plan, build and review a milestone:
[session-strategy.md](session-strategy.md). The completion gate for every
slice is `mise run verify` (see `AGENTS.md`).

## Order

### Next

| Order | ID | Milestone | Status | Notes |
| --- | --- | --- | --- | --- |
| 1 | M06D | Reachability and deferred delivery: registry-listed relays, operator passport, service mailbox | planning | Next. [Folder](milestones/M06D-reachability-and-deferred-delivery/). |
| 2 | M8 | Federation and trust | sketched | [Sketch](meta-implementation-plan.md#milestone-8-federation-and-trust). Moved ahead of M7. |
| 3 | M9 | AI concierge (M9A agentic foundations, M9B advanced orchestration) | sketched | [Sketch](meta-implementation-plan.md#milestone-9-expansion-track---ai-concierge). Moved ahead of M7. |
| 4 | M10 | Mobile edge | sketched | [Sketch](meta-implementation-plan.md#milestone-10-expansion-track---mobile-edge). Moved ahead of M7. Builds on the M06D mailbox, which serves sleeping phones. |

The order of M8, M9 and M10 among themselves is decided when the first of them
is planned.

### Deferred

| ID | Milestone | Status | Why |
| --- | --- | --- | --- |
| M7 | Resilience and operability: replication, redundancy, backups, attestation, metrics pipeline | deferred | Moved after M8 to M10 (2026-10-11). [Sketch](meta-implementation-plan.md#milestone-7-resilience-and-operability). The M8 to M10 sketches do not depend on replication. Check this again when each one is planned. |
| M5 (rest) | Query orchestrator, state versioning and rollback, developer tools, `ControllerAgreement` creation tool | deferred | Deferred on 2026-07-16 to run with M7 ([amendment](meta-implementation-plan.md#build-order-amendment-m5m7-resequencing-2026-07-16), [sketch](meta-implementation-plan.md#milestone-5-async-lifecycle-and-developer-experience)). |
| — | Later-phase additions: identity extensions, product expansion, financial services, MQTT shared subscriptions | deferred | [Sketch](meta-implementation-plan.md#later-phase-additions). Items and reasons: [deferred-backlog.md](deferred-backlog.md). |

### Done

| ID | Milestone | Folder |
| --- | --- | --- |
| M0 | Contract and decision gate | [m0-status.md](milestones/m0-status.md) |
| M1 | Local app model and lifecycle | [M01](milestones/M01-local-app-model/) |
| M2 | Reliable, operable node | [M02](milestones/M02-reliable-node/) |
| M3A, M3B (blob) | Structured state and security; blob storage | [M03](milestones/M03-sss/) |
| M3B (messaging), M3C | Messaging, streams and HTTP bridge | [M03B](milestones/M03B-messaging/) |
| — | Crate rename (interstitial) | [crate-rename-refactor.md](crate-rename-refactor.md) |
| M4A | Proxy and auth foundation | [M04A](milestones/M04A-proxy-and-auth-foundation/) |
| — | Deploy-time artifact delivery (interstitial) | [fnd-cfg-artifact-delivery-plan.md](fnd-cfg-artifact-delivery-plan.md) |
| M4B | FDAE data-aware authorization | [M04B](milestones/M04B-fdae-policy/) |
| M5A | App supervisor | [M05A](milestones/M05A-app-supervisor/) |
| M5B | Async primitives | [M05B](milestones/M05B-async-primitives/) |
| M5C | Logical discovery overlay. S1 to S3 done. S4 (cross-app `Bind`) is parked in the [backlog](deferred-backlog.md). | [M05C](milestones/M05C-logical-discovery-overlay/) |
| M6A | App platform surface | [M06A](milestones/M06A-app-platform-surface/) |
| M6B | Roym substrate foundations | [M06B](milestones/M06B-roym-substrate-foundations/) |
| M6C | Roym product | [M06C](milestones/M06C-roym-product/) |
| — | Conversation capability consolidation (interstitial) | [conversation-consolidation-plan.md](conversation-consolidation-plan.md) |
