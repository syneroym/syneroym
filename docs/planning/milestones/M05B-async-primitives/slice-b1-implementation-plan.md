# Slice B1 Implementation Plan — The Queue Core, Durable Delivery, and the DLQ Surface

**Status:** 📋 Planned (2026-08-04, revised same day after review). Not started.
Milestone: [task.md](task.md) slice **B1**; milestone-level plan:
[implementation-plan.md](implementation-plan.md). Design of record:
[ADR-0023](../../../decisions/0023-durable-async-primitives.md) §1–§5 and
[ADR-0021](../../../decisions/0021-binding-propagation-and-app-supervisor.md)
§5. Depends on **M05A P0, A0–A5e, A7 — all Complete** and on **ADR-0023 being
accepted**. Gates B2, B3, B4.

**The one-sentence summary.** B1 replaces one trait implementation so that a
binding push to a substrate that is offline survives the failure, survives a
supervisor restart, converges when the substrate returns, and — when it never
returns — becomes something an operator can see and replay. **This is M05A slice
A6.**

**Review pass (2026-08-04), all findings incorporated.** Thirteen findings; the
first draft did not work in four places, and one of them narrowed the slice:

- **Queueability was decided for one action out of six, and two of the rest
  carry a certificate that expires in four hours** (§0.11). A queued
  `apply_plan` or `renew_cert` delivered late installs an already-dead
  certificate — the exact argument the draft used to exclude `restart`, not
  applied to the two actions it also covers. **`write_bindings` becomes the only
  queueable action**, which is both correct and what A6 actually asks for.
- **The reference scenario could not pass** (§0.12). `RetryPolicy`'s defaults
  are 3 attempts from 100 ms, so an item dead-letters in under a second, where
  the scenario needs it to survive a supervisor restart plus an operator
  action. Compounded by `task.md` claiming "no new config field" while
  `SupervisorRole` has nowhere to put an attempt budget. And `retry_with_backoff`
  is an in-process sleeping loop — a durable queue needs a computed
  `next_attempt_at`, so only the jitter helper is reusable.
- **Nothing stopped the worker and the resident loop writing one instance at
  once** (§0.13). `instance_locks` exists and the draft never said the worker
  takes it.
- **A late delivery is usually `stale`, which is success** (§0.14). The draft
  left the outcome mapping unspecified, and the natural reading — dead-letter
  anything that is not `applied` — would flood the DLQ with normal events.

Plus: the upcast inventory was `app_supervisor`-only while its test claimed
tree-wide (§0.3); "one alert per dead letter" is unbuildable against
`AlertStore`'s unique index (§0.9); the marker inventory missed two entries
(§0.8); the dependency sweep orphans two workspace entries (§0.10); three
documents disagreed on when A6 closes (§4).

**Read the milestone plan's §0 and §1 first.** They carry the findings that bind
every slice.

---

## §0 — What M05A slice A6's scope note, ADR-0021 §5, and the shipped tree leave open, understate, or state wrongly

### 0.1 (Scope-changing, and it is the slice's best news) The App Supervisor needs no idempotency key, because A5 already built its dedup

An at-least-once queue requires every queued action to be safe to apply twice
(milestone D-B-3). The obvious reading is that B1 must therefore build an
idempotency-key mechanism before it can make anything durable — and the
requirements spec says exactly that, in general terms.

For the supervisor it is already done, by three mechanisms A5 built for
unrelated reasons:

| `SubstrateActor` action | Existing fence | Where |
|---|---|---|
| `apply_plan` | Content-hash dedup over `(manifest, app_context-minus-generation)`; an unchanged redeploy is a no-op | A5a phase 2b (failure-matrix row 10) |
| `write_bindings` | The epoch guard's four-case rule — equal epoch, equal content is an idempotent no-op | [ADR-0021](../../../decisions/0021-binding-propagation-and-app-supervisor.md) §3 |
| `renew_cert` | Installs a certificate in place; the same certificate installed twice is the same state | A5d |
| `restart` | **None** — genuinely not idempotent | §0.4 |
| `instance_identity`, `held_generation` | Reads | — |

**Consequence for scope:** B1 builds no idempotency-key machinery at all. The
key appears in B2, where a guest call has no such inheritance. This is the
single biggest reason B1 is small enough to be one slice, and it belongs in a
comment where a reader of B1's diff will find it, not only here.

**This table answers idempotence and not queueability**, and the review found
the draft silently treating them as one question. They are two: *may this be
applied twice* and *is it still meaningful an hour later*. §0.11 answers the
second.

### 0.2 (Correctness, blocking) A6's own scope note — "nothing above the trait changes" — is false under queue-always, and true only under try-then-queue

`write_bindings` returns `Vec<BindingWriteOutcome>`: `applied`, `no-op`,
`stale(u64)`, `conflict(u64)`
([control-plane.wit:324](../../../../crates/wit_interfaces/wit/control-plane/control-plane.wit#L324)).
The supervisor reads it — that is how convergence is reported and how a conflict
is distinguished from a stale write, both of which A5c wired into alerts and
into `status`.

An implementation that enqueues and returns immediately has no outcome to
return. It would have to synthesize an "accepted" variant, and every caller
above the trait would need to learn that accepted is not applied. That is
precisely the change A6 promises not to make.

**Resolved by attempting synchronously first and enqueuing only on transport
failure.** A reachable substrate takes exactly today's path and produces a real
outcome; an unreachable one produces a queued item plus the `Degraded` state the
supervisor already reports. ADR-0023 §2 records this as the design of record and
explains that it also protects the measured happy-path budget.

This is the finding that most changes what B1 *is*, and it was not visible from
A6's text alone.

### 0.3 (Understated) There are ten upcast sites, not six, and four of them are outside `app_supervisor`

`client.clone() as Arc<dyn SubstrateActor>` appears **six** times in
[service.rs](../../../../crates/app_supervisor/src/service.rs) — five in
production code (lines 877, 922, 1878, 2043, 2224) and one in tests (7028).
`as_actors` (line ~1875) already centralizes the map-shaped conversion, but four
other sites upcast a single client directly.

**And four more exist outside that file** (review finding; the draft's count was
`app_supervisor`-only while its test claimed "no direct upcast remains"):

| Site | What it is |
|---|---|
| [roymctl app.rs:555](../../../../apps/roymctl/src/commands/app.rs#L555), [:566](../../../../apps/roymctl/src/commands/app.rs#L566) | Production CLI deploy path |
| [binding_push_e2e.rs:413](../../../../crates/substrate/tests/binding_push_e2e.rs#L413) | e2e fixture |
| [multi_substrate_placement_e2e.rs:511](../../../../crates/substrate/tests/multi_substrate_placement_e2e.rs#L511) | e2e fixture |

Two consequences, and the second is the dangerous one:

- **`roymctl` deliberately keeps the undurable actor** (D-B1-11). A CLI process
  exits when the command finishes, so a durable queue there is a queue with no
  worker — items would be written and never drained, which is worse than not
  queueing. This is a decision, not an oversight, and the draft left it unstated
  in exactly the place §0.3's own argument warns about.
- **The e2e fixtures are how phase 5 could silently test nothing.**
  [task.md](task.md) says the reference scenario is written "in the shape"
  those two files establish. Written in that shape it upcasts a bare client,
  bypasses the durable actor entirely, and exercises none of this slice. Phase 5
  must build its actor through the phase-1 constructor, and test 24 asserts the
  queue was involved rather than only that convergence happened.

If B1 wraps at each site instead of introducing one construction point, the
wrapping is ten near-identical edits and the next person to add a call site
silently gets the undurable path. **B1's first phase is therefore a pure
refactor with no behavior change**, merged and reviewed on its own. Then the
wrap is one edit in one place, and `roymctl`'s opt-out is one visible argument
at one call site rather than an absence.

### 0.4 (Ambiguous, and the trait already anticipates it) `restart` must not be queued

`restart`'s doc comment says it outright: "A6 may keep this synchronous -- a
restart queued for later delivery is usually wrong -- and that is a decision for
A6's implementation"
([deploy.rs:52](../../../../crates/sdk/src/deploy.rs#L52)).

It is right, and the reason is worth writing down rather than inheriting: a
restart is remediation for a condition observed *now*. Delivered an hour later
it restarts a service that recovered on its own. The supervisor's bounded
remediation policy — attempts, backoff, terminal `Degraded` — already decides
what a failed restart means, and a queue behind it would be a second policy
disagreeing with the first.

§0.11 generalizes this from one action to all six.

### 0.5 (Correctness) `ActionState::Pending` was removed on the argument that nothing will ever enqueue work — and the outbox does not contradict it

A5c removed `ActionState::Pending` from `DeploymentJournal`, arguing "nothing in
this tree ever wrote one... there is still no `Pending` writer and never will be
one under this design"
([journal.rs:76](../../../../crates/app_orchestration/src/journal.rs#L76);
[deferred-backlog.md](../../deferred-backlog.md) *Recently resolved*).

A reader could take this slice as the moment that argument expires. It does not,
and the distinction matters for where the queue lives: the journal records
**deployments** — what was applied, to which substrate, in which state. The
outbox records **deliveries** — an attempt that has not succeeded yet. A
deployment whose delivery is queued is genuinely `InProgress`, not `Pending`;
the queue is what makes progress happen.

**So the queue is its own store, not more columns on `DeploymentJournal`** —
exactly the reasoning `AlertStore` gives for itself ("deliberately its own store
rather than more tables on `DeploymentJournal`",
[alerts.rs:3](../../../../crates/app_orchestration/src/alerts.rs#L3)).

**Its wiring point is named, because it is not the one the draft implied**
(review finding). `AlertStore::from_connection` is public and takes
`Arc<Mutex<Connection>>`; `SupervisorStore::from_connection` is **private** and
takes a bare `Connection`, building the `Arc<Mutex<_>>` itself and handing
clones to the journal and the alert store
([store.rs:74](../../../../crates/app_supervisor/src/store.rs#L74)). The queue
store is constructed **there**, as a fourth sibling on those same two lines —
not by a caller, which could not reach that constructor anyway. This is also the
line M05C's collision table points at.

### 0.6 (Coverage) The DLQ's stated purpose fails unless something reads it

"Preventing silent data loss" is the requirements spec's justification for the
DLQ. A table nothing surfaces converts silent loss into quiet loss, which is not
better in any way an operator would notice.

So terminal failure must raise an alert through the existing `AlertStore` and be
listable and replayable. That is a new `AlertKind` variant and two `supervisor`
WIT verbs — small, but it is scope that "build a DLQ" does not obviously
include, and leaving it out would let B1 pass its own tests while failing its
purpose.

### 0.7 (Correctness) Shutdown must not drain, and the existing unbounded-shutdown row is the reason

`AppSupervisor::run`'s shutdown latency is already unbounded in the number of
managed instances and unreachable substrates
([deferred-backlog.md](../../deferred-backlog.md) §8). A worker that drains its
queue on shutdown adds a second unbounded wait, and against exactly the same
unreachable substrates.

It is also unnecessary: the queue is durable, which is the entire point.
**Shutdown abandons in-flight work and lets the visibility timeout return it to
`Pending`.** Stated as a decision because "drain on shutdown" is the reflex.

### 0.8 (Stale) Four shipped markers claim this work by name, not two

The draft's inventory listed two; the review found two more:

| Marker | Whose | Note |
|---|---|---|
| [deploy.rs:8](../../../../crates/sdk/src/deploy.rs#L8) | **B1** | Describes the outbox as future work, and misattributes it to "A5's durable outbox implementation" — it is A6's |
| [deploy.rs:44](../../../../crates/sdk/src/deploy.rs#L44) | **B1** | "A6 replaces the implementation with an outbox/DLQ-backed one" |
| [deploy.rs:51-55](../../../../crates/sdk/src/deploy.rs#L51) | **B1** | `restart`'s own doc comment defers the queueability call to A6. **D-B1-3 makes it stale** — the call is made, and the comment must state the rule rather than promise a future decision |
| [router/proxy.rs:456](../../../../crates/router/src/proxy.rs#L456) | B2 | "Failed-after-retries fails directly -- no DLQ (M5)" |
| [rpc/proxy.rs:80](../../../../crates/rpc/src/proxy.rs#L80) | B2 | "Failed-after-retries fails directly -- no queueing (DLQ is M5)". Same class as the row above, on the request type rather than the call site |

Per the house rule against planning-doc IDs in code comments, every replacement
states the invariant, not the slice.

### 0.9 (Correctness) "One alert per dead letter" cannot be built on `AlertStore`, and a standing alert is the better answer anyway

The draft asked for one alert per dead letter. `AlertStore` cannot express that:
its unique index is
`(instance_id, IFNULL(logical_ref,''), substrate_did, kind) WHERE cleared_at IS NULL`
([alerts.rs:214](../../../../crates/app_orchestration/src/alerts.rs#L214)), and
`raise` updates a matching active row rather than opening a second
([:226](../../../../crates/app_orchestration/src/alerts.rs#L226)). Two dead
letters for the same member and substrate collapse into one refreshed row.

The review suggested riding the item id in `logical_ref` to force distinct rows.
**Rejecting that**: `logical_ref` means a logical service reference everywhere
else it appears, and overloading it would break `alerts`' own grouping for the
one caller that needs a different shape.

**The standing-alert reading is also the better operational answer.** What an
operator needs to know is "this member has undeliverable work", not one row per
failed item — the same reason `SubstrateUnreachable` is raised once per
substrate and never per service (D-A4-13). So: **one standing alert per
`(instance, logical_ref, substrate)`, with the dead-letter count in `detail`**,
refreshed as more accumulate.

**And it needs a clear path, which the draft never gave it.**
`RemediationExhausted` documents its own (`force-reconcile`/`adopt` clears it);
this one is cleared when that key's dead letters are gone — by successful replay
or by prune. An alert nothing can clear is a permanent red mark, which trains
operators to ignore the list.

### 0.10 (Chore, requested) Six unused dependencies ride along with phase 1, and two of them orphan a workspace entry

Requested 2026-08-04 to be folded into phase 1, on the reasoning that a
no-behavior-change refactor is the right merge to carry a no-behavior-change
manifest cleanup. **Verified against the tree** — each is present in its manifest
and has zero references in that crate's source:

| Crate | Dependency | Workspace entry after removal |
|---|---|---|
| `syneroym-router` | `quinn` | **n/a** — declared directly (`Cargo.toml:47`, pinned `0.11.11`), not through the workspace. The largest of the six: a full QUIC implementation the router reaches only through `iroh` |
| `syneroym-observability` | `metrics-util` | **Orphaned — remove `Cargo.toml:97` too.** Sole consumer. `metrics` itself stays; it is used |
| `syneroym-perf` (`tests/perf`) | `assert_cmd`, `sysinfo` | **Both stay.** `assert_cmd` is still used by `syneroym-substrate` and `roymctl`; `sysinfo` by `syneroym-observability` and `xtask` |
| `syneroym-sandbox-podman` | `metrics` | Stays — many consumers. The only `metrics` string in this crate's source is a comment on `engine.rs:264` |
| `syneroym-smoke-tests` | `syneroym-sdk` | Intra-workspace edge; removing it shortens the build graph |
| `syneroym-substrate` | `chromiumoxide` | **Orphaned — remove `Cargo.toml:169` too.** Sole consumer. Dev-dependency; the browser e2e suite is Playwright/TypeScript |

The review found the orphan half, which the first draft missed: a `[workspace.dependencies]`
entry with no consumer is dead weight that the next `cargo update` still
resolves.

**The residual risk is feature-gated code**, which a text search cannot fully
rule out. The grep covered every `.rs` file in each crate, including gated ones,
so the risk is small — but the check that settles it is `cargo check --workspace
--all-targets --all-features`, which the exit criteria already require and which
the implementer runs after the removal, not before.

**A tool would keep this from drifting again.** `mise.toml` has no
unused-dependency check. Deliberately **not** folded in here — adding a lint tool
means deciding whether CI fails on it, which is a bigger decision than this
chore. Backlog row instead.

### 0.11 (Scope-changing, blocking) Queueability was decided for one action out of six, and two of the others carry a certificate that dies in four hours

D-B1-3 said queueability is per action, then decided only `restart`. The review
found that the same argument disqualifies two more, and for a sharper reason
than staleness of intent — staleness of the *payload*:

- A `deployment-plan`'s service record carries
  `instance-certificate: option<string>`
  ([control-plane.wit:157](../../../../crates/wit_interfaces/wit/control-plane/control-plane.wit#L157)).
- `renew-cert` takes `instance-certificate: string`
  ([:250](../../../../crates/wit_interfaces/wit/control-plane/control-plane.wit#L250)).
- `renewed_cert_expires_hours` defaults to **4**
  ([config.rs:566](../../../../crates/core/src/config.rs#L566)).

A queued `apply_plan` or `renew_cert` delivered after that window installs a
certificate that is already dead. The substrate accepts it — it is a
well-formed, correctly-signed certificate — and the service then fails its
handshake closed. That is worse than the delivery never happening, because it
looks like success.

**Resolution: `write_bindings` is the only queueable action in B1** (D-B1-12).
Three reasons, and the third is what makes this a narrowing rather than a
retreat:

1. `BindingWrite` carries `service_id`, `app_instance_id`, `bindings`, and
   `generation`
   ([control-plane.wit:331](../../../../crates/wit_interfaces/wit/control-plane/control-plane.wit#L331))
   — **no certificate, nothing time-limited**. Its content is as valid an hour
   later as it was when queued.
2. It is exactly what A6 asks for. A6's scope note says "durable **push**
   delivery, retry against substrates that are offline at the time of the
   change", and ADR-0021 §5 is about binding propagation throughout. The
   binding push is the sticky-failure case the whole ADR is built around.
3. Making the other two durable is a **different design**, not more of the same
   one: the queue would have to store *intent* (service id, generation) rather
   than the payload, and re-mint the certificate at delivery time — which needs
   the supervisor's vault open at delivery time, and the vault is locked after
   every restart until a human runs `inject-kek`. That is a real question with a
   real dependency, and inheriting it silently into B1 is how a slice grows an
   order of magnitude.

Recorded as a backlog row with that reasoning, so the narrowing is a decision
rather than a gap.

### 0.12 (Correctness, blocking) The queue has no attempt budget, no clock, and nowhere to put either — and `RetryPolicy`'s defaults would dead-letter the reference scenario in under a second

Three connected problems the draft did not see, all found in review.

**The defaults are wrong by three orders of magnitude.** `RetryPolicy` defaults
to `max_attempts: 3`, `initial_backoff_ms: 100`, `backoff_multiplier: 2.0`,
`max_backoff_ms: 30_000`
([config.rs:867-878](../../../../crates/core/src/config.rs#L867)). Three
attempts from 100 ms is a total window of about 700 milliseconds. The reference
scenario needs an item to survive steps 2–6: a supervisor restart *and* an
operator bringing substrate B back. Under these defaults it is in the DLQ before
the operator reaches for the keyboard, and the scenario cannot pass.

**There is nowhere to configure it.** `SupervisorRole`
([config.rs:575-627](../../../../crates/core/src/config.rs#L575)) carries
`poll_interval_secs`, `db_name`, `max_restart_attempts`, `restart_backoff_secs`,
`alert_topic`, `master_backup_dir`, `master_anchor_refresh_interval_secs`,
`renewed_cert_expires_hours`, and `max_renewals_per_pass` — no queue field of
any kind. And [task.md](task.md)'s migration-impact section said "no new config
field", which the review correctly flagged as contradicting the scenario. That
line was mine and it was wrong; it is corrected.

**And `retry_with_backoff` cannot be the schedule.**
[retry.rs](../../../../crates/core/src/retry.rs) exposes exactly two things:
`retry_with_backoff`, an in-process loop that `tokio::time::sleep`s between
attempts, and `calculate_jittered_backoff`. A durable queue does not sleep — it
computes a `next_attempt_at` timestamp, writes it, and forgets the item until a
worker tick finds it due. **Only the jitter helper is reusable.**

This does *not* void milestone D-B-5, whose point was that the project must not
grow a second retry *policy*. It does correct its mechanism claim: the queue
reuses the `RetryPolicy` **struct** and `calculate_jittered_backoff`, and
implements its own timestamp arithmetic. D-B-5 is reworded accordingly.

**So B1 sizes its own knobs** (D-B1-13), rather than leaving four numbers
implied:

| Knob | Value | Reasoning |
|---|---|---|
| Attempt budget | Its own field, defaulting well above `RetryPolicy`'s 3 | The budget must outlast a human noticing an outage, not a transient socket error |
| Total window before dead-lettering | Stated explicitly, and **the property the test asserts** | The interval is an implementation detail; "how long an unreachable substrate has to come back" is the operator-facing number, the same way §0.4's sixty-refresh figure is in M05C |
| Worker tick | Its own field, well under `poll_interval_secs` (default 30) | Otherwise the queue is slower than the loop it exists to beat — the budget in `task.md` says recovery is within one worker tick, *not* one poll interval |
| Visibility timeout | Its own field, comfortably above the longest single attempt | Too short re-delivers work still in flight; too long strands a crashed worker's item |
| DLQ prune bound | A row cap with a stated number, pruned oldest-first on write | D-B1-9 said "prunable" and "the bound is asserted" without saying what the bound is or what triggers the prune |

### 0.13 (Correctness, blocking) The worker and the resident loop can write the same instance at the same time, and nothing stops them

The supervisor serializes per-instance writes through `instance_locks`, a
`DashMap<String, Arc<AsyncMutex<()>>>`
([service.rs:205](../../../../crates/app_supervisor/src/service.rs#L205)),
handed out by `instance_lock`
([:308](../../../../crates/app_supervisor/src/service.rs#L308)) and taken by
every lifecycle verb and by the loop's own pass.

Phase 3 spawns the worker as a second task beside that loop. The draft never
said it takes the same lock, so a queued write could interleave with a live pass
write for the same instance.

**The consequence is bounded but not harmless.** The epoch guard means a stale
write is rejected rather than applied, so no wrong binding lands. But the
four-case rule reports *equal epoch, different content* as `conflict` — the
signal ADR-0021 §3 reserves for **two writers disagreeing** — and interleaving
would let one supervisor produce that signal by racing itself. A spurious
`BindingConflict` alert on the one mechanism built to detect split-brain is a
bad failure: it is indistinguishable from the real thing.

**The worker takes the same `instance_lock` before applying an item**
(D-B1-14). One line, and the alternative is an alert nobody can trust.

### 0.14 (Correctness) A queued write delivered late is usually `stale`, and that is success

The worker's outcome mapping was unspecified. The natural reading of "the item
failed unless it returned `applied`" is wrong for this queue, and would flood
the DLQ with entirely normal events: by the time an unreachable substrate
returns, the resident loop has very often already pushed a newer epoch, so the
queued write arrives `stale` — meaning *the thing it wanted has already
happened*.

**The mapping** (D-B1-15):

| Outcome | Worker does | Why |
|---|---|---|
| `applied` | Complete, delete | Delivered |
| `no-op` | Complete, delete | Equal epoch, equal content — already true |
| `stale(n)` | **Complete, delete** | A newer epoch is installed. The queued intent is satisfied by something better. Retrying is pointless; dead-lettering would report normal convergence as data loss |
| `conflict(n)` | Complete, delete, **and raise `BindingConflict`** exactly as the synchronous path does | Two writers disagree at one epoch. Retrying cannot resolve that, and the alert is the whole response ADR-0021 §3 specifies. Suppressing it on the queued path would make the signal depend on whether the substrate happened to be reachable |
| Transport error | Retry per the budget (§0.12) | The only genuinely retryable case, and the only one that reaches the DLQ |

The `conflict` row is the one the draft's test list missed entirely: test 11
covered the synchronous path only.

---

## §1 — B1 decisions

| ID | Decision |
|---|---|
| **D-B1-1** | **Try-then-queue**: the durable `SubstrateActor` attempts synchronously first and enqueues only on transport failure (§0.2, ADR-0023 §2). This is what makes A6's "nothing above the trait changes" literally true, and it keeps the queue off the measured happy path. |
| **D-B1-2** | **B1 builds no idempotency-key machinery** (§0.1, ADR-0023 §1). The supervisor's safety comes entirely from A5's existing content-hash, epoch, and generation fences. Each is asserted by a test, not assumed. |
| **D-B1-3** | **Queueability is declared per action, and the declaration is exhaustive** (§0.4, §0.11). Not a generic wrap-every-method wrapper. `restart`'s own doc comment, which defers this call to A6, is rewritten to state the rule (§0.8). |
| **D-B1-4** | **Phase 1 is a pure refactor**: one constructor for `Arc<dyn SubstrateActor>`, all **ten** upcast sites routed through it, no behavior change, merged on its own (§0.3). The durable wrap is then one edit. |
| **D-B1-5** | **The queue is its own store**, constructed as a fourth sibling inside the private `SupervisorStore::from_connection` alongside the journal and the alert store (§0.5). `ActionState::Pending` stays removed and the argument for removing it stands. |
| **D-B1-6** | **Terminal failure raises one standing alert per `(instance, logical_ref, substrate)` with the dead-letter count in `detail`** — not one per item, which `AlertStore`'s unique index cannot express (§0.9) — **cleared when that key's dead letters are gone**, by replay or prune. Plus `dead-letters` and `replay` verbs on the `supervisor` interface, gated exactly as the surrounding verbs are: no new resource namespace, the mistake D-A5-24 corrected. |
| **D-B1-7** | **`replay` re-enqueues; it never executes inline.** A dead letter that fails again returns to the DLQ with its attempt history intact (ADR-0023 §5). |
| **D-B1-8** | **Shutdown does not drain.** In-flight work is abandoned and the visibility timeout returns it to `Pending` on the next start (§0.7). |
| **D-B1-9** | **A completed item is deleted, not tombstoned**; dead letters are capped at a stated row count and pruned oldest-first on write (§0.12's table). The bound is a number and a trigger, not an adjective. |
| **D-B1-10** | **Six unused dependencies are removed in phase 1**, as a separate commit inside the same merge, **plus the two `[workspace.dependencies]` entries they orphan** (`metrics-util`, `chromiumoxide`) (§0.10). Manifest-only; re-verified with `cargo check --workspace --all-targets --all-features`. |
| **D-B1-11** | **`roymctl` keeps the undurable actor, deliberately** (§0.3). A CLI process exits when the command finishes, so a durable queue there would be written and never drained — worse than not queueing. Expressed as a visible argument at the construction point, not as an absence. |
| **D-B1-12** | **`write_bindings` is the only queueable action** (§0.11). `apply_plan` and `renew_cert` embed an instance certificate that expires in 4 hours by default, so a late delivery installs a dead certificate and *looks* like success. Making them durable needs intent-not-payload storage plus a re-mint at delivery, which needs the vault open at delivery time — a different design, recorded as a backlog row. This is also exactly the scope A6 and ADR-0021 §5 ask for. |
| **D-B1-13** | **The queue sizes its own knobs on `SupervisorRole`** — attempt budget, total window, worker tick, visibility timeout, DLQ cap (§0.12). It reuses `RetryPolicy`'s **struct** and `calculate_jittered_backoff`, **not** `retry_with_backoff`, which sleeps in-process where a durable queue must compute a `next_attempt_at`. Milestone D-B-5's intent (no second retry *policy*) stands; its mechanism claim is corrected. |
| **D-B1-14** | **The worker takes the same `instance_lock` the resident loop takes**, before applying an item (§0.13). Without it one supervisor can race itself into a `BindingConflict` alert that is indistinguishable from real split-brain. |
| **D-B1-15** | **The outcome mapping is `applied`/`no-op`/`stale` → complete; `conflict` → complete **and** raise `BindingConflict` as the synchronous path does; transport error → retry** (§0.14). Only transport errors reach the DLQ. A late delivery arriving `stale` is convergence, not loss. |

---

## §2 — Phase plan and merge order

Five phases. The ordering rule is that **nothing changes behavior until phase
3**: phases 1 and 2 are a refactor and a new unreferenced crate.

1. **The upcast refactor** (D-B1-4), **plus the unused-dependency sweep**
   (D-B1-10). One constructor in
   [service.rs](../../../../crates/app_supervisor/src/service.rs); all ten sites
   routed through it, including `roymctl`'s two, which pass the undurable
   variant explicitly (D-B1-11). No behavior change, no new dependency.
   Tests: 1.

   **Two commits, one merge.** The refactor and the dependency sweep touch
   disjoint crates and share only the property that neither changes behavior, so
   they ride the same merge but stay separate commits — a `chore(deps)` next to
   a `refactor(app-supervisor)`, so a later bisect over either is clean.
2. **The queue crate.** `crates/async_queue/` → `syneroym-async-queue`. Schema
   (`outbox`, `dead_letters`), states (`Pending`, `InFlight`, `Dead`; completed
   items deleted per D-B1-9), a claim that is a conditional `UPDATE` with a
   visibility timeout, `next_attempt_at` arithmetic over `RetryPolicy` +
   `calculate_jittered_backoff` (D-B1-13), and a worker driven by
   `tokio::time::interval` in `build_pass_interval`'s `MissedTickBehavior`
   shape. Nothing calls it yet. Tests: 2–10.
3. **Config, wiring, and the durable actor.** The five `SupervisorRole` fields
   (D-B1-13); the queue store constructed inside
   `SupervisorStore::from_connection` (D-B1-5); the try-then-queue wrapper
   (D-B1-1) with its exhaustive queueability declaration (D-B1-3, D-B1-12) and
   its outcome mapping (D-B1-15); the `instance_lock` acquisition (D-B1-14). One
   edit at the constructor from phase 1. The worker is spawned beside the
   resident loop and joined the way `supervisor_join` already is
   ([runtime.rs:246](../../../../crates/substrate/src/runtime.rs#L246)), and
   **not** drained on shutdown (D-B1-8). Tests: 11–23.
4. **The DLQ surface.** The `AlertKind` variant, the standing alert with its
   count and its clear path (D-B1-6), `dead-letters` and `replay` on
   `supervisor.wit`, the dispatch arms, and `roymctl supervisor dead-letters` /
   `replay` (D-B1-7). Tests: 24–29.
5. **The reference scenario and the docs.** The two-substrate e2e from
   [task.md](task.md) — built through the phase-1 constructor, **not** in the
   bare-upcast shape the existing e2e fixtures use (§0.3) — plus the milestone
   plan §3's documentation and the four markers in §0.8 that are B1's.
   Tests: 30–31.

**What could move:**

- **Phase 1 should merge first and separately even if the slice is under
  pressure.** It is the cheapest phase and the one that prevents the next call
  site from silently getting the undurable path.
- **Phase 2 could merge with phase 3** — the crate is dead code until phase 3
  uses it. Reviewing a queue implementation apart from its first consumer is
  probably still worth one extra merge, since the visibility-timeout and claim
  semantics are the delicate part and they are legible on their own.
- **Phase 3 cannot be split** along "config versus actor". The actor is
  untestable at its real budget without the fields, and fields nothing reads are
  not reviewable.
- **Phase 4 cannot be dropped to save time.** Without it the DLQ is a table
  nothing reads, which fails the slice's own stated purpose (§0.6).
- **Phase 5's e2e is the only part with a real time cost** (two booted
  substrates plus a supervisor restart). The same split D-A5e-15 accepted
  applies: the in-process tests prove every property, the e2e proves the
  *sequence*. It must still land — a supervisor restart with a live queue is not
  provable in-process.

---

## §3 — B1 tests

**e2e cases are marked; everything else is a unit test.** Numbering is
per-milestone and restarts at 1.

**Phase 1:**

1. `every_actor_is_built_through_one_constructor` — the property is "no direct
   upcast remains outside the constructor", across all ten sites (§0.3). A test
   cannot read source, so **this is a review item, not a test**, and the plan
   says so rather than implying coverage. What *is* testable is D-B1-11:
   `roymctl`'s construction asks for the undurable variant explicitly, so the
   argument exists and can be asserted at the type level.

**Phase 2 —** `crates/async_queue`:

2. `an_enqueued_item_survives_reopening_the_database` — the whole point
3. `a_claimed_item_is_invisible_to_a_second_claim` — failure-matrix row 6
4. `a_claim_that_is_never_completed_returns_to_pending_after_its_visibility_timeout`
   — failure-matrix row 1, and the crashed-worker case
5. `next_attempt_at_follows_the_configured_policy_with_jitter` — D-B1-13.
   Asserts the computed timestamps against `RetryPolicy`'s multiplier and cap,
   and that `calculate_jittered_backoff` is what supplies the spread. Explicitly
   **not** an assertion about `retry_with_backoff`, which this queue does not use
6. `an_item_that_exhausts_its_attempts_moves_to_the_dlq_and_leaves_the_outbox`
   — failure-matrix row 4
7. `a_completed_item_is_deleted_not_tombstoned` — D-B1-9
8. `the_dlq_is_capped_and_prunes_oldest_first` — D-B1-9's bound, as a number
9. `a_terminal_error_skips_the_remaining_attempts` — failure-matrix row 9
10. `an_empty_queue_tick_issues_one_indexed_query_and_no_scan` — the idle budget

**Phase 3 —** config, wiring, the durable actor:

11. `a_reachable_substrate_never_touches_the_queue` — **the load-bearing budget
    test**. Asserted as "the queue is untouched", not as a timing, so it cannot
    pass by being fast
12. `a_successful_write_bindings_returns_the_same_outcomes_it_returns_today` —
    D-B1-1; the four `BindingWriteOutcome` variants, unchanged
13. `a_transport_failure_enqueues_and_leaves_the_instance_degraded`
14. `a_callee_error_is_not_enqueued` — only transport failures are retried
15. `a_failed_restart_is_never_enqueued` — D-B1-3, failure-matrix row 3
16. `a_failed_apply_plan_and_renew_cert_are_never_enqueued` — **D-B1-12**, the
    certificate-expiry case (§0.11). Asserts the refusal, and that the refusal
    names the reason, so a future slice removing it must confront the argument
17. `applying_a_queued_write_bindings_twice_is_a_no_op` — failure-matrix row 2
    against the epoch fence
18. `a_queued_write_delivered_stale_completes_and_does_not_dead_letter` —
    **D-B1-15**, the case that would otherwise flood the DLQ with normal
    convergence
19. `a_queued_write_delivered_conflicting_raises_the_same_alert_as_the_synchronous_path`
    — D-B1-15; the row test 12's synchronous coverage misses
20. `the_worker_and_a_loop_pass_never_write_one_instance_concurrently` —
    D-B1-14, driven by holding the lock and asserting the worker blocks
21. `shutdown_abandons_in_flight_work_rather_than_draining` — D-B1-8
22. `the_worker_resumes_a_queued_item_after_a_restart` — in-process analogue of
    e2e step 5
23. `recovery_completes_within_one_worker_tick_and_not_one_poll_interval` — the
    budget the draft asserted nowhere. Driven against a fake clock with
    `poll_interval_secs` set far above the worker tick, so passing by accident is
    impossible

**Phase 4 —** the DLQ surface:

24. `a_second_dead_letter_for_one_member_refreshes_the_alert_and_raises_its_count`
    — D-B1-6, replacing the draft's unbuildable "raises once per dead letter"
25. `the_alert_clears_when_the_last_dead_letter_for_that_key_is_gone` — D-B1-6's
    clear path, the thing `RemediationExhausted` documents and the draft omitted
26. `dead_letters_lists_what_the_store_holds`
27. `replay_re_enqueues_and_does_not_execute_inline` — D-B1-7
28. `a_replayed_item_that_fails_again_returns_to_the_dlq_with_its_history` —
    failure-matrix row 5
29. `every_new_verb_is_refused_without_the_gate_the_neighbouring_verbs_use` —
    failure-matrix row 14; extends
    `every_verb_is_refused_without_substrate_admin` rather than inventing a
    second list

**Phase 5 —** e2e and the two budgets that need a live path:

30. **(e2e)** `a_binding_push_to_an_offline_substrate_converges_after_it_returns`
    — [task.md](task.md)'s reference scenario steps 1–7, including the
    supervisor restart at step 5. Built through the phase-1 constructor, and
    asserts the item passed **through the queue**, not merely that convergence
    happened (§0.3)
31. **(e2e)** `a_permanently_unreachable_substrate_lands_in_the_dlq_and_replays`
    — step 8

**Also required by exit criteria 2 and 3, and named here so they are not
forgotten:**

32. `the_queue_lives_in_supervisor_db_under_the_same_protection_as_desired_state`
    — failure-matrix **row 13**'s supervisor half, which the draft left with no
    test. Asserts the queue tables are in the same database file and inherit its
    encryption posture, rather than opening one of their own
33. `enqueue_on_failure_costs_one_insert` — the `< 1 ms` budget, asserted as
    statement count rather than wall-clock, so CI noise cannot fail it

---

## §4 — What closing B1 closes

**M05A slice A6 closes when B1 lands**, not at milestone closeout. The review
found three documents disagreeing about this, and it is resolved in this
direction rather than the other:

- A6's own scope is "Replace the A5 trait's implementation with an outbox/DLQ-backed
  one: durable push delivery, retry against substrates that are offline at the
  time of the change, terminal-failure handling... Nothing above the trait
  changes." **B1 discharges all of it.** Holding A6's status until B3's
  scheduling work lands would be bookkeeping, not truth.
- The `[PLT-ASY]` traceability row is a **milestone** row and closes at
  milestone closeout, when B2–B4 are in. A6 is a **slice** and closes with its
  own deliverable. These are different things closing at different times, which
  is the normal case, not a contradiction.
- A6's trigger is worded "M5 item 1 marked Complete in the matrix". Read
  literally that is closeout. The pointer already added to
  [M05A task.md](../M05A-app-supervisor/task.md)'s A6 section notes that A6 is
  executed as B1; that note now also records that A6's status follows B1.

So, on B1 landing: A6 is recorded Complete in
[M05A status.md](../M05A-app-supervisor/status.md), and its
[deferred-backlog.md](../../deferred-backlog.md) §8 row moves to *Recently
resolved*. [task.md](task.md)'s exit criteria and the milestone plan's §3 are
corrected to match.

Also closed:

- **ADR-0021 §5's "after M5"** clause for the binding-push path, and with it the
  second convergence clause of that ADR's 2026-08-03 amendment — an unreachable
  dependent converging after it returns — which becomes measurable for the first
  time.
- **The three `deploy.rs` markers** (§0.8), including `restart`'s, which D-B1-3
  makes stale.

What it does **not** close: the proxy DLQ markers in `router` and `rpc` (B2),
scheduling (B3), sagas (B4), or the `[PLT-ASY]` matrix row, which needs all four
built slices. Long-running tasks are outside this milestone entirely (D-B-2).

It also does not close two rows its own decisions create: *durable delivery for
certificate-bearing actions* (D-B1-12) and *no unused-dependency check in the
build pipeline* (D-B1-10). Removing six dead entries is not the same as stopping
the seventh.
