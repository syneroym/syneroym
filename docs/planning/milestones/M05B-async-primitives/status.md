# M05B Async Primitives — Status

**Milestone:** [task.md](task.md) · **Design of record:**
[ADR-0023](../../../decisions/0023-durable-async-primitives.md) · **Plan:**
[implementation-plan.md](implementation-plan.md)

**Overall:** 📋 **Planned (2026-08-04). ADR-0023 accepted 2026-08-04.** B1 is
starting — the plan's §2 explains why acceptance was not a formality (its §2,
try-then-queue, determines what the durable implementation is).

**Plan layout.** [implementation-plan.md](implementation-plan.md) is milestone
level: the split call, cross-cutting findings and decisions, slice sequence,
docs/backlog impact. **Each slice's own findings, decisions, phases, and tests
live in that slice's file** —
[slice-b1-implementation-plan.md](slice-b1-implementation-plan.md) is the first,
and B2–B4 get theirs when picked up. The milestone plan's §2 records why: M05A's
`slice-a5` plan reached 6,531 lines by carrying five sub-slices in one file,
where every single-slice plan in that milestone sits between 990 and 2,143.

## Slice status

| Slice | Scope | Status | Gate |
|---|---|---|---|
| B1 | Queue crate, supervisor delivery outbox, DLQ with alert and operator surface. **Closes [M05A slice A6](../M05A-app-supervisor/task.md)** | 📋 Planned — [plan](implementation-plan.md) §2 | ADR-0023 accepted |
| B2 | Guest outbox and proxy DLQ: idempotency key, `enqueue`, receiver-side dedup | 📋 Planned (sketch only; owes its own `§0`) | B1 |
| B3 | Scheduled tasks: manifest surface, evaluation on the supervisor's pass tick, member selection, overlap prevention | 📋 Planned (sketch only; owes its own `§0`) | B1 |
| B4 | Saga compensations: the `undo-<operation>` convention, deploy-time check, step log, reverse walk | 📋 Planned (sketch only; owes its own `§0`) | B1 |
| ~~B5~~ | ~~Long-running tasks~~ | **Deferred out of this milestone (2026-08-04)** — [deferred-backlog.md](../../deferred-backlog.md) §8, target M5 final phase | — |

**B5 deferred, decided with the requester before any code was written.** It is
the only one of item 1's five mechanisms with no consumer and no near-term
consumer, and the one needing real new machinery: `dispatch_epoch_timeout_secs`
bounds every guest entry point at 5 seconds, so a long-running task needs a
third invocation path, not a flag ([plan](implementation-plan.md) §0.5, D-B-2).
Safe because the dependency in that area ran one way only — B5's failure branch
is defined in terms of B4's compensations, not the reverse — so **B4 is
unaffected** and keeps a near-term consumer B5 never had, in M6's Guild
scenario.

**Consequence for closeout:** M5 item 1 does not close whole, and the
`[PLT-ASY]` matrix row is scoped to the four mechanisms built here. M05A slice
A6's pickup trigger still fires soundly — A6 consumes the delivery half (B1),
which is fully built.

**Sequencing:** this milestone runs first, then
[M05C](../M05C-logical-discovery-overlay/task.md). Decided 2026-08-04.

**Three findings from planning changed scope before any code was written:**

- **The supervisor needs no idempotency key**
  ([B1 plan](slice-b1-implementation-plan.md) §0.1). Every action on
  `SubstrateActor` is already fenced by a mechanism M05A built for another
  reason — content-hash dedup, the binding epoch, in-place certificate install.
  At-least-once delivery inherits its correctness argument instead of making
  one, which is most of why B1 is one slice.
- **A6's own scope note is false under the obvious implementation**
  ([B1 plan](slice-b1-implementation-plan.md) §0.2).
  "Nothing above the trait changes" holds only if the durable actor attempts
  synchronously first and enqueues on failure; a queue-always design has no
  `BindingWriteOutcome` to return and would change every caller.
- **The specified cron lease has nothing to arbitrate here**
  ([milestone plan](implementation-plan.md) §0.4). Registry
  writes are partitioned by key ownership and the supervisor is already the
  single writer per app instance, so the lease reduces to target selection plus
  a local overlap guard. No community-registry change, and the two
  compare-and-set backlog rows stay open rather than being silently absorbed.

**One documentation defect blocks closeout and was fixed in the planning pass**
(§0.2): M05A slice A6's pickup trigger reads "M5 item 1 marked Complete in the
traceability matrix", and no `[PLT-ASY]` row exists in that matrix. The row is
created as **Pending** now so that marking it Complete at closeout is a state
change rather than an invention.

## Evidence

_(Empty. Each slice's verification evidence lands here as it completes, in the
shape [M05A status.md](../M05A-app-supervisor/status.md) uses: what was
delivered, the named tests that prove it, and anything delivered differently
from the plan with the reason.)_
