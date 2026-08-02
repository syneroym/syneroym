# Slice A5 Implementation Plan — The Supervisor Loop

**Status:** 📋 Planned (2026-07-31, revised same day after review). Not
started. Milestone: [task.md](task.md) slice **A5**. Design of record:
[ADR-0021](../../../decisions/0021-binding-propagation-and-app-supervisor.md)
§1/§3/§4/§5/§7/§8 and
[ADR-0020](../../../decisions/0020-stable-logical-service-identity.md)
§3/§4/§5. Depends on **P0, A0, A1, A2, A3, A4 — all Complete**. Gates A6.

**The one-sentence summary.** A5 turns four one-shot client-side library
calls (`compile`, `deploy::apply_plan`, `health::poll_once`,
`health::record_report`) into a resident substrate role that holds desired
state, pushes binding changes without redeploying, restarts what breaks
inside a bounded policy, refuses to fight a second writer, renews instance
certificates unattended, and answers an operator over a `supervisor`
interface.

**Review pass (2026-07-31), all findings incorporated.** Four were
blocking — the plan as first written did not work:

- **The generation gate would have rejected the supervisor's own deploys**
  (§0.18). `deployment-plan`/`app-context` carry no generation, so every
  deploy the supervisor issues presented `0` against a held `g ≥ 1` and
  failed the moment an operator ran `adopt`. `app-context` gains a
  `generation` field.
- **Instance certificates are bound to the calling client, so custody
  cannot come after the loop** (§0.19). `certify_instance` derives the key
  from the node identity *and the calling DID*, so an operator-minted
  certificate is rejected when the *supervisor* deploys. Master custody
  therefore moves from A5d into **A5b** — the supervisor cannot deploy a
  bound app at all without it, which made the original A5b→A5c ordering
  unbuildable.
- **Per-dependent binding convergence is not readable from the resolver**
  (§0.20). `StaticInventory` is keyed `(app_instance_id, service_name)` and
  shared per substrate, so every dependent on a node reports the same
  epoch. The epoch guard and the convergence read both move to the
  **persisted per-dependent `service_bindings` row**, reversing the first
  draft's §0.11 recommendation.
- **Failure-matrix row 10 was claimed with no mechanism** (§0.21).
  ADR-0021 §3 says outright that deploy dedup on content hash and binding
  dedup on epoch are different things and "neither covers the other". A5a
  now builds content-hash dedup, with tests.

Plus: `PlanApplier` widens into `SubstrateActor` so the binding push A6 is
meant to make durable actually sits behind the trait (§0.22); `restart` and
`undeploy` are generation-gated, not just `deploy` and `write-bindings`
(§0.23); a `release-app-instance` verb closes the "retired instances can
never be touched again" trap (§0.24); A5b's `status` polls on demand so its
read surface has a source; and the twelve A5-targeted backlog rows the first
draft did not mention are each resolved, retargeted, or explained (§17).

**Second review pass (2026-07-31), all findings incorporated.** Two more
sat inside A5a and together meant the generation stamp could be built
exactly as specified and still never hold a value the supervisor minted:

- **`adopt` had no substrate-side read or write path** (§0.26). D-A5-10
  says it "reads what the substrates report holding and writes `held + 1`",
  but no verb reads the stamp — `substrate-status` carries service-level
  records only — and the only writers were side effects of other calls.
  A5a gains `app-instance-management-of` and `claim-app-instance`.
- **The row-10 dedup skipped the generation persist** (§0.27). §4A returned
  early *before* `install_app_context`, the only place the management row
  was written, so an adopt followed by an unchanged redeploy left the
  substrate holding the old generation. Fixed structurally: the stamp is
  persisted immediately after `check_generation`, not deferred with the
  bindings — it records *who is writing*, not *what was installed*.

Plus three more: `release-app-instance` used an `app-instance/` selector no
existing grant covers, and now uses node-wide `orchestrator/deploy` instead
of inventing a resource namespace (§0.28); and `--upload-masters`
contradicted the plan `submit` sends (§0.30).

**Third review pass (2026-07-31).** One blocker, created by that second
pass's own pair of fixes, and it moved a design decision rather than a
scope estimate: **no client in this tree can send `?enc=`** (§0.29). The
`binary_json_rpc` constructor every `SyneroymClient` call uses hardcodes
`enc: None`, and there is no client-side ECDH anywhere — the only
`enc = Some` in the workspace is the preamble parser, and the only ECDH
bench is the *server* handshake. So unconditional master upload plus a
plaintext refusal would have refused every real call, leaving only the
negative test passing. Resolved by removing the key from the wire entirely:
the supervisor mints in place and substitutes, and arrival/backup become
local offline operations on the supervisor's host, the shape P0 already
established for `substrate claim`. `NativeInvocation.transport_encrypted`
is withdrawn with it — the property is now held by construction rather than
by a check nothing could satisfy. The client-side encryption half is
recorded as its own backlog row, since the substrate implementing an E2E
layer no client can speak is a gap independent of A5.

**A5a and A5b are merged; A5c has its own findings pass in Part IV
(2026-08-01, revised same day after review).** Per D-A5-2 each sub-slice
gets its own `§0` before execution. **Part IV** (§19-§25) is A5c's:
twenty-one findings, nine scope-changing, twenty-one decisions
(`D-A5c-N`), a phase plan, forty-eight named tests, and answers to §18
questions 8 and 9. Read it instead of §14 before starting A5c — §14
remains as the original phase-and-signature sketch, and Part IV corrects
it in five places. A5d and A5e still owe theirs.

**Review pass on Part IV (2026-08-01), §25.** Ten findings, two blocking;
nine incorporated, one answered differently. Both blockers were decisions
this pass got wrong rather than code it misread: **the cancellation token
could never fire in time**, because `run_until_shutdown` drops the loop
future before `shutdown()` is ever reached — so the loop is spawned and
joined instead (§19.8); and **a scalar epoch on `ApplyRequest` could not
express a per-dependency counter**, so a redeploy would write a lower
epoch over a higher one and the next push would conflict — the counter is
now per dependent service, with a written invariant that makes
`install_app_context`'s unguarded write correct (§19.3). Three further
findings became new sections (§19.19-§19.21).

**Code review on the merged A5c commit (2026-08-02).** Eighteen findings
against the shipped code (A-1..A-8 correctness, B-1..B-3
security/concurrency, C-1..C-4 test coverage, D-1..D-3 docs). Fourteen
incorporated as code or test fixes; four are conscious sign-offs, not
gaps left unaddressed by oversight.

- **A-1 (high, fixed):** the loop never converged after a partial deploy
  — `apply_write_phase` journaled only the subset of services this pass
  actually applied, so `Reconciler::compute_diff`'s next read forgot
  every already-landed service outside that subset and redeployed it,
  which then dropped this pass's own subset out of the *next* baseline
  in turn. Two services on two substrates alternated being redeployed
  forever. Fixed by `record_plan_for_pass`: the journaled baseline now
  carries every service already believed landed plus whatever this pass
  is applying, distinct from the (possibly narrower) plan that actually
  gets deployed.
- **A-2 (high, fixed):** the binding epoch — bookkeeping for *who is
  writing*, advanced before every apply (D-A5c-4) — was inside the
  deploy dedup hash, so every re-apply changed the hash and forced a
  real reinstall of every dependent service, the exact restart
  `write-bindings` exists to avoid. Fixed the same way `generation` was
  already excluded from that hash: each binding is hashed minus its
  `epoch`.
- **A-3 (high, fixed):** `ManagedService.restart_attempts` was hardcoded
  `0` — phase 6's own stated deliverable — despite the `remediation`
  table recording every attempt since phase 6. `handle_status` now reads
  it back.
- **A-4 (medium, fixed):** `AlertKind::PlacementChangeRefused` was
  declared and round-trip tested but never raised; `refuse_placement_change`
  now raises and publishes it before returning the refusal.
- **A-5 (medium, fixed):** a failed binding push propagated its error
  with no alert raised at all — only a clean `Stale`/`Conflict`
  *outcome* alerted, never the round trip failing outright (matrix row
  11's actual scenario). Fixed; row 11's task.md wording corrected to
  match what the code (and test 47) now actually do — see that row for
  why "instance marked `Degraded`" was never accurate and still is not,
  independent of this fix.
- **A-6 (medium, fixed):** four `let Ok(...) else { return }` reads at
  the top of `reconcile_instance_pass`, and the connect failures
  `connect_best_effort` returns, were silently discarded — no log, no
  trace of why an instance stopped being supervised. All five now log a
  `tracing::warn!`.
- **A-7 (medium, fixed):** a deployment record left `Applying` by a
  crash was never moved out of it, pinning `handle_status` to
  "Applying" forever once nothing else happened to re-apply for that
  instance. Fixed: a pass that finds its instance's latest record still
  `Applying` recovers it to `Degraded` on sight — safe because the
  per-instance lock the pass already holds proves no apply for this
  instance can genuinely still be in flight.
- **A-8 (low, fixed):** `last_reconciled_at` was hardcoded `None` under
  a comment saying no loop existed yet to fill it. The loop now stamps
  it (in-memory, per instance) at the end of every pass it runs.
- **B-1 (medium, conscious sign-off, no code change):** any verified
  caller can subscribe to the supervisor's alert topic with no
  capability check — already recorded accurately in
  `deferred-backlog.md` §8 (`messaging/subscribe needs no capability`)
  before this review, including the exact asymmetry (node-owner-only
  writes, unauthenticated-adjacent reads). Signed off here rather than
  building a `messaging/subscribe`-shaped ability this session: it is a
  router-wide gap affecting every deployed service's messaging endpoint,
  not specific to the supervisor, and is correctly scoped as its own
  post-M5 row.
- **B-2, B-3 (low, conscious sign-offs, no code change):** unbounded
  shutdown latency against many unreachable substrates, and an
  unbounded `instance_locks` map keyed on admin-caller-supplied ids.
  Both are real, both are the kind of thing A6's durable-delivery
  rework and ordinary operational limits (an admin-only surface, a
  bounded inventory) already bound in practice. Not worth a structural
  change in this pass; flagged here as read, not missed.
- **C-1..C-4 (medium/low, fixed):** four test-coverage gaps closed with
  new tests — a real `dispatch("status", …)` call now asserts a
  populated `bindings` array (C-1); `apply_write_phase`'s
  `did_to_alias -> clients -> actor` restart lookup is now driven
  end-to-end rather than only at its two ends (C-2); a counting fake
  pins `max_held_generation_from_clients`'s "one RPC per substrate, not
  per service" half of the poll-cost budget (C-3); two new tests drive
  `handle_submit` and a real loop pass against an externally-held
  `instance_lock`, rather than four anonymous holders of it (C-4).
- **D-1, D-2, D-3 (low, fixed):** the backlog's merged "resolved" row
  overclaimed `probe_cached` had gained single-flight dedup (it has
  not — only the pool-exhaustion half was fixed; the row is split and
  reopened); `status.md` said "three e2e" and listed two; a `submit`
  that persisted desired state and one that was refused now return
  distinguishable error messages.

**Fix-verification pass (2026-08-02), against commit e2583cd.** Two more
findings, both against the fixes above rather than the original code —
caught by re-review, not self-discovered:

- **E-1 (high, fixed): A-2's fix was inert.** `context_for_hash` excluding
  `epoch` changed nothing observable, because `manifest.instance_
  certificate`/`registry_certificate` were still hashed raw, and both are
  minted fresh — a new signature, a `SystemTime::now()`-derived expiry —
  on *every* apply through either real deploy path
  (`certify_placed_members`, called by the supervisor and by `roymctl app
  deploy` alike). So the manifest hash differed on every apply regardless
  of `epoch`, content, or anything else, and row 10's no-op branch was
  never reachable from a real caller — only from a test that leaves both
  certificate fields `None`, which is every row-10 test that existed
  before this pass. Fixed by hashing each certificate's stable identity
  fields instead of the raw blob: `master_did`/`temporary_did`/`scope`
  for the instance certificate (already parsed and verified earlier in
  `deploy_with_context`, reused rather than re-parsed); `service_id`/
  `substrate_id`/`endpoint_type`/`mechanisms`/`nickname`/`is_private`/
  `ttl` for the registry certificate (a new `stable_registry_certificate_
  for_hash` helper, falling back to the raw string on a parse failure so
  a malformed blob can only ever make the dedup more conservative, never
  less safe). A dropped-or-changed certificate must still force a
  reinstall — `a_redeploy_without_a_certificate_clears_a_previously_
  installed_one` already pinned that, and briefly regressed while a first
  attempt at this fix dropped the certificate fields from the hash
  entirely rather than hashing their stable parts. New regression test:
  `an_identical_redeploy_with_freshly_minted_certificates_is_still_a_
  no_op`, two independently-issued, byte-different certificates for the
  same member. Row 10 in `task.md` corrected to say so.
- **E-2 (high, fixed): `supervisor_loop_e2e` was flaky.** The matrix-row-12
  e2e's first wait loop used a 40s deadline against a fixture whose first
  deploy lands at 28–30s and whose `status` calls (D-A5-21's on-demand
  sweep) can each cost up to `MANAGED_SUBSTRATE_CONNECT_TIMEOUT` (10s)
  reaching the substrate the test deliberately stopped — leaving room for
  roughly one more poll after landing, and whether that poll's journal
  read (taken before the connect-timeout block) fell before or after the
  deploy committed was a coin flip. Widened to 90s, matching the second
  wait loop's own already-generous margin.

**Read §0 first, then §2.** §0 records **twenty-nine** places where
`task.md`'s A5 paragraph leaves a decision unmade, names a component that
does not exist, or is stale against what A0–A4 actually shipped, plus what
this plan's own first draft got wrong. **Eighteen** of them change what A5
has to build.

---

## §0 — What `task.md` leaves open, understates, or is stale

Same discipline as A0 §6 / A1 §6 / A2 §0 / A3 §0 / A4 §0 / P0 §0.

`task.md`'s A5 text is two paragraphs plus a three-bullet list on the
online-key posture. Everything below is what those lines do not say.

### 0.1 (Scope-changing) A5 is five slices of work, and its parts do not depend on each other in one chain

Counting only what `task.md` names explicitly, A5 contains twelve
workstreams:

| # | Workstream | Exists today? |
|---|---|---|
| 1 | Substrate role + config + component wiring | No |
| 2 | `supervisor` WIT interface (write verbs + read surface) | No |
| 3 | Persisted desired state, rebuildable | No |
| 4 | Reconcile loop over the shared compiler | No |
| 5 | Epoch-guarded binding writes (four-case rule) | No — and no binding-only **write path** at all (§0.2) |
| 6 | Owner + generation stamp | No storage, no wire field (§0.4, §0.18) |
| 7 | Bounded restart-in-place remediation | No — nothing on the substrate can restart a service (§0.3) |
| 8 | Probing bound external dependencies | Blocked — `Bind` has no manifest surface (§0.9) |
| 9 | Master-key custody in the supervisor's vault | No, no transport (§0.8), and **required earlier than task.md implies** (§0.19) |
| 10 | Unattended issue + renewal, `RotationPolicy` load-bearing | No |
| 11 | MQTT alert publication | No (A4 deferred it here) |
| 12 | Convergence budget measured, ADR-0021 §6 trigger evaluated | No — and two of the three budgets have no owner at all (§0.25) |

Plus the exit criteria: a two-node reference-scenario e2e including **step 5
(scale `backend` to two members)**, which needs a manifest that can express
more than one member — a field that does not exist (§0.14) — and a test for
every unmet failure-matrix row (5, 6, 7, 8, 9, 10, 11, 12, 13,
14-second-half, 15, 18).

For scale: A3 was one workstream (placement) plus an inventory and shipped
at ~1,980 planned lines; A4 was three (declaration, status query, sweep) at
~2,140. A5 is twelve.

**Recommendation: split, per §2.** Not because the work is optional, but
because a single merge containing a new substrate role, a new WIT package, a
new crate, four new orchestrator verbs, a schema change to an existing
table, master-key custody, and a resident remediation loop is unreviewable
and untestable in one pass. See **D-A5-1**.

### 0.2 (Scope-changing) There is no binding-only write path — the supervisor's central action has nothing to call

`task.md` says "epoch-guarded binding writes on the four-case rule". There
is no binding *write* on the wire at all. A2's `planned-service.app-context`
([control-plane.wit:243](../../../../crates/wit_interfaces/wit/control-plane/control-plane.wit#L243))
is the **initial-deploy carrier**, reachable only inside `deploy-plan`, and
`deploy` is a full reinstall: pushing a scale-out through it would restart
every dependent — exactly what ADR-0021 §2 exists to prevent and what the
reference scenario's step 5 forbids ("with no restart").

Already recorded: *"No binding-only write path — A5 cannot push a membership
change without restarting every dependent"* (`deferred-backlog.md`), which
names the shape wanted: `(service_id, app_instance_id, bindings, epoch)`.

The four-case rule itself also has no home: A2 left `LogicalResolver::
register` as an unguarded last-write-wins call, with the site marked in prose
— [orchestration.rs:351-354](../../../../crates/control_plane/src/service/orchestration.rs#L351)
("ADR-0021 §3's four-case epoch guard … belongs at exactly this call and is
the supervisor slice's"). See §3, **D-A5-3**, **D-A5-4**.

### 0.3 (Scope-changing) Nothing on a substrate can restart a service

"Bounded restart-in-place remediation with backoff, max attempts, and a
terminal `Degraded` state" needs a restart operation. There is none:

- The orchestrator dispatch table handles `readyz`,
  `resolve-instance-identity`, `deploy`, `deploy-plan`, `undeploy`, `list`,
  `status`, `node-facts-only`
  ([service.rs:391-485](../../../../crates/control_plane/src/service.rs#L391)).
- `roymctl svc start` / `svc stop` send `orchestrator.start` /
  `orchestrator.stop` and both fail `MethodNotFound`
  (`apps/roymctl/src/commands/svc.rs:325-336`) — A4 §0.9 found this and
  filed it as **TBD**; A5 is what makes it load-bearing.
- `ContainerEngine` has `deploy`/`stop`/`remove`/`readyz` but **no `start`**
  (`crates/sandbox_podman/src/engine.rs`).
- `AppSandboxEngine` can recompile from disk — `deploy_wasm` persists the
  artifact to `blobs_dir/<service_id>.wasm`
  ([engine.rs:613-617](../../../../crates/sandbox_wasm/src/engine.rs#L613))
  and `load_cached_wasm` reads it back — but that function is **private**
  ([engine.rs:1276](../../../../crates/sandbox_wasm/src/engine.rs#L1276)) and
  only `warn!`s when the artifact is missing rather than failing.

So remediation is not "call the restart verb"; it is "build the restart
verb, for three service types, two of which need a new engine method." See
§4, **D-A5-5**.

### 0.4 (Scope-changing) The generation stamp has no storage and no wire field

ADR-0021 §4: "Each app instance record on a substrate carries the managing
supervisor's DID and a monotonic generation." Today `app_instance_owners` is
`(app_instance_id, owner_did, created_at)`
([registry_store.rs:147-154](../../../../crates/data_db/src/registry_store.rs#L147)),
written first-write-wins by A2's post-review fix. There is no
`supervisor_did`, no `generation`, no accessor for either, and nothing on the
wire carries a generation into any call (§0.18 is the consequence).

Failure-matrix rows 8 and 9 are unbuildable until this exists, and it is a
**schema change to a table that already has rows**, unlike A2's and A4's
additions which were new tables. See §5, **D-A5-6**.

### 0.5 (Scope-changing) Every planning input the supervisor needs is client-side

A4 hit one instance of this (§0.12: "the obvious alert store is one A5 cannot
read"). A5 hits it three more times. The supervisor is a **substrate role**;
these are all files under `roymctl`'s `--dir` or its working directory:

| Input | Where it lives | Used by |
|---|---|---|
| `DeploymentJournal` | `deployments.db`, opened only from `apps/roymctl/src/commands/app.rs` | `apply_plan`'s resume-skip, `current_placement`, `check_no_placement_change` |
| `LocalFilesystemCatalog` | the manifest's own parent directory (`app.rs:379-381`); `compile` reads WASM artifacts through it | `compile` |
| Member master `.key` files | `<dir>/identities/member-<instance>-<name>-<index>.key` (`member_identity.rs:50`) | `substitute_and_certify_members`, `deployed_service_id` |

"Reconcile loop over the shared `app_orchestration` compiler" is therefore
not free: `compile` cannot run on the substrate without a catalog, and a
catalog implies the artifacts are already there.

**Recommendation:** `supervisor.submit` carries a **fully compiled
`DeploymentPlan` with every artifact already inlined** — i.e. exactly what
`sdk::mapper::map_deployment_plan_to_wit` produces today, whose
`ArtifactSource::Binary` variant already inlines WASM bytes. `roymctl`
compiles; the supervisor stores and applies. That keeps ADR-0021's "an
effectful adapter, not a second planner" literally true, and removes the
catalog problem entirely. See **D-A5-7** and §18 question 2.

### 0.6 (Understated) `apply_plan`'s future is not `Send`, and the journal is the reason

`DeploymentJournal` holds a bare `rusqlite::Connection` (`Send`, not `Sync`),
and `apply_plan` holds `&DeploymentJournal` across every `.await`. Harmless
while every caller blocks on it; fatal for a `tokio::spawn`ed supervisor
loop. Already a backlog row ("recorded so it is a known cost at A5, not a
surprise"). `AlertStore` has the identical shape (`alerts.rs:88`) and A5's
loop holds it across awaits too.

Fix is mechanical: `conn: Arc<Mutex<Connection>>` in both, lock per
statement. See §10.4, **D-A5-8**.

### 0.7 (Ambiguous) "rebuildable from manifests plus a substrate sweep" cannot rebuild desired state

A sweep returns `service-status`: `service_id`, `service_type`,
`endpoint_type`, `app_instance_id`, `service_name`, `phase`, `probe`,
certificate window
([control-plane.wit:286-308](../../../../crates/wit_interfaces/wit/control-plane/control-plane.wit#L286)).
It never returns the deployed *config* — no env, no args, no `custom_config`,
no artifact, no dependency list. So a sweep can rebuild **membership**
("which services of instance X are on which substrate") but not **desired
state** ("what they should be").

Two readings, and they differ in what A5 builds:

1. **Rebuild = re-submit.** The operator re-runs `supervisor submit` with the
   same manifest; the sweep is only used to avoid redeploying what is already
   correct. Cheap; the manifest is the source of truth, which is what "from
   manifests plus a substrate sweep" literally says.
2. **Rebuild = reconstruct without the manifest.** Needs the substrate to
   return the stored deploy manifest, which is a much larger disclosure
   surface and a new store.

**Recommendation: (1)**, stated in the docs so nobody expects (2). See
**D-A5-9**.

### 0.8 (Scope-changing) Master-key custody has no transport, and it is the one place ADR-0020 §3 is deliberately violated

ADR-0020 §4: "A supervisor running the online-key posture holds them in its
own substrate's vault rather than on disk beside its config." `task.md` adds
"A0 leaves them as ordinary `roymctl` identities under a computable name, so
adoption is a read, not a scan."

Two problems:

- **"A read" of what, from where.** The keys are files under `roymctl`'s
  `--dir` on the operator's machine. The supervisor runs on a substrate. A
  "read" only works if the two are the same host, which A3's own provisioning
  note already refused to assume ([task.md](task.md) P0: "A3's
  substrate-inventory design must not assume an operator can claim a
  substrate they have no shell on").
- **Shipping the key over the wire is exactly what ADR-0020 §3 forbids** —
  for a *substrate*. The supervisor is deliberately a master holder, so this
  is not a contradiction, but nothing in either ADR defines the authenticated
  path by which a master private key reaches it, or what stops that path
  being used against an ordinary substrate.

Three options, none free:

| Option | Cost |
|---|---|
| **(a)** Supervisor mints masters itself at `submit` | Cleanest — the key is born where it lives, never travels. But the operator never holds it, so backup (ADR-0020 §4's "backup-critical in the way a root key is") becomes an export verb, and adopting an app deployed by A0–A4 needs an import path anyway |
| **(b)** Explicit `supervisor.import-master` over the existing E2E-encrypted channel, gated on `substrate/admin` | Matches "adoption is a read"; the key travels once. **Ruled out for A5b by §0.29** — no client can produce that channel |
| **(c)** Out-of-band file placement into the supervisor's vault directory | No new surface; unusable for a remote substrate, which is the case that matters |

**Recommendation, as revised by §0.29 and §0.31: (a) plus a
process-mediated (c).** The supervisor mints what does not exist yet, and
`export-master`/`import-master` — **RPC verbs**, moving a file within an
operator-declared directory on the supervisor's host — adopt what A0–A4
already created and take the backup ADR-0020 §4 demands.

A file-level (c) done by an *offline* `roymctl` was the first revision's
answer and does not work: the vault is an encrypted service database whose
KEK exists only in the running process's memory, so no separate process can
open it (§0.31). Routing through the supervisor keeps (c)'s property that no
key crosses the wire, while removing the impossible requirement. (b) — a key
in the request body — returns once a client can encrypt. See **D-A5-15**,
**D-A5-27**, **D-A5-28**, and §18 questions 4 and 13.

### 0.9 (Stale) ADR-0021 §7's active probe has nothing to probe

`AppDependencySpec::Bind { instance: AppInstanceId }`
([models.rs:424](../../../../crates/app_orchestration/src/models.rs#L424))
names an app *instance*, not a service inside it. A backlog row says so
outright: *"Cross-app `Bind` dependency naming has no manifest surface"* →
targeted at "A5 / first real cross-app dependency".

So failure-matrix rows 15 and 18 ("bound cross-app dependency replaced")
cannot be written as tests until a manifest can name *which* service of app B
app A depends on, and the compiler resolves it. That is a manifest-format
change with a compiler change behind it, not a probe.

**Recommendation:** scope rows 15/18 into A5e and say plainly in `task.md`
that they carry a manifest-surface prerequisite the milestone text never
mentions. See **D-A5-16**.

### 0.10 (Ambiguous) Where `adopt` mints the generation, and what row 8 actually claims

ADR-0021 §4 says the generation is "issued by the operator", and A5's text
says "minted by the operator's `adopt` action, never self-incremented". Not
stated: **where the number is durable**. Three candidates — the operator's
CLI, the supervisor's store, the substrate's app-instance row — and the
answer decides whether a rebuilt supervisor can be fooled.

Also, failure-matrix row 8 reads "Second supervisor adopts a managed instance
| Lower generation rejected". A second supervisor that genuinely runs `adopt`
gets a *higher* generation and correctly wins; the row is really about a
second supervisor that **did not** adopt and presents the generation it
already holds.

**Recommendation:** the **substrate** is the durable arbiter — its
app-instance row holds `(supervisor_did, generation)`, and `adopt` reads it
and writes `held + 1`. That is the only placement where a rogue second
supervisor cannot simply believe its own store. Row 8's wording should be
corrected to "a second supervisor that has not adopted". See **D-A5-6**,
**D-A5-10**.

### 0.11 (Understated) The epoch guard needs a per-key read that does not exist

`EndpointRegistry` exposes `save_binding` and `all_bindings()` (a full scan)
— no per-`(service_id, dependency_name)` read
([local_registry.rs:388-402](../../../../crates/core/src/local_registry.rs#L388)).
A5 needs one, both for the guard and for the convergence read (§0.20). See
§3.1's `binding_of` and §6's `bindings_of`.

### 0.12 (Ambiguous) "Reported as a conflict, reported distinctly" has no error shape

ADR-0021 §3 requires four outcomes to be distinguishable by the caller, and
§5 requires the supervisor's status output to say it is best-effort. Every
orchestrator verb today returns `result<_, string>` — an opaque message.
Three of the four cases (`applied`, `no-op`, `conflict`) are not errors at
all: a conflict is a *reported* outcome that the supervisor turns into an
alert, not a failed RPC.

**Recommendation:** a typed `binding-write-outcome` variant returned per
binding, so `stale`/`conflict` carry the epoch the substrate holds. See §3.2,
**D-A5-4**.

### 0.13 (Understated) MQTT alert publication needs a topic scheme and a subscriber story

A4 deferred it here on the grounds that "only a substrate role holds an
in-process broker" (D-A4-10). True — `MqttBroker::publish(topic, payload)`
exists (`crates/mqtt_broker/src/lib.rs:149`). Not decided: the topic. The
broker namespaces guest topics per service (`namespace_topic_for_publish`),
and the only external consumer path is `SyneroymClient::subscribe(interface,
topic)` (`sdk/src/lib.rs:525`), which routes through the `messaging`
interface of a *service*. A supervisor role is not a deployed service.

**Recommendation:** publish under the supervisor's own service id with a
fixed topic (`<SupervisorRole.alert_topic>/<app_instance_id>`), and document
the `subscribe` call an operator uses. See **D-A5-13**.

### 0.14 (Scope-changing) Nothing in a manifest can express two members, so reference-scenario step 5 is unbuildable

`member_master_name(logical_ref, index)` takes an index, and
`substitute_and_certify_members` hardcodes `0` with the comment "nothing in
today's manifest format can express more than one member per
`PlannedService`" (`member_identity.rs:175`). `deployed_service_id` inherits
it (backlog row *"A4: `deployed_service_id` assumes member index 0"* → A5).
`PlannedService` carries one `service_id`, and `TopologyEntry.members` is a
`Vec` that only ever gets one element from the compiler.

Reference-scenario step 5 — "Operator scales `backend` to two members. A
*second* member master is minted" — therefore needs a manifest field
(`replicas`), a compiler emitting N members, `resolved_dependencies` carrying
all N, and `certify_placed_members`' one-master-per-service assertion
(`deploy.rs:292-302`) relaxed. That is a compiler and model change
`task.md`'s A5 paragraph never mentions and is the largest single item hiding
inside it. See **D-A5-17** and §18 question 5.

### 0.15 (Understated) The supervisor is a new crate, not a module — and this is forced, not stylistic

The supervisor needs both the app model (`syneroym-app-orchestration`) and a
substrate **client** (`syneroym-sdk`, for the actor trait against remote
substrates). `syneroym-sdk` already depends on `syneroym-app-orchestration`
(`sdk/src/health.rs:17`), so putting the supervisor in `app_orchestration` is
a dependency cycle. It also cannot go in `syneroym-control-plane`, which is
what the supervisor *calls*.

New crate: `crates/app_supervisor/` → `syneroym-app-supervisor` (directory
snake_case, package kebab-case, per AGENTS.md). See §10.

### 0.16 (Understated) `roymctl app::handle` is still untestable, and A5 adds a whole new command group

The backlog row (*"A3: `app::handle`'s own CLI-level orchestration has no
test coverage"*, retargeted to A5 by A4) names the fix: split `roymctl` into
`main.rs` + `lib.rs` so a `crates/substrate` e2e can call `commands::…::
handle` against the existing `Node` fixture. A5 adds ~9 more subcommands to
the same untestable surface.

**Recommendation:** do the split **in A5a**, not A5b. A5a already edits CLI
code (§4.4 swaps `svc start`/`svc stop` for `svc restart`), so the split
lands before any new command group rather than between two of them. See
**D-A5-14**.

### 0.17 (Stale) Small doc drift in `task.md` itself

- Failure-matrix rows **16** and **17** are Complete (P0 —
  `security_is_denied_without_substrate_admin`,
  `an_unowned_substrate_grants_no_node_wide_capability`, both cited in
  [status.md](status.md)) but carry no ✅ marker, unlike every other
  discharged row.
- Row 14's split is recorded ("The other half … needs a supervisor and is
  A5's") but the second half has no stated test. It needs one: compromising
  the supervisor must not reach an instance it does not manage.
- The A5 paragraph says delivery is "behind a narrow 'apply this action to
  that substrate' trait" as if it were new. `PlanApplier` shipped in A3
  (`sdk/src/deploy.rs:38`) for exactly this reason, and `StatusQuery`
  (`sdk/src/health.rs:26`) is its read-side twin. A5 *widens* the boundary
  (§0.22); it does not introduce it.

### 0.18 (Scope-changing, blocking) The generation is not on the deploy wire, so the gate would reject the supervisor's own deploys

`deployment-plan` is `{app-instance-id, blueprint-id, version, services}` and
`app-context` is `{app-instance-id, service-name, bindings}`
([control-plane.wit:225-255](../../../../crates/wit_interfaces/wit/control-plane/control-plane.wit#L225)).
Neither carries a generation.

The first draft's §5.4 said a plain deploy presents `0`. But A5c's reconcile
deploys through `sdk::deploy::apply_plan` → `deploy-plan`, which is that same
path. So the moment an operator ran `adopt` (generation ≥ 1), **every deploy
the supervisor itself issued would present `0` against a held `g ≥ 1` and hit
the "`< g` → Err" row.** The supervisor would be locked out of its own app on
its first reconcile.

**Fix:** `app-context` gains `generation: u64`. That is the correct record
for it — the generation is an *app-instance* property, and `app-context` is
the only app-instance-scoped thing on the deploy wire. A deploy with no
`app-context` participates in no app instance and is not gated at all, which
is already the right behavior for a standalone `svc deploy`. See §5.4 and
§7's call-site table, which now lists every `AppContext` construction site.

### 0.19 (Scope-changing, blocking) Instance certificates are bound to the calling client, so master custody cannot come after the loop

`certify_instance`'s own doc comment
([deploy.rs:236-239](../../../../crates/sdk/src/deploy.rs#L236)): "**Bound to
this client, not just this master.** The substrate derives the key from its
own node identity *and* the calling DID, so a certificate minted through one
client is rejected at deploy by any other substrate, and by the same
substrate reached as a different caller."

The supervisor is a different principal from the operator. So an
operator-minted instance certificate is **rejected** when the supervisor
performs the deploy — the substrate derives a different instance key and
`deploy_with_context`'s check (2)
([orchestration.rs:822-834](../../../../crates/control_plane/src/service/orchestration.rs#L822))
fails.

Consequences the first draft missed, which compound:

- Whoever deploys must certify, and certifying needs the **master private
  key**. So the supervisor needs custody *before* it can deploy a bound app —
  not after.
- `roymctl supervisor submit` cannot hand over a plan with usable
  certificates, only with master **DIDs**.
- A plan compiled without masters is worse still: `emit_bindings:
  *mint_masters` ([app.rs:607](../../../../apps/roymctl/src/commands/app.rs#L607)),
  so it carries an empty binding list and the compiler's fabricated
  `did:key:h…` ids — the standing backlog row *"`app deploy` without
  `--mint-masters` binds nothing"*. Storing that as desired state would give
  the supervisor nothing to push bindings for, which is its entire job.

**Fix:** master custody moves from A5d into **A5b**, and `submit` requires
masters. A5d keeps unattended *renewal*, `RotationPolicy`, revocation, and
the `SynSvcNativeService` refresh — which is a clean seam anyway: custody is
a store concern, renewal is a loop concern. See **D-A5-15**, §2, §11.4.

### 0.20 (Scope-changing) Per-dependent binding convergence is not readable from the resolver

An exit criterion: "An operator can read health, alerts, and **per-dependent
binding convergence**". The first draft's §0.11 recommended comparing against
`LogicalResolver::registry_entry`. That is wrong, and it breaks two things:

- `StaticInventory` is keyed `(app_instance_id, service_name)`
  ([resolver.rs:288-298](../../../../crates/app_orchestration/src/resolver.rs#L288))
  and is one shared instance per substrate. So every dependent on the same
  node reports the **same** epoch for a dependency, regardless of which
  dependent a write targeted. "Per-dependent" is unreadable.
- The conflict classification would be per-*instance*, not per-dependent,
  which produces false conflicts as soon as A5e adds cross-app `Bind` and two
  dependents legitimately hold different views.

The per-dependent value does exist: the persisted `service_bindings` row,
PK `(service_id, dependency_name)`
([registry_store.rs:127-136](../../../../crates/data_db/src/registry_store.rs#L127)).
The first draft deliberately chose not to read it.

**Fix:** the epoch guard classifies against the **persisted per-dependent
row**, and `Applied` writes both that row and the shared resolver entry.
`binding-epochs` on `service-status` reports the per-dependent row. The
shared resolver entry stays last-writer-wins by construction — within one app
instance a dependency name has exactly one member set, and two dependents
disagreeing at one epoch is precisely the `Conflict` the per-dependent row
now detects. See §3.1, §3.3, §6.

### 0.21 (Coverage, blocking) Failure-matrix row 10 has no mechanism and no test

The first draft claimed A5a closed row 10 "via the generation/idempotency
gate". It does not. Row 10 is "idempotent no-op for identical (instance,
service, content hash)", and ADR-0021 §3 says explicitly that deploy dedup on
content hash and binding dedup on epoch are two different things and "neither
covers the other". The generation gate is a third thing again.

Row 10 is an exit criterion ("Every row of the failure/security matrix has a
test"), so it must be built. It is a substrate-side write primitive of the
same family as the epoch guard, and `service_deploy_facts` (A4's table) is
its natural home. See §4A, **D-A5-18**.

### 0.22 (Understated) The binding push must sit behind the trait A6 replaces

ADR-0021 §5 and `task.md`'s A6 both say durable delivery arrives by swapping
the trait implementation with "nothing above the trait changing". And the
delivery ADR-0021 §5 calls *sticky* is the **binding push**, not the deploy:
"A dependent that was unreachable when its dependency changed holds a dead
binding indefinitely."

`PlanApplier` has one method, `apply(plan)`
([deploy.rs:38](../../../../crates/sdk/src/deploy.rs#L38)). Adding
`write_bindings` as a concrete `SyneroymClient` method would put the one
delivery A6 exists for **outside** the boundary A6 swaps.

**Fix:** widen the trait to `SubstrateActor` with `apply_plan`,
`write_bindings`, and `restart` — literally ADR-0021 §5's "apply this action
to that substrate", now that there are three actions. See §3.5,
**D-A5-19**.

### 0.23 (Understated) `restart` and `undeploy` are lifecycle actions and must be generation-gated

ADR-0021 §4: "A substrate rejects binding writes **and lifecycle actions**
from a lower generation." The first draft gated `deploy_with_context` and
`write_bindings` only. `restart` is a lifecycle action this plan itself adds,
and `undeploy` is the most destructive one there is. Leaving both ungated
means a **superseded** supervisor can still restart and undeploy services it
no longer manages — which weakens matrix row 9 and, more seriously, row 14's
second half (blast radius bounded to what a supervisor manages), the row A5d
is supposed to close.

**Fix:** `restart` and `undeploy` both take a `generation: u64` and run
`check_generation` when the service has a recorded app context. A standalone
service with no app context is ungated, unchanged. See §4.2, §4.3, §5.4.

### 0.24 (Scope-changing) Nothing releases the substrate-side stamp, so retiring an instance strands it

The first draft gave `supervisor.db` a `retired` flag but nothing clears
`supervisor_did`/`generation` on the substrate. After retiring, a plain
`roymctl app deploy` presents `0` against a held `g > 0` and is refused —
**forever**, with no verb that can undo it. The same trap catches an operator
handing an instance back to manual operation.

It also leaves the standing backlog row *"`app_instance_owners` rows never
get forgotten"* untouched, on the very table this plan replaces.

**Fix:** a `release-app-instance(app-instance-id, generation)` verb that
nulls `supervisor_did` and resets `generation` to 0, keeping `owner_did`;
called by `supervisor retire` and by a new `supervisor release`. Plus a
storage removal path used when the last service of an instance is undeployed,
which is what actually closes the forgetting row. See §5.6, **D-A5-20**.

### 0.25 (Coverage) Two of the three performance budgets have no owner

`task.md` lists three: **binding convergence** (5 s, provisional),
**health poll cost** ("must not be a meaningful load on a target substrate at
the intended inventory size"), and **resolution adds no network hop**. The
first draft covered only the first. The third already has an open backlog row
(*"No Criterion bench case pinning A2's 'no network hop' budget"*), and the
second is what A4's own row *"a wasm `rpc` probe costs a component
instantiation"* is waiting on.

**Fix:** poll cost is measured in **A5c** (where a standing poller first
exists), the no-network-hop bench and the convergence measurement in
**A5e**. See §14, §16.

### 0.26 (Scope-changing, blocking) `adopt` has no substrate-side read or write path

D-A5-10 and §11.1 both say `adopt` "reads what the substrates report holding
and writes `held + 1`". **Neither operation exists.**

- **No read.** `substrate-status` returns `node-facts` plus a list of
  `service-status`
  ([control-plane.wit:286-337](../../../../crates/wit_interfaces/wit/control-plane/control-plane.wit#L286)).
  Nothing on the wire carries `AppInstanceManagement`, so a supervisor
  cannot learn the held generation and cannot compute `held + 1`.
- **No write.** After §0.18's fix the stamp is written only as a *side
  effect* of `deploy`, `write-bindings`, `restart`, `undeploy`, or
  `release-app-instance`. So a minted generation becomes durable only on the
  supervisor's next real write — and §0.27 shows the most likely next write
  skips it entirely.

That also weakens D-A5-10's own argument. If `adopt` does not write, two
supervisors can both read `held = 1`, both mint `2`, and both believe they
manage the instance until one of them happens to issue a deploy. It still
converges (the second write hits the `== g, supervisor ≠ caller` conflict
row), but the loser discovers it lost *after* doing work, rather than at the
moment it claimed.

**Fix:** two verbs in A5a — `app-instance-management-of` (read) and
`claim-app-instance` (write), the latter subject to the same four-case rule
as every other write so a racing adopt resolves deterministically at the
substrate. See §5.7, **D-A5-22**.

### 0.27 (Correctness, blocking) The row-10 dedup returns before the generation is persisted

§5.5 made `install_app_context` the only place `set_app_instance_management`
runs on the deploy path — inheriting A2's "defer the write until every
fallible step has succeeded" rule. §4A's content-hash dedup returns `Ok(())`
*before* that, from near the top of `deploy_with_context`.

So: an operator runs `adopt` (generation 2) → the supervisor redeploys an
unchanged, running app → `check_generation` passes (2 > 1) → dedup returns
success → **the substrate still holds generation 1**. Combined with §0.26,
the new generation may never land at all.

Test 19 as first written (`an_identical_redeploy_of_a_running_service_is_a_
no_op`) would not have caught it: it asserts on the absence of a reinstall,
not on the recorded generation.

**Fix, structural rather than an ordering rule someone can re-break:**
persist the management row **immediately after `check_generation` succeeds**,
before the dedup check and before any artifact work, and remove it from
`install_app_context`. The stamp records *who is writing*, not *what was
installed*, so it does not belong behind the deferral rule that governs
bindings — and recording a manager whose deploy then fails is correct, not a
half-installed state: they are the manager, they just failed. See §4A, §5.5,
**D-A5-23**.

### 0.28 (Correctness) `release-app-instance` used a resource shape no grant covers

The first draft gated it on `substrate:<node>/app-instance/<id>`.
`covers_resource` is segment-wise prefix matching over a documented selector
set — `collection/`, `blob/`, `topic/`, `rpc/`, and orchestrator's
`app/<name>` ([capability.rs:56-90](../../../../crates/ucan/src/capability.rs#L56)).
`app-instance/` is a new first segment, so a supervisor's app-scoped
`orchestrator/deploy` on `substrate:<node>/app/<svc>` does **not** cover it;
only node-wide `substrate/admin` would, and nothing in the plan issues that
to a supervisor.

Reusing `app/<app_instance_id>` is worse, not better: it puts app-instance
ids and service ids in one selector namespace, so a grant for
`app/inst-1` would also cover a service literally named `inst-1`. Service
ids are `did:key:…` today, making a collision unlikely — but "unlikely" is
not a property to build an authorization boundary on.

**Fix:** `claim-app-instance` and `release-app-instance` gate on **node-wide
`orchestrator/deploy`** (`ResourceUri::substrate(node_did)`, which
`substrate/admin` entails) — an existing, issuable shape, checked by the
existing `has_node_wide_ability`. It is also honest: claiming or releasing an
app instance is a node-scoped act, because the instance spans services.

**The cost, stated rather than discovered:** a supervisor needs **node-wide**
`orchestrator/deploy` on each managed substrate, not merely an app-scoped
grant. That raises the bar of what §17's "nothing issues the supervisor's
grants" row is asking for, and it goes in the developer guide. A narrower
app-instance resource shape needs a selector-namespace decision and is its
own backlog row. See §5.6, §5.7, **D-A5-24**.

### 0.29 (Scope-changing, blocking) No client in this tree can speak the encryption layer, so a key must never be on the wire at all

§0.8 option (b) sends a member master private key "over the existing
E2E-encrypted channel". **There is no such channel available to a client.**

- `?enc=` is optional in the preamble (`enc: Option<String>`,
  [preamble.rs:183](../../../../crates/router/src/preamble.rs#L183)), and
  `RoutePreamble::binary_json_rpc` — the constructor every
  `SyneroymClient` call goes through — hardcodes `enc: None`
  ([preamble.rs:310-322](../../../../crates/router/src/preamble.rs#L310)).
  The Iroh path sets `pubkey` and `ucan` and nothing else
  ([lib.rs:469-471](../../../../crates/sdk/src/lib.rs#L469)); the HTTP
  passthrough preamble likewise hardcodes `enc: None`
  ([lib.rs:791-799](../../../../crates/sdk/src/lib.rs#L791)).
- **There is no client-side ECDH anywhere in the tree.** `ecdh-p256`
  appears only inside `crates/router` (preamble parsing, `routing.rs`,
  `route_handler/encryption.rs`, `dispatch.rs`) and in two benches — one of
  which is explicitly `ecdh_p256_server_handshake`. The only
  `enc = Some(...)` in the whole workspace is the preamble *parser*
  ([preamble.rs:254](../../../../crates/router/src/preamble.rs#L254)).

So the substrate implements an end-to-end encryption layer nothing can
currently use. That is a standing gap worth recording on its own (§17's
backlog), independent of A5.

**Two decisions are therefore joined, and were not before:** how custody
arrives, and whether the transport is encrypted. The first draft's pair —
unconditional upload (D-A5-26) plus a plaintext refusal (D-A5-25) — would
have refused *every real call*: A5b's happy path and e2e tests 18/20/21 all
submit, and every submit uploads. Only the negative test would have passed,
proving nothing about the accepting path, because no caller can produce one.

**The check is not the thing to weaken.** `?enc=` is end-to-end through
relays, and the relay path is exactly the untrusted middle a coordinator
sits in — Iroh's QUIC/TLS covers hop-to-hop only. A backup-critical private
key is the right thing to protect there.

**Fix: decouple them by keeping the key off the wire entirely.** A5b adopts
§0.8 option **(a)**, mint-in-place: the supervisor generates each member
master into its own vault, and nothing transfers. Backup and adoption move
a *file* within an operator-declared directory on the supervisor's host,
through `export-master`/`import-master` verbs that take a **name** and
return a **path** — §0.31 explains why those must be RPC verbs rather than
the offline `roymctl` commands this section first proposed.

Consequences, all of them wanted:

- No `NativeInvocation.transport_encrypted` in A5a. Building 37 sites of
  plumbing for a verb no slice ships would be speculative work, and the
  security property is now held **by construction** (no key on the wire)
  rather than by a check. **D-A5-25 is withdrawn.**
- The security property is *stronger*, not weaker: encrypting a key in
  transit is worse than never transmitting it.
- The key-in-the-request form of `import-master` — the remote-adoption
  convenience — waits on the client encryption half, and its backlog row
  says so along with the invocation-level flag it will need.

See §11.4, §12, **D-A5-27**, and §18 question 11.

### 0.30 (Correctness) `--upload-masters`' fallback contradicted the plan `submit` sends, and the fix moves substitution

§12's `Submit` handler always resolved-or-minted masters locally and
substituted them into the plan's `service_id`s and `resolved_dependencies`.
The flag's own help text then said "Without it the supervisor mints its own
masters instead" — but those would be **different DIDs** from the ones
already baked into `plan_json`. The two cannot both be true. And without the
keys the supervisor cannot certify at all (§0.19), so a no-upload submit
produced desired state it could store and never apply.

The first fix was "always upload", which §0.29 has now ruled out.

**Fix: the supervisor mints and substitutes.** `roymctl supervisor submit`
sends the plan with the compiler's **fabricated** ids; the supervisor
resolves-or-mints one master per `(app_instance_id, service_name, index)` in
its own vault and performs the substitution itself, before storing desired
state.

That is the more correct division anyway, and it removes a split the first
draft had not noticed: the supervisor already owns the master *lifecycle* —
it renews in A5d, and it mints member 1 with no operator present in A5e's
scale-out. Having the operator mint at submit and the supervisor mint at
scale-out would have put one lifecycle under two principals.

It also closes the **supervisor half** of the backlog row *"`app deploy`
without `--mint-masters` binds nothing"*: there is no unmastered submit
path, because the supervisor cannot store desired state it has no masters
for. The row was filed against `roymctl app deploy` though, which this does
not touch — §4.5 closes that half, and the row needs both.

The cost, stated: **the operator does not hold the master unless they export
it**, and ADR-0020 §4 calls it backup-critical. So `submit` prints the
minted DIDs and the export command, and the developer guide names the backup
duty — which `task.md`'s own non-goals already anticipated ("Backup is an
operator duty this milestone documents rather than automates").
`D-A5-7` changes accordingly: the submitted plan carries fabricated ids, not
substituted ones. See §12, **D-A5-26**.

### 0.31 (Scope-changing, blocking) The vault cannot be opened offline, and the KEK it needs does not survive a restart

§0.29's fix said `export-master` / `import-master` would "open the
supervisor's own `StorageProvider` from `config.storage.db_dir`, the same way
`roymctl substrate claim` reads the node's key file directly". **The analogy
does not carry.**

`substrate claim` reads a *plaintext* ed25519 key file. `MasterVault` reads
an *encrypted* service database, and the key that decrypts it exists only
inside the running substrate process:

- `KeyStore` holds it in memory alone — `kek: Arc<Mutex<Option<Zeroizing<[u8;
  32]>>>>`, and `KeyStore::new()` starts empty
  ([key_store.rs:29-31](../../../../crates/data_keystore/src/key_store.rs#L29)).
- The only production populator is the `security.inject-kek` RPC
  ([service.rs:328](../../../../crates/control_plane/src/service.rs#L328)).
  There is **no** config field (`crates/core/src/config.rs` contains no
  `kek` at all), no env var, no file.
- With encryption on and no KEK, the open fails closed:
  `verify_encryption_mode` returns `Err("EncryptionKeyRequired")`
  ([sqlite.rs:1363-1370](../../../../crates/data_db/src/sqlite.rs#L1363)).

So a separate `roymctl` process cannot decrypt the vault. Test 17 as first
written would have passed **only** against an `encryption_enabled = false`
fixture — a dev profile, which is exactly where the protection matters least,
so it would have proved nothing about the real posture.

**Fix: keep the operation inside the process that holds the KEK.**
`export-master` and `import-master` become **supervisor RPC verbs** over a
directory declared in `SupervisorRole` config — never a caller-supplied path.
`export-master(name)` writes the key there and returns **only the written
path**; `import-master(name)` reads from that same directory by name. No key
byte appears in any request or response, so §0.29's by-construction property
is intact and test 16 still holds as written.

**What this does and does not do to the shell requirement**, stated
precisely rather than overclaimed: it removes the need for a shell that can
*decrypt the vault*, which was impossible. It leaves a **file-retrieval**
requirement — the operator still has to collect the written file from the
supervisor's host. That is satisfiable with ordinary tooling an operator
already has (a mounted volume, a backup agent, `scp`), which the decryption
problem was not.

**And a constraint A5 inherits rather than introduces, which A5d's claim
rests on:** the supervisor can read its own vault only *after* an operator
has injected the KEK, and the KEK does not survive a restart. So:

- **A5b:** `submit` cannot mint masters into a locked vault. Startup order is
  boot → `inject-kek` → submit, and a supervisor whose vault is locked must
  say so loudly rather than failing obscurely on the first submit — the same
  posture `runtime.rs:377`'s unowned-substrate warning already takes.
- **A5d:** ADR-0020 §3's "issues and renews unattended" is unattended
  **between KEK injections**. A supervisor host that reboots stops
  certifying and renewing until a human runs `inject-kek`. §15 claims the
  automation and must say where it stops.

See **D-A5-28**, §11.4, §15, and §18 question 13.

---

## §1 — Decisions

| ID | Decision |
|---|---|
| **D-A5-1** | A5 ships as **five sub-slices** (§2): **A5a** substrate write primitives, **A5b** the role + store + interface + master custody, **A5c** the loop + remediation + MQTT, **A5d** unattended renewal, **A5e** scale-out + cross-app probes + budgets. Each is independently mergeable and each closes a named set of failure-matrix rows. A5a needs no supervisor at all. |
| **D-A5-2** | Each sub-slice gets its **own `§0` pass** before execution. This document plans A5a and A5b to executable detail and A5c–A5e to phase-and-signature detail; the latter three are explicitly *not* ready to hand to an implementer without their own findings pass. |
| **D-A5-3** | The binding write is a **new `orchestrator` verb**, `write-bindings`, not a reuse of `deploy`/`deploy-plan` (§0.2). It touches the binding tables and the resolver and nothing else — no artifact work, no restart, no lifecycle hook. |
| **D-A5-4** | Its result is a **typed `binding-write-outcome` per binding** (`applied` / `no-op` / `stale(u64)` / `conflict(u64)`), not a `result<_, string>` (§0.12). `stale` and `conflict` are outcomes, not RPC failures: the call succeeds and the supervisor decides what to do. |
| **D-A5-5** | Restart is a **new `orchestrator` verb**, `restart(service-id, generation)`, type-dispatched off the `service_deploy_facts.service_type` A4 records: wasm → evict + recompile from `blobs_dir`; container → `podman stop` then `podman start`; `tcp`/`nativehost` → refused with a reason, because the process runs outside this substrate (§0.3). Gated on `orchestrator/deploy` scoped to `substrate:<node>/app/<service_id>` **and** on the generation (§0.23). |
| **D-A5-6** | `app_instance_owners` is **replaced** by `app_instance_management` `(app_instance_id, owner_did, supervisor_did NULLABLE, generation INTEGER NOT NULL DEFAULT 0, created_at)` (§0.4). Pre-release, so no `ALTER`/migration ladder: the old table's DDL is deleted and the new one created unconditionally, exactly the pattern A2 and A4 used. The **substrate** is the durable arbiter of the generation (§0.10). |
| **D-A5-7** | `supervisor.submit` carries a **fully compiled `DeploymentPlan` with artifacts inlined**, using the compiler's **fabricated** service ids — `roymctl` compiles, the **supervisor** mints its masters and substitutes them (§0.5, §0.19, §0.30). The supervisor holds no catalog and never reads a manifest. |
| **D-A5-8** | `DeploymentJournal` and `AlertStore` both change `conn: Connection` → `conn: Arc<Mutex<Connection>>` so `&Self` is `Send + Sync`, unblocking `tokio::spawn` for A5c's loop and letting one connection back all three of the supervisor's stores (§0.6). Mechanical: every method already takes `&self`. |
| **D-A5-9** | "Rebuildable" means **re-submittable**: the manifest is the source of truth, the sweep only avoids redundant redeploys (§0.7). Written into the docs so nobody expects reconstruction without a manifest. |
| **D-A5-10** | `adopt` reads the substrate's held generation and writes `held + 1`. The supervisor never self-increments; a supervisor that reads a **higher** generation than its own stops managing that instance and raises `AlertKind::SupervisorSuperseded` (failure-matrix row 9). |
| **D-A5-11** | The supervisor's store is **one SQLite file** (`supervisor.db` under `app_data_dir`) holding desired state, the deployment journal (`DeploymentJournal`'s existing schema), and alerts (`AlertStore`'s existing schema), all over one `Arc<Mutex<Connection>>`. A4 promised A5 "the schema, the types, and the folding logic, not the file" — this is that promise cashed. |
| **D-A5-12** | The `supervisor` interface is a **new WIT package** (`crates/wit_interfaces/wit/supervisor/supervisor.wit`, `package syneroym:supervisor;`) with its own `bindgen!` module and its **own adherence test** (§11.5), not more functions on `orchestrator`. ADR-0021 §8's neighbouring-name test applies to the code layout too: the supervisor is a *client* of the orchestrator. |
| **D-A5-13** | Alerts publish to `MqttBroker` under the supervisor's own service id, topic `<SupervisorRole.alert_topic>/<app_instance_id>` (§0.13), in `record_report`'s **caller** — not inside `record_report`, which stays a pure fold `roymctl` also calls. |
| **D-A5-14** | `roymctl` is split into `main.rs` + `lib.rs` **in A5a**, before any new command group, closing the backlog row A3 opened and A4 grew (§0.16). |
| **D-A5-15** | Master custody is **A5b, not A5d** (§0.19). The supervisor mints into its own vault; adoption and backup go through `export-master`/`import-master` over an operator-declared directory on its host (§0.29, §0.31, D-A5-27). Custody is local to the supervisor's own node, so it needs no remote `set-secret` and does not trip the P0 backlog row about vault writes (§17). |
| **D-A5-16** | Failure-matrix rows 15/18 carry an unstated prerequisite — a manifest surface naming which service of a bound app is depended on (§0.9) — and are scoped into A5e with that prerequisite made explicit in `task.md`. |
| **D-A5-17** | Multi-member support (`ServiceSpec.replicas`, N members per logical service) is **A5e**, and reference-scenario step 5 goes with it (§0.14). A5a–A5d keep the single-member assumption and its existing backlog row, which A5e closes. |
| **D-A5-18** | Deploy idempotency (failure-matrix row 10) is a **content hash stored on `service_deploy_facts`**, compared at deploy: identical `(manifest, app_context-minus-generation)` against a service that is not `NotRunning`/`NotFound` is a no-op returning success (§0.21, §4A). Distinct from the epoch guard and from the generation gate, per ADR-0021 §3. |
| **D-A5-19** | `PlanApplier` widens into **`SubstrateActor`** with `apply_plan`, `write_bindings`, `restart` — ADR-0021 §5's "apply this action to that substrate" boundary, now that there are three actions, so A6 can swap all of them (§0.22). |
| **D-A5-20** | A `release-app-instance(app-instance-id, generation)` verb nulls the management stamp, called by `supervisor retire`/`release`; and `undeploy_impl` removes the management row when it removes the last service of an instance (§0.24). |
| **D-A5-21** | A5b's `status` runs an **on-demand sweep** (`sdk::health::poll_once` + `record_report`, inside the RPC) rather than reading rows nothing writes. A5c's resident loop then changes *when* they run, not *what a signal means* — exactly A4 §13's promise. This is what makes A5b's read surface a real exit-criterion deliverable. |
| **D-A5-22** | A5a adds **`app-instance-management-of`** (read) and **`claim-app-instance`** (write) so `adopt` has both halves it was specified to use (§0.26). `claim` runs the same four-case generation rule as every other write, so two supervisors racing an adopt lose deterministically at the substrate rather than at whichever one issues a deploy first. |
| **D-A5-23** | The management stamp is persisted **immediately after `check_generation`**, before the row-10 dedup and before any artifact work — not deferred to `install_app_context` (§0.27). It records *who is writing*, not *what was installed*, so A2's defer-until-everything-succeeded rule does not apply to it. |
| **D-A5-24** | `claim-app-instance` and `release-app-instance` gate on **node-wide `orchestrator/deploy`**, not on an invented `app-instance/` selector and not on `app/<app_instance_id>` (§0.28). Consequence, documented rather than discovered: a supervisor needs a node-wide grant on each managed substrate. |
| **D-A5-25** | ~~`NativeInvocation.transport_encrypted`~~ — **withdrawn** (§0.29). No client in this tree can send `?enc=`, so a plaintext refusal would refuse every real call; and once no key is on the wire, the flag guards nothing A5 ships. Kept in the table as a withdrawn id so the reasoning is findable rather than looking like an oversight. Recorded as a backlog row with the client-side encryption work it depends on. |
| **D-A5-26** | `supervisor submit` sends **fabricated** ids; the **supervisor** resolves-or-mints one master per `(app_instance_id, service_name, index)` in its own vault and substitutes them itself (§0.30). No upload, no flag. `submit` prints the minted DIDs and the `export-master` command, since ADR-0020 §4 makes backup an operator duty. |
| **D-A5-27** | Master **arrival and backup go through the supervisor process**, as `export-master` / `import-master` RPC verbs over a **config-declared directory** on its host — never a caller-supplied path, and never key bytes in a request or response (§0.29, §0.31). An offline `roymctl` cannot do it: the vault is encrypted and its KEK lives only in the running process's memory. The key still never crosses the wire, so §0.29's property holds by construction. |
| **D-A5-28** | The supervisor's vault is **unreadable until an operator injects the KEK, and the KEK does not survive a restart** (§0.31) — inherited, not introduced. A5b warns loudly at startup when the vault is locked and refuses `submit` with a message naming `inject-kek`; A5d's "unattended renewal" is unattended *between injections*, and §15 says so rather than implying otherwise. |

---

## §2 — The recommended split

Each sub-slice is independently mergeable and closes a named set of
failure-matrix rows. The seam is real: **A5a is a substrate capability set
with no supervisor in it**, testable against one node; **A5b is a process
with no autonomy**, testable by RPC; **A5c is the autonomy**.

| Sub-slice | Contents | Closes | Depends on |
|---|---|---|---|
| **A5a** — substrate write primitives | `write-bindings` + four-case guard (per-dependent, §0.20); `restart`; `app-instance-management-of` + `claim-app-instance` + `release-app-instance`; `app_instance_management` + generation gate on deploy/write-bindings/restart/undeploy; `generation` on `app-context`; deploy content-hash dedup; binding state on `service-status`; `SubstrateActor` trait; `roymctl` lib split; `svc restart`; `app deploy` refuses an unmastered manifest that declares dependencies (§4.5) | rows **5, 6, 7, 8, 9** (substrate half), **10** | A4 |
| **A5b** — the role, the store, the interface, custody | New crate; `[roles.supervisor]`; `supervisor.db`; `supervisor` WIT (submit / adopt / release / pause / resume / retire / force-reconcile / status / alerts); on-demand sweep in `status` (D-A5-21); mint-in-place master custody in the supervisor's vault, with local offline import/export (D-A5-27); `roymctl supervisor …`; `Arc<Mutex<Connection>>` fix | row **9** (supervisor half); the exit criterion "an operator can read health, alerts, and per-dependent binding convergence" | A5a |
| **A5c** — the loop and remediation | Resident reconcile loop; bounded restart with backoff + max attempts + terminal `Degraded`; binding push on membership change; MQTT publication; **health-poll-cost budget measured** | rows **11, 12, 13** | A5b |
| **A5d** — unattended renewal | Renewal on the loop; `RotationPolicy` load-bearing; `SynSvcNativeService` refresh on out-of-band install; certificate maximum lifetime; master-anchor refresh on a schedule; revocation surface | rows **1/3** (automation half), **14** (second half) | A5c |
| **A5e** — scale-out, cross-app, budgets | `ServiceSpec.replicas` + multi-member compiler; cross-app `Bind` manifest surface; ADR-0021 §7 probe; **convergence budget + §6 trigger evaluated in writing**; **no-network-hop Criterion bench** | rows **15, 18**; reference-scenario steps 5–6; two exit criteria | A5c, A5d |

**Milestone closes at the end of A5e.** A6 stays outside, per `task.md`.

---

# Part I — A5a: substrate write primitives

No supervisor. Everything below is testable against one node.

## §3 — Phase 1: the epoch-guarded binding write

### 3.1 `crates/app_orchestration/src/resolver.rs` — the pure rule

New, beside `TopologyEntry` (`TopologyEpoch` already derives `Ord`,
resolver.rs:74-77):

```rust
/// The four outcomes ADR-0021 §3 requires a binding write to be
/// distinguishable between. Kept as data rather than a `Result` because
/// three of the four are successes: only the caller decides whether
/// `Stale` or `Conflict` is worth an alert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingWriteOutcome {
    /// No entry held, or a strictly higher epoch. The caller applies.
    Applied,
    /// Same epoch, same membership. Success with no write -- the ordinary
    /// retry ADR-0021 §5 says to expect.
    NoOp,
    /// Same epoch, different membership. Two writers produced different
    /// answers at one epoch, which is the signal ADR-0021 §4 exists to
    /// catch.
    Conflict(TopologyEpoch),
    /// A lower epoch: a late-arriving retry. The mapping does not regress.
    Stale(TopologyEpoch),
}

/// Applies ADR-0021 §3's four-case rule. Pure: no storage, no resolver, so
/// the rule itself is unit-testable with no substrate.
///
/// `held` must come from the **per-dependent** persisted binding row, not
/// from the shared `StaticInventory` entry: that entry is keyed
/// `(app_instance_id, service_name)` and is one value per substrate, so
/// classifying against it would give every dependent on a node the same
/// answer and produce false conflicts the moment two dependents
/// legitimately differ.
///
/// "Content" is `(mode, members, sharding_strategy)` and deliberately
/// **not** `cache_ttl`: a TTL difference at one epoch is a policy
/// difference between two writers, not a disagreement about who is
/// serving the service, and reporting it as a two-writer conflict would
/// make the signal noisy exactly where it must be trustworthy.
#[must_use]
pub fn classify_binding_write(
    held: Option<&TopologyEntry>,
    incoming: &TopologyEntry,
) -> BindingWriteOutcome {
    let Some(held) = held else { return BindingWriteOutcome::Applied };
    match incoming.epoch.cmp(&held.epoch) {
        Ordering::Greater => BindingWriteOutcome::Applied,
        Ordering::Less => BindingWriteOutcome::Stale(held.epoch),
        Ordering::Equal => {
            let same = held.mode == incoming.mode
                && held.members == incoming.members
                && held.sharding_strategy == incoming.sharding_strategy;
            if same { BindingWriteOutcome::NoOp } else { BindingWriteOutcome::Conflict(held.epoch) }
        }
    }
}
```

The per-dependent read (§0.11, §0.20). `crates/core/src/storage.rs`, on
`EndpointStorage`:

```rust
    /// One persisted binding's `entry_json`. The epoch guard compares
    /// against exactly one row, so `load_all_bindings`' full scan (which
    /// exists for the startup replay) is the wrong shape for it.
    async fn load_binding(
        &self,
        service_id: &str,
        dependency_name: &str,
    ) -> Result<Option<String>>;

    /// Every persisted binding for one service, for `status`'s
    /// per-dependent convergence report.
    async fn load_bindings_for(&self, service_id: &str) -> Result<Vec<(String, String)>>;
```

and the matching `EndpointRegistry::binding_of` / `bindings_of` pass-throughs
(`crates/core/src/local_registry.rs`, beside `save_binding`).

Export `BindingWriteOutcome` and `classify_binding_write` from
`crates/app_orchestration/src/lib.rs`.

### 3.2 `control-plane.wit` — the wire surface

Added to `interface orchestrator`, after `dependency-binding`:

```wit
    /// Outcome of one epoch-guarded binding write (ADR-0021 §3). Four
    /// cases kept distinct because the supervisor's next action differs
    /// per case: `applied`/`no-op` are both success, `stale` means this
    /// writer is behind and must re-read, `conflict` means a second
    /// writer exists and is the signal the generation stamp (§4) is for.
    /// The u64 payload is the epoch this substrate holds.
    variant binding-write-outcome {
        applied,
        no-op,
        stale(u64),
        conflict(u64),
    }

    record binding-write {
        /// The deployed service whose binding table this targets.
        service-id: string,
        app-instance-id: string,
        bindings: list<dependency-binding>,
        /// The writing supervisor's generation for this app instance
        /// (ADR-0021 §4). A generation lower than the recorded one is
        /// rejected outright, before any binding is examined.
        generation: u64,
    }

    /// Push current bindings into a deployed service's configuration
    /// without reinstalling it (ADR-0021 §1/§3). Deliberately not
    /// `deploy`: `deploy` is a full reinstall and would restart every
    /// dependent, which the milestone's reference scenario step 5
    /// forbids. One outcome per binding, in the order sent.
    ///
    /// May only **update** a dependency the service already declared at
    /// deploy: a name with no existing binding row is refused, because a
    /// guest's declared dependency set is a deploy-time contract and a
    /// push must not be able to inject a logical name the service never
    /// asked for.
    write-bindings: func(write: binding-write)
        -> result<list<binding-write-outcome>, string>;
```

And on `app-context` (§0.18):

```wit
    record app-context {
        app-instance-id: string,
        service-name: string,
        bindings: list<dependency-binding>,
        /// The generation the writer manages this app instance at
        /// (ADR-0021 §4). `0` means "unmanaged", which is what every
        /// operator-driven `roymctl app deploy` sends and what an
        /// un-adopted instance accepts. A supervisor sends the
        /// generation its `adopt` minted -- without this field it would
        /// present 0 and be locked out of its own app on the first
        /// reconcile after an adopt.
        generation: u64,
    }
```

### 3.3 `crates/control_plane/src/service/orchestration.rs`

Trait method on `OrchestratorInterface` (after `status`):

```rust
    /// Epoch-guarded binding write (M05A A5, ADR-0021 §3). The only path
    /// that changes a dependent's resolution without redeploying it.
    async fn write_bindings(
        &self,
        write: BindingWrite,
        caller: &CallerContext,
    ) -> Result<Vec<BindingWriteOutcome>, String>;
```

Implementation, `write_bindings_impl` (**corrected post-review** against a
first draft that had the persist at the end of the function and no
app-instance owner check — see the notes after the block):

```text
# ---- authorization ------------------------------------------------------
# Same gate `deploy_with_context` applies, for the same reason: a binding
# write changes what a service calls, which is a deploy-class change to
# that service, not a read.
deploy_resource = substrate:<node_did>/app/<write.service_id>
if !caller.has_capability(deploy_resource, ORCHESTRATOR_DEPLOY):
    return Err("caller ... holds no orchestrator/deploy grant for '<id>'")

# The service must be deployed here and its recorded app context must
# match. Without this an authorized caller could write bindings into an
# app instance its service does not belong to -- the same hole A2's
# `binding.app_instance_id != ctx.app_instance_id` check closes on deploy.
match registry.app_context_of(&write.service_id):
    None      -> Err("'<id>' has no app context on this substrate")
    Some((instance, _)) if instance != write.app_instance_id ->
        Err("'<id>' belongs to app instance '<instance>', not '<sent>'")
    Some(_)   -> ()

# The same app-instance-owner gate `deploy_with_context` applies (its own
# `existing != caller.caller_did` check). The app-context match above
# proves `write.service_id` belongs to `write.app_instance_id`; it does
# not prove the caller may manage that instance as a whole -- an
# app-scoped grant on one service is not authority over every service the
# instance's shared resolver entry affects.
if let Some(existing) = registry.app_instance_management_of(&write.app_instance_id).map(owner_did)
   && existing != caller.caller_did
   && !has_node_wide_ability(caller, ORCHESTRATOR_DEPLOY):
    return Err("app instance '<id>' is owned by <existing>; a binding write
                into it must come from its owner or a substrate owner")

# ---- generation gate (§5.4), persisted immediately -----------------------
# D-A5-23: before any binding is examined, not after the loop -- the same
# rule every other gate site follows. A validation refusal below must not
# leave the accepting generation unrecorded.
management = check_generation(&write.app_instance_id, caller, write.generation)?
registry.set_app_instance_management(write.app_instance_id, management).await?

# ---- validate every binding before applying any of it --------------------
# `prepare_binding` and the `binding_of` existence check are both pure
# reads, so the whole list is checked up front -- a refusal partway
# through must not leave earlier bindings in the same call applied.
prepared = []
for b in write.bindings:
    (dependency_name, entry) = prepare_binding(b, &write.app_instance_id)?

    # Update-only: a push may not introduce a dependency the guest never
    # declared. A new dependency changes the guest's contract and needs a
    # redeploy.
    held_json = registry.binding_of(&write.service_id, &b.dependency_name).await?
    if held_json.is_none():
        return Err("'<service_id>' declares no dependency '<name>'; a new
                    dependency needs a redeploy, not a binding push")

    held = parse::<TopologyEntry>(held_json)
    outcome = classify_binding_write(Some(&held), &entry)
    prepared.push((b, dependency_name, entry, outcome))

# ---- apply, in order ------------------------------------------------------
outcomes = []
any_applied = false
for (b, dependency_name, entry, outcome) in prepared:
    if outcome == Applied:
        any_applied = true
        registry.save_binding(&write.service_id, &write.app_instance_id,
                              &b.dependency_name, &to_json(&entry)).await?
        logical_resolver.register(instance_id, dependency_name, entry)

    # NoOp / Stale / Conflict write nothing. NoOp in particular must not
    # re-register: re-registering evicts the resolver cache for an
    # unchanged entry, turning the ordinary retry into cache churn on the
    # hot path.
    outcomes.push(outcome)

# §4A's dedup key hashes what a deploy *sends*, not what is currently
# installed, so it cannot see a push that happened since the last deploy.
# Clear it so a repair redeploy of identical content takes the full
# reinstall path instead of matching the stale hash.
if any_applied:
    if let Some((service_type, health_check_json, _)) = registry.deploy_facts(&write.service_id):
        registry.set_deploy_facts(write.service_id, service_type, health_check_json, None).await?

return Ok(outcomes)
```

Corrected after A5a's own post-implementation review, against this
document's first draft: the persist used to sit after the per-binding
loop (contradicting §5.4/D-A5-23, which the shipped code follows), the
loop applied each binding as it validated it rather than validating the
whole list first, there was no app-instance owner check at all, and there
was no dedup-hash invalidation. The block above is what shipped, not what
was originally drafted here — keep it that way if this section is edited
again.

**Extract `prepare_binding`** from `deploy_with_context`'s inline loop
([orchestration.rs:916-943](../../../../crates/control_plane/src/service/orchestration.rs#L916)):

```rust
/// Validates one wire `dependency-binding` into `(LogicalServiceName,
/// TopologyEntry)`. Shared by the deploy path and `write_bindings` so the
/// two cannot validate differently -- every field is caller-supplied
/// (D-A2-15), and `LogicalServiceName::new` *panics* on an empty name or
/// one containing '/'.
fn prepare_binding(
    binding: &DependencyBinding,
    app_instance_id: &str,
) -> Result<(LogicalServiceName, TopologyEntry), String>
```

The `binding.app_instance_id != app_instance_id` refusal moves inside it, and
`deploy_with_context`'s loop becomes a call to it.

### 3.4 `crates/control_plane/src/service.rs` — dispatch

One new arm per verb (`write-bindings`, `restart`, `release-app-instance`),
following `status`'s existing shape. `test_wit_adherence`
([service.rs:582](../../../../crates/control_plane/src/service.rs#L582))
walks every WIT `orchestrator` function against this table, so a **missing**
arm fails on its own. It does not catch an **extra** arm — noted as a backlog
row in §17 rather than silently relied on.

### 3.5 `crates/sdk/src/deploy.rs` — `PlanApplier` widens into `SubstrateActor`

Per **D-A5-19** / §0.22: the binding push is the delivery ADR-0021 §5 calls
sticky and A6 exists to make durable, so it must sit behind the swapped
trait.

```rust
/// ADR-0021 §5's narrow "apply this action to that substrate" boundary.
/// Three actions, not one: A3 introduced this trait when applying a plan
/// was the only action, and A5 adds the two the supervisor's own loop
/// issues. A6 replaces the implementation with an outbox/DLQ-backed one
/// and nothing above this trait changes -- which only holds if every
/// action it must make durable is *on* it.
#[async_trait::async_trait]
pub trait SubstrateActor: fmt::Debug + Send + Sync {
    async fn apply_plan(&self, plan: WitDeploymentPlan) -> Result<(), String>;
    async fn write_bindings(
        &self,
        write: BindingWrite,
    ) -> Result<Vec<BindingWriteOutcome>, String>;
    /// Included for the same reason `stop` sits beside `start` on an
    /// engine: a fake in a test must be able to answer every action the
    /// supervisor takes. A6 may keep this synchronous -- a restart queued
    /// for later delivery is usually wrong -- and that is a decision for
    /// A6's implementation, not a reason to leave it outside the trait.
    async fn restart(&self, service_id: String, generation: u64) -> Result<(), String>;
}
```

`DeployTarget.applier: Arc<dyn PlanApplier>` → `actor: Arc<dyn SubstrateActor>`.
`ApplyRequest`/`apply_plan` unchanged apart from the field rename.

Client-side mirror types in `crates/sdk/src/lib.rs`, beside `ServiceStatus`,
with the same **no `rename_all`** note already on `InstancePhase` (the wire
tags are the literal Rust variant names `wit_bindgen`'s `additional_derives`
produces):

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BindingWriteOutcome { Applied, NoOp, Stale(u64), Conflict(u64) }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BindingWrite {
    pub service_id: String,
    pub app_instance_id: String,
    pub bindings: Vec<DependencyBinding>,
    pub generation: u64,
}
```

plus `SyneroymClient::write_bindings`, `::restart`, and
`::release_app_instance`, following `status`'s shape
(`sdk/src/lib.rs:741`).

## §4 — Phase 2: restart

### 4.1 Engines

`crates/sandbox_wasm/src/engine.rs`:

```rust
    /// Evict and recompile `service_id` from the artifact `deploy_wasm`
    /// persisted to `blobs_dir`. A5's remediation half of restart-in-place:
    /// a wasm service has no process, so "restart" means dropping the
    /// cached `InstancePre` (and with it the resolved FDAE policy) and
    /// rebuilding it from disk.
    ///
    /// Fails -- rather than `load_cached_wasm`'s `warn!` -- when no
    /// artifact is on disk: a supervisor must be able to tell "restarted"
    /// from "there was nothing to restart", or bounded remediation counts
    /// a no-op as an attempt and exhausts its budget against a service it
    /// never touched.
    ///
    /// **No identity work.** The instance key is HKDF-derived from the
    /// node identity and the calling DID
    /// (`Identity::derive_service_identity`), so a restart on the same
    /// node under the same caller yields the *same* key and the installed
    /// certificate stays valid; the endpoint record is unchanged, so
    /// there is nothing to republish. The reference scenario's step 4
    /// ("its instance key is new") describes reinstantiation on a
    /// *different* node, not restart-in-place -- see §14's note.
    pub async fn reload_wasm(&self, service_id: &str) -> Result<()> {
        Self::validate_service_id(service_id)?;
        let file_path = self.blobs_dir.join(format!("{service_id}.wasm"));
        if !file_path.exists() {
            anyhow::bail!("no WASM artifact on disk for {service_id}; redeploy it");
        }
        self.stop_wasm(service_id).await?;
        self.load_cached_wasm(service_id).await
    }
```

`load_cached_wasm` stays private; `reload_wasm` is the public entry point.

`crates/sandbox_podman/src/engine.rs`, mirroring `stop` (line 415):

```rust
    /// `podman start <name>`. The other half of a restart -- `stop` alone
    /// leaves the container created but down, which `readyz` correctly
    /// reports as `not-running` forever.
    pub async fn start(&self, service_id: &str) -> Result<()>;
```

`crates/control_plane/src/dummy_sandbox.rs`: mirroring no-op stubs for both,
following the convention A4 set for `readyz`/`is_deployed`. (This does not
repair the pre-existing `--no-default-features` breakage — that backlog row
stays open.)

### 4.2 `control-plane.wit`

```wit
    /// Restart a deployed service in place, without reinstalling it
    /// (A5's bounded remediation). Type-dispatched off the service type
    /// recorded at deploy: a wasm component is evicted and recompiled
    /// from the artifact the substrate already holds; a container is
    /// stopped and started; a `tcp` service is **refused**, because its
    /// process runs outside this substrate and there is nothing here to
    /// restart -- an error the supervisor must see rather than a silent
    /// success it would count as a remediation attempt.
    ///
    /// `generation` is checked against the app instance's recorded
    /// management stamp when the service has one (ADR-0021 §4's
    /// "lifecycle actions"): a superseded supervisor must not be able to
    /// restart services it no longer manages. A standalone service with
    /// no app context is ungated; send 0.
    restart: func(service-id: string, generation: u64) -> result<_, string>;

    /// An app instance's management stamp, or `none` if no deploy has
    /// ever named it here. `adopt`'s read half (§0.26): a supervisor must
    /// be able to learn the held generation before it can mint the next
    /// one, and no service-level record carries it.
    record app-instance-management {
        owner-did: string,
        supervisor-did: option<string>,
        generation: u64,
    }
    app-instance-management-of: func(app-instance-id: string)
        -> result<option<app-instance-management>, string>;

    /// Claim management of an app instance at `generation` -- ADR-0021
    /// §4's operator-minted adopt, made durable at the moment of the
    /// claim rather than on whatever write happens next. Subject to the
    /// same four-case rule as every other write, so two supervisors
    /// racing an adopt lose deterministically here instead of both
    /// believing they won until one issues a deploy.
    claim-app-instance: func(app-instance-id: string, generation: u64)
        -> result<_, string>;

    /// Clear an app instance's management stamp: `supervisor-did` back to
    /// none and `generation` back to 0, keeping the owner. Called by the
    /// supervisor on retire/release. Without it, an adopted instance can
    /// never be hand-deployed again -- a plain `app deploy` presents 0
    /// against a held generation and is refused forever, with no verb
    /// able to undo it.
    release-app-instance: func(app-instance-id: string, generation: u64)
        -> result<_, string>;
```

`undeploy` gains the same parameter (§0.23):

```wit
    undeploy: func(service-id: string, generation: u64) -> result<_, string>;
```

### 4.3 `orchestration.rs` — `restart_impl`

```text
# Same capability gate as deploy: a restart is a lifecycle write.
if !caller.has_capability(substrate:<node>/app/<service_id>, ORCHESTRATOR_DEPLOY):
    return Err("caller ... holds no orchestrator/deploy grant for '<id>'")

# Generation gate, only where an app instance exists (§0.23).
if let Some((instance, _)) = registry.app_context_of(service_id):
    check_generation(&instance, caller, generation)?

match registry.deploy_facts(service_id):
    None -> Err("no service type recorded for '<id>'; redeploy to record it")
    Some((t, ..)) -> match parse_service_type(t):
        Wasm       -> app_sandbox_engine.reload_wasm(id).await
        Container  -> { podman.stop(id).await?; podman.start(id).await }
        Tcp        -> Err("'<id>' is a tcp service; its process runs outside
                           this substrate and cannot be restarted here")
        NativeHost -> Err("'<id>' is a native-host service and has no
                           restart path")
```

`undeploy_impl` gains the same generation check, immediately after its
existing `orchestrator/undeploy` capability gate.

Reusing A4's `service_deploy_facts` means `restart`, `readyz`, and
`instance_phase` all answer from one source. A `tcp` refusal is what makes
A5c's remediation branch honest: a probe-failing TCP service is alert-only,
and there is no way to pretend otherwise.

### 4.4 `roymctl` — the lib split and `svc restart`

Per **D-A5-14**: `apps/roymctl/src/lib.rs` exporting `commands` and
`DEFAULT_API_URL`; `main.rs` reduced to the `clap` parse plus
`commands::run`; `Cargo.toml` gains `[lib] name = "roymctl"`. No behavior
change — it makes `commands::app::handle` (and later
`commands::supervisor::handle`) linkable from a `crates/substrate` e2e,
closing the backlog row A3 opened and A4 grew.

Then delete `SvcCommands::Start` and `SvcCommands::Stop`
(`svc.rs:325-336`, both `MethodNotFound` since before A4) and add
`roymctl svc restart --svc-id <id>`, wired to `SyneroymClient::restart` with
generation 0.

### 4.5 `app deploy` refuses an unmastered manifest that declares dependencies

The standing backlog row *"`app deploy` without `--mint-masters` binds
nothing"* names its own fix: "a manifest declaring `depends_on` should have
no unmastered deploy path at all". A5b closes it for the *supervisor* path
(§12's `submit` always resolves masters, so no unmastered submit exists),
but the **operator** path is where the row was actually filed, and A5b does
not touch it.

Today the deploy prints a warning and continues
([app.rs:568-576](../../../../apps/roymctl/src/commands/app.rs#L568)),
emitting an empty binding list because `emit_bindings` is tied to
`--mint-masters` ([app.rs:607](../../../../apps/roymctl/src/commands/app.rs#L607)).
The app then deploys "successfully" and a guest's first `dependency(...)`
call fails at runtime with `dependency-not-bound`. A warning at deploy time
and a failure at call time is the worst split available: the operator sees
the consequence far from the cause.

Replace the warning with a refusal:

```text
# Before the journal is written, beside the other pre-flight bails
# (D-A3-19: everything that can fail runs before a record exists).
if !mint_masters
   && target_plan.services.iter().any(|s| !s.resolved_dependencies.is_empty()):
    bail!("this manifest declares dependencies ({names}), and without
           --mint-masters they cannot be bound: the plan carries the
           compiler's fabricated ids, not real member masters, so a guest
           calling one by name gets `dependency-not-bound` at runtime.
           Re-run with --mint-masters.")
```

A manifest with **no** `depends_on` is unaffected — an unmastered deploy of
an independent service stays valid, which is what `svc deploy` and every
pre-A0 manifest rely on.

## §4A — Phase 2b: deploy idempotency (failure-matrix row 10)

Per **D-A5-18** / §0.21. Distinct from the epoch guard (which dedups binding
writes) and from the generation gate (which picks between writers).

`service_deploy_facts` gains a column:

```sql
CREATE TABLE IF NOT EXISTS service_deploy_facts (
    service_id        TEXT PRIMARY KEY,
    service_type      TEXT NOT NULL,
    health_check_json TEXT,
    manifest_hash     TEXT,          -- A5a: row 10's dedup key
    created_at        INTEGER NOT NULL
);
```

`EndpointStorage::save_deploy_facts` / `load_all_deploy_facts` and
`EndpointRegistry::set_deploy_facts` / `deploy_facts` widen by one field —
**~20 sites, itemized in §7's call-site table**, not the four implementors
alone.

In `deploy_with_context`, immediately after the capability and generation
gates, **after the stamp is persisted** (§0.27), and **before** any artifact
work:

```text
# §0.27 / D-A5-23: the stamp lands FIRST. `check_generation` has already
# accepted this writer, and the early return below must not be able to
# skip recording that -- an operator's `adopt` followed by an unchanged
# redeploy would otherwise leave the substrate holding the old
# generation. This is a write about *who is writing*, not about what was
# installed, so A2's defer-until-every-step-succeeded rule (which governs
# the bindings, and still does) does not apply to it: recording a manager
# whose deploy then fails is correct, not a half-installed state.
registry.set_app_instance_management(&instance_id, management).await?

# Canonical hash over (manifest, app_context-minus-generation). The
# generation is excluded deliberately: bumping it is a change of *writer*,
# not a change to the deployed service, and hashing it would make an
# adopt force a pointless reinstall of every service. That exclusion is
# exactly why the persist above has to be unconditional.
incoming_hash = blake3(canonical_json(&manifest, &app_context.without_generation()))

if registry.deploy_facts(&service_id).manifest_hash == Some(incoming_hash)
   && !matches!(self.instance_phase(&service_id, recorded_type).await,
                InstancePhase::NotRunning(_) | InstancePhase::NotFound):
    # Row 10: a retry after a lost response. Nothing changed and the
    # instance is up, so this is a no-op that reports success -- not a
    # reinstall that restarts a healthy service.
    info!(service_id, "deploy is identical to what is installed and running; no-op");
    return Ok(())
```

The hash is written only on **full** success, in the same place
`set_deploy_facts` already runs — a half-failed deploy must not be
deduplicated on the next attempt. The liveness condition is what makes a
retry against a *stopped* service still reinstall, which is correct: `restart`
is the cheap path, `deploy` is the repair path.

## §5 — Phase 3: the owner + generation stamp

### 5.1 `crates/core/src/storage.rs`

```rust
/// Who manages an app instance on this substrate (ADR-0021 §4). The
/// generation is a **tiebreaker among already-authorized writers**, not
/// an authorization mechanism: a party without `orchestrator/deploy` is
/// refused regardless of what generation it presents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppInstanceManagement {
    /// First-write-wins, unchanged from A2's `app_instance_owners`.
    pub owner_did: String,
    /// The supervisor that most recently wrote at `generation`. `None`
    /// until an operator's `adopt` names one -- an unadopted instance is
    /// writable by any authorized caller, which is what keeps A0-A4's
    /// hand-deploy path working after this lands, and what
    /// `release-app-instance` returns it to.
    pub supervisor_did: Option<String>,
    /// Minted by the operator's `adopt`, never self-incremented.
    pub generation: u64,
}
```

`EndpointStorage`: **replace** `load_all_app_instance_owners` /
`save_app_instance_owner` with

```rust
    async fn load_all_app_instance_management(
        &self,
    ) -> Result<Vec<(String, AppInstanceManagement)>>;
    async fn save_app_instance_management(
        &self,
        app_instance_id: &str,
        management: &AppInstanceManagement,
    ) -> Result<()>;
    /// Removal path, absent from A2's shape -- the standing backlog row
    /// *"`app_instance_owners` rows never get forgotten"*. Called when the
    /// last service of an instance is undeployed (§5.6). Idempotent.
    async fn remove_app_instance_management(&self, app_instance_id: &str) -> Result<()>;
```

**Four implementors** (verified): `SqliteEndpointStorage`
(`data_db/src/registry_store.rs:170`), `MockStorage`
(`core/src/storage.rs:141`), `FailingEndpointStorage`
(`control_plane/src/service/orchestration.rs:3770`),
`RemoveOwnerFailingStorage` (`router/tests/service_ownership.rs:152`).

### 5.2 `crates/data_db/src/registry_store.rs`

Delete the `app_instance_owners` DDL (lines 147-154) and add:

```sql
CREATE TABLE IF NOT EXISTS app_instance_management (
    app_instance_id TEXT PRIMARY KEY,
    owner_did       TEXT NOT NULL,
    supervisor_did  TEXT,
    generation      INTEGER NOT NULL DEFAULT 0,
    created_at      INTEGER NOT NULL
);
```

Pre-release: no `ALTER`, no version ladder, no data carried over. The old
table stops being created and stops being read — the same
unconditional-create pattern A2 and A4 used, proven by
`an_existing_database_gains_the_app_context_and_binding_tables_on_open`. Add
the sibling test. The upgrade effect (a pre-A5a substrate loses its
app-instance owner rows, and the next deploy re-establishes them
first-write-wins) is recorded in §17 rather than mitigated. See §18
question 7.

### 5.3 `crates/core/src/local_registry.rs`

Replace `app_instance_owner_of` / `set_app_instance_owner` with
`app_instance_management_of` / `set_app_instance_management` /
`remove_app_instance_management`. Rename the
`app_instance_owners: Arc<DashMap<String, String>>` field to
`app_instance_management: Arc<DashMap<String, AppInstanceManagement>>`
(3 construction sites: lines 96/121/264).

### 5.4 `orchestration.rs` — the gate

```rust
    /// ADR-0021 §4's single-writer rule, applied to every write that
    /// changes an app instance: `deploy_with_context`, `write_bindings`,
    /// `restart`, `undeploy`, `release_app_instance`.
    ///
    /// The generation is a tiebreaker, so an *unadopted* instance
    /// (`supervisor_did: None`) accepts any authorized writer -- that is
    /// what keeps A0-A4's operator-driven `app deploy` working unchanged
    /// after this lands, and what `release-app-instance` restores.
    fn check_generation(
        &self,
        app_instance_id: &str,
        caller: &CallerContext,
        presented: u64,
    ) -> Result<AppInstanceManagement, String>;
```

Rule table — the returned value is what the caller then persists.
**Corrected post-review**: the first two rows below originally read `no
row | any` and `supervisor_did: None | any`, both unconditionally
claiming supervision. That literal reading is what A5a first shipped, and
it broke the doc comment's own "unadopted instance accepts any authorized
writer" property the moment an app instance had *any* prior write — an
app instance's first-ever deploy (presenting `generation: 0`, the A0-A4
convention) stamped itself in as supervisor, locking out every later
un-adopted writer, including a node-wide caller redeploying over a
different owner. Split on `presented == 0` below to fix it: 0 means
unmanaged (the WIT `app-context.generation` doc), so it must never claim
supervision, regardless of what row is held.

| Held | Presented | Result |
|---|---|---|
| no row | `0` | `Ok` — record `(owner=caller, supervisor=None, generation=0)` |
| no row | `> 0` | `Ok` — record `(owner=caller, supervisor=caller, generation=presented)` |
| `supervisor_did: None` | `0` | `Ok` — no change (the row is returned as held) |
| `supervisor_did: None` | `> 0` | `Ok` — record `supervisor=caller, generation=presented` |
| `generation = g` | `> g` | `Ok` — record `supervisor=caller, generation=presented`. **This is what makes `adopt` land.** |
| `generation = g`, `supervisor = caller` | `== g` | `Ok` — no change. **This is the supervisor's steady state** (§0.18) |
| `generation = g`, `supervisor ≠ caller` | `== g` | `Err` — two supervisors at one generation (matrix row 8) |
| `generation = g` | `< g` | `Err`, **and the message names `g`** (matrix row 9) |

One consequence worth stating explicitly rather than leaving implicit:
`write_bindings_impl` gained its own app-instance owner check in the same
review round (§3.3), because the pre-correction table's bug had been
accidentally standing in for one — a non-owner used to be rejected here
as "a second writer at the same generation" before ever reaching a real
authorization gate. Fixing this table without that addition would have
left `write_bindings` reachable by any authorized writer of any service
in an app instance it does not own.

The last row's error text is load-bearing, not cosmetic — A5b parses it:

```
"app instance '<id>' is managed at generation <g> by <supervisor_did>;
 this write presented generation <p>. Stop managing this instance and
 alert -- never self-increment (ADR-0021 §4)."
```

Call sites: `deploy_with_context` (presenting `app_context.generation`, §0.18
— **not** a hardcoded 0), `write_bindings_impl`, `restart_impl`,
`undeploy_impl`, `claim_app_instance_impl`, `release_app_instance_impl`.
`deploy_with_context`'s existing app-instance-owner check
([orchestration.rs:905-914](../../../../crates/control_plane/src/service/orchestration.rs#L905))
stays as-is and the generation check follows it.

**Every one of those sites persists the returned value immediately**, before
anything else it does (§0.27, D-A5-23).

### 5.5 Where the management row is persisted

**Not** in `install_app_context` (§0.27). The terminal
`set_app_instance_owner` call
([orchestration.rs:362-365](../../../../crates/control_plane/src/service/orchestration.rs#L362))
is **deleted**, and `set_app_instance_management` runs at each gate site
immediately after `check_generation` returns — in `deploy_with_context` that
is before §4A's dedup and before any artifact work.

`install_app_context` keeps writing the app-context and binding rows under
A2's deferral rule, unchanged. Splitting the two is the point: bindings
describe what was installed and must not survive a failed deploy; the stamp
describes who is writing and must not be lost to an early return.

### 5.6 `release_app_instance_impl`, and forgetting the row

```text
# Node-wide `orchestrator/deploy` (§0.28, D-A5-24). NOT an invented
# `app-instance/<id>` selector -- `covers_resource` matches over a
# documented selector set with no such segment, so an app-scoped grant
# would not cover it and only `substrate/admin` would. And not
# `app/<app_instance_id>` either: that puts app-instance ids and service
# ids in one namespace, so a grant for `app/inst-1` would also cover a
# service named `inst-1`. Node-wide is an existing, issuable shape, and
# honest -- an app instance spans services, so releasing it is a
# node-scoped act.
if !self.has_node_wide_ability(caller, Ability::ORCHESTRATOR_DEPLOY):
    return Err("caller ... holds no node-wide orchestrator/deploy on this
                substrate; releasing an app instance is node-scoped
                because the instance spans services")

# The releasing writer must be the current manager (or ahead of it), so a
# superseded supervisor cannot release the instance out from under the
# live one.
management = check_generation(&app_instance_id, caller, generation)?
management.supervisor_did = None
management.generation = 0
registry.set_app_instance_management(app_instance_id, management).await
```

And in `undeploy_impl`, after `remove_app_context` succeeds:

```text
# Backlog row: `app_instance_owners` rows never get forgotten. The
# in-memory app-context map is the instance's membership, so once no
# service on this node names the instance, its management row is dead
# weight and its id can never be reclaimed by another caller.
if registry.app_context_of_any(&instance_id).is_none():
    registry.remove_app_instance_management(&instance_id).await?
```

needing one small accessor over the existing `service_app_contexts` DashMap.

### 5.7 `app_instance_management_of` and `claim_app_instance_impl` (§0.26)

The read:

```text
# A read, so `orchestrator/status` -- node-wide, or the caller is already
# the recorded owner or supervisor. Deliberately no new resource shape
# (§0.28): the only two narrower principals that could legitimately read
# this are named in the row itself.
held = registry.app_instance_management_of(&app_instance_id)
if !self.has_node_wide_ability(caller, ORCHESTRATOR_STATUS)
   && held.map_or(true, |m| m.owner_did != caller.caller_did
                        && m.supervisor_did.as_deref() != Some(&caller.caller_did)):
    # `Ok(None)`, not an error: indistinguishable from "no deploy has ever
    # named this instance here", so a caller with no grant cannot use this
    # to probe for an instance's existence -- the rule A4-10 already
    # applies to `status`'s `not-found`.
    return Ok(None)
return Ok(held)
```

The write (**corrected post-review**: the explicit `supervisor_did`/
`generation` assignment below this document originally specified is gone
— see the note after the block):

```text
if !self.has_node_wide_ability(caller, Ability::ORCHESTRATOR_DEPLOY):
    return Err(...)                                    # §0.28's gate

# Generation 0 means unmanaged (the WIT `app-context.generation` doc);
# claiming *at* 0 is a contradiction, not a request the gate below can
# make sense of, so it is refused outright rather than persisting a row
# with no supervisor recorded while still reporting success.
if generation == 0:
    return Err("app instance '<id>' cannot be claimed at generation 0; 0
                means unmanaged, so a claim must present a generation of
                1 or higher")

# Same four-case rule as every other write, so a racing adopt loses here
# rather than at whichever supervisor issues a deploy first.
management = check_generation(&app_instance_id, caller, generation)?
registry.set_app_instance_management(app_instance_id, management).await
```

A `claim` against an instance with **no row at all** creates one with
`owner_did = caller`, the same first-write-wins rule `deploy` uses. That is
what lets a supervisor adopt an instance before its first deploy lands.

Corrected after A5a's own post-implementation review: this document
originally had the write set `management.supervisor_did = Some(caller.
caller_did.clone())` and `management.generation = generation` explicitly
after `check_generation`, mirroring `release_app_instance_impl`'s own
explicit override. That was superseded when `check_generation` was fixed
to never claim supervision at `presented == 0` (§5.4's rule table said
"no row → any → record supervisor=caller", which is what this document's
draft literally implemented, and which contradicted the WIT `app-context.
generation` doc's "0 means unmanaged" on the instance's very first write).
Restoring the explicit override here would have let `claim(id, 0)`
"work" by writing a supervisor at the generation that means unmanaged --
so the write now refuses `generation == 0` and otherwise persists
whatever `check_generation` returns, unmodified, the same way every other
gate site does.

## §6 — Phase 4: binding state on `service-status`

`control-plane.wit`, added to `record service-status`:

```wit
        /// Per declared dependency of **this** service, the epoch this
        /// substrate currently serves it. Read from the per-dependent
        /// binding row, not the shared resolver entry: the resolver is
        /// keyed `(app-instance-id, service-name)` and is one value per
        /// node, so reading it would give every dependent the same
        /// answer and make "per-dependent convergence" unreadable.
        binding-epochs: list<tuple<string, u64>>,
```

Filled in `service_status_for` from `registry.bindings_of(service_id)`
(§3.1). Closes the backlog row *"A4: `status` reports no binding state"* and
supplies the exit criterion's per-dependent convergence data.

Mirror the field in `crates/sdk/src/lib.rs`'s `ServiceStatus`, in
`crates/sdk/src/health.rs`'s `service_status` test fixture, and in
`health_monitoring_e2e.rs`.

## §7 — A5a call sites

| Change | Sites |
|---|---|
| `EndpointStorage`: 2 methods replaced, 4 added (`load_binding`, `load_bindings_for`, `remove_app_instance_management`, widened deploy facts) | 4 implementors |
| `EndpointRegistry`: owner accessors renamed + 3 added | 3 in `orchestration.rs`, 1 field + 3 constructors in `local_registry.rs` |
| `AppContext` gains `generation` | wire record; `sdk/src/mapper.rs` (`map_deployment_plan_to_wit`'s `AppContext` construction); `apps/roymctl/src/commands/app.rs`; `orchestration.rs`'s `PreparedAppContext` + every `app_context(...)` test helper (`orchestration.rs:2519`); `crates/substrate/tests/multi_substrate_placement_e2e.rs` |
| `ServiceStatus` gains `binding_epochs` | wire type + `sdk/src/lib.rs` + `sdk/src/health.rs` fixture + `service_status_for` + `status_impl`'s `named_missing` arm + `health_monitoring_e2e.rs` |
| **`deploy_facts` gains `manifest_hash`** — ~20 sites, not 5 | `EndpointStorage` (4 implementors); `EndpointRegistry`'s field type, replay, and accessors (`local_registry.rs:86, 119, 158, 262, 330, 339, 346`); three destructuring reads in `orchestration.rs` (607, 1757, 1856); ~10 `set_deploy_facts` test setups in `orchestration.rs` (5510–6342). Mechanical, but budgeted rather than discovered — the same undercount A3 §8.3 and A4 §0.4 each hit once |
| `undeploy` gains `generation` | wire; `sdk::undeploy`; `roymctl svc remove`; dispatch arm; `deploy`'s own two rollback call sites (`orchestration.rs`, which pass the same caller — send the same generation they deployed at); e2e tests |
| `OrchestratorInterface` gains 5 methods | trait, impl, dispatch table, any test double |
| `PlanApplier` → `SubstrateActor`, 1 method → 3 | `sdk/src/deploy.rs` (trait, `SyneroymClient` impl, `DeployTarget`, unit-test fakes); `apps/roymctl/src/commands/app.rs` (2 casts); `crates/substrate/tests/multi_substrate_placement_e2e.rs` |
| `roymctl` lib split | `apps/roymctl/Cargo.toml`, `main.rs`, new `lib.rs` |
| `svc start`/`svc stop` deleted, `svc restart` added | `apps/roymctl/src/commands/svc.rs`, its parse tests |
| Dummy sandbox stubs | `crates/control_plane/src/dummy_sandbox.rs` |

## §8 — A5a tests

Unit, `crates/app_orchestration/src/resolver.rs`:
1. `a_higher_epoch_applies`
2. `an_equal_epoch_with_identical_members_is_a_no_op`
3. `an_equal_epoch_with_different_members_is_a_conflict` (matrix row 7)
4. `a_lower_epoch_is_stale` (matrix row 5)
5. `an_absent_entry_applies`
6. `a_cache_ttl_difference_at_one_epoch_is_not_a_conflict`

Unit, `crates/control_plane/src/service/orchestration.rs`:
7. `write_bindings_is_rejected_without_an_orchestrator_deploy_grant`
8. `write_bindings_refuses_a_service_whose_app_context_names_another_instance`
9. `write_bindings_refuses_a_dependency_the_service_never_declared`
10. `a_binding_write_at_the_current_epoch_with_identical_content_writes_nothing` (matrix row 6)
11. `a_binding_write_does_not_restart_the_service` — assert the sandbox engine's deploy count is unchanged; the property reference-scenario step 5 turns on
12. `two_dependents_of_one_instance_report_their_own_binding_epochs` (§0.20 — fails against the resolver-keyed reading)
13. `a_lower_generation_write_is_rejected_and_the_error_names_the_held_generation` (matrix row 9's substrate half)
14. `a_second_writer_at_the_same_generation_is_rejected` (matrix row 8)
15. `the_recorded_supervisor_may_write_repeatedly_at_its_own_generation` (§0.18's regression guard — the test that would have caught the blocking bug)
16. `an_unadopted_app_instance_accepts_any_authorized_writer` — the A0–A4 compatibility property
17. `releasing_an_app_instance_lets_a_plain_deploy_touch_it_again` (§0.24)
18. `undeploying_the_last_service_of_an_instance_forgets_its_management_row`
19. `an_identical_redeploy_of_a_running_service_is_a_no_op` (**matrix row 10**) — **and asserts the recorded generation advanced** (§0.27; without that assertion this test passes against the bug)
20. `an_identical_redeploy_of_a_stopped_service_still_reinstalls_it` (row 10's boundary)
21. `a_half_failed_deploy_does_not_record_a_manifest_hash`
22. `a_deploy_that_fails_after_the_gate_still_recorded_its_writer` (§0.27's other half — the stamp is not behind the deferral rule)
23. `restart_reloads_a_wasm_component_from_disk`
24. `restart_refuses_a_tcp_service_naming_why`
25. `restart_is_rejected_at_a_lower_generation` (§0.23)
26. `undeploy_is_rejected_at_a_lower_generation` (§0.23, and matrix row 14's blast-radius half at the substrate level)
27. `status_reports_the_epoch_it_currently_serves_per_dependency`
28. `app_instance_management_of_reports_the_held_generation_to_the_owner` (§0.26)
29. `app_instance_management_of_returns_none_to_a_caller_with_no_grant` — not an error, so it cannot be used to probe for an instance's existence (A4-10's rule)
30. `claim_app_instance_records_the_generation_without_any_other_write` (§0.26 — the property that makes `adopt` durable at the moment of the claim)
31. `a_second_claim_at_the_same_generation_is_rejected` — two supervisors racing an adopt
32. `claim_and_release_are_rejected_without_node_wide_orchestrator_deploy` (§0.28)
33. `status_reports_not_found_for_a_named_id_the_caller_may_not_see` — A4-10's rule, re-pinned because §6 adds a field to the same record (renumbered into the slot D-A5-25's withdrawn encryption test vacated, rather than leaving a gap that reads like a dropped test)

Unit, `apps/roymctl/src/commands/app.rs` (reachable now that §4.4 has split
the binary into `main.rs` + `lib.rs`):
34. `a_manifest_declaring_depends_on_is_refused_without_mint_masters` (§4.5), asserting the error names the dependencies
35. `a_manifest_with_no_dependencies_still_deploys_without_mint_masters` — the boundary: an unmastered deploy of an independent service stays valid, which `svc deploy` and every pre-A0 manifest rely on

e2e, **new** `crates/substrate/tests/binding_push_e2e.rs` (two nodes, the
`multi_substrate_placement_e2e.rs` `Node`/`boot_pair` pattern):
36. `a_membership_change_pushed_to_a_dependent_takes_effect_without_a_redeploy` — deploy frontend on A, backend on B, push a changed member list to frontend, assert the guest's next dependency call resolves to the new member and the frontend's wasm component was never recompiled
37. `a_stale_epoch_push_does_not_regress_the_mapping` (matrix row 5, live)

## §9 — A5a merge order

1. §3.1's pure rule + tests 1-6 — no dependencies, mergeable alone.
2. §5 generation stamp (storage → registry → gate → §5.5's persist point)
   + §0.18's `app-context` field + §5.7's two verbs + tests 13-18, 28-32.
   **These ship as one unit**: the field without the gate is inert, the gate
   without the field is §0.18's lockout, and the gate without §5.7's read is
   a stamp `adopt` cannot compute a successor to.
3. §3.2-3.5 `write-bindings` + `SubstrateActor` + tests 7-12.
4. §4A content-hash dedup + tests 19-22. **Must follow (2)**, since §0.27's
   fix is the persist point (2) establishes.
5. §4 `restart`, `undeploy`'s generation, the `roymctl` lib split, `svc
   restart`, and §4.5's `app deploy` refusal + tests 23-26, 34-35. The
   refusal rides with the lib split because that is what makes `app::handle`
   linkable, and so what makes 34-35 the first real tests of it.
6. §6 binding state + tests 27, 33.
7. e2e 36-37.

---

# Part II — A5b: the role, the store, the interface, custody

## §10 — Phase 1: the crate and the store

### 10.1 `crates/app_supervisor/` (`syneroym-app-supervisor`)

Dependencies: `syneroym-app-orchestration`, `syneroym-sdk`, `syneroym-core`,
`syneroym-identity`, `syneroym-rpc`, `syneroym-data-db`,
`syneroym-data-keystore`, `syneroym-mqtt-broker`, `rusqlite`, `tokio`,
`anyhow`, `async-trait`, `serde`, `tracing`. (§0.15: it cannot live in
`app_orchestration` — `sdk` already depends on that, so it would cycle.)

Modules: `lib.rs` (`AppSupervisor`), `store.rs` (`SupervisorStore`),
`service.rs` (the `supervisor` `NativeService`), `keys.rs` (custody, §11.4),
`reconcile.rs` (A5c).

### 10.2 `crates/core/src/config.rs`

```rust
pub struct RolesConfig {
    // ... existing fields ...
    pub supervisor: Option<SupervisorRole>,
}

/// The App Supervisor (ADR-0021 §8). Absent = this node runs no
/// supervisor, which is every deployment through A4.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SupervisorRole {
    /// Reconcile + health sweep interval. A5c; A5b serves RPC only and
    /// sweeps on demand inside `status` (D-A5-21).
    pub poll_interval_secs: u64,        // 30
    /// Desired state, journal, and alerts, under `app_data_dir`.
    pub db_name: String,                // "supervisor.db"
    /// Bounded remediation (A5c, failure-matrix row 13).
    pub max_restart_attempts: u32,      // 3
    pub restart_backoff_secs: u64,      // 30
    /// MQTT topic prefix for published alerts (A5c, D-A5-13).
    pub alert_topic: String,            // "supervisor/alerts"
    /// Where `export-master` writes and `import-master` reads (D-A5-27).
    /// **Operator-declared, never caller-supplied**: the verbs take a
    /// master *name*, not a path, so no caller can steer a private key
    /// to a location of its choosing or read one from outside this
    /// directory. Relative to `app_data_dir`.
    pub master_backup_dir: String,      // "master-backups"
}
```

`crates/substrate/Cargo.toml`: a `supervisor = ["dep:syneroym-app-supervisor"]`
feature, **added to `default`** alongside `community_registry`,
`coordinator_all`, `client_gateway` — otherwise A5b's e2e does not compile
under the workspace's own gates, which run `--all-features` for clippy but
plain `cargo test --workspace` for tests.

### 10.3 `SupervisorStore`

One SQLite file; three concerns (D-A5-11).

```sql
CREATE TABLE IF NOT EXISTS desired_state (
    app_instance_id TEXT PRIMARY KEY,
    plan_json       TEXT NOT NULL,   -- compiled DeploymentPlan, artifacts inlined,
                                     -- masters already substituted (D-A5-7)
    inventory_json  TEXT NOT NULL,   -- alias -> {did, api_url}
    owner_did       TEXT NOT NULL,   -- who submitted
    generation      INTEGER NOT NULL DEFAULT 0,
    paused          INTEGER NOT NULL DEFAULT 0,
    retired         INTEGER NOT NULL DEFAULT 0,
    submitted_at    INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);
```

plus `DeploymentJournal`'s and `AlertStore`'s existing DDL over the same
connection.

### 10.4 D-A5-8: the `Send` fix

`crates/app_orchestration/src/journal.rs` and `.../alerts.rs`:

- field `conn: Connection` → `conn: Arc<Mutex<Connection>>` (std, not tokio —
  every statement is a short synchronous call, and a tokio mutex would make
  `open_in_memory` async for no benefit).
- every method body gains `let conn = self.conn.lock().expect("… lock
  poisoned");` — the idiom this repo already uses for `StaticInventory`'s
  poisoned locks (`resolver.rs:283-286`).
- new `from_connection(conn: Arc<Mutex<Connection>>) -> Result<Self>` on both,
  so `SupervisorStore` can own one connection and hand it to all three.
- ~15 methods in `journal.rs`, ~5 in `alerts.rs`. No signature changes, so no
  call-site changes outside the two files.
- New test in `crates/sdk/src/deploy.rs`: `fn assert_send<T: Send>(_: T) {}`
  over a constructed `apply_plan` future.

## §11 — Phase 2: the `supervisor` interface

### 11.1 `crates/wit_interfaces/wit/supervisor/supervisor.wit`

```wit
package syneroym:supervisor;

interface supervisor {
    /// Submit or replace desired state for an app instance. The plan is
    /// already compiled, with artifacts inlined and member masters
    /// substituted -- the supervisor holds no catalog and never reads a
    /// manifest (D-A5-7).
    record submission {
        app-instance-id: string,
        /// JSON `DeploymentPlan`.
        plan-json: string,
        /// JSON alias -> {did, api-url}.
        inventory-json: string,
        /// The generation this submission writes at. Minted by `adopt`;
        /// 0 for a first submission of an unmanaged instance.
        generation: u64,
    }

    variant managed-state { applying, active, degraded, paused, retired, superseded }

    record managed-service {
        logical-ref: string,
        service-id: string,
        substrate-alias: string,
        substrate-did: string,
        /// Mirrors `sdk::health::Signal`'s discriminant as a string:
        /// "healthy" / "substrate-unreachable" / "instance-not-running" /
        /// "probe-failing" / "unknown" / "not-deployed".
        signal: string,
        detail: string,
        restart-attempts: u32,
    }

    /// Per dependent, per declared dependency: what the supervisor last
    /// wrote versus what the hosting substrate reports serving *for that
    /// dependent* (§0.20). Equal epochs mean converged.
    record binding-convergence {
        dependent-logical-ref: string,
        dependency-name: string,
        written-epoch: u64,
        observed-epoch: option<u64>,
        converged: bool,
    }

    record instance-status {
        app-instance-id: string,
        state: managed-state,
        generation: u64,
        supervisor-did: string,
        last-reconciled-at: option<u64>,
        services: list<managed-service>,
        bindings: list<binding-convergence>,
        /// ADR-0021 §5: delivery is best-effort synchronous. Said here
        /// rather than implied, so an operator reading a converged status
        /// is not reading a guarantee this build cannot make.
        delivery-note: string,
    }

    record alert {
        logical-ref: option<string>,
        substrate-did: string,
        kind: string,
        detail: string,
        first-seen-at: s64,
        last-seen-at: s64,
        cleared-at: option<s64>,
    }

    submit:          func(s: submission)          -> result<_, string>;
    /// Mints the next generation: reads what the substrates report
    /// holding and writes `held + 1`. The only minter (ADR-0021 §4).
    adopt:           func(app-instance-id: string) -> result<u64, string>;
    /// Hand the instance back to manual operation: clears the stamp on
    /// every substrate it is placed on (§0.24). Does **not** undeploy.
    release:         func(app-instance-id: string) -> result<_, string>;
    pause:           func(app-instance-id: string) -> result<_, string>;
    %resume:         func(app-instance-id: string) -> result<_, string>;
    /// Stop managing and release. Deliberately not a teardown: retiring a
    /// running app must not destroy it.
    retire:          func(app-instance-id: string) -> result<_, string>;
    force-reconcile: func(app-instance-id: string) -> result<_, string>;
    /// Write `name`'s master out of the vault into the operator-declared
    /// `master_backup_dir`, and return **only the path written**
    /// (D-A5-27). ADR-0020 §4 makes this backup mandatory, and
    /// mint-in-place means the operator holds nothing until they ask.
    ///
    /// No key bytes in the response, and no caller-supplied path: the
    /// argument is a *name*, so a caller cannot steer a private key
    /// anywhere. The operator collects the file from the host by whatever
    /// means they already use -- which an offline `roymctl` cannot
    /// replace, because the vault is encrypted and its KEK lives only in
    /// this process's memory (§0.31).
    export-master: func(name: string) -> result<string, string>;
    /// Adopt a master an operator already created (an A0-A4 deployment's
    /// `<dir>/identities/member-*.key`, placed into `master_backup_dir`)
    /// into the vault. Reads by name from that same directory; takes no
    /// key material and no path.
    import-master: func(name: string) -> result<_, string>;

    status:  func(app-instance-id: string)            -> result<instance-status, string>;
    alerts:  func(app-instance-id: string, all: bool) -> result<list<alert>, string>;
}

world supervisor-service {
    export supervisor;
}
```

`crates/wit_interfaces/src/supervisor.rs` + `pub mod supervisor;` in
`lib.rs`, following `control_plane.rs`'s `wit_bindgen::generate!` shape
verbatim.

### 11.2 Authorization

Every verb gates on **`substrate/admin` on the supervisor's own node**.
Rationale for the code comment: submitting desired state hands the supervisor
deploy authority on N remote substrates and master keys; there is no resource
narrower than the node that means anything here — the same argument
`security`'s gate already makes
([service.rs:289-309](../../../../crates/control_plane/src/service.rs#L289)).

**This is a coarse stand-in for the two read verbs.** `status` and `alerts`
are monitoring reads and should eventually be holdable by a monitoring-only
credential (`supervisor/status`, mirroring `orchestrator/status`). Gating
them at `substrate/admin` means a read-only operator needs node-owner
authority. Recorded as a backlog row (§17) per AGENTS.md's rule, not left
implicit.

The supervisor's own credentials **against the substrates it manages** come
from `inventory_json`'s per-alias entries, exactly as A3's inventory carries
them. A4 §13 flagged the tension: nothing issues those grants. A5b does not
close it — it surfaces it, by refusing `submit` with a named error when a
placed alias carries no credential.

### 11.3 Registration

In `crates/substrate/src/runtime.rs`:

- `setup_router` registers a `NativeHostChannel` endpoint for interface
  `"supervisor"` under the node's service id, beside the existing
  `"orchestrator"` and `"security"` registrations (lines 429-438), **only
  when `config.roles.supervisor.is_some()`**.
- `RuntimeServices` gains `#[cfg(feature = "supervisor")] supervisor:
  Option<AppSupervisor>`, `init` constructs it, `run_until_shutdown` adds a
  `supervisor_fut` arm to the `tokio::select!`, `shutdown` shuts it down —
  the exact `community_registry`/`coordinator`/`client_gateway` pattern.
- A5b's `run()` is `pending_component().await` after registration; the
  resident loop is A5c. Its **read surface is not idle**, though: per
  **D-A5-21**, `status` runs `sdk::health::poll_once` + `record_report` on
  demand inside the RPC, so the exit criterion is met in A5b and A5c changes
  only the schedule.

Dispatch: the supervisor's `NativeService` registers into
`NativeDispatchRegistry` under `"supervisor"`. `RouteHandler::plan_pipeline`
already routes `(JsonRpc, NativeHostChannel)` to
`ServiceStage::NativeService` (`dispatch.rs:210`), so no router change.

### 11.4 Master custody (moved from A5d — §0.19)

`crates/app_supervisor/src/keys.rs`:

```rust
/// Member master keys, held in the supervisor's own encrypted service
/// vault rather than on disk beside a config (ADR-0020 §4). Keyed by the
/// same computable name A0 uses for files
/// (`member-<instance>-<service>-<index>`), so adoption is a read.
///
/// **Local by construction.** The supervisor writes to *its own* node's
/// service database through `StorageProvider::open_service_db`, an
/// in-process call -- it never issues a remote `security/set-secret` and
/// so never needs `substrate/admin` on a managed substrate. That is what
/// keeps failure-matrix row 14's blast-radius claim intact and what
/// resolves the standing P0 backlog row on this exact question.
pub struct MasterVault { /* storage_provider, key_store, service_id */ }

impl MasterVault {
    pub async fn get(&self, name: &str) -> Result<Option<Identity>>;
    /// Mints and stores if absent. Prints ADR-0020 §4's backup warning to
    /// the log at mint time, and the minted DID is returned on `submit`'s
    /// response so an operator can record it.
    pub async fn get_or_mint(&self, name: &str) -> Result<Identity>;
    pub async fn import(&self, name: &str, key_bytes: &[u8]) -> Result<()>;
}
```

`get_or_mint` is the **ordinary** path (§0.30): `submit` calls it once per
`(app_instance_id, service_name, index)` and substitutes the resulting DIDs
into the plan itself. The deploy path then certifies each placed member
through the supervisor's *own* client — the only way to produce a
certificate the substrate accepts (§0.19).

**No master ever crosses the wire** (§0.29, D-A5-27). `export`/`import` are
RPC verbs, but they move a *file* on the supervisor's host into and out of
the operator-declared `master_backup_dir` — `export` returns a path,
`import` takes a name, and neither carries key bytes. They cannot be an
offline `roymctl` command instead: the vault is an encrypted service
database whose KEK exists only in this process's memory (§0.31).

**`export-master` writes through `Identity::save_to_path`, not through
`fs::write`.** That path already opens with `.mode(0o600)` on unix
([keys.rs:158-169](../../../../crates/identity/src/keys.rs#L158)) and is
pinned by `test_save_to_path_permissions`
([keys.rs:390-400](../../../../crates/identity/src/keys.rs#L390)), so a
member master lands at the same permissions every other locally-stored
identity does rather than at whatever the substrate's umask happens to be.
`MasterVault` creates `master_backup_dir` at `0o700` for the same reason —
the file mode protects the key, the directory mode keeps its *existence*
from being enumerable. A backup-critical key is the last thing that should
inherit a default.

**The vault is locked until an operator injects the KEK** (D-A5-28).
`MasterVault` therefore surfaces that state rather than failing obscurely:

```rust
    /// `Err(VaultLocked)` when `KeyStore` holds no KEK -- the ordinary
    /// state of a freshly-booted supervisor, since the KEK arrives by
    /// `security.inject-kek` and does not survive a restart
    /// (`key_store.rs`: it is in-memory only, and nothing in config or
    /// the environment supplies one). Every caller distinguishes it from
    /// a real storage failure, because the operator action differs.
    pub async fn get(&self, name: &str) -> Result<Option<Identity>, VaultError>;
```

and `AppSupervisor::init` logs the same shape of loud, actionable warning
`setup_connection_router` already logs for an unowned substrate
(`runtime.rs:377-383`):

```text
supervisor role is enabled but its vault is LOCKED: no KEK has been
injected, so it cannot mint, certify, or renew member masters. Inject one
with: roymctl --substrate <this node> security inject-kek --kek-hex <...>
```

`submit` prints each minted DID and the `export-master` invocation beside
it. ADR-0020 §4 calls a member master "backup-critical in the way a root key
is", and mint-in-place means the operator does not hold one until they ask
— so the moment it is created is the moment to say so, exactly as
`resolve_or_mint_member_master` already does for the file-backed case
(`member_identity.rs:112-116`).

### 11.5 An adherence test for the new package

`test_wit_adherence` covers `orchestrator` only. Add the sibling for
`supervisor` in `crates/app_supervisor/src/service.rs`, walking every
function in the WIT interface against the dispatch table with `wit_parser::
Resolve`, copied from
[service.rs:582](../../../../crates/control_plane/src/service.rs#L582).

## §12 — Phase 3: `roymctl supervisor`

`apps/roymctl/src/commands/supervisor.rs`:

```rust
#[derive(Subcommand, Debug, Clone)]
pub enum SupervisorCommands {
    /// Compile a manifest, resolve or mint its member masters, and hand
    /// the result to a supervisor as desired state.
    Submit {
        instance_id: String,
        manifest_path: PathBuf,
        #[arg(long)] inventory: Option<PathBuf>,
        /// Submit at this generation. Omit to reuse the one `adopt` last
        /// minted for this instance.
        #[arg(long)] generation: Option<u64>,
    },
    Adopt   { instance_id: String },
    Release { instance_id: String },
    Pause   { instance_id: String },
    Resume  { instance_id: String },
    Retire  { instance_id: String },
    Reconcile { instance_id: String },
    Status  { instance_id: String },
    Alerts  { instance_id: String, #[arg(long)] all: bool },

    /// Ask the supervisor to write a member master into its configured
    /// `master_backup_dir` and print the path it wrote (ADR-0020 §4's
    /// backup duty). Takes a name, not a path -- the destination is
    /// operator config on the supervisor's side, so no caller can steer a
    /// private key anywhere.
    ExportMaster { name: String },
    /// Ask the supervisor to adopt a master already placed in that same
    /// directory (an A0-A4 deployment's `<dir>/identities/member-*.key`).
    ImportMaster { name: String },
}
```

Both are **ordinary RPC calls**, not offline file work (§0.31, D-A5-27):
the vault is an encrypted service database and its KEK lives only in the
running supervisor's memory, so a separate `roymctl` process could not
decrypt it at all. What keeps the key off the wire is that these verbs move
a file *on the supervisor's host* — `export` returns a path, `import` takes
a name.

The operator still collects the exported file from that host. That is a
file-retrieval problem, solvable with tooling they already have (a mounted
volume, a backup agent, `scp`); the decryption problem was not solvable at
all.

`Submit`'s handler is `app deploy`'s front half, **stopping short of master
resolution**: read manifest → `compile` → load inventory → serialize both →
one `submit` RPC. The plan it sends carries the compiler's **fabricated**
service ids.

**The supervisor mints and substitutes** (§0.30, D-A5-26). Inside `submit`,
before storing desired state, it calls `MasterVault::get_or_mint` once per
`(app_instance_id, service_name, index)` and rewrites every `service_id` and
`resolved_dependencies` entry — `member_identity::
substitute_and_certify_members`' substitution half, moved into
`crates/app_supervisor` where the masters actually live. The certification
half stays out: §0.19 means certificates must be minted by whoever deploys,
which is the supervisor's own apply path.

There is no `--upload-masters` flag and no upload: the no-upload branch
would have submitted a plan naming masters the supervisor does not hold, and
the upload branch needs an encrypted transport no client can produce
(§0.29). Either way there is no unmastered submit path — the **supervisor**
half of the backlog row *"`app deploy` without `--mint-masters` binds
nothing"*, whose operator half §4.5 closes.

`submit`'s response returns each minted DID, and `roymctl` prints them with
the `export-master` command beside each one. ADR-0020 §4 calls a member
master "backup-critical in the way a root key is" and mint-in-place means
the operator holds nothing until they ask — so the mint is the moment to
say so, the same rule `resolve_or_mint_member_master` already follows
(`member_identity.rs:112-116`).

**`submit` refuses against a locked vault** (D-A5-28), naming the fix rather
than failing on the first mint:

```text
this supervisor's vault is locked (no KEK injected), so it cannot mint the
member masters this app needs. Run: roymctl --substrate <supervisor>
security inject-kek --kek-hex <...>, then re-submit.
```

**`emit_bindings` is always `true`** on the supervisor's apply path.
`ApplyRequest.emit_bindings` decides whether the WIT `app-context` carries a
binding list at all, and `roymctl app deploy` ties it to `--mint-masters`
([app.rs:607](../../../../apps/roymctl/src/commands/app.rs#L607)) because a
plan without masters has nothing real to bind. The supervisor always holds
masters by construction (§0.30), so the condition that flag encodes is
always met — hardcoding `true` rather than inheriting `roymctl`'s
conditional is deliberate, and stated here so it is not carried across by
copy.

**A manifest with `Spawn` dependencies is refused, naming why.** `app deploy`
silently keeps only `compiled.plans.last()`, dropping a spawned child's whole
plan — a standing backlog row. Copying that into a new command would
propagate a known bug into a surface that is supposed to manage whole apps.
Refusing is honest, costs nothing, and leaves the row open for `app deploy`
where it already is.

`--substrate <supervisor-node-did>` names the supervisor's node; the existing
global `--as`/`--ucan` supply the `substrate/admin` credential.

## §13 — A5b tests

Unit, `crates/app_supervisor/src/store.rs`:
1. `submitting_twice_replaces_desired_state_and_keeps_one_row`
2. `pause_and_resume_round_trip`
3. `retire_refuses_a_later_submit_until_un_retired` — renamed from
   `retire_is_terminal_and_a_later_submit_is_refused` when review round 2
   made `retire` non-terminal (`un_retire`, called by `handle_adopt`); the
   old name asserted a property that is no longer true

Unit, `crates/app_supervisor/src/service.rs`:
4. `every_verb_is_refused_without_substrate_admin`
5. `submit_is_refused_when_a_placed_alias_carries_no_credential`
6. `submit_is_refused_for_a_manifest_with_spawn_dependencies`
7. `adopt_mints_held_plus_one`
8. `retire_releases_the_stamp_on_every_placed_substrate` (§0.24)
9. `a_supervisor_that_reads_a_higher_generation_marks_the_instance_superseded_and_alerts` (matrix row 9)
10. `status_reports_the_delivery_note_rather_than_implying_convergence` (ADR-0021 §5)
11. `status_polls_on_demand_so_its_signals_are_not_empty` (D-A5-21)
12. `the_supervisor_wit_dispatch_table_covers_every_declared_function` (§11.5)

Unit, `crates/app_supervisor/src/keys.rs`:
13. `a_minted_master_round_trips_through_the_vault`
14. `submit_mints_one_master_per_service_and_substitutes_the_plans_ids` (§0.30)
15. `resubmitting_reuses_the_masters_already_in_the_vault_rather_than_minting_again`
16. `no_supervisor_verb_accepts_or_returns_key_material` — walks the WIT interface asserting no function takes or returns a key, pinning §0.29's by-construction property (`export-master` returns a *path*) so a later slice cannot quietly reintroduce a key-bearing verb
17. `export_master_writes_only_into_the_configured_backup_dir` — a name containing `/`, `..`, or an absolute prefix is refused, so the operator-declared destination cannot be steered (D-A5-27). **Also asserts `mode() & 0o777 == 0o600` on the written file and `0o700` on the directory** (`#[cfg(unix)]`, mirroring `test_save_to_path_permissions`): the write goes through `Identity::save_to_path`, and this is what would catch a later refactor swapping it for `fs::write`
18. `import_master_reads_only_from_the_configured_backup_dir` — the same rejection on the read side
19. `a_locked_vault_is_reported_as_vault_locked_not_as_a_storage_error` (D-A5-28) — **runs against an encryption-enabled fixture**, since §0.31's whole point is that the `encryption_enabled = false` dev profile proves nothing here
20. `submit_against_a_locked_vault_names_inject_kek` (D-A5-28)

Unit, `crates/app_orchestration`:
21. `journal_and_alert_store_are_send_and_sync`
22. `apply_plan_returns_a_send_future`

e2e, **new** `crates/substrate/tests/supervisor_interface_e2e.rs` (a
supervisor node plus one managed node):
23. `an_operator_submits_and_reads_back_status_over_the_supervisor_interface`
24. `a_second_supervisor_that_has_not_adopted_loses_every_write` (matrix row 8, live)
25. `a_supervisor_deploys_a_bound_app_using_masters_it_minted` (§0.19's regression guard — fails if custody is removed from A5b)
26. `adopt_reads_the_held_generation_from_the_managed_node_and_claims_the_next` (§0.26, live)
27. `a_pushed_binding_reaches_a_dependent_the_supervisor_deployed` — the `emit_bindings: true` property (§12); a plan applied with it false binds nothing and this fails

**Fixture note.** `submit` cannot obtain the supervisor's own grants — §11.2
says nothing issues them, and §0.28 raises the bar to a **node-wide**
`orchestrator/deploy` on each managed node. So the fixture hand-issues a
`CapabilityToken` from the managed node's owner to the supervisor node's DID
before any of these tests submits, following
[multi_substrate_placement_e2e.rs:265](../../../../crates/substrate/tests/multi_substrate_placement_e2e.rs#L265)'s
precedent. Written here so the tests are not authored assuming `submit` can
bootstrap its own authority.

---

# Part III — A5c / A5d / A5e

**Planned to phase-and-signature detail only.** Per **D-A5-2**, each needs
its own `§0` findings pass before execution — the A3/A4/A5a experience is
that a slice planned only at this altitude is where the scope-changing
findings hide.

## §14 — A5c: the loop and remediation

1. **`AppSupervisor::run`** — `tokio::interval(poll_interval_secs)`, one pass
   per non-paused, non-retired, non-superseded instance: `poll_once` →
   `record_report` → publish opened alerts to MQTT (D-A5-13) → reconcile →
   remediate.
2. **Reconcile** — `Reconciler::compute_diff` against the stored plan, then
   `deploy::apply_plan` through per-alias `SubstrateActor`s built from
   `inventory_json`. This is where `ActionState::Pending` finally gets a
   writer: the loop enqueues the diff's actions before applying them
   (backlog row).
3. **Remediation** — branch on `sdk::health::Signal`, which A4 separated at
   the source precisely for this:

   | Signal | Action |
   |---|---|
   | `SubstrateUnreachable` | Alert only; nothing to restart on a node you cannot reach (D-A4-13) |
   | `InstanceNotRunning` | `restart`, bounded: `max_restart_attempts` with `restart_backoff_secs` between; on exhaustion → terminal `Degraded`, alert only, **no further restarts** (matrix row 13) |
   | `ProbeFailing` | Policy choice — see §18 question 8 |
   | `Unknown` / `NotDeployed` | Never remediated (D-A4-19's rule applied to actions rather than exit codes) |

   **Restart needs no identity work** (§4.1): the instance key is derived, so
   a same-node restart keeps the same key and certificate, and the endpoint
   record is unchanged. Reference-scenario step 4's "its instance key is new"
   describes reinstantiation on a *different* node — which A5 does not do,
   since relocation stays a non-goal.
4. **Binding push** — after a reconcile changes membership, push to every
   dependent via `SubstrateActor::write_bindings` at the instance's
   generation. `Conflict` → alert. `Stale` → re-read and retry once, then
   alert.
5. **Placement change** — a re-`submit` that moves a service to a different
   substrate hits `check_no_placement_change`'s refusal, which A5 keeps
   (relocation is a milestone non-goal). The loop must **not** retry it: mark
   the instance `Degraded` with a `PlacementChangeRefused` alert and stop
   retrying *that service* while continuing to reconcile the others. Without
   this the loop fails permanently in a hot cycle.
6. **Attempt bookkeeping** — a `remediation` table keyed
   `(app_instance_id, logical_ref)` with `attempts`, `last_attempt_at`,
   `terminal`. Durable, so a supervisor restart resumes rather than resets.
   Cleared on a healthy sweep.
7. **New `AlertKind`s**: `RemediationExhausted`, `BindingConflict`,
   `SupervisorSuperseded`, `PlacementChangeRefused`. Same `AlertStore` table,
   no schema change.
8. **Health-poll-cost budget** (§0.25) — measure a steady-state sweep against
   a node hosting a realistic service count, including `rpc`-probed wasm
   services. Resolves or re-targets the backlog row *"A4: a wasm `rpc` probe
   costs a component instantiation"* with a number, and decides whether
   `probe_cached`'s missing single-flight (A4's other row) now matters, since
   A5c is the second concurrent poller that row was waiting for.

Closes matrix rows 11, 12, 13.

## §15 — A5d: unattended renewal

Custody is already in place (A5b, §11.4). A5d automates the cadence.

**Where "unattended" stops, stated up front** (§0.31, D-A5-28). The
supervisor reads its own vault only after an operator has injected the KEK,
and the KEK is memory-only — it does not survive a restart. So ADR-0020 §3's
"issues and renews unattended" means **unattended between KEK injections**: a
supervisor host that reboots stops certifying and renewing until a human runs
`inject-kek`, and every managed instance's certificates then age out on the
attended posture's timetable (failure-matrix row 3's outage).

That is inherited from M04A's KEK design, not introduced here, and A5d does
not fix it — a KEK that survives a restart is a key-management decision with
its own threat model. What A5d owes is honesty: the renewal loop raises a
`VaultLocked` alert (a new `AlertKind`) the moment it finds the vault shut,
so the gap between "unattended" and "unattended until reboot" is visible in
the same place every other supervisor fault is, rather than discovered when
a certificate expires. Backlog row in §17.

1. **Renewal** — each pass, for every managed member whose installed
   certificate is near expiry by `is_near_expiry_parts` (A4 moved this into
   `syneroym-identity` so the substrate sweep and this share one definition),
   reissue via `deploy::certify_instance` through the supervisor's own client.
2. **`RotationPolicy` becomes load-bearing** here and only here:
   `RestartOnRotation` → install then `restart`; `None` → install only.
3. **The `SynSvcNativeService` gap** — a certificate installed outside
   `deploy` leaves that service's by-value copy stale, and every
   `RelationshipProof` it signs then fails verification (backlog row). The
   install path must rebuild or refresh it. **A correctness prerequisite, not
   a follow-up**: without it, unattended renewal breaks cross-service calls
   rather than fixing them.
4. **Certificate maximum lifetime** — enforce one at install (backlog row
   *"No maximum lifetime enforced on an installed instance certificate"*,
   whose own text calls the online-key posture its natural fit): with renewal
   automated, short-lived certificates stop being an operational burden and
   become the default ADR-0020 §3 assumes.
5. **Master-anchor refresh on a schedule** — the backlog row describes it as
   "a daily operator duty nothing performs on a schedule", plus a
   read-modify-write race. The loop performs it; the race needs a
   compare-and-swap or a single-writer argument from the generation stamp.
6. **Revocation surface** — `roymctl supervisor revoke-instance` (backlog
   row *"No operator surface for revoking an instance key"*).

Closes matrix rows 1/3's automation half and row 14's second half — the
latter needing an explicit test that a compromised supervisor's reach is
bounded to the instances it manages (§0.23's generation gates on `restart`
and `undeploy` are what make that testable).

## §16 — A5e: scale-out, cross-app, budgets

1. **`ServiceSpec.replicas: u32`** (default 1, `#[serde(default)]`,
   skip-if-1 so no existing manifest changes) → `compile` emits N members.
   Whether that is `PlannedService.members: Vec<ServiceId>` or N
   `PlannedService`s is A5e's own `§0` call; the second keeps `PlannedService`
   a 1:1 unit of deploy work, which `apply_plan`, the journal, and
   `deployed_service_id` all already assume.
2. **Member-index generalization** — `deployed_service_id` and
   `substitute_and_certify_members` stop hardcoding `0`;
   `certify_placed_members`' one-master-per-service assertion
   (`deploy.rs:292-302`) relaxes to one-master-per-*member*. Closes the
   backlog row.
3. **Cross-app `Bind`** — a manifest surface naming which service of the
   bound instance is depended on (§0.9); the compiler resolves it; the host
   resolves the local declared name against the foreign `app_instance_id`,
   which ADR-0021 §2 already specifies. §0.20's per-dependent binding rows
   are what make two dependents holding different views representable.
4. **The probe** — online-key posture: the supervisor calls *as* the
   depending member using the master it holds, an active liveness signal.
   Attended: passive only, and the status output says which is in force.
   Matrix rows 15 and 18.
5. **Convergence budget** — measure from a membership change to every
   reachable dependent serving the new epoch, read directly off §6's
   `binding-epochs`. Compare against the 5 s provisional target, **write the
   answer down either way**, and evaluate ADR-0021 §6's trigger explicitly.
   `task.md` makes this an exit criterion, and a measurement taken but not
   recorded fails it.
6. **No-network-hop bench** (§0.25) — the Criterion case pinning A2's
   "resolution adds no network hop" budget, an open backlog row and the third
   of `task.md`'s three budgets.
7. **Reference scenario** — the full six-step e2e over two real nodes, which
   only becomes buildable here.

---

## §17 — Docs and backlog

**Docs**
- `docs/developer-guide.md` — a `[roles.supervisor]` config block; a
  `roymctl supervisor` walkthrough beside the multi-substrate deploy section;
  the `substrate/admin` grant the supervisor interface needs and the
  **node-wide** `orchestrator/deploy` grant the *supervisor* needs on each
  managed node — node-wide, not app-scoped, because `claim`/`release` are
  node-scoped acts (§0.28); the MQTT alert topic and the `subscribe` call to
  read it; the two postures and how to choose; that `retire`/`release` are
  how an instance goes back to manual operation; and — prominently — that
  the supervisor **mints member masters into its own vault**, that
  `roymctl supervisor export-master` writes each one into
  `master_backup_dir` for the operator to collect (the backup ADR-0020 §4
  calls mandatory, §0.29/§0.30), and that **the vault stays locked until
  `security inject-kek` runs after every restart**, so certification and
  renewal stop until it does (§0.31). The startup order — boot, inject the
  KEK, then submit — belongs in the walkthrough, not in a footnote.
- `task.md` — dated corrections for §0.1-§0.30; the A5 row split into
  A5a-A5e; ✅ on matrix rows 16/17; row 8's wording corrected per §0.10; rows
  15/18 annotated with their manifest-surface prerequisite; row 10 annotated
  with the content-hash mechanism.
- `status.md` — one section per sub-slice, matching A0-A4's shape.
- **`docs/planning/traceability-matrix.md`** — `[LFC-MGT]` (App Supervisor)
  and `[FND-IDT]` (stable service identity) flipped to Complete with
  evidence at A5e sign-off. An exit criterion; missing from the first draft's
  docs list.
- ADR-0021 — amendment for §0.10's "the substrate is the durable arbiter of
  the generation" and §0.22's three-action trait, neither of which §4/§5
  states. Decide at A5a sign-off.
- ADR-0020 — amendment for §0.8's custody transport and §0.19's
  certificates-are-bound-to-the-caller consequence, which §3/§4 leave
  undefined. Decide at A5b sign-off.

**Backlog rows resolved** (move to *Recently resolved* as each lands)

*A5a:* "No binding-only write path"; "A2's binding write is last-write-wins,
no epoch guard"; "`roymctl svc start`/`svc stop` call orchestrator methods
that do not exist"; "A4: `status` reports no binding state"; "A3:
failure-matrix row 10 is unmet" (§4A's content hash — a real mechanism, not
the generation gate); "`app_instance_owners` rows never get forgotten"
(§5.6); "A3: `app::handle` has no test coverage" (§4.4's lib split makes it
linkable; A5b adds the e2e that uses it); **"`app deploy` without
`--mint-masters` binds nothing"** — §4.5 turns the warning into a refusal
for a manifest that declares `depends_on`, which is the fix the row itself
names ("no unmastered deploy path at all"). Recorded against **A5a**, not
A5b: A5b's `submit` only removes the unmastered *supervisor* path, and the
row was filed against `roymctl app deploy`, which A5b does not touch. Both
halves are needed to close it.

*A5b:* "A3: `apply_plan`'s future is not `Send`"; "A4: the alert store is a
local file A5 cannot read"; "A supervisor needing
vault writes must hold `substrate/admin` on every managed substrate" —
**resolved by the custody design, not by building the per-service gate**:
§11.4's vault is the supervisor's *own* node's service database, written
in-process, so no remote `set-secret` is ever issued. The P0 row's underlying
ask (a per-service `set-secret` gate at `substrate:<node>/app/<svc>`) stays
open as its own row, retargeted to whenever supervisor-provisioned secrets
for *managed* services arrive — which A5 does not do.

*A5c:* "A3: `Degraded` has no automatic exit"; "A3: `ActionState::Pending`
has no writer"; "A4: alerts are not published to MQTT".

*A5d:* "unattended certificate renewal"; "a renewal outside `deploy` must
refresh `SynSvcNativeService`"; "no operator surface for revoking an instance
key"; "no maximum lifetime enforced on an installed instance certificate";
"master-anchor refresh is a read-modify-write with a race".

*A5e:* "A4: `deployed_service_id` assumes member index 0"; "cross-app `Bind`
dependency naming has no manifest surface"; "no Criterion bench case pinning
A2's 'no network hop' budget".

**Backlog rows retargeted, with the reason**
- *A relocated-away substrate keeps trying to publish a member's record* —
  targeted A5, but A5 does not relocate: `task.md`'s non-goal holds and A5c
  §5 refuses a placement change rather than performing one. → **post-M5**,
  with relocation.
- *A3: a redeploy that moves a service is refused, not relocated* — stays
  open for the same reason. A5c adds the one thing that was missing (the
  loop stops retrying a refusal instead of hot-cycling on it) but does not
  relocate. → **post-M5**.
- *Stale `StaticInventory` entries after undeploy or a dependency-dropping
  redeploy* — the row expected A5's "lifecycle management" to evict them.
  A5 makes the resolver a write target but adds no eviction trigger: `retire`
  is deliberately not a teardown, so it must not remove bindings a running
  app still resolves. The row's own analysis (bounded by app instances ×
  dependency names, re-derived correctly on restart) still holds. → **TBD**.
- *A4: `probe_cached` has no single-flight* — retarget from "A5" to **A5c**
  specifically, where the resident loop becomes the second concurrent poller
  the row was waiting for, and §14 step 8 measures whether it matters.
- *A4: a wasm `rpc` probe costs a component instantiation* — retarget to
  **A5c**, resolved or re-scoped by the poll-cost measurement.
- *`Relation.service` in an FDAE policy is not a declared-dependency name* —
  a policy-authoring concern; nothing in A5 touches policy authoring, and the
  supervisor's push does not make it more or less true. → **TBD**, off A5.
- *A deploy-only grantee's rollback attempt is denied a second time* — a
  grant-shape issue in `undeploy`'s gate, unrelated to supervision. A5a
  touches `undeploy`'s signature but not its capability logic. → **TBD**,
  off A5.

**Backlog rows to add**
- *`supervisor` read verbs are gated at `substrate/admin`* (§11.2) — a coarse
  stand-in; a monitoring-only credential should hold a `supervisor/status`
  ability instead, mirroring `orchestrator/status`. → **post-M5**.
- *App-instance lifecycle verbs need node-wide `orchestrator/deploy`*
  (§0.28) — `claim`/`release` are gated node-wide because no selector
  namespace exists for an app instance, and reusing `app/` would put
  app-instance ids and service ids in one namespace. A narrower resource
  shape needs a selector-namespace decision (a new first segment, plus the
  `covers_resource` docs and tests that go with it). → **post-M5**.
- *Master backup requires collecting a file from the supervisor's host*
  (§0.29, §0.31) — mint-in-place means the operator holds nothing until
  `export-master` writes it into `master_backup_dir`, and retrieving it from
  there is theirs to arrange. `task.md`'s non-goals already say backup is
  "an operator duty this milestone documents rather than automates"; what is
  new is that the file starts on the supervisor's host rather than the
  operator's. A remote export would put the key on the wire, so it is gated
  on the encryption-client row below. → **post-M5**.
- ***The supervisor's vault is locked until an operator injects the KEK, and
  the KEK does not survive a restart*** (§0.31, D-A5-28) — `KeyStore` is
  memory-only (`key_store.rs:29-31`), its sole production populator is the
  `security.inject-kek` RPC (`service.rs:328`), and no config field or env
  var supplies one. A rebooted supervisor therefore cannot mint, certify, or
  renew until a human acts, which bounds ADR-0020 §3's "unattended" claim
  (§15) and is why A5d raises a `VaultLocked` alert rather than implying
  otherwise. Inherited from M04A's KEK design, surfaced here because A5d is
  the first component whose correctness depends on it *continuously* rather
  than at deploy time. A restart-surviving KEK is a key-management decision
  with its own threat model. → **post-M5**.
- ***No client in the tree can use the substrate's E2E encryption layer***
  (§0.29) — the router implements `?enc=ecdh-p256` termination
  (`route_handler/encryption.rs`, `routing.rs`) but
  `RoutePreamble::binary_json_rpc` hardcodes `enc: None`, `SyneroymClient`
  never sets it on either the Iroh or HTTP path, and there is no
  client-side ECDH anywhere — the only `enc = Some` is the preamble parser
  and the only ECDH bench is the *server* handshake. So the layer that
  protects traffic through an untrusted relay is unreachable in practice.
  Building the client half (ephemeral P256 keypair, handshake exchange,
  AES-GCM framing) against the existing server half unblocks an RPC
  `import-master` (§0.8 option (b)) and would also need an
  invocation-level `transport_encrypted` flag to refuse a plaintext call
  (the withdrawn D-A5-25). **Found while planning A5, not caused by it**,
  and larger than a supervisor slice should carry. → **post-M5**, or its
  own slice.
- *`svc restart` cannot restart a `tcp` service* — by construction (§4.3);
  a supervisor's only remediation for one is alerting. → **TBD**.
- *`test_wit_adherence` catches a missing dispatch arm but never an extra
  one* — true of both the `orchestrator` and (new) `supervisor` tables; an
  arm for a function no WIT interface declares is unreachable dead code that
  no test flags. → **TBD**.
- *`app_instance_management` replaces `app_instance_owners` with no
  migration* — pre-release and deliberate; a pre-A5a substrate loses its
  app-instance ownership rows on upgrade and the next deploy re-establishes
  them first-write-wins. Recorded as a known upgrade effect, not a task.
- *A5's supervisor holds no issued grants on the substrates it manages* —
  A4 §13's tension, unclosed: `submit` refuses an alias with no credential
  rather than obtaining one. Issuing grants to a supervisor principal is
  operator work with no tooling. → **post-M5**.
- Whatever A5c/A5d/A5e's own `§0` passes find.

---

## §18 — Questions for the requester

1. **§0.1 / D-A5-1 / §2.** Accept the five-way split and its boundaries?
   A5a in particular is a substrate-only slice with no supervisor in it —
   confirm shipping it first, and separately, is wanted.
2. **§0.5 / D-A5-7.** Confirm `submit` carries a **compiled plan with
   artifacts inlined and masters substituted**, and that the supervisor never
   reads a manifest or holds a catalog.
3. **§0.7 / D-A5-9.** Confirm "rebuildable from manifests plus a substrate
   sweep" means *re-submittable*, not *reconstructable without a manifest*.
4. **§0.8 / §0.19 / D-A5-15.** Master custody now sits in **A5b**, because
   certificates are bound to the calling client and a supervisor cannot
   deploy a bound app without the master. Confirm both halves: the earlier
   placement, and mint-in-place **plus** explicit import (rather than one or
   the other).
5. **§0.14 / D-A5-17.** Confirm multi-member support (`replicas`, the
   compiler change, reference-scenario step 5) is A5e and not earlier — it is
   the largest hidden item, and steps 4/5 are "the milestone's real claim".
6. **§0.9 / D-A5-16.** Confirm rows 15/18 are understood to carry a
   manifest-surface prerequisite, and that adding that surface is in scope for
   A5e rather than deferred past the milestone.
7. **§5.2 / D-A5-6.** Confirm replacing `app_instance_owners` outright
   (dropping its rows) over an `ALTER TABLE`. Pre-release policy says in place
   with no ladder; this is the first time that policy meets a table with live
   rows in it.
8. **§14 step 3.** Should `ProbeFailing` trigger a bounded restart by
   default, or alert only? "Running but not ready" is often restart-fixable
   and often not; `task.md` calls it "a policy choice" and does not make it.
9. **§14 step 5.** Confirm that a re-`submit` with changed placement should
   be a refusal the loop stops retrying (marking the service `Degraded`),
   rather than A5 building operator-initiated relocation. Relocation is a
   milestone non-goal, but the non-goal's text is about *remediation*, and a
   deliberate operator re-placement is a different act. *(Reviewer's answer:
   keep it out — relocation needs an `undeploy` on the old substrate plus an
   ordering rule, which is A6-shaped work on the same durability gap.
   Recorded here pending the requester's confirmation.)*
10. **§0.28 / D-A5-24.** Confirm node-wide `orchestrator/deploy` for
    `claim`/`release` over inventing an `app-instance/` selector. The cost is
    real: a supervisor managing one app on a shared substrate needs a
    node-wide grant there. The alternative is a new selector namespace, with
    the `covers_resource` semantics and tests that implies.
11. **§0.29 / D-A5-27 — the one that moved a design decision, not a
    number.** No client in this tree can send `?enc=`, so the plan takes the
    key off the wire entirely: the supervisor mints in place, and
    adoption/backup are local offline operations on its host. The
    alternative is to **build the client-side encryption half in A5a**
    (ephemeral P256 keypair, handshake, AES-GCM framing, against the
    existing server half) and keep an RPC `import-master`. That is a real
    transport capability the product is missing regardless of A5 — the
    question is whether a supervisor slice is where it should be paid for.
    Recommended as written: mint-in-place is *stronger* (nothing to
    intercept), and the encryption client is filed as its own row so it is
    picked up rather than smuggled in here.
12. **§0.30 / D-A5-26.** Confirm the supervisor mints and substitutes, with
    the plan submitted carrying fabricated ids. The consequence to accept
    explicitly: the operator does not hold a member master until they run
    `export-master` and collect the file, and until then losing that node is
    the unrecoverable loss ADR-0020 §4 describes.
13. **§0.31 / D-A5-28 — an inherited constraint the milestone should decide
    it accepts.** The supervisor's vault is unreadable until an operator
    injects the KEK, and the KEK is memory-only, so a rebooted supervisor
    silently stops certifying and renewing until a human acts. A5 surfaces
    this (a loud startup warning, a `submit` refusal naming `inject-kek`, a
    `VaultLocked` alert) but does not fix it. Confirm that is the right
    scope — the alternative is a restart-surviving KEK, which is a
    key-management design with its own threat model and does not belong in a
    supervisor slice. Worth an explicit answer because ADR-0020 §3's
    "unattended" is one of the milestone's headline claims.

---

# Part IV — A5c: the loop and remediation

**Status:** 📋 Planned (2026-08-01). Not started. This is the `§0` findings
pass **D-A5-2** requires before A5c is handed to an implementer. Part III's
§14 planned A5c to phase-and-signature detail only; everything below is what
§14, `task.md`, and A5b left open, understated, or stated wrongly.

Same discipline as A0 §6 / A1 §6 / A2 §0 / A3 §0 / A4 §0 / P0 §0 / A5 §0.

**Headline.** §14 has eight steps. Reading the code A5c builds on turns them
into **eighteen findings, nine of which change what A5c has to build**, and
one of which is a **live defect in A5b that A5c is the first slice able to
see**: the supervisor has no placement-change refusal at all, so a re-submit
that moves a service silently runs two live copies of one member (§19.1).
A5c is not "put A5b's `status` on a timer." Roughly half of it is making
things A5b returns empty or hardcoded actually have a source.

---

## §19 — What §14, `task.md`, and A5b leave open, understate, or state wrongly

### 19.1 (Scope-changing, blocking) The supervisor has no placement-change refusal, and §14 step 5 says it does

§14 step 5 opens: "a re-`submit` that moves a service to a different
substrate hits `check_no_placement_change`'s refusal, which A5 keeps." That
is not true on the supervisor's side.

`check_no_placement_change` is a **private function in
`apps/roymctl/src/commands/app.rs:290`**, called from exactly one place
(`app.rs:574`, inside `roymctl app deploy`). It is not in `syneroym-sdk`,
not in `syneroym-app-orchestration`, and not reachable from
`crates/app_supervisor`. `SupervisorService::deploy_submission`
([service.rs:267](../../../../crates/app_supervisor/src/service.rs#L267))
goes straight from `placed_aliases` to `mint_and_substitute` to
`build_clients` to `apply_plan`, with no placement check anywhere on the
path.

**Consequence, today, with no loop involved.** An operator re-submits a plan
whose `frontend` moved from alias `edge-1` to `edge-2`. `submit` accepts it,
`apply_plan` deploys `frontend` on `edge-2`, and the copy on `edge-1` **keeps
running**. Both copies hold the same member master DID, both serve calls,
both write rows into their own local databases under that one identity. This
is exactly the two-publisher state D-A3-12 introduced the refusal to prevent,
reachable through a verb A5b shipped.

**Failure-matrix row 20 does not cover this.** Row 20's guarantee is that the
old substrate's *endpoint record* replay is rejected once the master signs a
newer one. That settles name resolution and nothing else. The old process is
still up, still reachable by anything holding a cached address or a direct
connection, and still the sole holder of the rows it wrote.

**Also, `check_no_placement_change` cannot simply be moved.** Its error
message calls `member_identity::deployed_service_id(dir, svc)`
(`app.rs:299`), which reads the operator's `<--dir>/identities/member-*.key`
files. The supervisor has no `--dir` and its masters are in the vault, not on
that path. So A5c builds a supervisor-side refusal, it does not lift one.

**Fix.** A pre-flight refusal in `handle_submit`, in the same block as the
`retired` and `generation` checks (`service.rs:411-431`) — before
`deploy_submission` runs, for the same reason B3 and N1 moved those there.
The inputs already exist on this side: `journal.
get_completed_actions_for_instance` plus `deploy::current_placement`, which
`handle_status` already uses at `service.rs:829`. `force-reconcile` needs the
identical check, for N3's reason: it never calls `store.submit`, so nothing
on that path would refuse anything. See **D-A5c-1**.

**This is an A5b defect, not new A5c work.** Recorded that way in `status.md`
so the slice that shipped it owns it, and fixed in A5c because that is the
next slice open.

### 19.2 (Scope-changing) `compute_diff` → `apply_plan` is the wrong pairing, and it is what the resume/skip backlog row is really about

§14 step 2 says: "`Reconciler::compute_diff` against the stored plan, then
`deploy::apply_plan` through per-alias `SubstrateActor`s." Two problems.

**(a) `apply_plan` cannot execute a diff.** It iterates
`resolve_targets(plan, ...)` and appends only `"ADD"` actions
([deploy.rs:220-243](../../../../crates/sdk/src/deploy.rs#L220)). There is no
`REMOVE` branch and no `UPDATE` branch. `ReconcileAction::Remove`
([reconcile.rs:15](../../../../crates/app_orchestration/src/reconcile.rs#L15))
has no executor anywhere in the tree. So a re-submit that drops a service
from the plan leaves that service running on its substrate forever, and the
loop never notices — it is not in the plan, so nothing polls it either.

**(b) The skip check is scoped to one `deployment_id`, and that is
deliberate at its other caller.** The backlog row *"The supervisor's
`submit`/`force-reconcile` never skip an already-landed service"* names the
fix as re-scoping `apply_plan`'s skip and requires every caller be audited
first. Audit:

| Caller | `deployment_id` it passes | Effect of re-scoping the skip to the instance |
|---|---|---|
| `roymctl app deploy` ([app.rs:604-637](../../../../apps/roymctl/src/commands/app.rs#L604)) | Reuses the latest record **only** when its state is `Applying`/`Degraded` **and** the plan is byte-identical; otherwise a fresh id | **Breaking.** An unchanged `app deploy` against a healthy `Active` instance becomes a no-op that deploys nothing. "Redeploy to repair" is a real operator move today, and its only escape would become `roymctl app forget` first |
| `SupervisorService::deploy_submission` ([service.rs:309](../../../../crates/app_supervisor/src/service.rs#L309)) | A fresh `journal.append` every call | Would skip correctly — the behaviour the row wants |
| `crates/sdk/src/deploy.rs` unit tests (8 sites) | Ids the test controls | Several assert on redeploy counts; would need rewriting |
| `multi_substrate_placement_e2e.rs` (5 sites), `binding_push_e2e.rs` (1) | Ids the test controls | `multi_substrate_placement_e2e.rs:772`'s `report_again` exists to assert a second `apply_plan` at the same id skips; unaffected |

**Recommendation: do not touch `apply_plan`.** The row is real but the fix it
names is the wrong one. The loop's work list should come from what is
*actually wrong*, not from journal archaeology: `compute_diff` for
plan-level changes, plus the health report for services that are placed but
not healthy. `apply_plan` is then called with a **filtered plan** containing
only the services that need work, which is a caller-side change with no
effect on `roymctl`. That resolves the row's underlying complaint (the full
hex-inlined Wasm artifact crossing the wire every pass) without a
caller-visible semantic change anywhere else. `ReconcileAction::Remove` gets
an explicit answer instead of being silently dropped: A5c does **not**
undeploy on a plan-level removal, and says so, because undeploying a stateful
service on a manifest edit is destructive and `retire` is deliberately not a
teardown. The loop raises an alert naming the orphan instead. See
**D-A5c-2**, **D-A5c-3**.

### 19.3 (Scope-changing) The written epoch has no home, and the first push would report a conflict against the supervisor itself

An exit criterion is per-dependent binding convergence, and §14 step 4 pushes
bindings at "the instance's generation". Generation and epoch are different
numbers and the epoch has no owner.

- `map_deployment_plan_to_wit` hardcodes `epoch: 0` for every binding it
  emits ([mapper.rs:317-318](../../../../crates/sdk/src/mapper.rs#L317)),
  with the comment "A2 mints no epochs; the supervisor does (A5)".
- `install_app_context` writes the persisted per-dependent binding row
  **unguarded** at whatever epoch arrived
  ([orchestration.rs:466-486](../../../../crates/control_plane/src/service/orchestration.rs#L466)) —
  the four-case guard is on `write-bindings` only.
- So after the supervisor's own initial deploy, every dependent's persisted
  row sits at **epoch 0**.
- `classify_binding_write` at an equal epoch with different members returns
  `Conflict`, not `Applied`
  ([resolver.rs:217-226](../../../../crates/app_orchestration/src/resolver.rs#L217)).

**Consequence:** the supervisor's very first `write-bindings` after a
membership change, sent at epoch 0 because nothing minted anything else,
comes back `Conflict(0)` — and §14 step 4 turns a `Conflict` into an alert.
The supervisor would raise `BindingConflict` against its own single-writer
deploy, on the first push it ever performs. There is no second writer.

**Fix — revised after review (F2, F3), because the first version of this
paragraph did not survive its own second redeploy.**

The first version said: `ApplyRequest` gains a scalar `epoch: u64` beside
`generation`, and the store table is keyed per *dependency*
`(app_instance_id, dependent_logical_ref, dependency_name)`. Those two do not
fit together. `ApplyRequest`
([deploy.rs:106-121](../../../../crates/sdk/src/deploy.rs#L106)) and
`map_deployment_plan_to_wit`
([mapper.rs:129-136](../../../../crates/sdk/src/mapper.rs#L129)) both take one
number for a whole apply, the way `generation` does. So once pushes advance
one dependency past another, a later redeploy writes the **same** number to
every binding — and because `install_app_context`'s `save_binding` is
unguarded, the lower number is silently persisted over the higher one. The
next push at `written + 1` then lands at an epoch the substrate has already
served: equal epoch, different content, `Conflict`. The failure this finding
exists to prevent, one redeploy later.

**The epoch is per dependent *service*, not per dependency.** One counter per
`(app_instance_id, dependent_logical_ref)`, incremented on any write that
changes any of that service's bindings, and carried on **every** binding that
write emits. `ApplyRequest` gains `binding_epochs: &BTreeMap<
LogicalServiceRef, u64>` — one entry per service in the apply, since
`apply_plan` deploys several — and `map_deployment_plan_to_wit` looks the
value up per service instead of hardcoding `0`. The persisted substrate-side
row stays keyed per dependency (A5a's, unchanged): all of one dependent's
rows simply share an epoch value, which is what makes per-dependency
*convergence* still readable while keeping the counter something a deploy can
carry.

**A deploy is an authoritative write, and that is what makes the unguarded
`save_binding` safe.** `install_app_context`'s own comment says the four-case
guard "belongs at exactly this call and is the supervisor slice's". A5c
deliberately **does not** put it there. Adding it would break `roymctl app
deploy`, which writes at epoch 0 every time: a second hand-deploy with changed
content would become `Conflict` at an equal epoch. Instead the invariant is
that the supervisor's counter **always advances before a write**, so every
deploy it issues carries a strictly higher number than anything it has itself
written, and an unguarded overwrite is the correct outcome rather than a
regression. That invariant is pinned by a test, not left to the comment.

**What `0` means, and why the operator path keeps it (F3).** `roymctl app
deploy` passes `generation: 0` with an explicit "unmanaged" comment
([app.rs:628-637](../../../../apps/roymctl/src/commands/app.rs#L628)) and
keeps epoch `0` for the same reason. So `0` means **"no supervisor has
written here"**, not "nothing has ever been written". That is the useful
meaning: a supervisor adopting a hand-deployed instance starts its counter at
0 and its first write is 1, which is greater than what the operator left, so
the first push applies. And the convergence read needs no special case for
this: an absent row in the supervisor's table reads as `written_epoch: 0`,
the hand-deployed substrate reports `observed: 0`, they are equal, and the
dependent correctly reports **converged** rather than the false negative F3
warned about.

See **D-A5c-4**.

### 19.4 (Scope-changing) `instance-status.bindings` cannot be filled from what `poll_once` returns

A5b disclosed this field as empty and said A5c populates it. The observed
half is not reachable from the sweep as written.

`ServiceStatus` — the substrate's wire answer — carries `binding_epochs:
Vec<(String, u64)>`
([sdk/src/lib.rs:108](../../../../crates/sdk/src/lib.rs#L108)), added by A5a
§6 for exactly this. But `poll_once` folds `ServiceStatus` into
`ServiceHealth` ([health.rs:88-96](../../../../crates/sdk/src/health.rs#L88))
and **drops the field**: `ServiceHealth` has `signal`, the two certificate
timestamps, and nothing else. So the supervisor's own sweep throws away the
one number the exit criterion needs.

**Fix.** `ServiceHealth` gains `binding_epochs: Vec<(String, u64)>`, filled
in **all five** production construction sites in `poll_once` — `health.rs:185`,
`:221`, `:243`, `:274`, `:289` — empty where there is no answer. (An earlier
draft of this paragraph said three; the review counted them. The eight further
sites in that file are test fixtures and follow the same change.)

`BindingConvergence` is then a join: `written_epoch` from §19.3's per-dependent
counter, `observed_epoch` from `ServiceHealth`, `converged = observed ==
Some(written)`. A dependent this supervisor has never written to reports
`written_epoch: 0` — and a hand-deployed one reports `observed: 0` as well,
so it converges correctly rather than reading as a false negative (§19.3's
answer to F3). A dependent the supervisor *has* written to but that has not
answered reports `observed: None` and `converged: false`, which is the real
unconverged case. See **D-A5c-5**.

### 19.5 (Scope-changing) The supervisor cannot reach the MQTT broker, and nothing can subscribe to it

D-A5-13 decides the topic. Four things underneath it are undecided or do not
work.

**(a) No handle.** `SharedNodeHandles`
(`crates/substrate/src/runtime.rs`) carries `key_store`,
`storage_provider`, `native_dispatch`, and `client_identity`. It does not
carry `messaging_broker`, which is built at `runtime.rs:705` and handed to
`AppSandboxEngine` and `ControlPlaneService`. `init_supervisor`
(`runtime.rs:595`) therefore has nothing to publish through, and
`SupervisorService::new` has no parameter for one.

**(b) No subscriber path.** `messaging` is registered in `EndpointRegistry`
**per deployed service only** — `NATIVE_CAPABILITY_INTERFACES`, looped over
at [orchestration.rs:1452](../../../../crates/control_plane/src/service/orchestration.rs#L1452),
inside `deploy`. The substrate registers exactly three interfaces under its
**own** DID: `orchestrator` (`runtime.rs:498`), `security` (`:503`), and
`supervisor` (`:514`). A supervisor role is not a deployed service, so
`SyneroymClient::subscribe("messaging", topic)` against the supervisor node
resolves no pipeline and fails. D-A5-13's documented operator flow — "publish
under the supervisor's own service id and document the `subscribe` call" —
does not currently work end to end.

**(c) The two namespacing functions are not symmetric — and A5c makes it
three behaviours, deliberately.**
`namespace_topic_for_publish(service_id, topic)` always prefixes;
`namespace_topic(service_id, topic)` (the subscribe side, used at
[dispatch.rs:405](../../../../crates/router/src/route_handler/dispatch.rs#L405))
passes a literal `svc/` prefix through unchanged
(`crates/mqtt_broker/src/lib.rs:70-82`). The published and subscribed strings
must be identical for a message to arrive. That is a test, not an argument.

**(e)'s containment fix adds a third behaviour**: the supervisor's own
subscribe path prefixes unconditionally, so that one endpoint **does not
honour the cross-service opt-in every other `messaging` endpoint honours**.
That divergence is the whole point of the fix, and it is exactly the kind of
thing a later reader corrects back to `namespace_topic` on the grounds that
every other subscribe uses it — silently reopening (e)'s reach. Two guards,
because either alone is weak:

- **A comment at both sites**, saying the divergence is intended and naming
  what it prevents: at the `messaging` registration in `runtime.rs` (which
  creates the endpoint) and at the branch itself (which is where the "fix"
  would be typed).
- **Test 26**, the negative case, which fails the moment the branch reverts.
  The comment explains why; the test is what actually holds.

**One layering cost to pick deliberately, not discover.** The obvious
implementation branches inside `handle_messaging_subscribe` on
`service_id == SUPERVISOR_DISPATCH_ID`, which puts a supervisor constant in
the router. The alternative is carrying the rule on the endpoint itself,
which needs a `SubstrateEndpoint` change for one case. Take the branch, and
say in its comment that the constant is there because the router has no other
way to tell a node-owned endpoint from a deployed service's. A third option —
a long-lived `subscribe-alerts` on the `supervisor` interface, which would
inherit the `substrate/admin` gate and close (e) outright rather than
narrowing it — needs a new `LongLivedStreamMethod` arm and its own capability
check, and is larger than the containment this slice needs. Recorded here so
the post-M5 row has a known shape rather than only a complaint.

**(d) Nothing is retained.** `MqttBroker::publish` is `try_publish` with no
retain flag; the retained variant is `#[cfg(test)]` only
(`mqtt_broker/src/lib.rs:149`, `:164`). An operator who subscribes *after* an
alert fires receives nothing. That is fine — `AlertStore` is the durable
record and the `alerts` verb is the read surface — but the developer guide
must say it, or an operator will treat the topic as a queue.

**(e) An authorization asymmetry worth deciding, not discovering — and the
reach is wider than this paragraph first said (F10).** Every verb on the
`supervisor` interface requires `substrate/admin` (`service.rs:101-118`).
`messaging/subscribe`'s only gate is that the caller is not anonymous
([dispatch.rs:350](../../../../crates/router/src/route_handler/dispatch.rs#L350)).
Publishing alerts to MQTT therefore makes them readable by **any verified
caller** — strictly weaker than the read verb carrying the same data.

The first draft stopped there, and understated it. `namespace_topic`, the
subscribe side, passes a literal `svc/` prefix through **unchanged** as a
deliberate cross-service opt-in
([mqtt_broker/src/lib.rs:70-72](../../../../crates/mqtt_broker/src/lib.rs#L70)).
So registering `messaging` under the node's own DID does not hand a caller the
alert topic — it hands them a subscribe handle for **any `svc/<id>/#` topic on
that node**. On a node already hosting deployed services that reach exists
anyway through those services' own `messaging` endpoints. On a
**supervisor-only node, which hosts none, it is new reach that A5c would
introduce** — on the one node in the fleet that holds member master keys and
manages other people's apps.

**Fix, narrow and local.** On the supervisor's `messaging` pipeline only,
subscribe namespaces with the **publish-side** rule
(`namespace_topic_for_publish`, which prefixes unconditionally) instead of
`namespace_topic`. A caller can then only ever subscribe within
`svc/supervisor/`, so the registration adds exactly the alert topic and
nothing else. The remaining asymmetry — any verified caller may read *this
supervisor's* alerts — stays a backlog row, now correctly scoped. This does
not touch the shared router path for deployed services, whose cross-service
opt-in is deliberate and unrelated. See **D-A5c-6**.

**(f) Where publication goes, and what a failure does.** D-A5-13 already
places it in `record_report`'s caller rather than inside it. That is right
and the reason is now concrete: `record_report` returns
`Vec<(AlertKind, String)>` of the incidents this pass **newly opened**
([health.rs:307-312](../../../../crates/sdk/src/health.rs#L307)), and it
returns them *after* the store write has committed. So publishing from the
caller cannot lose an alert by construction: the row exists before the
publish is attempted. A publish failure is logged and the pass continues; it
is never propagated with `?`. See **D-A5c-6**.

### 19.6 (Confirmed, with a correction) The new `AlertKind`s need no schema change — but §14 miscounts them, and one enum has two halves

§14 step 7 claims "Same `AlertStore` table, no schema change." **Confirmed.**
`alerts.kind` is `TEXT NOT NULL` and the active-row uniqueness index is
`(instance_id, IFNULL(logical_ref,''), substrate_did, kind)`
(`crates/app_orchestration/src/alerts.rs`) — both take an arbitrary new
string with no DDL change.

Two corrections:

- **§14's four are the wrong four.** `SupervisorSuperseded` already exists;
  A5b added it (`alerts.rs:49`) to close matrix row 9's supervisor half. A5c
  adds `RemediationExhausted`, `BindingConflict`, `PlacementChangeRefused`,
  and — found in review, §19.21 — **`OrphanedService`**, which D-A5c-3 needs
  and no other kind can carry. Four, but not §14's four.
- **`AlertKind` has a `Display` and a `FromStr`** (`alerts.rs:52`, `:66`).
  `FromStr` is how stored rows are read back. A variant added to one and not
  the other makes its own rows unreadable at the next `alerts` call, with no
  compile error. A round-trip test over every variant is the guard, and it
  does not exist yet.

### 19.7 (Scope-changing) The store's lock is not enough, and everything in A5b assumed a single caller

Every A5b path runs inside an RPC on `&self`. A resident loop is a second
caller against the same `SupervisorStore` (`Arc<Mutex<Connection>>`), the
same `MasterVault`, and the same managed substrates. The lock is held **per
statement**, never across a read-then-write, so it protects individual rows
and nothing above them.

Concrete races:

| Race | What happens |
|---|---|
| `submit` vs. a loop pass, same instance | `handle_submit` reads state (`service.rs:411`), deploys the **new** plan, then writes it (`:444`). A pass starting between the read and the write reads the **old** plan from the store and deploys that. Two `apply_plan` runs against the same substrates, interleaved, with different plans. Both present the same generation from the same supervisor DID, so `check_generation` accepts both (`Ordering::Equal`, matching `supervisor_did`) — the substrate offers no protection here |
| `retire`/`release` vs. a loop pass | `release_on_every_substrate` clears the stamp while a pass is mid-`apply_plan`. The pass's remaining deploys land against a released instance and re-establish a stamp `retire` just cleared |
| `adopt` vs. a loop pass | `claim_next_generation` writes `held + 1` to every substrate, then `set_generation` locally (`service.rs:534`). A pass reading between them uses the **old** generation and is refused by `check_generation` at every substrate it touches |
| Two loop passes | Cannot happen with one `tokio::interval` and a sequential body — but only as long as a pass is guaranteed to finish before the next tick. A pass that outruns `poll_interval_secs` (30s default) against slow substrates makes it possible. `tokio::interval`'s default `MissedTickBehavior::Burst` then fires immediately |
| `MasterVault::get_or_mint` | **Safe.** Its own `mint_lock` (`keys.rs:135`) already serializes read-then-write |

**Fix.** A per-app-instance async mutex — `DashMap<String,
Arc<tokio::Mutex<()>>>` on `SupervisorService` — acquired for the whole
duration of a loop pass and for the whole duration of `submit`,
`force-reconcile`, `adopt`, `release`, and `retire`. Not `pause`/`resume`
(single-column flag writes) and not `status`/`alerts` (reads, and blocking an
operator's read behind a slow pass is worse than a slightly stale answer).
Per-instance rather than global so one unreachable substrate cannot stall
every other instance's loop. Also set
`MissedTickBehavior::Skip` so a long pass drops the tick it overran instead
of queueing a burst. See **D-A5c-7**.

### 19.8 (Understated) The loop is not spawned, so it is dropped mid-pass at shutdown

D-A5-8's `Send` fix was justified as "unblocking `tokio::spawn` for A5c's
loop." That is not how the loop is wired. `run_supervisor_role(&self.
supervisor)` is `pin!`ed and raced inside `RuntimeServices::run`'s
`tokio::select!` (`runtime.rs:324`, `:335`). It is a borrowed future in a
select, not a spawned task, so it needs no `'static`.

The fix is still worth having — the loop holds the store across `.await`
points and `RuntimeServices::run`'s own future must stay `Send` — but the
sentence in D-A5-8 describes a mechanism A5c does not use, and it hides the
real consequence: **when any other arm of that `select!` completes, the loop
future is dropped wherever it happens to be.** If that is mid-pass, every
`SyneroymClient` the pass opened is dropped-not-closed. `Drop for
SyneroymClient` (added in A5b review round 2) backstops it with a background
close, but `shutdown_supervisor_role` runs *after* the `select!` returns, so
those spawned closes race runtime teardown and may never complete.

**Fix — revised after review (F1); the first version did not work.** It said:
the loop takes a `CancellationToken`, and `SupervisorService::shutdown`
cancels it and waits. **Nothing would ever cancel it in time.**
`run_until_shutdown` returns at `runtime.rs:339` and `supervisor_fut` dies
with that stack frame; `shutdown()` is not reached until `runtime.rs:116`, via
`services.shutdown()` → `shutdown_supervisor_role`. By then there is no pass
left to cancel and no task to wait on, so "cancels it and waits" waits on
nothing and the close still goes through `Drop` — the outcome this finding
says A5c must not rely on. Phase 5's test for it would have exercised a
mechanism that never runs.

**The loop is spawned**, so it outlives the `select!` scope:
`RuntimeServices` holds a `JoinHandle`, `run_until_shutdown` races
`&mut handle` in the same `select!` arm the pinned future occupies today (so a
supervisor that exits still brings the substrate down, unchanged), and
`shutdown` cancels the token and **awaits the handle**. The spawn happens at
the top of `run_until_shutdown`, not in `init` — A5b's startup-ordering note
is emphatic about not perturbing when the two composition calls run, and this
keeps the loop's start after both.

That also corrects this finding's own aside about **D-A5-8**. The first draft
said the `Send` fix "describes a mechanism A5c does not use." With the loop
spawned, `tokio::spawn` **is** the mechanism, `Send + 'static` is required,
and D-A5-8's stated justification was right after all. See **D-A5c-8**.

### 19.9 (Understated) `handle_status` connects to every substrate twice per call, and the loop makes that a per-interval cost

`handle_status` builds one client per placed alias for the health sweep
(`service.rs:856`), runs `poll_once`, closes them all (`:879`) — and then
calls `max_held_generation`, which calls `connected_client` **again** for the
same aliases (`:232`) and closes them again.

In A5b that is two connect/close cycles per substrate per operator command:
unnoticeable. On a 30-second timer it is two `wait_for_ready` handshakes per
substrate per interval, forever, where one would do. `MANAGED_SUBSTRATE_
CONNECT_TIMEOUT` is 10s (`service.rs:48`), so on a substrate that is slow but
reachable a single pass can spend 20 seconds of its 30-second budget
connecting.

**Fix.** One client set per pass, shared by the sweep, the generation read,
the reconcile, and the push, closed once at the end. This also has to happen
before §19.13's budget can be measured against anything meaningful. See
**D-A5c-9**.

### 19.10 (Understated) The loop re-certifies every member on every pass

`apply_with_clients` calls `deploy::certify_placed_members` unconditionally
before `apply_plan` (`service.rs:299`). Each call is one
`resolve-instance-identity` RPC per member plus one fresh signature — a
**new instance certificate every time**.

In A5b that runs once per operator `submit`. On a 30-second timer it mints
and installs a new certificate for every member of every managed instance
every 30 seconds. Beyond the cost, it churns exactly the artifact A5d exists
to renew on a considered schedule, and it makes A5d's near-expiry logic
untestable — nothing is ever near expiry.

Resolved together with §19.2: certification is scoped to the services the
filtered plan actually deploys. A pass where nothing needs work certifies
nothing. See **D-A5c-2**.

### 19.11 (Correctness) A partially-deployed app reports `Active`

Matrix row 12 requires a partial deploy to be visible as `Degraded` on the
read surface. It is not.

`handle_status` computes `overall_state` from `report.faults().is_empty()`
(`service.rs:969`). `HealthReport::faults` counts only the three signals
`Signal::is_fault` admits — `SubstrateUnreachable`, `InstanceNotRunning`,
`ProbeFailing` ([health.rs:79-84](../../../../crates/sdk/src/health.rs#L79)).
A service that **never deployed** has no completed placement in the journal,
so `handle_status` pushes an `ExpectedService` with empty `service_id` and
`substrate_did` (`service.rs:830-834`) and the sweep reports `NotDeployed`,
which is deliberately not a fault (D-A4-19: "I cannot tell" must not raise an
alert).

So an app where 2 of 5 services failed to deploy reports **`Active`**, with
no alert. D-A4-19's rule is right for a *poll* — a service that is not there
cannot be probed — but wrong for a *supervisor*, which knows the difference
between "not in the plan" and "in the plan and missing." The supervisor has
the plan; the sweep does not.

**Fix.** `overall_state` gains a source the sweep cannot give it: a service
present in the desired plan with no completed placement is a deploy failure,
not an unknown. `ManagedState::Degraded` when `report.faults()` is non-empty
**or** any planned service has no landed placement. `AlertKind::
InstanceNotRunning` is the wrong kind for it; this is what
`RemediationExhausted`'s sibling case looks like before any remediation has
run, so it reuses the existing `InstanceNotRunning` kind with a detail string
naming the failure, rather than adding a fourth new kind for a state the
operator reads as the same problem. See **D-A5c-10**.

### 19.12 (Ambiguous) `Superseded` is computed and discarded — and it should stay that way

`handle_status` computes `superseded` per call and never persists it
(`service.rs:895-943`). ADR-0021 §4 says a superseded supervisor stops
managing the instance, and with a loop there is finally something to stop.

**Recommendation: no column.** D-A5-6 already settled that the **substrate**
is the durable arbiter of the generation. A local cached copy can only ever
disagree with the authority, and a persisted flag would need a clearing rule
— which is precisely the trap N3 found on `retired`: a flag one path set and
no path cleared, with error messages naming a recovery that did not exist.
`max_held_generation` already returns `Option<u64>`, where `None` means
"could not reach any of them", so the transient case is handled without
persistence. The durable record that matters is the **alert**, and
`AlertStore` already provides it, raised and cleared per pass.

What each path does when superseded:

| Path | Behaviour |
|---|---|
| Loop pass | Raise/refresh `SupervisorSuperseded`. **Skip every write** for that instance this pass: no deploy, no push, no restart. Continue polling health — reads are safe and the operator still wants status |
| Loop pass, `held_max == None` | Do not skip. Nothing could be reached, so there is nothing to write to anyway, and the writes will fail on their own with real errors instead of a guess |
| `force-reconcile` | Refuse, naming `adopt`. It is a directed write; the substrate would reject it at `check_generation` regardless, and refusing locally produces a message that explains why |
| `submit` | Same refusal, in the same pre-flight as `retired`/`generation`/§19.1. Note the interaction: `submit` already requires the presented generation to equal the stored one, so a superseded supervisor's submit passes the local generation check and only fails at the substrate. The pre-flight is what makes the message right |

See **D-A5c-11**.

### 19.13 (Coverage) The health-poll-cost budget: decide what is measured, against what, before writing the loop

§0.25 assigned this budget to A5c. `task.md` states it as "must not be a
meaningful load on a target substrate at the intended inventory size" — no
number, and a budget derived from the first measurement can never fail, which
is the mistake the convergence budget deliberately avoids by setting its 5 s
target a priori.

**What is measured**, per steady-state pass against one managed node:

1. Wall-clock duration of the pass.
2. RPC count per substrate. Target: **one `status` call per substrate**, not
   one per service — `poll_once` already batches (`StatusQuery::status` takes
   `Vec<String>`), and §19.9's double-connect is the thing that breaks this.
3. The managed substrate's own CPU time attributable to serving one pass.

**Against what**, set a priori so it can fail:

- A pass over a **20-service instance on one substrate completes in under
  2 s** and issues **at most 2 RPCs to that substrate** (one `status`, one
  `app-instance-management-of`).
- Serving it costs the target substrate **under 5% of one core averaged over
  the 30-second poll interval**.

**Why the wasm number is the one at risk.** `probe_cached`'s minimum interval
is 5 s and the default `poll_interval_secs` is 30, so **every** sweep misses
the cache and pays a full component instantiation per `rpc`-probed wasm
service. At 20 wasm services that is 20 instantiations every 30 seconds, on
the target node, forever. This is the measurement the backlog row *"A4: a
wasm `rpc` probe costs a component instantiation"* has been waiting for.

**What it decides.** If the budget fails, the fix is a cheaper wasm liveness
signal, not a longer cache — a longer cache just makes the supervisor slower
to notice a fault, which is the one thing it exists to do. It also decides
the other retargeted row, *"A4: `probe_cached` has no single-flight"*: with
one poller issuing one batched `status` per pass, the loop is not concurrent
with itself, so that row becomes real only when an operator's `roymctl app
health` lands inside the loop's own miss window. The measurement says whether
one duplicated instantiation matters.

**Where.** A timed `#[tokio::test]` against one in-process node with a
generated 20-service instance, plus a `bench:` entry in `mise.toml` so it is
repeatable. **Written before the loop**, so `poll_interval_secs`' default is
chosen from the number rather than defended after it. See **D-A5c-12**.

### 19.14 (Understated) The four inert `SupervisorRole` fields, and the two inert types

Each of these has no production reader today. None should be deleted.

| Item | Location | Becomes |
|---|---|---|
| `poll_interval_secs` (default 30) | `crates/core/src/config.rs:566` | The loop's `tokio::interval` period, with `MissedTickBehavior::Skip` (§19.7) |
| `max_restart_attempts` (default 3) | same | The remediation table's ceiling; on exhaustion → terminal `Degraded` + `RemediationExhausted`, matrix row 13 |
| `restart_backoff_secs` (default 30) | same | Minimum wait between two restart attempts for one service, read against `remediation.last_attempt_at`. Note it equals the poll interval by default, so at defaults a service is restarted at most once per pass — deliberate, and worth stating in the config doc comment rather than leaving as a coincidence |
| `alert_topic` (default `"supervisor/alerts"`) | same | D-A5-13's prefix. The published string is `namespace_topic_for_publish(SUPERVISOR_DISPATCH_ID, "<alert_topic>/<app_instance_id>")` (§19.5c) |
| `SupervisorStore::all_active` | `store.rs:185` | The loop's work list. Its `ORDER BY app_instance_id ASC` plus a sequential pass body means one slow instance delays every later one; accepted for A5c (§19.7 keeps the per-instance locks, so the ordering is a latency property, not a correctness one) and revisited if the budget in §19.13 fails at more than one instance |
| `ManagedService.restart_attempts`, hardcoded `0` | `service.rs:959` | Read from the remediation table, per `(app_instance_id, logical_ref)` |
| `ManagedState::Applying`, never produced | `supervisor.wit`, `service.rs:963-973` | See §19.15 |

### 19.15 (Correctness) `ManagedState::Applying` becomes observable the moment a loop exists

`DeploymentState::Applying` is written by `apply_with_clients`
(`service.rs:312`) and overwritten with `Active` or `Degraded` before the
same RPC returns (`:348-358`). In A5b no reader can observe it: the only
writer holds `&self` for the whole window and `status` is a different call.

With a resident loop, an operator's `status` landing mid-pass **can** observe
it, and `Applying` is then the honest answer — "a reconcile is in flight, ask
again." A5c gives it a source: `journal.get_latest(instance).state ==
Applying` maps to `ManagedState::Applying`.

Precedence in `overall_state`, which currently reads
retired → superseded → paused → degraded/active: insert `Applying` **after**
`paused` and **before** the health-derived branch. A paused or superseded
instance's own state is more important than "busy", and a reconcile in flight
is more useful than a health verdict computed from a half-applied plan. See
**D-A5c-13**.

### 19.16 (Ambiguous) `paused` finally has to mean something

A5b's review pushed back on gating `submit`/`force-reconcile` on `paused`,
with the reason recorded: `paused` was spec'd only as "the resident loop
should not touch this automatically", and that loop did not exist. It does
now, so the definition can be completed rather than argued from absence.

**Decision: `paused` gates the loop and nothing else.** `all_active` already
excludes paused instances (`store.rs:190`), so the loop skips them for free —
**with one window the review found (F6)**: `all_active` filters at query time,
and D-A5c-7 leaves `pause` out of the per-instance lock as a single-column
write, so a `pause` arriving *after* the work list was read does not stop the
pass already running against that instance. That is the one moment `paused`
would not mean what D-A5c-14 says it means. Closed cheaply: the loop re-reads
the instance's own desired-state row at the start of each pass's **write**
phase — it needs the current generation from that row anyway — and skips the
writes if `paused` or `retired` flipped since the work list was built. The
WIT doc comment §24 already changes says pause takes effect at the next write
phase, not mid-write.
`submit`, `force-reconcile`, `adopt`, `release`, and `retire` stay allowed
while paused — every one is an operator's directed action, and pausing
automation to do something by hand is the ordinary reason to pause. A5b's
push-back therefore stands; A5c is where it stops being a gap and becomes a
documented rule, in the WIT doc comment on `pause` and in the developer
guide. See **D-A5c-14**.

### 19.17 (Understated) `restart_impl` has no service-owner check, and the loop is about to become its main caller

The standing backlog row explicitly defers this to "before the remediation
loop is built on top of `restart`." That is now.

State of the tree: `deploy_with_context` checks `owner_of(service_id)`
(`orchestration.rs:1029`) *and* the app instance's `owner_did`;
`undeploy_impl` checks `owner_of` (`:1836`); `write_bindings_impl` checks the
app instance's `owner_did` (`:1697`). `restart_impl`
(`orchestration.rs:2045-2097`) checks the `orchestrator/deploy` capability
and the generation, and never `owner_of`.

**Resolve it: add the check.** The row offers "add it, or document why a
restart is inside a service-scoped grant's remit when a redeploy is not." Two
reasons to add it rather than document the omission:

1. **It costs the supervisor nothing.** Both node-wide overrides
   (`has_node_wide_ability`) are already held: §0.28 requires node-wide
   `orchestrator/deploy`, and A5b's own e2e found node-wide
   `orchestrator/status` is needed too. A node-wide grantee skips the owner
   check by the same rule `deploy`/`undeploy` use.
2. **The remaining case is the one the other three refuse.** An app-scoped
   `orchestrator/deploy` grantee restarting a service a *different* caller
   owns. Three write paths call that a takeover; leaving the fourth open is
   not a considered position, it is an omission — which the row itself says.

Shape: mirror `undeploy_impl`'s block, with `ORCHESTRATOR_DEPLOY` as the
node-wide override ability, placed after the capability gate and before the
generation gate. See **D-A5c-15**.

### 19.18 (Scope-changing) A5c has no reachable membership change, so its binding push has no in-slice trigger

§14 step 4 pushes bindings "after a reconcile changes membership."
**Nothing in A5c can change a membership set.**

- The compiler emits exactly one member per `PlannedService`;
  `substitute_and_certify_members` hardcodes index `0` with the comment
  "nothing in today's manifest format can express more than one member"
  (`member_identity.rs:150`, the call itself at `:175`). `replicas` is A5e
  (D-A5-17).
- `mint_and_substitute` resolves one master per `(app_instance_id,
  service_name, 0)` and `get_or_mint` returns the existing one, so a
  re-submit produces the **identical** member DID (`keys.rs:280-293`, and its
  own test asserts exactly this).
- A service *added* by a re-submit cannot be pushed as a new binding:
  `write-bindings` refuses a dependency name with no existing row, by design
  ("a guest's declared dependency set is a deploy-time contract",
  `control-plane.wit:305-309`).
- A service *removed* by a re-submit is not applied at all (§19.2a).

So the push path is real code with no trigger reachable inside A5c's own
scope. Three honest options:

| Option | Assessment |
|---|---|
| Build the push in A5c, exercise it with a fixture that hand-edits a stored plan's `resolved_dependencies` | Keeps §19.3's epoch table and §19.4's convergence read in the slice that needs them for the exit criterion, and gives A5e a tested path to trigger. The test is a fixture, not a scenario — say so |
| Move the push to A5e with `replicas` | Cleaner trigger, but strands the exit criterion "per-dependent binding convergence" outside the slice `task.md` assigns it to, and leaves `instance-status.bindings` empty for two more slices |
| Build only the epoch table + convergence read in A5c, push in A5e | Splits one mechanism across two slices for no gain |

**Recommendation: option 1**, with the limitation written into `task.md`'s
A5c bullet rather than discovered at A5e. See **D-A5c-16**, and §22's note on
what could move.

### 19.19 (Correctness, found in review — F4) "Retry once at the re-read epoch" can only ever produce a conflict

§14 step 4's rule, carried into this pass unexamined: "`Stale` → re-read and
retry once, then alert."

`classify_binding_write` returns `Stale(held.epoch)` — the epoch the substrate
holds — and at an **equal** epoch with different content returns
`Conflict(held.epoch)`
([resolver.rs:217-226](../../../../crates/app_orchestration/src/resolver.rs#L217)).
So retrying **at** the held epoch cannot succeed. It converts a stale
rejection into a conflict alert and calls that the retry path. The originally
named test (`a_stale_outcome_is_retried_once_at_the_re_read_epoch_then_alerts`)
would have pinned the broken behaviour.

**Fix.** The retry is at **`held + 1`**, with the supervisor's own counter for
that dependent advanced to match so its table and the substrate agree
afterward. Two things stated with it, because both are easy to get wrong
later:

- **The "re-read" is unnecessary.** `Stale(held)` already carries the number.
  A second round trip to learn what the error just told you is pure latency.
- **Jumping ahead of a genuinely-ahead writer is not the epoch's problem.**
  If another writer legitimately holds a higher epoch, the arbiter is the
  **generation**, not the epoch — `check_generation` refuses a lower
  generation before any binding is examined. A stale epoch reached at an
  equal generation means one writer's own retry arrived out of order, which is
  exactly what advancing past it is for.

See **D-A5c-19**.

### 19.20 (Correctness, found in review — F5) `remediation.terminal` is the set-but-never-cleared flag §19.12 refuses to create

§14 step 6 gives the remediation table a `terminal` column, "cleared on a
healthy sweep." For the only signal that reaches it, that sweep cannot happen.

A service in terminal `Degraded` is never restarted again — matrix row 13 is
that property, and the test list pins it. Terminal is reached only from
`InstanceNotRunning` (D-A5c-17 keeps `ProbeFailing` out; D-A4-13 keeps
`SubstrateUnreachable` out). A service that is not running and that nothing
will restart cannot become healthy by itself, so **the sweep that would clear
the flag never fires**.

This is the same trap §19.12 cites as its whole reason for giving
`Superseded` no column, and the one N3 found on `retired`: a flag one path
sets, no path clears, and a message naming a recovery that does not exist. The
pass applied that lesson in one place and reintroduced the pattern two
sections later.

**Fix.** `force-reconcile` clears the remediation row for the instance, and
`adopt` clears it too — a new generation is a fresh start by construction.
`force-reconcile` already means "do the work now", which is exactly the
operator intent here, and `status.md`'s A5b note sets the precedent that
`adopt` is the way back in from a terminal flag. The
`RemediationExhausted` alert's detail names `force-reconcile`, the way the
placement-refusal alert names the manual relocation path. An out-of-band
recovery (an operator restarting the container themselves, a container restart
policy) still clears it through the healthy sweep, which stays as a second
path rather than the only one. See **D-A5c-20**.

### 19.21 (Coverage, found in review — F9) D-A5c-3's orphan alert has no kind and no test

§19.6 counts three new `AlertKind`s and its correction of §14's miscount is
right. But **D-A5c-3 raises a fourth alert** — "an alert naming the orphaned
service" when `compute_diff` yields a `ReconcileAction::Remove` the loop
deliberately does not execute — and none of the three covers it. Reusing
`InstanceNotRunning` would misreport it: the service is running, it is just no
longer wanted.

**Fix.** A fourth kind, `OrphanedService`, so §19.6 reads **four** new kinds:
`RemediationExhausted`, `BindingConflict`, `PlacementChangeRefused`,
`OrphanedService`. It joins the `Display`/`FromStr` round-trip test, and
D-A5c-3 gains a test that the alert is actually raised — nothing in the first
test list exercised D-A5c-3 at all. Still no schema change: §19.6's conclusion
holds for any number of new kinds.

---

## §20 — Decisions

| ID | Decision |
|---|---|
| **D-A5c-1** | The supervisor gets its **own** placement-change refusal (§19.1), in `handle_submit`'s pre-flight beside `retired`/`generation`, and in `handle_force_reconcile`. It reads `journal.get_completed_actions_for_instance` + `deploy::current_placement` — the inputs `handle_status` already uses — and never touches `roymctl`'s `--dir`. `check_no_placement_change` stays private to `roymctl`; nothing is shared or moved. Recorded as an **A5b defect fixed in A5c**, not as new scope. |
| **D-A5c-2** | The loop's work list is `Reconciler::compute_diff` **plus** the health report, and `apply_plan` is called with a **filtered plan** containing only the services that need work (§19.2, §19.10). `apply_plan` itself is not changed, so no existing caller's behaviour moves. Certification (`certify_placed_members`) is scoped to the same filtered set, so a pass with nothing to do mints no certificates. |
| **D-A5c-3** | A5c does **not** undeploy on a plan-level removal (`ReconcileAction::Remove`). Undeploying a stateful service because a manifest was edited is destructive, and `retire` is deliberately not a teardown. The loop raises an alert naming the orphaned service and leaves it running. Written into the docs, since the alternative reading is the natural one. |
| **D-A5c-4** | **Revised after review (F2, F3).** The **supervisor owns the binding epoch**, and the counter is **per dependent service**, not per dependency (§19.3): one value per `(app_instance_id, dependent_logical_ref)` in a `binding_epochs` table, carried on every binding a write emits for that service. `ApplyRequest` gains `binding_epochs: &BTreeMap<LogicalServiceRef, u64>` — not a scalar, which cannot express divergence once pushes advance — and `map_deployment_plan_to_wit` looks it up per service. The counter **always advances before a write**, which is the invariant that makes `install_app_context`'s unguarded `save_binding` correct rather than a regression; the four-case guard is deliberately **not** added there, since it would break `roymctl app deploy`'s repeated writes at epoch 0. `0` means "no supervisor has written here", the operator path keeps it, and a hand-deployed instance therefore reads as converged rather than as a false negative. |
| **D-A5c-5** | `ServiceHealth` gains `binding_epochs: Vec<(String, u64)>`, filled at **all five** production `poll_once` construction sites — `health.rs:185`, `:221`, `:243`, `:274`, `:289` (§19.4, count corrected in review). `BindingConvergence` joins it against D-A5c-4's counter: `converged = observed == Some(written)`, with an absent row reading as `0` so the hand-deployed case converges correctly. |
| **D-A5c-6** | MQTT (§19.5): `SharedNodeHandles` gains `messaging_broker`; `runtime.rs` registers `messaging → NativeHostChannel { service_id: SUPERVISOR_DISPATCH_ID }` beside the existing `supervisor` registration; publication happens in `record_report`'s caller over its returned newly-opened list, **after** the store write, so a publish failure cannot lose an alert; failures are logged, never propagated. Messages are **not retained** and the guide says so. **Added after review (F10):** on the supervisor's `messaging` pipeline only, subscribe namespaces with `namespace_topic_for_publish`'s unconditional rule rather than `namespace_topic`'s, so a literal `svc/` prefix cannot escape `svc/supervisor/`. Without it the registration hands every verified caller a subscribe handle for any `svc/<id>/#` topic on the node — new reach on a supervisor-only node, which hosts no services today. **This is a third namespacing behaviour and the divergence is intended** (§19.5c): that one endpoint deliberately does not honour the cross-service opt-in every other `messaging` endpoint does, so both the `runtime.rs` registration site and the branch itself carry a comment saying so, and test 26 fails if it is "corrected" back to `namespace_topic`. |
| **D-A5c-7** | A per-app-instance `tokio::Mutex` (a `DashMap<String, Arc<Mutex<()>>>` on `SupervisorService`) is held for a whole loop pass and for the whole of `submit`, `force-reconcile`, `adopt`, `release`, `retire` (§19.7). Not for `pause`/`resume`/`status`/`alerts`. `tokio::interval` uses `MissedTickBehavior::Skip`. |
| **D-A5c-8** | **Revised after review (F1).** The loop is **spawned**, so it outlives `run_until_shutdown`'s stack frame (§19.8): `RuntimeServices` holds the `JoinHandle`, races `&mut handle` in the `select!` arm the pinned future occupies today, and `shutdown` cancels the token and **awaits the handle**. A token alone does not work — `shutdown()` is not reached until after the `select!` has already dropped the pass, so it would cancel nothing and the close would still go through `Drop`. Spawning happens at the top of `run_until_shutdown`, not in `init`, to leave A5b's startup ordering untouched. This also restores **D-A5-8**'s original justification: `tokio::spawn` is the mechanism after all, so `Send + 'static` is genuinely required. `Drop for SyneroymClient` stays a backstop and is never the close path A5c relies on. |
| **D-A5c-9** | One client set per pass, shared by the health sweep, the generation read, the reconcile, and the push, closed once (§19.9). `handle_status`'s existing double-connect is fixed in the same change, since the loop and `status` share the pass body. |
| **D-A5c-10** | `overall_state` is `Degraded` when `report.faults()` is non-empty **or** any planned service has no landed placement (§19.11). `Signal::is_fault`'s definition is **not** changed — D-A4-19's rule is right for a poll; the supervisor adds plan knowledge the poll does not have. |
| **D-A5c-11** | `Superseded` gets **no column** (§19.12). It is recomputed per pass against the substrate, which D-A5-6 already made the durable arbiter. The alert is the durable record. A superseded instance is skipped for writes but still polled for health; `submit` and `force-reconcile` refuse and name `adopt`. |
| **D-A5c-12** | The health-poll-cost budget is set **a priori** (§19.13): a 20-service pass under 2 s, at most 2 RPCs per substrate per pass, under 5% of one core on the target averaged over the interval. Measured **before** the loop is written, and `poll_interval_secs`' default is chosen from the result. Resolves or retargets both A4 poller rows with a number. |
| **D-A5c-13** | `ManagedState::Applying` is sourced from `journal.get_latest(instance).state == Applying` (§19.15), ranked after `paused` and before the health-derived branch. |
| **D-A5c-14** | `paused` means "the resident loop does not touch this", and nothing else (§19.16). Every operator verb stays allowed while paused. A5b's push-back stands; A5c writes the rule into the WIT doc comment and the guide. **Added after review (F6):** the loop re-reads the instance's desired-state row at the start of each pass's write phase — it needs the current generation from it anyway — so a `pause` landing mid-pass stops the writes, and the doc comment says pause takes effect at the next write phase rather than mid-write. |
| **D-A5c-15** | `restart_impl` gains an `owner_of` check with a node-wide `ORCHESTRATOR_DEPLOY` override, matching `undeploy_impl` (§19.17). Closes the backlog row that was deferred to exactly this point. |
| **D-A5c-16** | The binding push ships in A5c with its epoch bookkeeping and convergence read, exercised by a **fixture** that hand-edits a stored plan's `resolved_dependencies` (§19.18). A5c has no reachable membership change of its own; the real trigger is A5e's `replicas`. `task.md`'s A5c bullet is corrected to say so. |
| **D-A5c-17** | `ProbeFailing` is **alert only** in A5c. See §21. |
| **D-A5c-18** | A re-submit with changed placement is a **permanent refusal** the loop never retries. The reviewer's answer is confirmed, with one correction: the refusal does not exist yet and must be built (D-A5c-1). See §21. |
| **D-A5c-19** | *(review, F4)* A `Stale(held)` outcome is retried at **`held + 1`**, not at `held` — retrying at the held epoch with changed content is a `Conflict` by the four-case rule, so the original wording described a path that could never succeed (§19.19). The supervisor's own counter advances to match. There is **no re-read**: `Stale` already carries the number. A writer genuinely ahead is arbitrated by the **generation**, not the epoch. |
| **D-A5c-20** | *(review, F5)* `remediation.terminal` is cleared by **`force-reconcile`** and by **`adopt`**, not only by a healthy sweep (§19.20) — a terminal `InstanceNotRunning` service is never restarted, so it cannot become healthy on its own and the sweep that would clear it never fires. The `RemediationExhausted` alert detail names `force-reconcile`. This applies §19.12's own rule (no flag without a clearing path) to the table §14 introduced. |
| **D-A5c-21** | *(review, F9)* A **fourth** new `AlertKind`, `OrphanedService`, carries D-A5c-3's alert for a service dropped from the plan but still running (§19.21). Reusing `InstanceNotRunning` would misreport it — the service *is* running. Still no schema change. |

---

## §21 — The two open questions, answered

### §18 question 8 — does `ProbeFailing` trigger a bounded restart, or alert only?

**Answer: alert only, in A5c.**

**What "running but not ready" means for the service types this tree actually
deploys.** `HealthCheck` has three variants
(`crates/app_orchestration/src/models.rs:271-282`), and `restart_impl` has
four type branches (`orchestration.rs:2077-2096`):

| Service type | Probe available | What `restart` does | Is a restart plausible remediation? |
|---|---|---|---|
| `container` | `TcpConnect`, `HttpGet` | `podman stop` then `podman start` | **Yes.** A wedged event loop, a leaked descriptor, a deadlocked process — the classic restart-fixable case |
| `wasm` | `Rpc` (invoke a method; any non-error return passes) | `reload_wasm`: evict the cached component, recompile from `blobs_dir/<id>.wasm` | **Rarely.** A guest is instantiated per call; a failing `rpc` probe means the component traps or the method errors, and recompiling the identical bytes reproduces the identical component. It helps only for engine-level state (a poisoned cache entry, allocator pressure) |
| `tcp` | `TcpConnect`, `HttpGet` | **Refused by construction** — the process runs outside this substrate | **No.** Already a backlog row |
| `nativehost` | none in practice | **Refused** | **No** |

So restart-on-`ProbeFailing` is meaningful for exactly one of four types.

**Three reasons that settle it, in order of weight.**

1. **There is no way to tell "still starting" from "broken", and the
   declaration has no field for it.** `TcpProbe` carries `interface` and
   `timeout_ms`; `HttpProbe` adds `path` and `expect_status`; `RpcProbe`
   names a method. **No initial delay, and no failure threshold.** A service
   that legitimately takes 40 s to warm up reports `ProbeFailing` on the
   first sweep after deploy. A restart-on-`ProbeFailing` policy restarts it,
   then restarts it again on the next sweep, and never lets it start —
   burning `max_restart_attempts` and landing in terminal `Degraded` on a
   service that was never broken. This is not a tuning problem; the field
   that would fix it does not exist.

2. **The two signals are separated at the source precisely because their
   authority differs.** `InstanceNotRunning` is a substrate-verified fact:
   the container is not up, the component is not loaded. `restart` is the
   exact inverse operation, and it is the substrate's own truth on both
   sides. `ProbeFailing` is an **author-declared assertion** whose meaning
   the supervisor does not know — the author chose the path, the expected
   status, and the timeout. Restarting on it converts a readiness assertion
   into a lifecycle action the author never asked for.

3. **The most common cause is a fault in a different service.** A
   `frontend`'s `/healthz` that checks its `backend` fails while `backend` is
   down. Restarting `frontend` repeatedly cannot fix that, and the instance
   is *already* `Degraded` and *already* alerting on `backend` — the restarts
   add no information and spend the remediation budget on the wrong service.

**What A5c does instead.** `ProbeFailing` raises and refreshes
`AlertKind::ProbeFailing` (which already exists, A4) and contributes to
`Degraded`. `InstanceNotRunning` gets the bounded restart. That keeps §14's
table intact except for the one undecided row.

**What would make restart-on-`ProbeFailing` safe later**, so this is revisited
on evidence rather than by mood:

- `HealthCheck` gains `initial_delay_secs` and `failure_threshold` (N
  consecutive failing sweeps), which is a manifest-format change.
- Remediation is gated on service type, so the `wasm`/`tcp`/`nativehost`
  cases are never attempted.

Filed as a backlog row targeted **post-M5**, not at A5e — A5e's scope is
scale-out and cross-app, and a manifest-format change for probe semantics
belongs with whatever revisits health declaration, not bolted onto it.

### §18 question 9 — is a re-submit with changed placement a permanent refusal?

**Answer: yes — the reviewer's answer is confirmed. With one correction that
changes what A5c has to build.**

**Confirming the reviewer's reasoning.** Relocation needs an `undeploy` on
the old substrate plus an ordering rule between that undeploy and the new
deploy, and both halves are best-effort synchronous today. If the undeploy
fails or the supervisor dies between the two, the result is the two-live-copy
state — the exact failure being avoided. Making it safe needs durable,
retried, ordered delivery, which is A6's outbox/DLQ work, gated on M5 item 1.
The question correctly notes that `task.md`'s non-goal text is about
*remediation* and a deliberate operator re-placement is a different act; the
answer is still the same, because the blocker is not the intent, it is the
durability of the two-step. An operator who wants a relocation today has a
working manual path (`svc remove` on the old node, `app forget`, re-submit),
and that path is honest about being two steps.

**The correction: the refusal does not exist, and the milestone currently
does the thing it declares a non-goal.** §14 step 5 assumes the supervisor
inherits `check_no_placement_change`. It does not (§19.1). So today a
re-submit with changed placement is not refused, not retried, and not
alerted — it is **silently applied**, leaving two live copies of one member.
"The loop must not retry it" was the whole of §14's requirement; the actual
requirement is "build the refusal, then do not retry it."

**What A5c builds, precisely:**

1. A pre-flight refusal in `handle_submit` and `handle_force_reconcile`
   (D-A5c-1), so a changed-placement submit fails **before** any deploy,
   mint, or certification work runs — the ordering B3 and N1 already
   established for `retired` and `generation`.
2. The loop marks that service `Degraded`, raises
   `AlertKind::PlacementChangeRefused`, and **stops attempting that one
   service** while continuing to reconcile the rest of the instance. Without
   this the loop retries a permanent refusal every 30 seconds forever, which
   §14 correctly identified as the hazard.
3. The alert's detail names the manual relocation path, so an operator has
   somewhere to go.

Both retargeted backlog rows stay retargeted, with their reasoning unchanged:
*"A relocated-away substrate keeps trying to publish a member's record"* and
*"A3: a redeploy that moves a service is refused, not relocated"* both remain
**post-M5**, with relocation.

---

## §22 — Phase plan and merge order

Each phase is independently reviewable. Phases 1-3 have no loop in them,
which keeps the largest behaviour change last.

1. **Substrate-side and shared-crate fixes, no supervisor involved.**
   §19.17's `owner_of` check on `restart_impl`; §19.4's `binding_epochs` on
   `ServiceHealth` (five construction sites); §19.3's per-service
   `binding_epochs` map on `ApplyRequest` and the removal of `mapper.rs`'s
   hardcoded `0`. Mergeable alone; every one is a small, testable change to an
   existing path. Tests 1-5.
2. **The A5b defects A5c inherits.** §19.1's placement refusal (submit +
   force-reconcile); §19.11's `Degraded`-on-missing-placement; §19.15's
   `Applying`; §19.9's single client set. All of these are correct with or
   without a loop, and shipping them first means the loop is built on a
   `status` that already tells the truth. Tests 6-13.
3. **Bookkeeping and the budget.** §19.3's per-dependent `binding_epochs`
   table; §14 step 6's `remediation` table **with its clearing paths**
   (§19.20); §19.6's four new `AlertKind`s with the `Display`/`FromStr` round
   trip; §19.13's budget harness. **The budget is measured here**, before the
   loop exists, against a hand-driven sweep — which is what makes the number a
   target rather than a description. Tests 14-22.
4. **MQTT.** §19.5's `SharedNodeHandles` field, the `messaging` registration,
   the unconditional-prefix rule on the supervisor's subscribe path (§19.5e),
   the publish call in `record_report`'s caller, and the publish/subscribe
   symmetry test. Independent of the loop: A5b's on-demand `status` already
   calls `record_report`, so this is observable the moment it lands. Tests
   23-27.
5. **The loop.** `AppSupervisor::run` **spawned**, with the interval, the
   cancellation token joined at shutdown (§19.8), the per-instance locks, the
   filtered-plan reconcile, and the `Superseded`/`paused` skip rules. Tests
   28-34.
6. **Remediation.** The `InstanceNotRunning` branch, backoff, attempt
   ceiling, terminal `Degraded`, `RemediationExhausted`, and D-A5c-3's
   `OrphanedService`. `ManagedService.restart_attempts` stops being `0`.
   Tests 35-41.
7. **The binding push**, its epoch advance, and the convergence read —
   including test 45, the exit criterion's own test. Last, because §19.18
   means its trigger is a fixture rather than a scenario. Tests 42-48.

**What could move between sub-slices**, stated the way A5b's pass moved
master custody:

- **Nothing needs to move *into* A5c from a later slice.** The one candidate
  — `replicas`, which would give the push a real trigger — is correctly A5e
  (D-A5-17) and is the largest single item in the milestone. Pulling it into
  A5c would make A5c the biggest sub-slice by a wide margin.
- **The binding push could move *out* to A5e** (§19.18, option 2), and this
  is the only genuine judgement call in this pass. Recommendation is to keep
  it, because the exit criterion "an operator can read per-dependent binding
  convergence" is A5b's stated debt and leaving it empty for two more slices
  makes it a milestone-end scramble. But if A5c runs long, this is the piece
  to move, and phase 7 is ordered last so that stays possible.

  **The review argued the opposite** — that F2 and F8 make moving the push
  look stronger than this paragraph grades it. Considered and **not taken**,
  because both findings, once resolved, cut the other way:

  - **F2 made the push smaller, not larger.** Its fix is a per-service
    counter and a rule that a deploy always advances it. That rule is needed
    for the **deploy** path whether or not a push exists, since
    `install_app_context` writes binding rows unguarded on every deploy. So
    moving the push would leave the epoch design in A5c anyway — and leave it
    with no consumer, which is how designs rot.
  - **F8 removed the expensive part.** The scramble it correctly predicted
    was a two-node push e2e with no honest trigger. There now is no such
    test: the wire path is already proven live by A5a, and the supervisor's
    own decisions are unit-tested against a fake actor. What remains in
    phase 7 is bookkeeping and a join.
  - **F7 is the decisive one.** The exit criterion is phrased as a *read
    surface*, and test 45 reads it. Without the push there is nothing to
    read, so moving the push moves the exit criterion — which is precisely
    the outcome the first paragraph above exists to avoid.

  Kept, therefore, with more support than before rather than less. Phase 7
  stays last regardless.
- **§19.1's placement refusal arguably belongs to A5b**, since it is A5b's
  defect. It is done in A5c because A5b is merged and there is no A5b to
  reopen; `status.md` records the ownership.

---

## §23 — A5c tests

Named the way §8 and §13 named theirs. **e2e are marked; everything else is a
unit test.** New e2e port blocks start at **12_200** — 8800-12_100 are taken
(`multi_substrate_placement_e2e.rs`, `health_monitoring_e2e.rs`,
`binding_push_e2e.rs`, `supervisor_interface_e2e.rs`), see the comment at the
top of `crates/substrate/tests/supervisor_interface_e2e.rs`.

**Phase 1 —** `crates/control_plane/src/service/orchestration.rs`,
`crates/sdk/src/health.rs`, `crates/sdk/src/mapper.rs`:

1. `restart_is_refused_for_a_service_owned_by_another_caller` (§19.17)
2. `restart_by_a_node_wide_deploy_grantee_ignores_the_service_owner` — the
   boundary that proves the supervisor is unaffected
3. `poll_once_carries_each_services_binding_epochs_through_to_service_health`
   (§19.4)
4. `poll_once_reports_empty_binding_epochs_for_an_unreachable_substrate` —
   the arm that would otherwise panic or fabricate
5. `a_plan_mapped_at_a_nonzero_epoch_emits_that_epoch_on_every_binding`
   (§19.3)

**Phase 2 —** `crates/app_supervisor/src/service.rs`:

6. `submit_is_refused_when_the_plan_moves_a_service_to_another_substrate`
   (§19.1) — asserts the error names both substrates
7. `submit_with_a_changed_placement_is_refused_before_any_deploy_work_runs` —
   same fixture trick B3 and N1 use: an inventory with no credential, so a
   "placement" error rather than a "credential" one proves the ordering
8. `force_reconcile_is_refused_when_the_stored_plan_moves_a_service` (N3's
   lesson: `force-reconcile` never calls `store.submit`, so it needs its own
   check)
9. `submit_is_allowed_when_a_service_keeps_its_substrate` — the boundary, so
   the refusal cannot be satisfied by refusing everything
10. `an_instance_with_a_planned_service_that_never_landed_reports_degraded`
    (§19.11, **matrix row 12**) — today reports `Active`
11. `a_fully_landed_healthy_instance_still_reports_active` — row 12's
    boundary
12. `status_reports_applying_while_a_reconcile_is_in_flight` (§19.15)
13. `status_connects_to_each_substrate_once` (§19.9) — counts connects
    through a fake `StatusQuery`; guards the double-connect regression

**Phase 3 —** `crates/app_supervisor/src/store.rs`,
`crates/app_orchestration/src/alerts.rs`, the budget harness:

14. `a_written_epoch_is_held_per_dependent_and_shared_by_its_dependencies`
    (§19.3 as revised — the counter is per service, not per dependency)
15. `a_redeploy_after_a_push_carries_an_epoch_above_what_was_pushed` (F2's
    failure: the scalar version wrote a lower epoch over a higher one, and
    the next push then conflicted)
16. `an_absent_row_reads_as_epoch_zero_so_a_hand_deployed_binding_converges`
    (F3 — the false negative the operator path would otherwise produce)
17. `remediation_attempts_survive_a_store_reopen` (§14 step 6's durability
    claim)
18. `a_healthy_sweep_clears_the_remediation_row`
19. `force_reconcile_clears_a_terminal_remediation_row` (§19.20 / F5 — the
    path that makes terminal escapable at all)
20. `adopt_clears_a_terminal_remediation_row` (§19.20's second clearing path)
21. `every_alert_kind_round_trips_through_display_and_from_str` (§19.6) —
    walks every variant, including `OrphanedService`; the guard against a
    variant added to one half only
22. `a_steady_state_sweep_of_twenty_services_stays_within_the_poll_budget`
    (§19.13, **D-A5c-12**) — asserts pass duration and per-substrate RPC
    count against the a-priori numbers

**Phase 4 — MQTT,** `crates/app_supervisor/`, `crates/substrate/`:

23. `a_newly_opened_alert_is_published_under_the_supervisors_own_topic`
24. `a_publish_failure_leaves_the_alert_stored_and_the_pass_running` (§19.5f)
25. `an_already_open_alert_is_not_republished_on_the_next_sweep` — the
    property `record_report`'s newly-opened return value provides
26. `a_subscribe_naming_another_services_topic_stays_inside_the_supervisors_namespace`
    (§19.5e / **F10**) — the negative test for the unconditional-prefix rule.
    Without it the registration widens reach to any `svc/<id>/#` on the node.
    **This is also the regression guard for §19.5c's deliberate divergence**:
    the supervisor's subscribe path is the one `messaging` endpoint that does
    not honour the cross-service opt-in, so it reads like a copy-paste slip
    against every other subscribe site. This test fails the moment someone
    "corrects" it back to `namespace_topic`; the comments at the two sites
    explain why, and this is what enforces it
27. **e2e** `an_operator_subscribed_to_the_alert_topic_receives_an_opened_alert`
    (`crates/substrate/tests/supervisor_alerts_e2e.rs`, ports **12_200**) —
    proves §19.5b's `messaging` registration and §19.5c's publish/subscribe
    string symmetry together, which no unit test can

**Phase 5 — the loop,** `crates/app_supervisor/src/service.rs`:

28. `the_loop_skips_paused_and_retired_instances` (§19.16)
29. `a_pause_landing_mid_pass_stops_that_passs_writes` (§19.16 / **F6**)
30. `the_loop_skips_every_write_for_a_superseded_instance_but_still_polls_it`
    (§19.12)
31. `an_unreachable_generation_read_does_not_mark_an_instance_superseded` —
    `max_held_generation`'s `None` case, on the loop's path this time
32. `a_submit_and_a_loop_pass_for_one_instance_do_not_interleave` (§19.7) —
    both driven concurrently against a counting fake actor
33. `shutdown_cancels_the_spawned_loop_and_waits_for_it_to_close_its_clients`
    (§19.8 as revised / **F1**) — asserts `shutdown` returns only after the
    handle has joined. The earlier name
    (`a_cancelled_pass_closes_every_client_it_opened`) would have passed
    against a token nothing ever cancelled in time
34. `a_pass_that_outruns_the_interval_does_not_queue_a_burst` (§19.7's
    `MissedTickBehavior::Skip`)

**Phase 6 — remediation:**

35. `instance_not_running_triggers_a_restart_on_the_next_pass`
36. `a_restart_is_not_retried_before_the_backoff_elapses`
37. `remediation_stops_after_max_attempts_and_alerts_once`
    (**matrix row 13**)
38. `a_terminal_degraded_service_is_never_restarted_again` (row 13's second
    half — the property the row's wording turns on)
39. `probe_failing_never_triggers_a_restart` (**§21 / D-A5c-17**, pinned as a
    test so a later slice changes the policy deliberately)
40. `substrate_unreachable_never_triggers_a_restart` (D-A4-13, re-pinned
    because A5c is the first slice that could get it wrong)
41. `a_service_dropped_from_the_plan_raises_orphaned_service_and_is_not_undeployed`
    (§19.21 / **F9** — D-A5c-3, which nothing exercised before)

**Phase 7 — the push and convergence:**

42. `a_membership_change_pushes_at_the_next_epoch_and_records_it`
43. `a_conflict_outcome_raises_binding_conflict_and_does_not_retry`
44. `a_stale_outcome_is_retried_once_above_the_held_epoch_then_alerts`
    (§19.19 / **F4** — renamed from `..._at_the_re_read_epoch_...`, which
    named a retry that could only ever produce a conflict)
45. **`status_reports_a_converged_binding_after_a_push_lands`** (§19.4 /
    **F7**) — the join itself, read off `instance-status.bindings` through a
    real `status` call. **This is the exit criterion's test**: everything
    else in this list covers one side of it. `bindings` is hardcoded empty
    today (`service.rs:988-991`), and A5b's row-9 experience is exactly what
    happens when a criterion is credited without a test on the surface it is
    phrased against
46. `a_dependent_that_does_not_answer_reports_unconverged_rather_than_absent`
    — test 45's negative half
47. `a_dependent_unreachable_during_a_push_leaves_the_instance_degraded_and_is_retried_when_it_next_answers`
    (**matrix row 11**) — a fake `SubstrateActor` whose `write_bindings`
    fails on pass 1 and succeeds on pass 2. **A unit test, deliberately** —
    see the note below
48. **e2e** `a_partial_deploy_is_degraded_and_its_failed_service_is_retried_without_rollback`
    (`crates/substrate/tests/supervisor_loop_e2e.rs`, ports **12_400**,
    **matrix row 12**) — two real nodes, one stopped at submit time, so one
    service lands and one fails; the surviving service is not rolled back,
    and the next pass retries only the failed one once the node returns

**Why row 11's test is a unit test, and why there is no supervisor push
e2e** (review **F8** asked how the two row-11/row-12 e2e tests — numbered 40
and 41 in the list as reviewed, now 47 and 48 — would trigger a push over two
real nodes, given §19.18 says A5c has no reachable membership change. A fair
question with no good answer in the original list.) Three facts settle it:

- §19.18's conclusion means **any** e2e push trigger would be artificial —
  reaching into `supervisor.db` on a running node, or adding a test-only
  verb. Both are worse than the thing they would prove.
- The wire path is **already proven live** by A5a's
  `binding_push_e2e.rs::a_membership_change_pushed_to_a_dependent_takes_
  effect_without_a_redeploy`. A5c adds no new substrate behaviour to push
  through; it adds the supervisor's own bookkeeping and decisions.
- Row 11's property — a push fails against an unreachable dependent, the
  instance goes `Degraded`, the next pass retries — is **entirely
  supervisor-side control flow**. A fake actor tests it deterministically;
  a two-node version tests the same branch through a flakier, slower path.

This inverts A5b's own reasoning rather than contradicting it: there, `adopt`
and `retire` were *not* unit-testable in isolation, so they were proven live.
Here the reverse holds, and the honest answer is the one that follows the
testability, not the format. Row 12 keeps its e2e (test 48) because its
trigger — stop one of two nodes — is real and needs no fixture surgery.

**Failure-matrix rows, mapped explicitly** — A5b credited row 9 with no test
and had to add one in review; that is not repeated:

| Row | Test | Kind |
|---|---|---|
| **11** — dependent unreachable during a push | 47 | unit (fake actor; see note above) |
| **12** — partial app deploy (3 of 5) | 10, 11, 48 | unit + e2e |
| **13** — remediation exceeds max attempts | 37, 38 | unit |

**Exit criterion** — "an operator can read health, alerts, and per-dependent
binding convergence": tests 45 and 46, on the read surface itself.

**Fixture note**, carried from §13 and still true: the supervisor's node-wide
`orchestrator/deploy` **and** `orchestrator/status` grants on each managed
node are hand-issued by the fixture. Nothing in this milestone issues them.

---

## §24 — Docs and backlog for A5c

**Docs**

- `docs/developer-guide.md` — the `[roles.supervisor]` block gains its four
  previously-inert fields with what each now controls (§19.14), including the
  note that `restart_backoff_secs` defaulting to the poll interval means one
  restart attempt per pass; the MQTT alert topic, the exact `subscribe` call,
  and **that messages are not retained** (§19.5d); that `paused` stops the
  loop and nothing else (§19.16); that a placement change is refused and the
  manual relocation path is `svc remove` → `app forget` → re-submit (§21);
  and that a plan-level service removal is **not** undeployed (D-A5c-3).
- `task.md` — the A5c bullet corrected for §19.18 (no reachable membership
  change in A5c, so the push's trigger is A5e's), and rows 11/12/13 annotated
  with their tests at sign-off.
- `status.md` — an A5c section in the A0-A5b shape, and the A5b section
  amended to own §19.1 as its own defect.
- ADR-0021 — §3's epoch is now explicitly minted and held by the supervisor
  (D-A5c-4); the ADR says the guard exists but not who owns the number.
  Decide at A5c sign-off.

**Backlog rows resolved**

- *"A3: `Degraded` has no automatic exit"* — the loop's healthy sweep clears
  remediation state and the alert.
- *"A3: `ActionState::Pending` has no writer"* — **re-argued, not resolved.**
  §14 step 2 planned the loop to enqueue diff actions as `Pending` before
  applying them. With D-A5c-2 the loop applies a filtered plan through
  `apply_plan`, which writes `InProgress` directly (`deploy.rs:242`), so
  there is still no `Pending` writer. Adding one purely to have one is
  ceremony. **Recommendation: delete the `Pending` variant** rather than
  invent a writer for it — or keep the row open with an honest target. Either
  is fine; pick one at A5c sign-off rather than leaving a third slice to
  rediscover it.
- *"A4: alerts are not published to MQTT"* — D-A5c-6.
- *"`restart` is the only lifecycle write with no service-owner check"* —
  D-A5c-15.
- *"The supervisor's `submit`/`force-reconcile` never skip an already-landed
  service"* — resolved by D-A5c-2's filtered plan, **not** by re-scoping
  `apply_plan`. The row's own named fix is rejected with the caller audit in
  §19.2 as the reason.
- *"A4: `probe_cached` has no single-flight"* and *"A4: a wasm `rpc` probe
  costs a component instantiation"* — resolved or retargeted **with a
  number**, by D-A5c-12.

**Backlog rows to add**

- ***A `ProbeFailing` service is never restarted, and `HealthCheck` has no
  initial delay or failure threshold*** (§21) — the two fields that would
  make restart-on-probe safe do not exist in `TcpProbe`/`HttpProbe`/
  `RpcProbe`, and remediation would also need per-service-type gating since
  `restart` is refused outright for `tcp` and `nativehost`. → **post-M5**,
  with whatever revisits health declaration.
- ***`ReconcileAction::Remove` has no executor anywhere in the tree***
  (§19.2a) — `compute_diff` produces it and `apply_plan` iterates only ADDs,
  so a service dropped from a plan runs forever and is not polled. A5c
  alerts on it (D-A5c-3) rather than undeploying. → **TBD**, needs a
  destructive-action policy first.
- ***`messaging/subscribe` needs no capability, only a handshake*** (§19.5e,
  widened after review F10) — the original wording, "MQTT alert topics are
  readable by any verified caller", was too narrow. `messaging/subscribe`
  gates only on the caller not being anonymous
  (`route_handler/dispatch.rs:350`), and `namespace_topic` passes a literal
  `svc/` prefix through as a deliberate cross-service opt-in
  (`mqtt_broker/src/lib.rs:70-72`), so **any** `messaging` endpoint on a node
  is a subscribe handle for **any** `svc/<id>/#` topic on it — not only the
  topic the endpoint belongs to. That is pre-existing on nodes hosting
  deployed services; A5c avoids widening it to supervisor-only nodes with a
  local unconditional-prefix rule (D-A5c-6) rather than by leaving the row to
  describe the narrow case. The real problem for post-M5 is the missing
  capability gate, and pairs with the existing row wanting a
  `supervisor/status` ability. → **post-M5**.
- Whatever A5d's and A5e's own `§0` passes find.

---

## §25 — Review response (2026-08-01)

An independent review of Part IV spot-checked roughly forty of its
`file:line` claims and found no false ones; every finding below is about a
**decision** this pass reached, not the code it read. Ten findings: **two
blocking, six should-fix, two minor**. Nine incorporated as raised, one
answered differently. No code exists yet, so nothing was re-run — A5c is
unimplemented and the working tree holds planning documents only.

| # | Finding | Disposition |
|---|---|---|
| **F1** | *(blocking)* D-A5c-8's `CancellationToken` cannot fire before the loop is dropped: `run_until_shutdown` returns at `runtime.rs:339` and kills the pinned future, while `shutdown()` is not reached until `runtime.rs:116` | **Incorporated.** Verified exactly as described. The loop is now **spawned** and `shutdown` joins the handle (§19.8, D-A5c-8). The review is also right that this restores D-A5-8's original `Send` justification, which the first draft had dismissed. Phase 5's test renamed, since the old one pinned a mechanism that never ran |
| **F2** | *(blocking)* A scalar `ApplyRequest.epoch` cannot carry a per-dependency counter, and `install_app_context`'s `save_binding` is unguarded, so a redeploy writes a lower epoch over a higher one and the next push conflicts | **Incorporated, with a design change.** The counter becomes **per dependent service**, `ApplyRequest` takes a map, and a written invariant ("the counter always advances before a write") is what makes the unguarded save correct. The guard is deliberately **not** added at `install_app_context` — it would break `roymctl app deploy`'s repeated writes at epoch 0 (§19.3, D-A5c-4) |
| **F3** | The operator deploy path's epoch is unspecified, so "0 means never written" would be false for every hand-deployed instance | **Incorporated.** `0` is redefined as "no supervisor has written here", `roymctl` keeps it, and the false negative the review predicted does not occur: the supervisor's absent row reads `0` and the substrate reports `0`, so they match and the dependent converges (§19.3) |
| **F4** | "Retry once at the re-read epoch" produces a `Conflict`, never an `Applied` | **Incorporated** as a new finding §19.19 and **D-A5c-19**: retry at `held + 1`, no re-read (`Stale` already carries the number), and a genuinely-ahead writer is the generation's problem. Test renamed |
| **F5** | `remediation.terminal` is the set-but-never-cleared flag §19.12 refuses to create — a terminal service is never restarted, so the healthy sweep that clears the flag never fires | **Incorporated** as §19.20 and **D-A5c-20**. The pass applied that lesson to `Superseded` and reintroduced the pattern two sections later. `force-reconcile` and `adopt` now clear it, and the alert detail names the verb |
| **F6** | *(minor)* A `pause` landing mid-pass still gets a full pass of writes | **Incorporated.** The loop re-reads the desired-state row at the start of its write phase — it needs the generation from it anyway — and the WIT doc comment states the boundary (§19.16, D-A5c-14) |
| **F7** | Nothing asserts `instance-status.bindings` is populated — the exit criterion the push is kept for | **Incorporated**, and it is the finding that most matters: this is A5b's row-9 failure in advance. Tests **45 and 46** now read the join off a real `status` call, and §23 names them as the exit criterion's test |
| **F8** | Tests 40/41 are e2e and §19.18 leaves them with no stated trigger; row 11's coverage rests on it *(numbering as reviewed; they are now 47 and 48)* | **Answered differently.** The review asked for the trigger to be stated. Stating it would not have helped — §19.18's own conclusion means any e2e trigger is artificial (reaching into `supervisor.db` on a running node, or a test-only verb). Row 11's property is entirely supervisor-side control flow, so its test is now a **unit test with a fake actor**, and there is **no supervisor push e2e at all**: A5a's `binding_push_e2e.rs` already proves the wire path live. Row 12 keeps its e2e, whose trigger (stop one of two nodes) is real. Reasoning written into §23 |
| **F9** | *(minor)* D-A5c-3's orphan alert has no `AlertKind` and no test | **Incorporated** as §19.21 and **D-A5c-21**: a fourth kind, `OrphanedService`, in the round-trip test plus a raise test. §19.6's "three new kinds" corrected to four — though still not §14's four |
| **F10** | §19.5e understates the reach: registering `messaging` under the node DID hands every verified caller a subscribe handle for any `svc/<id>/#` topic, new reach on a supervisor-only node | **Incorporated.** Verified. Closed locally by namespacing the supervisor's subscribe path with the publish-side unconditional rule, so nothing escapes `svc/supervisor/` (D-A5c-6), plus a negative test and a widened backlog row. **Follow-through:** that fix makes three namespacing behaviours where §19.5c already called two a hazard, and the supervisor's is the one endpoint that deliberately refuses the cross-service opt-in — so §19.5c now requires a comment at both the registration site and the branch, with test 26 as the guard that actually holds if someone reverts it |

**Two factual corrections the review made to this pass**, both verified and
applied: `ServiceHealth` has **five** production construction sites in
`poll_once`, not three (`health.rs:185, 221, 243, 274, 289`); and the
member-index-0 comment is at `member_identity.rs:150`, with the call at
`:175`.

**One judgement the review offered and this pass does not take:** that F2 and
F8 strengthen the case for moving the binding push out to A5e. §22 answers
it — both findings, once resolved, made the push smaller rather than larger,
and F7 means moving it would move the exit criterion with it.

**Test count: 41 → 48**, and one test converted from e2e to unit (row 11).

---

# Part V — A5d: unattended renewal

**Status:** 📋 Planned (2026-08-02). Not started. This is the `§0` findings
pass **D-A5-2** requires before A5d is handed to an implementer. §15 planned
A5d to a six-item sketch — renewal, `RotationPolicy`, the `SynSvcNativeService`
gap, certificate maximum lifetime, master-anchor refresh, revocation — plus
one paragraph naming what "unattended" does not mean. Everything below is
what §15, `task.md`, and A5c's shipped code leave open, understated, or
stated in a way the actual tree does not support.

Same discipline as A0 §6 / A1 §6 / A2 §0 / A3 §0 / A4 §0 / P0 §0 / A5 §0 / A5c
§19.

**Headline.** §15's six items read as six independent features. Reading A5c's
shipped loop against them turns the count into **fourteen findings, and half
of them are about one fact none of the six items states**: there is no
substrate verb today that installs a certificate without reinstalling the
whole service. Every one of "renewal," "`RotationPolicy` becomes load-bearing,"
and "the `SynSvcNativeService` gap" is downstream of that one missing verb, and
§15 discusses all three as if the install path already existed. It doesn't —
A5c never needed it, because A5c's only certificate writes happen inside
`apply_plan`'s full redeploy (§19.10). A5d is the first slice that installs a
certificate on its own, and it has nowhere to send that write.

---

## §26 — What §15, `task.md`, and A5c leave open, understate, or state wrongly

### 26.1 (Scope-changing, blocking) There is no verb that installs a certificate without reinstalling the service, and reusing `deploy` is the wrong fix

§15 item 1 says renewal "reissue[s] via `deploy::certify_instance` through the
supervisor's own client," as if minting the certificate is the whole job. It
is half the job. `certify_instance` (`crates/sdk/src/deploy.rs:320-352`)
returns a `DelegationCertificate`; nothing in the tree accepts one on its own.

**What exists.** The only production writer of an instance certificate is
`deploy_with_context`'s cert-verification block
(`crates/control_plane/src/service/orchestration.rs:1065-1099`), which runs
inline in the middle of a full deploy, followed by the `set_instance_cert`/
`remove_instance_cert` write and the `SynSvcNativeService` rebuild
(`orchestration.rs:1493-1509`, `:1569-1580`). `deploy_with_context` also
reinstalls the wasm component or container, reruns FDAE policy validation, and
bumps the service's config generation
(`orchestration.rs:1350-1355`, rolled back via `rollback_config_generation`
at `:564-568` on any later failure). None of that is renewal's business — the
manifest, the config, and the FDAE policy have not changed.

**Reusing `deploy` for renewal would be the exact churn §19.2 and §19.10
already found and fixed once.** §19.10 stopped the loop from re-certifying
every member every pass because it "churns exactly the artifact A5d exists to
renew on a considered schedule." Routing renewal back through `deploy` (or
`apply_plan`) reopens that: the full hex-inlined Wasm artifact §19.2's backlog
row complains about would cross the wire on every renewal cycle, for a change
that touches nothing but a certificate.

**Fix.** A new verb, sized like `restart` — an in-place lifecycle action next
to a full reinstall, not a variant of one:

```wit
/// Install a freshly-issued instance certificate on an already-deployed
/// service, without reinstalling it -- the certificate-only counterpart to
/// `restart`. `generation` is checked against the app instance's recorded
/// management stamp when the service has one, exactly as `restart` is.
renew-cert: func(service-id: string, generation: u64, instance-certificate: string) -> result<_, string>;
```

`renew_cert_impl` gated identically to `restart_impl`
(`orchestration.rs:2061-2127`): the same `ORCHESTRATOR_DEPLOY` capability
check (`:2067-2076`), the same owner-or-node-wide-grantee check A5c's
D-A5c-15 added to `restart` (`:2078-2090`), the same generation gate scoped to
an app context only (`:2092-2100`). See **D-A5d-1**.

### 26.2 (Scope-changing) The cert-verification block is inline in `deploy_with_context`; `renew-cert` cannot duplicate it

`deploy_with_context`'s three checks — the certificate names this `service_id`
(`orchestration.rs:1070-1077`), it certifies the key this node derives for
this caller (`:1078-1092`), and its signature/window/scope verify
(`:1093-1095`) — are security-critical and currently only exist inline in one
function. `renew_cert_impl` needs the identical three checks; copying them is
exactly the "inline fully-qualified path twice" problem this repo's own
`AGENTS.md` import-cleanup rule exists to catch one level up, except the risk
here is worse than an import: two copies of DID/signature verification
drifting apart silently is a security bug, not a style one. Factor the block
into `verify_installed_instance_cert(node_identity, caller_did, service_id,
cert_json) -> Result<DelegationCertificate, String>`, called from both
`deploy_with_context` and `renew_cert_impl`. See **D-A5d-2**.

### 26.3 (Scope-changing, correctness prerequisite) `renew-cert` must rebuild `SynSvcNativeService`, and every piece it needs is already a stored fact

§15 item 3 calls this "a correctness prerequisite, not a follow-up," which is
right, but confirm what is actually broken today: **nothing, yet.** The
backlog row (`docs/planning/deferred-backlog.md:81`) and `SynSvcNativeService`'s
own doc comment (`crates/control_plane/src/synsvc_native.rs:100-112`) are
both explicit that the by-value cert copy cannot drift *today* because the
sole production writer of an instance cert (`deploy_with_context`) already
rebuilds the native service in the same pass
(`native_dispatch.insert(service_id, Arc::new(SynSvcNativeService::new(...)))`,
`orchestration.rs:1493-1509`). A5d is what makes the gap real: it is the
first code path that installs a certificate through anything other than
`deploy_with_context`.

**Every non-certificate input `SynSvcNativeService::new` needs is already
reachable without the manifest.** `orchestration.rs:1493-1509` passes
`key_store`, `storage_provider`, `blob_provider`, `messaging_broker`,
`node_identity`, `caller.caller_did`, and an `Arc<Policy>` derived from
`fdae_policy` — all `self.*` fields on `ControlPlaneService` except the FDAE
policy, which is a local variable computed from the manifest during deploy.
But the FDAE policy is also *persisted*: `save_fdae_policy`/`load_fdae_policy`
(`crates/data_db/src/sqlite.rs:1914-1947`, trait at
`crates/data_db/src/traits.rs:102-105`) round-trip it per `service_id`
independent of any deploy call. So `renew_cert_impl` does not need the
manifest at all: `load_fdae_policy(service_id)`, parse if present, then
`native_dispatch.insert` a fresh `SynSvcNativeService` with the new cert and
everything else unchanged. See **D-A5d-3**.

**One thing to confirm rather than assume — `caller_did`.** `SynSvcNativeService::new`
takes `&caller.caller_did` (`orchestration.rs:1504`, constructor at
`synsvc_native.rs:368-401`), and ADR-0020 amendment 21 makes the derived
instance key depend on it. This is not a gap: §0.19/D-A5-19 already established
that a supervisor cannot manage placement it did not itself deploy, so the
same client — the supervisor's own — is the caller on both the original
`deploy` and every later `renew-cert` for that instance. `renew_cert_impl`
passing through whichever `caller.caller_did` the RPC arrived under is
therefore correct without special-casing, because that caller is always the
same DID a working renewal path can ever reach.

### 26.4 (Scope-changing, blocking) Vault-locked detection is a per-call `Result`, and `VaultLocked` needs a place in a schema that has none reserved for a global fact

§15 promises "the renewal loop raises a `VaultLocked` alert... the moment it
finds the vault shut," without saying what that alert targets. `AlertRecord`
(`crates/app_orchestration/src/alerts.rs:111-124`) requires **both**
`instance_id: AppInstanceId` and `substrate_did: String` — there is no
schema slot for a supervisor-process-wide fact belonging to no instance and
no substrate, and D-A5c-6 already set the precedent of not touching this
schema for a new concern.

**This is not actually a gap — it is the same shape `SubstrateUnreachable`
already uses.** A locked vault, like an unreachable substrate, is one root
cause that affects every instance with a member due for renewal; the fix is
the same fan-out: raise `VaultLocked` once per **placed member currently near
expiry**, scoped to that member's `instance_id`/`logical_ref`/`substrate_did`
— not one row per fact, one row per affected member, which is exactly what an
operator reading `supervisor alerts <instance_id>` needs to see and exactly
what the existing schema already supports with no change.

**Detection is two-tiered, cheapest check first.** `KeyStore::kek_is_loaded()
-> bool` (`crates/data_keystore/src/key_store.rs:70-72`) is a non-`Result`,
no-I/O check — call it once per pass, before any `MasterVault::get_or_mint`
attempt. If false, skip the entire renewal work-list for this pass (not the
rest of `reconcile_instance_pass` — health, remediation, and the binding push
all continue; none of them touch the vault), and raise/refresh `VaultLocked`
for every near-expiry member. `VaultError::Locked`
(`crates/app_supervisor/src/keys.rs:36-44`) remains the per-call variant a
direct `get`/`get_or_mint` surfaces, for the (should-not-happen, defensive)
case where `kek_is_loaded()` said yes but the vault open still fails between
the check and the call. See **D-A5d-4**.

### 26.5 (Understated) The near-expiry decision needs no new poll — `ServiceHealth` already carries the two timestamps A5a shipped for this

§15 does not say where "near expiry" comes from. `ServiceHealth`
(`crates/sdk/src/health.rs:88-102`) already carries
`instance_certificate_issued_at`/`instance_certificate_expires_at`
(`:94-95`), filled at all five production sites A5c's D-A5c-5 already wired.
`is_near_expiry_parts(issued_at, expires_at, now)`
(`crates/identity/src/delegation.rs:27-37`, the 25%-of-lifetime-remaining
rule) is a `const fn` over exactly those two numbers. So the renewal decision
for a placed member is a pure function of data the pass's existing health poll
(`reconcile_instance_pass`, `service.rs:387`) already produced this pass — no
new RPC, no new poll interval. Renewal is a **fourth work-list**, computed
alongside `needs_work` and `restart_candidates`, not folded into either: it is
neither a plan diff nor a health-based remediation, and treating it as one
would make it retry on the wrong triggers (§26.12). See **D-A5d-5**.

### 26.6 (Ambiguous) `RotationPolicy` is already on the wire and in the stored plan; the decision is client-side, and the substrate never needs to see it

§15 item 2 says `RotationPolicy` "becomes load-bearing here and only here,"
without saying which side reads it. It cannot be the substrate: `models.rs`'s
`RotationPolicy` (`crates/app_orchestration/src/models.rs:237-240`, field at
`:369`) maps one-way to `WitRotationPolicy` inside `service-config`
(`mapper.rs:165-168`, `control-plane.wit:75`) and is **never matched on
anywhere in the substrate** — confirmed dead code today. It does not need to
become live there either: the supervisor already holds the full stored plan
(the same `DeploymentPlan`/`ServiceSpec` per service `SupervisorStore`
persists from `submit`), so `svc.config.rotation_policy` is a local read
inside the renewal work-list, made once the new cert has installed
successfully. `RestartOnRotation` issues a `restart` call — the same RPC
A5c's remediation already exercises with its owner/generation gate — `None`
does nothing further. The wire field stays exactly as A5c's mapper already
emits it; A5d adds a reader, not a new carrier. See **D-A5d-6**.

### 26.7 (Scope-changing) A substrate-side maximum-lifetime cap would break the attended posture's own definition; the fix is a generous backstop, not a forcing function

§15 item 4 says renewal "closes" the backlog row wanting a maximum instance
certificate lifetime (`docs/planning/deferred-backlog.md:86`), framing it as
something the online-key posture simply enables. It does not simply enable
it — enforcing a **tight** cap substrate-side would contradict ADR-0020 §3
directly: "**Attended posture:** certificates are long-lived and revocation is
the real control... An operator issues on a stated cadence." An attended-mode
operator's `--expires-hours` is deliberately long-lived by design, and a hard
substrate-side ceiling tuned for the online-key posture's short renewal
cadence would refuse that operator's own deploys.

**The two postures need two different things from this row, and only one of
them is A5d's to give.** The online-key posture gets short-lived certificates
for free the moment renewal is automated — that is §15's own framing, and it
needs no enforcement, only A5d's renewal loop choosing a short
`expires_hours` when it calls `certify_instance`. What A0 never built, and
what the backlog row is actually asking for, is a **backstop**: nothing bounds
`expires_at_secs` at all today, so a client-side mint error (or a malicious
one) can produce a certificate valid for years, unnoticed until the near-expiry
warning simply never fires. The fix the row wants is a generous ceiling at
install verification — the same shape task.md's own `EndpointInfo.not_after`
precedent already uses and justifies: "deliberately generous (30 days) so it
is a backstop for a signer that stops renewing entirely, not a routine failure
mode." A 30-day cap, enforced inside §26.2's new shared
`verify_installed_instance_cert` helper (so both `deploy` and `renew-cert`
apply it identically) does not touch any attended-posture operator running
the CLI's actual default (24h) or anything a reasonable manual cadence would
choose, and catches exactly the unbounded mistake the row names. See
**D-A5d-7**.

### 26.8 (Scope-changing) Master-anchor refresh needs its own cadence and its own persisted "last refreshed" fact — riding the reconcile tick is not free

§15 item 5 says "the loop performs it," as if the existing 30-second pass
tick is the schedule. It should not be: `refresh_master_anchor`
(`crates/core/src/dht_registry.rs:624`) re-signs and republishes the whole
anchor payload over HTTP (and optionally the DHT), and the backlog row this
closes (`deferred-backlog.md:89`) calls the failure mode a **daily** duty — an
anchor older than 24 hours stops verifying at every consumer. Running that
call every 30 seconds is needless load for a fact that only needs to move once
a day; running it only when a service happens to need renewal ties two
unrelated cadences together for no reason.

**Fix.** `SupervisorRole` gains `master_anchor_refresh_interval_secs`
(default a fraction of the anchor's 24-hour validity window with real
margin — a day-scale interval refreshed well before expiry, the same
margin-before-expiry shape `is_near_expiry_parts`'s 25% already uses for
certificates). The 30-second pass tick stays the only timer in the process
(D-A5c-8 already made spawning a second one a reviewed, deliberate choice for
the main loop; a third scheduled thing does not get its own): each pass reads
a small persisted `(master_did, last_refreshed_at)` fact from
`SupervisorStore` and only calls `refresh_master_anchor` when overdue. This is
the same "evaluated every pass against a persisted fact, not a new timer"
shape D-A5c-9's single client set and D-A5c-14's pause re-read already
established for this loop.

**The read-modify-write race** (`deferred-backlog.md:89`: "a compare-and-set
the registry does not offer, or a single writer") is closed by the same fact
ADR-0020 amendment 20 already established for custody: mint-in-place means
exactly one `MasterVault` ever generates a given master, and export/import
moves a **file**, not concurrent live access. Under the topology this tree
actually supports — one supervisor process holding a given master at a time —
there is structurally one writer. A5d documents this as the accepted
invariant rather than building a CAS the registry (`crates/community_registry`)
does not expose; a redundant-supervisor deployment sharing one master via
`import-master` on two live processes is out of scope here and stays a backlog
row, since it needs the registry to grow compare-and-set semantics, not
anything A5d's loop can provide alone. See **D-A5d-8**.

### 26.9 (Coverage) `CertificateNearExpiry` and `CertificateExpired` already exist and are already dead; A5d wires them up, it does not add them

`AlertKind` (`crates/app_orchestration/src/alerts.rs:29-70`) already carries
`CertificateNearExpiry` (`:40`) and `CertificateExpired` (`:44`), with their
`Display`/`FromStr` arms already written (`:78-79`, `:98-99`) and already
covered by the round-trip test (`:391-403`). Neither is raised anywhere in
`crates/app_supervisor` today — grep confirms zero call sites. §15 does not
mention them at all, reading as though A5d must add new kinds; it must not,
and the round-trip test needs no new entries.

**Their purpose, precisely.** They are not renewal's success path — a
renewal that succeeds needs no alert. They are what a stalled renewal reports:
`CertificateNearExpiry` when a member is within the 25% window and either the
vault is locked (§26.4, alongside `VaultLocked` on the same member) or a
`renew-cert` call itself failed; `CertificateExpired` when
`is_expired_parts` (`delegation.rs:46-48`) says it is too late. Both need the
same clearing rule §19.20 (F5) established for `remediation.terminal`: raised
alerts with no path back to cleared are exactly the bug that finding exists to
prevent. Both clear the moment a subsequent pass's health poll reports a
non-near-expiry timestamp for that member — the same healthy-sweep clearing
shape D-A5c-11's `Superseded` and D-A5c-20's `remediation.terminal` already
use, recomputed from the substrate's own answer rather than tracked as a flag.
See **D-A5d-9**.

### 26.10 (Scope-changing, blocking) Revocation has no production write path today, and revoking a key without also stopping its renewal undoes itself

§15 item 6 names the backlog row (`deferred-backlog.md:88`) and stops there.
The row itself says the gap plainly: "nothing in the tree can *add* to a
master anchor's `revoked_keys`... the only non-empty publisher is a unit
test." `apps/roymctl/src/commands/identity.rs`'s `publish-anchor` hardcodes
`vec![]`. `MasterAnchorPayload.revoked_keys` (`dht_registry.rs:650`) is read
(`crates/router/src/handshake.rs:89`, `route_handler/io.rs:69`) but never
written outside `handshake.rs`'s own test (`:362-383`), which mutates a
`MockResolver`'s in-memory state directly — not a call any production code
path exercises.

**What the instance-level DID actually is, so the verb has an argument to
take.** The revoked entry is the derived **instance** DID (`temporary_did`),
not the master — the same value `resolve-instance-identity`
(`control-plane.wit:182`) already computes deterministically from `(node
identity, calling DID, service_id)`. `roymctl supervisor revoke-instance
<instance_id> <logical_ref>` needs no stored "current instance DID" table: it
re-derives the same value any deploy or renewal would, by asking the
substrate the member is placed on.

**The self-defeating case §15 does not mention.** Revoking a key and leaving
the member under active management is not a fix — the very next pass's
renewal work-list (§26.5) finds that member's certificate near expiry (or not
even that, if it hasn't reached the 25% window), mints a fresh one via
`certify_instance`, and installs it via `renew-cert`, silently reinstating the
key the operator just revoked. A revoked instance's certificate ages out
under the attended posture precisely because nothing renews it; under the
online-key posture, something does, and revocation must say so. `revoke-
instance`'s implementation therefore does two things in one operator action,
same as `retire` is one verb for "several related state changes, not a
teardown": (1) the anchor mutation — read-modify-write the current anchor
(§26.8's single-writer invariant applies identically here), append the
derived instance DID, re-sign, republish; (2) mark that specific
`(instance_id, logical_ref)` placement excluded from the renewal work-list
going forward — the same per-service "stop attempting this one, keep
reconciling the rest" shape §19.1's `PlacementChangeRefused` already
established, not a whole-instance pause. An operator who wants the process
itself gone still issues `undeploy`/`retire` separately, the same two-step
`§21`'s manual relocation path already documents. See **D-A5d-10**.

### 26.11 (Understated) Composition: renewal is a step inside the existing write phase, not a second pass or a second tick

The user-facing question this pass exists to answer plainly: **renewal folds
into `reconcile_instance_pass`, it does not get its own pass.** Three existing
decisions already force this rather than leave it open:

- **D-A5c-7's per-instance lock** is held for the whole of a resident pass
  (`service.rs:232-233`) and for every operator write. A standalone renewal
  path running on its own tick would either duplicate that lock (racing a
  concurrent `submit`/`adopt` against `renew-cert`, exactly what D-A5c-7
  exists to prevent) or need its own coordination invented from scratch for a
  problem the pass already solves.
- **D-A5c-9's one client set per pass** already builds and closes a
  `BTreeMap<SubstrateAlias, Arc<SyneroymClient>>` per instance
  (`reconcile_instance_pass`, `:349`, `:533`). Renewal needs a live client to
  the same substrate the health poll and the write phase already connected
  to this pass; reusing it is free, a second connection is not.
- **§26.5's near-expiry data comes from this pass's own health poll.** A
  separate renewal cadence would need its own poll to get the same two
  timestamps `ServiceHealth` already carries here.

So renewal is a fourth work-list inside `apply_write_phase`
(alongside `needs_work`, `restart_candidates`, and the binding push), gated
the same way the write phase already is — only entered when there is
something to do — and using the pass's existing clients, lock, and health
data. It needs no new interval config on `SupervisorRole` beyond §26.8's
master-anchor field: the near-expiry check is cheap enough (a pure function
over two integers already in memory) to evaluate every 30-second tick without
a separate cadence, the same way remediation's healthy-sweep check already
does. See **D-A5d-11**.

### 26.12 (Coverage) Dedup against `needs_work`, not against `restart_candidates`

A service already in this pass's `needs_work` (§19.2, D-A5c-2) is about to go
through `apply_plan` → `certify_placed_members`
(`deploy.rs:359`, called from `service.rs:1130-1138`), which mints a fresh
certificate unconditionally as part of that path — adding it to the renewal
work-list too would certify it twice in one pass for no reason. `restart_
candidates` (remediation, §21) is different: a bounded restart reloads the
running instance in place and touches no certificate at all, so a service
under remediation this pass still needs an independent renewal check. The
renewal work-list is therefore `{placed members near expiry} \ needs_work`,
evaluated **after** `needs_work` is computed and **independently of**
`restart_candidates`. See **D-A5d-12**.

### 26.13 (Understated) The order within one member's renewal: mint, install, then rotate — and a failure at any step must not silently skip the next

Per renewal candidate: `certify_instance` (mint, client-side, no substrate
write yet) → `renew-cert` (§26.1, installs + rebuilds `SynSvcNativeService`,
§26.3) → if `RotationPolicy::RestartOnRotation` (§26.6), `restart`. A failure
at `certify_instance` (e.g. the vault locked mid-loop, §26.4's race between
the cheap check and the actual mint) or at `renew-cert` (e.g. the substrate
unreachable) must not attempt the `restart` step — restarting a service whose
certificate installation just failed serves nothing and burns a lifecycle
action for no gain. Each step's failure raises `CertificateNearExpiry` (not
yet expired, so not the harder alert) naming which step failed, and the
member is retried next pass rather than the pass failing the whole instance.
See **D-A5d-13**.

### 26.14 (Understated) `revoke-instance` takes the same per-instance lock every other write verb does

§26.10's design has `revoke-instance` write two things: the anchor, and the
placement's renewal-exclusion. Both must happen under `handle_revoke_
instance`'s own acquisition of `instance_lock(instance_id)`
(`service.rs:173-178`), the same discipline `handle_submit`/`handle_adopt`/
`handle_release`/`handle_retire`/`handle_force_reconcile` already follow.
Without it, an operator's `revoke-instance` and a resident pass's renewal
work-list for the same member are racing: the pass could mint and install a
fresh certificate for a member the operator is mid-revoke on, in the gap
between the anchor write and the exclusion write landing. Taking the lock for
the whole verb, the same way every other instance-scoped write does, closes
the same class of race D-A5c-7 closed for every earlier verb. See
**D-A5d-14**.

### 26.15 (Correctness, found in review — F-A5d-1) The renewal-exclusion in §26.10 only stops the loop; `submit` and `force-reconcile` silently reinstate a revoked key

§26.10's design stops the resident loop's own renewal work-list from
re-certifying a revoked member, but that is not the only place a certificate
gets (re)minted. `handle_submit`/`deploy_submission` and
`handle_force_reconcile` (`service.rs:1089-1110`, `:1765-1810`) both call
`apply_with_clients(plan, plan, ...)` (`:1121-1135`) with the **full, stored
plan** — not a filtered one — which calls `certify_placed_members`
(`deploy.rs:359`) unconditionally for every service the plan names, with no
revocation check anywhere on that path. So an operator who revokes a member
and later runs an ordinary `submit` of the same plan, or a `force-reconcile`
(which `handle_force_reconcile`'s own doc, `service.rs:1798-1801`, already
notes bypasses several checks `submit` applies, for the identical reason
§19.1 first found), gets that member silently re-certified and reinstalled —
no error, no alert, nothing in §29's original test list catches it. This
directly undercuts §26.10's own premise: "revocation must say so." As
designed it only says so in one of at least two live re-mint paths.

**Fix.** A revoked placement is a persisted fact, not something derived only
inside the renewal work-list: `SupervisorStore` gains a small
`revoked_placements` table, `(app_instance_id, logical_ref) -> revoked_at`,
written by `revoke-instance` (§26.10) and read by **two** consumers: the
renewal work-list's exclusion (as originally designed) **and**
`apply_with_clients`, which now skips `certify_placed_members` (and the
deploy/redeploy call under it) for any service present in that table,
regardless of whether the caller was the loop, `submit`, or `force-reconcile`.
A revoked service is excluded from the effective plan the same way §19.1
excludes a placement-changed one — skipped, not failing the whole call —
and raises a new `AlertKind::InstanceRevoked` (D-A5c-21's precedent: add a
kind only when reusing an existing one would misreport, and none of the ten
existing kinds names "this service will not be reinstalled while revoked").
See **D-A5d-15**, and D-A5d-12's formula revised below.

### 26.16 (Understated, found in review — F-A5d-2) Renewal needs its own certificate lifetime, and nothing in this pass named one

§26.7 argues the online-key posture "gets short-lived certificates for free"
once renewal is automated, but never says how short. The supervisor already
has a constant for this — `INSTANCE_CERT_EXPIRES_HOURS`
(`crates/app_supervisor/src/service.rs:60-63`, `= sdk::deploy::
DEFAULT_INSTANCE_CERT_EXPIRES_HOURS = 24`), used by every existing
`certify_placed_members` call, with its own doc comment flagging exactly this
gap: "the attended posture's default, since A5b mints and certifies but does
not yet renew (A5d)." Left untouched, A5d could ship minting at the same 24
hours the deploy path already uses, which delivers none of what §15 promises
("certificates stay short-lived... become the default").

**Fix.** `SupervisorRole` gains `renewed_cert_expires_hours` (default **4**):
comfortably larger than the 30-second pass interval times the number of
passes `is_near_expiry_parts`'s 25%-remaining rule leaves before actual
expiry (many, at 4 hours), so renewal has wide margin to succeed before a
cert is genuinely at risk, while being a real reduction in blast radius from
24 hours. This constant is already supervisor-scoped, not shared with
`roymctl`'s own attended-posture `--expires-hours` default — changing it
touches no operator-facing CLI default. It replaces
`INSTANCE_CERT_EXPIRES_HOURS` for **both** the initial `certify_placed_members`
mint and every renewal's `certify_instance` call, so a managed instance has
one certificate lifetime throughout its life rather than a confusing 24-hour
first cert followed by 4-hour renewals. See **D-A5d-16**.

### 26.17 (Understated, found in review — F-A5d-3) A vault lock discovered mid-mint must raise the same alert as one discovered before the pass starts

§26.4's design raises `VaultLocked` from the cheap, up-front `kek_is_loaded()`
check, but §26.13's generic per-step failure handling — which raises
`CertificateNearExpiry` naming the failed step — is what actually runs when
the defensive race fires (`kek_is_loaded()` said unlocked, then the real
`certify_instance`/`get_or_mint` call hits `VaultError::Locked` between the
check and the call). The same root cause then surfaces under two different
`AlertKind`s depending purely on when it is detected within one pass, which
directly undercuts §26.4's own stated purpose — making the unattended/
attended gap "visible in the same place every other supervisor fault is."

**Fix.** The per-candidate mint/install step (§26.13, D-A5d-13) special-cases
`VaultError::Locked` specifically: that error, wherever it is caught, raises/
refreshes `VaultLocked` for that member — never the generic
`CertificateNearExpiry` per-step handler. Every other failure cause at any
step keeps raising `CertificateNearExpiry` naming the step, unchanged. One
condition, one alert kind, regardless of which of the two checks caught it.
See **D-A5d-17**.

### 26.18 (Understated, found in review — F-A5d-5, F-A5d-6) `renew_cert_impl`'s construction inputs and its FDAE re-parse failure mode need pinning down precisely

Two gaps in §26.3's design, both found by checking it against the real
`SynSvcNativeService::new` call site rather than a summary of it:

**(a) The enumerated input list was incomplete.** §26.3 named `key_store`,
`storage_provider`, `blob_provider`, `messaging_broker`, `node_identity`,
`caller.caller_did`, and the FDAE policy as "every non-certificate input" —
omitting two of the constructor's eleven parameters,
`service_proxy: Weak<dyn ServiceProxy>` and `row_authorizer: Weak<dyn
RowAuthorizer>` (`synsvc_native.rs:368-401`), supplied at the one existing
call site via `self.current_service_proxy()`/`self.current_row_authorizer()`
(`orchestration.rs:1505-1506`). Both remain `self`-reachable the identical
way, so D-A5d-3's conclusion still holds — but `renew_cert_impl` must mirror
the **whole** call site, not a partial enumeration of it, or an
implementation copying only the named list produces a native service with
dead proxy/authorizer hooks.

**(b) A stored FDAE policy that fails to re-parse must abort the renewal, not
silently drop enforcement.** `load_fdae_policy` can return `Some(json)` for
bytes that were valid when saved but that a schema/parser change since then
now rejects (the same `parse_and_validate` call the deploy path already runs,
`orchestration.rs:1352-1355`, which fails the *whole deploy* on a bad
document). `renew_cert_impl` must fail the same way — abort before installing
anything — rather than falling back to `fdae_policy: None`, which would
silently remove row/column filtering for that service's renewed instance.
See **D-A5d-18**.

### 26.19 (Correctness, found in review — F-A5d-7) `renew-cert` needs the same "is this actually deployed" gate `restart` already has

`restart_impl` refuses a `service_id` with no recorded deploy facts
(`orchestration.rs:2102-2106`) before doing anything else. §26.1/D-A5d-1's
gating for `renew-cert` copies `restart_impl`'s capability, owner, and
generation checks, but not this one. Without it, a caller holding an
`ORCHESTRATOR_DEPLOY` grant naming a `service_id` that was never deployed
could call `renew-cert` and have `renew_cert_impl` construct and register a
live `SynSvcNativeService`/native-dispatch entry for it: `owner_of(&service_
id).is_none()` passes the owner check vacuously, and `load_fdae_policy`
returning `None` for an unknown id is not an error. Low practical
exploitability — the supervisor's own renewal only ever targets a
`service_id` it itself deployed, per §0.19/D-A5-19 — but the gate is missing
and unaddressed. Fix: `renew_cert_impl` requires `self.registry.deploy_
facts(&service_id).is_some()` before proceeding, the identical signal
`restart_impl` already relies on. See **D-A5d-19**.

---

## §27 — Decisions

| ID | Decision |
|---|---|
| **D-A5d-1** | A new WIT verb, `renew-cert(service-id, generation, instance-certificate) -> result<_, string>`, installs a certificate in place without reinstalling the service (§26.1). Gated identically to `restart`: `ORCHESTRATOR_DEPLOY` capability, owner-or-node-wide-grantee check, generation gate scoped to an app context. `deploy`/`apply_plan` are not renewal's path; reusing them was considered and rejected as the exact per-pass churn §19.2/§19.10 already fixed once. |
| **D-A5d-2** | The cert-verification block inline in `deploy_with_context` (`orchestration.rs:1065-1099`) is factored into `verify_installed_instance_cert(node_identity, caller_did, service_id, cert_json)`, called from both `deploy_with_context` and the new `renew_cert_impl` (§26.2). No duplicated security-critical logic. |
| **D-A5d-3** | `renew_cert_impl` rebuilds `SynSvcNativeService` via `native_dispatch.insert`, sourcing every non-certificate field from `self.*` (already available on `ControlPlaneService`) and the FDAE policy from `storage_provider.load_fdae_policy(service_id)` (§26.3) — no manifest needed. Confirmed as a real prerequisite (nothing is broken by its absence *today*, since A5c's only cert writer already rebuilds this in the same pass) rather than a symptom of an existing bug. |
| **D-A5d-4** | `VaultLocked` is raised/cleared per **placed member currently near expiry**, using the existing `AlertRecord` schema (`instance_id`/`logical_ref`/`substrate_did`) unchanged — the same per-affected-entity fan-out `SubstrateUnreachable` already uses for one root cause touching several rows (§26.4). Detection: `KeyStore::kek_is_loaded()` (cheap, non-`Result`) checked once per pass before any vault call; `VaultError::Locked` remains the defensive per-call fallback. A locked vault skips only the renewal work-list, not the rest of the pass. |
| **D-A5d-5** | The near-expiry decision is a pure read of `ServiceHealth.instance_certificate_{issued,expires}_at` (already populated by D-A5c-5) through `is_near_expiry_parts` — no new poll (§26.5). Renewal is its own, fourth work-list (`needs_work`, `restart_candidates`, renewal, binding push), not folded into either existing one. |
| **D-A5d-6** | `RotationPolicy` is read from the supervisor's own stored plan (`ServiceSpec.rotation_policy`, already on the wire via `mapper.rs`) after a successful `renew-cert`, never from the substrate (§26.6) — the substrate-side field stays unread, as it is today. `RestartOnRotation` issues a `restart` call after install; `None` does not. |
| **D-A5d-7** | A **30-day** maximum instance-certificate lifetime is enforced inside D-A5d-2's shared `verify_installed_instance_cert`, applied uniformly to every deploy and every renewal (§26.7) — a generous backstop against an unbounded mint, matching the reasoning already used for `EndpointInfo.not_after`, not a forcing cap that would reject the attended posture's deliberately long-lived certificates. |
| **D-A5d-8** | `SupervisorRole` gains `master_anchor_refresh_interval_secs` (default a day-scale interval with real margin before the anchor's 24-hour validity window). No second timer: each 30-second pass reads a persisted `(master_did, last_refreshed_at)` fact from `SupervisorStore` and calls `refresh_master_anchor` only when overdue (§26.8). The read-modify-write race is closed by documenting mint-in-place's existing single-writer invariant, not by building compare-and-set the registry does not offer; concurrent redundant-supervisor custody of one master stays a backlog row. |
| **D-A5d-9** | `CertificateNearExpiry`/`CertificateExpired` (already-defined, already-tested, currently unused `AlertKind` variants) are wired to fire when a renewal attempt for a near-expiry/expired member does not succeed this pass (§26.9) — not on every near-expiry member unconditionally, since a member whose renewal *does* succeed needs no alert. Both clear on the next pass's health poll reporting a healthy timestamp, the same recomputed-not-flagged shape as `Superseded` (D-A5c-11) and `remediation.terminal` (D-A5c-20). |
| **D-A5d-10** | **Revised after review (F-A5d-1).** `roymctl supervisor revoke-instance <instance_id> <logical_ref>` re-derives the instance DID via `resolve-instance-identity`, appends it to the current master anchor's `revoked_keys` under D-A5d-8's single-writer invariant, and republishes. The exclusion itself is now a **persisted fact** (D-A5d-15), not something the renewal work-list alone enforces. Tearing the process down is still a separate `undeploy`/`retire`, the operator's own two-step. |
| **D-A5d-11** | Renewal is a step inside the existing `reconcile_instance_pass`/`apply_write_phase`, using the pass's own per-instance lock (D-A5c-7), its one client set (D-A5c-9), and its own health poll's data (§26.5) — not a second pass, not a second tick, not a second client build (§26.11). |
| **D-A5d-12** | **Revised after review (F-A5d-4).** The renewal work-list is `{placed members near expiry} \ needs_work \ revoked_placements` — a service already going through `apply_plan`/`certify_placed_members` this pass is not renewed again; a service under `restart_candidates` remediation is still independently checked, since a restart touches no certificate; a revoked placement (D-A5d-15) is never a candidate (§26.12, §26.15). |
| **D-A5d-13** | Per candidate, the order is mint → install → rotate (`certify_instance` → `renew-cert` → conditional `restart`), and a failure at any step skips the remaining steps for that member without failing the rest of the pass; the member is retried next tick (§26.13). `VaultError::Locked` at any step is carved out from this generic handling — see D-A5d-17. |
| **D-A5d-14** | `handle_revoke_instance` takes the instance's lock (`instance_lock`, D-A5c-7's mechanism) for the whole verb, so it cannot race the resident loop's own renewal of the same member (§26.14). |
| **D-A5d-15** | *(found in review, F-A5d-1)* `SupervisorStore` gains a `revoked_placements` table (`(app_instance_id, logical_ref) -> revoked_at`), written by `revoke-instance` and read by **both** the renewal work-list and `apply_with_clients` (§26.15). `apply_with_clients` skips `certify_placed_members` and the (re)deploy under it for any revoked placement, on **every** caller — `submit`, `force-reconcile`, and the resident loop alike — raising a new `AlertKind::InstanceRevoked` (D-A5c-21's precedent: a new kind only because none of the existing ten would describe it without misreporting) and continuing with the rest of the plan. Closes the gap where an ordinary `submit`/`force-reconcile` silently reinstated a revoked key. |
| **D-A5d-16** | *(found in review, F-A5d-2)* `SupervisorRole` gains `renewed_cert_expires_hours` (default **4**), replacing `INSTANCE_CERT_EXPIRES_HOURS` for **both** the initial `certify_placed_members` mint and every renewal's `certify_instance` call (§26.16) — one certificate lifetime for a managed instance's whole life, a real reduction from 24 hours, comfortably inside D-A5d-7's 30-day backstop, and untouched for `roymctl`'s own attended-posture default. |
| **D-A5d-17** | *(found in review, F-A5d-3)* `VaultError::Locked`, wherever caught during a renewal candidate's mint/install step, always raises/refreshes `VaultLocked` — never the generic `CertificateNearExpiry` per-step handler D-A5d-13 uses for every other failure cause (§26.17). One root cause, one alert kind, regardless of whether `kek_is_loaded()`'s up-front check or the per-call race caught it. |
| **D-A5d-18** | *(found in review, F-A5d-5, F-A5d-6)* `renew_cert_impl` mirrors the **whole** of `SynSvcNativeService::new`'s existing call site — including `service_proxy`/`row_authorizer`, omitted from §26.3's original enumeration — not a partial list of it (§26.18). A stored FDAE policy that fails to re-parse aborts `renew_cert_impl` before installing anything, the same failure mode `deploy_with_context` already has for a bad policy document, rather than silently dropping enforcement. |
| **D-A5d-19** | *(found in review, F-A5d-7)* `renew_cert_impl` requires `self.registry.deploy_facts(&service_id).is_some()` before proceeding, mirroring the gate `restart_impl` already has (§26.19) — refusing a `renew-cert` call against a `service_id` that was never deployed, even for a capability-holding caller. |

---

## §28 — Phase plan and merge order

Each phase is independently reviewable, and the substrate-side verb lands
before anything in the loop depends on it.

1. **Substrate-side, no supervisor involved.** D-A5d-2's
   `verify_installed_instance_cert` extraction (used, unchanged in behavior,
   by the existing `deploy_with_context`); D-A5d-7's 30-day cap added inside
   it; the new `renew-cert` WIT verb, `renew_cert_impl` (D-A5d-1, D-A5d-3,
   **D-A5d-18's full-call-site mirror and abort-on-bad-FDAE-reparse**,
   **D-A5d-19's `deploy_facts` gate**), and its `crates/sdk` client wrapper
   (`SyneroymClient::renew_cert`, mirroring `restart`'s shape at
   `sdk/src/lib.rs:764-772`). Mergeable alone; exercised directly against a
   running substrate with no supervisor changes. Tests 1-8, 33-35.
2. **Supervisor-side plumbing, no loop changes yet.** `SupervisorRole` gains
   `master_anchor_refresh_interval_secs` (D-A5d-8) and
   `renewed_cert_expires_hours` (**D-A5d-16**, replacing
   `INSTANCE_CERT_EXPIRES_HOURS`); `SupervisorStore` gains the
   `(master_did, last_refreshed_at)` table and **D-A5d-15's
   `revoked_placements` table**; `VaultLocked` and **`InstanceRevoked`**
   added to `AlertKind` (`Display`/`FromStr` match arms, round-trip test) —
   the two genuinely new variants (§26.9 confirms `CertificateNearExpiry`/
   `CertificateExpired` already exist). Tests 9-13, 36.
3. **The renewal work-list.** Computed inside `apply_write_phase` per
   D-A5d-5/11/12: near-expiry detection off the pass's existing `ServiceHealth`
   data, deduped against `needs_work` **and `revoked_placements`**, gated on
   `kek_is_loaded()` with `VaultLocked` fan-out (D-A5d-4) when locked. Per
   candidate: mint (at D-A5d-16's `renewed_cert_expires_hours`), install,
   conditional rotate, per-step failure handling (D-A5d-13, **with
   D-A5d-17's `VaultError::Locked` carve-out to `VaultLocked` rather than
   the generic handler**); `CertificateNearExpiry`/`CertificateExpired`
   raised on stall, cleared on the next healthy read (D-A5d-9). Tests 14-24,
   37-38.
4. **Master-anchor refresh on the existing tick.** D-A5d-8's overdue check
   and `refresh_master_anchor` call, evaluated once per pass per master this
   instance's plan names. Tests 25-27.
5. **Revocation surface.** `roymctl supervisor revoke-instance`
   (`SupervisorCommands::RevokeInstance`, following `Retire`'s single-`String`
   shape at `supervisor.rs:58`), `handle_revoke_instance` under the instance
   lock (D-A5d-14), the anchor mutation, **the `revoked_placements` write
   (D-A5d-15)**, and **`apply_with_clients`' skip-and-alert gate that every
   caller — `submit`, `force-reconcile`, and the loop — now shares**. Tests
   28-32, 39-40.

**What could move, stated the way A5c's §22 stated its own:**

- **Nothing here depends on A5e.** All six of §15's items are self-contained
  once phase 1's verb exists; the milestone's remaining scale-out work
  (`replicas`, cross-app `Bind`, ADR-0021 §7) touches none of this slice's
  surface.
- **Phase 5 (revocation) does *not* ship separately — withdrawn after
  review (F-A5d-1).** The first draft of this pass recommended the opposite,
  reasoning that matrix row 14's *property* (a revoked key fails while a
  fresh one from the same master still verifies) was already proven by A0's
  test, so the operator-surface half was the least load-bearing of the six
  items. That was wrong: as originally scoped, revocation didn't survive an
  ordinary `submit`/`force-reconcile` on the same instance (§26.15), so
  nothing in this plan actually proved the operator-facing property —
  "once I revoke this key, it stays revoked" — at all. D-A5d-15's
  `revoked_placements` table is now load-bearing for **every** caller that
  can mint a certificate, not only the loop, and phase 5 is no longer
  separable from phases 1-3 without leaving that gap open again.
- **§26.7's 30-day cap could, in principle, be its own tiny slice** ahead of
  everything else here, since it touches only `deploy_with_context` and needs
  no supervisor code at all. Not split out: it shares D-A5d-2's extraction
  work exactly, and extracting the helper twice (once for the cap, once for
  `renew-cert`) is strictly more work than doing both in phase 1.

---

## §29 — A5d tests

Named the way §8, §13, and §23 named theirs. **e2e are marked; everything
else is a unit test.**

**Phase 1 —** `crates/control_plane/src/service/orchestration.rs`,
`crates/sdk/src/lib.rs`, `control-plane.wit`:

1. `renew_cert_installs_a_new_certificate_without_touching_the_config_generation`
2. `renew_cert_rebuilds_syn_svc_native_service_with_the_new_certificate` —
   signs a `RelationshipProof` immediately after and asserts it verifies
   against the *new* cert, not the one construction started with
3. `renew_cert_is_refused_for_a_service_owned_by_another_caller`
4. `renew_cert_by_a_node_wide_deploy_grantee_ignores_the_service_owner`
5. `renew_cert_respects_the_same_generation_gate_as_restart`
6. `verify_installed_instance_cert_rejects_a_certificate_over_the_thirty_day_cap`
7. `verify_installed_instance_cert_accepts_the_cli_default_twenty_four_hour_certificate`
   — the attended-posture regression guard for D-A5d-7
8. `renew_cert_leaves_fdae_policy_untouched_when_none_was_ever_saved` — the
   `load_fdae_policy` `None` arm

**Phase 2 —** `crates/core/src/config.rs`, `crates/app_supervisor/src/store.rs`,
`crates/app_orchestration/src/alerts.rs`:

9. `supervisor_role_master_anchor_refresh_interval_secs_has_a_day_scale_default`
10. `store_persists_and_reads_back_last_refreshed_at_per_master`
11. `every_alert_kind_round_trips_through_display_and_from_str` — extended
    with `VaultLocked`, not a new test (the existing one at `alerts.rs:391`)
12. `vault_locked_is_scoped_to_the_specific_near_expiry_member_it_affects`
13. `raising_vault_locked_for_two_members_of_one_instance_opens_two_rows`

**Phase 3 —** `crates/app_supervisor/src/service.rs`:

14. `a_pass_renews_a_member_within_the_near_expiry_window`
15. `a_pass_does_not_renew_a_member_outside_the_near_expiry_window`
16. `a_member_in_needs_work_is_not_also_renewed_this_pass` (D-A5d-12)
17. `a_member_under_restart_remediation_is_still_checked_for_renewal` (D-A5d-12)
18. `a_locked_vault_skips_renewal_but_not_health_or_remediation_this_pass`
19. `a_locked_vault_raises_vault_locked_for_every_near_expiry_member`
20. `restart_on_rotation_follows_a_successful_install_with_a_restart_call`
21. `rotation_policy_none_installs_without_restarting`
22. `a_failed_mint_does_not_attempt_install_or_restart_for_that_member`
23. `a_failed_install_does_not_attempt_restart_for_that_member`
24. `certificate_near_expiry_clears_on_the_next_passs_healthy_read`

**Phase 4 —** `crates/app_supervisor/src/service.rs`,
`crates/core/src/dht_registry.rs`:

25. `master_anchor_refresh_is_skipped_when_not_yet_overdue`
26. `master_anchor_refresh_fires_once_the_interval_elapses`
27. `master_anchor_refresh_updates_last_refreshed_at_on_success`

**Phase 5 —** `apps/roymctl/src/commands/supervisor.rs`,
`crates/app_supervisor/src/service.rs`:

28. `revoke_instance_appends_the_derived_instance_did_to_revoked_keys`
29. `revoke_instance_writes_a_revoked_placements_row`
30. `a_renewal_pass_skips_a_revoked_placement_even_when_near_expiry`
31. `revoke_instance_takes_the_instance_lock_for_the_whole_call` — asserts a
    concurrent resident pass for the same instance blocks until it releases
32. **[e2e]** `revoked_instance_key_handshake_fails_while_a_fresh_one_
    verifies` — the automation half of matrix row 14, exercised through the
    real `revoke-instance` verb rather than `handshake.rs`'s existing
    `MockResolver` mutation, closing the row's "mechanism with no trigger
    outside tests" gap

**Tests added in review (F-A5d-1, F-A5d-2, F-A5d-3, F-A5d-5, F-A5d-6, F-A5d-7):**

33. `renew_cert_is_refused_for_a_service_id_with_no_recorded_deploy_facts`
    (D-A5d-19, §26.19)
34. `renew_cert_mirrors_the_deploy_call_sites_service_proxy_and_row_
    authorizer` (D-A5d-18a, §26.18) — asserts a call routed through the
    rebuilt native service resolves an FDAE-gated relation the same as
    immediately after a real `deploy`
35. `renew_cert_aborts_on_a_stored_fdae_policy_that_fails_to_reparse` —
    never installs the new certificate, never touches `native_dispatch`
    (D-A5d-18b, §26.18)
36. `every_alert_kind_round_trips_through_display_and_from_str` — extended
    with `InstanceRevoked` alongside `VaultLocked`, not a new test (D-A5d-15)
37. `renewal_mints_at_renewed_cert_expires_hours_not_the_deploy_default` —
    asserts the renewed certificate's lifetime is `renewed_cert_expires_
    hours` (4h default), strictly shorter than the prior
    `INSTANCE_CERT_EXPIRES_HOURS` (24h) (D-A5d-16, §26.16)
38. `a_vault_error_locked_race_during_mint_raises_vault_locked_not_
    certificate_near_expiry` — the defensive per-call path, distinct from
    test 19's up-front `kek_is_loaded()` path, both landing on the same
    alert kind (D-A5d-17, §26.17)
39. `a_submit_of_the_same_plan_does_not_recertify_a_revoked_placement` —
    the gap F-A5d-1 found: `submit`, not just the loop, must respect
    `revoked_placements` (D-A5d-15, §26.15)
40. `a_force_reconcile_does_not_recertify_a_revoked_placement_and_raises_
    instance_revoked_for_the_rest_of_the_plan` — `force-reconcile`'s
    unfiltered-plan path, the second re-mint route F-A5d-1 found, with the
    rest of the plan's services still reconciled normally

**Test count: 32 → 40.**

---

## §30 — Docs and backlog for A5d

**Docs**

- `docs/developer-guide.md` — the `[roles.supervisor]` block gains
  `master_anchor_refresh_interval_secs` and `renewed_cert_expires_hours`
  (default 4h, D-A5d-16); the online-key posture's actual certificate
  lifetime under automation (short, renewal-driven) versus the 30-day
  backstop (D-A5d-7) explained as two different numbers for two different
  purposes; `roymctl supervisor revoke-instance` documented beside
  `export-master`/`import-master`, including that it also stops that one
  placement's renewal **on every write path — the loop, `submit`, and
  `force-reconcile` alike (D-A5d-15)** — not the whole instance.
- `task.md` — the A5d bullet corrected: renewal needs a new `renew-cert` verb
  (not a reuse of `deploy`), and matrix rows 1/3's automation half and row
  14's second half are closed by phases 3 and 5 respectively, each with its
  test named.
- `status.md` — an A5d section in the A0-A5c shape.
- ADR-0020 — a third amendment, the same shape as the 2026-08-01 one: §3's
  "issues and renews unattended" now has a concrete mechanism
  (`renew-cert`, §26.1) instead of reusing `deploy`; §4's custody section gains
  a note that master-anchor refresh's read-modify-write race is closed by the
  same single-writer fact custody already established (§26.8), not a new
  guarantee.
- `docs/planning/deferred-backlog.md` — the row at line 81
  ("`SynSvcNativeService`... must also refresh") resolved by D-A5d-3; line 86
  (max lifetime) resolved by D-A5d-7 with the 30-day number recorded, not left
  `TBD`; line 88 (revocation surface) resolved by D-A5d-10; line 89
  (master-anchor refresh race/schedule) resolved by D-A5d-8.

**Backlog rows resolved**

- *"A renewal that installs a certificate outside `deploy` must also refresh
  `SynSvcNativeService`"* — D-A5d-3.
- *"No maximum lifetime enforced on an installed instance certificate"* —
  D-A5d-7, with the number (30 days) recorded.
- *"No operator surface for revoking an instance key"* — D-A5d-10, closed
  end-to-end (not only for the loop) by D-A5d-15.
- *"Master-anchor refresh is a read-modify-write with a race, and a daily
  operator duty nothing performs on a schedule"* — D-A5d-8, both halves: the
  schedule (per-pass overdue check against a persisted fact) and the race
  (single-writer invariant, not a CAS).

**Backlog rows to add**

- ***A redundant supervisor holding one master via `import-master` on two live
  processes has no compare-and-set for master-anchor refresh*** (§26.8) — the
  single-writer invariant D-A5d-8 relies on holds for the topology this tree
  supports today (one supervisor per master), not for a future
  redundant-supervisor deployment. Needs the registry
  (`crates/community_registry`) to grow real compare-and-set, or an explicit
  single-writer lease. → **post-M5**, pairs with A6's own single-writer cron
  lease note in `task.md`'s A6 section, since both are instances of the same
  "more than one supervisor for one thing" problem.
- ***`revoke-instance` has no path back*** — once a placement gets a row in
  `revoked_placements` (D-A5d-15), nothing removes it; an operator who
  revoked a key by mistake, or wants to bring a member back under management
  after replacing its instance key by hand, has no verb for it. The table
  gives a concrete place an "un-revoke" would write to, but A5d does not build
  the write. → **TBD**, needs a decided semantics for "un-revoke" (and for
  whether it also needs to remove the DID from the anchor's `revoked_keys`,
  which is a separate, harder read-modify-write than clearing one local row)
  that A5d does not need to invent to close matrix row 14.
- Whatever A5e's own `§0` pass finds.

---

## §31 — Review response (2026-08-02)

An independent review of Part V spot-checked 38 of its `file:line` claims
(against the commit-`e2583cd` baseline — an unrelated, concurrent
fix-verification pass was landing in `orchestration.rs` while the review ran,
which produced transient false-positive mismatches the reviewer re-verified
away rather than reported as real). **Zero citations were wrong**; two were
imprecise by the doc's own looser convention (a struct-open line versus its
field line, a function-signature line versus the read inside it) but not
incorrect. Every substantive problem the review found was in the
**decisions**, not the citations. Seven findings: **one blocking, four
should-fix, two minor**. All seven incorporated.

| # | Finding | Disposition |
|---|---|---|
| **F-A5d-1** | *(blocking)* D-A5d-10's revocation-exclusion only stopped the loop's own renewal work-list. `handle_submit`/`deploy_submission` and `handle_force_reconcile` both call `apply_with_clients` with the full, unfiltered plan, which calls `certify_placed_members` unconditionally for every service in it — so an ordinary `submit` or `force-reconcile` after a `revoke-instance` silently re-minted and reinstalled the revoked key, with no error and no alert | **Incorporated.** §26.15, **D-A5d-15**: a persisted `revoked_placements` table, read by *every* caller that can mint a certificate — the loop, `submit`, and `force-reconcile` alike — not only the renewal work-list. A new `AlertKind::InstanceRevoked` names the skip. Tests 39-40 added. The review's own judgment call — that this makes phase 5 more load-bearing, not less — is taken; §28's "could ship separately" recommendation is withdrawn |
| **F-A5d-2** | *(should-fix)* Neither §26.7 nor D-A5d-7 named the certificate lifetime the renewal loop's own `certify_instance` call should request. The supervisor's existing `INSTANCE_CERT_EXPIRES_HOURS` constant is 24h — the deploy-time default — and its own doc comment already flags it as needing to change once A5d ships, untouched by the original pass | **Incorporated** as §26.16, **D-A5d-16**: `SupervisorRole::renewed_cert_expires_hours` (default 4h), replacing the 24h constant for both the initial mint and every renewal, so a managed instance has one lifetime throughout its life rather than a confusing 24h-then-4h split. Test 37 added |
| **F-A5d-3** | *(should-fix)* The two vault-lock detection paths (the up-front `kek_is_loaded()` check and the defensive per-call `VaultError::Locked` race) landed on two different `AlertKind`s — `VaultLocked` from the first, the generic `CertificateNearExpiry` per-step handler from the second — for the identical root cause, undercutting §26.4's own stated purpose of making the vault-locked gap visible in one place | **Incorporated** as §26.17, **D-A5d-17**: `VaultError::Locked`, caught anywhere in the mint/install step, always raises `VaultLocked`, carved out from D-A5d-13's generic per-step handling. Test 38 added |
| **F-A5d-4** | *(should-fix)* D-A5d-12's formula (`{placed members near expiry} \ needs_work`) was stated once in §27 and never revised even though §28's phase 5 already implied a third exclusion term, leaving the decisions table — this document's own single source of truth, by its established convention — silently out of date the moment phase 5 landed | **Incorporated.** D-A5d-12 revised in place to `\ needs_work \ revoked_placements`, cross-referencing D-A5d-15 |
| **F-A5d-5** | *(minor)* §26.3's enumeration of `SynSvcNativeService::new`'s non-certificate inputs omitted two of its eleven parameters, `service_proxy`/`row_authorizer` — both `self`-reachable the same way as the rest, so D-A5d-3's conclusion held, but the specific list was incomplete enough that an implementer following only the enumeration rather than the real call site would wire a native service with dead proxy/authorizer hooks | **Incorporated** as part of §26.18, **D-A5d-18**: `renew_cert_impl` mirrors the whole call site rather than an enumerated subset. Test 34 added |
| **F-A5d-6** | *(minor)* Neither §26.3 nor D-A5d-3 said what happens if a stored FDAE policy fails to re-parse — silently falling back to `fdae_policy: None` would be a silent enforcement bypass for the renewed instance, a materially different (and worse) failure mode than deploy's, which fails the whole call on a bad document | **Incorporated** as the other half of §26.18/D-A5d-18: a re-parse failure aborts `renew_cert_impl` before installing anything, matching deploy's own behavior. Test 35 added |
| **F-A5d-7** | *(minor)* D-A5d-1's `renew-cert` gating mirrored `restart_impl`'s capability, owner, and generation checks but not its implicit "is this actually deployed" gate (`deploy_facts` must exist) — a capability-holding caller could otherwise construct a live native-dispatch entry for a `service_id` that was never deployed | **Incorporated** as §26.19, **D-A5d-19**: `renew_cert_impl` requires `deploy_facts(&service_id).is_some()`, the same signal `restart_impl` already relies on. Test 33 added |

**One judgment the review offered that this pass takes without modification:**
the recommendation to withdraw §28's "phase 5 ships separately" call. The
first draft reasoned from matrix row 14's *property* (already proven by A0's
test) to the operator-surface half being the least load-bearing item in the
slice; F-A5d-1 shows the property the *operator* actually needs —
revocation that survives an ordinary redeploy — was not proven by anything in
the original scope. Recorded in §28 rather than only here, since that is
where an implementer reads it.

**Test count: 32 → 40**, all eight added tests traceable to a specific
finding above.
