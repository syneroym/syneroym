# Milestone 5B: Async Primitives (M05B-async-primitives)

> **Provenance.** Milestone 5's **item 1** ("Async Primitives"), front-loaded
> ahead of M6 by the *M5–M7 Resequencing* amendment (2026-07-16) in
> [meta-implementation-plan.md](../../meta-implementation-plan.md). M5 itself has
> no milestone directory: its item 2 was split out as
> [M05A](../M05A-app-supervisor/task.md) (2026-07-27) and its items 2–4 are
> deferred to the final phase. This is therefore M5's second and last
> front-loaded half, and the naming follows M05A's.
>
> **Design of record:**
> [ADR-0023](../../../decisions/0023-durable-async-primitives.md), which
> discharges the "after M5" half of
> [ADR-0021](../../../decisions/0021-binding-propagation-and-app-supervisor.md)
> §5 and amends the `[PLT-ASY]` text in the requirements spec and the
> architecture doc in three places.
>
> **What this milestone is.** Today every retry in the tree is in-process and
> dies with the process. A binding push to a substrate that is offline is lost;
> a proxy call that exhausts its retries fails and is forgotten; nothing can be
> scheduled; a guest call cannot outlive its 5-second dispatch budget. This
> milestone adds durable, owner-local work queues, a Dead Letter Queue with an
> operator surface, scheduled tasks, a long-running invocation path with restart
> rules, and saga compensations.
>
> **What it is deliberately not.** Not exactly-once delivery. Not a distributed
> lock, and not a compare-and-set in the community registry (ADR-0023 §6). Not a
> replicated or cross-node queue — that is `[PLT-RED]` (M7). Not durable
> execution of WASM memory; the requirements spec rejects that itself. Not a
> workflow engine.

## Goal

By the end of M05B, work that was due while its target was unreachable still
happens after the target returns — across a process restart — and work that can
never happen is visible to an operator instead of being silently dropped. The
first proof is the App Supervisor: a binding push to an offline substrate
converges without an operator touching anything, closing
[M05A slice A6](../M05A-app-supervisor/task.md).

---

## Requirement IDs (Traceability)

| Requirement ID | Sub-scope in M05B | Current matrix status |
|---|---|---|
| `[PLT-ASY]` Asynchronous Operations & Scheduling | Outbox, DLQ, scheduled tasks, long-running-task restart rules, saga compensations | **No row exists.** One must be created — see the note below |
| `[PRD-OFF]` | "Safe workflows remain intelligible and converge after disconnection; unsafe retries fail explicitly" | Existing row, M5, **Pending**. The second clause is the idempotency-key refusal (ADR-0023 §4) |
| `[LFC-MGT]` (App Supervisor) | Slice A6's durable delivery only. Nothing else about the supervisor changes | Existing row, M5A, **Complete** — A6 is explicitly outside its exit criteria and must not reopen it |

> **`[PLT-ASY]` has no traceability-matrix row at all**, and M05A slice A6's
> pickup trigger is written as "M5 item 1 marked **Complete in the traceability
> matrix**". As written, that trigger can never fire. Creating the row is the
> first documentation task of this milestone, not a closeout chore.

---

## Explicit non-goals

- **No exactly-once delivery.** At-least-once, with the caller's existing fences
  supplying dedup (ADR-0023 §1). A guest caller with no such fence supplies an
  idempotency key or is refused.
- **No second retry *policy*.** `RetryPolicy`'s struct and
  `calculate_jittered_backoff`
  ([ADR-0003](../../../decisions/0003-retry-policy-ownership.md)) are reused;
  the queue adds *durability* and *terminal handling* rather than a competing
  policy. This is the constraint ADR-0021 §5 gave as the reason the trait
  boundary exists at all. *(Corrected 2026-08-04 after review: `retry_with_backoff`
  itself is **not** reused — it is an in-process loop that sleeps between
  attempts, where a durable queue computes a `next_attempt_at` and forgets the
  item until a worker tick finds it due. Only the jitter helper carries over.
  Each owner also sizes its own budget: 3 attempts from 100 ms is a socket
  retry, not a wait for an offline substrate.)*
- **No distributed lock, no registry compare-and-set, no lease resource.**
  ADR-0023 §6. Supervisor HA and the master-anchor CAS rows in
  [deferred-backlog.md](../../deferred-backlog.md) §8 stay open and are
  explicitly *not* closed here — they are unreachable-by-construction today, not
  blocked on a missing primitive.
- **No replicated or cross-node queue.** Each queue belongs to one process.
- **No durable execution / WASM memory snapshotting.** A crashed long-running
  task aborts and compensates; it never resumes mid-flight.
- **No workflow DSL or saga engine.** A naming convention (`undo-<operation>`),
  a durable step log, and a helper that walks it backwards.
- **No pull-side app directory.** ADR-0021 §6 is untouched by this milestone.
- **No change above the `SubstrateActor` trait.** Guaranteed structurally by
  try-then-queue (ADR-0023 §2), not by care.

---

## Dependency gates

| Gate | State |
|---|---|
| **M05A slices P0, A0–A5e, A7 Complete** | ✅ Complete 2026-08-04. B1's consumer is the shipped `SubstrateActor` and its shipped fences |
| **ADR-0023 accepted** | ⛔ **Proposed.** Blocks B1. The delivery-semantics decisions cannot be discovered during implementation — §2 alone changes what the durable implementation is |
| **`[PLT-ASY]` matrix row created** | ⛔ Not created. Blocks *closing* the milestone, not starting it |
| M04B FDAE | ✅ Complete. B2's guest surface is authorized through the existing proxy gate; no new authorization model |
| M7 replication | Not required, and must not become required — see non-goals |
| **M05C (discovery overlay)** | **Not a gate in either direction** — the two streams are design-independent, and the meta plan says so. They are **not file-independent**: both edit `app_supervisor/src/service.rs`, `store.rs`, `supervisor.wit`, and `app_orchestration/src/models.rs`. **B1 phase 1 lands before M05C slice S1 starts** — it is a pure refactor of the largest of those files, so it is one small merge either way round and everything after it is tidier. [M05C plan](../M05C-logical-discovery-overlay/implementation-plan.md) §2 carries the full table and the parallel-running convention |

**What this milestone gates:** M6 (the product/chat milestone) depends on this
and only this out of M5, per the 2026-07-16 resequencing. M05A slice A6's pickup
trigger fires on this milestone's completion.

---

## Slices

Five mechanisms. **Four slices are built here; the fifth is deferred.** The
reasoning for splitting rather than treating this as one slice is in
[implementation-plan.md](implementation-plan.md) §0.1 — briefly, they share a
theme and no mechanism, and M05A slice A5 reached the same conclusion about
itself in its own `§0.1` after starting as one slice.

| # | Scope | Consumer today | Gate |
|---|---|---|---|
| **B1** | The durable queue crate (`syneroym-async-queue`), the supervisor's delivery outbox, the DLQ with its alert and operator surface. **Closes M05A slice A6.** | Real — the App Supervisor | ADR-0023 accepted |
| **B2** | Guest-facing outbox and proxy DLQ: `call-options` gains `idempotency-key` and a separate `enqueue` function; receiver-side dedup; failed-after-retries proxy calls land in the DLQ | Real — M6 chat | B1 |
| **B3** | Scheduled tasks: a manifest schedule surface, evaluation on the supervisor's existing pass tick, member selection, overlap prevention | None yet | B1 |
| **B4** | Saga compensations: the `undo-<operation>` convention, deploy-time checking, a durable step log, reverse walk on terminal failure | Near-term — M6's Professional Services Guild, whose cross-provider workflow the architecture doc describes as exactly this | B1 |
| ~~**B5**~~ | ~~Long-running tasks~~ | — | **Deferred out of this milestone, 2026-08-04** |

### B5 is deferred, and why that is the right cut

Long-running tasks are the only mechanism here with **no consumer, no near-term
consumer, and genuinely new machinery**: a third guest invocation path with its
own epoch budget and concurrency accounting, because
`dispatch_epoch_timeout_secs` bounds every existing entry point at 5 seconds
([implementation-plan.md](implementation-plan.md) §0.5). Nothing downstream
waits on it — M6 depends on delivery, not on tasks.

Tracked in [deferred-backlog.md](../../deferred-backlog.md) §8 with a pickup
trigger, per the *Mandatory Deferred-Backlog Update* rule. **Target: M5's final
phase**, alongside items 2–4, which is where the rest of the deferred M5 work
already sits.

**B4 survives the cut, and it is worth saying why**, since B5's original gate
was B4. The dependency ran one way only — B5's failure branch is defined in
terms of compensations, not the reverse — so removing B5 leaves B4 intact. B4
also keeps a real near-term consumer that B5 never had: the architecture doc's
Guild scenario describes distributed sagas across independent providers, with
the consumer's own node as orchestrator, and that is M6 work.

Each of B2–B4 owes its own `§0` findings pass before execution, the rule
M05A's D-A5-2 established after A5a–A5e.

---

## Migration impact

- **New tables in existing databases.** The supervisor's outbox and DLQ tables
  are created in `supervisor.db` with `CREATE TABLE IF NOT EXISTS`, alongside
  `desired_state`, `deployments`, and `alerts`. No `ALTER TABLE` on an existing
  table, so no idempotent-add-column dance is needed — unlike A7's D-A7-2.
- **A WIT change guests must recompile against.** B2 adds two fields to
  `call-options` and one function to the `proxy` interface. `call-options` is
  `option<call-options>` at every call site and every field is additive, so a
  guest that does not recompile keeps working; one that wants the new behavior
  recompiles. No version ladder, per pre-release policy.
- **A manifest change.** B3 adds a schedule surface to `ServiceSpec`. Absent
  means unscheduled, which is every manifest that exists today.
- **Five new `SupervisorRole` config fields**, for B1's queue: attempt budget,
  total window, worker tick, visibility timeout, and DLQ cap. *(Corrected
  2026-08-04 after review — this line previously read "no new config field",
  which contradicted the reference scenario: `RetryPolicy`'s defaults are 3
  attempts from 100 ms, so an item would dead-letter in under a second where the
  scenario needs it to survive a supervisor restart plus an operator action. See
  the [B1 plan](slice-b1-implementation-plan.md) §0.12.)* `AppSandboxRole` is
  untouched — B5's `task_epoch_timeout_secs` went with the deferral.
- **Six unused dependencies removed** (B1 phase 1, see the slice plan's §0.10),
  **plus the two `[workspace.dependencies]` entries they orphan**
  (`metrics-util`, `chromiumoxide` — each has exactly one consumer;
  `assert_cmd` and `sysinfo` have others and stay). Manifest-only, no source
  edits; `syneroym-router` loses `quinn`, the largest of them.
- **Nothing is dropped and nothing is renamed.** This milestone adds
  mechanisms; it changes no existing schema, no existing wire field, and no
  existing behavior on the reachable path.

---

## Reference scenario (runnable)

Two real substrates and a real supervisor, in the shape M05A's
`binding_push_e2e.rs` and `multi_substrate_placement_e2e.rs` already establish.
The scenario is written against B1 and is the milestone's central proof:

1. Deploy an app instance whose dependent service lives on substrate **B** and
   whose dependency lives on substrate **A**. Adopt it. Confirm converged.
2. **Stop substrate B.**
3. Change membership on A (scale out), so a binding push to B becomes due.
4. The supervisor's synchronous attempt fails. Assert: the item is **in the
   outbox**, the instance reports `Degraded`, and the failure is *not* retried
   forever in-process.
5. **Restart the supervisor process.** This is the step no in-process retry can
   survive, and the reason the queue exists. Assert the item is still queued.
6. **Bring B back.**
7. Assert: within one worker tick, B holds the new binding, `supervisor status`
   reports converged, and the outbox is empty.
8. Take B down permanently and exhaust the attempt budget. Assert: the item is
   in the **DLQ**, an alert was raised, `roymctl supervisor dead-letters` lists
   it, and `replay` re-queues it rather than executing it inline.

Steps 5–7 are exactly ADR-0021's second convergence clause, which its
2026-08-03 amendment recorded as unmet and attributed to this milestone.

---

## Failure and security matrix

| # | Case | Required behavior |
|---|---|---|
| 1 | Supervisor crashes with items queued | Items survive; the worker resumes them on the next start. No item is lost and none is stuck `InFlight` forever — a claim has a visibility timeout |
| 2 | The same queued action is applied twice | No observable difference from applying it once, via the fences in ADR-0023 §1. Asserted per action, not assumed |
| 3 | A `restart` fails | Fails immediately; **never** queued (ADR-0023 §3). Remediation, not delivery, decides what happens next |
| 3a | An `apply_plan` or `renew_cert` fails | Fails immediately; **never** queued. Both embed an instance certificate, and `renewed_cert_expires_hours` defaults to 4 — a late delivery installs a dead certificate and looks like success ([B1 plan](slice-b1-implementation-plan.md) §0.11, D-B1-12) |
| 3b | The worker and the resident loop both have work for one instance | Serialized by the existing `instance_lock`. Without it a supervisor can race itself into a `BindingConflict` alert indistinguishable from real split-brain ([B1 plan](slice-b1-implementation-plan.md) §0.13) |
| 3c | A queued write arrives after a newer epoch was already pushed | Returns `stale`, and the worker **completes** the item. This is convergence, not loss; dead-lettering it would report the normal case as failure ([B1 plan](slice-b1-implementation-plan.md) §0.14) |
| 4 | Attempts exhausted | Item moves to the DLQ, an alert is raised, and the item is listable. Never silently dropped. The alert is **one standing row per `(instance, logical_ref, substrate)` with a count**, not one per item — `AlertStore`'s unique index cannot express the latter, and the former is what an operator wants ([B1 plan](slice-b1-implementation-plan.md) §0.9) |
| 4a | The last dead letter for a key is replayed or pruned | Its alert clears. An alert nothing can clear trains operators to ignore the list — `RemediationExhausted` documents its clear path and this must too |
| 5 | A dead letter is replayed and fails again | Returns to the DLQ with attempt history intact. No inline retry loop |
| 6 | Two workers in one process claim the same item | Impossible: the claim is a conditional `UPDATE` inside one connection |
| 7 | A guest enqueues a call with no idempotency key | **Refused** at the call boundary. This is `[PRD-OFF]`'s "unsafe retries fail explicitly" clause |
| 8 | A guest replays a call with a used idempotency key inside the TTL | The first result is returned; the target is not re-executed |
| 9 | A queued item names a service that no longer exists | Terminal, not retryable. Straight to the DLQ with a distinguishable reason |
| 10 | A scheduled task's run outlives its interval | The next tick does not double-start it (ADR-0023 §6's overlap half) |
| 11 | A substrate is partitioned from its supervisor when a tick is due | No run happens, and this is documented, not worked around. The honest cost of §6 |
| 12 | Queue growth is unbounded | It is not: completed items are deleted, dead letters are bounded and prunable, and the bound is asserted |
| 13 | Queue contents at rest | The supervisor's queue lives in `supervisor.db` under the same protection as `desired_state`; a service's queue is in its DEK-encrypted database. A payload never sits in an unencrypted store the surrounding data would not |
| 14 | Replay authorization | `replay` is a lifecycle write and is gated exactly as the surrounding `supervisor` verbs are. No new resource namespace |

---

## Performance budgets

| Budget | Target | Why |
|---|---|---|
| **Happy-path delivery does not touch the queue** | Zero queue reads or writes on a successful synchronous call | The load-bearing budget. ADR-0021's amendment measured reachable convergence in microseconds; try-then-queue exists to keep it there (ADR-0023 §2) |
| Enqueue on failure | < 1 ms | One indexed SQLite insert |
| Idle worker tick | No measurable cost with an empty queue | One indexed query on `next_attempt_at`; the worker must not scan |
| Recovery after a target returns | Within one worker tick, **not** one supervisor poll interval | The queue is the fast path back, otherwise it buys nothing over the loop's own retry |
| Guest dedup lookup | Within the existing proxy call budget | B2 puts a read on the receiver's hot path; it must not widen the call's own budget |

---

## Measurable exit criteria

1. The reference scenario above passes end to end against two real substrates.
2. Every row of the failure/security matrix has a named test that asserts it.
3. Every performance budget above has a measurement — the happy-path one as an
   assertion that the queue is untouched, not as a timing.
4. `crates/sdk/src/deploy.rs`'s `SubstrateActor` has an outbox-backed
   implementation, and **no caller above the trait changed**. Demonstrated by
   the diff, not asserted in prose.
5. The `no DLQ (M5)` markers are gone from
   [proxy.rs:456](../../../../crates/router/src/proxy.rs#L456) and from
   `system-architecture.md`'s Universal Proxy implementation-status note, with
   the behavior they describe actually built.
6. `[PLT-ASY]` exists as a traceability-matrix row and is marked **Complete for
   the four mechanisms built here**, with long-running-task restart rules named
   in the row as deferred and carrying its backlog link. Marking a row Complete
   while part of it is deferred is only acceptable when the row says exactly
   what is and is not in it — the shape `[PLT-DAP]`'s "Complete (foundations
   only)" already uses. **M05A slice A6's pickup trigger fires on this**, and it
   is sound: A6 consumes the delivery half (B1), which is fully built.
7. M05A slice A6 is recorded Complete in
   [M05A status.md](../M05A-app-supervisor/status.md), and its
   [deferred-backlog.md](../../deferred-backlog.md) §8 row moves to *Recently
   resolved* — **when B1 lands, not at milestone closeout.** B1 discharges A6's
   whole scope; holding its status until B3's scheduling work lands would be
   bookkeeping rather than truth. The `[PLT-ASY]` row above is a *milestone* row
   and does close at closeout; these are different things closing at different
   times. *(Corrected 2026-08-04 after review — three documents disagreed. See
   the [B1 plan](slice-b1-implementation-plan.md) §4.)*
8. The deferred B5 has a backlog row with a reason, a target, and a pickup
   trigger, and the six unused dependencies are gone.
9. `cargo +nightly fmt --all` clean.
10. `cargo clippy --workspace --all-targets --all-features` clean.
11. `cargo test --workspace` passes.
12. `mise run test:e2e` passes.
13. `wasm32-wasip2` test components rebuild against the changed WIT (B2 onward).
