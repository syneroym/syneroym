# M05B Async Primitives — Status

**Milestone:** [task.md](task.md) · **Design of record:**
[ADR-0023](../../../decisions/0023-durable-async-primitives.md) · **Plan:**
[implementation-plan.md](implementation-plan.md)

**Overall:** 🚧 **B1 complete 2026-08-05**, all gates green (evidence below).
ADR-0023 accepted 2026-08-04. B2-B4 remain planned (sketch only).

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
