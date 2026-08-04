# ADR-0023: Durable Async Primitives — At-Least-Once Delivery, Owner-Local Queues, No Distributed Lock

**Status**: Proposed (2026-08-04). Design of record for
[Milestone 5 item 1](../planning/meta-implementation-plan.md#milestone-5-async-lifecycle-and-developer-experience)
(async primitives) and for the milestone that builds it,
[M05B-async-primitives](../planning/milestones/M05B-async-primitives/task.md).

Discharges the "after M5" half of
[ADR-0021](0021-binding-propagation-and-app-supervisor.md) §5, which named an
outbox/DLQ-backed implementation of the `SubstrateActor` trait but did not
specify its semantics. Amends the `[PLT-ASY]` design text in
[system-architecture.md](../system-architecture.md) and
[system-requirements-spec.md](../system-requirements-spec.md) in the three
places named in §5, §6, and §7 below.

**Context**:

Five mechanisms are specified together under `[PLT-ASY]` — an outbox queue, a
Dead Letter Queue, cron leases, long-running-task restart rules, and
compensating transactions (sagas). They are specified as one item because they
share a theme (work that outlives the call that asked for it), not because they
share a mechanism. Two of them have a real consumer today; three do not.

The consumer that exists is the App Supervisor. It ships binding propagation
with **best-effort synchronous** delivery behind a narrow "apply this action to
that substrate" trait (`SubstrateActor`,
[deploy.rs:44](../../crates/sdk/src/deploy.rs#L44)), deliberately, because
durable delivery needs what this ADR settles. ADR-0021 §5 states the honest cost
of that: push failure is *sticky* where pull self-heals, so a dependent that was
unreachable when its dependency changed holds a dead binding indefinitely, and
restarting it does not help. ADR-0021's 2026-08-03 amendment measured the two
convergence clauses separately and found the second one — an unreachable
dependent converging after it returns — implicated the missing outbox, not the
push model.

The second consumer is M6's chat SynApp, whose offline behavior the requirements
spec describes as outbox queuing and sync on reconnection
([system-requirements-spec.md](../system-requirements-spec.md) `[PLT-ASY]`
Substrate Feature Coverage Matrix).

What is built today, and what is not:

- **Retry exists, once, and is in-process only.** `retry_with_backoff` plus the
  `RetryPolicy` struct on `SubstrateConfig`
  ([retry.rs](../../crates/core/src/retry.rs),
  [ADR-0003](0003-retry-policy-ownership.md)) give exponential backoff with
  jitter. `ProxyRouter::invoke_remote` runs its own copy of the same loop
  inline. Neither survives a process restart, and the proxy's own comment says
  so: "Failed-after-retries fails directly -- no DLQ (M5)"
  ([proxy.rs:456](../../crates/router/src/proxy.rs#L456)).
- **Nothing durable queues work.** `DeploymentJournal`'s `ActionState`
  deliberately *removed* its `Pending` variant, because "nothing enqueues work
  ahead of applying it" ([journal.rs:76](../../crates/app_orchestration/src/journal.rs#L76)).
- **No idempotency key exists anywhere in the tree.** `call-options` carries
  `idempotent: bool` and nothing else
  ([proxy.wit](../../crates/wit_interfaces/wit/proxy/proxy.wit)); no receiver
  stores or compares one.
- **No schedule can be expressed.** Neither `SynAppManifest` nor `ServiceSpec`
  has a cron or interval field.
- **No long-running invocation path exists.** Every guest entry point is bounded
  by `dispatch_epoch_timeout_secs`, default **5 seconds**, described in its own
  doc comment as "tight by design"
  ([config.rs:446](../../crates/core/src/config.rs#L446)).

**Decision**:

## 1. Delivery is at-least-once, and correctness comes from fences the caller already holds

A durable queue that retries after a crash cannot also promise exactly-once
without a transaction spanning two machines. This project will not build one. So
every queued action must be safe to apply twice.

For the App Supervisor — the first consumer — **this property already holds, and
it was not built for this ADR**. Every action on `SubstrateActor` is already
fenced by a mechanism ADR-0021 or its slices installed for a different reason:

| Action | What makes a repeat safe |
|---|---|
| `apply_plan` | Content-hash dedup on `(manifest, app_context-minus-generation)` — an unchanged redeploy is a no-op |
| `write_bindings` | The epoch guard's four-case rule: equal epoch and equal content is an idempotent no-op ([ADR-0021](0021-binding-propagation-and-app-supervisor.md) §3) |
| `renew_cert` | Installs a certificate in place; installing the same one twice is the same state |
| `restart` | **Not idempotent** — §3 |
| `instance_identity`, `held_generation` | Reads |

This is the load-bearing observation of the whole milestone: **the supervisor
needs no idempotency key**, because the generation stamp, the binding epoch, and
the content hash are already exactly the dedup mechanism an at-least-once queue
requires. The queue inherits a correctness argument rather than making one.

This table answers idempotence only. Whether an action should be *deferred* at
all is a separate question, and §3 answers it differently — three of these four
writes are not queueable.

A guest-facing outbox has no such inheritance, which is why an idempotency key
is introduced there and only there (§4).

## 2. Try-then-queue, not queue-always — and this corrects M05A slice A6's own scope note

A6's scope note reads: "Replace the A5 trait's implementation with an
outbox/DLQ-backed one... **Nothing above the trait changes**"
([M05A task.md](../planning/milestones/M05A-app-supervisor/task.md) A6). That
sentence is true only under this decision, and false under the obvious reading
of the sentence before it.

`write_bindings` returns `Vec<BindingWriteOutcome>` — `applied`, `no-op`,
`stale(u64)`, `conflict(u64)`
([control-plane.wit:324](../../crates/wit_interfaces/wit/control-plane/control-plane.wit#L324))
— and the supervisor reads it to report convergence and to distinguish a
conflict from a stale write. An implementation that enqueues and returns
immediately has **no outcome to return**. It would have to invent one, and every
caller above the trait would have to learn that "accepted" is not "applied".
That is a change above the trait.

So the durable implementation **attempts the call synchronously first, exactly
as today, and enqueues only on transport failure**. A reachable substrate
produces a real outcome on the same code path it does now; an unreachable one
produces a queued item and a `Degraded` instance, which is what the supervisor
already reports. Nothing above the trait changes, in fact rather than in
aspiration.

This also protects the measured budget. ADR-0021's amendment recorded reachable
dependents converging in microseconds; a queue-always design would put a SQLite
write on that path for no benefit. **The happy path must not touch the queue at
all**, and that is a tested budget, not a preference.

## 3. Queueability is declared per action, and most actions are not queueable

Idempotence and queueability are different questions. §1 answers the first —
*may this be applied twice*. This section answers the second — *is it still the
right thing to do an hour later*. An action can be perfectly idempotent and
still be wrong to defer.

Two things make an action non-queueable, and between them they disqualify three
of the four writes on `SubstrateActor`:

**The intent expires.** A restart is remediation for a condition observed *now*.
Delivered an hour later it restarts a service that recovered on its own. The
supervisor's bounded remediation policy — attempts, backoff, terminal
`Degraded` — already decides what a failed restart means, and a queue behind it
would be a second policy disagreeing with the first. The trait already
anticipates this in `restart`'s own doc comment.

**The payload expires**, which is sharper and was missed on this ADR's first
draft. A `deployment-plan`'s service record carries
`instance-certificate: option<string>`, `renew-cert` takes one outright, and
`renewed_cert_expires_hours` defaults to **4**. A queued `apply_plan` or
`renew_cert` delivered after that window installs a certificate that is already
dead — and the substrate *accepts* it, because it is well-formed and correctly
signed, after which the service fails its handshake closed. A delivery that
fails is recoverable; a delivery that succeeds into a broken state is not
obviously even noticed.

**So `write_bindings` is the only queueable action.** That is not a retreat from
what this ADR is for — it is precisely what ADR-0021 §5 asks for. The binding
push is the sticky-failure case that ADR is built around, and `BindingWrite`
carries no certificate and nothing else time-limited: its content is as valid an
hour later as when it was queued.

Making the certificate-bearing actions durable is a **different design**, not
more of this one: the queue would store *intent* rather than the payload and
re-mint at delivery time, which requires the supervisor's vault to be open at
delivery time — and the vault is locked after every restart until an operator
injects the KEK. Recorded in the deferred backlog with that reasoning rather than
attempted here.

Recorded as a decision because the alternative (a queue wrapper that treats every
method the same) is the shape a generic wrapper naturally takes, and because
"durable delivery" without this section reads as a promise about all four.

## 4. Queues are local to their owner; there is no shared queue and no queue on the wire

One library crate, three independent owners, each supplying its own SQLite
connection — the same arrangement `DeploymentJournal`, `AlertStore`, and
`SupervisorStore` already share (`AlertStore` is "deliberately its own store
rather than more tables on `DeploymentJournal`",
[alerts.rs:3](../../crates/app_orchestration/src/alerts.rs#L3)):

- the **supervisor's** delivery outbox, in `supervisor.db`;
- the **substrate's** proxy DLQ, for calls that exhausted their retries;
- a **service's** own outbox, in that service's DEK-encrypted database, for the
  guest-facing fire-and-forget path.

No queue is replicated, none is reachable from another node, and no item crosses
the wire as an item — what crosses the wire is the original call, retried. A
queue is recovery state for one process, not a distributed log. This keeps the
milestone entirely clear of `[PLT-RED]` (M7).

**This corrects the architecture doc in one place.** It describes the outbox as
a *client* concern — "A client uses an outbox queue and sends a fire-and-forget
message" ([system-architecture.md](../system-architecture.md) `[PLT-ASY]`). The
client in this tree (`SyneroymClient`, `crates/sdk`) has no persistence and no
storage dependency at all. The outbox lives on the substrate hosting the caller,
not in the calling client.

The guest-facing path is where an **idempotency key** appears, since §1's
inherited fences do not extend to arbitrary guest calls: `call-options` gains
`idempotency-key: option<string>`, the receiver stores `(caller DID, key)` with
a TTL, and a repeat within the window returns the first result instead of
re-executing. A queued call that supplies no key is refused rather than
silently delivered twice — the requirements spec already states this rule
("Non-idempotent calls fail directly unless the caller supplied an idempotency
key and opted into queuing"); nothing implemented it because nothing could.

## 5. The DLQ is an operator surface, not a graveyard

The stated purpose of a DLQ here is "preventing silent data loss". A table
nothing reads converts silent loss into quiet loss. So terminal failure raises
an alert through the existing `AlertStore` path, and the dead letter is
listable and replayable by an operator through `roymctl`.

Replay re-enqueues; it does not re-execute inline. A dead letter that fails
again returns to the DLQ with its attempt history intact, so an operator
retrying a genuinely broken target learns that rather than looping.

## 6. Scheduling needs no distributed lease in this topology, and the specification's contrary text is superseded

The requirements spec and the architecture doc both specify cron as a
lease-based cluster scheduler: nodes race for "an execution lease from the
Registry", the winner selects a worker node and dispatches to it, and the lease
is held until the run finishes. That design assumes a cluster of symmetric
worker nodes contending for shared mutable state. Syneroym is not that.

Two facts about this tree remove the contention the lease exists to resolve:

1. **Registry state is partitioned by key ownership, so distinct principals
   never contend.** A record is authorized by its own signature and keyed under
   the identity that signed it
   ([registry.rs:316](../../crates/community_registry/src/registry.rs#L316)).
   Only the holder of a master key can write that master's records. There is no
   resource two different writers can both legitimately write, so there is
   nothing for a lease to arbitrate. (The registry does already reject an older
   or equally-recent conflicting anchor with `409` — a timestamp fence on that
   one resource, not a general compare-and-set, and not needed as one.)
2. **Within an app instance, a single writer already exists.** The supervisor
   holds the instance at an operator-minted generation, and a supervisor finding
   a higher generation stops managing the instance
   ([ADR-0021](0021-binding-propagation-and-app-supervisor.md) §4). The
   scheduler is therefore not elected; it is the component that already owns the
   instance.

So the lease **reduces to two much smaller things**, neither of which is a
distributed lock:

- **Target selection.** With `replicas > 1` a logical service has N members,
  possibly on different substrates, and exactly one should run a given tick.
  The supervisor picks one. This is a decision by an existing single writer, not
  a race.
- **Overlap prevention.** A run that outlives its interval must not be
  double-started. This is one row in the supervisor's own store.

The only case that would genuinely need a distributed lock is two live
supervisors sharing one imported master — a deployment shape this project does
not support, and which is already recorded as unreachable by construction
because mint-in-place means one `MasterVault` ever holds a given master and
`export-master`/`import-master` move a *file*, not concurrent live access
([deferred-backlog.md](../planning/deferred-backlog.md) §8; `refresh_master_anchor`'s
own doc comment at
[dht_registry.rs:658](../../crates/core/src/dht_registry.rs#L658)).

**Consequence, stated plainly.** The supervisor becomes the scheduler, so a
substrate partitioned away from its supervisor runs no scheduled work. That is
the honest cost, and it is the cost the platform already pays for
reconciliation. It is not fixed by an outbox: the queue makes a *dispatch* that
was due survive, but a supervisor that is itself down issues no dispatch. Real
scheduling availability needs replicated supervisor state (M7), which is where
supervisor HA is already tracked.

**Revisit trigger, measured rather than aesthetic** (the same discipline
ADR-0021 §6 applies to itself): if a deployment ever runs more than one
supervisor for one app instance, this decision is void and a shared lease is
required. Nothing else changes it.

## 7. Sagas are a convention plus a helper; long-running tasks need an invocation path before they need restart rules

The requirements spec already rejects durable execution — no event-sourced
memory snapshotting, tasks abort and compensate rather than resume. Two
consequences follow that the spec does not draw.

**Sagas.** What a saga needs from the platform is a durable log of completed
steps and a rule for walking it backwards. Both fit in the queue crate as a
helper. What it needs from a *service* is an `undo-<operation>` function on the
same interface — a naming convention the platform can check at deploy time, not
a workflow language it interprets. (The spec writes `undo_<operation>()` with
an underscore; WIT identifiers are kebab-case, so the convention is
`undo-<operation>` and the spec's spelling is a prose artifact.) No workflow
DSL, no engine, no coordinator service.

**Long-running tasks.** The specified restart rules ("restarts from the
beginning only when idempotent or explicitly restartable; otherwise it fails and
runs compensations") presuppose that a long-running task can be *started*. None
can: every guest entry point is bounded by `dispatch_epoch_timeout_secs`,
default 5 seconds. The new machinery is therefore an invocation path with its
own much larger budget plus a durable intent record — and the restart rules are
the small part on top. It also means **compensations must exist before the
restart rules are complete**, since the non-restartable branch is defined in
terms of them, which inverts the order these two appear in the item text.

**Consequences**:

- **Enables**: durable binding delivery for the App Supervisor, closing M05A
  slice A6 and the second convergence clause of ADR-0021's amendment; a
  substrate-side offline outbox for M6's chat; a real DLQ behind the
  `no DLQ (M5)` markers in `proxy.rs` and the architecture doc; scheduled tasks
  with no new coordination layer.
- **Costs**: one WIT change (`call-options` gains two fields); one manifest
  change (a schedule surface); **five new `SupervisorRole` config fields** for
  the queue's own budget — tick, attempt count, backoff ceiling, visibility
  timeout, and DLQ row cap, because `RetryPolicy`'s defaults (3 attempts from
  100 ms) size a socket retry and not a wait for an offline substrate; and an
  at-least-once delivery model that callers outside the supervisor must supply
  an idempotency key to use safely.
- **Narrowed deliberately**: durable delivery covers the binding push and not
  the certificate-bearing actions (§3). The gap is recorded in the deferred
  backlog rather than left implied by the phrase "durable delivery".
- **Explicitly does not build**: exactly-once delivery; a distributed lock or a
  registry compare-and-set; a replicated or cross-node queue; durable execution
  of WASM memory; a workflow engine; a pull-side app directory (ADR-0021 §6 is
  unchanged by this ADR).
- **Supersedes**: the lease-based cluster scheduler text in
  `system-requirements-spec.md` and `system-architecture.md` `[PLT-ASY]`
  (§6 above), and the client-side placement of the outbox in the same sections
  (§4 above). Both get a dated implementation-status note in the shape of the
  Universal Proxy one already at `system-architecture.md` §PLT-DAT.
