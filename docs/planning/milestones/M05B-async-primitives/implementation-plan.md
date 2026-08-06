# M05B Implementation Plan — Async Primitives (milestone level)

**Status:** 📋 Planned (2026-08-04). B1 starting. Milestone:
[task.md](task.md). Design of record:
[ADR-0023](../../../decisions/0023-durable-async-primitives.md) (**Accepted
2026-08-04**, see §2 for why acceptance was step zero). Depends on **M05A P0,
A0–A5e, A7 — all Complete**. Gates **M6** and **M05A slice A6**.

**This document is milestone level only.** It holds the split call,
cross-cutting findings, cross-cutting decisions, the slice sequence and merge
order, and the docs/backlog impact. **Each slice's own `§0`, decisions, phases,
tests, and review passes live in that slice's own file** —
[slice-b1-implementation-plan.md](slice-b1-implementation-plan.md) is the first.
§2's *File discipline* note explains why this rule exists and what it is
preventing.

**The one-sentence summary.** Every retry in this tree today lives in a `tokio`
task and dies with the process; this milestone gives failed work a durable home,
a terminal state an operator can see, and a clock.

**Scope decided with the requester (2026-08-04), before drafting:**

- Build the shared queue core plus the two mechanisms that have real consumers
  (supervisor durable delivery, guest outbox/DLQ). Long-running-task restart
  rules and sagas ship as **convention plus a small helper**, not an execution
  engine — the requirements spec rejects durable execution itself.
- **The A6 swap executes here**, as slice B1, not separately afterwards. A
  primitive with only test consumers is an unvalidated primitive.
- **No lease work in the community registry.** The requester's reasoning was
  that the "infrastructure single-writer" problem is not a Syneroym problem:
  substrates own pieces of the registry and coordinators only relay. Checking
  the write path confirmed it — see §0.4, which is where that conversation is
  recorded. ADR-0023 §6 carries the conclusion.

---

## §0 — Cross-cutting findings

Findings that hold across the milestone, or that set a slice boundary. Findings
about one slice's own implementation live in that slice's file.

### 0.1 (Scope-changing) This is five mechanisms sharing a theme, not one mechanism, and only two have a consumer

`meta-implementation-plan.md` item 1 lists the Outbox queue, cron leases, the
DLQ, long-running-task restart rules, and sagas in one sentence. They share a
theme — work that outlives the call that asked for it — and share almost no
mechanism. The outbox and the DLQ are two halves of one store. Scheduling is a
clock and a selection rule. Restart rules need an invocation path that does not
exist (§0.5). Sagas need a WIT convention and a step log (§0.6).

This is the same shape as **M05A slice A5**, whose first-pass plan scoped "the
supervisor loop" as one slice and split into A5a–A5e once its own `§0.1` found
that its parts do not depend on each other in one chain. Making the same call
here up front rather than mid-slice: **five slices**, listed in
[task.md](task.md) — four built here as B1–B4, one deferred (see below).

**The item's own ordering is wrong in one place**, and this is what made the
deferral decision easy. It lists restart rules before sagas, but the
requirements spec defines a non-restartable task's failure path as "runs
compensations" — so compensations must exist first. The dependency runs one way
only.

**B5 (long-running tasks) is therefore deferred out of this milestone**
(decided with the requester, 2026-08-04). It is the only mechanism with no
consumer, no near-term consumer, and genuinely new machinery (§0.5), and because
nothing depends on it, removing it costs nothing here: B4 keeps its own
near-term consumer in M6's Guild scenario, and M6 depends on delivery rather
than on tasks. **Four slices built, one deferred with a backlog row** — see
[task.md](task.md)'s slice section and §3 below.

### 0.2 (Correctness, blocking on closeout) The pickup trigger names a traceability-matrix row that does not exist

M05A slice A6's trigger, in both
[M05A task.md](../M05A-app-supervisor/task.md) and
[deferred-backlog.md](../../deferred-backlog.md) §8, is "M5 item 1 (Outbox, DLQ,
cron leases) marked **Complete in the traceability matrix**".

There is no `[PLT-ASY]` row in
[traceability-matrix.md](../../traceability-matrix.md). Searching it for
`PLT-ASY` returns nothing; the nearest thing is `[PRD-OFF]`, targeted M5 and
Pending, which is a product-level outcome, not this item.

So the trigger as written can never fire. The row must be created — as
**Pending**, now — so that marking it Complete at closeout is a state change
rather than an invention.

### 0.3 (Stale) The architecture doc puts the outbox in the client, and the client in this tree has no storage at all

The architecture doc's `[PLT-ASY]` says "A client uses an outbox queue and sends
a fire-and-forget message, marking the operation as optimistically successful in
its local UI", and the requirements spec says offline requests "are durably
stored in an outbox queue".

`crates/sdk`'s dependency list has no storage crate — no `rusqlite`, no
`syneroym-data-db`. `SyneroymClient` is an RPC client. The "client" the
architecture doc describes is the Hub UI shell (M6), which does not exist yet.

For M6's chat the outbox must live **on the substrate hosting the calling
service**, in that service's own encrypted database. Recorded in ADR-0023 §4;
the architecture doc gets a dated implementation-status note in the same shape
as the Universal Proxy one already at §PLT-DAT.

### 0.4 (Scope-changing) The specified cron lease has nothing to arbitrate in this topology

The spec's design is a cluster of symmetric worker nodes racing for "an
execution lease from the Registry", the winner selecting a target node and
holding the lease until the run completes. Three things in this tree make that
neither buildable as written nor necessary:

1. **The app registry cannot back a lease.** `AppRegistry`/`StaticInventory` is
   an `Arc<RwLock<BTreeMap>>`
   ([resolver.rs:314](../../../../crates/app_orchestration/src/resolver.rs#L314))
   — in-process, per-substrate, no persistence, no network surface. It cannot
   arbitrate between two nodes at all.
2. **The community registry has nothing for two principals to contend over.**
   Records are authorized by their own signature and keyed under the identity
   that signed them
   ([registry.rs:316](../../../../crates/community_registry/src/registry.rs#L316)),
   so writers are partitioned by key ownership. It does reject an older or
   equally-recent conflicting anchor with `409`, which is a timestamp fence on
   that one resource — not a general compare-and-set, and not needed as one.
3. **Inside an app instance a single writer already exists.** The supervisor
   holds the instance at an operator-minted generation and stands down on
   finding a higher one (ADR-0021 §4).

So the lease reduces to **target selection** (which member of a `replicas > 1`
service runs this tick — a decision by an existing single writer) and **overlap
prevention** (one row in the supervisor's store). Neither is a distributed lock.

The one case that *would* need one — two live supervisors sharing one imported
master — is already recorded as unreachable by construction, in
`refresh_master_anchor`'s own doc comment
([dht_registry.rs:658](../../../../crates/core/src/dht_registry.rs#L658)) and in
two `deferred-backlog.md` §8 rows.

**Consequence: this milestone does not touch the community registry, and does
not close those backlog rows.** They stay open, and §3 adds one sentence to each
saying *why* this milestone passed over them — otherwise the next reader
reasonably assumes they were forgotten. ADR-0023 §6 carries the decision and its
honest cost (a substrate partitioned from its supervisor runs no scheduled
work).

### 0.5 (Scope-changing — and it is why B5 is deferred rather than built) A long-running task is not merely unbuilt — it is structurally prevented, and the number is 5 seconds

`dispatch_epoch_timeout_secs` defaults to **5** and its own doc comment calls it
"tight by design: this is the hot path a stuck or hostile guest would otherwise
hang forever"
([config.rs:446](../../../../crates/core/src/config.rs#L446)). It bounds every
ordinary guest entry point — proxy invocation, message delivery, one streaming
chunk. The one relief valve is `lifecycle_hook_epoch_timeout_secs` (30s), scoped
to `init`/`migrate`.

So "long-running tasks composed of multiple compute and service calls" have no
way to run at all. B5's real work is a **third invocation path** with its own
budget and its own concurrency accounting, plus a durable intent record. The
restart rules the item names are the small part on top — and they are the part
the spec describes, which is why the item reads as if the rules were the work.

Naming the number matters: a reader who assumes the budget is generous will
scope B5 as "add a flag". The backlog row this deferral creates carries the
number for exactly that reason.

### 0.6 (Sets B4's boundary) The saga convention conflicts with WIT's own naming, and it needs a deploy-time check to be a convention at all

The spec writes `undo_<operation>()`. WIT identifiers are kebab-case, so the
real convention is `undo-<operation>` — a prose artifact, but one that would
otherwise be copied into a WIT file and fail to parse.

More substantively: a convention nothing checks is a comment. A service
participating in a saga declares it, and the deploy path verifies the `undo-`
counterpart exists on the registered interface — the same class of check
`test_wit_adherence` already performs over a parsed interface's function list.
Cheap, and it converts "the developer should" into "the deploy refuses".

### 0.7 (Sets B3's boundary) Nothing in a manifest can express a schedule

Neither `SynAppManifest` nor `ServiceSpec`
([models.rs](../../../../crates/app_orchestration/src/models.rs)) has a cron,
interval, or schedule field, and no other type in the tree does. B3 needs a
manifest surface before it needs a clock — the same order A5e's `replicas`
followed.

It also needs to say *what* is invoked: a schedule names an interface and a
method on the service, which is the `(interface, method, params)` shape the
proxy already uses. No new dispatch mechanism, one new declaration.

---

## §1 — Cross-cutting decisions

Decisions that bind every slice. Slice-local decisions are `D-B1-N`, `D-B2-N`, …
in the slice's own file.

| ID | Decision |
|---|---|
| **D-B-1** | **Five mechanisms, four slices built (B1–B4)** (§0.1), each owing its own `§0` findings pass **and its own file** before execution — the rule D-A5-2 established, plus the file rule §2 adds. |
| **D-B-2** | **B5 (long-running tasks) is deferred out of this milestone**, target M5's final phase, with a backlog row carrying the 5-second finding (§0.5). Safe because the B4→B5 dependency runs one way only: B5's failure branch is defined in terms of compensations, not the reverse (ADR-0023 §7). B4 is unaffected and keeps its own near-term consumer. |
| **D-B-3** | **Delivery is at-least-once, never exactly-once** (ADR-0023 §1). Every queued action must be safe to apply twice, and each slice states what makes that true for its own actions rather than assuming it. |
| **D-B-4** | **One crate, `syneroym-async-queue` at `crates/async_queue/`**, with three independent owners: the supervisor's outbox in `supervisor.db`, the substrate's proxy DLQ, and a service's own outbox in its DEK-encrypted database (ADR-0023 §4). No shared queue, nothing replicated, nothing on the wire as a queue item. This is also what keeps failure-matrix row 13 (encryption at rest) true with no new mechanism. |
| **D-B-5** | **No second retry *policy*.** The queue reuses `RetryPolicy`'s struct and `calculate_jittered_backoff` from `syneroym-core` ([ADR-0003](../../../decisions/0003-retry-policy-ownership.md)) — the constraint ADR-0021 §5 gave as the reason the trait boundary exists at all. It does **not** reuse `retry_with_backoff`: that is an in-process loop that sleeps between attempts, where a durable queue computes a `next_attempt_at` timestamp and forgets the item until a worker tick finds it due. *(Mechanism claim corrected 2026-08-04 after review; the intent is unchanged. [B1 plan](slice-b1-implementation-plan.md) §0.12.)* Each queue owner sizes its own budget — `RetryPolicy`'s defaults (3 attempts from 100 ms) are for a socket retry, not for waiting out an offline substrate. |
| **D-B-6** | **No community-registry change and no lease resource** (§0.4, ADR-0023 §6). Scheduling is target selection plus local overlap prevention. The two compare-and-set backlog rows stay open, each gaining one sentence explaining why this milestone passed over them. |
| **D-B-7** | **A `[PLT-ASY]` traceability row is created now, as Pending** (§0.2), so A6's pickup trigger can fire as written at closeout rather than requiring the row to be invented at the same moment it is marked Complete. |
| **D-B-8** | **B2's guest surface is a separate `enqueue` function, not an overloaded `call`.** The spec itself says offline-capable calls cannot synchronously return data; encoding that in `call`'s existing `result<string, proxy-error>` would need a sentinel string, which rots. Additive `call-options` fields plus one new function; no existing signature changes. |
| **D-B-9** | **A queued guest call with no idempotency key is refused at the call boundary.** This is `[PRD-OFF]`'s "unsafe retries fail explicitly" clause, and the requirements spec already states the rule — nothing implemented it because nothing could. |

---

## §2 — Slice sequence, merge order, and file discipline

**Step zero, before any code: accept ADR-0023.** Not a formality. Its §2
(try-then-queue) determines what the durable implementation *is*, and its §1
determines whether an idempotency-key mechanism is in B1's scope — a difference
of roughly a slice. The precedent is ADR-0020/ADR-0021, both accepted before
M05A's slice plans were written.

**Sequence.** B1 first; it is the M6 gate and the only slice with a consumer
waiting. B2, B3, and B4 may then run in parallel.

```
ADR-0023 accepted → B1 ─┬→ B2
                        ├→ B3
                        └→ B4          (B5 deferred, D-B-2)
```

**Then M05C.** Decided with the requester (2026-08-04): this milestone runs
first, then [M05C](../M05C-logical-discovery-overlay/task.md) (the logical
discovery overlay). The two are design-independent but edit the same four files,
so a sequence removes the rebasing entirely rather than managing it —
[M05C's plan](../M05C-logical-discovery-overlay/implementation-plan.md) §2 has
the collision table. It also matches the priority the meta plan already states:
M05B unblocks A6 and M6, where the overlay unblocks nothing yet.

**Slice plans:**

| Slice | Plan | State |
|---|---|---|
| B1 | [slice-b1-implementation-plan.md](slice-b1-implementation-plan.md) | 📋 Planned, full detail |
| B2 | *(not written)* | Sketch in [task.md](task.md); owes its own `§0` |
| B3 | [slice-b3-implementation-plan.md](slice-b3-implementation-plan.md) | 📋 Planned, full detail (2026-08-06). Not started |
| B4 | *(not written)* | Sketch in [task.md](task.md); owes its own `§0` |
| ~~B5~~ | — | **Deferred** (D-B-2); backlog row, target M5 final phase |

### File discipline — what this milestone is deliberately not repeating

M05A's slice plans, by line count:

| Plan | Lines | Slices covered |
|---|---|---|
| `slice-a7` | 990 | 1 |
| `slice-a0` | 1,032 | 1 |
| `slice-a2` | 1,388 | 1 |
| `slice-a3` | 1,984 | 1 |
| `slice-a1` | 2,121 | 1 |
| `slice-a4` | 2,143 | 1 |
| **`slice-a5`** | **6,531** | **5, plus four review passes and a code review** |

A5 is three times the next largest, and the cause is visible in its own table of
contents: Parts I–VI, one per sub-slice, appended into the file the original
single-slice plan started as.

**To be clear about what this is and is not saying.** A5 shipped — all five
sub-slices Complete, the milestone closed 2026-08-04, and the plan's own
successive `§0` passes are why the difficult findings surfaced before the code
did. The plan worked. It just got **large**, and a large plan is harder to
navigate and to keep internally consistent: A5's §14 is corrected in five places
by its Part IV, and the document has to tell the reader so. That cost is worth
avoiding where it is avoidable, and it is avoidable by where files are cut.

The size came **not** from "a milestone had five slices" — M05A had twelve and
is fine — but from one file carrying five. So:

- **One slice, one file**, in the 1,000–2,000-line band every other M05A plan
  sits in. A slice whose plan is heading past that is a slice that needs
  splitting, and the line count is the early warning.
- **A slice's review passes append to that slice's file, never to this one.**
- **This file stays milestone level** and does not grow when a slice does. If a
  review finding is genuinely cross-cutting it lands in §0/§1 here *and* is
  cited from the slice, rather than being duplicated.

---

## §3 — Docs and backlog impact

**Created by this plan (documentation, no code):**

- [ADR-0023](../../../decisions/0023-durable-async-primitives.md) — **Accepted 2026-08-04**.
- `[PLT-ASY]` row in [traceability-matrix.md](../../traceability-matrix.md),
  status **Pending**, target M5B (D-B-7, §0.2).
- A pointer from `meta-implementation-plan.md`'s M5 item 1 to this milestone, in
  the shape item 2 already uses to point at M05A, plus the withdrawal of its
  "add the single-writer cron lease" clause (§0.4).
- A pointer from [M05A task.md](../M05A-app-supervisor/task.md)'s A6 section to
  this plan. **Nothing else in M05A's docs changes** — its exit criteria
  deliberately exclude A6 and must not be reopened.

**Backlog rows this milestone resolves** (move to *Recently resolved* at
closeout, not before):

| Row | How |
|---|---|
| §8 *App Supervisor: durable push delivery (outbox/DLQ)* | B1. This is A6, and it is the milestone's central deliverable. **Moves to *Recently resolved* when B1 lands, not at milestone closeout** — B1 discharges A6's whole scope ([B1 plan](slice-b1-implementation-plan.md) §4). The "at closeout, not before" rule below governs the other rows in this table |

**Backlog rows this milestone deliberately does not resolve, each needing one
added sentence saying so** — otherwise the next reader assumes they were
overlooked:

| Row | Why it stays open |
|---|---|
| §8 *App Supervisor: single-writer lease / supervisor HA* | D-B-6/§0.4: there is no lease here to give it. HA needs replicated supervisor state (M7), and the generation stamp already stops two supervisors flapping. Retargeted "M05 / M07" → **M07** |
| §8 *A redundant supervisor holding one master ... has no compare-and-set* | Unreachable by construction, not blocked on a primitive. This milestone builds no CAS (ADR-0023 §6) |
| §8 *Master-anchor refresh is a read-modify-write with a race ...* | Same. Its schedule half is already served by the resident loop's tick |
| §8 *App Supervisor: shutdown latency is unbounded ...* | Unchanged, and B1 must not make it worse — its D-B1-8 exists for that reason |

**New backlog rows this milestone's own choices create:**

- **Long-running tasks and their restart rules (deferred B5).** Added **now**,
  not at closeout, because the deferral is decided now and an undeferred-looking
  slice list is how work gets forgotten. Carries §0.5's finding — the machinery
  is a third invocation path, not a flag, because
  `dispatch_epoch_timeout_secs` bounds every existing guest entry point at 5
  seconds. Target: M5's final phase, with items 2–4.
- **Durable delivery for certificate-bearing actions.** B1 makes only
  `write_bindings` queueable; `apply_plan` and `renew_cert` embed an instance
  certificate that expires in 4 hours by default, so a late delivery installs a
  dead certificate and looks like success. Making them durable needs the queue
  to store *intent* rather than the payload and re-mint at delivery time — which
  needs the supervisor's vault open at delivery time, and it is locked after
  every restart. A different design, not more of the same one
  ([B1 plan](slice-b1-implementation-plan.md) §0.11, D-B1-12).
- **No unused-dependency check in the build pipeline.** D-B1-10 removes six dead
  entries; nothing stops a seventh.
- **The guest outbox has no client-side half.** The architecture doc's UI-shell
  outbox (§0.3) is M6's, not this milestone's.
- **A tick missed while the supervisor was down.** B3's `§0` settles run-late
  versus skip; whichever it chooses, the other is a row.
- **No cross-node queue.** Deliberate (D-B-4). Belongs with `[PLT-RED]` (M7) if
  it ever returns.
- **`replay` is all-or-nothing per item.** No partial or bulk replay surface in
  B1.

**Doc amendments at closeout** (all are exit criteria):

- `system-architecture.md` `[PLT-ASY]` — a dated implementation-status note in
  the Universal Proxy note's shape, covering the client-side outbox placement
  (§0.3) and the lease-based scheduler (§0.4), both superseded by ADR-0023.
- `system-requirements-spec.md` `[PLT-ASY]` — the same two points.
- `developer-guide.md` — an operator section for the DLQ verbs, and the
  partitioned-substrate consequence of ADR-0023 §6 stated plainly, in the shape
  the `pause`/`resume` consequence is already stated.

---

## §4 — What closing this closes

Against `meta-implementation-plan.md` item 1's own text — "Implement the Outbox
queue, cron lease mechanisms, Dead Letter Queue (DLQ), long-running task restart
rules, and compensating transactions (sagas)":

| Item-text phrase | Closed by | Note |
|---|---|---|
| Outbox queue | B1 (supervisor), B2 (guest) | |
| Dead Letter Queue | B1 (surface + supervisor), B2 (proxy) | |
| cron lease mechanisms | B3 | **Delivered differently from the specification.** Target selection plus overlap prevention, no distributed lease — ADR-0023 §6 records why and what it costs |
| compensating transactions (sagas) | B4 | Convention plus helper, not an engine — the requirements spec's own rationale |
| long-running task restart rules | **Nothing — deferred** | D-B-2. The invocation path that has to exist first (§0.5) is real new machinery with no consumer. Backlog row, target M5 final phase. **So item 1 does not close whole**, and the `[PLT-ASY]` matrix row says which four of five mechanisms it covers |

And beyond the item text:

- **M05A slice A6** closes, the only slice in that milestone gated on an
  external trigger.
- **ADR-0021 §5's "after M5"** clause is discharged, and its 2026-08-03
  amendment's second convergence clause — an unreachable dependent converging
  after it returns — becomes measurable for the first time.
- **M6 is unblocked.** Per the 2026-07-16 resequencing, the async primitives are
  M6's only dependency out of M5.
- **`[PRD-OFF]`** gets both of its clauses: convergence from B1/B2's delivery,
  explicit failure from D-B-9's refusal of an unkeyed queued call.

What it does **not** close: M5 items 2, 3, and 4, all still deferred to the
final phase; the Logical Service Discovery Overlay's S1–S4, which are a separate
stream (see [task.md](task.md)'s dependency gates); supervisor HA; and anything
in `[PLT-RED]`.

---

## §5 — Questions for the requester

1. ~~**B3's missed tick.**~~ **Answered 2026-08-06: skipped, not run late.**
   Implemented as a watermark plus a grace window of `2 *
   poll_interval_secs`, not as a next-due timestamp — the naive
   `cron.after(last_run)` form is silently run-late
   ([B3 plan](slice-b3-implementation-plan.md) §0.5, D-B3-6). Run-late gets a
   backlog row.
2. **B2's dedup TTL.** How long a receiver remembers an idempotency key bounds
   how long a queued call stays safe to deliver. It should probably match or
   exceed the outbox's own total retry window, which makes it a derived number
   rather than a configured one — worth confirming before B2 treats it as
   configurable.
3. ~~**B5's necessity in this milestone.**~~ **Answered 2026-08-04: deferred**
   to M5's final phase alongside items 2–4, with a backlog row (D-B-2, §0.5).
   Nothing downstream waits on it, and the one dependency in the area — B4
   before B5 — points the other way, so B4 is unaffected.
