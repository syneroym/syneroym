# M05B Async Primitives — Status

**Milestone:** [task.md](task.md) · **Design of record:**
[ADR-0023](../../../decisions/0023-durable-async-primitives.md) · **Plan:**
[implementation-plan.md](implementation-plan.md)

**Overall:** 🚧 **B1 complete 2026-08-05**, all gates green (evidence below).
ADR-0023 accepted 2026-08-04. **B2 has a full plan as of 2026-08-05** (not
started); B3 and B4 remain sketch only.

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
| B1 | Queue crate, supervisor delivery outbox, DLQ with alert and operator surface. **Closes [M05A slice A6](../M05A-app-supervisor/task.md)** | ✅ **Complete (2026-08-05)** — evidence below | ADR-0023 accepted |
| B2 | Guest outbox and proxy DLQ: idempotency key, `enqueue`, receiver-side dedup | ✅ **Complete (2026-08-05)** — all five phases — see "B2 — delivery" below. Plan: [slice-b2-implementation-plan.md](slice-b2-implementation-plan.md) (2026-08-05, revised same day after review). Its `§0` narrows the slice in four places and its `§5` carries six document corrections | B1 |
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

## B1 — Verification evidence (2026-08-05)

Five phases per [slice-b1-implementation-plan.md](slice-b1-implementation-plan.md)
§2, all landed on `feat/m05b-b1-async-queue-outbox-dlq`.

**What shipped, by phase:**

- **Phase 1** (`refactor(app-supervisor): route SubstrateActor upcasts
  through one constructor`, `chore(deps): remove six unused dependencies` —
  actually four; see below): `deploy::build_actor` (generic over any
  `SubstrateActor`) replaces all ten raw `client.clone() as Arc<dyn
  SubstrateActor>` upcasts (six in `app_supervisor`, two in `roymctl`, two
  in e2e fixtures); `roymctl` and the e2e fixtures call it directly
  (D-B1-11's undurable opt-out), `app_supervisor`'s real-client sites are
  routed through it in phase 1 and re-pointed at `build_durable_actor` in
  phase 3. The dependency sweep found `assert_cmd`/`sysinfo`
  (`syneroym-perf`) and `metrics` (`syneroym-sandbox-podman`) still have
  other consumers on the shipped tree, so the removal is **four**
  dependencies (`quinn`, `metrics-util`, `syneroym-sdk` from
  `syneroym-smoke-tests`, `chromiumoxide`), not the six the plan's §0.10
  reviewed — corrected in [deferred-backlog.md](../../deferred-backlog.md)'s
  matching row rather than left to silently disagree with the diff.
- **Phase 2** ([crates/async_queue](../../../../crates/async_queue/src/lib.rs),
  `syneroym-async-queue`): `outbox`/`dead_letters` tables, `Pending`/
  `InFlight`/`Dead` states (a completed item is deleted, not tombstoned,
  D-B1-9), a claim as one conditional `UPDATE` with a visibility timeout,
  `next_attempt_at` arithmetic over `RetryPolicy` + `calculate_jittered_
  backoff` (D-B1-13 — `retry_with_backoff` itself is not used, since it
  sleeps in-process where this queue computes a timestamp and forgets the
  item). 15 unit tests, all named in the plan's §3 phase-2 list.
- **Phase 3** (`crates/core/src/config.rs`, `crates/sdk/src/deploy.rs`,
  `crates/app_supervisor/src/{outbox.rs,service.rs,store.rs}`): five new
  `SupervisorRole` fields (`queue_tick_secs`, `queue_max_attempts`,
  `queue_max_backoff_secs`, `queue_visibility_timeout_secs`,
  `queue_dlq_max_rows`), defaults per §0.12's arithmetic (`the_configured_
  defaults_give_a_ten_hour_window` pins the nominal 36,738s window
  unjittered). The queue store is a fourth sibling inside
  `SupervisorStore::from_connection` alongside the journal and alert store
  (D-B1-5). `deploy::build_durable_actor` wraps a connected client:
  `write_bindings` attempts synchronously first (D-B1-1) and, only on a
  **transport** failure, enqueues before returning the identical error a
  bare client would have — distinguished from a **callee** failure via
  `deploy::is_callee_error`, which downcasts to `syneroym_rpc::JsonRpcError`
  (present only when a real wire error-frame came back; anything else,
  including a connect that never got that far, is transport). Every other
  action (`apply_plan`, `restart`, `renew_cert`) stays synchronous-only
  (D-B1-3/D-B1-12). The worker (`run_queue_worker`/`queue_worker_tick`/
  `deliver_queued_item`) claims due items, takes the same `instance_lock`
  a live pass holds (D-B1-14), and applies the D-B1-15 outcome mapping
  (`applied`/`no-op`/`stale` → complete; `conflict` → complete **and**
  raise `BindingConflict`; transport error → retry). Spawned beside the
  resident loop in `runtime.rs` (`spawn_queue_worker_role`,
  `queue_worker_join`) and **not** awaited on shutdown (D-B1-8) — an
  in-flight delivery against an offline substrate is exactly the case the
  queue exists for, and waiting for it would make shutdown hang on it.
- **Phase 4** (`crates/app_orchestration/src/alerts.rs`,
  `crates/wit_interfaces/wit/supervisor/supervisor.wit`,
  `apps/roymctl/src/commands/supervisor.rs`): `AlertKind::DeliveryExhausted`,
  a **standing** alert per `(instance, logical_ref, substrate)` with the
  dead-letter count in `detail` (D-B1-6 — `AlertStore`'s unique index
  cannot express one row per item), refreshed as more accumulate and
  cleared when the last one for that key is gone. `dead-letters`/`replay`
  on `supervisor.wit`, gated exactly as the surrounding verbs are;
  `roymctl supervisor dead-letters`/`replay`. `replay` re-enqueues; it
  never executes inline (D-B1-7).
- **Phase 5** ([crates/substrate/tests/durable_outbox_e2e.rs](../../../../crates/substrate/tests/durable_outbox_e2e.rs)):
  the two-substrate reference scenario, built through the phase-1/3
  constructors (not a bare upcast) so the queue is actually exercised, not
  bypassed.

**A real gap found and fixed while writing phase 5, not by the plan's own
review.** The push loop's two "could not even reach an actor" branches (no
known alias for a landed DID; a `connect_best_effort` that timed out before
a client existed) raised `BindingConflict` and moved on — the same shape
the plan's §0.9 review left them in. `build_durable_actor`'s enqueue-on-
transport-failure only fires *inside* an attempted `write_bindings` call,
and neither branch gets far enough to make one, so **a substrate that is
durably offline — exactly ADR-0023's reference scenario — was never queued
at all, only ever reported.** A synchronous call that fails mid-flight
(the case every other test drove) was durable; a substrate the supervisor
never even reaches to begin with was not. Fixed by
`enqueue_unreachable_push` (`service.rs`): advances the binding epoch and
builds the same `BindingWrite` payload `write_bindings_at_epoch` would
have, then enqueues it directly through `SupervisorOutbox`, bypassing the
(nonexistent) actor. Applied at both of the resident loop's own branches
and the equivalent branch in `apply_with_membership_pushes` (the
`submit`/`force-reconcile` path), for the same reason. Found only because
phase 5's e2e drives a substrate that is torn down *before* the write is
ever attempted, rather than one that fails mid-call — exactly the
distinction §0.3's review worried the bare-upcast e2e fixtures would let
slip past untested, just one level deeper than that review caught.

**A second, unrelated finding, fixed alongside it — corrected once during
verification, worth recording both the wrong fix and the right one.**
iroh's `Endpoint` self-enumerates every local network interface —
including loopback's own link-local IPv6 (`fe80::1%lo0`, which loses its
interface scope once serialized and published, making it unroutable) and
this machine's real LAN/global-IPv6 addresses — as "direct address"
candidates that ride along in `Endpoint::addr()`'s `EndpointAddr`. None of
those candidates can ever succeed between two local processes sharing only
a self-hosted relay, but each is still tried (and timed out) by every peer
that receives them before falling back to the relay it needed anyway. The
first fix attempt added `.addr_filter(AddrFilter::relay_only())` to
`build_iroh_endpoint`
([crates/router/src/net_iroh.rs](../../../../crates/router/src/net_iroh.rs)) —
**ineffective**, caught in review and confirmed by reading iroh 0.97's own
source directly (not docs.rs prose, which had led the first attempt
astray): `addr_filter` (`endpoint.rs:261-262`) gates only what a
registered `.address_lookup(...)` service publishes, and this crate
registers none; `watch_addr()`, what `.addr()` reads, builds straight from
the socket layer's own `ip_addrs()`/`home_relay()`, a path the filter
never touches. The apparent improvement from that first attempt was
measurement noise, not a fix — a rerun with it in place still took 407s,
slower than an earlier "passing" run. **The real construction site** is
[crates/coordinator_iroh/src/coordinator.rs:269](../../../../crates/coordinator_iroh/src/coordinator.rs#L269),
the `/v1/info` HTTP endpoint every peer in this crate's e2e harnesses
resolves through: it calls `iroh_endpoint.addr()` directly and serializes
the *entire* unfiltered `EndpointAddr`. Fixed there instead — filtering
`endpoint_addr.addrs` to `TransportAddr::is_relay` before serializing,
the same way this file's own global-registry registration path already
avoids the problem with a bare `EndpointAddr::new(node_id)`. The
ineffective `net_iroh.rs` change was reverted rather than left as
harmless-looking dead code carrying an incorrect rationale in its comment.
Confirmed working, not just plausible: zero occurrences of the bogus
addresses in either e2e test's log after the real fix, versus dozens
before it. This is a real-time fix, not a correctness one — the reference
scenario passed both before and after in terms of what it asserts, but
individual runs before ranged from ~90s to over 25 minutes (once requiring
a hard kill); after, both e2e tests land consistently around 5-7 minutes.

**A third finding, also found only by phase 5's e2e, also a real
correctness gap rather than a test artifact.** A5e's resident loop
reclassifies an unlanded push as a candidate on every pass until it lands
(`compute_diff` deliberately falls back to the previous baseline on
Degraded so the next pass retries). `enqueue_unreachable_push` — called
from that same repeatedly-reclassified branch — enqueued a fresh row every
time, so a substrate offline for several passes accumulated duplicate rows
for the identical logical write, each starting its own attempt budget at
zero; a newer, immediately-due duplicate kept winning the next claim over
an older one waiting out its backoff, so no single row ever actually
exhausted `queue_max_attempts` and reached the DLQ. Symptomatic first as
the DLQ e2e test's own arrival check timing out even at a 280s deadline;
confirmed by the delivery-attempt counter visibly resetting to 0 multiple
times in a `RUST_LOG=debug` run, rather than climbing 0→1→2 once. Fixed
with `SupervisorOutbox::already_pending` — a linear scan over the (small,
per-instance) outbox for an existing row with the same key — checked
before either enqueueing or advancing the binding epoch, since advancing
the epoch on a skipped enqueue would strand the local counter ahead of
whatever the eventually-delivered, earlier-epoch write actually lands,
permanently preventing `is_converged` from ever agreeing again. Two new
unit tests
(`crates/app_supervisor/src/outbox.rs`:
`a_second_enqueue_for_the_same_key_while_one_is_still_pending_is_a_no_op`,
`already_pending_is_false_for_a_key_never_enqueued`).

**Tests added: 41** — 15 in `syneroym-async-queue` (phase 2), 5 in
`crates/sdk/src/deploy.rs` (`build_durable_actor`'s try-then-queue and the
transport/callee distinction), 4 in `crates/app_supervisor/src/outbox.rs`
(`QueueKey`'s round trip, and the dedup finding above), 14 in
`crates/app_supervisor/src/service.rs` (worker, wiring, DLQ dispatch,
alert lifecycle), 1 in `crates/app_supervisor/src/store.rs` (the queue's
own protection under `supervisor.db`), and 2 e2e
(`a_binding_push_to_an_offline_substrate_converges_after_it_returns`,
`a_permanently_unreachable_substrate_lands_in_the_dlq_and_replays`).
Counted by diffing test-attribute counts against `main`.

**Gates, run 2026-08-05:**

- `cargo +nightly fmt --all`: clean.
- `cargo clippy --workspace --all-targets --all-features`: clean, zero
  warnings.
- `cargo test --workspace` (sandboxed, `--no-fail-fast`): 1404 passed, 63
  failed across 25 targets — every one of them the identical pre-existing,
  environmental sandbox-bind category [M05A status.md](../M05A-app-supervisor/status.md)
  already documents (`community-registry`, `control-plane`'s HTTP-probe
  tests, `coordinator-iroh` (3 targets), `mqtt-broker`, `sdk --test
  connect_timeout`, every `syneroym-substrate` e2e target). Every crate
  this slice actually touches — `roymctl`, `syneroym-app-supervisor`,
  `syneroym-async-queue`, `syneroym-coordinator-iroh`, `syneroym-router`,
  `syneroym-sdk` — is fully green in this same sandboxed run: 0 failures.
- `mise run test:e2e` (sandbox disabled, required for real port binds):
  12/12 green (8 main + 4 multi-hop), unchanged pass count from before
  this slice — expected, B1 adds no browser-visible surface.
- `durable_outbox_e2e` (sandbox disabled, required for real port binds and
  a real supervisor restart), both green after the coordinator.rs and
  outbox dedup fixes above, zero occurrences of the bogus addresses in
  either run's log: `a_binding_push_to_an_offline_substrate_converges_
  after_it_returns` 1/1 (~287s), `a_permanently_unreachable_substrate_
  lands_in_the_dlq_and_replays` 1/1 (~403s).

## B1 — Post-landing review pass (2026-08-05)

An independent review of the shipped B1 code (not of the plan — the code
actually on `feat/m05b-b1-async-queue-outbox-dlq`) found sixteen further
gaps: four correctness bugs left the outbox/DLQ inconsistent with the
invariants its own comments and D-B1-6/D-B1-9 state; four concurrency/
shutdown claims did not hold, most seriously the queue worker not actually
honoring D-B1-8 despite a test that claimed it did; four config/security
edges; four coverage gaps, including the reference scenario's own steps
4/5/7 having no RPC surface to assert the outbox directly. All sixteen were
verified against the shipped source (not taken on the review's word) before
being incorporated — none were pushed back on outright, though two landed
narrower than the review's own suggested fix, noted below.

**Fixed, by area:**

- **Epoch/outbox skew (finding 1).** `push_bindings` advanced the binding
  epoch on every call, but `DurableActor::write_bindings` only enqueues
  once per key. Two consecutive connect-succeeds-write-fails passes for the
  same key could advance `written_epoch` past what the queue would ever
  actually deliver, permanently reading a later successful delivery as
  unconverged. Fixed with the same already-pending guard
  `enqueue_unreachable_push` already carried, now applied symmetrically.
  Regression test: `two_consecutive_transport_failures_for_one_key_do_not_
  strand_the_written_epoch` (confirmed to fail without the fix).
- **`replay` bypassing the one-row-per-key invariant (finding 2).**
  `Queue::replay` now refuses when the outbox already holds a pending row
  for the dead letter's key, rather than inserting a second one.
- **An unnotified DLQ prune (finding 3) and a global, not per-instance, cap
  (finding 4).** `FailOutcome::DeadLettered` now carries `pruned_keys`; the
  cap and its eviction are scoped by a caller-supplied `group_key` the
  queue crate stores but never parses (the supervisor's own app instance
  id), so one instance's dead letters cannot evict another's, and every
  pruned key's `DeliveryExhausted` alert clears the same way an explicit
  `replay` already did.
- **Shutdown did not actually honor D-B1-8 (finding 5).** `run_queue_worker`
  only raced cancellation against the next interval tick; once a tick
  fired, `queue_worker_tick` ran every claimed item (up to
  `QUEUE_WORKER_CLAIM_LIMIT`, each up to `MANAGED_SUBSTRATE_CONNECT_TIMEOUT`)
  to completion regardless of `shutdown`. The test that claimed otherwise
  was vacuous: it set `queue_tick_secs` to 3600 (contradicting its own
  comment) against an unpaused clock, so the interval never ticked inside
  the test's own window, and it left the substrate unscripted, which fails
  `connect` immediately rather than modeling a delivery in flight. Fixed by
  racing cancellation into `deliver_queued_item` itself via `tokio::select!`,
  checked between every item; the test now uses a `FakeDelivery::Blocks`
  variant that genuinely never resolves, so a passing result means
  cancellation interrupted a real, ongoing connect.
- **A poison-pill item never reached its attempt budget (finding 7).**
  `attempts` only advances inside `Queue::fail`; a delivery that panics or
  a worker that crashes before calling it left the counter untouched, so
  the item was reclaimed forever. Added `claim_count`, advanced on every
  `claim_due`, independent of `attempts`; the queue worker dead-letters an
  item through the ordinary terminal path once its claim count alone
  reaches the configured budget.
- **The retry clock read before the connect it was timing (finding 8).**
  `deliver_queued_item` captured `now` at function entry and reused it
  after a `connect` that can itself take up to 10s, understating the early
  backoff waits. Fixed by re-reading the clock immediately before
  `fail_queued_item`.
- **`queue_max_attempts = 0` silently disabled retry (finding 9).** Clamped
  to 1, mirroring the existing `max_renewals_per_pass == 0` clamp, with the
  same `tracing::warn!`.
- **Every reached-and-answered write-bindings failure was terminal on the
  queued path (finding 10).** `is_callee_error` is `true` for any
  `JsonRpcError`, but `control_plane`'s dispatch maps every server-side
  refusal — stale generation, authorization gap, a genuinely gone service —
  to the same wire code. Narrowed with `deploy::is_target_gone_error`,
  matched against the one message `write_bindings_impl` emits specifically
  for "gone". **Landed narrower than the review's other suggested option**
  (a dedicated wire error code) — that needs a `control_plane` dispatch
  taxonomy decision out of scope here; tracked in
  [deferred-backlog.md](../../deferred-backlog.md) as a message-text
  fragility, not closed.
- **`already_pending` failed open on a broken read (finding 11).**
  `is_ok_and` reported a failed read as "not pending", writing the
  duplicate row the guard exists to prevent. Also replaced the linear
  `Queue::all()` scan with an indexed `Queue::has_pending` lookup (folding
  in finding 15's measurement gap: the production enqueue path was paying
  a full-table scan the `< 1ms` budget test never actually exercised).
- **`replay`'s error codes named a caller's own mistake as an internal
  error (finding 12).** An unknown or cross-instance dead-letter id now
  answers `InvalidParams`, without naming which other instance (if any)
  actually owns the id.
- **No RPC surface for the outbox itself (finding 13).** `supervisor.wit`
  gained `outbox` beside `dead-letters`/`replay`, gated identically;
  `roymctl supervisor outbox`. `durable_outbox_e2e.rs`'s steps 4/5/7 now
  assert the item is actually in the outbox, survives the restart as the
  same item, and is gone once delivery converges — task.md's own wording
  for those steps, previously only inferred from alerts/`is_converged`.
- **Coverage (findings 14, 16).** Added a direct test for row 12's outbox
  bound (`the_outbox_holds_at_most_one_row_per_key_regardless_of_how_many_
  enqueue_attempts`) and for the prune-clears-alert path
  (`a_pruned_dead_letter_clears_its_own_alert_too`). A retired instance's
  still-queued item is now completed silently rather than delivered (which
  would resurrect a binding the operator just released) or dead-lettered
  with a noisy alert against an instance nobody is going to act on.

**Confirmed already correct, not touched:** the `instance_lock` acquisition
(D-B1-14) and its test; the queueability declaration (`restart`/`apply_
plan`/`renew_cert` never queued); the D-B1-15 outcome mapping; the
ten-hour-window arithmetic; admin gating on both new verbs.

**Tests:** 9 new (5 in `syneroym-async-queue`, 4 in
`syneroym-app-supervisor`), 3 existing tests corrected where they had
relied on the since-fixed behavior (`a_callee_error_on_replay_dead_letters_
immediately` and `a_replayed_item_that_fails_again_returns_to_the_dlq_
with_its_history` now script the exact "gone" message finding 10's fix
checks for; `the_alert_clears_when_the_last_dead_letter_for_that_key_is_
gone` now lets the first replay's item resolve before replaying the second,
since two simultaneous pending rows for one key is exactly what finding 2's
fix refuses).

**Gates, re-run 2026-08-05 after the review fixes:**

- `cargo +nightly fmt --all`: clean.
- `cargo clippy --workspace --all-targets --all-features`: clean, zero
  warnings.
- `cargo test --workspace` (sandboxed, `--no-fail-fast`): 1412 passed, 64
  failed. Every failure is either the same pre-existing sandbox-bind
  category this slice's original evidence already documents, or one
  additional pre-existing flake (`keys::tests::get_or_mint_warns_with_the_
  wording_matching_its_kind`, in a file this review never touched; passes
  in isolation, so it is order/concurrency-sensitive under the full
  workspace run rather than caused by anything here). Every crate this
  review touched — `syneroym-async-queue`, `syneroym-app-supervisor`,
  `syneroym-sdk`, `syneroym-wit-interfaces`, `roymctl` — is fully green run
  in isolation.
- `durable_outbox_e2e` (sandbox disabled): both tests still green with the
  new outbox assertions added, `2 passed; 0 failed`, ~690s combined.
- `mise run test:e2e`: unaffected, no browser-visible surface changed by
  this pass either.


---

## B2 — delivery (2026-08-05), complete

**All five phases of the slice plan are complete and verified against every
gate.** One plan decision was narrowed on evidence -- D-B2-11's terminal
rule, described below -- and nothing else was reversed.

**What shipped.**

- **Phase 1 — the key, end to end, reading nothing.** `idempotency-key` on
  `call-options` ([proxy.wit](../../../../crates/wit_interfaces/wit/proxy/proxy.wit));
  `ProxyRequest.idempotency_key`
  ([rpc/proxy.rs](../../../../crates/rpc/src/proxy.rs)); the optional
  `JsonRpcRequest` member, `#[serde(default)]` and skipped when absent
  ([rpc/types.rs](../../../../crates/rpc/src/types.rs)); the host function
  passing it through
  ([sandbox_wasm/host_capabilities.rs](../../../../crates/sandbox_wasm/src/host_capabilities.rs)).
  The struct-literal sweep the plan predicted landed in 23 sites across 13
  files.
- **Phase 3 — the guest outbox and `enqueue`.** `enqueue` on `proxy.wit`
  and as a second `ServiceProxy` method with a refusing default body
  ([rpc/proxy.rs](../../../../crates/rpc/src/proxy.rs)); the three refusals
  (no key, no unexpired instance certificate, a node-level or self target);
  try-then-queue; the `QueuedCall` payload storing intent rather than a
  resolved DID; the per-service encrypted outbox and the worker's outcome
  mapping ([router/proxy_outbox.rs](../../../../crates/router/src/proxy_outbox.rs));
  the worker itself (`ProxyRouter::run_outbox_worker`,
  `drain_outboxes_once`) owned by `RuntimeServices`, raced in the same
  `tokio::select!` and cancelled-not-drained on shutdown
  ([substrate/runtime.rs](../../../../crates/substrate/src/runtime.rs));
  and the `enqueue-peer` driver on the `proxy-test` guest fixture.
- **Phase 4 — the DLQ and its operator surface.** Dead-letter admission for
  a keyed synchronous call (`ProxyRouter::record_failed_call`), the two
  stale markers rewritten to state the invariant with no milestone id, the
  three `proxy-*` verbs on the orchestrator interface with the same
  `orchestrator/status` gate their neighbours use, the `ProxyQueueInspector`
  contract that lets the control plane read queues the router owns without
  inverting the crate dependency, the SDK methods, and
  `roymctl svc proxy-outbox|proxy-dead-letters|proxy-replay`.
- **Phase 2 — the store, the guard, and the router's new handle.**
  `Queue::open_encrypted` and the `DedupStore` three-state record with both
  its bounds ([async_queue](../../../../crates/async_queue/src/));
  `StorageProvider::service_db_dir` promoted with a default body so the
  test fakes keep compiling
  ([data_db/traits.rs](../../../../crates/data_db/src/traits.rs)); the five
  `queue_*` fields on `AppSandboxRole`
  ([core/config.rs](../../../../crates/core/src/config.rs)); the shared
  guard on both dispatch entry points
  ([router/call_dedup.rs](../../../../crates/router/src/call_dedup.rs),
  called from `ProxyRouter::invoke_local_guarded` and
  `RouteHandler::dispatch_json_rpc_once`).

**Deviations from the plan, and why.**

- **`ProxyRouter::new` did not gain parameters.** The plan's `§0.18`
  predicted ~10 mechanical call-site edits for a new constructor argument.
  A `with_dedup_guard` builder was used instead: the constructor already
  takes seven arguments, and nine of the ten call sites would have passed a
  `None` that says nothing. "This node can fence" is a property of the
  deployment, so it reads better as something a node either has or does
  not. The coordinator-mode case the plan wanted covered
  (`a_keyed_call_is_refused_on_a_node_with_no_storage_provider`) is covered
  either way.
- **`ServiceProxy` was not yet extended.** That belongs to phase 3.
- **The dedup store lives in `syneroym-async-queue`, not in the router.**
  It shares `async.db` and the queue crate's own connection/timestamp
  conventions, so putting it beside the queue avoided a second `rusqlite`
  dependency and a second set of the same idioms.

**Verification evidence.**

- Test attributes added, diffed against `main`: 16 in
  `syneroym-async-queue` (20 → 36), 8 in `router/src/call_dedup.rs`, 6 in
  `router/src/proxy_outbox.rs`, 24 in `router/src/proxy.rs` (19 → 43), 4 in
  the new `router/tests/call_dedup_dispatch.rs`, 4 in
  `control_plane/src/service/orchestration.rs`, 3 in `rpc/src/types.rs`, 4
  in `sandbox_wasm/src/host_capabilities.rs` — **69 net new tests**.
- `cargo +nightly fmt --all`: clean.
- `cargo clippy --workspace --all-targets --all-features`: clean, zero
  warnings.
- `cargo test --workspace --no-fail-fast` (sandboxed): the failure set is
  B1's documented pre-existing sandbox-bind category above, plus
  `syneroym-substrate --test proxy_outbox_e2e` -- this slice's own new
  target, which joins that category for the same reason every other
  substrate e2e is in it (two real substrates, real port binds, which the
  sandbox refuses). Run with the sandbox disabled it is green, three times
  consecutively. The category is
  `community-registry`, `control-plane`, `coordinator-iroh` ×3,
  `mqtt-broker`, `sdk --test connect_timeout`, and every
  `syneroym-substrate` e2e target. One further addition was genuinely ours
  and is fixed:
  `router --test proxy_dispatch` failed with "component imports instance
  `syneroym:proxy/proxy@0.1.0`, but a matching implementation was not found
  in the linker" until `test-components/proxy-test` was rebuilt against the
  changed WIT. That is exit criterion 13's migration cost observed
  directly, not a defect. After the rebuild: 8/8 green.
- Every crate touched is green in the same run: `syneroym-rpc`,
  `syneroym-core`, `syneroym-data-db`, `syneroym-async-queue`,
  `syneroym-router`, `syneroym-sandbox-wasm`. One `router --test
  native_dispatch_identity` run failed inside `mainline`'s DHT actor thread
  and passed on re-run — the same environmental sandbox-bind flake, not a
  regression.
- `mise run test:e2e` (sandbox disabled, required for real port binds):
  **12/12 green** (8 main + 4 multi-hop), the same pass count B1 recorded.
  The wire change (an optional `JsonRpcRequest` member, skipped when
  absent) is invisible to a browser client, as the byte-identical-frame
  test asserts.
- `router --test proxy_dispatch`: 8/8 green after rebuilding the
  `proxy-test` guest artifact, which is exit criterion 13's migration cost
  observed directly rather than argued.

**Phase 5 complete.** Both e2e cases are green in
[proxy_outbox_e2e.rs](../../../../crates/substrate/tests/proxy_outbox_e2e.rs):
`a_queued_guest_call_to_an_offline_node_lands_after_it_returns` and
`a_permanently_unreachable_target_lands_in_the_dlq_and_replays`, 2 passed
in ~111 s for the pair, stable across three consecutive runs (111.6 s,
110.6 s, 111.6 s) -- deliberately re-run, since the case turns on a restart
window and a single green run would not distinguish a fix from a race.

The delivery case drives the guest's own `enqueue-peer` export, asserts the
item through the `proxy-outbox` verb at every stage (queued, still queued
as the *same* item after the calling substrate restarts, gone once
delivered), and asserts it stays gone.

**Getting there required narrowing D-B2-11**, and that narrowing is the
resolution of the open question this section previously recorded. A remote
"service not found" is now **retried within the ordinary attempt budget**
rather than dead-lettering on the first hit
([proxy_outbox.rs](../../../../crates/router/src/proxy_outbox.rs),
`disposition_of`). The reason is that at the wire it is indistinguishable
from a target that is merely mid-restart: a node republishes its own
endpoint record before its services finish coming up, so a delivery attempt
lands in a window where the address is right and the service is not there
yet. Treating that as terminal gives up during exactly the outage a durable
queue exists to survive. A target that is genuinely gone still
dead-letters, just not on the first attempt -- the budget and the
poison-pill ceiling both still bound it, and the DLQ e2e proves that end
holds. This is the same kind of narrowing B1 had to make to its own
terminal rule (`deploy::is_target_gone_error`), reached independently and
for the same reason. **No wire-protocol change.**

Two boundaries the narrowing deliberately does not cross:

- **A dependency name that resolves to nobody stays terminal**
  (failure-matrix row 9). That is not a failed delivery, it is a failure to
  have anything to deliver to, so it is decided before the retry
  classification is reached (`ProxyRouter::resolve_queued_target`).
- **`PermissionDenied`, `UnsupportedTarget` and `UnsupportedProtocol` stay
  terminal**: those are settled questions, and retrying re-asks them.

**Three production defects the e2e surfaced, all fixed:**

- `enqueue`'s immediate try-then-queue attempt ran under the full call
  budget *and* the retry loop, so a guest calling a fire-and-forget verb
  against a down target blocked past the sandbox's own
  `dispatch_epoch_timeout_secs` (5 s) and was interrupted mid-call -- the
  guest **trapped** instead of getting "accepted for delivery". The probe
  now runs under its own 2 s bound.
- The outbox refused any caller with no `state.db`, because `queue_for`
  gated on `service_exists`. A guest that has never touched its own data
  layer has none, so the check refused exactly the callers the queue is
  for.
- The receiver-side dedup guard had the *same* mistake, and it was the one
  that actually stopped delivery: it used `service_exists` to mean "is a
  deployed service", so the target node refused every keyed delivery to a
  guest with no `state.db`. It now asks the endpoint registry, which is the
  authority for what a node hosts.

None of the three was reachable from an in-process test, which is the
argument for the e2e stated concretely rather than asserted.

**Exit criteria 5 and 13 are marked ✅** on the strength of the mechanisms
existing and being unit-tested, which is what those criteria ask for; the
e2e is corroborating evidence for the sequence, not the criterion itself.

**One further deviation, recorded for the same reason as the two above.**
The plan's `§0.22` specified the operator verbs but not how the control
plane would reach queues the router owns, and the crate dependency runs
router → control-plane, so the obvious handle is not available. A
`ProxyQueueInspector` trait was added to `syneroym-rpc` — the crate both
sides already share — and wired through a post-construction `OnceLock`
exactly as `ControlPlaneService::service_proxy` already is. Same shape,
same ordering reason, no new pattern.
