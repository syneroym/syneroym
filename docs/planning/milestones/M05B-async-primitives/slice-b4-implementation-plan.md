# Slice B4 Implementation Plan — Saga Compensations

**Status:** 📋 Planned (2026-08-07), **all open questions answered
(2026-08-07)**. Not started. Milestone:
[task.md](task.md) slice **B4**; milestone-level plan:
[implementation-plan.md](implementation-plan.md) §0.6 and §1. Design of
record: [ADR-0023](../../../decisions/0023-durable-async-primitives.md) §7
(sagas are a convention plus a helper), §4 (owner-local queues, the
idempotency key), §5 (a terminal failure is an operator surface, not a
graveyard). Depends on **B1 — Complete 2026-08-05** and **B2 — Complete
2026-08-05** (this slice reuses B2's receiver-side fence for every undo it
sends). **Closes the milestone**: exit criterion 6 and the `[PLT-ASY]`
matrix row both wait on this slice.

**The one-sentence summary.** B4 lets a service record the steps of a
multi-service workflow durably as it takes them, and lets it — or a missed
deadline — ask the substrate to walk that record backwards, calling
`saga-undo-<method>` on each step's own target, with the same retry, fence and
operator surface B1/B2 already built for delivery.

**Read first:** ADR-0023 §7 (three paragraphs, and §0.1 below disagrees with
one sentence of it); the milestone plan's §0.5 (the 5-second
`dispatch_epoch_timeout_secs` finding — it is the reason B5 was deferred and
it also decides B4's central question); D-B-3 (at-least-once) and D-B-5 (no
second retry policy). From the shipped tree, in this order:

- [router/src/proxy.rs](../../../../crates/router/src/proxy.rs) —
  `enqueue_call`'s three up-front refusals, `request_from`,
  `drain_outboxes_once`/`drain_one_outbox`/`run_outbox_worker`. B4's worker
  is the same shape, and its store is the same shape as `ProxyOutbox`.
- [router/src/proxy_outbox.rs](../../../../crates/router/src/proxy_outbox.rs)
  — `queue_for`'s single-flight open, `resolve_target`, `disposition_of`.
- [async_queue/src/lib.rs](../../../../crates/async_queue/src/lib.rs) —
  `init_schema`, `record_dead_letter`'s one-transaction rule (B2 review
  finding N3), `backoff_before_wait`.
- [sandbox_wasm/src/engine.rs](../../../../crates/sandbox_wasm/src/engine.rs)
  `exports_authorize_rows` (line 600) and
  [control_plane/src/service/orchestration.rs](../../../../crates/control_plane/src/service/orchestration.rs)
  `deploy_wasm_service` (line 698). Together they are the *only* deploy-time
  export gate in the tree, and §0.2 says why the milestone plan names the
  wrong precedent.

**Two decisions were taken with the requester (2026-08-07) before drafting
the phases**, and both deviate from ADR-0023 §7 and the milestone plan §0.6,
so both owe those documents an amendment (§5):

- **No manifest declaration of saga participation.** A service with no
  compensation is the ordinary case, not an author who forgot, so absence
  already means "takes part in no saga" (§0.4a).
- **The marker is `saga-undo-<method>`, not `undo-<method>`.** `undo-` is an
  ordinary business verb, and a check built on it would refuse legal names
  and could make the walk call a business function believing it to be a
  compensation (§0.4b).

Both came out of §4's Q1 and are recorded there with their reasoning; the
second is what makes the first safe. A third decision from the same
conversation, §4's Q2, kept the plan as drafted: a failed step does **not**
compensate automatically.

**Everything in this document about "what B1/B2/B3 built" was checked
against the shipped source**, not against their plans — all three shipped
materially different from their first drafts after review rounds, and
[status.md](status.md) records that.

**One naming hazard.** M04B's own slice B4 (FDAE) left `D-B4-1` in a code
comment at
[orchestration.rs:713](../../../../crates/control_plane/src/service/orchestration.rs#L713).
This document's `D-B4-N` ids are a *different* B4. That collision is
harmless only because the project rule forbids planning-doc ids in code at
all — so none of the ids below may appear in any comment, doc comment or
test name this slice writes.

---

## §0 — What the input documents leave open, understate, or state wrongly

### 0.1 (Scope-changing, blocking) "Both fit in the queue crate as a helper" is wrong about the reverse walk, and the number is 5 seconds again

ADR-0023 §7: *"What a saga needs from the platform is a durable log of
completed steps and a rule for walking it backwards. Both fit in the queue
crate as a helper."*

The log fits. The walk does not, and the reason is the same finding that
deferred B5 out of this milestone. A compensating walk of N steps is N
**remote** calls; `DEFAULT_PROXY_CALL_TIMEOUT` is 30 s and the outbox worker
already treats a single connect as costing seconds. A guest that calls
`compensate` and waits for the walk is a guest inside
`dispatch_epoch_timeout_secs` — **5 seconds**
([config.rs:446](../../../../crates/core/src/config.rs#L446)), "tight by
design". B2 hit exactly this and it is recorded in [status.md](status.md):
`enqueue`'s *single* synchronous probe blew the 5 s budget and **trapped the
guest**, and had to be given its own 2 s bound. A walk of two steps against
one unreachable provider is that failure multiplied.

So the reverse walk is **not** an inline helper the guest calls and waits
for. It is:

- a state transition the guest (or a deadline) asks for, which returns
  immediately — the same "accepted, not done" contract `enqueue` already
  has; plus
- a **worker** that performs one undo per saga per tick, in reverse index
  order, with `RetryPolicy` backoff between attempts and a terminal state an
  operator can see.

That is more machinery than "a helper", and this section exists so the size
is not discovered mid-slice. It is still **not** an engine (§0.3).

### 0.2 (Stale) The deploy-time check's named precedent is a test that parses a `.wit` file; the deploy path has no `.wit` file

The milestone plan §0.6 says the deploy path verifies the `undo-`
counterpart *"the same class of check `test_wit_adherence` already performs
over a parsed interface's function list."*

`test_wit_adherence`
([control_plane/src/service.rs:821](../../../../crates/control_plane/src/service.rs#L821))
is a `#[cfg(test)]` function that reads `wit/control-plane/control-plane.wit`
off disk with `wit_parser`, a dev-dependency. A deployed guest component is
a `.wasm` artifact; its `.wit` source is not shipped, not fetched, and not
parseable at deploy.

**The real precedent is one file away and does exactly the right thing.**
`AppSandboxEngine::exports_authorize_rows`
([engine.rs:600](../../../../crates/sandbox_wasm/src/engine.rs#L600))
inspects the *compiled* component's static type — `component_type()`,
`get_export(interface)`, `get_export(function)` — with no instantiation, and
`deploy_wasm_service`
([orchestration.rs:713-733](../../../../crates/control_plane/src/service/orchestration.rs#L713))
calls it after `deploy_wasm` and rolls the deploy back on a miss. B4
generalises that one method and reuses that one call site. Nothing about
`wit_parser` is involved.

Two consequences the milestone plan does not draw:

1. The check runs **on the substrate**, against the compiled component and
   nothing else. §0.4 shows that this is enough on its own, and that the
   manifest declaration the milestone plan assumes is not needed at all.
2. It covers the supervisor's `submit`/`force-reconcile` path for free —
   both end at the same substrate `deploy` verb. B3 needed a second
   enforcement site (`refuse_unrunnable_schedules`) because its cap lived in
   `validate()` only; B4 does not, and this is worth stating so nobody adds
   one "for symmetry".

### 0.3 (Sets the boundary) The non-goal says "no saga engine", and B4 has a worker — the line is *who decides what runs next*

[task.md](task.md)'s non-goals: *"No workflow DSL or saga engine. A naming
convention (`undo-<operation>`), a durable step log, and a helper that walks
it backwards."* The architecture doc's own opening line for `[PLT-ASY]` is
the sharper statement: the substrate handles this *"by delegating explicit
workflow management to the business logic"*.

The line B4 draws, stated as three refusals:

- **The platform never decides the forward path.** There is no DSL, no
  declared step list, no "next step" rule. The guest calls `step` when its
  own logic reaches a step. A failing step is returned to the guest, which
  decides whether to retry it, take a different provider, or give up.
- **The platform never decides a workflow failed.** Only two things start a
  compensation: the guest asking for one, and a **deadline** the guest
  itself declared expiring (which is the crash case — a guest is disposable
  and does not exist between calls, so nothing else can notice).
- **The platform's whole contribution to the backward path is order plus
  delivery**: newest step first, one at a time, each undo fenced by an
  idempotency key, retried on its own backoff, terminal state visible to an
  operator.

A worker that walks a list backwards is not an engine by any of the three
tests above. Recorded because "we shipped a worker" reads like a non-goal
violation to a reader who does not have this paragraph.

### 0.4 (Scope-changing, decided with the requester 2026-08-07) There is no manifest declaration, and the marker prefix is `saga-undo-` rather than `undo-`

Two connected decisions, taken before drafting phase 1, that move this slice
away from what the milestone plan §0.6 and ADR-0023 §7 describe. Both need
an amendment sentence in those documents (§5).

**(a) No declaration. Not participating is the default, and it is the common
case.** The milestone plan's model is that a service "participating in a
saga declares it, and the deploy path verifies the `undo-` counterpart
exists". Carrying that declaration to the substrate means a new field on the
app-model `ServiceConfig` *and* on the wire `wasm-manifest` — and a Rust
struct literal must list every field, so that is roughly **90 mechanical
edits across ~40 files** (106 `ServiceConfig {` sites, ~20
`WasmManifest {` sites), the price of adding *any* field there whatever its
shape.

What it buys is narrower than it first looks. Nothing at runtime reads the
declaration: the walk calls the compensation named by the step it recorded,
never a manifest. So the declaration catches exactly one thing — an author
who *intends* their service to be compensable and forgets to write the
compensation. Most services will have no compensation at all, and that is
not an error but the ordinary case: an idempotent or read-only operation
needs nothing undone. Absence therefore has to mean "takes part in no saga"
regardless of whether a declaration exists, which leaves the declaration
protecting one author mistake, at the cost of the largest diff in the slice,
in a design where the *caller* is usually a different developer on a
different node who cannot read the participant's manifest anyway and must
handle a failed compensation regardless.

Without it, that mistake is still not silent — only later: the walk calls a
compensation the target does not export, the target answers "method not
found", the saga reaches `failed`, and `roymctl svc sagas` shows it with an
error message that names the convention (§0.13's error-text requirement).

Adding the declaration later costs the same ~90 edits and, pre-release, no
compatibility work at all. Recorded as a backlog row with a pickup trigger
rather than built now (§5).

**(b) The marker is `saga-undo-<method>`, because `undo-` is an ordinary
word.** This is what makes (a) safe, and it is a genuine defect in the
convention as the requirements spec writes it. `undo-last-update` is a
perfectly plausible *business* verb — a service may legitimately export it
with no relation to any saga. Two things go wrong with a bare `undo-`
prefix:

- The export-derived check ("every `undo-x` must have an `x`") would refuse
  a valid deploy whose `undo-last-update` has no `last-update` beside it. A
  check that punishes a legal name is worse than no check.
- Worse in the other direction: a saga step whose method happens to pair
  with such a verb would have the platform call a *business* function
  believing it to be a compensation, with the forward params merged in
  (§0.7). Extra JSON members are ignored by the binder, so the call may well
  succeed and do the wrong thing quietly.

`saga-undo-` cannot collide: "saga" is a platform word, and no domain
interface has a reason to start a function with it. `compensate-` was
considered and rejected — in a product with a mutual-credit ledger,
`compensate-payment` is a business verb waiting to happen. So the convention
this slice implements is:

```
forward:      reserve
compensation: saga-undo-reserve
```

and the deploy gate is one rule, sound because the prefix is reserved:
**every `saga-undo-<x>` export must have an `<x>` on the same interface.**
~40 lines, no manifest change, no wire change, and it cannot false-refuse.

Confirmed against the tree: no WIT file and no test component today exports
any function beginning `undo-`, so nothing existing breaks either way.

### 0.5 (Correctness) A step recorded only after its call returns is a step that can be lost exactly when it matters

The obvious implementation records a step once the forward call succeeds —
"a durable log of *completed* steps", as ADR-0023 §7 words it. That loses
the ambiguous case, and the ambiguous case is the one a saga exists for: the
call left this node, the target ran it, and the substrate died before the
result came back. On recovery the log says the step never happened, so the
walk skips it, and the workflow is left half-applied — which is the exact
state compensations exist to prevent.

**Rule: write the intent before the call, write the outcome after.** A step
row is created in state `pending` before dispatch and moved to `done` (with
the result) or `failed` (with the error) after. The walk compensates `done`
**and** `pending` steps, and skips `failed` ones.

This makes the convention's contract slightly stronger than the spec's, and
it must be documented on the WIT and in the developer guide: **an
`saga-undo-<op>` may be called for an operation that never happened.** It is the
same property at-least-once delivery already demands of every queued call
(D-B-3), one level up, and it is free for any undo written as "ensure this
is not in effect" rather than "reverse this".

### 0.6 (Correctness, and it decides a table column) A queued (`enqueue`d) call cannot be a saga step

The architecture doc says compensations fire "when a multi-step operation
**or queued task** hits a terminal failure". The second half is not
buildable here and must be scoped out explicitly rather than half-attempted:

`enqueue` returns "accepted for delivery", never "delivered" — that is its
whole contract, and the guest is gone by the time the outbox worker
succeeds or dead-letters. A step whose completion is unknown cannot be
recorded as `done` and cannot be sensibly undone. So **a saga step is
always a synchronous call**; `step` is `call` plus a log write, never
`enqueue` plus one.

The reverse direction — a *dead-lettered* queued call triggering a saga
compensation — is a genuine and useful feature and is out of scope: it
needs the outbox worker to know about sagas (a coupling neither B2 nor this
slice has) and it needs an answer to "which saga", which nothing records.
Backlog row (§5), not a TODO in code.

### 0.7 (Correctness) The undo needs the forward call's result, and the architecture doc calls it something that does not exist here

The requirements spec: *"the compensating `undo` functions generally accept
the identical arguments as the original forward operation (along with the
generated resource ID)"*. There is no "generated resource ID" concept
anywhere in this tree. The nearest real thing — and what the sentence is
plainly reaching for — is **the forward call's own return value**, which is
where a created resource's id would in fact be.

The binding rule falls out of `json_to_wasm_params`
([conversions.rs:344](../../../../crates/sandbox_wasm/src/conversions.rs#L344)),
which binds a JSON **object** by parameter name, an **array** positionally,
and ignores extra members/elements in both cases:

- forward params were an object → send the same object plus a
  `"forward-result"` member;
- forward params were an array → send the same array with the result
  appended as its last element;
- forward params were `null`/absent → send `{"forward-result": ...}`.

A forward call that returned nothing sends no `forward-result` at all, which
binds to `none` for an `option<string>` parameter. So the convention's full
shape is:

```wit
// forward
reserve: func(item: string) -> result<string, string>;
// its compensation -- same parameters, plus one optional trailing member
saga-undo-reserve: func(item: string, forward-result: option<string>) -> result<_, string>;
```

An undo that does not declare `forward-result` still works (the extra member
is ignored), which is why the deploy gate checks **existence only** and not
arity. Stated here because "check the signature matches" is the obvious
review comment and it would be wrong.

### 0.8 (Interaction with B2, sharp) A keyed forward step that fails writes a proxy dead letter an operator can replay *outside* the saga

`ProxyRouter::invoke` calls `record_failed_call` for every failed call
carrying an idempotency key
([proxy.rs:316](../../../../crates/router/src/proxy.rs#L316)). A saga step
is an ordinary `invoke`, so a *keyed* step that fails leaves a replayable
row in the service's proxy DLQ. `proxy-replay` re-enqueues it; the outbox
worker then delivers the **forward** operation — possibly after the saga it
belonged to was already compensated.

This is not a defect introduced by B4; it is B2's operator surface meeting
B4's new caller. Three options were weighed: refuse a key on a step
(surprising, and a keyed step is *safer* against a retry), strip it
silently (never), or accept and document. **Accept and document**, plus a
backlog row, because the failure needs an operator to actively replay a row
whose error text names a saga step, and closing it properly means teaching
the DLQ about sagas — which is §0.6's coupling again.

### 0.9 (Sets the store's shape) This is the third copy of "open this service's `async.db` with its DEK, once"

`ProxyOutbox::queue_for`
([proxy_outbox.rs:184](../../../../crates/router/src/proxy_outbox.rs#L184))
and `CallDedupGuard::store_for`
([call_dedup.rs:265](../../../../crates/router/src/call_dedup.rs#L265)) are
already the same twelve lines twice: cache lookup, single-flight lock,
re-check, `load_service_dek`, `service_db_dir`, `spawn_blocking` open,
publish. B4's saga log is the same file (`ASYNC_DB_NAME`), the same key, and
the same hazard the B2 review's F1 fixed — two handles to one file are two
connections.

A third verbatim copy is the wrong answer and a full refactor of the two
existing owners is out of scope. **Middle path:** extract only the part that
is genuinely identical and stateless —

```rust
// crates/router/src/service_async_db.rs
/// Resolves `(directory, dek)` for one service's async database. The three
/// per-service stores in this crate all need exactly this and nothing else;
/// the caching and single-flighting stay with each owner, because what they
/// cache differs.
pub(crate) async fn async_db_location(
    storage_provider: &Arc<dyn StorageProvider>,
    key_store: &Arc<KeyStore>,
    service_id: &str,
) -> Result<(PathBuf, Option<Zeroizing<[u8; 32]>>), ProxyError>;
```

— and have `SagaStore` use it from the start, with `ProxyOutbox` and
`CallDedupGuard` moved onto it in the same phase (both keep their own cache,
lock and refusal rules; only the four middle lines move). That is a net
deletion, not a refactor with a blast radius.

### 0.10 (Bound) A saga has three unbounded dimensions and B1's failure-matrix row 12 applies to all of them

Row 12 says queue growth is bounded and *asserted*. A saga log has three
counters, and the right answer differs per counter — the same split B2's F9
had to make between refusing and evicting:

| Dimension | Bound | Refuse or evict |
|---|---|---|
| Open sagas per service | `saga_max_open` (64) | **Refuse** `begin`. An open saga is work somebody expects to finish |
| Steps per saga | `saga_max_steps` (64) | **Refuse** `step`. Same reason, and a 65-step workflow is a design question |
| Terminal (`compensated`/`failed`) rows | `saga_max_terminal_rows` (1000) | **Evict** oldest-first, exactly as `dead_letters` does |
| One step's stored params + result | `MAX_SAGA_PAYLOAD_BYTES` (256 KiB, the same constant B2 chose for a queued call) | **Refuse** the step |

Committed sagas are **deleted**, rows and steps, not tombstoned — D-B1-9's
rule, for the same reason (a committed saga can never be walked again, so
nothing reads the row).

### 0.11 (Correctness) `attempts` must be incremented before the undo is dispatched, not after

B1's post-landing review finding 7 is the precedent: `attempts` advanced
only inside `fail`, so a worker that panicked or was killed mid-delivery
left the counter untouched and the item was reclaimed forever. B1's fix was
a second counter (`claim_count`) because its queue could not restructure the
write order.

A saga has no claim (§0.12), so it can just do the right thing: **increment
before dispatch.** A crash mid-undo therefore costs an attempt, which is
what bounds the poison pill; a re-dispatch after that crash is safe because
every undo carries an idempotency key and B2's receiver-side fence answers
the duplicate from the first call's record. This is the first place in the
tree where B2's fence is load-bearing for something other than B2, and it is
worth saying so: without it, "increment before dispatch" would be trading a
double execution for a bounded loop.

### 0.12 (Narrows the slice) There is no claim, no visibility timeout, and no second worker

Three things B1/B2 needed that B4 does not:

- **No claim.** A queue is claimed because items are independent and a tick
  may process many concurrently. A saga's compensation is strictly
  sequential — one undo at a time, newest index first — and one process owns
  the file. `next_attempt_at` on the saga row is the whole scheduler.
- **No visibility timeout.** It exists to release an item a crashed worker
  held. Nothing is held: a crashed undo left `attempts` already incremented
  (§0.11) and the step still uncompensated, so the next tick simply retries
  it.
- **No second worker task.** The sweep runs inside the existing outbox
  worker loop, which already has the tick, the cancellation token, the
  shutdown rule (cancel, do not drain) and the `Option`-gated no-op when the
  node has no per-service storage. Adding a second `tokio::spawn` in
  `runtime.rs` would duplicate all four for no benefit.

### 0.13 Documents that are ambiguous or stale against the current code

| Document | Problem | This plan's answer |
|---|---|---|
| ADR-0023 §7 | "Both fit in the queue crate as a helper" | §0.1. The log fits; the walk needs a worker |
| implementation-plan.md §0.6 | Names `test_wit_adherence` as the precedent for the deploy check | §0.2. The precedent is `exports_authorize_rows` |
| implementation-plan.md §0.6, ADR-0023 §7 | Both describe a service *declaring* saga participation in its manifest, with the deploy path checking the declaration | §0.4(a). No declaration is built: absence already means "no saga", and the check is derived from the component's own exports. Both documents get an amendment sentence (§5) |
| system-requirements-spec.md, ADR-0023 §7 | Fix the marker as `undo_<operation>` / `undo-<operation>` | §0.4(b). `undo-` is an ordinary business word (`undo-last-update`), so the marker is `saga-undo-`. Amendment sentence in the same pass |
| task.md *Migration impact* | Lists B2's WIT change and B3's manifest change; says nothing about B4 | §5 records the correction, in the shape exit criteria 13 already uses for B2. After §0.4, B4's only migration cost is one additive WIT interface — no manifest change and no wire change |
| task.md *Failure and security matrix* | Has no saga-specific row. B4 inherits rows 2, 12, 13, 14 for its own tables and owns nothing uniquely | §3 states the four inherited rows and adds three B4-local ones **to this document**, not to the milestone matrix |
| system-architecture.md `[PLT-ASY]`, system-requirements-spec.md `[PLT-ASY]` | `undo_<operation>` (underscore); "the generated resource ID"; compensations firing from a queued task | ADR-0023 §7 already corrects the spelling. §0.7 answers the resource id; §0.6 scopes out the queued-task trigger. All three land in the closeout doc note this slice owes anyway |
| task.md exit criterion 6 | "⬜ **Milestone closeout (needs B4)**" | This slice does the closeout doc work (§5), so criterion 6 flips here or not at all |

---

## §1 — Decisions

| ID | Decision |
|---|---|
| **D-B4-1** | **A saga is guest-driven forward, platform-driven backward.** The guest calls `begin`/`step`/`commit`/`compensate`; the platform never chooses a forward step and never declares a workflow failed except by the guest's own declared deadline (§0.3) |
| **D-B4-2** | **`compensate` returns immediately; a worker performs the walk.** One undo per saga per tick, strictly descending step index, `RetryPolicy` backoff between attempts (§0.1). The guest learns the outcome by calling `status`, not by waiting |
| **D-B4-3** | **Every undo carries a host-minted idempotency key, `saga:<saga-id>:<idx>`.** The guest cannot set or see it. This is what makes "increment attempts before dispatch" (§0.11) safe, and it reuses B2's receiver-side fence unchanged. The fence's TTL is already derived from the same `queue_*` retry window the compensation backoff uses, so the two cannot drift |
| **D-B4-4** | **Intent is written before the call, outcome after; the walk compensates `done` and `pending` steps** (§0.5). The convention's documented contract is therefore "an undo may be called for an operation that never happened" |
| **D-B4-5** | **A saga step is a synchronous call. `enqueue` can never be a step** (§0.6) |
| **D-B4-6** | **The undo receives the forward params plus the forward result**, merged by §0.7's three-case rule. The deploy gate checks **existence only**, never arity or types |
| **D-B4-7** | **The marker is `saga-undo-<method>`, and it is a reserved prefix** (§0.4b). `undo-` alone is an ordinary business word, which would make both the deploy check and the walk itself ambiguous. One constant, `SAGA_UNDO_PREFIX`, used by the gate and the walk so the two can never spell it differently |
| **D-B4-8** | **No manifest declaration; the deploy gate is derived from the component's own exports** (§0.4a): every `saga-undo-<x>` export must have an `<x>` on the same interface. Sound only because of D-B4-7. One generalisation of `exports_authorize_rows` and one new block in `deploy_wasm_service` (§0.2), rolling back exactly as the stage-4 gate does. **No `ServiceConfig` field, no wire change, no ~90-site sweep** — the declaration is a backlog row with a pickup trigger, not a silent omission |
| **D-B4-9** | **The step log lives in `syneroym-async-queue` (`saga.rs`), in the service's own `async.db`**, beside `outbox`/`dead_letters`/`call_dedup` — one more table pair in the file that already holds this service's async state, under the same DEK (ADR-0023 §4, failure-matrix row 13) |
| **D-B4-10** | **No claim, no visibility timeout, no second worker task** (§0.12). The sweep runs inside `run_outbox_worker`, which is renamed `run_async_worker` |
| **D-B4-11** | **Five new `AppSandboxRole` fields**, and no new retry policy (D-B-5): `saga_max_open`, `saga_max_steps`, `saga_max_terminal_rows`, `saga_default_deadline_secs`, `saga_max_deadline_secs`. The compensation backoff and attempt budget are the existing `queue_max_attempts`/`queue_max_backoff_secs` — an undo is a delivery, and a second budget would only disagree with the first |
| **D-B4-12** | **`ServiceProxy` gains five methods, each with a refusing default body** — the shape B2 used for `enqueue` and B3 for `SubstrateActor::run_scheduled`, so no existing fake grows a method it never calls |
| **D-B4-13** | **An operator surface ships in this slice**: `sagas` (read gate) and `saga-compensate` (write gate) on the `orchestrator` interface, `roymctl svc sagas|saga-compensate`, and the e2e asserts through them. B1's review finding 13 and B3's D-B3-11 both say why |
| **D-B4-14** | **The e2e drives a countable participant.** One new fixture component, `test-components/saga-test`, deployed twice: once as the orchestrating driver and once as the participant that records what was reserved and what was undone, in that order |
| **D-B4-15** | **`saga-compensate` (operator) re-arms; it never walks inline.** A `failed` saga returns to `compensating` with the current step's attempts reset. Same rule and same reason as `replay` (ADR-0023 §5) |

---

## §2 — Phases

Merge order is phase order. Each phase compiles and its own tests pass on
its own. Branch: `feat/m05b-b4-saga-compensations`.

Phase 1 is small and touches no existing struct (§0.4). It is independent
of phases 2-4, so it can land first or last; first is better, because it
fixes the one constant every later phase spells.

### Phase 1 — the convention and its deploy-time gate

**New file** `crates/app_orchestration/src/saga.rs`, re-exported from
`lib.rs` beside the `schedule` re-exports:

```rust
//! The saga compensation convention (ADR-0023 §7, as amended -- see this
//! slice's plan §0.4). A service that can undo an operation exports a
//! second function beside it, named by prefixing the forward operation.

/// The reserved prefix a compensation is named with. Reserved is the
/// operative word: a plain `undo-` is an ordinary business verb
/// (`undo-last-update` is a perfectly good API), so a bare `undo-` marker
/// would make both the deploy check and the backward walk ambiguous --
/// the walk could call a business function believing it to be a
/// compensation. Nothing in a domain interface begins with `saga-`.
///
/// One constant, read by the deploy gate and by the walk, so the two can
/// never spell it differently.
pub const SAGA_UNDO_PREFIX: &str = "saga-undo-";

/// The compensation's name for a forward operation.
#[must_use]
pub fn saga_undo_name(method: &str) -> String { format!("{SAGA_UNDO_PREFIX}{method}") }

/// The forward operation a compensation names, or `None` when `function`
/// is not a compensation at all.
#[must_use]
pub fn compensated_operation(function: &str) -> Option<&str> {
    function.strip_prefix(SAGA_UNDO_PREFIX)
}
```

There is **no manifest type and no wire change** in this slice (§0.4a):
nothing declares participation, and nothing needs to.

**Engine**, `crates/sandbox_wasm/src/engine.rs` — generalise, keeping
`exports_authorize_rows` as a one-line caller so its two existing call sites
and the `dummy_sandbox` mirror do not move:

```rust
    /// Whether `service_id`'s compiled component exports `function` on
    /// `interface`. Cheap: reads the cached `InstancePre`'s static component
    /// type, no instantiation. `interface` is matched exactly, which is what
    /// dispatch itself does (`get_wasm_func`), so a name that passes here is
    /// a name a call can reach.
    #[must_use]
    pub fn exports_function(&self, service_id: &str, interface: &str, function: &str) -> bool

    /// Every function `interface` exports, or `None` when the component
    /// does not export that interface at all. Backs the deploy gate: a
    /// `saga-undo-x` with no `x` beside it.
    #[must_use]
    pub fn exported_functions(&self, service_id: &str, interface: &str) -> Option<Vec<String>>

    #[must_use]
    pub fn exports_authorize_rows(&self, service_id: &str) -> bool {
        self.exports_function(service_id, Self::AUTHORIZER_INTERFACE, "authorize-rows")
    }
```

`crates/control_plane/src/dummy_sandbox.rs` gains the two mirrors returning
`false`/`None` with the same "wasm feature off" doc comment its neighbour
carries.

**The gate**, `deploy_wasm_service`
([orchestration.rs:733](../../../../crates/control_plane/src/service/orchestration.rs#L733)),
in a new block immediately after the stage-4 one and before
`register_wasm_endpoints`:

```
// One rule, over the interfaces the manifest already declares. Sound only
// because `saga-undo-` is reserved: a component that exports it is
// unambiguously claiming a compensation, so a missing counterpart is a
// defect and never a legal business name (§0.4b).
for iface in &wasm_manifest.interfaces:
    for f in engine.exported_functions(service_id, iface).unwrap_or_default():
        if let Some(forward) = compensated_operation(&f):
            if !engine.exports_function(service_id, iface, forward):
                rollback; refuse "component exports '<f>' on '<i>' but no
                                  '<forward>' beside it: a saga compensation
                                  must name an operation this component
                                  actually has"
```

Rollback is `rollback_config_generation` + `rollback_fdae_policy`, copied
from the block above it verbatim — it is two calls, and factoring them out
is a separate change.

`SAGA_UNDO_PREFIX`, `saga_undo_name` and `compensated_operation` are
re-exported from `syneroym-app-orchestration`, which both `control_plane`
and `router` already depend on.

**No sweep, no manifest type, no wire field** (§0.4a). Phase 1's whole diff
is one new file, two engine methods, two `dummy_sandbox` mirrors, and one
block in `deploy_wasm_service`.

**What this deliberately does not catch, and where it surfaces instead.** A
service that means to be compensable and exports no compensation at all
deploys cleanly. The mistake appears when a saga tries to unwind: the walk
calls `saga-undo-<method>`, the target answers "method not found", the saga
reaches `failed` and is listed by `roymctl svc sagas`. **The error text is
therefore part of the contract, not a nicety** — when an undo fails with a
not-found-shaped callee error, the recorded `last_error` must name the
convention: *"target does not export `saga-undo-<m>` on `<i>`; a saga
participant must export `saga-undo-<method>` for every operation a step
calls"*. Phase 4 owns that message and tests it.

**Phase 1 tests** (`syneroym-app-orchestration`, then
`syneroym-control-plane`):

- `saga_undo_name_and_compensated_operation_round_trip`
- `compensated_operation_ignores_a_plain_undo_prefixed_function` — the
  §0.4b case: `undo-last-update` is a business verb, not a compensation
- `deploy_refuses_a_component_whose_compensation_has_no_forward_operation`
- `deploy_accepts_a_component_exporting_both_halves`
- `deploy_accepts_a_component_exporting_a_plain_undo_prefixed_function`
  (the false-refusal the reserved prefix exists to prevent — this is the
  test that would fail under a bare `undo-` marker)
- `deploy_accepts_a_component_with_no_compensations_at_all` (the common
  case, §0.4a)
- `a_refused_compensation_pairing_rolls_back_the_config_generation`

### Phase 2 — the durable step log (`syneroym-async-queue`)

**New file** `crates/async_queue/src/saga.rs`, re-exported from `lib.rs`
beside the `dedup` re-exports. It shares `open_connection` (already private
to the crate) and the crate's Unix-milliseconds convention.

**Schema** (`init_schema`, unconditional `IF NOT EXISTS`, same reasoning
comment as `Queue::init_schema`):

```sql
CREATE TABLE IF NOT EXISTS sagas (
    saga_id         TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    app_instance_id TEXT,
    state           TEXT NOT NULL,   -- open|compensating|compensated|failed
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    deadline_at     INTEGER NOT NULL,
    next_attempt_at INTEGER,         -- compensating only; NULL otherwise
    last_error      TEXT
);
CREATE INDEX IF NOT EXISTS idx_sagas_due      ON sagas(state, next_attempt_at);
CREATE INDEX IF NOT EXISTS idx_sagas_deadline ON sagas(state, deadline_at);
CREATE INDEX IF NOT EXISTS idx_sagas_updated  ON sagas(state, updated_at);

CREATE TABLE IF NOT EXISTS saga_steps (
    saga_id     TEXT NOT NULL,
    idx         INTEGER NOT NULL,
    target      TEXT NOT NULL,       -- JSON QueuedTarget
    routing_key TEXT,
    interface   TEXT NOT NULL,
    method      TEXT NOT NULL,
    params      BLOB NOT NULL,       -- JSON
    result      BLOB,                -- JSON, NULL until done
    state       TEXT NOT NULL,       -- pending|done|failed|compensated
    attempts    INTEGER NOT NULL DEFAULT 0,   -- compensation attempts
    last_error  TEXT,
    created_at  INTEGER NOT NULL,
    PRIMARY KEY (saga_id, idx)
);
```

`saga_steps` has no `ON DELETE CASCADE`: this crate does not enable
`PRAGMA foreign_keys`, and every delete path here already deletes both
tables inside one transaction.

**Types and API:**

```rust
pub struct SagaConfig {
    pub retry: RetryPolicy,           // reused, per D-B-5 -- no second policy
    pub max_open: u32,
    pub max_steps: u32,
    pub max_terminal_rows: u32,
    pub max_payload_bytes: usize,
    pub default_deadline_ms: i64,
    pub max_deadline_ms: i64,
}
impl From<&AppSandboxRole> for SagaConfig { /* clamps each field to >= 1 */ }

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum SagaState { Open, Compensating, Compensated, Failed }

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StepState { Pending, Done, Failed, Compensated }

pub struct StepIntent {           // what `record_step_intent` stores
    pub target: String,           // JSON QueuedTarget, opaque to this crate
    pub routing_key: Option<String>,
    pub interface: String,
    pub method: String,
    pub params: Vec<u8>,
}

pub struct StepRow {              // what the walk reads back
    pub idx: u32,
    pub target: String,
    pub routing_key: Option<String>,
    pub interface: String,
    pub method: String,
    pub params: Vec<u8>,
    pub result: Option<Vec<u8>>,
    pub attempts: u32,
}

pub struct SagaInfo {             // the operator/guest listing shape
    pub saga_id: String,
    pub name: String,
    pub state: SagaState,
    pub steps: u32,
    pub compensated_steps: u32,
    pub created_at: i64,
    pub deadline_at: i64,
    pub last_error: Option<String>,
}

pub struct SagaHead { pub saga_id: String, pub app_instance_id: Option<String> }

/// What a failed undo attempt means for the saga.
pub enum CompensationOutcome { Retry { next_attempt_at: i64 }, Failed }

pub struct SagaLog { conn: Arc<Mutex<Connection>>, config: SagaConfig }

impl SagaLog {
    pub fn open_encrypted<P: AsRef<Path>>(dir: P, db_name: &str,
        dek: Option<&[u8; 32]>, config: SagaConfig) -> Result<Self>;
    pub fn open_in_memory(config: SagaConfig) -> Result<Self>;
    pub fn from_connection(conn: Arc<Mutex<Connection>>, config: SagaConfig) -> Result<Self>;

    pub fn begin(&self, saga_id: &str, name: &str, app_instance_id: Option<&str>,
                 deadline_ms: i64, now: i64) -> Result<()>;
    pub fn record_step_intent(&self, saga_id: &str, intent: &StepIntent, now: i64) -> Result<u32>;
    pub fn record_step_outcome(&self, saga_id: &str, idx: u32,
                               result: Option<&[u8]>, error: Option<&str>, now: i64) -> Result<()>;
    pub fn commit(&self, saga_id: &str) -> Result<()>;
    pub fn mark_compensating(&self, saga_id: &str, now: i64) -> Result<bool>;

    pub fn due_compensations(&self, now: i64, limit: u32) -> Result<Vec<SagaHead>>;
    pub fn abandoned(&self, now: i64, limit: u32) -> Result<Vec<SagaHead>>;
    pub fn next_uncompensated_step(&self, saga_id: &str) -> Result<Option<StepRow>>;
    pub fn begin_undo_attempt(&self, saga_id: &str, idx: u32, now: i64) -> Result<u32>;
    pub fn mark_step_compensated(&self, saga_id: &str, idx: u32, now: i64) -> Result<()>;
    pub fn fail_compensation(&self, saga_id: &str, idx: u32, now: i64, error: &str,
                             terminal: bool) -> Result<CompensationOutcome>;
    pub fn finish_compensation(&self, saga_id: &str, now: i64) -> Result<()>;

    pub fn status(&self, saga_id: &str) -> Result<Option<SagaInfo>>;
    pub fn list(&self) -> Result<Vec<SagaInfo>>;
    pub fn rearm(&self, saga_id: &str, now: i64) -> Result<bool>;
    pub fn open_count(&self) -> Result<u32>;
}
```

**Non-trivial bodies, as pseudo-code.**

`begin` — one `BEGIN IMMEDIATE`, because the open-count check and the insert
must not race two concurrent guest calls (B2 review F1's lesson):

```
tx = BEGIN IMMEDIATE
n = SELECT COUNT(*) FROM sagas WHERE state = 'open'
if n >= config.max_open: return Err("this service already has N open sagas
                                     (the limit is M); commit or compensate
                                     one before starting another")
INSERT INTO sagas VALUES (saga_id, name, app_instance_id, 'open',
                          now, now, deadline_ms, NULL, NULL)
tx.commit()
```

`record_step_intent` — the state check and the index allocation are the same
transaction, so a `compensate` landing between them cannot add a step to a
saga already walking backwards:

```
if intent.params.len() > config.max_payload_bytes: return Err(...)
tx = BEGIN IMMEDIATE
state = SELECT state FROM sagas WHERE saga_id = ?      -- None => Err("unknown saga")
if state != 'open': return Err("saga <id> is <state>; steps may only be
                                added while it is open")
idx = SELECT COALESCE(MAX(idx) + 1, 0) FROM saga_steps WHERE saga_id = ?
if idx >= config.max_steps: return Err(...)
INSERT INTO saga_steps (..., state='pending', attempts=0, created_at=now)
UPDATE sagas SET updated_at = now WHERE saga_id = ?
tx.commit(); return idx
```

`record_step_outcome` — `result` present means `done`, `error` present means
`failed`; both `None` is a caller bug and returns `Err`. Also bounds the
stored result by `max_payload_bytes`, truncating to `NULL` with a
`last_error`-style note rather than refusing: the call already happened, and
refusing here would lose the step entirely.

`due_compensations`:

```
SELECT saga_id, app_instance_id FROM sagas
 WHERE state = 'compensating' AND next_attempt_at <= ?now
 ORDER BY next_attempt_at LIMIT ?limit
```

`abandoned` — the crash case (§0.3):

```
SELECT saga_id, app_instance_id FROM sagas
 WHERE state = 'open' AND deadline_at <= ?now
 ORDER BY deadline_at LIMIT ?limit
```

`next_uncompensated_step` — **descending** index, and `pending` counts
(§0.5):

```
SELECT ... FROM saga_steps
 WHERE saga_id = ? AND state IN ('done','pending')
 ORDER BY idx DESC LIMIT 1
```

`fail_compensation`:

```
tx = BEGIN IMMEDIATE
attempts = SELECT attempts FROM saga_steps WHERE saga_id=? AND idx=?
UPDATE saga_steps SET last_error = ?error WHERE saga_id=? AND idx=?
if terminal or attempts >= config.retry.max_attempts:
    UPDATE sagas SET state='failed', next_attempt_at=NULL,
                     last_error=?error, updated_at=?now
    prune_terminal(tx, config.max_terminal_rows)
    outcome = Failed
else:
    wait = backoff_before_wait(&config.retry, attempts)   // the crate's own helper
    next  = now + wait
    UPDATE sagas SET next_attempt_at=?next, last_error=?error, updated_at=?now
    outcome = Retry { next_attempt_at: next }
tx.commit()
```

`finish_compensation` — everything is compensated:

```
tx = BEGIN IMMEDIATE
DELETE FROM saga_steps WHERE saga_id = ?
UPDATE sagas SET state='compensated', next_attempt_at=NULL, updated_at=?now
prune_terminal(tx, config.max_terminal_rows)
tx.commit()
```

`prune_terminal` — oldest-first over `state IN ('compensated','failed')` by
`updated_at`, deleting the excess above `max_terminal_rows`, mirroring
`Queue::prune_dead_letters`. Returns the pruned ids so a caller *could* log
them; unlike B1's dead letters there is no alert to clear, so nothing is
required to consume it.

`commit` deletes both rows in one transaction (§0.10) and returns
`Err` when the saga is not `open` — committing a compensating saga would
silently drop the walk.

**Config**, `crates/core/src/config.rs`, on `AppSandboxRole` beside the five
`queue_*` fields, each `#[serde(default = "...")]` with a `const fn`
default in the same style:

| Field | Default | Why this number |
|---|---|---|
| `saga_max_open` | 64 | §0.10. One workflow per open saga; a service with 64 in flight has a design problem, not a capacity one |
| `saga_max_steps` | 64 | §0.10 |
| `saga_max_terminal_rows` | 1000 | the same number `queue_dlq_max_rows` uses, for the same operator-visibility reason |
| `saga_default_deadline_secs` | 3600 | one hour: long enough for a human-paced multi-provider workflow, short enough that a crashed one compensates the same day |
| `saga_max_deadline_secs` | 86400 | the ceiling a guest may ask for. Above it, `begin` refuses rather than clamping — the same choice B3 made for `timeout_ms` (a workflow must not silently run under a deadline it did not ask for) |

**Phase 2 tests** (all in `syneroym-async-queue`, in-memory):

- `a_step_added_to_a_compensating_saga_is_refused`
- `steps_are_indexed_in_the_order_they_were_recorded`
- `the_next_uncompensated_step_is_the_highest_index_first`
- `a_pending_step_is_compensated_like_a_done_one` (§0.5)
- `a_failed_step_is_never_compensated`
- `committing_deletes_the_saga_and_its_steps`
- `committing_a_compensating_saga_is_refused`
- `a_failed_undo_schedules_a_backoff_and_a_terminal_one_fails_the_saga`
- `an_exhausted_attempt_budget_fails_the_saga`
- `abandoned_lists_only_open_sagas_past_their_deadline`
- `due_compensations_ignores_a_saga_whose_next_attempt_is_in_the_future`
- `terminal_rows_are_pruned_oldest_first_at_the_cap`
- `an_over_limit_open_count_refuses_a_new_saga`
- `an_over_limit_step_count_refuses_a_new_step`
- `an_over_sized_params_payload_is_refused`
- `rearm_returns_a_failed_saga_to_compensating_with_attempts_reset`
- `the_configured_deadline_ceiling_refuses_rather_than_clamps`

### Phase 3 — the guest surface

**WIT**, appended to
`crates/wit_interfaces/wit/proxy/proxy.wit` (same package, so `use proxy.{}`
needs no cross-package import):

```wit
/// Saga compensations (ADR-0023 §7). A service that drives a multi-service
/// workflow records each step as it takes it, so that a terminal failure --
/// or this substrate dying mid-workflow -- can be walked backwards, calling
/// `saga-undo-<method>` on each step's own target.
///
/// The platform never chooses a forward step and never decides the workflow
/// failed. It orders the backward walk, delivers each undo under an
/// idempotency key it mints itself, retries on a backoff, and shows an
/// operator what it could not undo.
interface saga {
    use proxy.{call-target, call-options, proxy-error};

    enum saga-state { open, compensating, compensated, failed }

    record saga-status {
        saga-id: string,
        name: string,
        state: saga-state,
        steps: u32,
        compensated-steps: u32,
        created-at: s64,
        deadline-at: s64,
        last-error: option<string>,
    }

    /// Opens a saga and returns its id. `deadline-secs` bounds how long the
    /// workflow may stay open: past it, the substrate compensates the saga
    /// on its own, which is the only way a workflow interrupted by a crash
    /// is ever unwound (a component does not exist between calls). Absent
    /// takes the node's default; above the node's ceiling is refused.
    begin: func(name: string, deadline-secs: option<u64>)
        -> result<string, proxy-error>;

    /// Takes one forward step and records it. Identical to `proxy.call` in
    /// what it sends and what it returns; additionally, the intent is
    /// written to this service's durable log *before* the call and the
    /// outcome after it.
    ///
    /// Because the intent is written first, a step whose result never came
    /// back is compensated too -- so `saga-undo-<method>` must tolerate being
    /// called for an operation that never happened.
    step: func(
        saga: string,
        target: call-target,
        %interface: string,
        method: string,
        params: string,
        options: option<call-options>,
    ) -> result<string, proxy-error>;

    /// The workflow reached its goal. The log is dropped and the saga can
    /// never be compensated afterwards.
    commit: func(saga: string) -> result<_, proxy-error>;

    /// Give up: walk the recorded steps backwards. Returns as soon as the
    /// saga is marked -- success here means "accepted", not "undone". Poll
    /// `status` for the outcome.
    ///
    /// Each undo is sent to the step's own target and interface as
    /// `saga-undo-<method>`, with the forward call's parameters and, when there
    /// was one, its result as a trailing `forward-result`.
    compensate: func(saga: string) -> result<_, proxy-error>;

    status: func(saga: string) -> result<saga-status, proxy-error>;
}
```

`crates/wit_interfaces/wit/host/host.wit`, in `world host-environment`:
`import syneroym:proxy/saga@0.1.0;`.

**Migration note, to be verified rather than assumed.** This is additive in
the direction that does not break guests: a component whose own world does
not import `saga` links exactly as before, because the linker may provide
more than a component imports. B2's rebuild requirement came from changing
the *shape* of `call-options`, which is a record `call` already took. **The
verification is `cargo test -p syneroym-router --test proxy_dispatch`
without rebuilding any `test-components/` artifact** — it must pass. If it
does not, that finding replaces this paragraph.

**Types**, `crates/rpc/src/proxy.rs`, beside `QueuedCall`:

```rust
pub struct SagaBegin {
    pub caller_service_id: String,
    pub app_instance_id: Option<String>,
    pub name: String,
    pub deadline_secs: Option<u64>,
}

/// One forward step, as the host hands it to the proxy. Mirrors
/// `QueuedCall`'s field set for the same reason: the undo it will one day
/// produce has to be rebuildable long after the caller is gone.
pub struct SagaStepRequest {
    pub caller_service_id: String,
    pub app_instance_id: Option<String>,
    pub saga_id: String,
    pub target: QueuedTarget,
    pub routing_key: Option<String>,
    pub interface: String,
    pub method: String,
    pub params: Value,
    pub idempotency_key: Option<String>,
    pub protocol: Option<String>,
    pub timeout_ms: Option<u64>,
}

pub enum SagaState { Open, Compensating, Compensated, Failed }

pub struct SagaInfo { /* saga_id, name, state, steps, compensated_steps,
                        created_at, deadline_at, last_error */ }
```

**Trait**, `ServiceProxy` (`crates/rpc/src/proxy.rs`) — five methods, each
with the same refusing default body `enqueue` already uses ("this proxy has
no durable saga log behind it"):

```rust
    async fn saga_begin(&self, req: SagaBegin) -> Result<String, ProxyError>;
    async fn saga_step(&self, req: SagaStepRequest) -> Result<Value, ProxyError>;
    async fn saga_commit(&self, service_id: &str, saga_id: &str) -> Result<(), ProxyError>;
    async fn saga_compensate(&self, service_id: &str, saga_id: &str) -> Result<(), ProxyError>;
    async fn saga_status(&self, service_id: &str, saga_id: &str) -> Result<SagaInfo, ProxyError>;
```

**Store**, new `crates/router/src/saga.rs`:

```rust
/// One node's saga logs: one per driving service, opened once, in that
/// service's own encrypted database beside its outbox and its fence records.
pub struct SagaStore {
    storage_provider: Arc<dyn StorageProvider>,
    key_store: Arc<KeyStore>,
    resolver: Arc<LogicalResolver>,
    config: SagaConfig,
    logs: Mutex<HashMap<String, SagaLog>>,
    open_lock: tokio::sync::Mutex<()>,
}
```

with `log_for(service_id)` (cache, single-flight, re-check — §0.9's
`async_db_location` helper does the middle), `existing_log_for` (operator
verbs, never creates a file), `open_services`, `log_file_exists`, and
`resolve_step_target(app_instance_id, target, routing_key)` — the same body
as `ProxyOutbox::resolve_target`, taking the pieces separately because a
step is not a `QueuedCall`.

`ProxyRouter` gains `sagas: Option<Arc<SagaStore>>` and `with_sagas(...)`,
built in `route_handler.rs` next to the outbox (line 258-276) from the same
`deps` and `SagaConfig::from(&config.roles.app_sandbox...)`.

**Router methods** (`crates/router/src/proxy.rs`):

```rust
async fn saga_begin_impl(&self, req: SagaBegin) -> Result<String, ProxyError> {
    let store = self.sagas.as_ref().ok_or(Internal("no durable saga log on this node"))?;
    // Same refusal, same reason as enqueue's: every undo this saga may
    // later send is delivered without the caller present, so a service with
    // no unexpired instance certificate would have every one of them
    // refused as anonymous. Failing at `begin` is that answer, given at the
    // only moment a caller can read it.
    require_instance_certificate(&req.caller_service_id)?;
    let deadline = clamp_or_refuse(req.deadline_secs, config)?;   // refuses above the ceiling
    let saga_id = uuid::Uuid::new_v4().to_string();               // host-minted, never guest-chosen
    let log = store.log_for(&req.caller_service_id).await?;
    spawn_blocking(move || log.begin(&saga_id, &name, app_instance_id, deadline, now_ms()))?;
    Ok(saga_id)
}

async fn saga_step_impl(&self, req: SagaStepRequest) -> Result<Value, ProxyError> {
    let log = store.log_for(&req.caller_service_id).await?;
    let target = store.resolve_step_target(&req)?;            // terminal: a name bound to nobody
    let idx = spawn_blocking(|| log.record_step_intent(&saga_id, &intent, now))?;
    let outcome = self.invoke(request_from_step(&req, target)).await;   // note: `invoke`, so a
                                                                       // keyed step still gets
                                                                       // B2's dead-letter row
                                                                       // (§0.8)
    match &outcome {
        Ok(v)  => log.record_step_outcome(&saga_id, idx, Some(json(v)), None, now),
        Err(e) => log.record_step_outcome(&saga_id, idx, None, Some(&e.to_string()), now),
    }
    outcome
}
```

`saga_commit_impl`, `saga_compensate_impl` (`mark_compensating`, then
returns; the worker does the rest), `saga_status_impl` are thin wrappers
over the log, each mapping "unknown saga" to
`ProxyError::Internal("unknown saga <id>")` — deliberately not
`ServiceNotFound`, which the queued classifier reads as retryable.

**Host functions**, `crates/sandbox_wasm/src/host_capabilities.rs`:
`impl saga::Host for HostState`, five functions, each in the shape
`proxy::Host::enqueue` already has — the `read_only` refusal first, then
`service_proxy.upgrade()`, then params parsing, then the call. `step`
follows `enqueue`'s target handling exactly (validate a dependency name,
never resolve it here) and `map_proxy_error` is reused unchanged.
`engine.rs:488` gains `saga::add_to_linker::<_, HasSelf<HostState>>(...)`.

**Phase 3 tests:**

`syneroym-router` (`src/saga.rs` and `src/proxy.rs`):

- `a_step_records_its_intent_before_the_call_lands`
- `a_step_whose_call_fails_is_recorded_as_failed_and_still_returns_its_error`
- `a_step_on_an_unknown_saga_is_refused`
- `begin_is_refused_for_a_service_with_no_unexpired_instance_certificate`
- `begin_is_refused_above_the_deadline_ceiling`
- `a_deadline_of_none_takes_the_node_default`
- `commit_drops_the_log_so_a_later_compensate_is_refused`
- `two_concurrent_begins_through_a_cold_cache_share_one_log_handle` (the
  shape of B2's F1 regression test — a second handle to one file is a second
  connection)
- `a_saga_id_is_minted_by_the_host_and_is_unique_per_begin`

`syneroym-sandbox-wasm` (`host_capabilities.rs`): one test per host
function that the refusal and the pass-through both reach the fake proxy
with the fields the WIT carried, plus
`a_stage_four_after_step_instance_cannot_open_a_saga`.

### Phase 4 — the reverse walk and the operator surface

**The walk** (`crates/router/src/proxy.rs`):

```rust
/// One pass over every saga log this node has open, plus every deployed
/// service whose log file already exists -- the second half is what lets a
/// restart pick up a saga written before it. Mirrors
/// `drain_outboxes_once` exactly, including the undeployed-service rule.
pub async fn sweep_sagas_once(&self) -> usize {
    let Some(store) = &self.sagas else { return 0 };
    for service_id in open_logs ∪ deployed_with_a_log_file {
        let log = store.log_for(&service_id).await?;
        if !deployed.contains(&service_id) {
            // Nothing removes a service's data directory on undeploy. Its
            // sagas are dropped rather than compensated: the operator
            // withdrew the whole service, and sending undos on behalf of
            // something that no longer exists is the mirror of B2's
            // "delivering would resurrect intent an operator withdrew".
            log.drop_all_for_undeployed()?;   // one DELETE pair, logged at info
            continue;
        }
        // The crash case: an open saga past its deadline starts walking
        // back. Nothing else can notice, because a guest does not exist
        // between calls.
        for head in log.abandoned(now, SAGA_SWEEP_LIMIT)? {
            log.mark_compensating(&head.saga_id, now)?;
            warn!(saga = %head.saga_id, "saga passed its deadline; compensating");
        }
        for head in log.due_compensations(now, SAGA_SWEEP_LIMIT)? {
            settled += self.compensate_next_step(&service_id, &log, &head).await;
        }
    }
    settled
}

/// One undo. Deliberately one per saga per tick: the walk is ordered, so a
/// step that fails must not be overtaken by the step below it, and a saga
/// with a slow provider must not hold the tick against every other saga.
async fn compensate_next_step(&self, service_id, log, head) -> usize {
    let Some(step) = log.next_uncompensated_step(&head.saga_id)? else {
        log.finish_compensation(&head.saga_id, now)?;   // nothing left: done
        return 1;
    };
    // Before dispatch, not after (§0.11): a crash inside the call must cost
    // an attempt, or a poison step is retried forever. Safe only because
    // the key below lets the receiver answer a duplicate from its record.
    let attempts = log.begin_undo_attempt(&head.saga_id, step.idx, now)?;
    if attempts > u32::from(config.retry.max_attempts) {
        log.fail_compensation(.., "undo attempted repeatedly without ever completing", true)?;
        return 1;
    }
    let target = match store.resolve_step_target(&head, &step) {
        Ok(t) => t,
        // A dependency name bound to nobody is not a failed delivery, it is
        // having nothing to deliver to -- terminal on its own terms, the
        // same split `drain_one_outbox` makes.
        Err(e) => { log.fail_compensation(.., &e.to_string(), true)?; return 1; }
    };
    let req = ProxyRequest {
        target_service: target,
        interface: step.interface.clone(),
        method: saga_undo_name(&step.method),
        params: merge_forward_result(&step.params, step.result.as_deref()),  // §0.7
        caller: CallerContext::service_system(service_id),
        origin: CallOrigin::Guest { service_id: service_id.to_string() },
        protocol: ProxyProtocol::JsonRpcV1,
        idempotent: true,
        idempotency_key: Some(format!("saga:{}:{}", head.saga_id, step.idx)),
        timeout: None,
    };
    match self.invoke_inner(&req).await {          // `invoke_inner`, not `invoke`: an undo has no
        Ok(_) => { log.mark_step_compensated(..)?; }   // caller holding the error, so a proxy dead
        Err(e) => match proxy_outbox::disposition_of(&e) {
            Disposition::Delivered => log.mark_step_compensated(..)?,   // the receiver already ran it
            Disposition::Retry     => { log.fail_compensation(.., false)?; }
            Disposition::Terminal  => { log.fail_compensation(.., true)?; }
        },
    }
    1
}
```

`merge_forward_result` is §0.7's three-case rule, a pure function, tested on
its own.

**The loop.** `run_outbox_worker` is renamed **`run_async_worker`** — it now
drains outboxes *and* sweeps sagas, and a name that says only the first is
how the next reader misses the second. Call sites to update:
`crates/substrate/src/runtime.rs:286`, `crates/router/src/proxy.rs:1741`
(test), and any further hit from
`rg 'run_outbox_worker' --type rust`. Body:

```rust
    tokio::select! {
        () = cancel.cancelled() => return,
        _ = ticker.tick() => {}
    }
    tokio::select! {
        () = cancel.cancelled() => return,
        settled = async {
            let a = self.drain_outboxes_once().await;
            let b = self.sweep_sagas_once().await;
            (a, b)
        } => { ... }
    }
```

Cancellation stays raced *into* the work, not merely against the tick —
B1's D-B1-8 and the B2 review's F18, which was vacuous twice. The saga sweep
inherits that for free by living inside the same `select!` arm.

**Operator surface.** `control-plane.wit`, `orchestrator` interface, beside
the three `proxy-*` verbs:

```wit
    record saga-record {
        saga-id: string,
        name: string,
        state: string,          // open|compensating|compensated|failed
        steps: u32,
        compensated-steps: u32,
        created-at: s64,
        deadline-at: s64,
        last-error: option<string>,
    }

    /// Every saga `service-id`'s own log holds, oldest first.
    sagas: func(service-id: string) -> result<list<saga-record>, string>;

    /// Re-arms a saga whose compensation gave up: it returns to
    /// `compensating` with the current step's attempts reset, and the
    /// worker picks it up on its next tick. Never walks inline, for the
    /// same reason `proxy-replay` never delivers inline.
    saga-compensate: func(service-id: string, saga-id: string) -> result<_, string>;
```

`ProxyQueueInspector` (`crates/rpc/src/proxy.rs`) gains
`async fn sagas(&self, service_id: &str) -> Result<Vec<SagaInfo>, String>`
and `async fn rearm_saga(&self, service_id: &str, saga_id: &str) -> Result<(), String>`,
with its doc comment widened from "durable proxy queues" to "durable proxy
state". Two impls to update: `ProxyOutbox`
([proxy_outbox.rs:397](../../../../crates/router/src/proxy_outbox.rs#L397))
— which now needs the `SagaStore` handle, so the impl moves to a small
`ProxyState` wrapper holding both, wired through the same `OnceLock`
`ControlPlaneService::proxy_queue_inspector` already uses — and
`FakeProxyQueues` in
[orchestration.rs:9140](../../../../crates/control_plane/src/service/orchestration.rs#L9140).

Gating, following B2's F7 exactly: `sagas` takes
`authorize_proxy_queue_access` (read, `orchestrator/status`);
`saga-compensate` takes `authorize_proxy_queue_write` (`orchestrator/deploy`),
because it causes calls to leave this node.

Dispatch arm in `control_plane/src/service.rs` beside `"proxy-replay"`
(line 605), parameter parsing in the same positional-or-named shape;
`SyneroymClient::sagas`/`saga_compensate` in `crates/sdk/src/lib.rs` beside
`proxy_replay` (line 830); `roymctl svc sagas --svc-id` and
`roymctl svc saga-compensate --svc-id --saga-id` in
`apps/roymctl/src/commands/svc.rs`, printing in the same column style as
`ProxyDeadLetters`.

**Developer guide.** A "Compensating a workflow (sagas)" subsection after
"Scheduling a service"
([developer-guide.md:980](../../../../docs/developer-guide.md#L980)),
matching its shape: the manifest snippet, the WIT convention with both
halves, the "an undo may be called for an operation that never happened"
rule, the deadline's meaning, and the two operator verbs. Lands with this
slice, not at closeout, exactly as B3's did.

**Phase 4 tests** (`syneroym-router` unless noted):

- `merge_forward_result_adds_a_member_to_an_object_and_an_element_to_an_array`
- `merge_forward_result_makes_an_object_when_the_forward_params_were_null`
- `a_forward_call_that_returned_nothing_sends_no_forward_result`
- `the_walk_undoes_the_newest_step_first`
- `the_walk_undoes_a_pending_step_too`
- `an_undo_carries_the_saga_and_step_as_its_idempotency_key`
- `a_retryable_undo_failure_schedules_a_backoff_and_keeps_the_saga_compensating`
- `a_terminal_undo_failure_fails_the_saga_immediately`
- `an_undo_the_receiver_had_already_run_counts_as_compensated` (the
  `Delivered` disposition — the arm B2's N1 found missing one layer up)
- `a_saga_past_its_deadline_starts_compensating_without_the_guest`
- `an_undeployed_services_sagas_are_dropped_rather_than_compensated`
- `cancellation_interrupts_an_in_flight_undo_and_leaves_the_saga_compensating`
  — with a fixture that genuinely blocks until released, and asserted by
  reverting the `select!` to a between-ticks check (the B2 F18 discipline:
  every fix in this phase is proved by reverting it and watching its test
  fail)
- `syneroym-control-plane`: `sagas_lists_what_the_services_log_holds`,
  `saga_compensate_is_not_reachable_with_only_the_read_grant`,
  `sagas_is_refused_without_a_status_grant`
- `roymctl`: the two new subcommands in `tests/cli_args.rs`

### Phase 5 — the fixture and the end-to-end proof

**New fixture** `test-components/saga-test` (excluded from the workspace in
the root `Cargo.toml`, path constants in
`crates/core/src/test_constants.rs`, both alongside `scheduled-test`'s
entries). One component, deployed **twice** under two service ids — as the
driver and as the participant — which is how `proxy-test` already serves as
both caller and target.

```wit
package syneroym-test:saga-test@0.1.0;

interface saga-driver {
    /// begin -> step(reserve, a) -> step(reserve, b) -> commit|compensate.
    /// Returns the saga id so the test can poll `sagas` for it.
    run-workflow: func(peer: string, items: string, outcome: string)
        -> result<string, string>;
    /// begin -> one step -> return, leaving the saga open. For the
    /// deadline/restart case.
    start-and-abandon: func(peer: string, item: string, deadline-secs: u64)
        -> result<string, string>;
}

interface saga-participant {
    reserve: func(item: string) -> result<string, string>;
    saga-undo-reserve: func(item: string, forward-result: option<string>)
        -> result<_, string>;
    /// The audit log, newest last: "reserve:a", "undo:a", ... Backed by the
    /// component's own data layer, so it survives the instance and the test
    /// can read the *order* the undos ran in, not only the final state.
    ledger: func() -> result<string, string>;
}
```

The participant stores a JSON **object** payload (`{"log": [...]}`), never a
bare JSON scalar — B3's own e2e found that the data layer's `payload` column
is declared `JSON`, which SQLite gives NUMERIC affinity, so a bare number is
stored as an `INTEGER` and the host's text read then fails
([status.md](status.md), B3 deviations).

**`crates/substrate/tests/saga_e2e.rs`**, two tests, one real substrate for
the driver and one for the participant (the shape `proxy_outbox_e2e.rs`
already establishes), both failing loudly — not skipping — when the fixture
artifact is not built:

1. `a_failed_workflow_is_undone_in_reverse_order`
   1. Deploy the driver on A and the participant on B. Adopt. (Nothing is
      declared anywhere — the participant's `saga-undo-reserve` export is
      the whole of its participation, §0.4a.)
   2. Call `run-workflow(peer, "a,b", "compensate")` through the gateway.
   3. Assert the participant's `ledger` is `reserve:a, reserve:b`
      immediately after the call (the walk has not run yet — `compensate`
      returns before it).
   4. Assert `roymctl svc sagas` on A shows the saga `compensating`.
   5. Within a few worker ticks, assert the ledger is
      `reserve:a, reserve:b, undo:b, undo:a` — **the order is the
      assertion**, and it is the one thing no in-process test proves about
      two real nodes.
   6. Assert the saga is `compensated` and that a second sweep changes
      nothing (the walk is not re-run).
2. `a_workflow_abandoned_across_a_restart_is_compensated_by_its_deadline`
   1. `start-and-abandon(peer, "a", deadline-secs = 60)`; assert the ledger
      is `reserve:a` and the saga is `open`.
   2. **Tear down substrate A and restart it**, the step no in-process test
      can express and the reason the log is durable.
   3. Assert through `sagas` that the saga survived the restart as the same
      id, still `open`.
   4. Past the deadline, assert the ledger gains `undo:a` and the saga
      reaches `compensated` — with the witness being the supervisor-side
      record as well as the ledger, since B3-07 showed a post-restart
      transport can be the thing that actually failed.

Both tests anchor waits to observed state with a deadline loop, never to a
fixed sleep (B3's second deviation). Run **at least three times** before the
result is trusted, for the reason B3-07 records: the first flake there was
real and the requirement was not ceremonial.

---

## §3 — Failure and security rows this slice owns

Four inherited from [task.md](task.md)'s matrix, applied to B4's own tables,
and three that are B4-local (§0.13 explains why they live here and not in
the milestone doc).

| # | Case | Required behavior | Test |
|---|---|---|---|
| 2 | The same undo is applied twice | No observable difference: every undo carries `saga:<id>:<idx>` and B2's receiver-side fence answers the duplicate from the first call's record (D-B4-3) | `an_undo_carries_the_saga_and_step_as_its_idempotency_key`, `an_undo_the_receiver_had_already_run_counts_as_compensated` |
| 12 | Saga growth is unbounded | It is not: §0.10's four bounds, two refusing and two evicting, each asserted | the four `*_is_refused` / `terminal_rows_are_pruned_*` tests |
| 13 | Saga contents at rest | The log is in the calling service's own `async.db` under its own DEK, beside its outbox and fence records — the same protection its `state.db` has | a store test that the file is unreadable without the DEK, in the shape `app_supervisor/src/store.rs`'s own protection test takes |
| 14 | Re-arm authorization | `saga-compensate` causes calls to leave the node, so it takes the write gate, not the listing's read gate (B2's F7) | `saga_compensate_is_not_reachable_with_only_the_read_grant` |
| B4-a | The substrate dies mid-workflow | The saga survives, and its declared deadline is what eventually unwinds it — no guest is involved, because a component does not exist between calls | e2e test 2 |
| B4-b | A step is added while the saga is walking backwards | Refused inside the same transaction that reads the state, so a `compensate` landing between the check and the insert cannot be overtaken | `a_step_added_to_a_compensating_saga_is_refused` |
| B4-c | An undo can never be delivered | The saga reaches `failed`, keeps its step history and its last error, is listed by `sagas`, and is re-armable by an operator. Never silently dropped, never retried inline (ADR-0023 §5) | `an_exhausted_attempt_budget_fails_the_saga`, `rearm_returns_a_failed_saga_to_compensating_with_attempts_reset` |

**One security note that is not a row.** A saga's undo travels as
`CallerContext::service_system(driver)` with `CallOrigin::Guest`, byte-identical
to what the driver's own live `call` would build — the same rule
`request_from` states for a queued call, and for the same reason:
authorization at the receiver must not differ between the immediate call and
the one the platform makes later on the caller's behalf. No new identity, no
new ability, no new resource namespace.

---

## §4 — Questions for the requester

**Q1. ~~Is the manifest declaration worth its sweep?~~ Answered 2026-08-07:
no declaration.** The requester's reasoning went further than the question
did and is recorded in §0.4a as the decision's own argument: a service with
no compensation is not an author who forgot, it is **the ordinary case** —
an idempotent or read-only operation has nothing to undo — so "not declared"
has to mean "takes part in no saga" whether or not a declaration exists.
That leaves the declaration guarding one author mistake at the price of the
slice's largest diff. Dropped, with a backlog row (§5). The same
conversation produced §0.4b, which is the sharper finding of the two: a bare
`undo-` prefix is an ordinary business verb, so the export-derived check
that replaces the declaration is only sound because the marker became
`saga-undo-`.

**Q2. ~~Should a failed step compensate automatically?~~ Answered
2026-08-07: no**, as planned (D-B4-1). A failed step is returned to the
guest, which may retry it, choose another provider, or give up; only an
explicit `compensate` or an expired deadline starts the walk. The deadline
covers the one case a guest cannot handle — its own death.

**Q3. ~~Deadline defaults.~~ Answered 2026-08-07: as proposed.**
`saga_default_deadline_secs = 3600` and `saga_max_deadline_secs = 86400`.
Recorded as what they are: an hour and a day are honest first guesses, not
measurements. The default has to be longer than a human-paced workflow and
shorter than "nobody will ever notice", and M6's Guild scenario — food prep,
then delivery — sits comfortably inside both. Both are config fields, so a
deployment that finds them wrong changes them without a rebuild; if M6's
real workflows land outside the default, that is a number to revisit, not a
design to reopen.

**No questions remain open. This plan is executable as written.**

---

## §5 — Docs and backlog impact

**This slice closes the milestone**, so it carries the closeout doc work
[implementation-plan.md](implementation-plan.md) §3 lists:

- `[PLT-ASY]` in
  [traceability-matrix.md](../../traceability-matrix.md): **Pending →
  Complete**, scoped to the four mechanisms (the row already carries that
  scoping). This is what fires M05A slice A6's pickup trigger, which A6's
  own row already recorded as discharged by B1.
- `system-architecture.md` and `system-requirements-spec.md` `[PLT-ASY]`:
  a dated implementation-status note in the Universal Proxy note's shape,
  covering all six supersessions now (client-side outbox placement, the
  lease-based scheduler, the `undo_<operation>` spelling **and the prefix
  itself**, the "generated resource ID", and compensations firing from a
  queued task — §0.4b, §0.6, §0.7).
- **ADR-0023 §7 amendment, dated, in the shape its own earlier corrections
  take.** Two sentences: the marker is `saga-undo-` because `undo-` is an
  ordinary business verb (§0.4b), and there is no manifest declaration
  because "not declared" is the ordinary case rather than an author error,
  so the deploy check is derived from the component's exports (§0.4a).
  The milestone plan's §0.6 gets a pointer to the same amendment. This is
  the one place a *decision* changed rather than an implementation detail,
  so it lands in the ADR, not only here.
- `developer-guide.md`: the saga subsection (phase 4) plus the
  partitioned-substrate consequence of ADR-0023 §6 stated plainly, which
  B3 left to closeout.
- [task.md](task.md) exit criteria 6 (✅) and a new criterion recording B4's
  own migration cost — **one additive WIT interface, no manifest change and
  no wire change** — since the *Migration impact* section names none
  (§0.13).
- [status.md](status.md): the B4 delivery section, in B3's shape.

**Backlog rows this slice's own choices create** (all §8, per the
*Mandatory Deferred-Backlog Update* rule):

| Row | Why |
|---|---|
| A dead-lettered queued call cannot trigger a compensation | §0.6. Needs the outbox worker to know which saga an item belonged to, which nothing records. Target TBD |
| A saga step's proxy dead letter is replayable outside its saga | §0.8. An operator replaying it re-runs a forward step of a possibly-compensated workflow. Closing it means teaching the DLQ about sagas — the same coupling as the row above |
| The deploy gate checks existence, not shape | §0.7. `saga-undo-x` may take anything; a mismatch surfaces as a failed undo at compensation time, hours later. Checking arity/types needs the forward function's own type, which `exported_functions` could return |
| **A service cannot declare that it *intends* to be compensable** | §0.4a. Deliberate: absence means "no saga", which is the ordinary case, and a declaration would cost ~90 literal edits to guard one author mistake. Cost of not having it: a participant that means to be compensable and exports nothing deploys cleanly, and the mistake surfaces as a `failed` saga at compensation time. **Pickup trigger: M6's Guild work showing authors actually forget it.** Pre-release, adding it later is the same edits and no compatibility work |
| A saga step's target is not checked for a compensation at call time | Cross-node, the driver cannot see what the participant exports. Local, it could. Not attempted, so a missing compensation is discovered by the walk, not by the step |
| A guest cannot read why a compensation failed beyond `last-error` | `status` returns the saga's last error, not per-step history. The operator verb has the same limit |
| No partial or per-step re-arm | `saga-compensate` re-arms the whole saga from its current step; there is no "skip this step" verb. Same shape as B1's "`replay` is all-or-nothing per item" |
| A third copy of the per-service `async.db` open was avoided but not removed | §0.9 extracts `async_db_location`; the caching and single-flight logic still exists three times |
| Deadlines cannot be extended | A guest that legitimately needs longer must ask for it at `begin`. An `extend` verb is the obvious follow-up and is not built |

**Rows this slice must *not* touch:** the two compare-and-set rows and the
supervisor-HA row stay open with their existing "this milestone passed over
them" sentences (D-B-6). B5's row stays open with its 5-second finding — and
§0.1 shows that finding constraining a *second* slice, which is worth adding
as one sentence to that row rather than leaving it looking like a B5-only
concern.

---

## §6 — Completion checklist

- [ ] Q1-Q3 all answered 2026-08-07 (§4); no decision is left for execution time
- [ ] Five phases landed in order, each green on its own
- [ ] `cargo +nightly fmt --all` clean
- [ ] `cargo clippy --workspace --all-targets --all-features` clean, zero warnings
- [ ] `cargo test --workspace --no-fail-fast`: no failure outside the documented pre-existing sandbox-bind category; every crate this slice touches green in isolation (`syneroym-app-orchestration`, `syneroym-async-queue`, `syneroym-router`, `syneroym-rpc`, `syneroym-sandbox-wasm`, `syneroym-control-plane`, `syneroym-sdk`, `roymctl`)
- [ ] `cargo test -p syneroym-router --test proxy_dispatch` green **without rebuilding any `test-components/` artifact** — the additive-WIT claim in phase 3, verified rather than argued
- [ ] `mise run test:e2e` 12/12
- [ ] `saga_e2e` green three consecutive times, both cases
- [ ] Each phase-4 fix proved by reverting it and watching its own test fail (the B2 N-round discipline)
- [ ] No planning-doc ids (`D-B4-N`, "slice B4", milestone ids) in any added code, doc comment or test name
- [ ] Import cleanup pass over every edited file
- [ ] Backlog rows in §5 added; `[PLT-ASY]` marked Complete; the three doc amendments landed
- [ ] `status.md` B4 section written, including anything that shipped differently from this plan and why
