# Slice S2 Implementation Plan — Tier 2: the Signed Topology Document

**Status:** ✅ Complete (2026-08-08). Milestone:
[task.md](task.md) slice **S2**; milestone-level plan:
[implementation-plan.md](implementation-plan.md). Design of record:
[ADR-0022](../../../decisions/0022-two-tier-logical-service-discovery.md) §3,
§4, §5, §6. Depended on **S1 — Complete 2026-08-08**. Gates S3 and S4.
Verification evidence lives in [status.md](status.md).

**The one-sentence summary.** S2 makes "who are the members of this app's
logical service" an answerable question for a caller *outside* the app, by
having the supervisor sign a topology document per logical service, serving it
over a new `resolve` RPC, and giving the caller a verify-and-cache path that
routes without another network call.

**Read the milestone plan's §0 and §1 first.** §0.4 (the `LogicalResolver`
foreign-entry collision) is assigned to this slice by name and is settled in
§1 below as **D-S2-1**. §0.2 (the locked vault) binds this slice a second time:
S1's consequence was a record that decays; S2's is an RPC that cannot answer.

---

## §0 — What ADR-0022, task.md, and the shipped tree leave open, understate, or state wrongly

Eight findings. Three change the slice's scope; two are contradictions between
the ADR and the tree; three are ambiguities the ADR never had to resolve
because nothing had implemented §3 yet.

**Revised 2026-08-08 after plan review**, which found eight gaps, all
confirmed against the tree and all incorporated: two missing production call
sites (§3 phase 1), the undeclared narrowing of ADR-0022 §3's substrate-side
fetch requirement (new §0.8), a resource shape that was both node-bound and
colliding with an existing selector key space (§0.5, rewritten), a validation
placed after a durable write (§3 phase 3), pre-S2 instances stuck at epoch 0
(D-S2-4), `TopologyEntry.epoch` silently carrying two counters (D-S2-2), a
`cache_ttl_ms` the fetch trait discarded (D-S2-6), and five coverage gaps
against task.md's exit criteria (§4, §5, §7).

**Revised again after a second review round**, which found two defects the
first revision itself introduced, both confirmed and both fixed: the
`resolve`-path epoch write was specified as the *advancing* form, which
without the instance lock lets a concurrent `submit` move the epoch with no
membership change — the property §0.1 exists to establish (D-S2-4, now
insert-only with a stale-plan guard); and the WIT `sharding-strategy-json`
field matched neither the Rust field's name nor its type, so test 45 as
specified would have failed against this plan's own design (§3 phase 3, now
`sharding-strategy`). Two traceability corrections came with them: budget 4 is
S1's, not test 43's, and `covers_resource` serves S4's per-service narrowing
only once S4 also moves the *checked* resource to the selector-bearing form.

**Third round** raised one non-defect that turned out to be a defect. The WIT
field's `option<string>` is not literally true of `ShardingStrategy`'s
`range_sharding` variant, which serializes as an object -- offered as an
accepted trade, since manifest validation refuses that variant. It does not:
`SynAppManifest::validate` is compile-time only, and `submit`/`force-reconcile`
take an already-compiled plan that nothing re-checks -- the gap
`refuse_replicas_above_cap` and `refuse_unrunnable_schedules` already exist to
close for two other manifest rules. A hand-authored plan could therefore have
put a range table into a **signed** document. Closed by a third sibling
(D-S2-15) rather than annotated, which makes the WIT declaration a checked
property instead of a comment asking future readers to be careful.

**Post-merge code review (2026-08-09)** found five defects in the shipped
code the three planning rounds above did not catch, all fixed in place
rather than re-planned:

- `handle_resolve`'s document cache was keyed `(app_instance_id,
  service_name)` with no check binding the cached document to the app DID
  actually being resolved, so an in-place handover (`import-master` +
  `adopt`, unchanged membership) could serve a document signed by the
  *previous* master to a caller resolving the *new* DID -- a document that
  fails `verify` at every caller for up to half of `not_after`. The same
  cache hit condition also never compared `generation`, which `adopt` can
  advance with no epoch change, so a cached document's `generation` could
  read stale. Both are now part of the cache hit condition, not the key.
- D-S2-4's insert-only claim for `handle_resolve` ("can never advance
  [the epoch]") is now true only of its **fast path**. Two lock-free
  attempts landing on a fingerprint mismatch could mean a `submit` is
  genuinely still in flight (a transient, and previously fatal, race) or
  that an earlier `submit`'s best-effort fingerprint write never landed,
  which insert-only can never repair on its own -- contrary to the comment
  claiming it would. `handle_resolve` now falls back to taking the
  per-instance async lock and using the advancing form once, exactly the
  repair `record_topology_fingerprint`'s own doc comment describes; it is
  never held on the common path.
- `RegistryClient::lookup`'s HTTP branch verified a returned
  `SignedEndpointInfo` against *its own* embedded `service_id`, never
  against the DID the lookup actually asked for -- the DHT branch a few
  lines below already guards this (`extract_verified_endpoint_from_packet`),
  the HTTP branch did not. A compromised or malicious registry could
  answer a Tier-1 lookup for app A with any other party's validly
  self-signed record, redirecting `RegistryTopologyFetcher` to a
  supervisor of the attacker's choosing (and presenting the caller's UCAN
  to it). Now checked, gated on `id` being a full DID so a shorthash-alias
  lookup -- which cannot be checked this way by construction -- is
  unaffected.
- `TopologyBuildError::NoSuchService` (caller input) and
  `InconsistentPlan` (a compiler defect) both mapped to `InternalError` in
  `handle_resolve`; only the former now maps to `InvalidParams`.
- `service_topology` refused members disagreeing on mode or sharding
  strategy but not two members sharing one `member_index` -- silently
  order-dependent on `plan.services`'s own order rather than refused like
  every other disagreement in that function. Now a third `InconsistentPlan`
  case.

Three lower-severity gaps were also closed: `resolve` was missing from
`every_verb_is_refused_without_substrate_admin`'s hand-maintained list; a
configured `0` for `topology_document_not_after_secs`, or a `cache_ttl`
at or above half of it, went unvalidated (both now clamped with a
warning, the same shape `max_renewals_per_pass` already uses); `AppDid`
permitted `/` and `#`, unlike its two siblings, despite being interpolated
into a `synapp:<app-did>` `ResourceUri`. One finding was investigated and
declined as a code change: `cache_ttl`'s "on expiry try to refresh" has no
implementation anywhere yet (not a regression introduced by any of the
above), recorded instead as a sharper backlog row rather than built ahead
of a scheduled refresher nothing calls yet.

### 0.1 (Correctness, and it is this slice's sharpest finding) The topology epoch ADR-0022 §6 says "already exists" does not exist

ADR-0022 §6:

> "The counter already exists: the supervisor stamps per-dependent binding
> epochs and compares written against observed to judge convergence."

That counter is
[`binding_epochs`](../../../../crates/app_supervisor/src/store.rs), keyed
`(app_instance_id, logical_ref)` where `logical_ref` is the **dependent
member's** `MemberRef` display string
([service.rs:3536](../../../../crates/app_supervisor/src/service.rs#L3536)).
It counts *writes pushed to one dependent*. It is advanced once per push
attempt, and a `Stale` outcome jumps it to `held + 1` — it moves for reasons
that have nothing to do with a member set changing, and it does not move when
a member set changes for a service that no dependent declares.

What ADR-0022 §3's document needs, and what S5 will fence the data path on, is
a **per-logical-service** counter that changes when and only when
`(mode, members, sharding_strategy)` changes. Reusing the per-dependent one
would give a document whose `epoch` increments on unrelated retries and stays
flat across a genuine scale-out — the exact inverse of what the reference
scenario's step 6 asserts.

**Consequence, scope-changing:** S2 adds a real per-logical-service epoch
(D-S2-4), a new store table, and its bump point at `submit`. This is the one
part of the slice that is not "wire up what exists".

### 0.2 (Scope-changing) The document cannot carry a `sharding_strategy`, because the supervisor never sees one

S1 put `sharding_strategy` on
[`ServiceSpec`](../../../../crates/app_orchestration/src/models.rs#L551) — the
*manifest* type. The supervisor holds no manifest and never reads one
(`supervisor.wit`'s own `submission` doc says so): it holds a compiled
`DeploymentPlan`, and
[`PlannedService`](../../../../crates/app_orchestration/src/models.rs#L776) has
no `sharding_strategy` field. `compiler.rs` never copies one across.

So a document built from the stored plan can only ever carry `None`. Both S1's
plan §0.5 and the milestone plan §0.5 assumed the manifest surface was the
whole surface; it is half of it.

**Resolved in favour of finishing the path**: `PlannedService` gains
`sharding_strategy`, cloned from `ServiceSpec` by the compiler, exactly as
`schedule` already is ([models.rs:803](../../../../crates/app_orchestration/src/models.rs#L803)
— "Cloned from `ServiceSpec.schedule` … every member of a scaled scheduled
service carries the identical spec"). Same `#[serde(default,
skip_serializing_if = "Option::is_none")]`, so an existing plan's JSON is
byte-for-byte unchanged. Ship-before-enforce (milestone D-C-4) is about no
*consumer*, not about a field that cannot physically be populated.

### 0.3 (Correctness) The document "feeds straight into `LogicalResolver::register`" — but `TopologyEntry` cannot express the one caching rule the ADR gives

ADR-0022 §3:

> "Cache with TTL; on expiry try to refresh; if the refresh fails, keep using
> the previous document until `not_after`; past `not_after`, fail.
> `LogicalResolver` already has the TTL, epoch, and explicit-eviction
> machinery this needs, and the document feeds straight into
> `LogicalResolver::register`."

It has the TTL and the eviction. It has no `not_after`.
[`TopologyEntry`](../../../../crates/app_orchestration/src/resolver.rs#L231)
carries `mode`, `members`, `sharding_strategy`, `epoch`, `cache_ttl` — and the
`cache_ttl` governs the *front cache only*. On expiry `get_topology` re-reads
the `AppRegistry` entry, which never expires. So a foreign document registered
today would resolve forever, including long past `not_after`, which is
failure-matrix row 6's exact prohibition ("Fails. Not 'stale but usable'").

**Resolved:** `TopologyEntry` gains `not_after: Option<u64>` (unix seconds),
`#[serde(default)]`, `None` meaning "no expiry" — which is every binding
written by the intra-app push path and therefore no behavior change there. The
resolver refuses an expired entry (D-S2-2). The two-level structure then maps
onto the ADR's rule exactly: `cache_ttl` is "when to try to refresh",
`not_after` is "when to stop answering".

### 0.4 (Correctness, milestone §0.4 assigned here) Keying foreign entries by app DID is right, and it moves the rendezvous domain separator

Milestone plan §0.4 states the collision and names the natural fix — key
foreign entries by app DID. That fix is correct and is taken (D-S2-1). What it
does not mention is the second-order effect.

[`rendezvous_select`](../../../../crates/app_orchestration/src/resolver.rs#L443)
takes `app_instance_id` and `service_name` as the hash's domain separator, fed
from `logical_ref.app_instance_id.as_str().as_bytes()`
([resolver.rs:659](../../../../crates/app_orchestration/src/resolver.rs#L659)).
If a foreign entry's key is the app DID, that string becomes the separator for
foreign callers while intra-app callers keep using the human instance id. Two
callers of the *same* `Sharded` service then hash the same routing key to
different members.

**Not reachable today**, on three independent grounds: `TopologyMode::Sharded`
is compiled by nothing (backlog §8, still open after S1), `Redundant`'s keyed
selection is load balancing rather than correctness, and no cross-app caller
exists until S3/S4. **It becomes reachable exactly when S5 enforces the epoch
fence**, which is the point at which "wrong member" stops being a slow answer
and becomes a wrong shard.

**Resolved:** taken as a documented divergence with a backlog row targeted at
S5, not fixed here. The eventual fix is one canonical separator everywhere —
the app DID — which requires the intra-app push path to learn the app DID
(a field on the wire `dependency-binding` record). Adding that now would put a
field on a wire record for a consumer two milestones away, with no way to test
that the two paths agree until that consumer exists. Recorded in D-S2-1 and as
a backlog row so S5 does not rediscover it.

### 0.5 (Ambiguous, and the docs disagree with each other) Who may call `resolve` is S2's problem, and task.md gives it to S4

- **ADR-0022 §5** says visibility is declared per logical service in the
  submitted plan: open to all, or requiring a UCAN.
- **task.md's slice table** puts "UCAN-scoped per-service exposure declared in
  the submitted plan" in **S4**, whose own gate is "a first real cross-app
  dependency exists" (D-C-7) — possibly never, on a schedule.
- **task.md's failure matrix row 7** ("A caller not authorized for a logical
  service fetches its document → clean denial, **never** a filtered member
  list") goes live the moment `resolve` exists, which is S2.
- **The shipped tree** gates every supervisor verb behind
  `require_admin` — `substrate/admin` on the supervisor's own node
  ([service.rs:2685](../../../../crates/app_supervisor/src/service.rs#L2685)).
  Left as-is, `resolve` is unreachable by any caller outside the app and the
  milestone's goal is not demonstrated at all.

So S2 must pick an interim rule. Three options:

| Option | Cost |
|---|---|
| Keep `require_admin` | S2 ships a verb only the node owner can call. The milestone goal — "a caller outside an app instance resolves that app's logical service" — is not met, and the reference scenario's step 3 cannot be written honestly |
| Open to any authenticated caller | Matches ADR §5's "open to all" branch and needs no new machinery, but makes every managed app's full topology readable by anyone who can dial the node, with no operator control and no way to opt out until S4. A default that leaks is a bad default even when a later slice can change it |
| A capability check now, the *declaration* in S4 | One new ability, one resource shape (both reusing existing machinery), and matrix row 7 gets a real test in the slice that creates the hazard. S4 adds the manifest declaration that can widen a service to "open to all" |

**Recommended and taken: the third** (D-S2-7). It is the reading of ADR §5's
"in the same manner as any other service's access control" that the tree
already implements everywhere else, it makes the default closed rather than
open, and it splits S4's row cleanly: S2 owns the *check*, S4 owns the
*declaration* that relaxes it. This is a reassignment of half of task.md's S4
row and is called out here rather than left for review to find.

**The resource this check names is `synapp:<app-did>`, and the first draft of
this plan got it wrong twice** (both found in review):

- `substrate:<node-did>/app/<name>` **already means something else.** The
  `app/` selector holds a **`service_id`**, not an app instance id —
  `require_service_ability`
  ([orchestration.rs:930](../../../../crates/control_plane/src/service/orchestration.rs#L930))
  builds `substrate:{node}/app/{service_id}`, and `owning_service_id`
  ([io.rs:109](../../../../crates/router/src/route_handler/io.rs#L109)) parses
  it back out as one for the owner-rooted trust check. Putting an
  `app_instance_id` in the same slot puts two key spaces under one selector,
  and `owning_service_id` would then hand an app instance id to a
  `registry.owner_of(service_id)` lookup.
- **It is node-bound, which is the exact failure ADR-0022 §5 named.** §5 chose
  submitted-plan declaration over node config because node config is "neither
  reproducible nor able to survive a handover". A resource rooted in the
  supervising node's DID has the same defect: every grant issued against it
  is meaningless the moment `export-master`/`import-master` move the instance.

`synapp:<app-did>` fixes both. It is a new base under the existing `synapp:`
prefix, so it collides with nothing (`owning_service_id` returns `None` for a
base with no `:svc:` segment), and `resource_is_local`
([io.rs:94](../../../../crates/router/src/route_handler/io.rs#L94)) treats
every `synapp:` resource as local by construction — so the same resource
string evaluates correctly at whichever node holds the app after a handover.
S4's per-logical-service narrowing is then a `/service/<name>` selector on
that base — but **`covers_resource` serves that narrowing only if S4 also
moves the checked resource to the selector-bearing form.** Its rule is that a
selector on the *held* capability with none on the *requested* resource is not
covered, so an S4 grant narrowed to `synapp:<did>/service/orders` will
**fail** S2's check against the bare `synapp:<did>`. That is the correct
direction to fail in — a narrowed grant must not satisfy a broader ask — but
it means S4 has one line to change on the check side, not zero. Said here so
S4 does not read "the existing rule already serves it" as "no change needed".

**What this still does not fix, and the reason is not the resource shape.** A
grant surviving a handover needs its *trust root* to survive too, and the root
is the node's `admin_ucan_root` — a chain issued by node A's owner is not a
trusted root at node B. The root that would survive is the **app master
itself**, the app-level analogue of ADR-0015 A6's per-service `owner_of` root.
Nothing can issue such a chain today: the app master lives only in the
supervisor's vault (ADR-0022 §1 — the registry's keying rule puts it there),
and there is no verb to delegate from it, nor should one be added casually to
an interface whose own test asserts no verb touches key material. So at S2 a
`supervisor/resolve` grant is node-admin-rooted and **must be re-issued after
a handover**. Recorded as a backlog row (§5) with that named cause, targeted
at S4, which is where handover-survivability is actually exercised.

### 0.6 (Ambiguous) Signing per request puts a latency surface on the key-holding process, which is what §3 set out to avoid

ADR-0022 §3's third reason for choosing a document is:

> "It keeps a latency-sensitive surface off the process holding every master
> key."

Building and signing a fresh document inside every `resolve` call does the
opposite at the supervisor's own hop: a vault read plus an Ed25519 signature,
under the per-request path, on the process that holds every member master and
the app master. It also means a locked vault fails *every* `resolve`, not just
the ones that need a new signature — the availability property §3 chose the
document form for, lost at exactly the moment (a supervisor restart) it is
most wanted.

**Resolved:** the supervisor signs once per `(service, epoch)` and serves the
stored copy afterwards (D-S2-6). In-process, not persisted — the same choice
`last_reconciled` makes, and for the same reason: after a restart the vault is
locked anyway, so a persisted copy would buy one signature and a durability
question. A cached document is re-signed when its remaining validity drops
below half, so a served copy always outlives the caller's own cache TTL.

### 0.7 (Understated) Two things ADR-0022 §3 mentions that this slice deliberately does not build

- **`shard_map` as a separate document field.** §3's schema lists
  `sharding_strategy` *and* `shard_map`.
  [`ShardingStrategy::RangeSharding(RangeRoutingTable)`](../../../../crates/app_orchestration/src/resolver.rs#L174)
  already carries the map inside the strategy, and
  `TopologyEntry`/`ResolvedTopology`/`select_member` all read it from there.
  A second, parallel field would give a reader two sources that can disagree
  inside one signed payload. **One field** (D-S2-3). Additionally moot in
  practice: S1 refuses `RangeSharding` at manifest validation.
- **MQTT epoch-bump publication for early invalidation.** §3 says a
  supervisor could publish epoch bumps on its existing alert topic so a
  subscriber drops a cached document before its TTL. Nothing in task.md's
  budgets, exit criteria, or failure matrix depends on it — it is a latency
  optimisation over an already-correct TTL path, and it needs a subscriber,
  which nothing is until S3. **Out of S2**, with a backlog row (D-S2-10).

### 0.8 (Scope-narrowing, and it must be declared) ADR-0022 §3 requires the *substrate* to fetch, and after S2 nothing on the substrate does

§3 ends with a named requirement:

> "Resolution happens substrate-side, not in the guest. The host capability
> already carries the resolver … The substrate fetches, verifies, and
> registers a foreign app's document there, so no guest re-implements Tier 1
> and Tier 2 for itself."

S2 ships the pieces — `TopologyFetcher`, `RegistryTopologyFetcher`,
`register_verified` — and two production callers of them: the
`roymctl app resolve` command and any program using the SDK. It does **not**
ship a substrate-side caller. `runtime.rs` constructs no fetcher and holds
none; `host_capabilities.rs` is touched only for the key rename; a guest's
`CallTarget::Dependency` naming a foreign app still fails exactly as it does
today.

**That is a real narrowing against the design of record, and it is
deliberate**, for the reason D-C-7 already gives about S4: there is nothing to
trigger the fetch from. A guest reaches a dependency by *declared name*
([host_capabilities.rs:1132](../../../../crates/sandbox_wasm/src/host_capabilities.rs#L1132)),
and the host maps that name through a stored app-context row that
`prepare_binding` refuses to write for a different app instance
([orchestration.rs:293](../../../../crates/control_plane/src/service/orchestration.rs#L293)).
Until S4 replaces that refusal with an authorization check, no declared
dependency can *name* a foreign app, so a fetcher wired into `runtime.rs`
would be reachable by nothing. Wiring it anyway would be dead code carrying an
`Arc` through four constructors.

**Consequences, both recorded rather than left implicit:** a backlog row (§5)
targeted at S3/S4, and an entry in §6's "what this does not close". The
performance budget this touches is handled in §4's note on test 53 — the first
draft's version of that test could not have failed, since no resolve path
could reach a fetcher at all.

---

## §1 — S2 decisions

| ID | Decision |
|---|---|
| **D-S2-1** | **The resolver's key gains an explicit scope: `AppScope::Local(AppInstanceId)` \| `AppScope::Foreign(AppDid)`** (§0.4, milestone D-C-6). The two namespaces become disjoint *by type*, not by a naming convention a future caller can violate. The rendezvous domain separator moves with the key for foreign entries; the resulting local/foreign divergence is documented and recorded as a backlog row targeted at S5, not fixed here. |
| **D-S2-2** | **`TopologyEntry` gains `not_after: Option<u64>`** (§0.3), `#[serde(default)]`, `None` = no expiry. `LogicalResolver::get_topology` refuses an expired entry, on the cache-hit path as well as the registry path. Excluded from `classify_binding_write`'s content comparison, for the same reason `cache_ttl` already is: it is a policy value, not a disagreement about who serves the service. **`TopologyEntry.epoch` now carries two different counters** — the per-dependent binding epoch on a local entry, the per-logical-service topology epoch on a foreign one (§0.1 spends its length proving these are not the same thing). They never meet because D-S2-1's keys are disjoint, so the separation rests *entirely* on `AppScope`, and that must be said in a doc comment on the field rather than inferred. It is also the thing S5 will trip on: enforcing the epoch fence on the data path means reading this field without knowing which counter produced it. |
| **D-S2-3** | **One strategy field, no separate `shard_map`** (§0.7). `ShardingStrategy` already carries the range table. |
| **D-S2-4** | **A per-logical-service topology epoch, advanced only by a real membership change, and advanced only under the instance lock** (§0.1). Stored as `(app_instance_id, service_name) -> (epoch, fingerprint)`; the fingerprint is a BLAKE3 hash of `(mode, ordered members, sharding_strategy)`. **Two entry points, and the difference is the whole safety argument.** `submit` holds the instance lock ([service.rs:3770](../../../../crates/app_supervisor/src/service.rs#L3770)) and uses the **advancing** form. `resolve` deliberately holds no lock, so it uses an **insert-only** form (`ON CONFLICT DO NOTHING`, then read back) that can initialise a missing row — every instance submitted before this slice has none, and a bare read would sign it at epoch `0` forever — but can never advance one. An advancing call from an unlocked reader is not safe, and the first revision of this plan wrongly claimed it was: `resolve` reads the plan and writes the fingerprint in two steps, so a `submit` landing between them makes the reader record a fingerprint for the *previous* plan, which then bumps the epoch on the next read and again on a genuinely no-op resubmit — the epoch moving without a membership change, which is exactly what §0.1 exists to prevent and what test 35 asserts against. Insert-only removes the write, and therefore the race, without taking a lock on the fetch path. **Rows are never deleted**, so a service removed and re-added later never reuses a lower epoch — a fence that goes backwards is not a fence. |
| **D-S2-5** | **`resolve` takes the app DID, not the app instance id.** Tier 1 answers with a DID (ADR-0022 §1: the DID is the network identity, `app_instance_id` is only the human name), so requiring the human name would make the chain unfollowable. The supervisor looks the instance up by `desired_state.app_master_did`. |
| **D-S2-6** | **The supervisor signs once per `(service, epoch)` and caches the signed document in process** (§0.6), re-signing when less than half its validity remains. Not persisted. **`cache_ttl_ms` is inside the signed document**, not a sibling field on the response: the fetch trait returns a `SignedTopologyDocument`, so a TTL carried outside it is discarded before any caller can read it — the first draft's `fetch_and_register` could not have been written. A reader that wants its own TTL passes an override to `to_topology_entry`, which is where the "the reader owns `cache_ttl`" property actually belongs. |
| **D-S2-7** | **`resolve` is authorized by `supervisor/resolve` on `synapp:<app-did>`** (§0.5), which a bare `substrate:<node>` `substrate/admin` grant covers (`Capability::grants` short-circuits on `is_substrate_scope`), so the node owner keeps working unchanged. Deliberately **not** `substrate:<node>/app/<id>`: that selector already holds a `service_id`, and it is node-bound, which is the handover failure ADR-0022 §5 named. The grant is still node-admin-*rooted* at S2 and must be re-issued after a handover — backlog row, §5. Unknown app and unauthorized caller return the **same** error, so a caller with no grant cannot probe for an app's existence — A4-10's rule, already applied by `app_instance_management_of`. S4 adds the manifest declaration that can widen a service to "open to all". |
| **D-S2-8** | **A denial is total.** There is no code path that filters a member list — the document is built whole or not at all, and the authorization check runs before the document is built (ADR-0022 §5, matrix row 7). Pinned by a test that asserts the refusal carries no member DIDs at all. |
| **D-S2-9** | **`PlannedService` gains `sharding_strategy`, cloned from `ServiceSpec` by the compiler** (§0.2), in `schedule`'s exact shape. |
| **D-S2-10** | **No MQTT epoch-bump publication in S2** (§0.7). Backlog row; S3 owns it if a subscriber appears. |
| **D-S2-11** | **The document type and the verify path live in `syneroym-app-orchestration`; the network fetch lives in `syneroym-sdk`, behind a trait declared in `app_orchestration`.** `control_plane` cannot depend on `sdk` (`sdk → router → control_plane` is a cycle), and S4's consumer is in `control_plane` — so the verify half must sit below both, and the fetch half must be reachable as `Arc<dyn TopologyFetcher>` injected from `runtime.rs`. Getting this backwards is a rewrite in S4, not a refactor. |
| **D-S2-12** | **A locked vault fails `resolve` loudly**, naming `inject-kek`, and raises the existing `AlertKind::VaultLocked` (milestone D-C-2) — never a silent empty answer. An already-cached signed document is still served, since serving it needs no key. |
| **D-S2-13** | **No substrate-side fetch in S2** (§0.8). ADR-0022 §3's "the substrate fetches, verifies, and registers" is narrowed to "the SDK and `roymctl` do", because until S4 relaxes `prepare_binding`'s intra-app refusal no declared dependency can name a foreign app, so a fetcher held by `runtime.rs` would be reachable by nothing. Backlog row and a §6 entry, not a silent omission. |
| **D-S2-14** | **`resolve` answers for a paused instance and refuses for a retired one.** `pause` stops the resident loop touching an instance and nothing else (`supervisor.wit`); its members keep running and stay worth routing to, so refusing here would invent a second meaning for `pause`. Stated as a decision rather than left to fall out of the code, because S1 made `pause` load-bearing for Tier-1 decay and a reader will reasonably expect symmetry. `retired` is different in kind — the supervisor has stopped managing the instance, so its stored plan is no longer a claim about anything — and is folded into the same denial as "unknown app" (D-S2-7). |
| **D-S2-15** | **`submit`/`force-reconcile` re-check S1's two `sharding_strategy` rules against the compiled plan**, as a third sibling of `refuse_replicas_above_cap` and `refuse_unrunnable_schedules` ([service.rs:2810](../../../../crates/app_supervisor/src/service.rs#L2810) states the rule those two exist for, verbatim: manifest validation runs at compile time and "`submit`/`force-reconcile` take an already-compiled plan, which nothing between the compiler and here re-checks"). Without it, S1's refusal of `RangeSharding` is a *manifest* rule only, and a hand-authored plan puts a range table — naming member `ServiceId`s of its author's choosing — straight into a **signed** Tier-2 document. That is what makes the WIT field's `option<string>` true rather than merely documented: with both ends checked, no value that can reach the wire is anything but a bare string. |

---

## §2 — Phase plan

Four phases. Nothing is observable outside the node until phase 3.

1. **The resolver key and expiry** — D-S2-1, D-S2-2. Pure refactor plus one new
   rule, no new feature. Tests 19–23.
2. **The document type and the plan field** — D-S2-3, D-S2-9, D-S2-11's lower
   half. `app_orchestration` gains the `topology_document` module, the
   `TopologyFetcher` trait, and a `syneroym-identity` dependency. Tests 24–33.
3. **The supervisor side** — D-S2-4, D-S2-5, D-S2-6, D-S2-7, D-S2-8, D-S2-12,
   D-S2-14, D-S2-15. The epoch table and its two record points, the document builder
   and its cache, the `resolve` verb, the config fields. Tests 34–51.
4. **The client side, the CLI, and the e2e** — D-S2-11's upper half and
   D-S2-13's declared boundary, plus the reference scenario. Tests 52–59, docs
   and backlog.

**What could move:**

- **Phases 1 and 2 can merge.** Neither is reachable from the network.
- **Phase 1 cannot be deferred past phase 4.** Registering a foreign document
  under a key that can collide is the thing milestone §0.4 exists to prevent,
  and it would be invisible in every test that has only one app.
- **Phase 3's `resolve` cannot ship without phase 3's authorization.** They are
  the same commit: a verb that answers everyone, shipped "temporarily", is the
  leak §0.5 rejects.

---

## §3 — Exact changes

### Phase 1 — the resolver key and expiry

**`crates/app_orchestration/src/models.rs`**

Add, after `ServiceId`'s wrapper:

```rust
define_string_wrapper!(
    AppDid,
    "An app instance's own master DID (ADR-0022 §1) -- its network identity, \
     as distinct from `AppInstanceId`, its human name.",
    |s: &str| {
        if !s.starts_with("did:key:") {
            return Err(anyhow!("AppDid must start with 'did:key:'"));
        }
        Ok(())
    }
);
```

**`crates/app_orchestration/src/resolver.rs`**

New types, above `AppRegistry`:

```rust
/// Which app a topology entry belongs to (ADR-0022 §1, milestone plan §0.4).
///
/// `Local` is an app instance deployed through this node, keyed by the name
/// this node's own operator chose -- unique here by construction. `Foreign`
/// is another app's topology, learned from a verified Tier-2 document and
/// keyed by the app master DID, which is globally unique. Two unrelated apps
/// both called `chat` are two different keys, because they are two different
/// DIDs; keying both by the human name would silently re-point one at the
/// other's members.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum AppScope {
    Local(AppInstanceId),
    Foreign(AppDid),
}

impl AppScope {
    /// The bytes this scope contributes to `rendezvous_select`'s domain
    /// separator.
    ///
    /// Deliberately *not* canonical across the two variants: an intra-app
    /// caller separates by the instance id and a foreign caller by the app
    /// DID, so the two disagree about which member a routing key selects.
    /// Unreachable today (`Sharded` is compiled by nothing, `Redundant`'s
    /// keyed path is load balancing, and no cross-app caller exists), and
    /// it becomes reachable when shard rebalancing enforces the epoch fence
    /// on the data path. Fixing it needs one canonical separator -- the app
    /// DID -- which needs the intra-app push path to carry the app DID on
    /// the wire. Recorded in the deferred backlog rather than built against
    /// a consumer that does not exist.
    #[must_use]
    pub fn as_str(&self) -> &str { … }
}

/// The key of a topology entry: which app, and which logical service inside
/// it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TopologyKey {
    pub app: AppScope,
    pub service_name: LogicalServiceName,
}

impl TopologyKey {
    #[must_use]
    pub fn local(app_instance_id: AppInstanceId, service_name: LogicalServiceName) -> Self { … }
    #[must_use]
    pub fn foreign(app_did: AppDid, service_name: LogicalServiceName) -> Self { … }
}

impl fmt::Display for TopologyKey {
    // `<scope>/<service_name>`, matching `LogicalServiceRef`'s own form.
}
```

`AppRegistry`'s four methods change signature:

```rust
fn register(&self, key: TopologyKey, entry: TopologyEntry);
fn get(&self, key: &TopologyKey) -> Option<TopologyEntry>;
fn invalidate(&self, key: &TopologyKey);
fn list(&self, app: &AppScope) -> Vec<LogicalServiceName>;
```

`StaticInventory`'s inner map becomes `BTreeMap<TopologyKey, TopologyEntry>`;
`TopologyCache`'s becomes `DashMap<TopologyKey, CacheEntry>`.

`LogicalResolver`'s public methods change the same way:

```rust
pub fn resolve(&self, key: &TopologyKey, routing_key: Option<&[u8]>) -> Result<ServiceId>;
pub fn resolve_all(&self, key: &TopologyKey) -> Result<AllMembers>;
pub fn invalidate(&self, key: &TopologyKey);
pub fn register(&self, key: TopologyKey, entry: TopologyEntry);
```

`select_member` takes `key: &TopologyKey` instead of
`logical_ref: &LogicalServiceRef`, and passes `key.app.as_str().as_bytes()`
where it passed `logical_ref.app_instance_id.as_str().as_bytes()`.

`TopologyEntry` gains:

```rust
    /// Unix seconds after which this entry must stop resolving (ADR-0022 §3,
    /// failure-matrix row 6: past `not_after`, fail -- not "stale but
    /// usable"). `None` for an entry pushed by the intra-app binding path,
    /// which has no expiry and is refreshed by a later push.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_after: Option<u64>,
```

…and its existing `epoch` field gains the doc comment D-S2-2 requires, because
the field now carries two counters that are not comparable with each other:

```rust
    /// Which counter this is depends on the entry's `AppScope`, and nothing
    /// in the type says so:
    ///
    /// - under `AppScope::Local`, the **per-dependent binding epoch** the
    ///   supervisor advances on every push to one dependent
    ///   (`SupervisorStore::advance_binding_epoch`), which is what
    ///   `classify_binding_write` compares;
    /// - under `AppScope::Foreign`, the **per-logical-service topology
    ///   epoch** a Tier-2 document carries (ADR-0022 §6), which changes when
    ///   and only when a member set or mode does.
    ///
    /// They are never compared with each other only because the two scopes
    /// are disjoint keys -- the separation is `AppScope`'s, not this
    /// field's. Anything that later reads this epoch without knowing the
    /// scope (shard rebalancing's data-path fence is the one on the map) has
    /// to establish the scope first.
    pub epoch: TopologyEpoch,
```

`ResolvedTopology` gains the same `not_after: Option<u64>`, copied in
`get_topology`.

`get_topology` gains the expiry check, applied to **both** paths — a cache
entry whose `cache_ttl` outlives its `not_after` must not keep answering:

```
fn get_topology(key) -> Result<Arc<ResolvedTopology>> {
    now = unix_now()
    if let Some(resolved) = cache.get(key) {
        if resolved.is_expired(now) { cache.evict(key); return Err(expired_error(key, resolved)) }
        return Ok(resolved)
    }
    entry = registry.get(key).ok_or(not registered)?
    resolved = Arc::new(ResolvedTopology::from(entry, not_after))
    if resolved.is_expired(now) { return Err(expired_error(...)) }   // never cached
    cache.insert(key, resolved, entry.cache_ttl)
    Ok(resolved)
}
```

`expired_error` names the key and the expiry time, so a caller can tell
"expired" apart from "never registered" without matching on text — both are
`anyhow::Error` today and stay so; the wording is the contract with the tests,
as elsewhere in this module.

`classify_binding_write`'s `same` comparison is **unchanged** (it already lists
`mode`, `members`, `sharding_strategy` explicitly) — add a sentence to its doc
saying `not_after` is excluded for the same reason `cache_ttl` is.

`empty_resolver()` is unchanged.

**`crates/app_orchestration/src/lib.rs`** — re-export `AppScope`,
`TopologyKey`, and `AppDid`.

**Call sites to update (production):**

| File:line | Change |
|---|---|
| [runtime.rs:895](../../../../crates/substrate/src/runtime.rs#L895) (`replay_persisted_bindings`) | `app_registry.register(TopologyKey::local(instance_id, service_name), entry)` |
| [runtime.rs:1330](../../../../crates/substrate/src/runtime.rs#L1330) (test) | `list(&AppScope::Local(AppInstanceId::new("app-1")))` |
| [orchestration.rs:583](../../../../crates/control_plane/src/service/orchestration.rs#L583) (`install_app_context`) | `TopologyKey::local(prepared.instance_id.clone(), dependency_name.clone())` |
| [orchestration.rs:2069](../../../../crates/control_plane/src/service/orchestration.rs#L2069) (`write_bindings`) | `TopologyKey::local(app_instance_id, dependency_name)` |
| [orchestration.rs:302](../../../../crates/control_plane/src/service/orchestration.rs#L302) (`prepare_binding`) | `TopologyEntry { …, not_after: None }` |
| [host_capabilities.rs:1140-1158](../../../../crates/sandbox_wasm/src/host_capabilities.rs#L1140) (`CallTarget::Dependency`) | build a `TopologyKey::local(...)` instead of a `LogicalServiceRef`; the `AppInstanceId::try_new` error mapping is unchanged |
| [proxy_outbox.rs:232-248](../../../../crates/router/src/proxy_outbox.rs#L232) (`QueuedTarget::Dependency`) | same shape as `host_capabilities`: `AppInstanceId::try_new(call.app_instance_id)` → `TopologyKey::local(...)`, then `resolver.resolve(&key, …)`. **M05B B2 code, added after the milestone plan's own file table was written** |
| [saga.rs:156-172](../../../../crates/router/src/saga.rs#L156) (saga-step dependency) | identical shape again, from the step's `app_instance_id`. **M05B B4 code, same reason it is missing from the milestone table.** No test module in this file at all, so it is compiler-checked only — the plan's own test 21 is what covers the behavior these two share |

The last two are **not** mechanical. Each has to decide it is
`TopologyKey::local` rather than `Foreign` (it is: both read an
`app_instance_id` out of this node's own stored app-context row), and each
carries a `dependency '…' is no longer bound: {e}` message that a reader will
now sometimes see wrapping `expired_error`'s wording rather than "not
registered". Check both messages still read correctly for an expired foreign
entry once S3/S4 can produce one.

**Call sites to update (tests/benches),** all mechanical — let the compiler
enumerate them, as S1 did for `EndpointInfo.generation`:
`app_orchestration/benches/resolver.rs` (3 registers, 5 resolves),
`app_orchestration/src/resolver.rs`'s own test module,
`control_plane/src/service/orchestration.rs` tests
([3860](../../../../crates/control_plane/src/service/orchestration.rs#L3860),
3953, 4088 — three `logical_resolver.resolve` calls),
`sandbox_wasm/src/host_capabilities.rs` tests (4),
`router/src/proxy.rs` tests (3), `router/src/proxy_outbox.rs` tests (from line
468), `router/tests/proxy_dispatch.rs` (2), and every `TopologyEntry { … }`
literal (24 across 7 files) gains `not_after: None`.

### Phase 2 — the document type and the plan field

**`crates/app_orchestration/Cargo.toml`** — add `syneroym-identity.workspace =
true`. No cycle: `identity` is a leaf (`ed25519-dalek`, `z32`, `serde`,
`anyhow`, …) and depends on nothing in this workspace.

**New file `crates/app_orchestration/src/topology_document.rs`:**

```rust
//! Tier 2 of the logical discovery overlay (ADR-0022 §3): the signed
//! topology document a caller outside an app instance fetches, verifies
//! against the app's own DID, caches, and routes from -- without trusting
//! whoever relayed it.

/// The signed payload. Every field is inside the signature; a relay that
/// edits any of them produces a document that does not verify.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopologyDocument {
    /// The app's human name (ADR-0022 §1). Carried for display and for the
    /// `LogicalServiceRef` an intra-app reader would build; never the key
    /// a foreign reader stores this under -- that is `app_did`.
    pub app_instance_id: AppInstanceId,
    /// The app instance's own master DID: the key this document is signed
    /// by, and the identity a reader resolved in Tier 1.
    pub app_did: AppDid,
    pub service_name: LogicalServiceName,
    pub mode: TopologyMode,
    /// Member master DIDs, in member-index order -- never physical
    /// addresses (ADR-0022 §4). Ordered so an unchanged plan produces
    /// byte-identical signed content.
    pub members: Vec<ServiceId>,
    /// Carries the range table for `RangeSharding` inside itself; there is
    /// deliberately no separate `shard_map` field (D-S2-3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sharding_strategy: Option<ShardingStrategy>,
    /// The per-logical-service topology epoch (ADR-0022 §6). Shipped
    /// unenforced: shard rebalancing is what makes a caller carry it on a
    /// request and a member reject a request that no longer entitles it to
    /// a key.
    pub epoch: TopologyEpoch,
    /// The supervisor's own app-instance generation at the moment of
    /// signing -- what tells two supervisors' documents apart, the same
    /// role `generation` plays on the Tier-1 record (ADR-0022 §2).
    pub generation: u64,
    pub issued_at: u64,
    /// Unix seconds. A cached copy routes until this and then fails
    /// (failure-matrix row 6).
    pub not_after: u64,
    /// How long the signer suggests a reader holds this before re-asking.
    /// Advice, not authority -- `not_after` is the authority, and a reader
    /// may substitute its own TTL at `to_topology_entry`. Inside the
    /// signature nonetheless (D-S2-6): a value carried *beside* the document
    /// is discarded by any interface that passes the document alone, which
    /// is every interface here.
    pub cache_ttl_ms: u64,
}

/// The document plus the app master's signature over it. z-base-32 Ed25519
/// over the RFC-8785 canonicalization of the document, the shape
/// `DelegationCertificate` and every UCAN token in this tree already use --
/// deliberately not a pkarr `SignedPacket`, which is the registry's own
/// admission format and is not what this travels through.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedTopologyDocument {
    pub document: TopologyDocument,
    pub signature: String,
}

impl TopologyDocument {
    /// Signs with the app master. The caller is responsible for the key
    /// actually being this app's master -- `sign` cannot check it, so
    /// `verify` re-derives the DID from the signature side.
    pub fn sign(self, app_master: &Identity) -> Result<SignedTopologyDocument>;
}

impl SignedTopologyDocument {
    /// Verifies the signature against `expected_app_did` and refuses a
    /// document past its `not_after`.
    ///
    /// Takes the DID the caller resolved in Tier 1 rather than trusting the
    /// document's own `app_did`, which is confused-deputy prevention in the
    /// shape `DelegationCertificate::verify_chain` already uses: a
    /// perfectly valid document for a *different* app must not satisfy a
    /// lookup for this one.
    pub fn verify(&self, expected_app_did: &AppDid) -> Result<()>;

    /// The registry entry this document becomes. `cache_ttl_override`
    /// replaces the signer's suggested `cache_ttl_ms` -- the signer decides
    /// when the answer stops being usable (`not_after`), the reader decides
    /// how often to re-ask, and `None` means "take the signer's advice".
    #[must_use]
    pub fn to_topology_entry(&self, cache_ttl_override: Option<Duration>) -> TopologyEntry;

    /// The key a *foreign* reader stores this under (D-S2-1).
    #[must_use]
    pub fn foreign_key(&self) -> TopologyKey;
}

/// Verify, convert, and register in one call -- the whole client-side path
/// ADR-0022 §3 describes, minus the fetch.
///
/// Verification happens here, once per fetch, and never on the resolve path
/// (task.md's own budget: "Verify on register, not on read").
pub fn register_verified(
    resolver: &LogicalResolver,
    signed: &SignedTopologyDocument,
    expected_app_did: &AppDid,
    cache_ttl_override: Option<Duration>,
) -> Result<TopologyKey>;

/// How a caller reaches a supervisor to fetch a document. A trait rather
/// than a concrete client because the only implementation needs
/// `syneroym-sdk`, and `syneroym-control-plane` -- S4's consumer -- cannot
/// depend on it (`sdk -> router -> control_plane` is a cycle). Anything
/// holding an `Arc<dyn TopologyFetcher>` is injected from the substrate's
/// composition root, which depends on both.
///
/// **Nothing on the substrate holds one after S2** (§0.8, D-S2-13): the
/// implementors are `roymctl` and SDK callers. The trait is shaped for the
/// substrate-side consumer ADR-0022 §3 asks for so that S3/S4 add a holder,
/// not a signature.
#[async_trait::async_trait]
pub trait TopologyFetcher: fmt::Debug + Send + Sync {
    /// Tier 1 then Tier 2: resolve `app_did` to its supervising node, call
    /// that supervisor's `resolve`, and return the document **unverified** —
    /// verification is the caller's, so a fetcher can never be the trust
    /// boundary.
    async fn fetch(
        &self,
        app_did: &AppDid,
        service_name: &LogicalServiceName,
    ) -> Result<SignedTopologyDocument>;
}

/// A stable hash of everything a topology epoch must change on:
/// `(mode, ordered members, sharding_strategy)`. Not `not_after`, not
/// `generation`, not `issued_at` -- those move on every signing and would
/// make the epoch a clock.
#[must_use]
pub fn topology_fingerprint(
    mode: TopologyMode,
    members: &[ServiceId],
    sharding_strategy: Option<&ShardingStrategy>,
) -> String;
```

`sign`'s canonical payload is built the way `DelegationCertificate` builds
its own: `serde_json::to_value(&self)` → `substrate::canonicalize_json_value`
→ `serde_json::to_string` → `identity.sign(bytes)` → `z32::encode`. Reuse
`Identity::sign_json` and `substrate::verify_json_signature` directly rather
than re-implementing either.

**`crates/app_orchestration/src/models.rs`** — `PlannedService` gains, after
`topology_mode`:

```rust
    /// Cloned from `ServiceSpec.sharding_strategy` -- every member of a
    /// scaled sharded service carries the identical value, exactly as
    /// `topology_mode` and `schedule` already do. Needed on the *plan*, not
    /// only the manifest: the supervisor holds no manifest, so a Tier-2
    /// topology document (ADR-0022 §3) built from the stored plan could
    /// otherwise never name a strategy at all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sharding_strategy: Option<ShardingStrategy>,
```

**`crates/app_orchestration/src/compiler.rs`** — inside the `for member_index
in 0..spec.replicas` loop (around line 160), add `sharding_strategy:
spec.sharding_strategy.clone(),` to the `PlannedService` literal.

**Every other `PlannedService { … }` literal** gains
`sharding_strategy: None` — compiler-enumerated, same as S1's `generation: 0`
pass.

### Phase 3 — the supervisor side

**`crates/core/src/config.rs`** — two new `SupervisorRole` fields, with
`default_supervisor_*` functions and `Default` entries in the existing shape:

```rust
    /// How long a signed topology document (ADR-0022 §3) stays usable after
    /// it is signed. This is the window a caller with a cached document
    /// keeps routing while this supervisor is down -- the availability
    /// property the document form exists for -- and equally the window a
    /// caller may act on a member set this supervisor has already changed.
    /// One hour balances the two: comfortably longer than any restart, far
    /// shorter than the Tier-1 record's own 30-day backstop, which answers
    /// a slower question.
    pub topology_document_not_after_secs: u64,       // default 3_600
    /// What a fetching caller is told to re-ask on, carried inside the
    /// signed document as `cache_ttl_ms`. Advice, not authority -- the
    /// signer owns `not_after`, and a reader may substitute its own TTL --
    /// but the supervisor is the only party that knows how often this app's
    /// topology actually moves, so it is the right party to advise. Five
    /// minutes: twelve re-asks inside one document's life, each of which is
    /// a no-op if nothing changed.
    pub topology_document_cache_ttl_secs: u64,       // default 300
```

**`crates/app_supervisor/src/store.rs`**

New table, in `open`'s `execute_batch`, beside `app_tier1_refresh`:

```sql
-- ADR-0022 §3/§6: the per-logical-service topology epoch a Tier-2
-- document carries and shard rebalancing will fence the data path on.
-- Distinct from `binding_epochs`, which counts writes pushed to one
-- *dependent* and moves for reasons unrelated to a member set changing.
-- `fingerprint` is what decides whether a submit is a change at all.
-- Rows are never deleted: a service removed from a plan and re-added
-- later must not reuse a lower epoch.
CREATE TABLE IF NOT EXISTS topology_epochs (
   app_instance_id TEXT NOT NULL,
   service_name    TEXT NOT NULL,
   epoch           INTEGER NOT NULL,
   fingerprint     TEXT NOT NULL,
   PRIMARY KEY (app_instance_id, service_name)
);
```

New methods:

```rust
/// `0` when nothing has ever been recorded -- "no epoch claimed", the same
/// reading `EndpointInfo.generation`'s own default carries.
pub fn topology_epoch(&self, app_instance_id: &str, service_name: &str) -> Result<u64>;

/// Records `fingerprint`, returning the epoch that now applies: unchanged
/// when the fingerprint matches what is stored, `stored + 1` otherwise, `1`
/// when nothing is stored. One statement -- `INSERT … ON CONFLICT DO UPDATE
/// SET epoch = epoch + 1, fingerprint = excluded.fingerprint WHERE
/// fingerprint != excluded.fingerprint` -- read back under the same store
/// mutex, so two callers cannot interleave a read and a write around it.
///
/// **`handle_submit` only** (D-S2-4). This form advances the epoch, and
/// advancing is only safe while the instance lock is held: the caller must
/// have read the plan it is fingerprinting and written it durably with no
/// other writer in between. `handle_resolve` holds no lock and uses
/// `initialise_topology_epoch` instead.
pub fn record_topology_fingerprint(
    &self,
    app_instance_id: &str,
    service_name: &str,
    fingerprint: &str,
) -> Result<u64>;

/// The insert-only counterpart, for a caller holding no instance lock
/// (D-S2-4): `INSERT … ON CONFLICT DO NOTHING`, then read the row back.
/// Creates a row at epoch `1` when none exists -- the backfill every
/// pre-S2 instance needs -- and otherwise changes nothing at all.
///
/// Returns `(epoch, stored_fingerprint)`. The caller **must** compare the
/// returned fingerprint with the one it passed: they differ exactly when a
/// `submit` landed between the caller's plan read and this call, meaning
/// the plan in hand is already stale. Signing then would pair the previous
/// plan's members with the new plan's epoch, inside a signature, where it
/// cannot be corrected afterwards.
pub fn initialise_topology_epoch(
    &self,
    app_instance_id: &str,
    service_name: &str,
    fingerprint: &str,
) -> Result<(u64, String)>;

/// The instance this app master DID belongs to (D-S2-5). `None` for a DID
/// no instance on this supervisor has ever recorded -- which `resolve`
/// deliberately reports identically to "not authorized".
pub fn instance_by_app_master_did(&self, app_master_did: &str) -> Result<Option<DesiredState>>;
```

`instance_by_app_master_did` is `SELECT … FROM desired_state WHERE
app_master_did = ?1 AND app_master_did != ''` reusing the same row mapper
`get` uses.

**`crates/app_supervisor/src/topology.rs` (new)** — the supervisor's own
document machinery, a sibling of `tier1.rs`:

```rust
/// The signed document for one logical service, plus when it was signed --
/// the value `SupervisorService`'s in-process cache holds (D-S2-6).
struct CachedDocument { signed: SignedTopologyDocument, epoch: u64 }

/// Groups a stored plan's members into one logical service's topology.
/// Pure: no vault, no store, no clock, so the grouping rule is testable on
/// its own.
///
/// Members come out in `member_index` order, so an unchanged plan hashes
/// and signs identically every time. A plan whose members disagree about
/// `topology_mode` or `sharding_strategy` is a compiler bug, not a
/// recoverable state, and is refused here rather than silently resolved to
/// whichever member happened to sort first.
pub fn service_topology(
    plan: &DeploymentPlan,
    service_name: &LogicalServiceName,
) -> Result<ServiceTopology, TopologyBuildError>;

pub struct ServiceTopology {
    pub mode: TopologyMode,
    pub members: Vec<ServiceId>,
    pub sharding_strategy: Option<ShardingStrategy>,
}
```

Pseudo-code:

```
service_topology(plan, name):
    members = plan.services
        .filter(s => s.logical_ref.service_name == name)
        .sorted_by(member_index)
    if members.is_empty(): return Err(NoSuchService(name))
    mode = members[0].topology_mode
    strategy = members[0].sharding_strategy
    if any member disagrees on mode or strategy: return Err(InconsistentPlan)
    Ok(ServiceTopology { mode, members: members.map(service_id), strategy })
```

**`crates/app_supervisor/src/service.rs`**

New fields on `SupervisorService` (and two new `new()` parameters, threaded
from `init_supervisor`; `new` is already `#[allow(clippy::too_many_arguments)]`):

```rust
    /// `SupervisorRole.topology_document_not_after_secs`.
    topology_document_not_after_secs: u64,
    /// `SupervisorRole.topology_document_cache_ttl_secs`.
    topology_document_cache_ttl_secs: u64,
    /// Signed Tier-2 documents, keyed `(app_instance_id, service_name)`
    /// (D-S2-6): one signature per epoch, not one per request. In process
    /// only -- after a restart the vault is locked anyway, so persisting
    /// these would buy one signature and a durability question. A cached
    /// copy is still served while the vault is locked, which is the
    /// availability property ADR-0022 §3 chose the document form for.
    signed_documents: DashMap<(String, String), CachedDocument>,
```

New `refuse_unshardable_plan`, the third sibling of `refuse_replicas_above_cap`
and `refuse_unrunnable_schedules` (D-S2-15), placed beside them and called from
their two existing call sites —
[service.rs:3834-3835](../../../../crates/app_supervisor/src/service.rs#L3834)
in `handle_submit` and
[service.rs:4259-4260](../../../../crates/app_supervisor/src/service.rs#L4259)
in `handle_force_reconcile`:

```
/// S1's two `sharding_strategy` rules, re-applied to an already-compiled
/// plan. `SynAppManifest::validate` enforces both, and nothing between the
/// compiler and here re-checks them -- the exact gap the two functions
/// above exist to close, now with a sharper consequence: a strategy that
/// reaches this supervisor goes into a *signed* Tier-2 document
/// (ADR-0022 §3), where a reader acts on it against member ids the plan's
/// author chose.
fn refuse_unshardable_plan(plan: &DeploymentPlan) -> Result<(), String> {
    counts = members per logical_ref                    // as refuse_replicas_above_cap builds
    for svc in &plan.services:
        let Some(strategy) = &svc.sharding_strategy else { continue }
        if matches!(strategy, ShardingStrategy::RangeSharding(_)):
            return Err("'<l_ref>' declares a range_sharding strategy in this plan; range
                        sharding names concrete members by ServiceId, which is reachable
                        only once shard rebalancing assigns them")
        if counts[&svc.logical_ref] <= 1:
            return Err("'<l_ref>' declares a sharding_strategy with one member in this
                        plan; a strategy over one member is not a selection")
    Ok(())
}
```

The wording deliberately mirrors `SynAppManifest::validate`'s own two messages,
so an operator who hits the rule from either end reads the same sentence.

`handle_submit` — **the fingerprints are computed before
`self.store.submit(…)` and written after it**, and the split is not
cosmetic. `service_topology` can fail (`InconsistentPlan`, `NoSuchService`),
and `store.submit` is the durable desired-state write
([service.rs:3860](../../../../crates/app_supervisor/src/service.rs#L3860)):
computing after it would return an error to the operator while leaving stored
desired state that no later `resolve` can build a document from — a refusal
that refused nothing. Everything fallible goes in front of the write, and only
the epoch rows go behind it.

The position within `handle_submit` is otherwise right as it stands:
`mint_and_substitute` runs before `store.submit`, so `plan_json_substituted`
already names real member master DIDs and the fingerprint is over the members
a document will actually carry — not over the compiler's fabricated ids.

```
// before store.submit -- everything that can fail
fingerprints = []
for each distinct service_name in plan:                      // BTreeSet, so ordered
    t = topology::service_topology(&plan, &service_name)?    // a compiler bug refuses the submit
                                                             // with nothing written
    fingerprints.push((service_name,
                       topology_fingerprint(t.mode, &t.members, t.sharding_strategy.as_ref())))

self.store.submit(…)?                                        // unchanged, the durable write

// after store.submit -- infallible in practice, and a failure here is a
// stale epoch on an otherwise-correct stored plan, which `resolve`'s own
// backfill (D-S2-4) repairs on the next read
for (service_name, fp) in fingerprints:
    epoch = store.record_topology_fingerprint(app_instance_id, service_name, &fp)?
    if epoch changed: signed_documents.remove((app_instance_id, service_name))
```

The cache eviction here is belt-and-braces: `handle_resolve` re-signs on an
epoch mismatch anyway, and removing it on write means a scale-out is visible
without waiting for that comparison.

New `handle_resolve`, and one line in `dispatch`:

```rust
"resolve" => self.handle_resolve(&invocation.caller, invocation.params).await,
```

```
handle_resolve(caller, params):
    (app_did_str, service_name_str) = parse params            // InvalidParams on failure
    app_did = AppDid::try_new(app_did_str)                     // InvalidParams: not a DID at all
    service_name = LogicalServiceName::try_new(service_name_str)

    // Look up first, authorize second, and report both failures the same
    // way (D-S2-7): the lookup is a local read that tells the caller
    // nothing, and returning a distinguishable "no such app" would let an
    // ungranted caller enumerate this node's apps.
    state = store.instance_by_app_master_did(app_did.as_str())?
    denied = Custom(PERMISSION_DENIED_CODE,
                    "no app instance '<did>' is resolvable by caller <did> on this supervisor")
    let Some(state) = state else { return Err(denied) }
    // `synapp:<app-did>`, not `substrate:<node>/app/<id>` (D-S2-7): the
    // latter's `app/` slot already holds a `service_id`, and it dies on the
    // handover ADR-0022 §5 explicitly worried about. A bare
    // `substrate:<node>` `substrate/admin` grant still covers this, because
    // `Capability::grants` short-circuits on `is_substrate_scope`.
    if !caller.has_capability(
           &ResourceUri(format!("synapp:{app_did}")),
           &Ability(Ability::SUPERVISOR_RESOLVE.to_string())) {
        return Err(denied)                                     // identical message
    }
    if state.retired { return Err(denied) }                    // a retired instance answers nothing
    // `state.paused` is deliberately NOT checked (D-S2-14): pause stops the
    // resident loop, not the members, and they stay worth routing to.

    // The plan read, the epoch, and the signature must describe one plan.
    // This call holds no instance lock, so a `submit` can land between the
    // read and the sign -- the loop re-reads and retries once, then gives
    // up rather than signing a mismatched pair (D-S2-4).
    for attempt in 0..2:
        plan = DeploymentPlan::from_json(&state.plan_json)?
        topo = topology::service_topology(&plan, &service_name)  // NoSuchService -> a *distinct*,
                                                                 // ordinary error: the caller is
                                                                 // already authorized for this app
        fp = topology_fingerprint(topo.mode, &topo.members, topo.sharding_strategy.as_ref())
        // Insert-only, never advancing (D-S2-4): initialises the row a
        // pre-S2 instance never got, and touches nothing otherwise.
        (epoch, stored_fp) = store.initialise_topology_epoch(
            &state.app_instance_id, service_name.as_str(), &fp)?
        if stored_fp == fp: break
        // A submit landed under us. Re-read and try once more.
        state = store.instance_by_app_master_did(app_did.as_str())?.ok_or(denied)?
    if stored_fp != fp:
        return Err(InternalError("this app instance's plan is changing faster than a document
                                  can be signed for it; retry"))

    // D-S2-6: one signature per (service, epoch), re-signed when less than
    // half the document's own validity remains.
    key = (state.app_instance_id.clone(), service_name.to_string())
    if let Some(cached) = signed_documents.get(&key)
       && cached.epoch == epoch
       && cached.signed.document.not_after.saturating_sub(now) > not_after_secs / 2 {
        return Ok(payload(cached.signed))
    }

    doc = TopologyDocument {
        app_instance_id, app_did, service_name, mode: topo.mode,
        members: topo.members, sharding_strategy: topo.sharding_strategy,
        epoch: TopologyEpoch(epoch), generation: state.generation,
        issued_at: now, not_after: now + self.topology_document_not_after_secs,
        cache_ttl_ms: self.topology_document_cache_ttl_secs * 1_000,
    }
    master = keys::existing_app_master(&self.vault, &state.app_instance_id).await
        // Locked -> raise AlertKind::VaultLocked (D-S2-12) and return an
        // InternalError naming `inject-kek`, the same wording
        // `refresh_due_app_tier1_record` already uses.
        // None    -> InternalError: this instance has no app master; run `adopt`.
    if derive_did_key(master.public_key()) != app_did:
        raise AlertKind::AppIdentityMismatch; return InternalError
        // The same guard `sign_tier1_record` makes -- a document signed
        // under a DID nobody looked up is worse than no document.
    signed = doc.sign(&master)?
    signed_documents.insert(key, CachedDocument { signed: signed.clone(), epoch })
    clear AlertKind::VaultLocked
    Ok(payload(signed))
```

`payload(signed)` is exactly `serde_json::to_value(&signed)` — nothing beside
the signed document, so what a caller deserializes is what was signed
(D-S2-6).

**`crates/ucan/src/capability.rs`** — one new ability constant:

```rust
    /// ADR-0022 §5: fetch one app instance's Tier-2 topology documents from
    /// the supervisor that holds it. Flat -- it entails only itself, and is
    /// entailed by `substrate/admin` like everything else.
    pub const SUPERVISOR_RESOLVE: &'static str = "supervisor/resolve";
```

**`crates/wit_interfaces/wit/supervisor/supervisor.wit`** — the records and
the verb (field names kebab-case; the JSON on the wire is the snake_case serde
form, which test 45 pins):

```wit
    /// One logical service's membership, signed by the app instance's own
    /// master (ADR-0022 §3). A document, not an RPC answer: any party may
    /// relay it, and a reader trusts the signature against the app DID it
    /// resolved in Tier 1 rather than the connection it arrived on.
    record topology-document {
        app-instance-id: string,
        app-did: string,
        service-name: string,
        mode: topology-mode-name,
        /// Member master DIDs in member-index order -- never addresses
        /// (ADR-0022 §4). Turning one into a location is Tier 3, unchanged.
        members: list<string>,
        /// The sub-strategy a `sharded` selection uses (ADR-0022 §6):
        /// `"hash_sharding"` or `"entity_tag_sharding"`. Absent when the
        /// service declares none.
        ///
        /// Named to match the Rust field exactly, with **no `-json`
        /// suffix**: that suffix means "the Rust side is a `String`" --
        /// `submission`'s own `plan-json` and `inventory-json` both do,
        /// as does `params-json` in `control-plane.wit` -- and this one is
        /// an enum.
        ///
        /// `string` is exact for every value that can appear here, but the
        /// Rust type it mirrors is wider: `ShardingStrategy` has a third
        /// variant, `range_sharding`, which serializes as an object rather
        /// than a string. It cannot reach this field -- it is refused at
        /// manifest validation *and* re-refused against an already-compiled
        /// plan at `submit`/`force-reconcile` (`refuse_unshardable_plan`),
        /// which is what makes this declaration true rather than merely
        /// documented. **If that second refusal is ever lifted, this
        /// declaration stops being accurate**, and the replacement is a WIT
        /// `variant` -- which will also need `no_supervisor_verb_accepts_or_
        /// returns_key_material` reconsidered, since the range table's bound
        /// fields are named `start-key`/`end-key` and that guard refuses any
        /// record field whose name contains "key".
        sharding-strategy: option<string>,
        epoch: u64,
        generation: u64,
        issued-at: u64,
        not-after: u64,
        /// How long the signer suggests a reader holds this before
        /// re-asking. Advice: `not-after` is the authority, and a reader may
        /// substitute its own. Inside the signature so it survives every
        /// interface that passes the document alone.
        cache-ttl-ms: u64,
    }
    /// "singleton" | "redundant" | "sharded".
    type topology-mode-name = string;

    record signed-topology-document {
        document: topology-document,
        /// z-base-32 Ed25519 over the RFC-8785 canonicalization of
        /// `document`.
        signature: string,
    }

    /// Tier 2 (ADR-0022 §3). Takes the app's **master DID**, not its human
    /// name: Tier 1 answers with a DID, so requiring the name would make
    /// the chain unfollowable.
    ///
    /// Answers the full member set and mode, or denies -- never a filtered
    /// list (§5): rendezvous hashing over a partial set returns a
    /// confident wrong answer with no error raised.
    ///
    /// An unknown app and an unauthorized caller are reported identically,
    /// so a caller with no grant cannot probe for an app's existence.
    resolve: func(app-did: string, service-name: string)
        -> result<signed-topology-document, string>;
```

**`crates/substrate/src/runtime.rs`** — `init_supervisor` passes
`role.topology_document_not_after_secs` and
`role.topology_document_cache_ttl_secs` into `SupervisorService::new`.

### Phase 4 — the client side, the CLI, and the e2e

**`crates/sdk/src/topology.rs` (new)** — the one `TopologyFetcher`
implementation:

```rust
/// Tier 1 → Tier 2 → a verified document, over the real network.
///
/// Holds a registry URL rather than a `RegistryClient` so each fetch is
/// independent; a supervisor connection is opened per fetch and dropped,
/// the same one-shot shape `LiveQueueConnector` uses.
#[derive(Debug)]
pub struct RegistryTopologyFetcher {
    registry_url: String,
    connect_timeout: Duration,
    /// Presented on the supervisor connection -- `resolve` is authorized
    /// (ADR-0022 §5), so a fetch without one only works for the node owner.
    caller_ucan: Option<CapabilityToken>,
    identity: Option<Identity>,
}
```

```
fetch(app_did, service_name):
    // Tier 1: the app DID resolves to the substrate supervising it.
    tier1 = RegistryClient::new(false, Some(registry_url)).lookup(app_did, false).await?
    tier1.verify()?                       // self-signed under the app DID; no other trust input
    supervisor_did = tier1.info.substrate_id
    // Tier 3 for the supervisor itself: `SyneroymClient::connect` does the
    // second lookup and picks a mechanism.
    client = SyneroymClient::new_with_identity(supervisor_did, registry_url, identity)
             .with_connect_timeout(connect_timeout)
             [.with_ucan(caller_ucan)]
    client.wait_for_ready(connect_timeout).await?
    resp = client.request("supervisor", "resolve", json!([app_did, service_name])).await?
    client.shutdown().await
    Ok(serde_json::from_value::<SignedTopologyDocument>(resp.result)?)
```

Verification is **not** done here — `register_verified` (phase 2) is the only
place a document is trusted, so no fetcher can become the trust boundary. A
convenience `fetch_and_register(&resolver, app_did, service_name)` calls
`fetch` then `register_verified(…, None)`; the suggested TTL travels inside
the signed document (D-S2-6), so nothing has to be carried alongside it.

**Nothing in `crates/substrate/src/runtime.rs` constructs this.** §0.8 and
D-S2-13 say why, and §5 carries the backlog row. The two production callers
S2 does ship are `roymctl app resolve` and any SDK program.

**`apps/roymctl/src/commands/app.rs`** — one new subcommand, the outside
caller's surface:

```
/// Resolve an app's logical service to its current member set: look the app
/// DID up in the registry (Tier 1), fetch the signed topology document from
/// the supervisor holding it (Tier 2), verify it against the app DID, and
/// print the members. Prints the members as DIDs, not addresses -- turning
/// one into an address is an ordinary registry lookup (Tier 3).
Resolve { app_did: String, service_name: String }
```

**`crates/substrate/tests/topology_document_e2e.rs` (new)** — copies the
`Node`/`boot_pair`/`supervisor_role`/`compiled_plan_json` helpers from
`tier1_endpoint_record_e2e.rs` (which itself copied them from
`app_instance_identity_e2e.rs`), with `replicas = 2` on the manifest's one
service so the member set is a set. Port blocks start at **15_400** — the next
free after `tier1_endpoint_record_e2e.rs`'s 15_000–15_302 — one block of six
per test.

**How the outside caller in tests 54–59 is authorized**, since nothing else in
the tree issues this shape yet: a `resolve_grant` helper beside the existing
`node_wide_supervisor_grant`, issuing from the *supervisor node's owner*
(which `Node::boot` already installs as `config.iam.admin_ucan_root`) to a
freshly generated caller identity:

```rust
CapabilityToken::issue(
    supervisor_node_owner,
    caller_did,
    vec![Capability {
        with: ResourceUri(format!("synapp:{app_did}")),
        can: Ability(Ability::SUPERVISOR_RESOLVE.to_string()),
        caveats: None,
    }],
    Map::new(), 3600, vec![],
)
```

The caller holds no `substrate/admin` and is not part of the app instance,
which is what makes test 54 an honest reading of the reference scenario's
step 3. The `app_did` is only known after `adopt`, so the grant is issued
mid-test rather than at boot.

**Docs:** `status.md` gains an S2 evidence section; `implementation-plan.md`'s
§2 slice table points S2 at this file; `docs/developer-guide.md` gains the two
new `[roles.supervisor]` config fields.

---

## §4 — S2 tests

**e2e cases are marked; everything else is a unit test.** Numbering is
per-milestone and continues from S1's 18.

**Phase 1 — the resolver key and expiry:**

19. `two_foreign_apps_with_the_same_instance_id_do_not_collide` — milestone
    §0.4, failure-matrix row 8. Two documents naming `chat`, different app
    DIDs; both resolve to their own members
20. `a_local_entry_and_a_foreign_entry_with_the_same_service_name_are_distinct`
21. `an_entry_past_its_not_after_stops_resolving` — matrix row 6, asserted on
    both the cache-hit path and the registry path
22. `an_entry_with_no_not_after_resolves_as_it_does_today` — the
    absent-means-current-behavior property for every intra-app binding
23. `a_not_after_difference_at_one_epoch_is_not_a_binding_conflict` — D-S2-2's
    exclusion from `classify_binding_write`

**Phase 2 — the document:**

24. `a_document_verifies_against_the_app_did_that_signed_it`
25. `a_document_signed_under_a_different_key_is_rejected` — matrix row 5's
    negative half, at unit scale
26. `a_document_whose_members_were_altered_after_signing_is_rejected` — every
    field is inside the signature, proven on the one field a relay would most
    want to edit
27. `a_document_past_its_not_after_is_rejected` — matrix row 6 at the document
    layer, distinct from test 21's resolver layer
28. `a_document_verifies_from_bytes_alone_with_no_connection` — matrix row 5:
    serialize, drop everything else, verify from the bytes and the app DID
29. `a_document_for_a_different_app_did_is_rejected` — the confused-deputy
    guard: a valid document that is not the one asked for
30. `a_document_converts_to_a_topology_entry_preserving_mode_members_and_epoch`
    — matrix row 10, the "field is carried and preserved" pin
31. `a_sharding_strategy_survives_the_document_round_trip` — including the
    `RangeSharding` variant's table. **A type-level guard, not a claim about
    reachability**: `refuse_unshardable_plan` (D-S2-15) stops that variant
    reaching the wire at all, so this test exercises a value the document
    *can hold* and the transport never carries. Worth keeping in that form —
    a serde container that silently corrupts a variant of its own field type
    is a defect whether or not today's callers can produce it — but a reader
    must not take this test as evidence the WIT's `option<string>` is wrong
32. `a_fingerprint_changes_when_the_member_set_changes_and_not_otherwise` —
    D-S2-4: same members reordered hash the same; a member added does not
33. `a_relocated_member_does_not_change_the_document` — matrix row 9: the
    document names DIDs, so a member's substrate moving changes nothing here

**Phase 3 — the supervisor:**

34. `a_first_submit_starts_every_services_topology_epoch_at_one`
35. `a_resubmit_that_does_not_change_membership_leaves_the_epoch_alone`
36. `a_resubmit_that_scales_a_service_out_increments_only_that_services_epoch`
37. `a_topology_epoch_never_goes_backwards_when_a_service_is_removed_and_re_added`
    — D-S2-4's "rows are never deleted"
38. `resolve_returns_a_document_naming_every_member_master_did_in_index_order`
39. `resolve_refuses_a_caller_holding_no_grant_for_this_app` — matrix row 7
40. `resolve_reports_an_unknown_app_and_an_unauthorized_caller_identically` —
    D-S2-7's probing guard, asserted on the exact error string
41. `a_refused_resolve_carries_no_member_dids_at_all` — D-S2-8: matrix row 7's
    "never a filtered member list", pinned as an assertion about the refusal
    payload rather than as an absence of code
42. `resolve_on_a_locked_vault_fails_loudly_and_names_inject_kek` — D-S2-12
43. `resolve_signs_once_per_epoch_and_serves_the_cached_document_afterwards` —
    D-S2-6, asserted on the signature bytes being identical across two calls
44. `a_membership_change_re_signs_the_document_at_the_new_epoch`
45. `the_resolve_payloads_json_keys_match_the_wit_records_field_names` — walks
    `supervisor.wit` with `wit_parser` (already a dev-dependency, already used
    by `the_supervisor_wit_dispatch_table_covers_every_declared_function`) and
    asserts **set equality** between the WIT record's field names with `-`
    replaced by `_` and the serialized payload's keys. The WIT record and the
    serde struct are two descriptions of one wire format, and nothing else
    stops them drifting. **The fixture must declare a `sharding_strategy`**:
    it is `skip_serializing_if = "Option::is_none"`, so a fixture without one
    omits the key and the comparison reads a real name match as a mismatch.
    Every other field on this document is unconditionally serialized
46. `resolve_is_refused_for_an_instance_that_has_no_app_master_did` — the
    pre-A7 instance case, the same skip `refresh_due_app_tier1_record` makes
47. `resolve_backfills_a_topology_epoch_for_an_instance_submitted_before_this_slice`
    — D-S2-4: a store row written with no `topology_epochs` entry resolves at
    epoch 1, not 0, and a second resolve does not advance it again
48. `a_resolve_never_advances_a_topology_epoch` — D-S2-4's insert-only rule,
    driven at the store layer where the race is expressible without timing:
    record fingerprint A through `record_topology_fingerprint` (epoch 1), then
    call `initialise_topology_epoch` with a *different* fingerprint B (the
    stale-plan case a lock-free reader can hit) and assert the epoch is still
    1, the stored fingerprint is still A, and the returned fingerprint is A so
    the caller can see it lost the race. This is the test the first revision
    of this plan would have failed
49. `resolve_answers_for_a_paused_instance_and_refuses_for_a_retired_one` —
    D-S2-14, the two halves in one test because the decision is the contrast
50. `refuse_unshardable_plan_refuses_a_hand_authored_range_sharding_plan` —
    D-S2-15, driven against a `DeploymentPlan` built directly rather than
    compiled, since a compiled one cannot carry the value. Its sibling
    `refuse_unshardable_plan_refuses_a_strategy_over_a_single_member`, and the
    paired `..._allows_a_plan_with_no_strategy`, follow
    `refuse_replicas_above_cap`'s existing refuse/allow pair exactly
51. `a_submitted_plan_declaring_range_sharding_is_refused_before_anything_is_stored`
    — the same rule at dispatch level: the refusal runs beside its two
    siblings, ahead of `store.submit`, so nothing durable is written. This is
    the test that makes the WIT's `option<string>` a checked property rather
    than a comment

**Phase 4 — the client, and the reference scenario:**

52. `a_verified_document_registers_under_the_app_did_not_the_instance_id` —
    D-S2-1 through `register_verified`
53. `one_fetch_and_register_serves_every_later_resolve` — the two performance
    budgets that are this slice's, measured as task.md states them rather than
    as timings. A `CountingFetcher` records `fetch` calls; the test runs one
    `fetch_and_register`, then N `resolve` calls, and asserts `fetch_calls ==
    1` (budget 1: "resolution after the first fetch — no network call") and
    that `register_verified` ran exactly once (budget 3: "verify once per
    fetch, not once per resolve" — `register_verified` is the only place
    `verify` is called, so its call count *is* the verification count).
    **Deliberately not the first draft's version**, which asserted a
    panicking fetcher is never called: with no resolve path able to reach a
    fetcher at all (§0.8), that test could not have failed. This one is a
    genuine guard, because the obvious S3 change — a refresh hook on the
    resolve path — is exactly what would break it
54. **(e2e)** `an_outside_caller_resolves_an_apps_members_and_calls_one` —
    reference scenario steps 3 and 5, from a caller that is not part of the
    app instance and holds only a `supervisor/resolve` grant
55. **(e2e)** `a_relayed_document_verifies_for_a_party_that_never_contacted_the_supervisor`
    — step 4, matrix row 5
56. **(e2e)** `a_scaled_out_service_supersedes_the_cached_document_at_a_new_epoch`
    — step 6
57. **(e2e)** `a_cached_document_still_routes_after_the_supervisor_is_down` —
    step 7, matrix row 4's first half, and the one test that proves the
    supervisor is off the path after the first fetch
58. **(e2e)** `a_caller_with_no_cached_document_fails_cleanly_when_the_supervisor_is_down`
    — matrix row 4's **second** half ("a caller with none fails cleanly"),
    which nothing else covers: same shutdown as test 57, a caller that never
    fetched, asserting a clean error rather than a hang or a partial answer
59. **(e2e)** `a_document_forged_under_a_different_key_is_rejected` — step 8

**Matrix coverage after S2** (task.md's "every failure/security matrix row has
a named test"): row 4 → 57 (cached) and 58 (no cache); row 5 → 28, 55; row 6 →
21, 27; row 7 → 39, 40, 41; row 8 → 19; row 9 → 33; row 10 → 30. Rows 1, 2, 3,
11 are S1's and already have named tests.

**Performance-budget coverage** (exit criterion 3, "every budget has a
measurement"). Only two of task.md's four budgets are this slice's:

| Budget | Covered by |
|---|---|
| 1 — resolution after the first fetch makes no network call | Test 53's `fetch_calls == 1` |
| 3 — document verification once per fetch, not once per resolve | Test 53's second assertion: `register_verified` is the only caller of `verify`, so its call count *is* the verification count |
| 2 — Tier-1 lookup within the existing registry budget | **S1's**, unchanged by this slice |
| 4 — Tier-1 refresh cost: one signature and one publish per interval, on the existing pass tick | **S1's**, unchanged by this slice. Test 43 is *not* this budget — it measures Tier-2 document signing, which is a different cadence on a different code path; the first revision of this plan mapped them together |

---

## §5 — Backlog rows this slice creates

Added at closeout, per the Mandatory Deferred-Backlog Update rule:

- **The rendezvous domain separator differs between intra-app and foreign
  callers of one logical service** (§0.4, D-S2-1). Target: S5 (M7
  `[PLT-RED]`), because that is when the epoch fence makes a wrong member a
  wrong shard rather than a slow answer. The fix is one canonical separator —
  the app DID — which needs the intra-app push path to carry it on the wire.
- **No early cache invalidation: an epoch bump is not published** (§0.7,
  D-S2-10). A caller learns a new member set at its own `cache_ttl`, not
  sooner. ADR-0022 §3 names the MQTT alert path as the vehicle. Target: S3, if
  a subscriber appears.
- **`resolve`'s visibility is a capability check with no manifest
  declaration** (§0.5, D-S2-7). ADR-0022 §5's per-logical-service "open to
  all" branch is not expressible until S4 adds the declaration; until then
  every service on a supervisor is equally reachable to anyone holding
  `supervisor/resolve` for that app, with no per-service narrowing. Target:
  S4.
- **Signed documents are not persisted across a supervisor restart** (D-S2-6).
  After a restart the vault is locked, so the first `resolve` for each service
  fails until an operator runs `inject-kek` — even though a perfectly valid
  document existed a moment earlier. Target: TBD; it is the same
  restart-surviving-KEK problem the milestone plan §3 already declines to fix.
- **No substrate holds a `TopologyFetcher`, so ADR-0022 §3's substrate-side
  fetch does not exist yet** (§0.8, D-S2-13). A guest naming a foreign app as
  a declared dependency still fails; the fetch is reachable only from
  `roymctl` and SDK programs. Target: S3 (the gateway is a substrate-side
  caller with a real trigger) or S4 (a cross-app declared dependency),
  whichever lands first.
- **A `supervisor/resolve` grant does not survive a handover** (§0.5,
  D-S2-7). The resource (`synapp:<app-did>`) does — it is app-scoped and
  always local — but the *trust root* is the supervising node's
  `admin_ucan_root`, so a chain issued by the old node is not rooted at the
  new one. The root that would survive is the app master itself, the app-level
  analogue of ADR-0015 A6's per-service `owner_of` root; nothing can issue
  such a chain today, because the app master exists only inside the
  supervisor's vault and there is no verb to delegate from it. Target: S4.
- **A verified foreign document is lost on a caller-side restart.**
  `replay_persisted_bindings`
  ([runtime.rs:882](../../../../crates/substrate/src/runtime.rs#L882)) rebuilds
  the resolver only from `EndpointRegistry`'s stored *bindings*, and nothing
  persists a foreign entry — the mirror image of D-S2-6's supervisor-side row,
  on the reader's side. Consequence: a restarted caller must re-fetch, so the
  "supervisor is off the availability path" property holds within a process
  lifetime and not across one. Latent until S3/S4 gives a substrate a reason
  to hold foreign entries at all. Target: S3/S4, with the same row.

**Rows this slice updates:** *`TopologyMode::Sharded` is compiled by nothing*
gains the `PlannedService.sharding_strategy` half (§0.2) — still not closed,
since `compiler.rs`'s `replicas > 1 ⇒ Redundant` line is unchanged.

---

## §6 — What closing S2 closes

- **Tier 2 of ADR-0022**, and with it S3's and S4's shared gate.
- **Milestone plan §0.4**, the `LogicalResolver` foreign-entry collision,
  settled in S2 as milestone D-C-6 requires.
- **The `epoch` field on the wire, unenforced** (D-C-4, ADR-0022 §6) — the
  second of the two ship-before-enforce declarations this milestone owes S5.
- **`[PLT-DAP-01]`'s cross-app half**, up to the point of resolution; S3
  supplies the external addressing form.
- **Incidentally, a pre-existing gap in `submit`'s plan validation**
  (D-S2-15): S1's two `sharding_strategy` manifest rules were enforced at
  compile time only, like the `replicas` and schedule rules before their own
  siblings were written. S2 closes it because S2 is what makes an unchecked
  strategy consequential -- it ends up inside a signature.

What it does **not** close: the gateway hostname and routing-key header (S3);
cross-app `Bind` and the per-service visibility declaration (S4); epoch
enforcement and rebalancing (S5, M7); the locked-vault KEK problem, which S2
makes costly in one more place without fixing; and — the one place S2
knowingly falls short of its own design of record — **ADR-0022 §3's
substrate-side fetch** (§0.8, D-S2-13). The pieces ship and are shaped for it;
no substrate holds one, so a guest still cannot reach a foreign app by
declared name.

---

## §7 — The milestone's exit criteria, against this slice

task.md's ten criteria are milestone-level, so most close at S3. The three
that need an answer *from this slice* rather than at milestone closeout:

- **Criterion 3, "every performance budget has a measurement."** Covered by
  §4's budget map. The cache budget is asserted as "no network call", not as a
  timing, exactly as the criterion words it. Two of the four budgets are S1's
  and unchanged here — the map says which, rather than claiming an S2 test
  measures them.
- **Criterion 10, "`wasm32-wasip2` test components rebuild against any changed
  WIT."** This slice changes `supervisor.wit` and nothing else. That file is
  host-side only: it is bound by
  [`wit_interfaces/src/supervisor.rs`](../../../../crates/wit_interfaces/src/supervisor.rs)
  for the host world, it appears in no `test-components/*/wit/` tree, and no
  guest imports it. **So no guest rebuild is required, and the criterion is
  satisfied vacuously — which is worth stating rather than leaving as a gap a
  reader has to re-derive.** It stops being vacuous the moment a slice touches
  `host.wit` or `app-config.wit`; S3's routing-key header is the nearest
  candidate.
- **Criterion 2, "every failure/security matrix row has a named test."**
  Covered by §4's matrix map. Row 4's two halves are separate tests (57 and
  58) — the first draft of this plan mapped the row to a single test that
  covered only the cached half.
