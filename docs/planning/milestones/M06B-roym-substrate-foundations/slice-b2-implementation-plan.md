# M06B Slice B2 — Declared Service Visibility, Both Layers: Implementation Plan

> **Status**: draft for review. Nothing implemented yet.
>
> **Scope** (from [task.md](task.md)'s B2 row and `D-06B-4`): two declarations
> for one question — *who may see and reach this service*.
>
> - **Publication half** ([ADR-0018](../../../decisions/0018-service-record-visibility.md)):
>   `service-config.visibility`, plus a publication path that reads it, so a
>   service's endpoint record reaches the community registry because someone
>   said so — not because a certificate happened to be in the manifest.
> - **Resolution half** ([ADR-0022](../../../decisions/0022-two-tier-logical-service-discovery.md)
>   §5): a per-logical-service *"open to all"* declaration, so a caller on an
>   unaffiliated installation can fetch a topology document with no
>   pre-installed token.
>
> The two halves share a milestone and a slice, and nothing else. They touch
> disjoint code, have separate types, and can be built in either order. §6
> sequences them so either can land alone if the other slips.

---

## §0 What B1 hands to B2

Four things B1 established that this plan picks up rather than rediscovers.
Recorded here because a later reader will otherwise re-derive all four.

1. **`D-B1-10` is re-opened here, deliberately.** B1 kept the **node**
   identity on the Tier-2 topology-resolution path because
   `[iam].grant_resolve_to_node_did` matches on the node DID
   ([io.rs:204-210](../../../../crates/router/src/route_handler/io.rs#L204)).
   ADR-0022 §5's per-logical-service *"open to all"* is the mechanism that
   gate stands in for. **This plan does not switch the fetcher to the person's
   identity** — see `D-B2-9` for why the answer is still "node identity", and
   why that is now a choice rather than an inheritance.
2. **The enum already shipped.** `visibility` in
   [control-plane.wit:68-72](../../../../crates/wit_interfaces/wit/control-plane/control-plane.wit#L68)
   and `Visibility` in
   [models.rs:534-542](../../../../crates/app_orchestration/src/models.rs#L534)
   both landed with M06A A1, defaulting to `private`. What is left is
   `service-config.visibility` and the publication path that reads it. **No new
   enum.**
3. **The migration rule is stated, and the default is safe.** Undeclared means
   unpublished — failure-matrix row 11 and exit criterion 10 both assert it.
   The product is unreleased, so this is a change **in place**: no
   compatibility shim, no version ladder, no dual-read period (`D-B2-3`).
4. **ADR-0018's status note is owed when B2 lands** ([task.md](task.md),
   "Owed as slices land"). §8 carries the exact edit, and it depends on the
   §2 scope decision `D-B2-8`.

---

## §1 Findings from reading the tree

Verified 2026-08-18 against the tree at commit `42898ad`. Line numbers are
from that tree.

### F1 — ADR-0018's Context describes only *one* of the two publication paths, and the other publishes unconditionally

ADR-0018 says publication happens "if and only if a pre-signed
`registry_certificate` happened to be supplied", and cites `roymctl svc
deploy --identity`. That is accurate for the **standalone `svc deploy`** path.

It is **not** the whole picture. On the **app deploy** path,
`deploy::certify_placed_members`
([deploy.rs:683-751](../../../../crates/sdk/src/deploy.rs#L683)) mints an
`EndpointInfo` with a hardcoded `is_private: false` for **every placed
member**, unconditionally, and returns it keyed by `ServiceId`;
`mapper::map_deployment_plan_to_wit`
([mapper.rs:345](../../../../crates/sdk/src/mapper.rs#L345)) puts it in each
member's `DeployManifest`. Both of its callers do this on every apply:

- `AppSupervisor` reconcile ([service.rs:3485](../../../../crates/app_supervisor/src/service.rs#L3485)),
- `roymctl app deploy` ([member_identity.rs:204](../../../../apps/roymctl/src/commands/member_identity.rs#L204)).

**Consequence for this slice.** "Undeclared = unpublished" is not achieved by
adding a field and reading it at the substrate. `certify_placed_members` must
stop minting records for members whose declared visibility is `private`, or
every app-deployed member keeps publishing exactly as before and exit
criterion 10 fails on the path that matters most. This is the single largest
correction this plan makes to its input documents (§9.1).

### F2 — a redeploy never clears a previously stored record file

[orchestration.rs:1589-1599](../../../../crates/control_plane/src/service/orchestration.rs#L1589)
writes `hosted_apps_dir/<service_id>.json` when a certificate is present, and
does nothing when it is absent. Only `undeploy`
([:2501-2506](../../../../crates/control_plane/src/service/orchestration.rs#L2501))
removes it.

So a service redeployed from `public` to `private` keeps a stale record file,
and `EndpointPublisher::publish_all_services`
([endpoint_publisher.rs:64-75](../../../../crates/core/src/endpoint_publisher.rs#L64))
keeps republishing it on every heartbeat until the record's own `not_after`
lapses (30 days by default). **Making a service private must delete the
file.** This is a real defect the new declaration exposes, not a new one.

### F3 — the enum shipped; three things did not

Present: WIT `enum visibility { public, internal, private }`
([control-plane.wit:68](../../../../crates/wit_interfaces/wit/control-plane/control-plane.wit#L68)),
model `Visibility` with `#[default] Private`
([models.rs:534](../../../../crates/app_orchestration/src/models.rs#L534)),
and `asset-bundle.visibility` using it for a different question (are these
bytes readable unsigned).

Absent: `service-config.visibility` (WIT
[:90](../../../../crates/wit_interfaces/wit/control-plane/control-plane.wit#L90)),
`ServiceConfig.visibility` (model
[models.rs:475](../../../../crates/app_orchestration/src/models.rs#L475)), and
any reader of either.

The model enum is `#[serde(rename_all = "lowercase")]`, not `snake_case` as
ADR-0018 §1's snippet writes — identical output for these three names (§9.4).

### F4 — the WIT record change is mechanical but wide

WIT records have no `Default`, so adding one field to `service-config` breaks
every struct literal. Counted by construction shape:

| Literal | Count | Where |
|---|---|---|
| WIT `ServiceConfig { env: … }` | **85**, of which **4 are production code** (`sdk/src/lib.rs` ×3, `sdk/src/mapper.rs` ×1) | The other 81: 49 in `control_plane/src/service/orchestration.rs` and 5 in `control_plane/src/service.rs` — all inside `mod tests` (`:3589`, `:797`) — plus 27 across `crates/*/tests` and `crates/sandbox_wasm/benches` |
| model `ServiceConfig { service_type: … }` | **38** | 7 in `app_orchestration/src/models.rs`, the rest across `app_supervisor`, `sdk`, `roymctl`, e2e fixtures |

None uses `..Default::default()`. Both edits are one added line per literal;
the compiler enumerates every site. See §4 for the mechanical recipe.

A raw `grep -c 'ServiceConfig {'` answers **129**, not 123: six of those are
`fn … -> ServiceConfig {` signature lines, which need no edit. Excluding them
(`grep -v -- '-> ServiceConfig {'`) gives exactly 123 = 85 + 38. Same cause
for `sdk/src/mapper.rs`, which a raw grep credits with 4 literals and actually
has one production WIT literal ([:204](../../../../crates/sdk/src/mapper.rs#L204))
plus two test-only model helpers behind `-> ServiceConfig` signatures.

### F5 — the supervisor already reads the plan before it authorizes, so an "open" check fits with no reordering

`handle_resolve`
([service.rs:5312](../../../../crates/app_supervisor/src/service.rs#L5312))
runs, in order: parse params → `instance_by_app_master_did` (reads
`state.plan_json`) → `has_capability(synapp:<app_did>, supervisor/resolve)` →
`retired` check → epoch/fingerprint loop → sign.

The declared visibility of one logical service is a pure read of
`state.plan_json` plus `topology::resolve_service_name`
([topology.rs:75](../../../../crates/app_supervisor/src/topology.rs#L75)), both
already available at that point. The check inserts between the state read and
the capability check with **no reordering** of anything else.

### F6 — the resolution half needs no WIT change at all

`submit` carries the plan as `plan-json`
([supervisor.wit:8-17](../../../../crates/wit_interfaces/wit/supervisor/supervisor.wit#L8)),
a JSON `DeploymentPlan`. A new `#[serde(default)]` field on `PlannedService`
crosses that boundary with no interface edit, exactly as `topology_mode`,
`sharding_strategy`, and `schedule` already do
([models.rs:847-871](../../../../crates/app_orchestration/src/models.rs#L847)).
The declaration never has to reach a substrate — only the supervisor, which
signs and serves the document.

### F7 — an unaffiliated caller already reaches `resolve`; only the capability check refuses it

`HandshakeVerifier::verify_preamble`
([handshake.rs:78-82](../../../../crates/router/src/handshake.rs#L78)) accepts
a preamble with a bare pubkey and no delegation, returning the key as its own
master. `dispatch_json_rpc_unfenced`
([dispatch.rs:210-212](../../../../crates/router/src/route_handler/dispatch.rs#L210))
rejects only `None` callers. So a stranger with a freshly generated key
already reaches `SupervisorService::dispatch("resolve")` today and is refused
at exactly one line — `has_capability`. **Removing that one refusal for an
`open` service is the whole of exit criterion 10's second half.** No transport,
handshake, or dispatch change.

### F8 — the client gateway and the WebRTC coordinator need no code change for `open`, only a corrected warning

Both build a `RegistryTopologyFetcher` with the node identity and an optional
`resolve_ucan`, then an `AppHostResolver`
([gateway.rs:126-160](../../../../crates/client_gateway/src/gateway.rs#L126),
[coordinator.rs:87-115](../../../../crates/coordinator_webrtc/src/coordinator.rs#L87)).
Both call the same shared `credential_warning`
([sdk/src/topology.rs](../../../../crates/sdk/src/topology.rs)) whose
`NeitherConfigured` text says app-scoped hostnames "will be refused by any
supervisor they reach". After B2 that is false for an `open` service. The
warning text — in **two** call sites plus the shared function's doc — is the
only edit either component needs.

### F9 — `DeployFacts` is a tuple alias, and widening it is the cost of reporting visibility in `list`

`pub type DeployFacts = (String, Option<String>, Option<String>)`
([storage.rs:19](../../../../crates/core/src/storage.rs#L19)), persisted in
`service_deploy_facts` ([registry_store.rs:120](../../../../crates/data_db/src/registry_store.rs#L120)).
ADR-0018 §4 requires the substrate to store the declared visibility "so `list`
can report it". That means: one column, one widened tuple, `EndpointStorage`
trait + both impls (`SqliteEndpointStorage`, `MockStorage`), and ~15
destructuring sites. `service_deploy_facts` already carries the in-tree
precedent for an idempotent `ALTER TABLE … ADD COLUMN` on this exact table
([registry_store.rs:133-147](../../../../crates/data_db/src/registry_store.rs#L133)),
which that comment argues is not a version ladder. Reuse it (`D-B2-6`).

### F10 — tests and fixtures that will fail on the new validation

**Three distinct failure modes**, and a `registry_certificate` grep finds only
the first. (A) is a deploy refusal; (B) is a refusal inside `compile()`,
triggered by *placement plus `depends_on`* in manifests that never mention a
certificate; (C) is **not a refusal at all** — nothing complains, the member's
record simply stops being published and a later dial fails. **Enumerated
exhaustively**, by grepping `PlacementSelector::Substrate`, `depends_on`, and
member-DID dials across `crates/substrate/tests` rather than by following
certificates.

**(A) Refused by `validate_publication` — a certificate supplied with no declaration:**

| Site | Fix |
|---|---|
| [master_endpoint_record_e2e.rs:224](../../../../crates/substrate/tests/master_endpoint_record_e2e.rs#L224) (`bare_tcp_manifest`, used at :366 and :479) | `visibility: Some(Internal)` in the helper's `ServiceConfig` |
| [multi_substrate_placement_e2e.rs:515-545](../../../../crates/substrate/tests/multi_substrate_placement_e2e.rs#L515) (`certify_and_publish` — a private copy of `certify_placed_members`) | declare `visibility = "internal"` on the fixture manifest's two `ServiceSpec`s, and have the copy skip private members like the real one |
| [multi_substrate_placement_e2e.rs:760](../../../../crates/substrate/tests/multi_substrate_placement_e2e.rs#L760) (direct `registry_certificate: Some(…)`) | `visibility: Some(Internal)` on that manifest |
| [binding_push_e2e.rs:420-450](../../../../crates/substrate/tests/binding_push_e2e.rs#L420) (`certify_and_publish` — a **third** private copy, feeding `ApplyRequest.registry_certificates` at [:498](../../../../crates/substrate/tests/binding_push_e2e.rs#L498)) | same as `multi_substrate_placement_e2e`'s copy |

**(B) Refused by `D-B2-14`(a) — explicit cross-substrate `depends_on` with an undeclared (therefore `private`) dependency.** These fail **inside `compile()`**, before any deploy, and a certificate grep never finds them:

| Site | Placement | Fix |
|---|---|---|
| [reference_scenario_e2e.rs:276-326](../../../../crates/substrate/tests/reference_scenario_e2e.rs#L276) | `backend` on `managed-b`, `frontend` on `managed-a` with `depends_on: ["backend"]` | `visibility = "internal"` on `backend`. **Also needed for the test to pass at all**: its cross-node dependency call needs `backend`'s record |
| [binding_push_e2e.rs:259-285](../../../../crates/substrate/tests/binding_push_e2e.rs#L259) | `backend` on `BACKEND_ALIAS`, `frontend` on `FRONTEND_ALIAS`, depends on it | same |
| [durable_outbox_e2e.rs:303-329](../../../../crates/substrate/tests/durable_outbox_e2e.rs#L303) | `backend` on `managed-a`, `frontend` on `managed-b`, depends on it | same |
| [multi_substrate_placement_e2e.rs:341-367](../../../../crates/substrate/tests/multi_substrate_placement_e2e.rs#L341) | same shape | already covered by (A)'s fix |

**(C) Nothing is refused — the record simply stops existing.** One alias, no
`depends_on`, no certificate, so neither (A) nor (B) fires; but a **gateway or
caller on another node dials the member by DID through the registry**, and
after `D-B2-7` there is no record to find. This is the class F10's old
catch-all row gestured at, and both files in it are fixtures B2's own new
tests extend:

| Site | Why it needs the record | Fix |
|---|---|---|
| [gateway_hostname_e2e.rs:300-352, 556-565](../../../../crates/substrate/tests/gateway_hostname_e2e.rs#L556) — test 99, *"the managed node's own gateway (a different node from the one supervising the app)"* | `resolve_target` returns the member DID as the connect target ([gateway.rs:611-617](../../../../crates/client_gateway/src/gateway.rs#L611)), and the gateway dials it with `SyneroymClient::new_with_identity(service_id, registry_url, …)` ([gateway.rs:418](../../../../crates/client_gateway/src/gateway.rs#L418)) — a registry lookup, even for a same-node member: the gateway holds no `EndpointRegistry` | `visibility = "internal"` on the fixture's one service. **Test 43 extends this fixture** |
| [topology_document_e2e.rs:482-500](../../../../crates/substrate/tests/topology_document_e2e.rs#L482) — `an_outside_caller_resolves_an_apps_members_and_calls_one` | Reference-scenario steps 3 and 5: the caller fetches the document, then routes to a member — Tier 3, through the registry (F13) | same. **Tests 41 and 42 extend this fixture** |

**(D) Checked and *not* affected** — recorded so the next reader does not
re-check them:

- [supervisor_interface_e2e.rs:294-360](../../../../crates/substrate/tests/supervisor_interface_e2e.rs#L294) — three services, all on `MANAGED_ALIAS`, and **no cross-node dial**: the test drives the supervisor's own RPCs, never a member.
- [supervisor_loop_e2e.rs:258-284](../../../../crates/substrate/tests/supervisor_loop_e2e.rs#L258) — two services on two aliases, but **both `depends_on` are empty** and nothing dials a member by DID.

`reference_scenario_e2e` is the one to run first: it is the M05A reference
scenario, it deploys through `submit` rather than `ApplyRequest`, and it fails
at `compiled_plan_json`'s unwrap rather than at a deploy — a failure mode
none of the other rows produce.

**Verified unaffected**: the Playwright suites — `global-setup.ts:196` and
`global-setup-multihop.ts:285,299` deploy with **no** `--identity`, and publish
separately through `roymctl registry register`, which is not a deploy path
(F12). ADR-0018 §5's "verified blast radius: nil" holds for `svc deploy`, and
is **wrong for the app path** (F1, §9.1).

### F11 — `roymctl registry register` is a third publication path, unmentioned by ADR-0018

[registry.rs:22-37](../../../../apps/roymctl/src/commands/registry.rs#L22)
signs and publishes an `EndpointInfo` directly, with its own `--private` flag,
never touching a deploy manifest. It is unaffected by this slice and stays
that way: it is an explicit operator action to publish, which *is* a
declaration, just not the manifest's. Worth one sentence in the ADR so a
reader does not conclude publication now has exactly one door (§9.5).

### F12 — the Tier-1 app record's `is_private` stays hardcoded `false`

`app_supervisor/src/tier1.rs` publishes the app instance's own record with
`is_private: false`, and there is no app-level visibility declaration
anywhere. ADR-0022 §5 is explicitly **per logical service**, so this is out of
B2's scope; the existing backlog row (deferred-backlog §*"The Tier-1 record's
`is_private` is hardcoded `false`"*) stays open. Exit criterion 10 is
unaffected: a published Tier-1 record only reveals *which node supervises this
app*, and a restricted logical service is still refused at `resolve`.

### F13 — the two declarations interact at Tier 3, and neither is sufficient alone

ADR-0022 §4: a topology document names **member master DIDs, never
addresses**. Turning a member DID into a location stays Tier 3 — an ordinary
registry lookup of that DID. So for an outside caller to actually *reach* an
`open` logical service:

| `topology_visibility` | `visibility` | What an outside caller gets |
|---|---|---|
| `restricted` | any | Refused at `resolve`. Cannot even see the member list. |
| `open` | `private` | The document, and then **nothing to dial** — every member DID fails Tier 3 with *"No valid Iroh mechanism found"* ([net_iroh.rs:144](../../../../crates/router/src/net_iroh.rs#L144)). |
| `open` | `internal` | Works, end to end, inside one community registry. |
| `open` | `public` | Works, and the member records also propagate to a parent registry. |

**`internal` is the right value for a multi-substrate app's members, not
`public`.** `is_private` gates *only* the parent-registry relay
([registry.rs:227](../../../../crates/community_registry/src/registry.rs#L227));
`lookup_endpoint` ([registry.rs:306-314](../../../../crates/community_registry/src/registry.rs#L306))
serves every admitted record regardless of it. So cross-node resolution inside
one registry costs nothing beyond "registered here", which is exactly what
`internal` means — **once F15 is fixed.**

### F15 — `internal` does not mean "registered here only" today: DHT publication ignores `is_private`

`RegistryClient::register`
([dht_registry.rs:289-336](../../../../crates/core/src/dht_registry.rs#L289))
publishes the pkarr packet to the Mainline DHT whenever `dht_client` is
present. **`is_private` is never read on that path.** `EndpointPublisher::
publish_service` goes through this function
([endpoint_publisher.rs:58](../../../../crates/core/src/endpoint_publisher.rs#L58)),
so with `substrate.enable_bep0044_dht = true` an `internal` record is
globally resolvable by anyone — the opposite of the tier's own definition.

This is not a documentation problem to caveat. `internal` means "registered
with this substrate's registry only; **not** propagated upward"
(ADR-0018 §1). A global DHT is strictly wider than a parent registry, so
publishing there is the same violation the relay gate already prevents,
through a second door nobody closed. ADR-0018 §4's table names only the
relay, which is how the gap survived the ADR (§9.6).

Nothing in the tree relies on the current behaviour: every code path
hardcodes `is_private: false`, and the only producer of a `true` is `roymctl
registry register --private` (F11), whose whole point is not to publish
widely. `D-B2-16` closes it.

This is why the two declarations are independent rather than one four-valued
field (`D-B2-1`): the useful pairs are three of the four above, and `(open,
private)` is a real, detectable operator mistake rather than a value the
type system should hide.

### F16 — dependency and shape state

- `syneroym-control-plane` already imports `SignedEndpointInfo` from
  `syneroym_core::dht_registry` (used by `stable_registry_certificate_for_hash`,
  [orchestration.rs:250](../../../../crates/control_plane/src/service/orchestration.rs#L250)),
  so the deploy-time validation needs **no new dependency**.
- `syneroym_sdk::Visibility` is the **WIT** enum (re-export,
  [lib.rs:35-40](../../../../crates/sdk/src/lib.rs#L35)); `mapper.rs` aliases
  the model one as `ModelVisibility`. Keep that convention.
- `is_private` composes exactly as ADR-0018 §4 states: it gates parent-registry
  propagation only ([registry.rs:226-231](../../../../crates/community_registry/src/registry.rs#L226)).
- The deploy dedup hash already covers `&manifest.config`
  ([orchestration.rs:1556](../../../../crates/control_plane/src/service/orchestration.rs#L1556)),
  so a redeploy that changes **only** visibility correctly reinstalls rather
  than no-op'ing. Nothing to add.

---

## §2 Decisions

> **Reviewer sign-off 2026-08-18.** The three open questions §9 raised are
> answered: (1) yes, an app declares publication per service and an
> undeclared member is not published — see `D-B2-3`, `D-B2-7`, `D-B2-14`;
> (2) `D-B2-8`'s split confirmed — ADR-0018 §2 ships, §3 defers;
> (3) two independent declarations, not one four-valued field — `D-B2-1`,
> and F13 for the interaction rule that falls out of it.

| # | Decision | Why |
|---|---|---|
| **D-B2-1** | **Two fields, two types, one slice.** Publication is `ServiceConfig.visibility: Visibility` (`public`/`internal`/`private`, default `private`). Resolution is `ServiceSpec`/`PlannedService`.`topology_visibility: TopologyVisibility` (`restricted`/`open`, default `restricted`). Neither reuses the other's enum. | They answer different questions with different cardinality. ADR-0018's middle tier (`internal` = registered here, not propagated) is meaningless for a topology document, which is not registered anywhere; ADR-0022 §5 is explicitly binary — *"open to all, or requiring a UCAN"* — and §5 forbids any partial answer. One shared enum would give each half a value it must reject at runtime, which is the shape the backlog already flags for `asset-bundle.visibility` (row: *"How `asset-bundle.visibility` reconciles with ADR-0018's…"*). This decision closes that row by keeping the enum reuse to the two questions that genuinely share three values. |
| **D-B2-2** | **The publication declaration lives on `ServiceConfig` (so it crosses the WIT and reaches the substrate); the resolution declaration lives on `ServiceSpec`/`PlannedService` (so it stops at the supervisor).** | Publication is validated and stored by the substrate — it must cross. Resolution is answered by the supervisor from its stored plan; a substrate never sees a topology document. Putting it in `ServiceConfig` would also make it part of the substrate's deploy dedup hash, so editing an access declaration would restart the service — the exact defect `ServiceSpec.schedule`'s own doc comment records ([models.rs:607-615](../../../../crates/app_orchestration/src/models.rs#L607)). |
| **D-B2-3** | **Undeclared = unpublished, changed in place. No shim, no dual-read, no `PRAGMA user_version` bump.** | task.md's Migration-impact section and ADR-0018 §5. The product is unreleased. The one loud consequence — `svc deploy --identity` without `--visibility` now **fails** — is intended: it is the difference between "you said public and gave me nothing to publish" and silence. |
| **D-B2-4** | **The substrate validates the declaration against the supplied certificate and refuses a deploy on any of four mismatches**, before any artifact work: (a) `public`/`internal` with no certificate; (b) `private` with a certificate; (c) certificate whose `info.is_private != (visibility == internal)`; (d) certificate whose `info.service_id != service_id`. | (a)–(c) are ADR-0018 §4's table, stated as deploy failures. (d) is the ADR's own "Do first" note, moved from `roymctl` to the substrate: doing it here covers **every** client, not just the one CLI, and turns a certificate the registry would reject forever-while-printing-success into an immediate, local error. `roymctl` gets the same check for a better message (`D-B2-11`). |
| **D-B2-5** | **A `private` deploy deletes any stored record file for that service.** | F2. Otherwise "make this private" leaves the substrate republishing the old record for up to 30 days, which would make failure-matrix row 11 pass on a fresh deploy and fail on the redeploy that matters. |
| **D-B2-6** | **The declared visibility is stored per service in `service_deploy_facts` and reported by `orchestrator/list` and `roymctl svc list`.** | ADR-0018 §4 requires it, and it is the whole complaint the ADR exists to fix: without it, an operator still cannot tell "deliberately private" from "forgot", they just cannot tell it from a different place. Widening `DeployFacts` is mechanical (F9). |
| **D-B2-7** | **`certify_placed_members` mints no record for a member whose declared visibility is `private`, and sets `is_private = (visibility == internal)` otherwise.** No signature change — it already takes `&DeploymentPlan`. | F1. This is where "undeclared = unpublished" actually becomes true for apps. Reading the same plan field the mapper reads means the two cannot disagree; a member is either absent from the returned map *and* absent from the manifest, or present in both. |
| **D-B2-8** | **B2 implements ADR-0018 §1 and §4 in full. ADR-0018 §2 (record export/import) ships as `roymctl svc deploy --record-out` plus `SyneroymClient::new_with_record`; §3 (the peer-substrate known-records store) is deferred with a backlog row.** Confirmed 2026-08-18. | §3 is a persisted, verified-on-load store threaded into `net_iroh::resolve_iroh_addr`, and it has **no exit criterion and no failure-matrix row** in this milestone. The ADR's own scope note shrinks it further: same-node siblings already resolve through the local registry first ([proxy.rs:422-426](../../../../crates/router/src/proxy.rs#L422)) and a `DeploymentPlan` deploys to one substrate, so the store is needed only for genuinely cross-node private targets, which nothing in the tree has today. §2 is ~70 lines and closes the real user-visible half ("I made it private and now my own client cannot reach it"). Shipping §2 without §3 is a defensible line; shipping neither is not, because then `private` means "unreachable" rather than "unlisted". |
| **D-B2-9** | **`D-B1-10` re-opened and re-affirmed: the Tier-2 fetcher keeps the node identity.** The `[iam].grant_resolve_to_node_did` gate stays exactly as it is; `open` is added **beside** it, not in place of it. | Now a choice, not an inheritance. Three reasons it stays: (1) the same-node gate answers a case `open` deliberately does not — an app's *own* operator resolving their *own* `restricted` services with no token; (2) switching the fetcher to the person's identity would make every app-scoped hostname depend on that person holding a grant, breaking F8's two components for the unauthenticated-browser case M06A A5 already proves; (3) `open` is per logical service, so it cannot replace a node-wide operator convenience. The gate's config doc gets one sentence saying `open` is now the other way in. |
| **D-B2-10** | **An `open` service is answered to any verified caller; an unknown app, a retired instance, an unknown service name, and a `restricted` service are all still refused identically.** The visibility read happens **before** the capability check and **cannot** widen any other refusal. | ADR-0022 §5 plus `handle_resolve`'s existing anti-enumeration property. An `open` declaration does leak the existence of *that* service to anyone who guesses its name — that is what "open to all" means, and it is the owner's declaration. Everything else stays indistinguishable. |
| **D-B2-11** | **`roymctl svc deploy` gains `--visibility <public\|internal\|private>` (default `private`) and `--record-out <path>`, and validates that `--identity`'s `did:key` equals `--svc-id` before building anything.** The new refusal reaches **`--master` too**, not only `--identity`: `signing_identity` falls back to the master ([svc.rs:237-238](../../../../apps/roymctl/src/commands/svc.rs#L237)), so a `--master` deploy with no `--visibility` now fails the same way. `--master` needs no DID check added — it already has one ([svc.rs:274-285](../../../../apps/roymctl/src/commands/svc.rs#L274)); that is why the new check is `--identity`-only. | ADR-0018 §1, §2, and its "Do first" note. `--visibility` is required because `svc deploy` builds a `DeployManifest` directly with no `SynAppManifest` to read a declaration from. `--asset-visibility` keeps its own name and meaning; the two are separate questions on one command and must not be merged. |
| **D-B2-12** | **The SDK pairs certificate and declaration in one type: `Publication { Private, Public(SignedEndpointInfo), Internal(SignedEndpointInfo) }`, replacing `registry_certificate: Option<SignedEndpointInfo>` on `DeploySvcOptions`, `deploy_svc_wasm`, `deploy_svc_tcp`, and `deploy_container`.** | The same 17 call sites have to be edited either way (they all pass `None` today). This shape makes ADR-0018 §4's three-row table unrepresentable-wrong at the client edge, and makes the substrate's validation a genuine second check rather than the only one. Fallback if the reviewer prefers minimal churn: an added `visibility: Option<Visibility>` parameter beside the existing certificate — same edit count, weaker invariant. |
| **D-B2-13** | **No new WIT enum, no new WIT package, and no change to `supervisor.wit`.** The only WIT edits are two added record fields (`service-config.visibility`, `deployed-service.visibility`). | F3, F6. `control-plane.wit` is absent from `wit/host/deps/` and is not imported by `host.wit`, so it drives no `wasm32-wasip2` guest build — this is the orchestrator's JSON-RPC contract, one file, two additive fields. Still verify the `wasm32-wasip2` build per the milestone gate. |
| **D-B2-14** | **A plan whose declarations contradict its own placement is refused, by one shared function over a `&DeploymentPlan`, called from two entry points**: `compile()` (the deploy client) **and** `handle_submit` (the supervisor). Two checks: (a) a service placed on a **different, explicitly named** substrate from a service that `depends_on` it, while that dependency declares `visibility = private`; (b) a service declaring `topology_visibility = open` while declaring `visibility = private`. | This is the whole answer to §9.1's *"is the silence a problem?"* On the `svc deploy` path `D-B2-3`'s change is loud — the deploy fails. On the app path it would not be: the deploy succeeds and the *first cross-node call* fails much later with `"No valid Iroh mechanism found for service <did>"`, naming neither visibility nor the manifest. **Over a plan, not a manifest, and at both entry points**: a supervisor receiving `plan-json` through `submit` never runs manifest validation (F6 makes the point for `topology_visibility`, and it cuts the other way here), so a manifest-only check would protect `roymctl app deploy` and leave the `submit` path — the one `reference_scenario_e2e` and `topology_document_e2e` use — silent. (b) is F13's `(open, private)` row: a document naming members nobody can dial. |
| **D-B2-15** | **`internal`, not `public`, is the documented answer for a multi-substrate app's members.** The `--visibility` help text, the developer guide, and every fixture in §4.3 say so. | F13: `is_private` gates only the parent-registry relay; `lookup` serves every admitted record. So cross-node resolution inside one community registry needs "registered here", which is exactly `internal`. Defaulting the *guidance* to `public` would push operators into propagating records upward for a reachability property `internal` already gives them — re-creating, in documentation, the over-publication this ADR exists to remove. |
| **D-B2-16** | **DHT publication is gated on `!is_private`**, in `RegistryClient::register` — one condition on the `if let Some(dht)` arm ([dht_registry.rs:324](../../../../crates/core/src/dht_registry.rs#L324)), covering every caller at once. | F15: today an `internal` record is published to the global Mainline DHT, so the tier `D-B2-15` recommends means the opposite of its own definition whenever `enable_bep0044_dht` is on. Gating in `register` rather than in `EndpointPublisher` is deliberate: it is the single function every publish path crosses, so no future caller can forget it, and `is_private` is already inside the signed payload it is handed. Blast radius nil — every code path hardcodes `is_private: false`, and the only producer of a `true` is `roymctl registry register --private`, for which this is a fix. |

| **D-B2-17** | **`register` reports failure when it published to no channel at all.** Today `http_success` starts as `self.registry_url.is_none()`, so a node with no HTTP registry returns `Ok(())` on the strength of the DHT arm — which `D-B2-16` now skips for a private record. Track whether *any* channel actually published, and return `Err` naming the cause when none did. | Without it, `D-B2-16` turns a supported configuration (`registry_url: None` + DHT, pinned by `a_self_signed_record_registers_to_the_dht_with_no_http_registry_configured`, [dht_registry.rs:1074](../../../../crates/core/src/dht_registry.rs#L1074)) into a **silent** no-op for exactly the tier `D-B2-15` tells operators to use. "Blast radius nil" was true of today's tree, where nothing produces `is_private: true`, and false of B2's own output. This also **gives `D-B2-16` its test** (§7.32): `dht_client` is a concrete `Option<pkarr::Client>` built inside `RegistryClient::new` ([:266](../../../../crates/core/src/dht_registry.rs#L266)) with no seam to inject a fake — which is why the existing DHT test can only assert `is_ok()` and says so. An `Err` on the registry-less private path is an **observable consequence of the gate itself**, so the wiring is proven without extracting a trait for one condition. Deliberately **not** made fatal at deploy: `deploy`'s publish is warn-only on purpose ([orchestration.rs:2233-2240](../../../../crates/control_plane/src/service/orchestration.rs#L2233)) — *"a registry that is down must not fail a deploy"* — and a missing registry gets the same treatment, surfacing at deploy and on every heartbeat sweep as a named warning rather than as silence. |

---

## §3 Exact type and signature changes

### 3.1 `crates/app_orchestration/src/models.rs`

**(a) `ServiceConfig` — one field**, appended after `assets` ([:501](../../../../crates/app_orchestration/src/models.rs#L501)):

```rust
    /// Whether this service's endpoint record is published, and how far it
    /// travels (ADR-0018 §1). Declared, never inferred from whether a
    /// certificate was supplied. Absent means `private`: publication is a
    /// privacy decision, and a default of `public` would preserve the exact
    /// accident of publishing because someone held a key.
    #[serde(default)]
    pub visibility: Visibility,
```

`Visibility` is unchanged (already `Default = Private`,
[:534-542](../../../../crates/app_orchestration/src/models.rs#L534)). No
`skip_serializing_if`: unlike `replicas`, this field's presence in serialized
plan JSON is a feature — a stored plan should say what it declared.

**(b) New enum**, beside `Visibility`:

```rust
/// Who may fetch a logical service's Tier-2 topology document (ADR-0022 §5).
///
/// Binary by construction, not three-valued like [`Visibility`]: a topology
/// document is never registered anywhere, so `internal`'s "registered here,
/// not propagated" has nothing to mean. §5 also forbids a filtered member
/// list -- a caller receives the whole member set and mode, or a clean
/// denial -- so there is no third answer to express.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TopologyVisibility {
    /// The caller must hold `supervisor/resolve` on `synapp:<app-did>`.
    /// Today's behaviour, and the default: access is asked for, not assumed.
    #[default]
    Restricted,
    /// Any verified caller may fetch this service's topology document, with
    /// no capability and no pre-installed token.
    Open,
}
```

**(c) `ServiceSpec` — one field**, after `schedule` ([:615](../../../../crates/app_orchestration/src/models.rs#L615)):

```rust
    /// Who may fetch this logical service's topology document (ADR-0022 §5).
    /// Part of the desired state, so it survives a supervisor handover --
    /// node-local supervisor config would be neither reproducible nor
    /// portable. Absent means `restricted`, which is what every manifest
    /// written before this field already means.
    #[serde(default, skip_serializing_if = "is_restricted")]
    pub topology_visibility: TopologyVisibility,
```

**(d) `PlannedService` — the same field**, after `sharding_strategy`
([:871](../../../../crates/app_orchestration/src/models.rs#L871)), with the
same attributes and a doc comment saying it is cloned from `ServiceSpec`
exactly as `topology_mode`/`schedule`/`sharding_strategy` are, and that the
supervisor holds no manifest so the plan is the only place it can read it
from.

**(e) One helper**, beside `is_zero_u32`/`is_one_u32`:

```rust
fn is_restricted(v: &TopologyVisibility) -> bool {
    matches!(v, TopologyVisibility::Restricted)
}
```

`skip_serializing_if` on (c)/(d) and not on (a) is deliberate: an unchanged
manifest's TOML and an unchanged plan's JSON stay byte-for-byte identical for
the resolution half (matching `replicas`/`sharding_strategy`), while the
publication half is always written because it is the fact an operator most
wants to read back.

### 3.2 `crates/wit_interfaces/wit/control-plane/control-plane.wit`

**(a) `service-config`** — one field, appended after `assets`
([:107](../../../../crates/wit_interfaces/wit/control-plane/control-plane.wit#L107)):

```wit
        /// Whether this service's endpoint record is published, and how far
        /// it travels (ADR-0018). `public` requires a `registry-certificate`
        /// whose `is_private` is false; `internal` requires one whose
        /// `is_private` is true; `private` requires none, and refuses one.
        /// Absent means `private` -- tolerant decoding of a field an older
        /// caller omits, not a compatibility concession.
        visibility: option<visibility>,
```

**(b) `deployed-service`** — one field ([:187](../../../../crates/wit_interfaces/wit/control-plane/control-plane.wit#L187)):

```wit
        /// The visibility this service was deployed with (ADR-0018 §4), so
        /// "deliberately private" is distinguishable from "forgotten" in a
        /// listing. Absent for a service deployed before this field existed.
        visibility: option<visibility>,
```

Also extend the `enum visibility` doc comment at
[:64-67](../../../../crates/wit_interfaces/wit/control-plane/control-plane.wit#L64)
— it currently says ADR-0018 "adds the `service-config` field" in the future
tense. Update to the shipped state.

### 3.3 `crates/sdk/src/mapper.rs`

`WitServiceConfig` construction ([:204-237](../../../../crates/sdk/src/mapper.rs#L204))
gains one field, mirroring `map_asset_bundle`'s existing style — the model
field always has a value after serde defaulting, so the wire always carries an
explicit one:

```rust
            visibility: Some(map_visibility(svc.config.visibility)),
```

and one shared function replacing the inline `match` currently inside
`map_asset_bundle` ([:114-119](../../../../crates/sdk/src/mapper.rs#L114)), so
both fields map through one definition:

```rust
const fn map_visibility(v: ModelVisibility) -> WitVisibility {
    match v {
        ModelVisibility::Public => WitVisibility::Public,
        ModelVisibility::Internal => WitVisibility::Internal,
        ModelVisibility::Private => WitVisibility::Private,
    }
}
```

`map_asset_bundle` then reads `visibility: Some(map_visibility(bundle.visibility))`.

### 3.4 `crates/app_orchestration/src/compiler.rs`

One line inside the `PlannedService { … }` literal
([:161-177](../../../../crates/app_orchestration/src/compiler.rs#L161)):

```rust
                    topology_visibility: spec.topology_visibility,
```

Placed beside `sharding_strategy`. `Copy`, so no `.clone()`.

### 3.5 `crates/sdk/src/deploy.rs` — `certify_placed_members`

Signature unchanged. Body change inside the per-service loop
([:727-750](../../../../crates/sdk/src/deploy.rs#L727)): the record block
becomes conditional on the member's own declaration. See §5.1 for the
pseudo-code, and note the instance certificate is **unaffected** — it is a
different artifact answering a different question (may this service
authenticate outbound calls), and a private service still needs one.

Doc comment gains: *"A member declaring `private` visibility gets no record at
all — the map simply has no entry for it, which `mapper` maps to
`registry_certificate: None`."*

### 3.6 `crates/sdk/src/lib.rs`

**(a) New type**, beside `DeploySvcOptions` ([:308](../../../../crates/sdk/src/lib.rs#L308)):

```rust
/// What a deploy declares about publishing this service's endpoint record
/// (ADR-0018 §4). One type rather than two loose fields, so the three legal
/// pairings are the only ones a caller can express -- the substrate still
/// validates independently, since a client's word is not a check.
#[derive(Debug, Default)]
pub enum Publication {
    /// Never registered. The default.
    #[default]
    Private,
    /// Registered, and propagated to parent registries. The record's own
    /// `is_private` must be `false`.
    Public(SignedEndpointInfo),
    /// Registered with the local registry only. The record's own
    /// `is_private` must be `true`.
    Internal(SignedEndpointInfo),
}

impl Publication {
    /// `(visibility, serialized certificate)` for a `DeployManifest`.
    fn split(self) -> Result<(Visibility, Option<String>)> { … }
}
```

**(b)** `DeploySvcOptions.registry_certificate: Option<SignedEndpointInfo>` →
`publication: Publication`.

**(c)** `deploy_svc_wasm`, `deploy_svc_tcp`, `deploy_container`: the
`registry_certificate: Option<SignedEndpointInfo>` parameter becomes
`publication: Publication`, and each of the three `ServiceConfig { … }`
literals gains `visibility` from `split()`.

### 3.7 `crates/control_plane/src/service/orchestration.rs`

**(a) New free function**, beside `stable_registry_certificate_for_hash`
([:250](../../../../crates/control_plane/src/service/orchestration.rs#L250)):

```rust
/// ADR-0018 §4: the substrate *validates* the declaration against the signed
/// artifact rather than deciding it -- `is_private` lives inside the
/// signature, so only the signer can set it. Returns the model `Visibility`
/// to record for this service.
fn validate_publication(
    service_id: &str,
    declared: Option<WitVisibility>,
    certificate: Option<&str>,
) -> Result<AppVisibility, String>
```

See §5.2. Called from `deploy` immediately after the instance-certificate
verification at [:1407-1415](../../../../crates/control_plane/src/service/orchestration.rs#L1407),
before the app-context work — so a mis-declared deploy fails before anything
touches storage.

**(b) `deploy`'s certificate-write block**
([:1589-1599](../../../../crates/control_plane/src/service/orchestration.rs#L1589))
gains an `else` arm that removes a stale file (`D-B2-5`, §5.3).

**(c) `set_deploy_facts` call** ([:2190-2198](../../../../crates/control_plane/src/service/orchestration.rs#L2190))
gains the validated visibility as a fourth recorded fact.

**(d) `write_bindings`' facts rewrite** ([:2403-2409](../../../../crates/control_plane/src/service/orchestration.rs#L2403))
destructures and re-writes four fields instead of three, preserving visibility.

**(e) `DeployedService` construction** ([:3152-3164](../../../../crates/control_plane/src/service/orchestration.rs#L3152))
gains `visibility: registry.deploy_facts(&service_id).and_then(|f| f.3).map(…)`.

**(f)** One `info!` at deploy naming the declared visibility, in the same
place and the same spirit as the asset bundle's
([:1892-1900](../../../../crates/control_plane/src/service/orchestration.rs#L1892)):
an author who did not mean to leave a service unpublished has exactly one
place to notice.

### 3.8 `crates/core/src/storage.rs` and `crates/core/src/local_registry.rs`

```rust
// storage.rs:19
/// (`service_type`, `health_check_json`, `manifest_hash`, `visibility`) --
/// what a deploy recorded about a service. `visibility` is `None` only for a
/// service whose row predates the column (ADR-0018).
pub type DeployFacts = (String, Option<String>, Option<String>, Option<String>);

// storage.rs -- EndpointStorage trait
async fn load_all_deploy_facts(
    &self,
) -> Result<Vec<(String, String, Option<String>, Option<String>, Option<String>)>>;

async fn save_deploy_facts(
    &self,
    service_id: &str,
    service_type: &str,
    health_check_json: Option<&str>,
    manifest_hash: Option<&str>,
    visibility: Option<&str>,
) -> Result<()>;

// local_registry.rs:355
pub async fn set_deploy_facts(
    &self,
    service_id: String,
    service_type: String,
    health_check_json: Option<String>,
    manifest_hash: Option<String>,
    visibility: Option<String>,
) -> Result<()>;
```

Visibility is stored as the lowercase string (`"public"`/`"internal"`/
`"private"`), matching how `service_type` is already stored via
`service_type_str`. `syneroym-core` deliberately does not depend on
`syneroym-app-orchestration` (deferred-backlog records why), so a string is
the right currency at this boundary — the same reasoning that put `public:
bool` on `ServiceAssets`.

### 3.9 `crates/data_db/src/registry_store.rs`

- `CREATE TABLE IF NOT EXISTS service_deploy_facts` ([:120](../../../../crates/data_db/src/registry_store.rs#L120))
  gains `visibility TEXT`.
- One idempotent `ALTER TABLE service_deploy_facts ADD COLUMN visibility TEXT`
  guarded on `"duplicate column name"`, immediately after the existing
  `manifest_hash` one ([:133-147](../../../../crates/data_db/src/registry_store.rs#L133)),
  reusing and extending that block's own comment. This is not a version
  ladder: there is no schema version tracked and nothing branches on it.
- `load_all_deploy_facts` / `save_deploy_facts` SQL widened by one column.
- `MockStorage` in `crates/core/src/storage.rs` widened identically.

### 3.10 `crates/app_supervisor/src/topology.rs`

One new pure function, beside `service_topology`:

```rust
/// The `topology_visibility` every member of `service_name` declares
/// (ADR-0022 §5). Members of one logical service carry identical copies --
/// the compiler clones the spec's value onto each -- so a disagreement is a
/// compiler defect, reported the same way `service_topology` reports one
/// rather than resolved to whichever member sorted first. **The safe answer
/// on disagreement is `Restricted`**: an inconsistent plan must never widen
/// access.
pub fn service_topology_visibility(
    plan: &DeploymentPlan,
    service_name: &LogicalServiceName,
) -> Result<TopologyVisibility, TopologyBuildError>
```

Alternative considered and rejected: adding the field to `ServiceTopology` and
returning it from `service_topology`. That would put the access answer behind
the epoch/fingerprint retry loop, which runs *after* the point where the
authorization decision has to be made (F5).

### 3.11 `crates/app_supervisor/src/service.rs` — `handle_resolve`

One block inserted between the state read
([:5341](../../../../crates/app_supervisor/src/service.rs#L5341)) and the
capability check ([:5349](../../../../crates/app_supervisor/src/service.rs#L5349)).
See §5.4. No other change: the epoch loop, the signing cache, the vault
handling, and every error mapping stay exactly as they are.

The method's doc comment gains one bullet:

```
/// - a logical service the plan declares `open` is answered to any verified
///   caller with no capability at all (ADR-0022 §5); every other refusal --
///   unknown app, retired instance, unknown service, `restricted` service --
///   stays indistinguishable from the others.
```

### 3.12 `crates/sdk/src/topology.rs`, `client_gateway/src/gateway.rs`, `coordinator_webrtc/src/coordinator.rs`

Text only. `credential_warning`'s `NeitherConfigured` message in both call
sites becomes:

```
"… neither `roles.<component>.resolve_ucan` nor `iam.grant_resolve_to_node_did`;
 app-scoped (-a…-s…) hostnames will be refused by any supervisor they reach,
 unless the app declares that logical service `open` (ADR-0022 §5). Unscoped
 (-s only) hostnames are unaffected."
```

and `CredentialWarning`'s own doc comment notes the third way in.

### 3.13 `crates/core/src/config.rs`

`grant_resolve_to_node_did`'s doc ([:1240-1257](../../../../crates/core/src/config.rs#L1240))
gains one sentence: this gate is the *operator-side* answer for a node's own
apps; a service the app itself declares `open` needs neither this nor
`resolve_ucan` (`D-B2-9`).

### 3.14 `apps/roymctl/src/commands/svc.rs`

**(a)** Two flags on `Deploy`:

```rust
        /// Whether this service's endpoint record is published (ADR-0018):
        /// "public" (registered and propagated), "internal" (registered
        /// with this substrate's registry only), or "private" (never
        /// registered; the default). `public`/`internal` require
        /// `--identity` or `--master`, since only the service's own key can
        /// sign a record the registry will admit.
        #[arg(long, default_value = "private")]
        visibility: String,
        /// Write the signed endpoint record to this path instead of relying
        /// on the registry (ADR-0018 §2). The file is a `SignedEndpointInfo`
        /// -- self-contained and independently verifiable -- to hand to
        /// whoever should be able to reach a `private` service.
        #[arg(long)]
        record_out: Option<PathBuf>,
```

**(b)** `parse_asset_visibility` is generalized to `parse_visibility(value,
flag_name)` and used by both flags — one parser, two flags, two error
messages that name the right flag.

**(c)** The certificate-building block ([:250-273](../../../../apps/roymctl/src/commands/svc.rs#L250)):
`is_private` becomes `visibility == Internal` instead of a literal `false`;
the record is built only when visibility is not `private`; `--identity`'s DID
is checked against `--svc-id` (`D-B2-11`); and the three `client.deploy_*`
calls pass a `Publication` instead of `cert`.

**(d)** `SvcCommands::List`'s table gains a `VISIBILITY` column.

### 3.15 `crates/sdk/src/lib.rs` — `new_with_record` (ADR-0018 §2, gated on `D-B2-8`)

```rust
/// Connect using a privately-shared endpoint record (ADR-0018 §2).
///
/// Supersedes `new_with_mechanisms` for every case that has a record: the
/// record is verified on import, names the substrate that hosts the service,
/// and that substrate's own record is public -- so a private service stays
/// reachable by anyone the deployer handed the file to, with no registry
/// entry for the service itself.
pub fn new_with_record(
    record: SignedEndpointInfo,
    registry_url: String,
) -> Result<Self>
```

Verifies (`record.verify()`), reads `info.substrate_id`, and constructs a
client whose lookups resolve **that** DID through the registry. The raw
`new_with_mechanisms` constructor stays for genuinely registry-less bootstrap
use, as the ADR requires.

### 3.16 `crates/app_orchestration` — one plan-level check, two entry points (`D-B2-14`)

**One function**, in `compiler.rs` beside `compile`:

```rust
/// Refuses a plan whose visibility declarations contradict its own
/// placement (ADR-0018 + ADR-0022 §5). Operates on the **plan**, not the
/// manifest: a supervisor receives `plan-json` through `submit` and never
/// sees a `SynAppManifest`, so a manifest-level check would leave that
/// whole path silent.
pub fn validate_plan_visibility(plan: &DeploymentPlan) -> Result<()>
```

**Two call sites**, which are the two places a plan is accepted:

| Call site | Why here |
|---|---|
| `compile()`, on each produced `DeploymentPlan` before it is returned ([compiler.rs:180-186](../../../../crates/app_orchestration/src/compiler.rs#L180)) | The client-side, early failure: `roymctl app deploy` and `roymctl supervisor submit` both compile locally first, so an operator sees the contradiction before anything is sent |
| `AppSupervisor::handle_submit`, immediately after `DeploymentPlan::from_json` ([service.rs:3897](../../../../crates/app_supervisor/src/service.rs#L3897)) | The backstop. A plan built programmatically, or by a client that is not `roymctl`, reaches the supervisor without ever having been compiled here. Failing the `submit` — rather than the reconcile that follows it — puts the error in front of the caller who can fix it |

Deliberately **not** called from `certify_placed_members`: by the time a plan
reaches it, both entry points have already run, and a third copy would be a
third thing to keep in sync. A hand-built plan in a test bypasses both — which
is exactly what test 34a needs in order to assert the runtime behaviour the
checks exist to prevent.

Both checks are **conservative by construction**. `PlannedService.substrate`
is `Option<SubstrateAlias>`, where `None` means "the substrate this deploy was
aimed at" ([models.rs:834-839](../../../../crates/app_orchestration/src/models.rs#L834))
— unknowable here. So check (a) compares two aliases only when **both** are
explicitly named and differ. A `None`-versus-`Some` pair may or may not be
cross-node, and falsely refusing a working plan is worse than a missed
warning: the runtime failure is still there as the backstop, and `D-B2-15`'s
documentation is the other half of the answer.

### 3.17 `crates/core/src/dht_registry.rs` — the gate and the honest result (`D-B2-16`, `D-B2-17`)

```rust
// register(), rewritten around one added local. `http_success`'s existing
// meaning is unchanged; `published` is what the return value now rests on.
let mut published = false;

if let Some(url) = &self.registry_url {
    …unchanged… // on success: http_success = true; published = true;
}
if !http_success {
    return Err(anyhow!("Failed to register endpoint via HTTP registry"));
}

// ADR-0018 §4: `is_private` means "not propagated beyond this registry".
// The Mainline DHT is a *wider* channel than a parent registry, so the same
// flag has to gate it -- otherwise `internal` means "global" on any node
// with `enable_bep0044_dht` on, which is the opposite of what it declares.
if let Some(dht) = &self.dht_client
    && !signed_info.info.is_private
{
    …unchanged… // publish_dht_packet(…); published = true;
}

// D-B2-17: every channel was either absent or excluded, so nothing was
// published and saying `Ok` would be a lie. Names both halves of the cause,
// because the fix is one or the other.
if !published {
    // Name the channel that is actually absent. Both branches are reachable
    // by configuration: the private one whenever `registry_url` is unset,
    // and the second on a node with `registry_url: None` AND
    // `enable_bep0044_dht: false` -- unusual, since the config default is
    // `!cfg!(test)` (config.rs:1141), but a node can be written that way,
    // and telling that operator their public record is "marked private"
    // would send them to fix the one thing that is correct.
    return Err(if signed_info.info.is_private {
        anyhow!(
            "nothing published for '{}': this node has no HTTP registry configured, \
             and a record marked private is deliberately not published to the DHT. \
             Configure `substrate.registry_url`, or declare this service `public`",
            signed_info.info.service_id
        )
    } else {
        anyhow!(
            "nothing published for '{}': this node has no HTTP registry configured \
             and no DHT client. Set `substrate.registry_url` or enable \
             `substrate.enable_bep0044_dht`",
            signed_info.info.service_id
        )
    });
}
Ok(())
```

Gated inside `register` rather than in `EndpointPublisher` because this is the
single function every publish path crosses (`publish_service`, `roymctl
registry register`, `roymctl app deploy`'s anchor refresh), so no future caller
can forget it, and `is_private` is already inside the payload it is handed.

The master-anchor path (`register_master`) is **not** gated: an anchor is not
an endpoint record, carries no `is_private`, and must stay resolvable — every
delegation-bearing connection resolves it (B1's F7).

**Where the `Err` surfaces**: `deploy`'s publish call warns and continues
([orchestration.rs:2233-2240](../../../../crates/control_plane/src/service/orchestration.rs#L2233)),
and `publish_all_services` warns per service on each hourly sweep
([endpoint_publisher.rs:70-73](../../../../crates/core/src/endpoint_publisher.rs#L70)).
So a misconfigured node warns at deploy and keeps warning — the same treatment
a permanently unreachable registry already gets, which is the right comparison:
both are "this record is not where you think it is".


---

## §4 Call sites

### 4.1 Mechanical, compiler-enumerated

| Change | Sites | Recipe |
|---|---|---|
| WIT `ServiceConfig` literal gains `visibility: None` | **85** (F4) | The added field is last in the record, and every literal already ends with `assets: …`. Edit the **4 production sites by hand first** (`sdk/src/lib.rs` ×3, `sdk/src/mapper.rs` ×1) — they get a **real** value, not `None` — then add `visibility: None,` after `assets` in the 81 test/bench literals and run `cargo check --workspace --all-targets` until clean. |
| model `ServiceConfig` literal gains `visibility: Visibility::Private` | **38** (F4) | Same approach; `assets` is the last field there too. Fixtures that must publish (F10) get `Visibility::Public`. |
| `DeployFacts` destructuring widened | ~15 | `deploy_facts(&id)` patterns already use `(t, _, _)` / `(_, Some(c), _)` / `(recorded_type, ..)`. Only the fixed-arity ones need a fourth `_`. |
| `save_deploy_facts` / `set_deploy_facts` calls | 4 non-test + ~8 test | Add the trailing argument. |
| `deploy_svc_wasm` / `deploy_svc_tcp` / `deploy_container` callers | **17** (`substrate_ownership_e2e` ×3, `proxy_outbox_e2e` ×4, `basic_lifecycle` ×3, `saga_e2e`, `messaging_client_e2e`, `stream_client_e2e` ×2, `podman_lifecycle`, `roymctl svc.rs` ×2) | `None` → `Publication::Private`; `Some(cert)` → `Publication::Public(cert)`. |
| `DeploySvcOptions { registry_certificate: … }` | 2 (`roymctl svc.rs`, `sdk` internal) | → `publication:` |

### 4.2 Behavioural — each needs a decision, not a mechanical edit

| Site | Change |
|---|---|
| [dht_registry.rs:324](../../../../crates/core/src/dht_registry.rs#L324) `register` | Gate DHT publication on `!is_private` (§3.17). |
| [sdk/src/deploy.rs:727](../../../../crates/sdk/src/deploy.rs#L727) `certify_placed_members` | Skip the record for `private`; `is_private` from the declaration (§5.1). |
| [orchestration.rs:~1415](../../../../crates/control_plane/src/service/orchestration.rs#L1415) `deploy` | Call `validate_publication`; fail early (§5.2). |
| [orchestration.rs:1589](../../../../crates/control_plane/src/service/orchestration.rs#L1589) | Delete a stale record file on `private` (§5.3). |
| [app_supervisor/src/service.rs:5345](../../../../crates/app_supervisor/src/service.rs#L5345) `handle_resolve` | Open-visibility bypass of the capability check (§5.4). |
| [compiler.rs:173](../../../../crates/app_orchestration/src/compiler.rs#L173) | Clone `topology_visibility` onto each member. |
| [roymctl svc.rs:257](../../../../apps/roymctl/src/commands/svc.rs#L257) | Build the record from the declaration; `--record-out`; `--identity`/`--svc-id` equality check. |
| [gateway.rs:143](../../../../crates/client_gateway/src/gateway.rs#L143), [coordinator.rs:104](../../../../crates/coordinator_webrtc/src/coordinator.rs#L104) | Warning text (§3.12). |

### 4.3 Fixtures that must declare visibility to keep passing

**F10's full set — seven distinct files across three groups, plus two
checked-clean.** `binding_push_e2e` and `multi_substrate_placement_e2e` each
appear in two groups, which is why the rows below add to more than seven. Do
not shorten this list to the certificate-bearing ones: each group fails
differently, and only (A) is findable by grepping for certificates.

- **(A) certificate with no declaration** — refused by `validate_publication`
  at deploy: `master_endpoint_record_e2e.rs`,
  `multi_substrate_placement_e2e.rs` (×2 sites), `binding_push_e2e.rs`.
- **(B) cross-substrate `depends_on` with an undeclared dependency** —
  refused by `D-B2-14`(a) inside `compile()`, before any deploy:
  `reference_scenario_e2e.rs`, `binding_push_e2e.rs`, `durable_outbox_e2e.rs`,
  `multi_substrate_placement_e2e.rs`.
- **(C) nothing refused; the record just stops existing** — the dial fails at
  runtime: `gateway_hostname_e2e.rs`, `topology_document_e2e.rs`. **Repaired
  in P4 like every other group** — §3.5 is what breaks them, and P4's gate is
  a green full suite. Tests 41–44, which are *written on* these two fixtures,
  are P5's.
- **Checked, not affected** (do not re-check): `supervisor_interface_e2e.rs`,
  `supervisor_loop_e2e.rs` — neither dials a member by DID.

Run [reference_scenario_e2e.rs](../../../../crates/substrate/tests/reference_scenario_e2e.rs)
**first**: it is the M05A reference scenario, it deploys through `submit`
rather than `ApplyRequest`, and it is the only one that dies at
`compiled_plan_json`'s unwrap instead of at a deploy — so it exercises both
`D-B2-14`'s entry point and the record it needs for its own cross-node call.
Every fixture in all three groups declares `internal`, not `public` (`D-B2-15`).

---

## §5 Pseudo-code

### 5.1 `certify_placed_members`, per-service loop

```python
# Inside `for svc in &plan.services`, after the instance certificate is
# minted and inserted -- the instance certificate is unconditional, because
# a private service still authenticates its outbound calls.

if svc.config.visibility == Visibility::Private:
    # No record at all. Not an empty record, not an unpublished one: the
    # map simply has no entry, so `mapper` maps this member's
    # `registry_certificate` to `None`, and the substrate's own validation
    # (§5.2, case b) then agrees with the declaration instead of refusing it.
    continue                      # -> the next member

record = EndpointInfo {
    service_id:     svc.service_id,
    substrate_id:   client.service_id(),
    endpoint_type:  Service,
    mechanisms:     vec![],       # grafted on at lookup (ADR-0018's finding)
    nickname:       None,
    # The one line ADR-0018 §4 is about: `is_private` lives INSIDE the
    # signature, so only this signer can set it, and the substrate can only
    # check that it agrees with what was declared.
    is_private:     svc.config.visibility == Visibility::Internal,
    ttl:            None,
    not_after:      now + DEFAULT_ENDPOINT_NOT_AFTER_SECS,
    generation:     0,
}.sign(master)
records.insert(svc.service_id, json(record))
```

### 5.2 `validate_publication`

```python
# `declared` is the wire value; absent means private, exactly as if the
# caller had said so (ADR-0018 §1: tolerant decoding, not a concession).
v = declared.unwrap_or(Private)

match (v, certificate):
    (Private, None):
        return Ok(Private)

    (Private, Some(_)):
        # Refuse rather than guess which the operator meant. Publishing
        # anyway would reinstate the accident this whole ADR removes;
        # dropping the certificate silently would throw away a signature the
        # operator deliberately produced.
        return Err("service '{id}' declares visibility 'private' but a registry
                    certificate was supplied -- declare 'public' or 'internal',
                    or deploy without the certificate")

    (Public | Internal, None):
        # The complaint the ADR exists to fix: "you said public and gave me
        # nothing to publish" now says so instead of staying silent.
        return Err("service '{id}' declares visibility '{v}' but no registry
                    certificate was supplied -- a record must be signed by the
                    service's own key, which this substrate does not hold")

    (Public | Internal, Some(json)):
        # A parse failure is a deploy error here, not a warning at publish
        # time 30 seconds later on a heartbeat nobody is watching.
        signed = parse::<SignedEndpointInfo>(json)
                 or return Err("registry certificate for '{id}' does not parse: {e}")

        # (d): catches a certificate minted for a different service -- today
        # that deploy prints success and the registry rejects `/register`
        # forever. Checked here rather than only in `roymctl` so every client
        # gets it (D-B2-4).
        if signed.info.service_id != service_id:
            return Err("registry certificate for '{id}' names service
                        '{signed.info.service_id}' -- it would be rejected by the
                        registry, which resolves the signing key from that field")

        # (c): the declaration and the signed artifact must agree. `internal`
        # is the ONLY value that means `is_private: true`.
        want_private = (v == Internal)
        if signed.info.is_private != want_private:
            return Err("service '{id}' declares visibility '{v}', but its registry
                        certificate carries is_private={signed.info.is_private};
                        the record is signed, so this can only be fixed by
                        re-signing it")

        # Deliberately NOT verified here: `record.verify()` resolves the
        # signer's key and is the registry's own admission check
        # (`verify_endpoint_signature`). `EndpointPublisher::build_record`
        # already refuses to republish a record that stops verifying. Adding a
        # third copy would put a network-dependent check on the deploy path.
        return Ok(v)
```

### 5.3 `deploy`'s record-file write

```python
# Replaces the `if let Some(cert) = &manifest.registry_certificate` block.
match &manifest.registry_certificate:
    Some(cert):
        write(hosted_apps_dir / f"{service_id}.json", cert)      # as today
    None:
        # D-B2-5. `validate_publication` already proved visibility is
        # `private` here, so a stored record is a leftover from an earlier,
        # public deploy of the same service_id. Leaving it means the
        # heartbeat keeps republishing a record the operator has just
        # declared private -- for up to `not_after` (30 days by default).
        # Same treatment `undeploy` already gives the file; a failure is
        # warned, not fatal, exactly as the write's own failure is.
        path = hosted_apps_dir / f"{service_id}.json"
        if path.exists() and remove_file(path) is Err(e):
            warn("failed to remove the stored endpoint record for {service_id}
                  after a private redeploy; it may keep being republished: {e}")
```

### 5.4 `handle_resolve`'s open-visibility check

```python
# Between the `state` read and the capability check. `state.plan_json` is
# already in hand; this adds no store read and no lock.

# Resolve the supplied name the SAME way the epoch loop below does -- exact
# name or `short_hash` -- so a caller reaching an `open` service by its
# hashed gateway segment is treated identically to one using its real name.
open_to_all = False
if let Ok(plan) = DeploymentPlan::from_json(&state.plan_json):
    if let Ok(name) = topology::resolve_service_name(&plan, &service_name):
        # Any error here (no such service, ambiguous hash, inconsistent
        # plan) leaves `open_to_all` false. That is the safe direction: an
        # unresolvable name must never widen access, and an UNAUTHORIZED
        # caller must not learn which of those errors it hit -- it gets
        # `denied()` below, same as always. An AUTHORIZED caller still gets
        # today's specific `InvalidParams` from the loop further down,
        # unchanged, because this block never returns an error of its own.
        open_to_all = topology::service_topology_visibility(&plan, &name)
                          .map(|v| v == TopologyVisibility::Open)
                          .unwrap_or(False)

# NOTE the real order, which is not the obvious one: `state.retired` is
# checked AFTER `has_capability` today ([:5350] then [:5361]), not before.
# Leave it there -- do not reorder. The bypass below can let an ungranted
# caller past the capability check on a retired instance, and the retired
# check three lines later still refuses it, so test 20 holds with a
# minimal diff. Reordering would be a behaviour-neutral churn in a function
# whose ordering carries several other deliberate properties.

if not open_to_all and not caller.has_capability(
        ResourceUri(f"synapp:{app_did}"),
        Ability(SUPERVISOR_RESOLVE)):
    return Err(denied())

# ... everything below is unchanged: the two-attempt epoch loop, the locked
# repair, the signature cache, the vault read, the alerts, the signing.
```

Two properties worth stating explicitly, because they are what makes this
three lines instead of a redesign:

- **The document is unchanged.** An `open` caller receives byte-identical
  bytes to a granted one, from the same signature cache. ADR-0022 §5 forbids a
  filtered member list, and this design cannot produce one — there is no
  branch after the check.
- **Nothing else becomes distinguishable.** An unknown app, a retired
  instance, an unknown service name, and a `restricted` service all still
  return the same `denied()`. Only a service whose owner declared it `open`
  answers, which is the declaration's entire meaning.

### 5.5 `roymctl svc deploy`'s record and publication

```python
visibility = parse_visibility(visibility_flag, "--visibility")

# The ADR's "Do first", now enforced: a mismatch here silently builds a
# certificate the registry rejects at /register forever, while printing
# success.
if let Some(id) = &named_identity:
    if derive_did_key(id.public_key()) != svc_id:
        bail("--identity resolves to {did}, which is not --svc-id {svc_id};
              the registry resolves a record's signing key from its own
              service_id, so this record could never be admitted")

publication = match (visibility, signing_identity):
    (Private, _):
        Publication::Private
    (Public | Internal, None):
        # Caught locally with the flag names in it, rather than as a
        # substrate error two round-trips later.
        bail("--visibility {v} needs --identity or --master: only the
              service's own key can sign a record the registry will admit")
    (v, Some(id)):
        record = EndpointInfo { …, is_private: v == Internal, … }.sign(id)
        if let Some(path) = record_out:
            write(path, json(record))          # ADR-0018 §2's export
            print("wrote the signed endpoint record to {path}")
        Publication::Public(record) if v == Public else Publication::Internal(record)

client.deploy_svc_wasm_with_options(…, DeploySvcOptions { publication, … })
```

Note `--record-out` writes the record whenever one is signed, including for
`public`/`internal` — the ADR says "instead of (or as well as)". A `private`
service with `--record-out` and an identity is the interesting case and it
works: the record is signed and written, and nothing is sent to the substrate.

### 5.6 `validate_plan_visibility` (`D-B2-14`)

```python
# Index once: `resolved_dependencies` names member ServiceIds, and the
# placement being compared belongs to the member those ids point at.
by_id = { svc.service_id: svc for svc in &plan.services }

for svc in &plan.services:
    # --- (a) a private dependency on another substrate can never be dialled ---
    #
    # Only fires when BOTH placements are explicitly named and differ. `None`
    # means "wherever this deploy was aimed", which this function cannot
    # resolve -- refusing on a maybe would reject working plans.
    for (dep_name, members) in &svc.resolved_dependencies:
        for member_id in members:
            dep = by_id.get(member_id)
            if dep is None:
                continue          # a cross-app member; not this check's business
            if svc.substrate.is_some() and dep.substrate.is_some()
               and svc.substrate != dep.substrate
               and dep.config.visibility == Visibility::Private:
                return Err("'{svc.logical_ref}' on substrate '{svc.substrate}'
                            depends on '{dep_name}' on substrate
                            '{dep.substrate}', but '{dep_name}' declares
                            visibility 'private' -- its endpoint record is never
                            registered, so this dependency could never resolve to
                            an address. Declare 'internal': registered with the
                            community registry, not propagated upward, which is
                            what a cross-substrate member needs")

    # --- (b) `open` topology over unpublished members names nobody dialable ---
    #
    # F13's (open, private) row: the caller receives a signed member list and
    # then fails Tier 3 on every entry.
    if svc.topology_visibility == Open and svc.config.visibility == Private:
        return Err("'{svc.logical_ref}' declares topology_visibility 'open' but
                    visibility 'private' -- an outside caller would receive its
                    member list and then be unable to resolve any member to an
                    address, because a private member is never registered.
                    Declare visibility 'internal' alongside it")
```

Note (a) walks `resolved_dependencies` rather than the manifest's
`depends_on`: the plan is what both entry points hold, and the compiler has
already expanded each declared dependency into its member list, so a scaled
dependency is checked per member with no extra logic.

Deliberately **not** checked here: a service declaring `open` in an app that
is never deployed to more than one substrate. That is not a contradiction —
it is an app whose owner has decided outsiders may resolve it, which is
exactly the declaration's purpose.

---

## §6 Phases

Each phase leaves the workspace compiling and every test green. P1–P4 are the
publication half; P5 is the resolution half and depends on nothing in P1–P4.

| # | Phase | Contents |
|---|---|---|
| **P1** | **The declarations** | §3.1 (model fields + `TopologyVisibility`), §3.2 (two WIT fields), §3.3 (mapper), §3.4 (compiler), plus the 123 mechanical literal edits (§4.1). Nothing reads the new fields yet. Green build is the gate. |
| **P2** | **Substrate validation and the record file** | §3.7(a)(b)(f), §5.2, §5.3. The four refusals and the stale-file delete. `roymctl` and the sdk still send `None`, so this phase is exercised only by new unit tests. |
| **P3** | **Storage and reporting** | §3.8, §3.9, §3.7(c)(d)(e), plus `roymctl svc list`'s column. |
| **P4** | **The client side** | §3.5 (`certify_placed_members`), §3.6 (`Publication`), §3.14 (`roymctl` flags), §3.16 (`validate_plan_visibility` and its two entry points), §3.17 (the DHT gate), §4.3 (**all three fixture groups, group (C) included**). **This is the phase that changes behaviour** — land §3.16 *before* §3.5 within the phase, so the loud failure exists before the silent one becomes possible. §3.17 is independent of the rest of P4 and can land first. Group (C) is broken by §3.5, which lands here, so its two `visibility = "internal"` declarations land here too — this phase's gate is a green **full suite**, which it cannot be otherwise. Only the new tests *written on* those fixtures (41–44) wait for P5. |
| **P5** | **Resolution: "open to all"** | §3.10, §3.11, §3.12, §3.13, plus tests 41–44 — written on the two group-(C) fixtures P4 has already repaired. Independent of P1–P4 except for `TopologyVisibility` (P1) and those repairs (P4). |
| **P6** | **ADR-0018 §2's export/import** (gated on `D-B2-8`) | §3.15 (`new_with_record`), `--record-out`'s consumer test. |
| **P7** | **Documentation and completion pass** | §8's document edits, deferred-backlog rows, `status.md`, the import cleanup pass, `cargo +nightly fmt --all`, clippy, `cargo test --workspace`, `mise run test:e2e`, and a `wasm32-wasip2` build check (§3.2's WIT edit is not guest-facing, but the milestone gate asks for it). |

---

## §7 Tests

Numbered so a reviewer can map each to a failure-matrix row or exit criterion.
Every row of the milestone's matrix that B2 owns is row 11; exit criterion 10
is B2's alone.

### Unit — `validate_publication` (`control_plane`, table-driven)

1. `private` + no certificate → `Ok(Private)`.
2. `None` (absent) + no certificate → `Ok(Private)` — **absence must not mean publish** (failure-matrix row 11's first half).
3. `public` + no certificate → `Err`, message names the missing certificate.
4. `internal` + no certificate → `Err`.
5. `private` + certificate → `Err`, message says declare or drop.
6. `public` + certificate with `is_private: true` → `Err`.
7. `internal` + certificate with `is_private: false` → `Err`.
8. `public` + certificate for a different `service_id` → `Err` (`D-B2-4`(d)).
9. `internal` + certificate with `is_private: true` → `Ok(Internal)`.
10. unparseable certificate + `public` → `Err`, and the error names parsing.

### Unit — `certify_placed_members` (`sdk`)

11. A plan with one `private` and one `public` member returns exactly one record, keyed by the public member.
12. An `internal` member's record carries `is_private: true`; a `public` member's carries `false`.
13. Every member's **instance** certificate is minted regardless of visibility.

### Unit — `validate_plan_visibility` (`app_orchestration`, `D-B2-14`)

14. Two services on explicitly different aliases, the dependency `private` → `Err`, and the message names both services and `internal`.
15. The same pair with the dependency `internal` → `Ok`.
16. The same pair with **no** explicit placement on either → `Ok` (no false positive on an unresolvable `None`).
17. One `Some(a)`, one `None`, dependency `private` → `Ok` (conservative; the runtime failure is the backstop).
18. `topology_visibility = open` with `visibility = private` → `Err`, message names both fields. *(F13's (open, private) row)*
19. `topology_visibility = open` with `visibility = internal` → `Ok`.

20. The same contradiction inside a plan handed to **`handle_submit`** is refused there too — the backstop entry point, since a plan reaching a supervisor was never compiled here (`D-B2-14`).

### Unit — `service_topology_visibility` (`app_supervisor`)

21. A plan declaring `open` returns `Open`; an undeclared plan returns `Restricted`.
22. Members disagreeing returns `Err(InconsistentPlan)` — and the caller treats it as `Restricted` (§5.4's `unwrap_or(False)`).
23. A `short_hash`-resolved name reaches the same answer as the exact name.

### Unit — `handle_resolve` (`app_supervisor`, existing harness)

24. An ungranted caller resolving an `open` service receives the signed document.
25. An ungranted caller resolving a `restricted` service is refused — **unchanged**, the existing test.
26. An ungranted caller naming a service that does not exist gets the same refusal as 25 (no enumeration).
27. An ungranted caller resolving an `open` service on a **retired** instance is refused — the check that runs *after* the bypass (§5.4).
28. A granted caller naming a non-existent service still gets `InvalidParams`, not `denied` — today's behaviour, pinned so §5.4's `unwrap_or` cannot silently coarsen it.
29. The document served to an ungranted `open` caller is byte-identical to the one served to a granted caller (ADR-0022 §5's no-filtering rule).

### Unit — storage and DHT gating (`data_db`, `core`)

30. `save_deploy_facts` with a visibility, reopen the file, `load_all_deploy_facts` returns it.
31. A row written before the column exists loads as `None` (the `ALTER TABLE` path).
32. **The DHT gate, proven without a fake** (`D-B2-16`/`D-B2-17`). `dht_client` is a concrete `Option<pkarr::Client>` with no injection seam, so the gate is asserted through its observable consequence, in three cases against `RegistryClient::new(true, None)` — DHT enabled, no HTTP registry:
    - a record with `is_private: false` → `Ok` (this is the existing `a_self_signed_record_registers_to_the_dht_with_no_http_registry_configured`, unchanged — proving the gate discriminates on `is_private` and nothing else);
    - a record with `is_private: true` → **`Err`**, and the message names both the missing registry and the private flag;
    - the same private record against `RegistryClient::new(true, Some(url))` with a live test registry → `Ok`, and the record is retrievable by `lookup` (so `internal` still publishes where it is supposed to).

### Integration / e2e

33. **`svc deploy` with no `--visibility` and no `--identity`** deploys and publishes nothing — `hosted_apps_dir` has no file and a registry lookup misses. *(exit criterion 10, first half; failure-matrix row 11)*
34. **`svc deploy --identity X` with no `--visibility`** fails, loudly, naming `--visibility`. *(ADR-0018 §5's one intended behaviour change)*
35. **`svc deploy --identity X --visibility public`** publishes, and the record resolves through the registry.
36. **A public service redeployed as private** stops being published: the stored file is gone and the next heartbeat sweep publishes nothing for it. *(`D-B2-5` — the case F2 found)*
37. **An app deployed with no declaration publishes no member records**, and a cross-node call to one of its members fails to resolve — the honest consequence of `D-B2-3`, asserted rather than discovered. Built by placing both services on the **same** alias (so `D-B2-14`'s check does not fire) and then dialling the member DID directly, which is the shape that isolates "not published" from "manifest refused". *(new test in `multi_substrate_placement_e2e.rs`)*
38. **The same app with `visibility = "internal"` on both services** resolves cross-node exactly as it does today. *(the existing test, re-pointed — `internal`, not `public`, per `D-B2-15`, which also proves F13's claim that `is_private` does not affect `lookup`)*
39. **The same app with `visibility = "public"`** also resolves, and its records additionally reach a parent registry — pinning that the two tiers stay distinguishable.
40. **`svc list` distinguishes a private service from a public one.** *(ADR-0018 §4's `list` requirement)*
41. **A caller on an unaffiliated installation, holding no token and no grant, resolves an `open` logical service** — the caller identity is freshly generated, and the supervisor node has `grant_resolve_to_node_did = false` and no `admin_ucan_root` grant for it. *(exit criterion 10, second half; reference-scenario step 13)* — new test in [topology_document_e2e.rs](../../../../crates/substrate/tests/topology_document_e2e.rs), which already boots two independent nodes and already builds an outside-caller identity; the new test is that fixture **minus** the `supervisor/resolve` grant its helper issues today ([:278-281](../../../../crates/substrate/tests/topology_document_e2e.rs#L278)).
42. **The same caller is refused for a `restricted` service on the same app** — one app, two services, two answers, proving the declaration is per logical service and not per app.
43. **Through the client gateway**: an app-scoped hostname for an app this gateway holds no grant for **succeeds** when the service is `open`, and still fails when it is `restricted`. The existing `an_app_scoped_hostname_for_an_app_this_gateway_holds_no_grant_for_is_refused` ([gateway_hostname_e2e.rs:736](../../../../crates/substrate/tests/gateway_hostname_e2e.rs#L736)) becomes the negative half and keeps passing unchanged.
44. **Both declarations are needed and each is load-bearing** (F13): the same fixture as 43, with the service `open` but `private`, resolves the document and then **fails to reach any member**. This is the one test that proves the two fields are not redundant, and it is the reason `D-B2-14`(b) refuses the pairing at compile — so this test asserts against a plan constructed directly, bypassing both of `D-B2-14`'s entry points, and says so in a comment.
45. *(gated on `D-B2-8`)* **A `private` service deployed with `--record-out`** is unreachable through a registry lookup, and reachable by `SyneroymClient::new_with_record` holding the exported file.

---

## §8 Documents this slice edits

| Document | Edit |
|---|---|
| [ADR-0018](../../../decisions/0018-service-record-visibility.md) | **Owed by task.md.** Status note: "Implemented by M06B slice B2" → the shipped state, which under `D-B2-8` reads *"§1 and §4 implemented by M06B B2 (date); §2's export/import shipped with it; §3's known-records store deferred — see deferred-backlog"*. Also: correct the Context section, which describes only the `svc deploy` path (F1/§9.1); correct §1's `snake_case` to the shipped `lowercase` (§9.4); correct §5's "verified blast radius: nil", which is true of `svc deploy` and false of the app path (§9.1); add one line noting `roymctl registry register` is a separate, unaffected publication path (F11); and **correct §4's table, which defines `internal` against the parent-registry relay alone and does not mention the Mainline DHT** — a second, wider publication channel that ignored `is_private` entirely until `D-B2-16` (F15, §9.6). |
| [ADR-0022](../../../decisions/0022-two-tier-logical-service-discovery.md) | §5 gains an implementation note naming the field, its default, and the slice — the same treatment §1 and §7's amendments already carry. Status stays `Proposed` (that is a whole-ADR question, not B2's). |
| [task.md](task.md) | B2's row → Complete, with links to this plan and `status.md`. **Exit criterion 10's wording split into its two declarations** (§9.3): "a service that declares no visibility is not published, and not reachable across installations; one that declares `topology_visibility = open` *and* a registered visibility is resolvable by a caller on an unaffiliated installation with no pre-installed token". **Reference-scenario step 13 carries the identical conflation** — *"A service declares `visibility: open`"* ([task.md:272](task.md)) — and gets the same split. The Migration-impact bullet gains the app-path consequence (§9.1). |
| [status.md](status.md) | A `B2 — What shipped` section and its evidence, in B1's format. |
| [deferred-backlog.md](../../deferred-backlog.md) | **To "Recently resolved"**: *"Declared service-record visibility is still only Proposed"* (line 130), *"`resolve`'s visibility is a capability check with no manifest declaration"* (line 268), *"A gateway or coordinator can only resolve a *remote* app whose operator issued it a grant"* (line 274), *"M05C S4 bundles two halves on a gate only one of them clears"* (line 129 — its visibility half is now shipped; the `Bind` half stays, so this row is **split**, not moved), and *"How `asset-bundle.visibility` reconciles with ADR-0018's `service-config.visibility`"* (line 147, answered by `D-B2-1`). **New rows**: §10's four. **Unchanged**: the Tier-1 `is_private` row (F12) and the `ServiceAssets` `public: bool` row (line 146). |
| [developer-guide.md](../../../developer-guide.md) | The `svc deploy` section gains `--visibility`/`--record-out`; the ports/config section's `grant_resolve_to_node_did` note gains the `open` alternative. |
| [roym-integrated-experience-spec.md](../../../roym-integrated-experience-spec.md) | G4's two halves marked shipped. Owed at **milestone close**, not per slice — recorded here so it is not lost. |

---

## §9 Ambiguities and stale statements in the input documents

Flagged rather than guessed at. Items 1–3 were open questions and are now
**resolved** (reviewer, 2026-08-18); their resolutions are kept here because
each one leaves an edit owed to an input document. The rest are corrections
this plan already applies.

1. **Resolved — an app declares publication per service, with no exemption
   for app-deployed members.** ADR-0018's Context and §5 are written as if
   `svc deploy` were the only publication path; it is not
   (`certify_placed_members` publishes every placed member unconditionally
   with `is_private: false`, F1). Two consequences the ADR does not carry, and
   must:
   - The blast radius is **not nil**. Every app-deployed member is published
     today and stops being published unless its manifest declares
     `internal`/`public`. Existing fixtures break (F10), and a multi-substrate
     app whose manifest declares nothing stops resolving across nodes — a
     member's registry record is the only thing that turns its DID into an
     address.
   - The failure would be **quiet on this path**: the deploy succeeds and the
     first cross-node call fails much later with *"No valid Iroh mechanism
     found"*, naming neither visibility nor the manifest. `D-B2-14` is the
     answer — the contradiction is statically detectable, so the deploy client
     refuses it at compile time with a message that names the fix.

   Two things make the strict reading the right one rather than merely the
   safe one. The alternative — exempting app-deployed members — would mean
   `private` has a different meaning depending on which verb deployed the
   service, which is the same class of implicitness ADR-0018 exists to remove.
   And `internal` (not `public`) is what a cross-substrate member actually
   needs (F13, `D-B2-15`), so the cost of the strict reading is one declared
   word per service, not a privacy concession. **Owed**: ADR-0018's Context
   and §5 corrected (§8).

2. **Resolved — `D-B2-8`'s split confirmed: ADR-0018 §2 ships, §3 defers.**
   Neither §2 nor §3 has an exit criterion, a failure-matrix row, or a mention
   in task.md's B2 row, and the ADR calls §3 "the load-bearing part". The
   deferral is therefore a real narrowing of an Accepted ADR and must be
   visible in three places, not one: ADR-0018's status note (§8), a
   deferred-backlog row (§10), and this paragraph. If §3 is ever pulled back
   in, it is a `known_records` table in `registry_store.rs`, verify-on-import
   and verify-on-load, an import verb in `roymctl`, and threading through
   `net_iroh::resolve_iroh_addr`
   ([net_iroh.rs:128](../../../../crates/router/src/net_iroh.rs#L128)) — the
   single chokepoint, as the ADR says. Roughly one phase the size of P3.

3. **Resolved — two independent declarations, not one four-valued field.**
   task.md's exit criterion 10 mixes ADR-0018's enum
   (`public`/`internal`/`private`) with ADR-0022 §5's binary
   (`open`/`restricted`) in one sentence, which invites the four-valued
   reading. It should not be read that way, for a reason stronger than "the
   ADRs use different words": the two questions have **different consequences
   for different parties**. Publication decides whether a *record* exists in a
   *registry* for anyone to find; topology visibility decides whether a
   *supervisor* answers a *fetch*. A single ladder would have to order them,
   and they do not order — F13's table has three useful pairs and one
   mistake, not four points on a line.

   **One correction to an earlier draft of this plan**, worth stating because
   it is the kind of error the F13 table exists to prevent: that draft
   defended `(private, open)` as meaningful — "a member reached by logical
   name through the supervisor rather than by a registry lookup". That is
   **wrong**. ADR-0022 §4 keeps Tier 3 exactly as it is, so a caller who
   fetches a topology document still resolves each member DID through the
   registry. `(private, open)` yields a signed member list nobody can dial.
   It is the one pairing `D-B2-14`(b) refuses, and test 34a pins the
   behaviour. **Owed**: exit criterion 10's wording split into its two
   declarations in task.md (§8).

4. **ADR-0018 §1's code snippet is stale against the shipped enum.** It writes
   `#[serde(rename_all = "snake_case")]` and a doc comment per variant; the
   shipped enum ([models.rs:534](../../../../crates/app_orchestration/src/models.rs#L534))
   is `lowercase` with one shared doc comment. Identical wire output for these
   three names. Corrected in the ADR by §8, not in code.

5. **`roymctl registry register` publishes an endpoint record with no deploy
   and no declaration** (F11). Neither ADR mentions it. It is unaffected and
   should stay so — but ADR-0018 reads as though publication has exactly one
   door, and a reader auditing "can a record be published without a
   declaration" will find this and be right. One sentence in the ADR.

6. **ADR-0018 §4's table defines `internal` against one publication channel
   and there are two.** The table's middle row reads "registered here, not
   propagated upward", which is exactly what `is_private` does at the
   parent-registry relay — and nothing at all at the Mainline DHT, where
   `RegistryClient::register` publishes unconditionally (F15). So `internal`
   has meant "globally resolvable" on any node with
   `enable_bep0044_dht = true` since the flag existed. `D-B2-16` closes it in
   code; the ADR still needs the sentence, because the table as written is
   what a future reader will trust. **This is a fifth correction owed to
   ADR-0018**, alongside the four §8 already carries.

   Worth stating plainly: the fix is not a caveat on `internal`. A caveat
   would leave the tier meaning two different things depending on a node's
   transport configuration, which is precisely the kind of incidental
   behaviour this ADR exists to replace with a declaration.

7. **ADR-0022 §5 says the declaration is "part of the desired state submitted
   through `submit`" and says nothing about where it lives in the manifest.**
   This plan puts it on `ServiceSpec` (per logical service, cloned to each
   member, `D-B2-2`), which is the only place that satisfies both "per logical
   service" and "survives a handover". Recording the choice because the ADR
   does not make it.

8. **ADR-0022 §5's anti-enumeration cost is unstated.** Declaring a service
   `open` makes its existence discoverable to anyone who guesses the app DID
   and service name, weakening the "an unknown app and an unauthorized caller
   are refused identically" property that `handle_resolve`'s doc comment calls
   out by name. That is inherent in "open to all" and is the owner's
   declaration to make — but it is a property the S2 slice deliberately built
   and this slice deliberately narrows, so it belongs in the ADR's
   consequences, not only in a code comment.

9. **The deferred-backlog row *"An `-i`-less gateway host's meaning depends on
   the app"*** (line 275) says its fix is "a manifest-declared default
   interface, stable across adding a second one — a surface S4 is already
   opening for per-service visibility". B2 opens that surface
   (`ServiceSpec.topology_visibility`) but **does not** add a default-interface
   field. The row's target should be re-pointed off S4 (which no longer owns
   the visibility half) rather than left pointing at a slice that will close
   without addressing it.

---

## §10 What this plan does not decide, and the backlog rows it owes

New rows for [deferred-backlog.md](../../deferred-backlog.md):

| Row | Why it is deferred | Target |
|---|---|---|
| **A privately-shared endpoint record cannot be imported by a peer *substrate*, only by a client — so `private` means "not reachable across nodes", not merely "not listed"** | `D-B2-8`/§9.2: ADR-0018 §3's `known_records` store, verified on import and on load and threaded into `net_iroh::resolve_iroh_addr`, is not built. A `private` service is reachable by a client holding its record (§2, shipped) but **not** by a peer substrate's `ProxyRouter::invoke_remote`. Same-node siblings are unaffected — `ProxyRouter` tries the local `EndpointRegistry` first, and a `DeploymentPlan` deploys to one substrate. This is a deliberate, documented narrowing of an **Accepted** ADR, and it is why `D-B2-14`(a) refuses a cross-substrate `private` dependency at compile rather than letting it fail at runtime. | TBD (M06C if a cross-node private target appears) |
| **A visibility change is only honoured at deploy time; there is no verb to change it in place** | Flipping a service from `public` to `private` means a redeploy, which reinstalls the service. `renew_cert` is the precedent for a narrow in-place update verb, and no equivalent exists for the record. Not needed for any B2 criterion. | TBD |
| **`topology_visibility` has no per-caller middle tier** | ADR-0022 §5 is binary by decision, and `D-B2-1` implements exactly that. A service that wants "these three DIDs, and nobody else" still needs a `supervisor/resolve` UCAN per caller — which works, but has no manifest surface and no issuing verb. | TBD |
| **DHT publication has no test seam, so `D-B2-16`'s gate is proven only through its side effects** | `RegistryClient.dht_client` is a concrete `Option<pkarr::Client>` built inside `RegistryClient::new` ([dht_registry.rs:266](../../../../crates/core/src/dht_registry.rs#L266)); the existing DHT test already records that this is why it can only assert `is_ok()`. Test 32 works around it via `D-B2-17`'s `Err` rather than extracting a trait — right for one condition, wrong the moment a second behaviour depends on *what* was published. A `DhtPublisher` trait is the fix when that day comes. | TBD |
| **A published record's `not_after` still has no renewal verb** | Pre-existing (`EndpointPublisher::warn_on_near_expiry_records`'s own doc comment says so), and B2 makes it more visible by making publication deliberate: an operator who declared `public` now has a 30-day clock and a `warn!` as their only tooling. Not created here. | TBD |

Explicitly **not** decided here: whether the Tier-1 app record gains an
app-level visibility declaration (F12 — its existing row stays open);
relaxing the registry's one keying rule so a substrate may publish for a
service it hosts (ADR-0018 names it a non-goal, and M04A B7's F9 found the
motivation unsound); revocation of a shared private record beyond its TTL;
populating `EndpointInfo.delegation` so an owner's master key vouches for a
per-service key (it already verifies; no flow populates it); and anything
about `syneroym:conversation`, the outbox, or the dual-build shim (B3/B4/B5).
