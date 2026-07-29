# Slice A2 Implementation Plan — Host-Side Dependency Resolution

**Status:** 📋 Planned (2026-07-29). Not started. Design of record:
[ADR-0021](../../../decisions/0021-binding-propagation-and-app-supervisor.md)
§2 (host resolves, guest names), plus
[ADR-0020](../../../decisions/0020-stable-logical-service-identity.md) §1 for
the identity half. Milestone: [task.md](task.md) slice A2. Depends on A0 and
A1, both complete.

**Read §0 first.** Planning found seven places where `task.md` / ADR-0021
describe a tree that does not exist, or leave a choice the plan has to make.
Three of them change what A2 has to build. They are listed with a recommended
resolution each; §1's decisions (D-A2-1 … D-A2-14) then take those
resolutions as given.

**Review round 2 (2026-07-29), all findings incorporated.** Six follow-ups,
four of them introduced by round 1's own revisions: §3.5 still said
`app.rs:148` was unchanged after D-A2-16 gave it work to do; the new
`emit_bindings` parameter arrived with no sweep of its own (**10** sites);
moving `LogicalResolver` into `AppSandboxEngine::init`'s signature pushed a
manifest dependency out to **three more crates** (§2.3); and §4.3's snippet
broke D-A2-15 one line after stating it. Two bookkeeping fixes: the
`AppSandboxEngine::init` total is 55, not 57 (§2.4), and only **three** WIT
`PlannedService` literals change, not four — `mapper.rs:258` is the app-model
struct, already counted in §3.2 (§3.1).

**Review round 1 (2026-07-29), all findings incorporated.** Three plan claims
were wrong about the tree and are corrected in place: the three `proxy.wit`
"copies" are one file plus two committed symlinks (§4.1); `roymctl svc deploy`
goes through `crates/sdk/src/lib.rs`'s three `deploy_*` methods rather than
building a manifest itself (§3.5); and `SynSvcNativeService::new` has 24 call
sites, not 10 (§5.2). Six call-site sweeps were undercounted and are now
enumerated with real counts. Five design gaps are closed: wire strings went
through panicking constructors (§3.4/§2.5), the non-`--mint-masters` deploy
path wrote unusable bindings (§3.3), Phase 3's migration break was
undocumented (§5.4), the cached-certificate invariant was unstated (§5.2), and
the no-network-hop budget had no test (§7). One decision is **reversed**:
D-A2-6 carried `app-context` on `deploy-manifest`, which both broke 71 literal
sites and rested on a claim about A5 that today's `deploy` cannot deliver — it
now rides on `planned-service` (4 sites), and A5's push path is named
explicitly.

---

## §0 — What the design of record gets wrong or leaves open

Same discipline as A0's §6 and A1's §6: recorded here rather than silently
worked around, so ADR-0020/ADR-0021 can carry a dated amendment at sign-off.

### 0.1 (Scope-changing) `StaticInventory` has *no* callers at all — not "no real ones"

`task.md` says "`StaticInventory` gains its first real `.register()` callers,"
which reads as though the wiring exists and only the data is missing. It does
not. `AppRegistry`, `LogicalResolver`, `StaticInventory`, `TopologyEntry`, and
`TopologyEpoch` are exported by
[app_orchestration/src/lib.rs:22](../../../../crates/app_orchestration/src/lib.rs#L22)
and referenced **nowhere else in the workspace** — verified by
`grep -rn "StaticInventory\|LogicalResolver\|AppRegistry" --include="*.rs" crates apps`,
whose only hit outside `resolver.rs` is that re-export line. Nothing
constructs a `LogicalResolver`, nothing owns one, nothing persists a
`TopologyEntry`, and no substrate-side type knows which app instance a
deployed service belongs to.

So A2 is not "add a resolution call at one site." It has to build:
ownership of the registry, delivery of bindings to the substrate, persistence
across restart, and the plumbing that lets a per-invocation `HostState` reach
the resolver. Phases 0–1 below are entirely that, and they are the bulk of the
slice.

### 0.2 (Scope-changing) A cross-app `Bind` dependency cannot be named by a service today

ADR-0021 §2 says "A cross-app `Bind` dependency (§7) is likewise named by its
**local declared name**, with the host holding the foreign `app_instance_id`
from the deploy that established the bind." No manifest can express that:

- `ServiceSpec.depends_on: Vec<LogicalServiceName>`
  ([models.rs:269](../../../../crates/app_orchestration/src/models.rs#L269))
  and `SynAppManifest::validate` rejects any entry that is not a service in
  the *same* manifest ([models.rs:317-329](../../../../crates/app_orchestration/src/models.rs#L317)).
- `AppDependencySpec::Bind { instance }`
  ([models.rs:277](../../../../crates/app_orchestration/src/models.rs#L277))
  names an app instance and **no service inside it**, and the compiler's
  `Bind` arm does nothing but a cycle check
  ([compiler.rs:91-102](../../../../crates/app_orchestration/src/compiler.rs#L91)).

**Recommended resolution (taken as D-A2-2):** A2 resolves **intra-app**
declared dependencies only. Cross-app binding needs a manifest surface first
(a way for a service to say "I depend on dependency `db`'s service
`postgres`"), which is a separate, additive change; it gets a backlog row with
ADR-0021 §7 as the pickup trigger. The WIT record A2 introduces is already
shaped to carry a foreign `app_instance_id` when that surface exists, so
nothing has to be redesigned.

### 0.3 (Scope-changing) `{member_master_did, expected_asserter_did}` are the same string

`task.md` A2 says binding entries carry `{member_master_did,
expected_asserter_did}` **per member**. But A2's own next paragraph declares
`expected_asserter_did` to *be* the member master DID. Under ADR-0020 §2 a
`ServiceId` in the app model already **is** a member master DID, so the pair
is one value written twice, and a binding record with both fields invites them
to drift.

**Recommended resolution (taken as D-A2-3):** the binding carries one member
DID per member (`TopologyEntry.members: Vec<ServiceId>`, unchanged). The
"publication" D-B3-8 was owed is satisfied by that DID being the asserter DID
— a dependent that holds the binding holds the value. What A2 must actually
change is the *credential* side, so that a proof signed by an instance key
still verifies against that master DID (Phase 3).

**Not resolved here, flagged for confirmation:** whether A2 should also make
`Relation.service` in an FDAE policy a *declared dependency name* resolved
through the same registry (which would let `Relation.expected_asserter_did`
become optional, derived from the binding). ADR-0021 §2 is about the **guest
proxy target**, not the policy document, and `syneroym_fdae::plan_read` would
have to take a resolver — an API change reaching through `data_db`'s whole
auth path and both read ingresses. **Recommendation: out of scope for A2**,
backlog row with A5 as the target; A2 delivers the credential change that
makes a policy-declared master DID keep working across reinstantiation, which
is the load-bearing half. If the milestone owner reads `task.md`'s
"publication gap" sentence as requiring the policy change too, it is a
separately-sequenced phase, not a tweak to Phase 3.

### 0.4 ADR-0021 still describes A1's superseded design

ADR-0021's Context, §2 preamble, and Consequences all say endpoint records are
"**delegation-signed**" and cite ADR-0020 §6 for it. A1's fifth pass
(2026-07-29) reversed that: the record is signed by the member master
directly, carries no certificate, and has an ordinary DHT home
([status.md](status.md)'s A1 "Fifth pass" section). ADR-0021's *argument* is
unaffected — it needs only that a master DID resolves to an address, which is
now more true, not less. Doc-only; fold into the sign-off amendment.

### 0.5 `TopologyMode::Sharded` has no expressible strategy

`ShardingStrategy` (`HashSharding` / `EntityTagSharding` / `RangeSharding`)
exists on `TopologyEntry`, but no manifest field sets it and the compiler
always writes `TopologyMode::default()` (= `Singleton`)
([compiler.rs:143](../../../../crates/app_orchestration/src/compiler.rs#L143)).
`select_member` already treats `Sharded` with `sharding_strategy: None` as
hash sharding
([resolver.rs:580-601](../../../../crates/app_orchestration/src/resolver.rs#L580)).

**Resolution (D-A2-4):** the wire binding carries `mode` and no strategy;
`Sharded` means hash sharding until a manifest can say otherwise. Backlog row.

### 0.6 A guest has no way to supply a routing key

`LogicalResolver::resolve(&logical_ref, routing_key: Option<&[u8]>)` needs a
key for `Sharded` (hard error without one) and for keyed `Redundant`
selection. `syneroym:proxy/proxy::call` has no such argument.

**Resolution (D-A2-5):** `call-options` gains `routing-key: option<string>`.
Additive to an existing optional record, so no existing guest changes.

### 0.7 The `(None, CallOrigin::Native)` gap is narrower than the backlog row implies

[deferred-backlog.md](../../deferred-backlog.md) row *"A service's signing
identity is still its instance key…"* item (a) says the `CallOrigin::Native`
arm "keeps presenting the *node's* key on the wire." Only the **`caller.proof
== None`** sub-case does
([proxy.rs:402-404](../../../../crates/router/src/proxy.rs#L402)). The
`(Some(proof), Native)` arm forwards the *original caller's* proof verbatim,
and that is load-bearing: `resolve_fetches`' doc comment (D-B3-9,
[fdae_fetch.rs:66-73](../../../../crates/rpc/src/fdae_fetch.rs#L66)) depends
on the remote re-deriving `subject_did`/`anchor_did` from the real chain, and
`resolve_relation`'s A1 path authorizes on exactly that. Replacing it with the
calling service's identity would break FDAE B3.

**Resolution (D-A2-13):** Phase 4 changes the `(None, Native)` case only.

---

## §1 — Decisions

| # | Decision |
|---|---|
| **D-A2-1** | The guest names a dependency through a **new `call-target` variant** on `syneroym:proxy/proxy::call`, not a new function and not a magic string prefix. `service(string)` keeps today's raw-DID/alias behavior for self-proxy, native dispatch, and external callers (ADR-0021 §2's "second variant"); `dependency(string)` is resolved host-side. A variant makes the two mutually exclusive at the type level, which a `option<string>` pair would not. |
| **D-A2-2** | Intra-app dependencies only (§0.2). A declared dependency name **is** the target's `LogicalServiceName` inside the caller's own `app_instance_id`, so `LogicalServiceRef { app_instance_id: <host's own>, service_name: <declared name> }` is the whole resolution input. Cross-app: backlog. |
| **D-A2-3** | A binding entry carries member **DIDs** only; `expected_asserter_did` is that same DID, not a second field (§0.3). |
| **D-A2-4** | Bindings carry `mode` and no sharding strategy; `sharded` = hash sharding (§0.5). |
| **D-A2-5** | `call-options.routing-key: option<string>` (§0.6). Bytes are taken as the string's UTF-8 encoding. |
| **D-A2-6** | Bindings arrive on **`planned-service`** — the `deploy-plan` payload — not on `deploy-manifest` and not as a separate RPC. Two reasons, one of which reverses this plan's first draft. (1) *Correctness:* the first draft justified `deploy-manifest` by saying A5's push would then be "a `deploy`-shaped apply rather than a new verb." Today's `deploy` is a **full reinstall** — it redoes artifact work, re-registers every endpoint, rebuilds `SynSvcNativeService` ([orchestration.rs:803](../../../../crates/control_plane/src/service/orchestration.rs#L803)), and calls `undeploy` to roll back on failure — and there is no content-hash idempotency anywhere in the tree (failure-matrix row 10 is unbuilt). So that push would restart every dependent, which is exactly what ADR-0021 §2 exists to prevent and what the reference scenario's step 5 forbids ("**with no restart**"). ADR-0021 §3 also keeps the two dedup rules deliberately separate. **A5 needs a binding-only write path**, and A2's field is the *initial-deploy carrier* only — see §8.1. (2) *Cost:* `deploy-manifest` has **71** literal construction sites across nine crates; `planned-service` has **4**. A WIT record has no field defaults, so every one of those is a compile error for a field that 67 of them would set to `None`. |
| **D-A2-15** | Every string that arrives from the wire or from a database row is parsed with `try_new`, never `new`. `define_string_wrapper`'s `new` **panics** ([models.rs:42-45](../../../../crates/app_orchestration/src/models.rs#L42)) and `LogicalServiceName` rejects empty strings and any `/` — so `new` on a caller-supplied `dependency_name` panics the control-plane task, and `new` on a stored row panics substrate startup. A bad deploy value fails that deploy; a bad stored row is warn-skipped like an unparseable `TopologyEntry` beside it. |
| **D-A2-16** | `app deploy` **without** `--mint-masters` emits an app context with **no bindings**, plus a warning naming the consequence. `--mint-masters` is optional ([app.rs:36](../../../../apps/roymctl/src/commands/app.rs#L36)); without it `resolved_dependencies` still holds the compiler's fabricated `did:key:h…` ids, which are not real ed25519 keys ([compiler.rs:172-180](../../../../crates/app_orchestration/src/compiler.rs#L172)). Publishing those would make `dependency(...)` resolve *successfully* and then fail one layer down as `service-not-found` — destroying exactly the distinction D-A2-14 exists to draw. An absent binding gives the guest `dependency-not-bound`, which is the true answer. |
| **D-A2-7** | The substrate persists bindings and app context in `EndpointStorage` as **opaque JSON**, and the composition root replays them into `StaticInventory` at startup. `syneroym-core` does not gain a dependency on `syneroym-app-orchestration` (it would be acyclic — `app_orchestration` has no `syneroym-*` deps — but core is the wrong place for app-model types, and JSON is exactly how instance certificates are already stored). |
| **D-A2-8** | `StaticInventory` stays the only `AppRegistry` implementation and stays purely in-memory (ADR-0021 §1). Durability is the substrate's, layered above it — not a new trait method and not a second implementation. |
| **D-A2-9** | Undeploy removes the *service's* persisted app-context and binding rows, and **leaves the in-memory `StaticInventory` entry alone**. A `TopologyEntry` is an app-scoped fact ("where does `backend` live in instance X"), not a per-dependent one; removing it when one of several dependents goes away would break the others. Cost: entries for fully-removed app instances linger in memory until restart. Bounded by (app instances × dependency names) and owned by A5's lifecycle. Backlog row. |
| **D-A2-10** | A2's binding write is **last-write-wins**, with no epoch guard. ADR-0021 §3's four-case rule (lower rejects / equal+identical no-op / equal+different conflict / higher applies) is A5's, at this exact write point — the code carries a comment naming it so the guard lands in one place rather than being re-derived. Failure-matrix rows 5–7 are A5's evidence, unchanged. |
| **D-A2-11** | `RelationshipProof` gains an optional `delegation` field carrying the JSON `DelegationCertificate` that binds the signing instance key to `asserter_did`. `asserter_did` becomes the **member master** whenever a certificate is present; without one, the pre-existing "the signer is its own asserter" shape is unchanged, so every pre-A0 service keeps working. The field is inside the signed payload. |
| **D-A2-12** | `RelationshipProof::verify` checks the certificate with `DelegationCertificate::verify` (**live-credential** strictness, wall-clock expiry included), not A1's `verify_chain`. A1 needed the looser check because a stored endpoint record is read long after it was signed; a relationship proof is minted per fetch and lives 60 s, so a certificate that has already lapsed means the responder should have renewed. Accepted scope is `[SCOPE_SERVICE_INSTANCE]` exactly — the narrow single-value check failure-matrix row 2 describes, at a third site. |
| **D-A2-13** | `CallOrigin::Native` gains `service_id: Option<String>`; only the `caller.proof == None` case changes behavior (§0.7). |
| **D-A2-14** | Resolution failure is a **distinct** guest-visible error (`proxy-error::dependency-not-bound`), not `service-not-found`. A guest must be able to tell "my binding has not arrived / I never declared this" from "the DID does not resolve to an address," because under push the first is the expected transient state (ADR-0021 §5) and the second is not. Constructed in the host function before a `ProxyRequest` exists, so `syneroym_rpc::ProxyError` is untouched. |

---

## §2 — Phase 0: registry ownership, persistence, and startup replay

No behavior change on its own; everything after it depends on it.

### 2.1 `EndpointStorage` — five new methods

`crates/core/src/storage.rs`, on `pub trait EndpointStorage`:

```rust
/// Load every recorded app context as (service_id, app_instance_id, service_name).
async fn load_all_app_contexts(&self) -> Result<Vec<(String, String, String)>>;
/// Record which app instance and logical name `service_id` was deployed as (upsert).
async fn save_app_context(
    &self,
    service_id: &str,
    app_instance_id: &str,
    service_name: &str,
) -> Result<()>;
/// Forget `service_id`'s app context and every binding row it wrote. Idempotent.
async fn remove_app_context(&self, service_id: &str) -> Result<()>;

/// Load every recorded dependency binding as
/// (service_id, app_instance_id, dependency_name, topology_entry_json).
async fn load_all_bindings(&self) -> Result<Vec<(String, String, String, String)>>;
/// Record one dependency binding for `service_id` (upsert on
/// (service_id, dependency_name)).
async fn save_binding(
    &self,
    service_id: &str,
    app_instance_id: &str,
    dependency_name: &str,
    topology_entry_json: &str,
) -> Result<()>;
```

There is no `remove_binding`: `remove_app_context` covers binding removal too,
since a redeploy overwrites the rows it still declares and `deploy` clears the
service's rows first (§3.4).

**Implementations to update (4):**

| File | What |
|---|---|
| `crates/core/src/storage.rs` (`MockStorage`) | two more `DashMap`s: `app_contexts: DashMap<String, (String, String)>`, `bindings: DashMap<(String, String), (String, String)>` keyed `(service_id, dependency_name)` |
| `crates/data_db/src/registry_store.rs` (`SqliteEndpointStorage`) | two tables + five methods, pattern-identical to `save_cert`/`load_all_certs`/`remove_cert` ([registry_store.rs:245-300](../../../../crates/data_db/src/registry_store.rs#L245)) |
| `crates/control_plane/src/service/orchestration.rs` (test impl in `mod tests`) | stub methods |
| `crates/router/tests/service_ownership.rs` (test impl) | stub methods |

SQL for `registry_store.rs`, appended to the same unconditional
`execute_batch` that creates `service_instance_certs`
([registry_store.rs:90](../../../../crates/data_db/src/registry_store.rs#L90))
— A0's D-A0-10 removed the `PRAGMA user_version` gate precisely so an
in-place addition like this is correct on an existing database:

```sql
CREATE TABLE IF NOT EXISTS service_app_context (
    service_id      TEXT PRIMARY KEY,
    app_instance_id TEXT NOT NULL,
    service_name    TEXT NOT NULL,
    created_at      INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS service_bindings (
    service_id      TEXT NOT NULL,
    app_instance_id TEXT NOT NULL,
    dependency_name TEXT NOT NULL,
    entry_json      TEXT NOT NULL,
    created_at      INTEGER NOT NULL,
    PRIMARY KEY (service_id, dependency_name)
);
```

The in-test schema at `registry_store.rs:511+` (bare `CREATE TABLE`) needs the
same two tables.

### 2.2 `EndpointRegistry` — app context, in memory and through

`crates/core/src/local_registry.rs`. New field alongside `service_certs`:

```rust
/// `service_id` -> (`app_instance_id`, `service_name`) for a service
/// deployed as part of an app instance. Absent for a standalone
/// `svc deploy`, which resolves no declared dependencies.
service_app_contexts: Arc<DashMap<String, (String, String)>>,
```

Methods (mirroring `set_owner`/`owner_of`/`remove_owner` exactly):

```rust
pub async fn set_app_context(
    &self, service_id: String, app_instance_id: String, service_name: String,
) -> Result<()>;
#[must_use]
pub fn app_context_of(&self, service_id: &str) -> Option<(String, String)>;
pub async fn remove_app_context(&self, service_id: &str) -> Result<()>;

/// Persist one dependency binding. The in-memory `AppRegistry` is written
/// separately by the caller -- this store only makes the write survive a
/// restart.
pub async fn save_binding(
    &self, service_id: &str, app_instance_id: &str,
    dependency_name: &str, entry_json: &str,
) -> Result<()>;
/// Every persisted binding, for the composition root's startup replay.
pub async fn all_bindings(&self) -> Result<Vec<(String, String, String, String)>>;
```

`load_from_db` gains a loop populating `service_app_contexts` from
`storage.load_all_app_contexts()`. `new_mock` initializes the new field.
Bindings are deliberately **not** mirrored into a `DashMap` here — their
in-memory home is `StaticInventory`.

### 2.3 Four `Cargo.toml` files gain the app-orchestration dependency

Putting `Arc<LogicalResolver>` in `AppSandboxEngine::init`'s signature pushes
the dependency out to every crate that *constructs* an engine, not just the
one that stores it. Acyclic in all four cases —
`crates/app_orchestration/Cargo.toml` has no `syneroym-*` dependencies at all.

| Crate | Section | Why |
|---|---|---|
| `crates/sandbox_wasm` | `[dependencies]` | `HostState` / `AppSandboxEngine` hold the resolver |
| `crates/substrate` | `[dependencies]` | `runtime.rs` names `StaticInventory`, `LogicalResolver`, `TopologyEntry`, `AppInstanceId`, `LogicalServiceName` (§2.5) |
| `crates/router` | `[dev-dependencies]` | 8 tests plus `benches/proxy.rs` construct an engine |
| `crates/coordinator_iroh` | `[dev-dependencies]` | `tests/multi_hop_relay.rs:84` |

`crates/control_plane` and `crates/smoke-tests` already depend on it.

The `#[doc(hidden)] empty_resolver()` helper §2.4 suggests **does not avoid
this** — a crate still has to depend on `app_orchestration` to call it. The
helper saves repetition, not manifest edits.

### 2.4 `AppSandboxEngine` holds the resolver

`crates/sandbox_wasm/src/engine.rs`:

- new field `logical_resolver: Arc<LogicalResolver>`;
- `AppSandboxEngine::init` gains a trailing parameter
  `logical_resolver: Arc<LogicalResolver>` (after `endpoint_registry`);
- in `create_store_and_instance`
  ([engine.rs:900-915](../../../../crates/sandbox_wasm/src/engine.rs#L900)),
  read the app context and pass both into `HostState::new`:

```rust
let app_context = self.endpoint_registry.app_context_of(service_id);
let host_state = HostState::new(
    /* ...unchanged args... */,
    app_context.map(|(instance, _name)| instance),   // app_instance_id
    self.logical_resolver.clone(),
);
```

**`AppSandboxEngine::init` call sites: 55.** Two are not tests or benches and
must be reviewed individually — `crates/substrate/src/runtime.rs:510` (the
real composition root) and **`crates/smoke-tests/src/main.rs:301`** (a
standalone binary, easy to miss because it is neither). The other 53 are
tests/benches: 24 in `control_plane/src/service/orchestration.rs`, 11 in
`control_plane/src/service.rs`, 8 across `router/tests`, 8 across
`sandbox_wasm/tests` and `benches`, plus `coordinator_iroh/tests/
multi_hop_relay.rs:84` and `router/benches/proxy.rs:119`. All take
`Arc::new(LogicalResolver::new(Arc::new(StaticInventory::new())))`. Worth a
test-only helper (`fn empty_resolver() -> Arc<LogicalResolver>`) exported from
`app_orchestration` behind `#[doc(hidden)]` rather than 55 copies of that
expression. `crates/control_plane/src/dummy_sandbox.rs`'s stand-in engine is
unaffected (it has its own `init`).

### 2.5 Composition root

`crates/substrate/src/runtime.rs`, `build_route_handler_deps`:

```rust
let app_registry = Arc::new(StaticInventory::new());
// Replay persisted bindings before anything can resolve one: a restarted
// substrate must answer a guest's first call, and nothing re-pushes on
// restart (ADR-0021 §5 -- push failure is sticky, and so is push absence).
for (_service_id, instance, dep_name, entry_json) in registry.all_bindings().await? {
    // Every one of these three is a stored string, so all three are
    // fallible (D-A2-15) -- `LogicalServiceName::new` would *panic*
    // substrate startup on a row containing a '/', which is a strictly
    // worse outcome than the warn-and-skip the JSON parse beside it
    // already chose for the same class of corruption.
    let parsed = (|| -> anyhow::Result<_> {
        Ok((
            AppInstanceId::try_new(&instance)?,
            LogicalServiceName::try_new(&dep_name)?,
            serde_json::from_str::<TopologyEntry>(&entry_json)?,
        ))
    })();
    match parsed {
        Ok((instance_id, service_name, entry)) => {
            app_registry.register(instance_id, service_name, entry)
        }
        Err(e) => warn!(%instance, %dep_name, error = %e,
            "skipping an unreadable persisted binding"),
    }
}
let logical_resolver = Arc::new(LogicalResolver::new(app_registry.clone()));
```

`logical_resolver` is then passed to **both** `AppSandboxEngine::init` (the
read side) and `ControlPlaneService::init` (the write side). One
`StaticInventory`, one `LogicalResolver` over it, two holders — the
`Arc<StaticInventory>` itself never leaves `runtime.rs`, because every write
must also evict the resolver's cache (§3.4).

### 2.6 `ControlPlaneService` holds the write side

`crates/control_plane/src/service.rs`:

- new field `logical_resolver: Arc<LogicalResolver>`;
- `init` gains a trailing `logical_resolver: Arc<LogicalResolver>` parameter.

**`ControlPlaneService::init` call sites: 37.** One production
(`runtime.rs:548`); the rest are tests, and two groups sit outside where an
implementer would look: **9 in `crates/control_plane/src/service.rs`'s own
test module** (not `orchestration.rs`, which has the deploy tests) and
`coordinator_iroh/tests/multi_hop_relay.rs:134`. The remainder are in
`orchestration.rs` and `router/tests`.

---

## §3 — Phase 1: bindings on the wire, from manifest to substrate

### 3.1 WIT — `crates/wit_interfaces/wit/control-plane/control-plane.wit`

Inside `interface orchestrator`, before `deploy-manifest`:

```wit
    /// How a logical service name maps to physical members. Mirrors
    /// `TopologyMode` in the app model.
    variant topology-mode {
        singleton,
        redundant,
        sharded,
    }

    /// One declared dependency of the service being deployed, resolved to
    /// the member master DIDs that currently serve it. The dependent's
    /// host resolves a guest-supplied dependency name through this;
    /// nothing else in the substrate reads it.
    record dependency-binding {
        /// The name the depending service declared. Today always a
        /// logical service name inside `app-instance-id`.
        dependency-name: string,
        /// The app instance the members live in. Equal to the dependent's
        /// own `app-instance-id` today; a distinct value once a manifest
        /// can express a cross-app bind.
        app-instance-id: string,
        mode: topology-mode,
        /// Member master DIDs, in selection order.
        members: list<string>,
        /// Increments on any membership or mode change; the epoch guard
        /// that reads it is the supervisor's.
        epoch: u64,
        cache-ttl-ms: u64,
    }

    /// Which app instance this service is being deployed as part of, and
    /// what its declared dependencies currently resolve to. Absent for a
    /// standalone deploy that participates in no app.
    record app-context {
        app-instance-id: string,
        service-name: string,
        bindings: list<dependency-binding>,
    }
```

and one field on **`planned-service`** (D-A2-6 — *not* on `deploy-manifest`):

```wit
    record planned-service {
        service-id: string,
        logical-ref: string,
        manifest: deploy-manifest,
        /// This service's app instance and dependency bindings (ADR-0021
        /// §2). Absent leaves the service unable to resolve a declared
        /// dependency name -- it can still be called, and can still call
        /// out by DID.
        app-context: option<app-context>,
    }
```

`deploy-manifest` is untouched, so the 71 `DeployManifest { … }` literals
across `sdk`, `control_plane`, `sandbox_wasm`, `router`, `coordinator_iroh`,
`substrate/tests`, and two benches keep compiling unchanged. Exactly **three**
WIT `PlannedService { … }` literals change:
[mapper.rs:205](../../../../crates/sdk/src/mapper.rs#L205) (production),
[orchestration.rs:1296](../../../../crates/control_plane/src/service/orchestration.rs#L1296),
and `:1375` (tests).

`grep` returns a fourth,
[mapper.rs:258](../../../../crates/sdk/src/mapper.rs#L258), which is **not**
one of these: it is the *app-model* `PlannedService` (it carries `config`,
`resolved_dependencies`, `topology_mode` — fields the WIT record does not
have), and it is already listed in §3.2's table for the
`resolved_dependencies` shape change. Counting it here would be
double-counting the same edit. Three against 71 is the real cost ratio behind
D-A2-6.

**Threading it through `deploy`.** The WIT `deploy` function keeps its
two-argument `(service-id, manifest)` signature — a standalone deploy carries
no app context (D-A2-2). `deploy_plan`
([orchestration.rs:1177](../../../../crates/control_plane/src/service/orchestration.rs#L1177))
currently ends in `self.deploy(service_id, deploy_manifest, caller).await?`.
Split the body:

```rust
// The trait method: no app context, so no bindings. Unchanged for every
// existing caller, including the JSON-RPC `deploy` dispatch.
async fn deploy(&self, service_id: String, manifest: DeployManifest,
                caller: &CallerContext) -> Result<(), String> {
    self.deploy_with_context(service_id, manifest, None, caller).await
}

// Private inherent method holding today's entire deploy body plus §3.4's
// binding block. `deploy_plan` calls this one, passing
// `service.app_context`.
async fn deploy_with_context(
    &self, service_id: String, manifest: DeployManifest,
    app_context: Option<AppContext>, caller: &CallerContext,
) -> Result<(), String> { /* ... */ }
```

Note `deploy`'s internal self-call on the native-capability rollback path
([orchestration.rs:787](../../../../crates/control_plane/src/service/orchestration.rs#L787))
calls `self.undeploy`, not `self.deploy`, so no recursion concern.

### 3.2 App model — `crates/app_orchestration/src/models.rs`

`PlannedService.resolved_dependencies: Vec<ServiceId>` loses the dependency
*names*, which the binding needs. Change it in place (pre-release, no
compatibility shim):

```rust
    /// Declared dependency name -> the member master DIDs currently serving
    /// it. One entry per `ServiceSpec.depends_on` name; the member list is
    /// a single-element `Singleton` today.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub resolved_dependencies: BTreeMap<LogicalServiceName, Vec<ServiceId>>,
```

**Every site touching the field:**

| File | Change |
|---|---|
| [compiler.rs:126-136](../../../../crates/app_orchestration/src/compiler.rs#L126) | build the map: `spec.depends_on.iter().map(\|dep\| (dep.clone(), vec![derive_deterministic_service_id(&dep_ref)])).collect()` |
| [compiler.rs:266](../../../../crates/app_orchestration/src/compiler.rs#L266) (test) | assert against the map |
| [journal.rs:332](../../../../crates/app_orchestration/src/journal.rs#L332), [reconcile.rs:161](../../../../crates/app_orchestration/src/reconcile.rs#L161), [models.rs:614/728](../../../../crates/app_orchestration/src/models.rs#L614), [sdk/src/mapper.rs:253+](../../../../crates/sdk/src/mapper.rs#L253) (tests) | `vec![]` → `BTreeMap::new()` |
| [member_identity.rs:196-204](../../../../apps/roymctl/src/commands/member_identity.rs#L196) | substitute the map's **values**: `svc.resolved_dependencies = svc.resolved_dependencies.iter().map(\|(name, members)\| Ok((name.clone(), members.iter().map(\|d\| substitution.get(d).cloned().ok_or_else(...)).collect::<Result<Vec<_>>>()?))).collect::<Result<BTreeMap<_,_>>>()?;` |

`member_identity.rs`'s substitution is load-bearing here: without it a binding
would name the compiler's fabricated DID, which `resolve_did_key` rejects —
the A0 finding recorded in [status.md](status.md).

### 3.3 Mapper — `crates/sdk/src/mapper.rs`

`map_deployment_plan_to_wit` gains one parameter, `emit_bindings: bool`, and
builds the app context per service:

```rust
let app_context = Some(WitAppContext {
    app_instance_id: plan_instance_id.clone(),      // captured before the loop
    service_name: svc.logical_ref.service_name.to_string(),
    // D-A2-16: without member-master substitution these members are the
    // compiler's fabricated `did:key:h…` ids, which resolve to no key.
    // Publishing them would make `dependency(...)` resolve and then fail
    // a layer down as `service-not-found`; an empty list gives the guest
    // the true answer, `dependency-not-bound`.
    bindings: if emit_bindings {
        svc.resolved_dependencies
            .iter()
            .map(|(name, members)| WitDependencyBinding {
                dependency_name: name.to_string(),
                app_instance_id: plan_instance_id.clone(),   // intra-app only (D-A2-2)
                mode: map_mode(target_modes[name]),
                members: members.iter().map(|m| m.to_string()).collect(),
                epoch: 0,        // A2 mints no epochs; the supervisor does (A5)
                cache_ttl_ms: DEFAULT_BINDING_CACHE_TTL_MS,
            })
            .collect()
    } else {
        Vec::new()
    },
});
```

`app deploy` passes `emit_bindings: *mint_masters`
([app.rs:148](../../../../apps/roymctl/src/commands/app.rs#L148)) and, when it
is `false` **and** any service declares a `depends_on`, prints:

```
warning: deploying without --mint-masters, so declared dependencies are not
bound. A guest calling one by name gets `dependency-not-bound` until the app
is redeployed with --mint-masters.
```

Two details that are easy to get wrong:

- `plan.app_instance_id` is moved into `WitDeploymentPlan` at the end of the
  function ([mapper.rs:218](../../../../crates/sdk/src/mapper.rs#L218)); clone
  it into a local **before** the `for svc in plan.services` loop.
- `mode` comes from the **dependent's own** `topology_mode` today, which is
  wrong in principle (the mode belongs to the *target*) but produces the
  correct value while every service is `Singleton`. Read the target's mode
  instead by building a `BTreeMap<LogicalServiceName, TopologyMode>` over
  `plan.services` before the loop. Do it properly — it is three lines and
  removes a landmine the first `Redundant` service would step on.
  `target_modes` in the snippet above is that map, and `map_mode` is the
  `TopologyMode` → `WitTopologyMode` arm.

`DEFAULT_BINDING_CACHE_TTL_MS`: a new `pub const` in
`app_orchestration::resolver`, value `60_000` — matching what the module's own
tests already treat as ordinary (`Duration::from_secs(60)`).

**`map_deployment_plan_to_wit` call sites: 10** — a sweep this revision
introduced, so it did not exist to be counted in the first round. One
production (`apps/roymctl/src/commands/app.rs:148`) and **9** in `mapper.rs`'s
own tests: 285, 306, 325, 336, 374, 398, 415, 425, 446. Every test passes
`true`, except a new one that pins D-A2-16 by passing `false` (§7). Note
`mapper.rs:67` is the definition and `:335` is a *test function name*
containing the string, not a call — a plain `grep` returns 12 hits for 10
calls.

### 3.4 Deploy — `crates/control_plane/src/service/orchestration.rs`

In `deploy_with_context` (§3.1), after the instance-certificate verification
block
([orchestration.rs:534-572](../../../../crates/control_plane/src/service/orchestration.rs#L534))
and before the artifact work:

```rust
// ADR-0021 §2: the dependent's host resolves declared dependency names,
// so the bindings have to be on file before the service can serve a
// single call. Written before the artifact work for the same reason the
// certificate is: a malformed binding is a deploy failure, not a routing
// failure discovered later.
if let Some(ctx) = &app_context {
    // A redeploy fully declares this service's app context, so its
    // previous rows go first -- a dependency dropped from the manifest
    // must not survive as a stale row (the same "absence means removal"
    // rule `fdae_policy` already follows above).
    self.registry.remove_app_context(&service_id).await.map_err(|e| e.to_string())?;
    // Validate before storing, so §4.3's read of this row can only fail on
    // real corruption rather than on something a deploy caller sent. The
    // registry itself stores plain `String`s -- `syneroym-core` has no
    // app-model types (D-A2-7) -- so this is the only place the shape can
    // be enforced on the way in.
    AppInstanceId::try_new(&ctx.app_instance_id)
        .map_err(|e| format!("app context names an invalid app instance id: {e}"))?;
    LogicalServiceName::try_new(&ctx.service_name)
        .map_err(|e| format!("app context names an invalid service name: {e}"))?;
    self.registry
        .set_app_context(service_id.clone(), ctx.app_instance_id.clone(), ctx.service_name.clone())
        .await
        .map_err(|e| e.to_string())?;

    for binding in &ctx.bindings {
        // D-A2-15: all three of these are caller-supplied strings, so all
        // three are fallible. `LogicalServiceName::new` *panics* on an
        // empty name or one containing '/', which would let an
        // authorized-but-buggy deploy caller kill the control-plane task.
        let instance_id = AppInstanceId::try_new(&binding.app_instance_id)
            .map_err(|e| format!("binding names an invalid app instance id: {e}"))?;
        let dependency_name = LogicalServiceName::try_new(&binding.dependency_name)
            .map_err(|e| format!("binding names an invalid dependency name: {e}"))?;
        let entry = TopologyEntry {
            mode: map_topology_mode(binding.mode),
            members: binding
                .members
                .iter()
                .map(|m| ServiceId::try_new(m))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("binding '{}' names an invalid member DID: {e}",
                                     binding.dependency_name))?,
            sharding_strategy: None,          // D-A2-4
            epoch: TopologyEpoch(binding.epoch),
            cache_ttl: Duration::from_millis(binding.cache_ttl_ms),
        };
        let entry_json = serde_json::to_string(&entry).map_err(|e| e.to_string())?;
        self.registry
            .save_binding(&service_id, &binding.app_instance_id,
                          &binding.dependency_name, &entry_json)
            .await
            .map_err(|e| e.to_string())?;
        // Last-write-wins (D-A2-10). ADR-0021 §3's four-case epoch guard
        // -- lower rejects, equal+identical no-ops, equal+different is a
        // reported conflict, higher applies -- belongs at exactly this
        // call and is the supervisor slice's.
        self.logical_resolver.register(instance_id, dependency_name, entry);
    }
}
```

**Cache invalidation is not optional.** `LogicalResolver` caches a
`ResolvedTopology` for `cache_ttl` and only re-reads the registry on expiry or
an explicit `invalidate`
([resolver.rs:509-534](../../../../crates/app_orchestration/src/resolver.rs#L509))
— it does **not** compare epochs against the registry on a cache hit, despite
the module doc claiming it does (a pre-existing doc inaccuracy; fix the
comment while here). Without an explicit invalidate, a scale-out would take up
to a minute to take effect, which contradicts the milestone's 5 s convergence
budget before the supervisor is even written.

`ControlPlaneService` therefore holds the **resolver**, not the bare inventory
(§2.6) — and `LogicalResolver.registry` is private, so the write path needs
one new method on it, which is also what makes "register then evict" a single
step nobody can half-perform:

```rust
/// Register `entry` and drop any cached copy in one step -- the write
/// path's only entry point, so a binding write can never leave a stale
/// cached topology behind.
pub fn register(
    &self,
    instance_id: AppInstanceId,
    service_name: LogicalServiceName,
    entry: TopologyEntry,
) {
    let logical_ref = LogicalServiceRef {
        app_instance_id: instance_id.clone(),
        service_name: service_name.clone(),
    };
    self.registry.register(instance_id, service_name, entry);
    self.cache.evict(&logical_ref);
}
```

In `undeploy`, in the terminal teardown block beside `remove_owner` /
`remove_instance_cert`
([orchestration.rs:1056-1065](../../../../crates/control_plane/src/service/orchestration.rs#L1056)):
`self.registry.remove_app_context(&service_id).await`, warn-not-fail like
every other step there. Persisted rows only — the in-memory `StaticInventory`
entry stays (D-A2-9).

`map_topology_mode` is a private `fn` in `orchestration.rs`, three arms.

### 3.5 `roymctl`

`app deploy` needs no new **flag** — `--mint-masters` already exists and is the
signal D-A2-16 keys on. It does need a change at
[app.rs:148](../../../../apps/roymctl/src/commands/app.rs#L148): pass
`emit_bindings: *mint_masters` to the mapper, and print §3.3's warning when
that is `false` and any service in the plan declares a `depends_on`. (An
earlier draft of this section said line 148 was "unchanged"; that predates
D-A2-16 and is wrong.)

`svc deploy` needs **no change at all**, and it is worth being precise about
why, because the first draft of this plan got it wrong. `roymctl svc deploy`
does not build a `DeployManifest` itself — it goes through
`SyneroymClient::deploy_svc_wasm`
([sdk/src/lib.rs:505](../../../../crates/sdk/src/lib.rs#L505)),
`deploy_svc_tcp` ([:548](../../../../crates/sdk/src/lib.rs#L548)), and
`deploy_container` ([:586](../../../../crates/sdk/src/lib.rs#L586)), which are
the public single-service deploy surface. Since D-A2-6 puts `app-context` on
`planned-service` rather than `deploy-manifest`, none of those three change.

A single-service deploy therefore participates in no app instance and resolves
no dependency names — a guest that tries gets `dependency-not-bound`, which is
the honest answer. **If A5 ever needs to push a binding to a
single-service-deployed member**, `crates/sdk/src/lib.rs` is where that
surface belongs; §8.1 records why it must not be `deploy`.

---

## §4 — Phase 2: the guest names a dependency, the host resolves it

### 4.1 WIT — `syneroym:proxy/proxy@0.1.0`

**One file. Do not create a second copy.** An earlier draft of this plan
called these "three identical copies" and said to `diff` them after editing —
that is wrong and actively dangerous, because following it would replace a
symlink with a real file and silently fork the interface. The two dependency
paths are **committed symlinks** (mode `120000` in `git ls-files -s`):

```
crates/wit_interfaces/wit/host/deps/proxy -> ../../proxy
test-components/proxy-test/wit/deps/proxy -> ../../../../crates/wit_interfaces/wit/proxy
```

`find . -name proxy.wit` returns exactly one path. Edit only:

- `crates/wit_interfaces/wit/proxy/proxy.wit`

```wit
    /// What a call is addressed to. Raw-DID targeting stays for the callers
    /// that are not dependency resolution: a guest's self-proxy into its own
    /// native capabilities, native dispatch, and external/`roymctl` callers
    /// that legitimately address a specific DID. A declared dependency is
    /// resolved by the host against *this* component's own app instance --
    /// a guest cannot name another app's instance, and cannot snapshot the
    /// resolved DID, so a re-pushed binding takes effect on the next call.
    variant call-target {
        service(string),
        dependency(string),
    }

    variant proxy-error {
        service-not-found(string),
        /// The named dependency resolves to nothing: never declared, or its
        /// binding has not arrived yet. Distinct from `service-not-found`,
        /// which is a DID that does not resolve to an address.
        dependency-not-bound(string),
        unsupported-protocol(string),
        /* ...unchanged... */
    }

    record call-options {
        protocol: option<string>,
        idempotent: bool,
        timeout-ms: option<u32>,
        /// Selection key for a multi-member dependency: rendezvous hashing
        /// for `redundant`, required for `sharded`, ignored for
        /// `singleton` and for a raw-DID target.
        routing-key: option<string>,
    }

    call: func(
        target: call-target,
        %interface: string,
        method: string,
        params: string,
        options: option<call-options>,
    ) -> result<string, proxy-error>;
```

### 4.2 `HostState`

`crates/sandbox_wasm/src/host_capabilities.rs`, two new fields:

```rust
    /// The app instance this component was deployed as part of, from the
    /// substrate's own records -- never from the guest (ADR-0021 §2: a
    /// guest that could name an app instance could address an arbitrary
    /// one). `None` for a standalone deploy, which resolves no dependency.
    pub app_instance_id: Option<String>,
    /// Resolves a declared dependency name to a member's master DID.
    /// `Arc`, unlike `service_proxy`: `LogicalResolver` holds only an
    /// `Arc<dyn AppRegistry>` and no path back to the engine, so there is
    /// no cycle to guard against.
    pub logical_resolver: Arc<LogicalResolver>,
```

`HostState::new` gains both as trailing parameters (it is already
`#[allow(clippy::too_many_arguments)]`).

**`HostState::new` call sites: 22.** One production (`engine.rs:901`, §2.4).
The rest pass `None` and a fresh empty resolver, and two groups are easy to
miss because they are not in `tests/`: **9 in `host_capabilities.rs`'s own
test module** (1408, 1459, 1510, 1525, 1547, 1577, 1716, 2239, 2281) and
`engine.rs:2184`. The remainder: `tests/lifecycle_hooks.rs` ×5,
`tests/blob_store_integration.rs`, `tests/abac_integration.rs` ×2, and
`benches/wasm_engine.rs` ×**3** (71, 93, 117 — not 2).

### 4.3 `proxy::Host::call`

Replacing [host_capabilities.rs:1053-1129](../../../../crates/sandbox_wasm/src/host_capabilities.rs#L1053):

```rust
async fn call(
    &mut self,
    target: proxy::CallTarget,
    interface: String,
    method: String,
    params: String,
    options: Option<proxy::CallOptions>,
) -> Result<String, proxy::ProxyError> {
    // ...read_only guard, service_proxy upgrade, params parse: unchanged...

    let (protocol_tag, idempotent, timeout_ms, routing_key) = match &options {
        Some(o) => (o.protocol.as_deref(), o.idempotent, o.timeout_ms, o.routing_key.clone()),
        None => (None, false, None, None),
    };
    let protocol = ProxyProtocol::parse(protocol_tag)
        .map_err(proxy::ProxyError::UnsupportedProtocol)?;

    // ADR-0021 §2: the host supplies `app_instance_id`, the guest supplies
    // only the declared name. Resolution happens here, before the
    // `ProxyRequest` exists, so a guest never holds the resolved DID and
    // cannot snapshot it past a re-push.
    let target_service = match target {
        proxy::CallTarget::Service(service) => service,
        proxy::CallTarget::Dependency(name) => {
            let app_instance_id = self.app_instance_id.as_deref().ok_or_else(|| {
                proxy::ProxyError::DependencyNotBound(format!(
                    "component '{}' was not deployed as part of an app instance, so it has \
                     no declared dependency '{name}'",
                    self.component_id
                ))
            })?;
            let logical_ref = LogicalServiceRef {
                // D-A2-15 applies here too: this string came out of a
                // `service_app_context` row, not out of the guest. A
                // corrupted row is a substrate-side fault, so it maps to
                // `Internal`, not to the guest-facing "you are not bound".
                app_instance_id: AppInstanceId::try_new(app_instance_id).map_err(|e| {
                    proxy::ProxyError::Internal(format!(
                        "stored app context for '{}' is unreadable: {e}",
                        self.component_id
                    ))
                })?,
                service_name: LogicalServiceName::try_new(&name).map_err(|e| {
                    proxy::ProxyError::DependencyNotBound(format!("invalid dependency name: {e}"))
                })?,
            };
            self.logical_resolver
                .resolve(&logical_ref, routing_key.as_deref().map(str::as_bytes))
                .map_err(|e| proxy::ProxyError::DependencyNotBound(format!(
                    "dependency '{name}' of '{}' is not bound: {e}", self.component_id
                )))?
                .to_string()
        }
    };

    // The self-proxy caller-forwarding rule is unchanged, and is now
    // evaluated against the *resolved* target: a component that reaches
    // its own service through a declared dependency name is still the
    // same service, so it still forwards its real caller.
    let caller = if target_service == self.component_id {
        self.caller.clone()
    } else {
        CallerContext::service_system(&self.component_id)
    };
    let req = ProxyRequest { target_service, interface, method, params, caller,
        origin: CallOrigin::Guest { service_id: self.component_id.clone() },
        protocol, idempotent,
        timeout: timeout_ms.map(|ms| Duration::from_millis(ms.into())) };
    // ...invoke + string unwrapping: unchanged...
}
```

`LogicalResolver::resolve` is synchronous and lock-free on a cache hit, so it
adds no `.await` — which matters, because `HostState` holds non-`Sync` WASI
internals and a borrow held across a yield point breaks the generated `Host`
trait's `Send` bound (the constraint `resolve_query_auth` documents at
[host_capabilities.rs:246-253](../../../../crates/sandbox_wasm/src/host_capabilities.rs#L246)).
This is also what keeps the milestone's "resolution adds no network hop"
budget true by construction.

### 4.4 Consumers of the changed WIT

| File | Change |
|---|---|
| `test-components/proxy-test/wit/world.wit` | `call-peer` gains a `target-kind: string` (`"service"` \| `"dependency"`) argument, so one fixture drives both variants; keep `service: string` as the name/DID |
| `test-components/proxy-test/src/lib.rs` | build the `proxy::CallTarget` from `target_kind`; regenerate `src/bindings.rs` |
| `crates/router/tests/proxy_dispatch.rs` | every `json_rpc_body("call-peer", params)` gains the new argument |
| `crates/coordinator_iroh/tests/multi_hop_relay.rs:789` | same |
| `crates/sandbox_wasm/src/host_capabilities.rs` tests (`self_proxy_forwarding_does_not_extend_to_a_different_target_service`, line 1387) | `proxy::Host::call(... CallTarget::Service(...) ...)` |

`crates/core/src/test_constants.rs:61`'s comment describing `call-peer`
needs the extra argument mentioned.

**`wasm32-wasip2` check:** `greeter`, `data-layer-test`, and
`miniapp-demo1-web` import no proxy interface (verified by
`grep -rln proxy test-components/`), so `proxy-test` is the only fixture that
has to rebuild.

---

## §5 — Phase 3: a relationship proof that survives reinstantiation

### 5.1 `crates/rpc/src/relationship_proof.rs`

```rust
pub struct RelationshipProof {
    /// The **member master** DID this assertion is made under, when the
    /// signer holds an instance certificate; otherwise the signer's own
    /// DID (the pre-ADR-0020 shape, still what a service deployed without
    /// a master produces). Checked against the policy-declared
    /// `expected_asserter_did` -- which ADR-0020 §2 makes a member master
    /// too, so a member reinstantiated on another node keeps satisfying
    /// every policy that names it.
    pub asserter_did: String,
    pub relation: String,
    pub principal: String,
    pub ids: Vec<String>,
    pub valid_until_secs: u64,
    /// JSON `DelegationCertificate` binding the instance key that produced
    /// `signature` to `asserter_did`. Inside the signed payload, so it
    /// cannot be swapped for another master's certificate. `None` = the
    /// signature is `asserter_did`'s own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegation: Option<String>,
    pub signature: String,
}
```

`sign` is replaced (not augmented — one signing path, no second code route to
keep in step):

```rust
/// Signs `ids` with `instance`'s key. With `certificate`, the assertion is
/// made under that certificate's master and the certificate travels with
/// the proof; without one, the signer asserts as itself.
pub fn sign(
    instance: &Identity,
    certificate: Option<&DelegationCertificate>,
    relation: &str,
    principal: &str,
    ids: Vec<String>,
) -> anyhow::Result<Self> {
    let asserter_did = match certificate {
        Some(cert) => cert.master_did.clone(),
        None => derive_did_key(&instance.public_key()),
    };
    let mut proof = Self {
        asserter_did,
        relation: relation.to_string(),
        principal: principal.to_string(),
        ids,
        valid_until_secs: now_secs()? + RELATIONSHIP_PROOF_TTL_SECS,
        delegation: certificate.map(DelegationCertificate::to_json).transpose()?,
        signature: String::new(),
    };
    let unsigned = serde_json::to_value(&proof)?;
    proof.signature = instance.sign_json(&unsigned)?;
    Ok(proof)
}
```

`verify` gains one branch:

```rust
pub fn verify(&self, expected_asserter_did: &str) -> Result<(), RelationshipProofError> {
    // asserter/TTL checks: unchanged.
    // ...
    let mut unsigned = self.clone();
    unsigned.signature = String::new();
    let unsigned_value = serde_json::to_value(&unsigned)
        .map_err(|e| RelationshipProofError::Internal(e.to_string()))?;

    // The signing key is the instance key the certificate names, and the
    // certificate is what ties it to `asserter_did`. Checked with the full
    // `verify` (wall-clock expiry included), not A1's reader-level
    // `verify_chain`: a relationship proof is minted per fetch and lives 60
    // seconds, so a lapsed certificate means the responder stopped
    // renewing -- the attended posture's outage, surfaced here rather than
    // silently accepted (D-A2-12).
    let signer_did = match &self.delegation {
        Some(json) => {
            let cert = DelegationCertificate::from_json(json)
                .map_err(|e| RelationshipProofError::BadDelegation(e.to_string()))?;
            cert.verify(&self.asserter_did, &[SCOPE_SERVICE_INSTANCE])
                .map_err(|e| RelationshipProofError::BadDelegation(e.to_string()))?;
            cert.temporary_did
        }
        None => self.asserter_did.clone(),
    };
    verify_json_signature(&signer_did, &unsigned_value, &self.signature)
        .map_err(|e| RelationshipProofError::BadSignature(e.to_string()))
}
```

New error variant:

```rust
    /// The proof carries a delegation certificate that does not tie its
    /// signer to `asserter_did`, is outside its validity window, or is
    /// scoped for something other than a service instance.
    #[error("relationship proof delegation is invalid: {0}")]
    BadDelegation(String),
```

**Revocation is not checked here**, and that is a real bound worth stating:
`HandshakeVerifier` consults the master anchor on an inbound *connection*, but
a proof arrives in a response **body**, which no handshake covers. A revoked
instance key can therefore still produce an accepted proof until its
certificate expires. That is the attended posture's certificate lifetime (24 h
by default from `certify-instance`). Recorded as a backlog row rather than
solved here: fixing it means giving `verify` a `MasterAnchorResolver`, which
`crates/rpc` cannot reach without a dependency inversion.

`crates/rpc/Cargo.toml` already depends on `syneroym-identity`, so
`DelegationCertificate` and `SCOPE_SERVICE_INSTANCE` are in reach.

**`RelationshipProof::sign` call sites: 9.** One production
(`synsvc_native.rs:321`, inside `sign_relationship_proof`); four in
`relationship_proof.rs`'s own tests (139, 148, 159, 171); three in
`fdae_fetch.rs`'s tests (261, 337, 361); and one **cross-crate**, easy to miss
because it is a test fixture in a crate that has nothing to do with signing:
`sandbox_wasm/src/host_capabilities.rs:2227`. All non-production sites pass
`None` for `certificate`, preserving today's self-asserted shape.

### 5.2 `SynSvcNativeService` signs with its certificate

`crates/control_plane/src/synsvc_native.rs`:

- new field `instance_cert: Option<DelegationCertificate>`;
- `new` gains a trailing `instance_cert: Option<DelegationCertificate>`
  parameter;
- `sign_relationship_proof`
  ([synsvc_native.rs:315](../../../../crates/control_plane/src/synsvc_native.rs#L315))
  gains a `certificate: Option<&DelegationCertificate>` parameter and forwards
  it; its single call site passes `self.instance_cert.as_ref()`.

The doc comment on `service_identity`
([synsvc_native.rs:72-92](../../../../crates/control_plane/src/synsvc_native.rs#L72))
needs the second half: the derived key still signs, but the DID it asserts
under is now the member master when a certificate is installed.

**`SynSvcNativeService::new` call sites: 24**, not the 10 an earlier draft
listed. One production — `orchestration.rs:803`, which passes
`installed_instance_cert.clone()`, the value `deploy` already verified four
ways at [orchestration.rs:538-572](../../../../crates/control_plane/src/service/orchestration.rs#L538)
(no second load from the registry and no second verification). The other 23
are tests and all pass `None`, preserving their behavior exactly:
`router/tests/ucan_context.rs:94`, `proxy_dispatch.rs:381,503`, and **20 in
`router/tests/native_dispatch_identity.rs`** (278, 343, 389, 433, 473, 517,
560, 643, 701, 914, 989, 1108, 1211, 1306, 1483, 1561, 1772, 2369, 2614,
2786).

**The cached-certificate invariant, stated because D-A2-12 makes it load-bearing.**
`SynSvcNativeService` holds the certificate by value from construction, while
`ProxyRouter` reads it live from the registry on every call
([proxy.rs:414](../../../../crates/router/src/proxy.rs#L414)). Those cannot
drift **today**, and only for one reason:
[orchestration.rs:876](../../../../crates/control_plane/src/service/orchestration.rs#L876)
is the sole production writer of an instance certificate, it runs inside
`deploy`, and `deploy` rebuilds the native service in the same pass. Any
future code that installs a certificate outside `deploy` — an unattended
renewal is exactly that, and it is A5's — **must** also rebuild or refresh the
native service. Otherwise the cached copy goes stale, D-A2-12's wall-clock
check rejects every proof it signs, and there is no fallback to the pre-A0
self-asserted shape: `asserter_did` is already the master, so dropping the
certificate would fail the asserter check instead. A comment on the field
records this; A5 inherits the obligation.

### 5.3 What does *not* change

`crates/fdae`'s `Relation.expected_asserter_did` stays a required,
policy-declared DID (§0.3). `resolve_fetches`'s
`proof.verify(&fetch.expected_asserter_did)` call
([fdae_fetch.rs:135](../../../../crates/rpc/src/fdae_fetch.rs#L135)) is
unchanged — the whole point is that the same call now succeeds for a
reinstantiated member.

### 5.4 Migration impact — this phase *breaks* a policy shape, and says so

The symmetric half of §5.3, which the first draft left out. Once a service
holds an instance certificate it asserts under its **member master**, so a
policy whose `expected_asserter_did` names the **derived instance DID** stops
verifying the moment Phase 3 merges. That declaration is not hypothetical: it
is what
[federated_fdae_e2e.rs:407](../../../../crates/substrate/tests/federated_fdae_e2e.rs#L407)
computes, and it is the only shape available to a policy author between A0 and
A2 — certificates existed from A0 onward, and until this phase they changed
nothing about who asserted.

Concretely:

- A service deployed **without** an instance certificate is unaffected
  (`delegation: None`, self-asserted, identical to today). That covers the
  existing `federated_fdae_e2e` fixture, which installs no certificate — so
  the suite stays green.
- A service deployed **with** one needs its dependents' policies to name the
  member master instead. That is a one-line policy edit, and it is the whole
  point: the master DID is the value that survives reinstantiation, which is
  failure-matrix row 19.

Pre-release, so this is an in-place change with no compatibility shim and no
dual-acceptance window (accepting *either* DID would defeat D-B3-8 — the
policy's trust anchor would no longer pin one key). **Add a bullet to
`task.md`'s Migration impact section at sign-off**, beside the A1
`verify_endpoint_signature` and A2 proxy-interface bullets, which currently
list neither this nor its consequence for policies authored during A0-A2.

---

## §6 — Phase 4: the transport half

`crates/rpc/src/proxy.rs`:

```rust
pub enum CallOrigin {
    Guest { service_id: String },
    /// Substrate-internal. `service_id` names the deployed service the call
    /// is made *on behalf of* when there is one -- the FDAE
    /// relationship-proof fetch, which must travel as that service rather
    /// than as the node. `None` for node-level internals and tests.
    Native { service_id: Option<String> },
}
```

`crates/router/src/proxy.rs`, `invoke_remote_at`'s match
([proxy.rs:394-423](../../../../crates/router/src/proxy.rs#L394)) gains one
arm **before** the existing `(None, Native)` arm:

```rust
    // A0 built this for the guest-origin arm; the same reasoning applies to
    // a substrate-internal call made on a service's behalf. Only the
    // no-proof case: `(Some(proof), Native)` below forwards the original
    // caller's chain verbatim, which is what lets the destination
    // re-derive `subject_did`/`anchor_did` and authorize the real caller
    // (D-B3-9). Presenting the service's identity there instead would
    // silently change who the destination thinks is asking.
    (None, CallOrigin::Native { service_id: Some(sid) }) => {
        if let Some(cert) = self.registry.instance_cert(sid)
            && !cert.is_expired()
            && let Some(owner) = self.registry.owner_of(sid)
        {
            let instance = self.node_identity.derive_service_identity(&owner, sid);
            preamble.pubkey = Some(hex::encode(instance.public_key().to_bytes()));
            preamble.delegation = Some(cert);
        } else {
            preamble.pubkey = Some(hex::encode(self.node_identity.public_key().to_bytes()));
        }
    }
```

The expired/absent fallback is the node identity here, **not** anonymous —
unlike the guest arm, which falls back to anonymous because it never presented
anything before A0. A substrate-internal call has always presented the node's
key, and dropping to anonymous would break native-dispatch destinations that
reject an anonymous caller outright.

**Call sites for the `CallOrigin::Native` shape change:**

| File | Change |
|---|---|
| `crates/rpc/src/fdae_fetch.rs:103` | `CallOrigin::Native { service_id: Some(local_service_id.to_string()) }` — `resolve_fetches` and `resolve_one_fetch` gain a `local_service_id: &str` parameter |
| `crates/sandbox_wasm/src/host_capabilities.rs:270` | pass `&self.component_id` (cloned before the `.await`, per that function's own `Send` note) |
| `crates/control_plane/src/synsvc_native.rs:446` | pass `&self.service_id` |
| `crates/router/src/proxy.rs:535` (test helper), `:875` | `CallOrigin::Native { service_id: None }` |
| `crates/rpc/src/fdae_fetch.rs:280` | `matches!(received.origin, CallOrigin::Native { .. })` — an assert, not a call |
| `crates/router/benches/proxy.rs:51`, `crates/coordinator_iroh/tests/multi_hop_relay.rs:992` | `{ service_id: None }` |

**`resolve_fetches` call sites: 9** (the signature change, distinct from the
`CallOrigin` literal change above). Two production —
`sandbox_wasm/src/host_capabilities.rs:270` and
`control_plane/src/synsvc_native.rs:446`. Five in `fdae_fetch.rs`'s own tests
— **274, 292, 320, 349, 369** (an earlier draft cited only `:280`, which is
the `matches!` assert above, not a call). Two in
`router/tests/native_dispatch_identity.rs` — 2514 and 2735.

`check_native_capability_gate`'s `let CallOrigin::Guest { .. } = ... else`
([proxy.rs:211](../../../../crates/router/src/proxy.rs#L211)) is unaffected —
`Native` in any shape still falls through.

---

## §7 — Tests

### Unit

| Where | Test |
|---|---|
| `app_orchestration/src/resolver.rs` | `register_through_the_resolver_evicts_the_cached_topology` (the §3.4 landmine: a second `register` with new members is visible immediately, not after `cache_ttl`) |
| `core/src/local_registry.rs` | app context round-trips through storage; `remove_app_context` is idempotent; `all_bindings` returns what `save_binding` wrote |
| `data_db/src/registry_store.rs` | the two new tables are created on a database that predates them (the D-A0-10 regression, one table over); binding upsert replaces in place |
| `control_plane/src/service/orchestration.rs` | a deploy carrying an app context registers a resolvable binding; a redeploy that drops a dependency leaves no stale persisted row; a binding naming a non-`did:key:` member fails the deploy; undeploy clears the persisted rows; **`a_dependency_name_containing_a_slash_fails_the_deploy_rather_than_panicking`** and the same for an empty name and an invalid app instance id (D-A2-15 — these assert `Err`, and would panic the test task if `new` were used) |
| `substrate/src/runtime.rs` | **`an_unreadable_persisted_binding_is_skipped_not_fatal`** — a stored row with a `/` in its dependency name and a row with unparseable JSON both warn-and-skip, and startup completes (D-A2-15) |
| `sandbox_wasm/src/host_capabilities.rs` | `a_dependency_name_resolves_to_its_bound_member_before_the_request_is_built` (via `RecordingProxy`, asserting `ProxyRequest.target_service`); `an_unbound_dependency_name_is_dependency_not_bound_and_never_reaches_the_proxy`; `a_component_with_no_app_context_cannot_name_a_dependency`; `a_raw_did_target_is_unchanged`; `a_routing_key_selects_deterministically_across_a_two_member_binding`; `a_dependency_resolving_to_the_components_own_service_still_forwards_the_real_caller` |
| `rpc/src/relationship_proof.rs` | a proof signed by an instance key with a certificate verifies against the **master**; the same proof fails against the instance DID; a certificate from a *different* master is rejected; an expired certificate is rejected (D-A2-12); a `routing`-scoped certificate is rejected (failure-matrix row 2, third site); a tampered `delegation` field breaks the signature; the no-certificate shape still round-trips |
| `router/src/proxy.rs` | `a_native_origin_call_on_a_services_behalf_presents_that_services_instance_key`; `a_native_origin_call_with_a_caller_proof_still_forwards_the_proof_verbatim` (the D-B3-9 guard); `a_native_origin_call_with_no_service_id_still_presents_the_node_identity`; expired certificate falls back to the node identity, not anonymous |
| `sdk/src/mapper.rs` | the app context carries one binding per `depends_on` entry, with the **target's** topology mode; a plan with no dependencies emits an empty binding list; **`emit_bindings_false_publishes_no_fabricated_member_dids`** (D-A2-16 — the guard against the non-`--mint-masters` path shipping unresolvable ids) |
| `app_orchestration/src/compiler.rs` | `resolved_dependencies` is keyed by declared name |
| `roymctl` `member_identity.rs` | substitution rewrites binding members to master DIDs and preserves the dependency names |

### Integration / e2e

- `crates/router/tests/proxy_dispatch.rs`: a real WASM guest (`proxy-test`)
  driving `call-peer` with `target-kind = "dependency"` reaches the bound
  target, and reaches a *different* target after the binding is re-registered
  — the "a guest cannot snapshot an identifier it never holds" claim, proven
  rather than asserted.
- `crates/substrate/tests/dependency_binding_e2e.rs` (new, two real
  substrates, modelled on `master_endpoint_record_e2e.rs`): deploy a
  two-service app with `--mint-masters` across node A and node B; the guest on
  A calls its declared dependency by name and reaches B's member; **undeploy
  and redeploy that member on B** (a reinstantiation — new instance key, same
  master) and show the *same* call still works with no operator action and no
  binding rewrite. That is the reference scenario's step 4 seen from the
  dependent's side, which A1's e2e proved only from the registry's.
- `crates/substrate/tests/federated_fdae_e2e.rs`: extend so the responding
  service holds an instance certificate and its `RelationshipProof` verifies
  against the **member master** the policy declares — failure-matrix row 19,
  live.

### Performance budget

`task.md` makes "resolution adds no network hop — the name → master-DID step
must stay an in-process cache lookup" an **A2 obligation**, and §4.3 argues it
by construction (a synchronous `resolve`, no `.await`). An argument is not a
gate, so pin it two ways:

- `crates/router/benches/proxy.rs` gains a `dependency`-target case beside its
  existing `service`-target one. The claim under test is that the two differ
  by a cache lookup, not a round trip — assert on the *shape* of the number
  (single-digit microseconds against a warm resolver) rather than a tight
  threshold that a loaded CI runner would flake on.
- The host-side unit test `a_dependency_name_resolves_to_its_bound_member…`
  asserts `RecordingProxy` saw **exactly one** `invoke` — resolution must add
  no second proxy call, which is the failure mode a future "resolve by asking
  the supervisor" refactor would introduce and which ADR-0021 §8 forbids
  outright.

`resolver.rs`'s existing `test_cache_hit_latency_under_100ns` already covers
the resolver itself; neither of the above duplicates it.

### Gates

`cargo +nightly fmt --all`, `cargo clippy --workspace --all-targets
--all-features`, `cargo test --workspace` (sandboxed; the socket-bind failure
set documented in [status.md](status.md) is the baseline), `mise run test:e2e`,
and a `wasm32-wasip2` rebuild of `proxy-test` — the one fixture whose WIT
changes.

---

## §8 — Phase order and independent mergeability

| Phase | Content | Independently mergeable? |
|---|---|---|
| 0 | storage/registry/resolver wiring (§2) | Yes — dead code until Phase 1, but no behavior change and no risk |
| 1 | bindings on the wire (§3) | Yes — bindings are written and persisted; nothing reads them yet |
| 2 | guest-facing resolution (§4) | Needs 0+1. The WIT change lands here, so this is the one phase that forces a fixture rebuild |
| 3 | relationship-proof trust chain (§5) | **Fully independent of 0-2.** Could ship first if A2 needs to be split |
| 4 | transport half (§6) | Independent of 0-2; pairs naturally with 3 |

Suggested merge order 3 → 4 → 0 → 1 → 2: the identity half is the part with a
standing backlog row against it, has no wiring prerequisites, and closes
failure-matrix row 19 on its own.

### 8.1 What A2 hands A5, and the one thing it deliberately does not build

A2's `planned-service.app-context` is the **initial-deploy carrier** for
bindings. It is not, and must not become, the supervisor's push channel —
D-A2-6 spells out why, and it is worth restating here because the temptation
at A5 will be to reuse the field that already exists.

`deploy` is a full reinstall with no content-hash idempotency, so pushing a
membership change through it would restart every dependent. The reference
scenario's step 5 requires the opposite in as many words: *"`frontend`
resolves across both from the next call, **with no restart**."* ADR-0021 §2
names the rolling-restart alternative and rejects it; §3 keeps deploy's dedup
key `(instance, service, content hash)` and the binding write's `epoch +
content` explicitly separate, "neither covers the other."

So **A5 needs a binding-only write path** — a narrow verb that takes
`(service_id, app_instance_id, bindings, epoch)`, applies ADR-0021 §3's
four-case guard, and touches nothing else. A2 leaves it in exactly the right
shape to add: the `dependency-binding` record is already defined, the
persistence and in-memory write are already one function
(`logical_resolver.register` + `registry.save_binding`, §3.4), and that
function is the single place the epoch guard has to land. `crates/sdk/src/
lib.rs` is where the client half belongs, beside the three `deploy_*` methods.

Alternative, if A5 would rather not add a verb: give `deploy` real
`(instance, service, content hash)` idempotency — failure-matrix row 10, which
is unbuilt — so a binding-only change short-circuits the artifact and instance
work. That is strictly more work than the narrow verb and buys a row A5 has to
build anyway, so the verb is the recommendation.

---

## §9 — Backlog rows this slice adds

To be written into [deferred-backlog.md](../../deferred-backlog.md) as part of
the slice, per the mandatory update rule:

1. **Cross-app `Bind` dependency naming** (§0.2) — no manifest surface lets a
   service name a service inside a bound app instance. ADR-0021 §2 and §7
   assume one. Target: A5 / trigger is the first real cross-app dependency.
2. **`Relation.service` as a declared dependency name** (§0.3) — closing the
   D-B3-8 publication gap on the *policy* side, so `expected_asserter_did`
   becomes derivable rather than hand-declared. Needs `plan_read` to take a
   resolver. Target: A5.
3. **`ShardingStrategy` is not expressible in a manifest** (§0.5) —
   `sharded` silently means hash sharding.
4. **A relationship proof's delegation is not revocation-checked** (§5.1) —
   bounded by the certificate lifetime; needs a `MasterAnchorResolver` inside
   `crates/rpc`.
5. **Stale `StaticInventory` entries after undeploy** (D-A2-9) — in-memory
   only, cleared by restart, owned by A5's lifecycle.
6. **Binding writes are last-write-wins** (D-A2-10) — ADR-0021 §3's epoch
   guard lands at §3.4's write point in A5; failure-matrix rows 5-7.
7. **No binding-only write path** (§8.1) — A2 delivers bindings on the
   initial deploy only. A5 cannot push a membership change through `deploy`
   without restarting every dependent, which the reference scenario's step 5
   forbids. Target: **A5**, as a prerequisite of the supervisor's push rather
   than an optimization.
8. **A renewal that installs a certificate outside `deploy` must refresh
   `SynSvcNativeService`** (§5.2) — the cached certificate would otherwise go
   stale and D-A2-12's wall-clock check would reject every proof it signs,
   with no fallback. Bites the moment A5's unattended renewal lands. Target:
   **A5**.
9. **`app deploy` without `--mint-masters` binds nothing** (D-A2-16) — the
   warning is honest, but the real fix is that a manifest declaring
   `depends_on` should not have an unmastered deploy path at all. Revisit when
   the supervisor owns master custody (A5, ADR-0020 §4).

And one row to **resolve** when A2 lands: *"A service's signing identity is
still its instance key, so `expected_asserter_did` does not survive
reinstantiation"* — both halves (a) and (b) are Phases 4 and 3 respectively.
Move it to "Recently resolved."
