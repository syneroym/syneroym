# Slice B3 Implementation Plan — Scheduled Tasks

**Status:** 📋 Planned (2026-08-06). Not started. Milestone:
[task.md](task.md) slice **B3**; milestone-level plan:
[implementation-plan.md](implementation-plan.md) §0.7 and §1. Design of
record: [ADR-0023](../../../decisions/0023-durable-async-primitives.md) §6
(no lease, target selection plus overlap prevention) and §3 (queueability is
declared per action). Depends on **B1 — Complete 2026-08-05**;
[B2 — Complete 2026-08-05](status.md). Gates nothing.

**The one-sentence summary.** B3 lets a manifest say *when* a service's
method should run, and makes the supervisor — which is already the single
writer for an app instance — fire that method on exactly one member at the
right time, with nothing distributed anywhere.

**Read first:** the milestone plan's §0.4 (why there is no lease), §0.7 (why
a manifest surface comes before a clock), and D-B-3/D-B-5; ADR-0023 §3 (the
"intent expires" rule, which decides that a scheduled run is **never**
queued) and §6. From the shipped tree: `reconcile_instance_pass` and
`apply_write_phase` in
[app_supervisor/src/service.rs](../../../../crates/app_supervisor/src/service.rs),
which this slice adds a fifth work-list to, and `renewal_candidates` in the
same file, which is the shape B3's own selection function copies.

**Two decisions were taken with the requester (2026-08-06) before drafting**,
because both change the stored schema and neither is recoverable from the
docs:

- **A missed tick is skipped, not run late** (milestone plan §5 q1, open
  since 2026-08-04). §0.5 gives the exact rule this implies, which is *not*
  "next = cron.after(last_run)".
- **A schedule is written as a cron expression, evaluated in UTC.** §0.7
  flags the one library trap this carries.

**Everything in this document about "what B1/B2 built" was checked against
the shipped source**, not against their plans — B1's plan in particular
describes a design that shipped materially different after three review
rounds, and [status.md](status.md) says so.

---

## §0 — What the input documents leave open, understate, or state wrongly

### 0.1 (Scope-changing, blocking) There is no way for the supervisor to invoke a method on a deployed service, and "no new dispatch mechanism" is a claim about *shape*, not about a path that exists

The milestone plan's §0.7 says a schedule "names an interface and a method on
the service, which is the `(interface, method, params)` shape the proxy
already uses. **No new dispatch mechanism, one new declaration.**" Read as
"nothing has to be built to make the call", that is false in three
independent places on the shipped tree:

1. **`SubstrateActor` has no call action.** Its six methods are
   `apply_plan`, `write_bindings`, `restart`, `renew_cert`,
   `instance_identity`, `held_generation`
   ([sdk/src/deploy.rs:50](../../../../crates/sdk/src/deploy.rs#L50)). This
   is the *only* surface the supervisor's write phase acts through.
2. **The `orchestrator` interface has no call verb.** Twenty-odd functions
   in
   [control-plane.wit](../../../../crates/wit_interfaces/wit/control-plane/control-plane.wit)
   — deploy, undeploy, restart, renew-cert, write-bindings, status,
   proxy-outbox/dead-letters/replay — and not one of them invokes a method
   on a deployed service. **This is a statement about the verb list, not
   about the node**: `ControlPlaneService` itself already calls guest
   methods, for the `rpc` readiness probe. §0.10 reconciles with that
   precedent, which decides three details this section does not.
3. **`SyneroymClient` cannot address a member over the connection the pass
   already holds.** `service_id` is fixed at construction and resolved
   through a registry lookup in `connect`
   ([sdk/src/lib.rs:315](../../../../crates/sdk/src/lib.rs#L315)); the
   supervisor's clients are built per *substrate alias* from
   `SupervisorInventoryEntry` (`connected_client`,
   [service.rs:2109](../../../../crates/app_supervisor/src/service.rs#L2109)).
   Calling a member service would be a second client, a second registry
   resolution path, and a second connection per tick.

**Resolution (D-B3-1): one new `orchestrator` verb, `run-scheduled`, which
dispatches locally through the `ServiceProxy` handle the control plane
already holds.** `ControlPlaneService` carries
`service_proxy: OnceLock<Weak<dyn ServiceProxy>>`
([control_plane/src/service.rs:99](../../../../crates/control_plane/src/service.rs#L99))
with a `current_service_proxy()` accessor, wired post-construction by
`RouteHandler::init` — the same handle the FDAE cross-service fetch uses
([synsvc_native.rs:465](../../../../crates/control_plane/src/synsvc_native.rs#L465)).
`CallOrigin::Native`'s own doc names "control-plane internals" as one of its
three reasons for existing
([rpc/src/proxy.rs](../../../../crates/rpc/src/proxy.rs)). So the *dispatch*
is genuinely reused and §0.7's promise holds; what has to be added is the
one hop that gets the supervisor's decision to that dispatch.

**Why not the supervisor calling the member service directly**, which is the
other obvious design and is worth writing down so it is not re-proposed:

- **Authorization is wrong-shaped.** The supervisor holds `substrate/admin`
  on its own node and, through the inventory UCAN, `orchestrator/*` on each
  managed substrate. It holds nothing on the *app's* own interface, and the
  guest would see a caller DID it has no reason to trust. Through
  `run-scheduled` the caller the guest sees is
  `CallerContext::service_system(service_id)`
  ([rpc/src/native.rs:117](../../../../crates/rpc/src/native.rs#L117)) — the
  service acting as itself, which is what a scheduled tick actually is, and
  the same principal B2 settled on for a self-proxy (its F10 push-back).
- **It doubles the resolution surface** for no gain: a second registry
  lookup per member per tick, on a path the pass already has an open,
  authenticated connection for.

### 0.2 (Sets the manifest surface) Nothing can express a schedule, and *where* the field goes decides whether editing a cron string restarts a running service

Confirmed against the tree, as §0.7 says: `ServiceSpec` has four fields
([models.rs:520](../../../../crates/app_orchestration/src/models.rs#L520)),
`ServiceConfig` twelve
([models.rs:446](../../../../crates/app_orchestration/src/models.rs#L446)),
and no type anywhere carries a cron, interval, or time-of-day field.

The placement question the docs do not ask: **`ServiceConfig` is what
becomes the substrate-side `deploy-manifest`** (`map_deployment_plan_to_wit`,
[sdk/src/mapper.rs](../../../../crates/sdk/src/mapper.rs)), and the
substrate dedups an `apply_plan` by content hash over that manifest
(ADR-0023 §1). Putting `schedule` inside `ServiceConfig` therefore means
**editing a cron string changes the deploy hash**, so a resubmit reinstalls
the component, mints a fresh certificate, and restarts the service — for a
change the substrate has no use for at all.

**Resolution (D-B3-2): `schedule` is a field on `PlannedService`, beside
`substrate` and `topology_mode`, and is not mapped onto the wire** — the
same reason `mapper.rs` already gives for not mapping `substrate` ("a
substrate has no use for the placement of services it is not hosting").
The substrate learns *what* to run from `run-scheduled`'s own arguments, at
the moment it must run it, and never stores a schedule.

### 0.3 (Correctness) Adding a `PlannedService` field makes a schedule edit look like an `Update` to the reconciler, and the exclusion belongs **inside the shared classifier**, not at two call sites

`Reconciler::compute_diff` matches members by `MemberRef` and compares the
whole `PlannedService`, so a schedule-only edit emits
`ReconcileAction::Update` → a full redeploy, which the substrate then dedups
by content hash. The redeploy is wasted work at best and a certificate
re-mint at worst.

M05A hit exactly this for membership changes and answered it with
`only_resolved_dependencies_changed`
([service.rs:1937](../../../../crates/app_supervisor/src/service.rs#L1937))
and `membership_only_push_candidates`
([service.rs:1956](../../../../crates/app_supervisor/src/service.rs#L1956)),
and **its own second review round found the classifier applied on the loop's
path and not on `submit`'s**.

**A first draft of this plan said "apply the classifier in the `Update` arm
of `reconcile_instance_pass` and in `apply_with_membership_pushes`'s
equivalent". There is no equivalent, and that framing re-opens the exact gap
it cites M05A for.** The two paths exclude members by different means:

- The loop iterates `diff.actions` itself and skips adding a member to
  `needs_work` when `push_member_refs.contains(member_ref)`
  ([service.rs:1030](../../../../crates/app_supervisor/src/service.rs#L1030)),
  from the classifier call at
  [:1020](../../../../crates/app_supervisor/src/service.rs#L1020).
- `submit` never builds a `needs_work` set at all. It retains members out of
  the plan it is about to apply:
  `apply_plan.services.retain(|s| !push_member_refs.contains(...))`
  ([service.rs:2505](../../../../crates/app_supervisor/src/service.rs#L2505)),
  from the classifier call at
  [:2500](../../../../crates/app_supervisor/src/service.rs#L2500).

Both read the **same** `push_member_refs`, and both get it from the one
shared classifier. So the fix has exactly one site.

**Resolution (D-B3-3): a schedule-only member joins the exclusion set but
not the push list.** `membership_only_push_candidates` is renamed to
`classify_update_actions` and its first return value is renamed
`redeploy_exclusions` — it already means "members this pass must not
redeploy", and `push_member_refs` was only an accurate name while pushes
were the sole reason for exclusion:

```rust
/// Splits a diff's `Update` actions into (a) members no caller should
/// redeploy this pass and (b) the subset of those that need a binding
/// push. The two are not the same set: a member whose only change is its
/// schedule must not be redeployed (the substrate has no use for the
/// change) and has nothing to push either.
fn classify_update_actions(
    landed: &[ActionRecord],
    actions: &[ReconcileAction],
) -> (BTreeSet<String> /* redeploy_exclusions */, Vec<(PlannedService, String)>)
```

```
for Update { old, new } in actions:
    member_ref = new.member_ref()
    if only_schedule_changed(old, new):
        redeploy_exclusions.insert(member_ref)      // and nothing else
        continue
    if only_resolved_dependencies_changed(old, new)
       && let Some(row) = current_placement(landed, member_ref):
        redeploy_exclusions.insert(member_ref)
        push_candidates.push((new.clone(), row.substrate_did))
```

Note the asymmetry in the landed-placement check, which is deliberate: a
membership push needs a substrate to push *to*, so a never-landed member
falls through to the redeploy path. A schedule exclusion needs no substrate,
so it applies whether or not the member has landed.

Both existing call sites keep compiling unchanged, and neither can be fixed
without the other.

**Consequence, stated rather than discovered later:** a schedule-only diff is
excluded from `needs_work` and produces no push candidate either, so nothing
journals a new baseline and the same `Update` recurs on every pass. That is
**harmless and deliberate** — no code acts on it, and the schedule itself is
read from the stored desired-state plan, not from the diff. It is the same
resting state a membership-only push already leaves behind (a push pass
journals no baseline either), so this slice introduces no new class of
behaviour. A backlog row records it.

### 0.4 (Narrows the slice) The "lease" reduces further than ADR-0023 §6 says: the overlap half is already structural, and what is actually missing is a watermark

ADR-0023 §6 reduces the specified lease to **target selection** plus
**overlap prevention**, and calls the latter "one row in the supervisor's own
store". On the shipped tree the overlap half is narrower still:

- `run_pass` takes `instance_lock` and holds it for the whole of
  `reconcile_instance_pass`
  ([service.rs:784](../../../../crates/app_supervisor/src/service.rs#L784)),
  and B3's run is awaited inline inside that pass. Two passes for one
  instance cannot overlap at all, so two ticks for one schedule cannot
  either.
- The guest side is bounded independently: `dispatch_epoch_timeout_secs`
  caps every guest entry point at 5 seconds (milestone plan §0.5).

So **within one live supervisor process, overlap is prevented by
construction**, not by a row. What a row is genuinely needed for is the
*crash* case: a supervisor that dies after starting a run must not re-fire
the same tick on restart. That is a **watermark**, written before the call,
not a lease released after it — and the difference matters, because a lease
that is never released blocks the schedule forever, where a watermark that
was written for a run that then failed simply skips to the next occurrence,
which is what the requester's skip answer asks for anyway.

### 0.5 (Correctness) "Skip a missed tick" is not `cron.after(last_run)`, and the naive version silently becomes "run late"

The requester's answer is **skip**. The obvious implementation — store
`last_run_at`, compute `next = cron.after(last_run_at)`, fire when
`now >= next` — is *run-late*: a supervisor down for a day comes back, finds
`next` long past, and fires immediately. Every "skip" implementation needs a
window, not a next-due timestamp.

**The rule (D-B3-6).** Per schedule, store `evaluated_at`, a watermark
advanced on **every** pass that looks at the schedule, whether or not a run
happens. Each pass:

```
grace       = 2 * poll_interval_secs          // tolerate one dropped pass
window_start = max(evaluated_at, now - grace)
due          = window_start < now
               && the cron has at least one occurrence in (window_start, now]
```

and `evaluated_at = now` afterwards, unconditionally. Properties this buys,
each of which is a test:

- **A gap longer than `grace` fires nothing.** `window_start` clamps to
  `now - grace`, so an occurrence from hours ago is not in the window. This
  is the skip answer, exactly.
- **Several missed occurrences inside the window collapse to one run**, never
  a burst.
- **A pass that runs late by less than `grace` still fires its tick**, so
  ordinary jitter (a slow substrate delaying the previous pass) does not
  silently drop a nightly job.
- **A paused or retired instance needs no special case.**
  `reconcile_instance_pass` returns before evaluation for both
  ([service.rs:825](../../../../crates/app_supervisor/src/service.rs#L825)),
  so `evaluated_at` goes stale, and on resume the clamp makes the stale
  watermark irrelevant. Resuming a paused instance fires no backlog, with no
  code that knows what "paused" means.
- **A schedule seen for the first time never fires for the past**: the row is
  created with `evaluated_at = now` and produces no run on that pass.
- **A clock that goes backwards** makes `window_start >= now`, which is not
  due, and the watermark is re-stamped forward.

### 0.6 (Scope) A scheduled run is never queued, and no document says so

A reader who has just read B1 will reasonably assume a failed scheduled run
lands in the outbox. It must not, and the reason is already written down for
a different action: ADR-0023 §3's **"the intent expires"**, the rule that
makes `restart` non-queueable. A tick for 03:00 delivered at 06:00 runs work
whose window has passed, and the supervisor has a better retry than a queue —
**the next tick**. Recorded as D-B3-7 so the queue crate's absence from this
slice reads as a decision rather than an omission.

This also settles failure-matrix rows 10 and 11 without new machinery:
row 10 (a run outliving its interval) is §0.4's structural answer plus the
watermark; row 11 (a substrate partitioned from its supervisor) is
ADR-0023 §6's documented cost, and B3 adds the operator-visible half — the
`schedules` verb shows a `last_run_at` that has stopped advancing.

### 0.7 (Library trap, must be verified against source before use) The `cron` crate does not parse crontab syntax

The requester chose a cron expression. The obvious dependency, the `cron`
crate, takes a **six- or seven-field** expression with *seconds first*
(`sec min hour dom mon dow [year]`), so the crontab line an operator would
write — `0 3 * * *` — either fails to parse or parses as something else
entirely. That is a silent, five-characters-different footgun in a
user-facing manifest field.

**Use `croner`** (latest stable), which parses standard five-field Vixie
syntax and accepts an optional seconds field, and evaluate against
`chrono::Utc` — `chrono` is already a workspace dependency
([Cargo.toml](../../../../Cargo.toml)).

**This claim must be re-verified by reading the crate's own source before the
dependency is added** (`~/.cargo/registry/src/**/croner-*/src/`), not from
docs.rs prose — the same discipline B1's `addr_filter` correction had to
learn the hard way ([status.md](status.md), B1 evidence). If `croner`'s
five-field default turns out not to hold, the fallback is `cron` **plus a
documented six-field surface**, not a silent mismatch.

### 0.8 (Correctness) A manifest-time cap does not bind the interface that actually accepts a plan

D-B3-10 leans on `MAX_SCHEDULED_SERVICES` at manifest validation as what
keeps a pass bounded. `submit` takes a **compiled plan as JSON** and never
sees the manifest — the same hole `refuse_replicas_above_cap`
([service.rs:2202](../../../../crates/app_supervisor/src/service.rs#L2202))
exists to close, in its own words "the manifest-time cap re-checked at the
interface that actually accepts a compiled plan". This plan even admits the
gap elsewhere, in `schedule_decisions`' bad-cron branch ("can only come from
a hand-edited submission"), and then does not close it.

**Resolution: `refuse_schedules_above_cap(plan)`**, a sibling of
`refuse_replicas_above_cap`, called from the same two places it is
(`handle_submit` and `handle_force_reconcile`). It counts distinct
`logical_ref`s carrying a schedule and refuses above the cap. It does
**not** re-validate the cron string: a bad cron is per-schedule, is already
handled by `schedule_decisions`' watermark branch, and refusing a whole
submission over one unparseable field would take the instance out of
reconciliation entirely.

### 0.9 (Correctness, failure-matrix row 4a) Rotating the target across substrates leaves a `ScheduledRunFailed` alert nothing can clear

`AlertStore::raise`/`clear` key on `(instance, logical_ref, substrate_did)`
([alerts.rs:283](../../../../crates/app_orchestration/src/alerts.rs#L283)).
D-B3-5 rotates the run across members, and with `replicas > 1` those members
may sit on different substrates. So: a tick fails on substrate A and raises a
row under A; the next tick succeeds on B and clears **B's** row, which was
never raised. A's row stands forever. That is failure-matrix row 4a exactly
— "an alert nothing can clear trains operators to ignore the list" — and B1
had to answer the same shape for `DeliveryExhausted`.

**Resolution: `ScheduledRunFailed` is keyed under a constant sentinel, not
under the substrate that happened to run the tick.** The precedent is in this
same file: `NEVER_LANDED_SUBSTRATE_DID`
([service.rs:74](../../../../crates/app_supervisor/src/service.rs#L74))
exists because a per-substrate key was wrong for a fact that belongs to a
service rather than to a substrate. A schedule is such a fact — it belongs to
the logical service, which is why its state row is keyed that way too
(D-B3-4).

```rust
/// The `substrate_did` a scheduled-run alert is keyed under. A schedule
/// belongs to a logical service, and its target rotates across members
/// that may be on different substrates -- keying the alert under whichever
/// one ran the failing tick means the next tick's success clears a
/// different row and the failure stands forever.
const SCHEDULE_SUBSTRATE_DID: &str = "supervisor:schedule";
```

The substrate that actually ran the tick goes in the alert's `detail`, where
it is information rather than identity.

### 0.10 (Reconciles a precedent the first draft missed) The substrate already invokes a method on a deployed service, and it made three choices this slice must either follow or argue against

§0.1's bullet 2 is true of the `orchestrator` **verb list** and reads as a
claim about the node, which is wrong. `ControlPlaneService` already calls a
guest method: the `rpc` readiness probe resolves an interface and dispatches
through `AppSandboxEngine::execute_probe_json`
([orchestration.rs:2980](../../../../crates/control_plane/src/service/orchestration.rs#L2980)).
Nothing on the *supervisor's* side can reach it — that part of §0.1 stands,
and `run-scheduled` is still needed — but the probe made three decisions
worth reconciling rather than re-deciding by accident:

1. **`caller: None`**, with a comment calling it "still a substrate-
   originated probe, the same choice `ProxyRouter::invoke_local` makes".
   D-B3-1 picks `CallerContext::service_system(service_id)` and argued it
   only against "the supervisor calls the member directly". **The real
   reason it differs: `ProxyRequest.caller` is not an `Option`**
   ([rpc/src/proxy.rs](../../../../crates/rpc/src/proxy.rs)) — going through
   the proxy, "no caller" is not expressible, and `service_system` is the
   nearest true statement (the service acting as itself), which is also what
   B2 settled on for a self-proxy. A probe bypasses the proxy and so has the
   option; a scheduled run does not, and should not, because bypassing the
   proxy would restrict `run-scheduled` to WASM services.
2. **`params: Value::Array(vec![])`, not `Value::Null`.** The first draft sent
   `Null` for an absent `params-json`, which is a different wire shape from
   the one in-tree caller of a no-argument guest method. **Follow the
   precedent**: absent `params-json` sends `Value::Array(vec![])`. An explicit
   `params-json` is passed through verbatim.
3. **The probe is bounded by `probe_instance_permits`**, a concurrency pool
   added because a supervisor polling every few seconds multiplies by service
   count. **A scheduled run does not get a pool, and here is the argument
   rather than an omission:** a probe's rate is the *poll cadence* times the
   service count, where a scheduled run's rate is the cron occurrence count,
   which cannot exceed one per schedule per minute and is serialized per
   instance by `instance_lock`. It is also indistinguishable, on the
   receiving node, from any ordinary inbound proxy call — and those are not
   pooled either. So `run-scheduled` adds no load class that the node does
   not already accept. What is genuinely unbounded is *many supervisors ×
   many instances on one substrate*, and that is a backlog row (§5), not
   something to size a pool for before a first measurement.

### 0.11 (Reconciles a type the first draft missed) `ScheduleSpec` overlaps `RpcProbe`, and the overlap argues for one field, not for reuse

`RpcProbe` ([models.rs:396](../../../../crates/app_orchestration/src/models.rs#L396))
already carries `interface: InterfaceName`, `method: String`, and
`timeout_ms: u32` with a shared default. `ScheduleSpec` was drafted with the
first two and neither the third nor a reason.

**Not reused as a type**: a probe is a liveness question the substrate asks
on its own cadence, a schedule is work the supervisor asks for on the
operator's cadence, and they will diverge (a probe will want an expected
result, a schedule will want a delivery deadline). Embedding one in the other
couples two manifest surfaces for three shared fields.

**The third field is right; `RpcProbe`'s number is not.** `ScheduleSpec`
gains `timeout_ms: u32` so an app author can state a run's budget instead of
being cut off by a number in the supervisor's source — but with **its own
default function**, not the probe's:

```rust
/// A scheduled run's default budget. Deliberately *not*
/// `DEFAULT_PROBE_TIMEOUT_MS` (2 s): a probe is a liveness ping, and this
/// section's own argument is that a schedule is not a probe. 2 s is below
/// `dispatch_epoch_timeout_secs` (5 s), so inheriting it would have the
/// supervisor kill a run at 2 s that the guest's own budget permits to
/// run for 5 -- work the platform allows, stopped by the component that
/// asked for it. 10 s covers that budget plus a round trip.
pub const DEFAULT_SCHEDULE_TIMEOUT_MS: u32 = 10_000;
```

Taking the probe's 2 s would have been the exact mistake this section warns
about: arguing that the two are different and then copying the number
anyway.

**The ceiling is 30 seconds, not five minutes**, and the reason is what the
budget can actually buy. A guest cannot execute for longer than
`dispatch_epoch_timeout_secs` (5 s) whatever the supervisor is willing to
wait — a longer scheduled budget is time spent waiting on a hung substrate,
not on work, and a genuinely long-running task needs B5's third invocation
path (milestone plan §0.5), which is deferred. So:

```rust
/// A manifest's own `timeout_ms` decides one run's budget; this is the
/// ceiling it cannot exceed. 30 s rather than something generous, because
/// this call is awaited inline inside a reconcile pass that holds the
/// instance lock and runs instances one at a time -- so every second here
/// is a second every *other* app instance waits (`run_pass`'s own doc
/// calls that an accepted latency property). A budget above the guest's
/// own 5 s execution limit buys no extra work, only a longer wait on a
/// substrate that is not answering.
const SCHEDULED_RUN_CEILING: Duration = Duration::from_secs(30);
```

**The pass-level exposure, stated rather than left to be discovered.** With
`MAX_SCHEDULED_SERVICES = 16` all timing out at the ceiling, one instance can
hold its own lock for 8 minutes and delay the instances after it in the same
pass. That worst case needs sixteen schedules due in the same minute against
sixteen simultaneously unresponsive targets. It is bounded, it is stated
here, and it is a backlog row (§5) rather than a per-pass budget invented
before a measurement — but at five minutes the same worst case was **80
minutes**, which is longer than the outage it would be reporting.

### 0.12 Documents that are ambiguous or stale against the current code

| Where | What it says | What is true |
|---|---|---|
| [task.md](task.md) B3 row | "member selection" | No rule is given anywhere — not in task.md, not in the milestone plan, not in ADR-0023 §6, which says only "the supervisor picks one". The requirements spec's own words are "randomly or via load metrics". D-B3-5 chooses round-robin over *healthy* members and records why |
| [task.md](task.md) migration section | "**A manifest change.** B3 adds a schedule surface to `ServiceSpec`" | Correct as far as it goes, and incomplete: the compiled `PlannedService` needs the field too (§0.2), and that is what makes §0.3's classifier necessary |
| [task.md](task.md) migration section | "**Five new `SupervisorRole` config fields**", framed as B1's | Already corrected once by B2 (which added five to `AppSandboxRole` against a line saying it was untouched). **B3 adds none** — the grace window derives from `poll_interval_secs` and the per-run ceiling is a constant. Worth stating because the pattern so far is one config block per slice |
| [implementation-plan.md](implementation-plan.md) §3 | A new backlog row is owed for "a tick missed while the supervisor was down … whichever it chooses, the other is a row" | Discharged by this plan: skip is chosen, run-late is the row |
| `system-requirements-spec.md` `[PLT-ASY]` "Periodic & Scheduled Tasks" | Lease-based cluster scheduler, registry-held leases, delegated execution to a worker node | Superseded by ADR-0023 §6. Already scheduled for a dated implementation-status note at milestone closeout ([implementation-plan.md](implementation-plan.md) §3); **B3 adds no new doc debt here**, it is the slice that makes the existing note true |
| Project rule *No Planning-Doc References in Code* | Never cite milestone/slice ids or planning-doc section numbers in code | The surrounding supervisor code cites them constantly (`M05A A5c §19.7`, `D-A5e-7`, `M05B B1 review finding 5`). **New code in this slice must not**, even though its neighbours do: cite `ADR-0023 §3`/`§6`, or state the invariant with no reference at all. Do not cite `D-B3-N` either — a decision id is a planning-doc id |

---

## §1 — Decisions

| ID | Decision |
|---|---|
| **D-B3-1** | **The supervisor does not call the service; it asks the substrate to.** One new `orchestrator` verb, `run-scheduled`, dispatching locally through `ControlPlaneService`'s existing `ServiceProxy` handle with `CallOrigin::Native` and `CallerContext::service_system(service_id)` (§0.1) |
| **D-B3-2** | **`schedule` lives on `ServiceSpec` and on `PlannedService`, never inside `ServiceConfig` and never on the wire** (§0.2) |
| **D-B3-3** | **A schedule-only diff is excluded from redeploy inside the one shared classifier** — `membership_only_push_candidates`, renamed `classify_update_actions`, whose first return value both existing call sites already read (§0.3). Not a change at two call sites |
| **D-B3-4** | **A schedule belongs to the logical service, not to a member.** Stored state is keyed by `(app_instance_id, logical_ref)`; exactly one member runs a tick. Every member's `PlannedService` carries an identical clone of the spec, exactly as `resolved_dependencies` and `topology_mode` already do |
| **D-B3-5** | **Target selection is round-robin over the members this pass found `Healthy`**, persisted as `last_member_index`. Round-robin rather than "lowest index" because the spec's stated purpose for the lease is "to prevent load skew"; restricted to healthy members so one down member cannot silence the schedule; `Healthy` read from the pass's own `health::HealthReport`, so selection needs no extra poll — the same input `renewal_candidates` uses |
| **D-B3-6** | **Missed ticks are skipped, via a watermark and a grace window of `2 * poll_interval_secs`** (§0.5). `evaluated_at` is written on every pass, and `last_run_at` **before** the call, so a supervisor that dies mid-run skips rather than repeats |
| **D-B3-7** | **A scheduled run is never queued and never dead-letters** (ADR-0023 §3, §0.6). The next tick is the retry. A failed run raises `AlertKind::ScheduledRunFailed`, keyed under the `SCHEDULE_SUBSTRATE_DID` sentinel rather than under whichever member ran it (§0.9, failure-matrix row 4a), and cleared by the next successful run |
| **D-B3-8** | **`run-scheduled` takes the same gate as `restart`** — `orchestrator/deploy` on `substrate:{node}/app/{service_id}`, the owner check, and the app-instance generation check. No new ability, no new resource namespace. The privilege question this raises is answered in §4 |
| **D-B3-9** | **`SubstrateActor::run_scheduled` ships with a refusing default body**, so the **seven** existing test fakes do not have to grow a method they never call — the shape B2 used for `ServiceProxy::enqueue`. The tree has nine `impl SubstrateActor`: two real (`SyneroymClient`, `DurableActor`) and seven fakes. Both real ones implement it; `DurableActor` forwards it without queuing (D-B3-7) |
| **D-B3-10** | **Zero new `SubstrateConfig` fields.** Grace derives from `poll_interval_secs`; the per-run bound is a *manifest* field, `ScheduleSpec.timeout_ms`, reusing `RpcProbe`'s field but **not** its 2 s default (§0.11), under an absolute `SCHEDULED_RUN_CEILING` of 30 s — the run is awaited inline in a pass other instances queue behind, and a longer budget buys no work the guest's own 5 s limit allows; the number of scheduled services is capped at `MAX_SCHEDULED_SERVICES` (16, following `MAX_REPLICAS`) **and re-checked at `submit`** (§0.8), which is what keeps a pass bounded without a per-pass cap |
| **D-B3-11** | **An operator surface ships in this slice, not after it.** B1's post-landing review finding 13 was "no RPC surface for the outbox itself", and its own e2e could not assert what it claimed until that verb existed. B3 ships `schedules` on `supervisor.wit` and `roymctl supervisor schedules`, and its e2e asserts through them |
| **D-B3-12** | **The e2e drives a countable guest.** A new `test-components/scheduled-test` component with `tick`/`tick-count` exports backed by its own data layer, so "ran exactly once" is observable from the test process. This also closes the fixture gap B2's F19 push-back recorded ([status.md](status.md)) |

---

## §2 — Phases

Merge order is phase order. Each phase compiles and its own tests pass on its
own.

### Phase 1 — the manifest and plan surface (`syneroym-app-orchestration`)

**New dependency.** `croner` (latest stable) in `[workspace.dependencies]`
and in `crates/app_orchestration/Cargo.toml`, after §0.7's source check.
`chrono` is already a workspace dependency and is inherited the same way.

**New type**, in a new file `crates/app_orchestration/src/schedule.rs`,
re-exported from `models.rs` alongside its siblings:

```rust
/// When a service's method runs, and which method. Declared per logical
/// service; exactly one member runs a given tick.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScheduleSpec {
    /// Standard five-field cron, evaluated in UTC. UTC rather than a
    /// configurable zone because a zone makes every occurrence a
    /// DST question, and there is no operator-facing zone setting
    /// anywhere else in a manifest to be consistent with.
    pub cron: String,
    /// The interface the method is exported on. Must be one of the
    /// service's own declared `interfaces`.
    pub interface: InterfaceName,
    pub method: String,
    /// JSON text passed verbatim as the call's params. Absent sends an
    /// empty positional array -- the shape the one existing in-tree caller
    /// of a no-argument guest method sends (the `rpc` readiness probe),
    /// not `null` (§0.10).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<String>,
    /// How long one run may take. Same *field* as `RpcProbe.timeout_ms`
    /// and a different default: a probe's 2 s is a liveness ping and is
    /// below the guest's own 5 s execution budget (§0.11). Clamped at
    /// `SCHEDULED_RUN_CEILING`, since this call is awaited inside a
    /// reconcile pass that other app instances are queued behind.
    #[serde(default = "default_schedule_timeout_ms")]
    pub timeout_ms: u32,
}

impl ScheduleSpec {
    /// Parses `cron`. Fails with the parser's own message, which names the
    /// offending field.
    pub fn parsed(&self) -> Result<croner::Cron>;
}

/// Whether `cron` has at least one occurrence in `(after, until]`, both
/// Unix seconds, UTC. Pure; the whole of D-B3-6's due-ness test.
pub fn has_occurrence_in(cron: &croner::Cron, after: u64, until: u64) -> Result<bool>;
```

`has_occurrence_in` is the one non-obvious function:

```
if after >= until { return Ok(false) }              // clock went backwards
let next = cron.find_next_occurrence(Utc.timestamp(after), /* inclusive = */ false)?;
Ok(next.timestamp() as u64 <= until)
```

*(`croner`'s exact method name and inclusivity flag are to be read off its
source, not guessed — §0.7. The contract this file must implement is the
half-open interval above, whatever the call spells.)*

**Changed types:**

- `ServiceSpec` ([models.rs:520](../../../../crates/app_orchestration/src/models.rs#L520))
  gains, after `replicas`:
  ```rust
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub schedule: Option<ScheduleSpec>,
  ```
- `PlannedService` ([models.rs:680](../../../../crates/app_orchestration/src/models.rs#L680))
  gains the identical field, after `member_index`.

Both `skip_serializing_if`, so an unscheduled manifest's TOML and an
unscheduled plan's JSON are byte-for-byte unchanged — the property
`replicas` and `member_index` already hold and that
`a_manifest_without_replicas_compiles_to_one_member_at_index_zero` and its
siblings assert.

**`SynAppManifest::validate()`**
([models.rs:588](../../../../crates/app_orchestration/src/models.rs#L588)),
a fourth check after the existing three:

```
let mut scheduled = 0;
for (name, spec) in &self.services {
    let Some(sched) = &spec.schedule else { continue };
    scheduled += 1;
    sched.parsed()?;                                     // cron parses
    if !spec.config.interfaces.contains(&sched.interface) {
        bail!("service '{name}' schedules '{iface}/{method}' but does not declare interface '{iface}'")
    }
    if sched.method.trim().is_empty() { bail!(...) }
    if let Some(p) = &sched.params { serde_json::from_str::<Value>(p)?; }
}
if scheduled > MAX_SCHEDULED_SERVICES { bail!(...) }      // D-B3-10
```

The interface check is the cheap half of the same idea B4 owes for
`undo-`: a convention nothing checks is a comment. It cannot check that the
*method* exists — the manifest names an artifact, not a parsed WIT — and
that is stated in the field's doc rather than left as an implied promise.

**`compile_recursive`**
([compiler.rs:163](../../../../crates/app_orchestration/src/compiler.rs#L163)):
one line inside the `for member_index in 0..spec.replicas` loop,
`schedule: spec.schedule.clone(),`, beside `topology_mode` — every member
carries the same clone (D-B3-4).

**Call sites that must be updated.** Neither struct derives `Default` and no
literal uses `..Default::default()`, so every one of these is a compile
error, not a silent miss — the same shape B2's 23-site `ProxyRequest` sweep
took. Counted on the tree at 2026-08-06:

| Struct | Files | Literals |
|---|---|---|
| `PlannedService` | 12: `app_orchestration/src/{compiler,journal,reconcile,substrate_inventory,models}.rs`, `app_supervisor/src/{keys,service}.rs`, `control_plane/src/service/orchestration.rs`, `sdk/src/{deploy,mapper}.rs`, `roymctl/src/commands/{app,member_identity}.rs` | **28** (`mapper.rs` 4, `service.rs` 4, `models.rs` 5) |
| `ServiceSpec` | **10**, of which **8 are e2e test files under `crates/substrate/tests/`** (`binding_push`, `multi_substrate_placement`, `supervisor_interface`, `reference_scenario`, `supervisor_alerts`, `durable_outbox`, `supervisor_loop`, `app_instance_identity`), plus `app_orchestration/src/catalog.rs` and `roymctl/src/commands/app.rs` | **17** |

**45 literals, not 48**: a raw `rg 'PlannedService \{'` also matches the
`pub struct` and `impl` lines in `models.rs`, and `rg 'ServiceSpec \{'`
matches its `pub struct` line — `models.rs` declares `ServiceSpec` and never
constructs one.

The `ServiceSpec` half matters for planning more than for correctness: eight
of those files are in the pre-existing sandbox-bind e2e category
([status.md](status.md)), so they compile in every `cargo test --workspace`
run but are not *run* under the sandbox. A `cargo check --workspace
--all-targets` is what catches them early; do not wait for a test run.

```bash
rg -n 'PlannedService \{|ServiceSpec \{' --type rust
```

**Tests (phase 1):**

- `a_manifest_with_no_schedule_serializes_byte_for_byte_as_before`
- `a_scheduled_service_round_trips_through_toml_and_json`
- `a_schedule_naming_an_undeclared_interface_is_refused_at_validation`
- `a_schedule_with_an_unparseable_cron_is_refused_at_validation`
- `a_schedule_whose_params_are_not_json_is_refused_at_validation`
- `more_than_the_cap_of_scheduled_services_is_refused_at_validation`
- `every_member_of_a_scaled_scheduled_service_carries_the_same_schedule`
- `has_occurrence_in_is_true_for_a_daily_cron_across_its_own_hour`
- `has_occurrence_in_is_false_for_a_window_that_misses_the_occurrence`
- `has_occurrence_in_is_false_when_the_window_is_empty_or_inverted`
- `a_five_field_crontab_line_parses_as_the_hour_an_operator_meant` — the
  §0.7 regression, pinning that `"0 3 * * *"` means 03:00 and not 03:00:00's
  second
- `a_six_field_expression_is_read_as_seconds_first` — the other half of
  §0.7, kept as a unit test rather than shortening the e2e with it (phase 5)
- `a_schedule_timeout_defaults_above_the_guests_own_execution_budget` —
  §0.11's number, pinned as a relationship (`DEFAULT_SCHEDULE_TIMEOUT_MS >
  dispatch_epoch_timeout_secs`) rather than as a literal, so the two cannot
  drift into the supervisor killing runs the guest is allowed to finish

### Phase 2 — the invocation path (WIT → control plane → SDK)

**`control-plane.wit`**, in the `orchestrator` interface next to `restart`
([control-plane.wit:218](../../../../crates/wit_interfaces/wit/control-plane/control-plane.wit#L218)):

```wit
    /// Run one scheduled tick against a deployed service: dispatch
    /// `method` on `interface` locally, as the service itself. The
    /// caller decides *when*; this node only executes. Never queued
    /// (ADR-0023 §3) -- a tick whose window has passed is not worth
    /// delivering late, and the caller's next tick is the retry.
    ///
    /// `generation` follows `restart`'s rule: gated only where an app
    /// instance exists, so a superseded supervisor cannot keep firing
    /// ticks at an instance another one now manages.
    ///
    /// The dispatched method's own return value is discarded. There is
    /// nobody to return it to, and a scheduled run that produces a value
    /// nothing reads is a design mistake in the app, not a wire shape.
    run-scheduled: func(
        service-id: string,
        generation: u64,
        %interface: string,
        method: string,
        params-json: option<string>,
    ) -> result<_, string>;
```

**`OrchestrationApi`**
([control_plane/src/service/orchestration.rs:120](../../../../crates/control_plane/src/service/orchestration.rs#L120)),
a new trait method beside `restart`, with the matching impl at
[:1033](../../../../crates/control_plane/src/service/orchestration.rs#L1033)
delegating to `run_scheduled_impl`:

```rust
async fn run_scheduled(
    &self,
    service_id: String,
    generation: u64,
    interface: String,
    method: String,
    params_json: Option<String>,
    caller: &CallerContext,
) -> Result<(), String>;
```

**`run_scheduled_impl`**, beside `restart_impl`
([:2241](../../../../crates/control_plane/src/service/orchestration.rs#L2241)),
whose first three blocks it copies verbatim in intent (D-B3-8):

```
// Same gate as `restart`: this is a lifecycle write, not a service call.
deploy_resource = "substrate:{node_did}/app/{service_id}"
refuse unless caller.has_capability(deploy_resource, orchestrator/deploy)

// Same owner check `restart_impl` carries, and for the same reason.
if let Some(owner) = registry.owner_of(&service_id)
   && owner != caller.caller_did
   && !has_node_wide_ability(caller, ORCHESTRATOR_DEPLOY) { refuse }

// Same generation gate, only where an app instance exists.
if let Some((instance, _)) = registry.app_context_of(&service_id) {
    let management = check_generation(&instance, caller, generation)?;
    registry.set_app_instance_management(instance, management).await?;
}

let params = match params_json {
    Some(text) => serde_json::from_str(&text).map_err(|e| format!("params-json is not JSON: {e}"))?,
    // An empty positional array, not `Value::Null` -- the shape the one
    // existing in-tree caller of a no-argument guest method sends (§0.10).
    None => Value::Array(vec![]),
};
let proxy = self.current_service_proxy().upgrade()
    .ok_or("service proxy unavailable for a scheduled run")?;
proxy.call(ProxyRequest {
    target_service: service_id.clone(),
    interface, method, params,
    caller: CallerContext::service_system(&service_id),
    origin: CallOrigin::Native { service_id: Some(service_id) },
    protocol: ProxyProtocol::JsonRpcV1,
    idempotent: false,          // a tick is not safe to repeat by default
    idempotency_key: None,      // and therefore never fenced or replayed
    timeout: None,              // the proxy's own default; the guest's
                                // epoch budget is the real ceiling
}).await.map(|_| ()).map_err(|e| e.to_string())
```

A target that is absent or not callable surfaces as the proxy's own
`ServiceNotFound`/`UnsupportedTarget`, so no extra existence check is
written here — the resolution the proxy already performs is the check.

**Dispatch arm** in `ControlPlaneService`'s `orchestrator` match
([control_plane/src/service.rs:492](../../../../crates/control_plane/src/service.rs#L492)
is `restart`'s), accepting both the positional tuple and the kebab-named
object shape `proxy-replay` established
([:567](../../../../crates/control_plane/src/service.rs#L567)):

```rust
"run-scheduled" => {
    let (service_id, generation, interface, method, params_json):
        (String, u64, String, String, Option<String>) = /* tuple, then named fallback */;
    self.run_scheduled(service_id, generation, interface, method, params_json, &invocation.caller)
        .await
        .map_err(RpcError::InternalError)?;
    Ok(NativeResponse { payload: serde_json::json!({"status": "ran"}) })
}
```

`test_wit_adherence`
([control_plane/src/service.rs:782](../../../../crates/control_plane/src/service.rs#L782))
fails without this arm — that is the mechanism, not an extra test to write.

**`SubstrateActor`**
([sdk/src/deploy.rs:50](../../../../crates/sdk/src/deploy.rs#L50)), a
seventh method **with a default body** (D-B3-9):

```rust
    /// Run one scheduled tick on a deployed member. Never queued: the
    /// intent expires (ADR-0023 §3), and the schedule's next tick is a
    /// better retry than a delivery hours later.
    ///
    /// Defaulted because most implementations of this trait are fakes for
    /// control flow that has nothing to do with scheduling; an
    /// implementation that means to run ticks overrides it.
    async fn run_scheduled(
        &self,
        _service_id: String,
        _generation: u64,
        _interface: String,
        _method: String,
        _params_json: Option<String>,
    ) -> Result<(), String> {
        Err("this actor does not run scheduled tasks".to_string())
    }
```

- `impl SubstrateActor for SyneroymClient`
  ([:88](../../../../crates/sdk/src/deploy.rs#L88)): builds the five-tuple
  and calls `self.request("orchestrator", "run-scheduled", params)`,
  checking `{"status": "ran"}`, exactly as `restart` does at
  [:113](../../../../crates/sdk/src/deploy.rs#L113).
- `impl SubstrateActor for DurableActor<T>`
  ([:278](../../../../crates/sdk/src/deploy.rs#L278)): forwards to the
  inner actor with **no** outbox involvement, with a comment saying why
  (ADR-0023 §3, same sentence `restart`'s forward already carries).
- The **seven** fakes are untouched: `CountingActor`, `BindingActor`,
  `RenewalActor` and `FakeSubstrateClient` in `app_supervisor`'s tests,
  `FailingApplier` and `DurableTestActor` in `sdk`'s, `NoopApplier` in
  `roymctl`. Nine `impl SubstrateActor` in total, matching D-B3-9.

**Tests (phase 2):**

- `run_scheduled_is_refused_without_an_orchestrator_deploy_grant`
- `run_scheduled_is_refused_for_a_service_another_caller_owns`
- `run_scheduled_is_refused_at_a_stale_generation`
- `run_scheduled_dispatches_the_named_method_as_the_service_itself` —
  asserts the `CallerContext` the target observes is
  `system:<service_id>`, which is the whole of §0.1's authorization argument
- `run_scheduled_passes_absent_params_as_an_empty_positional_array` — the
  name states §0.10's decision, so a later change back to `null` has to
  argue with a test name rather than slip past one
- `run_scheduled_refuses_params_json_that_is_not_json`
- `run_scheduled_reports_a_callee_error_rather_than_swallowing_it`
- `a_durable_actor_runs_a_scheduled_tick_without_touching_the_queue` (in
  `sdk`, beside `build_durable_actor`'s existing five)

### Phase 3 — evaluation, selection, and the fifth work-list (`syneroym-app-supervisor`)

**Store** ([store.rs](../../../../crates/app_supervisor/src/store.rs)), a
seventh table in the `CREATE TABLE IF NOT EXISTS` block at
[:118](../../../../crates/app_supervisor/src/store.rs#L118) (no
`ALTER TABLE`, so no A7-style idempotent-add-column dance):

```sql
CREATE TABLE IF NOT EXISTS scheduled_runs (
    app_instance_id   TEXT    NOT NULL,
    logical_ref       TEXT    NOT NULL,   -- LogicalServiceRef, not MemberRef
    evaluated_at      INTEGER NOT NULL,
    last_run_at       INTEGER,
    last_member_index INTEGER NOT NULL DEFAULT 0,
    last_error        TEXT,
    PRIMARY KEY (app_instance_id, logical_ref)
)
```

with:

```rust
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScheduleState {
    pub evaluated_at: i64,
    pub last_run_at: Option<i64>,
    pub last_member_index: u32,
    pub last_error: Option<String>,
}

impl SupervisorStore {
    /// Every schedule state this instance has, keyed by logical ref.
    pub fn schedule_states(&self, app_instance_id: &str)
        -> Result<BTreeMap<String, ScheduleState>>;
    /// Advances the watermark and nothing else -- a pass that looked and
    /// found nothing due.
    pub fn record_schedule_evaluated(&self, app_instance_id: &str, logical_ref: &str, at: i64)
        -> Result<()>;
    /// Watermark, run time and selected member in one statement, written
    /// **before** the call so a crash mid-run skips rather than repeats.
    pub fn record_schedule_started(&self, app_instance_id: &str, logical_ref: &str,
                                   at: i64, member_index: u32) -> Result<()>;
    /// Sets or clears `last_error` after the call returns.
    pub fn record_schedule_outcome(&self, app_instance_id: &str, logical_ref: &str,
                                   error: Option<&str>) -> Result<()>;
}
```

Rows are deleted alongside the instance's other state wherever
`clear_remediation_for_instance` is called on retire/release — find the call
sites and match them, rather than leaving a table that only grows.

**Alerts** ([alerts.rs:29](../../../../crates/app_orchestration/src/alerts.rs#L29)):
`AlertKind::ScheduledRunFailed`, with its `Display` arm
(`"SCHEDULED_RUN_FAILED"`, [:118](../../../../crates/app_orchestration/src/alerts.rs#L118))
and `FromStr` arm ([:131](../../../../crates/app_orchestration/src/alerts.rs#L131)).
One standing row per `(instance, logical_ref, SCHEDULE_SUBSTRATE_DID)` —
**the sentinel, not the member's own substrate** (§0.9), so a failure on one
member and a success on another under `replicas > 1` touch the same row.
Cleared by the next successful run for that logical ref (D-B3-7). The
existing round-trip test over every variant covers the two new arms.

**Selection**, a pure function beside `renewal_candidates`
([service.rs:1437](../../../../crates/app_supervisor/src/service.rs#L1437)) —
pure for the same reason that one is: the whole rule is testable with a
fixed `now`, no vault, no client, no store.

```rust
/// What this pass owes one schedule.
enum ScheduleDecision {
    /// Looked, nothing due (or nothing runnable). Advance the watermark
    /// only -- this is what makes a missed tick a skip rather than a
    /// backlog (ADR-0023 §6's overlap half, applied to time).
    Watermark { logical_ref: String },
    Run {
        logical_ref: String,
        service_id: String,
        substrate_did: String,
        member_index: u32,
        schedule: ScheduleSpec,
    },
}

fn schedule_decisions(
    plan: &DeploymentPlan,
    states: &BTreeMap<String, ScheduleState>,
    report: &health::HealthReport,
    now: u64,
    grace_secs: u64,
) -> Vec<ScheduleDecision>
```

**No `landed` argument.** A first draft passed one, purely to call
`deploy::current_placement` for the target's substrate DID. It is redundant:
`ServiceHealth` already carries `substrate_did` **and** `member_index`
([sdk/src/health.rs:93](../../../../crates/sdk/src/health.rs#L93)), from the
same report this function already takes, and a member with `Signal::Healthy`
is by definition one the sweep reached on a real substrate. Reading the
placement from the report keeps the function a pure fold over one input
rather than a join across two that could disagree.

```
group plan.services by logical_ref, keeping only groups whose members carry a schedule
for (l_ref, members) in groups:                       // BTreeMap order: deterministic
    let sched = members[0].schedule                   // identical across members, D-B3-4
    let Ok(cron) = sched.parsed() else {
        // A plan that got past validation with a bad cron can only come
        // from a hand-edited submission. Watermark and move on; the
        // failure is reported by the alert the run would have raised.
        push Watermark; continue
    }
    let Some(state) = states.get(l_ref) else { push Watermark; continue }   // first sight
    let window_start = max(state.evaluated_at as u64, now.saturating_sub(grace_secs))
    // No `?`: this function returns a plain Vec. An unreadable cron is
    // treated exactly as the parse failure above -- watermark and move on.
    if has_occurrence_in(&cron, window_start, now) != Ok(true) { push Watermark; continue }

    // The report is the single source for "which members are runnable and
    // where they are": a Healthy signal carries the substrate that answered.
    let healthy: Vec<&ServiceHealth> = report.services
        .filter(|h| h.logical_ref == l_ref && h.signal == Signal::Healthy)
        .sorted by member_index
    if healthy.is_empty() { push Watermark; continue }   // a skipped tick, not a late one

    // Round-robin: the first member strictly after the last one used,
    // wrapping. A member that has gone away simply drops out of the ring.
    let pick = healthy.iter().find(|h| h.member_index > state.last_member_index)
                      .unwrap_or(healthy[0]);
    push Run { logical_ref: l_ref, service_id: pick.service_id,
               substrate_did: pick.substrate_did, member_index: pick.member_index,
               schedule: sched }
```

**Wiring in `reconcile_instance_pass`**, after `renewal_candidates` is
computed ([service.rs:1089](../../../../crates/app_supervisor/src/service.rs#L1089)):

```rust
let schedule_states = self.store.schedule_states(app_instance_id).unwrap_or_default();
let schedule_decisions = Self::schedule_decisions(
    &plan, &schedule_states, &report, now, 2 * self.poll_interval_secs,
);
```

and `|| !schedule_decisions.is_empty()` added to the write-phase guard at
[:1131](../../../../crates/app_supervisor/src/service.rs#L1131). A
`Watermark` decision is a store write and therefore counts as work.

**`WritePhase`** ([service.rs:89](../../../../crates/app_supervisor/src/service.rs#L89))
gains `schedule_decisions: &'a [ScheduleDecision]`, and
`apply_write_phase` calls the runner **after** `refresh_due_master_anchors`
and **before** `publish_opened_alerts`
([:1424](../../../../crates/app_supervisor/src/service.rs#L1424)), so the
pass finishes its reconciliation work before spending time on app work, and
so a scheduled run's alert is published with the rest:

```rust
self.run_due_schedules(
    instance_id, app_instance_id, &schedule_decisions,
    did_to_alias, &Self::actors_from_clients(clients),
    fresh_state.generation, now, &mut opened,
).await;
```

`actors_from_clients` (not `durable_actor`) because a scheduled run is never
queued — the same call `renew_due_members` already makes
([:1405](../../../../crates/app_supervisor/src/service.rs#L1405)).

**`run_due_schedules`:**

```
// `SCHEDULED_RUN_CEILING` (30 s) and `DEFAULT_SCHEDULE_TIMEOUT_MS` (10 s),
// both §0.11.

for decision in decisions:
  match decision:
    Watermark { logical_ref } =>
        store.record_schedule_evaluated(app_instance_id, logical_ref, now)   // log on Err

    Run { logical_ref, service_id, substrate_did, member_index, schedule } =>
        let Some(alias) = did_to_alias.get(substrate_did) else {
            store.record_schedule_evaluated(..); continue }        // skip, not late
        let Some(actor) = actors.get(&SubstrateAlias::new(alias)) else {
            store.record_schedule_evaluated(..); continue }

        // Before the call, not after: a supervisor that dies inside the
        // call must skip this tick on restart, not repeat it.
        store.record_schedule_started(app_instance_id, logical_ref, now, member_index)?

        let budget = min(schedule.timeout_ms, SCHEDULED_RUN_CEILING);
        let outcome = timeout(budget, actor.run_scheduled(
            service_id, generation, schedule.interface, schedule.method, schedule.params)).await;

        match outcome:
          Ok(Ok(())) =>
              store.record_schedule_outcome(.., None)
              // The sentinel, never `substrate_did` -- §0.9.
              alerts.clear(instance, Some(logical_ref), SCHEDULE_SUBSTRATE_DID, ScheduledRunFailed)
          Ok(Err(e)) | Err(_elapsed) =>
              store.record_schedule_outcome(.., Some(&detail))
              // `detail` names the substrate that ran it; the key does not.
              if alerts.raise(instance, Some(logical_ref), None, SCHEDULE_SUBSTRATE_DID,
                              ScheduledRunFailed, &detail)? { opened.push(..) }
```

The two early `continue` branches deliberately advance the watermark rather
than leaving it: the tick's window has passed and the substrate was not
reachable, which is failure-matrix row 11's documented cost, not a delivery
to retry. This is the one place where B3 differs in shape from B1's
`enqueue_unreachable_push` — and the difference is exactly ADR-0023 §3.

**Phase-3 tests** (in `service.rs`'s test module, against a fake actor and
an in-memory store, the shape the renewal and restart tests already use):

- `a_schedule_seen_for_the_first_time_does_not_fire_for_the_past`
- `a_due_schedule_runs_exactly_one_member`
- `a_schedule_evaluated_twice_inside_one_cron_minute_runs_once`
- `a_tick_missed_while_the_supervisor_was_down_is_skipped_not_run_late` —
  the requester's decision, pinned
- `a_pass_delayed_by_less_than_the_grace_window_still_runs_its_tick`
- `a_paused_instance_fires_no_backlog_when_it_resumes`
- `selection_rotates_across_healthy_members_on_consecutive_ticks`
- `an_unhealthy_member_is_never_selected_and_does_not_block_the_schedule`
- `a_schedule_with_no_healthy_member_advances_its_watermark_and_skips`
- `a_failed_run_raises_scheduled_run_failed_and_the_next_success_clears_it`
- `a_failed_run_is_never_enqueued_onto_the_outbox` — asserts the queue is
  empty, the direct statement of D-B3-7 and of the happy-path budget
- `the_run_is_recorded_before_the_call_so_a_crash_mid_run_skips_the_tick`
- `a_superseded_instance_runs_no_scheduled_work` (the existing superseded
  early-return, asserted for this work-list too)
- `a_schedule_only_edit_does_not_redeploy_the_service` (loop path)
- `a_schedule_only_edit_on_submit_does_not_redeploy_the_service` (the §0.3
  second path — the one M05A's review found missing the first time)
- `a_schedule_only_update_is_excluded_from_redeploy_but_is_not_a_push_candidate`
  — directly on `classify_update_actions`, the one place §0.3's fix lives,
  asserting both halves of its return value
- `a_simultaneous_schedule_and_membership_edit_is_not_classified_as_membership_only`
- `a_failure_on_one_member_is_cleared_by_a_success_on_another_members_substrate`
  — §0.9 / failure-matrix row 4a, and the reason the sentinel exists.
  Confirm it fails when the alert is keyed on the member's own substrate
- `refuse_schedules_above_cap_refuses_a_submitted_plan_above_the_cap` and
  `..._allows_a_plan_exactly_at_the_cap` — §0.8, mirroring the two
  `refuse_replicas_above_cap` tests already there
  ([:5867](../../../../crates/app_supervisor/src/service.rs#L5867))
- `a_schedule_timeout_is_clamped_to_the_ceiling`

**The classifier change (§0.3), one site, both paths.**
`only_schedule_changed`, beside `only_resolved_dependencies_changed`
([:1937](../../../../crates/app_supervisor/src/service.rs#L1937)):

```rust
fn only_schedule_changed(old: &PlannedService, new: &PlannedService) -> bool {
    old.service_id == new.service_id
        && old.substrate == new.substrate
        && old.config == new.config
        && old.topology_mode == new.topology_mode
        && old.member_index == new.member_index
        && old.resolved_dependencies == new.resolved_dependencies
        && old.schedule != new.schedule
}
```

fed into the renamed `classify_update_actions` per §0.3's pseudo-code.

**The rename has five call sites, not two.** Two production
(`reconcile_instance_pass` [:1020](../../../../crates/app_supervisor/src/service.rs#L1020),
`apply_with_membership_pushes` [:2500](../../../../crates/app_supervisor/src/service.rs#L2500))
and **three in tests** ([:6764](../../../../crates/app_supervisor/src/service.rs#L6764),
[:7098](../../../../crates/app_supervisor/src/service.rs#L7098),
[:7333](../../../../crates/app_supervisor/src/service.rs#L7333)), plus three
doc comments naming it
([:1014](../../../../crates/app_supervisor/src/service.rs#L1014),
[:2471](../../../../crates/app_supervisor/src/service.rs#L2471),
[:6721](../../../../crates/app_supervisor/src/service.rs#L6721), the last of
which calls it "the same classifier"). The compiler catches the calls; it
does not catch that those three tests now bind a set whose meaning has
widened from "members to push" to "members not to redeploy". **Their names
and comments are part of this rename**, not collateral — a test asserting
`push_member_refs` that silently keeps its name is how the next reader
concludes the set still means only pushes.

`only_resolved_dependencies_changed` must **also** gain
`old.schedule == new.schedule`, or a simultaneous schedule + membership edit
classifies as membership-only and the schedule change is silently dropped
from the plan the pass records.

**The submit-side cap (§0.8)**, beside `refuse_replicas_above_cap`
([:2202](../../../../crates/app_supervisor/src/service.rs#L2202)) and called
from the same two sites:

```rust
fn refuse_schedules_above_cap(plan: &DeploymentPlan) -> Result<(), String> {
    let scheduled: BTreeSet<&LogicalServiceRef> = plan.services.iter()
        .filter(|s| s.schedule.is_some()).map(|s| &s.logical_ref).collect();
    if scheduled.len() > MAX_SCHEDULED_SERVICES { return Err(...) }
    Ok(())
}
```

### Phase 4 — the operator surface

**`supervisor.wit`**, a record beside `outbox-item`
([supervisor.wit:231](../../../../crates/wit_interfaces/wit/supervisor/supervisor.wit#L231))
and a verb beside `outbox`:

```wit
    /// One schedule this instance declares, with what the supervisor has
    /// most recently done about it. `last-run-at` that has stopped
    /// advancing while `evaluated-at` keeps moving is the visible form of
    /// a substrate the supervisor cannot reach (ADR-0023 §6's stated
    /// cost).
    record scheduled-task {
        logical-ref: string,          // the logical service, not a member
        cron: string,
        %interface: string,
        method: string,
        evaluated-at: s64,
        last-run-at: option<s64>,
        last-member-index: u32,
        last-error: option<string>,
    }

    /// Every schedule this instance declares, in logical-ref order.
    schedules: func(app-instance-id: string) -> result<list<scheduled-task>, string>;
```

**`handle_schedules`** in `service.rs`, modelled line for line on
`handle_outbox` ([:4259](../../../../crates/app_supervisor/src/service.rs#L4259)):
`require_admin`, parse the one-tuple, read the stored plan for the
declarations and `schedule_states` for the state, left-join them, and
serialize. A declared schedule with no state row yet appears with
`evaluated-at: 0`.

**Dispatch arm** `"schedules" => self.handle_schedules(..)` in the
`NativeService` match ([:4381](../../../../crates/app_supervisor/src/service.rs#L4381)).
`the_supervisor_wit_dispatch_table_covers_every_declared_function`
([:5572](../../../../crates/app_supervisor/src/service.rs#L5572)) fails
without it, and `no_supervisor_verb_accepts_or_returns_key_material`
([:5591](../../../../crates/app_supervisor/src/service.rs#L5591)) covers the
new record for free.

**`roymctl`**: a `Schedules { instance_id: String }` variant in
`SupervisorCommands`
([supervisor.rs:112](../../../../apps/roymctl/src/commands/supervisor.rs#L112)
is `Outbox`) and the matching handler arm
([:398](../../../../apps/roymctl/src/commands/supervisor.rs#L398)), printing
pretty JSON exactly as `outbox` does.

**The unsupervised deploy path.** `roymctl app deploy` deploys a compiled
plan with no supervisor behind it, and the developer guide tells operators to
"pick one per app instance"
([developer-guide.md:553](../../../../docs/developer-guide.md#L553)). A
manifest with a `schedule` validates, compiles, and deploys that way — and
then **nothing ever runs it**, silently, because the supervisor is the
scheduler (ADR-0023 §6). `roymctl app deploy` therefore **warns** when the
plan it is about to apply carries a schedule, naming `roymctl supervisor
submit` as what makes it run. A warning rather than a refusal, matching the
posture `app deploy` already takes for a registry that does not resolve every
member ("warns (does not fail)", same guide section): the deploy is valid,
one declared behaviour just will not happen.

**User-facing documentation, owed by this slice and not by closeout.**
`replicas` shipped with its own developer-guide subsection ("Scaling a
service") covering the field, its cap, and what it does *not* do. B3 ships a
manifest field, a `roymctl supervisor schedules` verb, and a rule about
missed ticks that an operator cannot guess. It owes the same: a
**"Scheduling a service"** subsection under the supervisor section covering
the five-field UTC cron surface, the `interface`/`method`/`params`/`timeout_ms`
fields, the cap, **that a missed tick is skipped and not run late**, that a
schedule finer than `poll_interval_secs` collapses to one run per pass, that
`roymctl app deploy` runs no schedules, and the partitioned-substrate cost
(which is ADR-0023 §6's consequence stated where an operator will meet it).
The milestone's own closeout list already owes a developer-guide note for the
DLQ verbs and the §6 cost; **this is the slice-level half and lands with the
slice**, not after it.

**Tests (phase 4):**

- `app_deploy_warns_when_the_plan_carries_a_schedule_and_still_deploys`
- `schedules_is_refused_without_substrate_admin` (folds into the existing
  `every_verb_is_refused_without_substrate_admin` loop at
  [:4621](../../../../crates/app_supervisor/src/service.rs#L4621) — add the
  verb there rather than writing a second test)
- `schedules_lists_a_declared_schedule_that_has_never_run`
- `schedules_reports_the_member_and_time_of_the_last_run`

### Phase 5 — end to end

**New fixture**, `test-components/scheduled-test` (D-B3-12): a WASM
component importing `syneroym:data-layer` only, exporting

```wit
interface scheduled-driver {
    /// Increments a persisted counter. What a scheduled tick calls.
    tick: func() -> result<_, string>;
    /// Reads it back, so a test process can assert "ran exactly once"
    /// without holding the service's DEK.
    tick-count: func() -> result<u32, string>;
}
```

A **new** component rather than an extension of `proxy-test`: adding a
data-layer import to `proxy-test`'s world changes the linker every existing
test that instantiates it builds, and B2 already paid once for a guest
artifact that had to be rebuilt against a changed WIT.

**Wiring the fixture in — three places, one of which does not exist:**

1. Root `Cargo.toml` `exclude`, like every sibling component.
2. **`crates/core/src/test_constants.rs`**, which is where every fixture's
   artifact path and interface constant lives
   ([:55](../../../../crates/core/src/test_constants.rs#L55) is
   `proxy-test`'s pair): add `SCHEDULED_TEST_DRIVER_INTERFACE` and
   `scheduled_test_wasm_path()`. A first draft of this plan named only the
   component directory and missed this.
3. **There is no `mise` task that builds test components.** `mise.toml` has
   `test:rust`, `test:e2e`, benches and release tasks, and nothing for
   `wasm32-wasip2` — the components are built by hand
   (`cargo build --target wasm32-wasip2 --release` per directory), and the
   tests that use them **skip silently when the artifact is missing**
   (`proxy_dispatch.rs`'s header says so, and its helper returns `None`).

   That last point is a direct threat to D-B3-11: an e2e that skips when the
   fixture was never built is an e2e that "could not assert what it claimed",
   which is the exact B1 failure D-B3-11 exists to prevent. So
   `scheduled_task_e2e` **fails, not skips**, on a missing artifact, with a
   message naming the build command. Adding a `mise run build:test-components`
   task is the better fix for the whole tree and is proposed as a backlog row
   rather than smuggled into this slice.

**`crates/substrate/tests/scheduled_task_e2e.rs`**, one real substrate and
one real supervisor (the shape `durable_outbox_e2e.rs` establishes, minus
the second substrate — B3 needs no unreachable target):

1. Deploy and adopt an instance whose service is `scheduled-test`, with
   `schedule = { cron = "* * * * *", interface = "scheduled-driver",
   method = "tick" }`. Confirm converged.
2. Assert `tick-count == 0` and that `schedules` reports the declaration
   with no `last-run-at`.
3. Wait past one cron minute. Assert `tick-count == 1`, and that `schedules`
   now reports `last-run-at` and `last-member-index == 0`.
4. Within the same cron minute, assert `tick-count` is **still 1** across
   several poll intervals — the watermark, and the direct statement of
   failure-matrix row 10.
5. Restart the supervisor process after a gap longer than the grace window.
   Assert `tick-count` does **not** jump: the skipped ticks are skipped.
   This is the step no in-process test can make, and the reason the e2e
   exists.
6. Assert the supervisor's outbox and DLQ are both empty throughout —
   D-B3-7 observed rather than argued.

`test_name`s: `a_scheduled_task_runs_on_its_own_cadence_and_only_once_per_tick`
and `a_supervisor_restart_skips_the_ticks_it_missed`.

**The wall-clock cost, since this test is bounded by a real minute boundary
and not by anything the test controls.** Five-field cron's finest period is
one minute, so:

- Set the e2e supervisor's `poll_interval_secs` to **2**, which makes the
  grace window 4 s. Both are already per-test config, not constants.
- Step 3 waits for the next minute boundary plus one poll: **up to ~62 s**.
- Step 4 needs several passes inside the same minute: **~10 s**.
- Step 5 needs the supervisor down across a whole cron occurrence and back:
  **~70 s**, plus restart.
- One run is therefore **~2.5–3 minutes**, and the three consecutive runs
  §6 asks for are **~8–9 minutes**. That is in the same band as
  `durable_outbox_e2e`'s ~287 s and ~403 s, so it needs no special handling
   — but it must be stated, not discovered.

**The six-field seconds form is deliberately not used to shorten this.**
`croner` accepts an optional seconds field, and a `*/5 * * * * *` fixture
would cut the test to under a minute — but the operator-facing surface this
slice ships is five-field, and an e2e that only ever exercises a six-field
expression is not testing what an operator will write. Six-field parsing gets
a phase-1 **unit** test instead, where it costs nothing.

---

## §3 — Failure and security matrix rows this slice owns

| Row | How B3 answers it |
|---|---|
| 10 — a run outlives its interval, the next tick must not double-start it | Structural within a live process (`instance_lock` + an inline await, §0.4); the watermark covers the crash case. Tested by phase 5 step 4 and `a_schedule_evaluated_twice_inside_one_cron_minute_runs_once` |
| 11 — a substrate partitioned from its supervisor when a tick is due | No run happens, the watermark still advances (so nothing back-fires later), and it is **visible**: `schedules` reports a `last-run-at` that stopped moving while `evaluated-at` keeps moving. Documented in the developer guide at closeout, per ADR-0023 §6 |
| 13 — queue contents at rest (by extension: schedule state at rest) | `scheduled_runs` is a table in `supervisor.db`, under the same protection as `desired_state` and the outbox. `store.rs`'s existing at-rest test is extended to name it |
| 14 — replay authorization (by extension: run authorization) | `run-scheduled` takes `restart`'s gate exactly; `schedules` takes `require_admin`, like every other supervisor verb. No new ability and no new resource namespace (D-B3-8) |

---

## §4 — The one privilege question this slice raises, answered

`run-scheduled` lets an `orchestrator/deploy` holder invoke **any** method on
**any** interface of a service on that node, as `system:<service_id>`. That
is more expressive than `restart`, which is the verb whose gate it copies,
so it deserves an explicit answer rather than an assumption.

**The answer: it grants nothing that gate did not already grant.** The same
`orchestrator/deploy` capability authorizes `deploy`, which replaces the
service's own component with arbitrary code under the same service id. A
principal who can replace the code can already cause any behaviour the code
could have; being able to call one of its existing methods is strictly
weaker. So this is not a widening of the deploy grant, it is a use of it.

**The narrowing that was considered and rejected for now:** having the
substrate verify that `(interface, method)` matches a schedule the service
actually declares. That requires the schedule to cross the wire and be
stored substrate-side, which reintroduces §0.2's content-hash problem and
gives the substrate a second copy of a declaration only the supervisor acts
on. Recorded as a backlog row with that cost, rather than built here on the
strength of an argument the paragraph above already answers.

---

## §5 — Docs and backlog impact

**New backlog rows this slice's own choices create:**

- **A missed tick is skipped; run-late is not offered.** The milestone plan
  §3 already anticipates this row ("whichever it chooses, the other is a
  row"). Carries §0.5's grace-window rule so a future run-late option is a
  change to one clamp, not a redesign. Pickup trigger: a real consumer that
  needs a nightly job to catch up after an outage.
- **A schedule-only edit re-diffs on every pass and journals no baseline**
  (§0.3). Harmless — nothing acts on it — but it means `compute_diff` is not
  a reliable "is there anything to do" signal for this class of change.
  Shared with the membership-only push path, which already behaves this way.
- **`run-scheduled` is not verified against the service's own declaration**
  (§4), with the content-hash cost that verification would carry.
- **A scheduled run's return value is discarded**, and a run that needs to
  report something has no channel. Adjacent to B2's own "the guest outbox
  has no client-side half" row.
- **Cron granularity is bounded below by `poll_interval_secs`.** A schedule
  finer than the poll interval collapses to one run per pass, silently.
  Documented in the field's own doc and in the developer guide; a row records
  that nothing refuses it.
- **A scheduled run is awaited inline in a sequential pass, so one
  instance's schedules delay every later instance's reconciliation**
  (§0.11). Bounded — `MAX_SCHEDULED_SERVICES` × `SCHEDULED_RUN_CEILING` = 8
  minutes worst case, needing sixteen unresponsive targets at once — and
  deliberately not solved here: moving the run off the pass thread gives up
  §0.4's structural overlap argument and would need a real overlap guard in
  its place. Pickup trigger: a measured pass that a schedule actually
  delayed.
- **Substrate-side concurrency for scheduled runs is unbounded across
  instances** (§0.10). One substrate hosting scheduled services for many app
  instances, each with its own supervisor, has nothing like the `rpc` probe's
  `probe_instance_permits` pool between it and a burst on a shared cron
  minute (`0 * * * *` is what everyone writes). Not sized here — the pool
  that exists was added after a measured poll cost, and there is no
  measurement for this yet. Pickup trigger: the first deployment running
  scheduled services for more than a handful of instances on one substrate.
- **No `mise` task builds the test components** (phase 5), so every
  component-backed test in the tree skips silently on a missing artifact.
  Pre-existing and wider than this slice; B3 makes its own e2e fail loudly
  instead, which does not fix the others.
- **A schedule deployed by `roymctl app deploy` never runs**, and the answer
  is a warning rather than a refusal. A row records that the warning is the
  only thing standing between an operator and a silently dead schedule.

**Backlog rows this slice closes:**

- B2's F19 fixture gap ("closing this properly needs a countable target
  fixture") — `test-components/scheduled-test` is that fixture, and its
  `tick-count` export is what an exactly-once assertion at a receiver needs.
  Move to *Recently resolved* only if B3's fixture is actually reused by a
  B2 e2e assertion; otherwise note the fixture now exists and leave the row
  open with a narrower trigger.

**Doc work owed by this slice itself (phase 4, not closeout):** the
developer guide's "Scheduling a service" subsection, in the shape
"Scaling a service" already has.

**Doc amendments (at milestone closeout, not on this slice landing):** the
`[PLT-ASY]` lease text in `system-requirements-spec.md` and
`system-architecture.md`, and the developer-guide statement of the
partitioned-substrate cost — all three already listed in
[implementation-plan.md](implementation-plan.md) §3 as milestone exit
criteria. B3 is the slice that makes them true; it does not write them.

---

## §6 — Completion checklist

- Build the new fixture by hand before anything else — there is no task for
  it (phase 5):
  ```bash
  cargo build --manifest-path test-components/scheduled-test/Cargo.toml --target wasm32-wasip2 --release
  ```
- `cargo check --workspace --all-targets` early, for phase 1's 45-literal
  sweep — eight of the `ServiceSpec` sites are e2e files that a sandboxed
  test run compiles but never executes
- `cargo +nightly fmt --all`
- `cargo clippy --workspace --all-targets --all-features`
- `cargo test --workspace` — compare the failure set against the
  pre-existing sandbox-bind category [status.md](status.md) documents;
  anything new is this slice's
- `mise run test:e2e` — expect 12/12 unchanged; B3 adds no browser-visible
  surface
- `scheduled_task_e2e` with the sandbox disabled (real port binds), run
  **three times** — it turns on a restart window and a wall-clock minute
  boundary, and one green run does not distinguish a fix from a race. This
  is the lesson B2's phase 5 recorded. Budget **~8–9 minutes** for the three
  (phase 5's arithmetic), and confirm it **fails rather than skips** with the
  fixture artifact deleted — a passing run against a missing component is
  the failure D-B3-11 exists to prevent
- Import cleanup pass over every edited file (project rule)
- [deferred-backlog.md](../../deferred-backlog.md) updated with §5's rows
- **No planning-doc ids in any new comment or test name** (§0.12's last row)
