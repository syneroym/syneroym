# Slice B2 Phase 4 — Manifest + deploy + persistence plumbing: Implementation Plan

> Planning artifact for M04B Slice B2 **Phase 4** (deploy/persist/manifest).
> Phase 1 (`crates/fdae`), Phase 2 (`crates/data_db`), and Phase 3 (WIT
> `check-access` + host `QueryAuth` + CLS strip) are on
> `feat/m04b-slice-b2-data-db` (PR #87). This phase makes a **deployed**
> service's policy real: declared in the manifest, validated at deploy,
> persisted in `substrate.db`, and loaded at instantiation.
>
> Grounded on `feat/m04b-slice-b2-data-db` @ `0edae7e` (Phase 3 + its
> post-commit review fixes). `crates/fdae`'s `Policy`/`parse_and_validate`/
> `compile_read`, `crates/data_db`'s `QueryAuth`/`ReadOutcome`/`check_access`/
> `strip_masked_fields`, `HostState.fdae_policy`/`query_auth()`, and the WIT
> `check-access` function are **fixed ground truth** — Phase 4 changes none of
> them.
>
> Cross-refs: `slice-b2-implementation-plan.md` §9 (this phase), §10 (`strict:`),
> §12 (ambiguities), §13 (phase split); `slice-b2-phase3-plan.md`;
> ADR-0017 §1/§2.1/§8 + Amendments (2026-07-20); `task.md` Decision Register
> (D-04-02-c, -d, -g).

---

## 0. Branch + scope

Continue on `feat/m04b-slice-b2-data-db`, committing on top of `0edae7e` so
this rides PR #87 with Phases 1-3. Per AGENTS.md, staging/commits are allowed
on a feature branch (not `main`).

**Scope: Phase 4 only**, per `slice-b2-implementation-plan.md` §13.4 —
the `fdae`/`policy_path` field on both `ServiceConfig` types plus the SDK WIT
mapper, deploy-time read + validate (+ the `strict:` author-time warning),
`save_fdae_policy`/`load_fdae_policy` on `StorageProvider` with a new
`fdae_policies` table, and load-at-instantiation. Plus — resolved during
planning, see §2 — **real FDAE enforcement on the native dispatch path**,
which is where a verified external identity actually reaches the store today.

**What Phase 4 does and does not make live** is the single most important
thing to get right in this phase's reporting, and the line does **not** fall
between "native" and "WASM" — it falls between **ingresses that carry a
router-verified caller and ingresses that synthesize a capability-less
`service_system` identity** (§2). Short form: after Phase 4 a `data-layer`
read arriving from a verified external caller **is** row-filtered by the
deployed policy; a read originating inside a guest — whether through the WIT
host functions or through the guest's own `syneroym:proxy` call into its
service's native `data-layer` — carries a **loaded policy against a principal
that holds nothing**, and returns empty. Do not describe Phase 4 as "FDAE
enforcement works for guests", and do not describe it as "native is enforced,
WASM is not" either: native dispatch has both kinds of ingress.

**D-04-02-d** (stale relationship data / Zanzibar "new enemy") is re-confirmed
as an M7 deferral and **not a Phase 4 gate** — it concerns replicated
relationship state, and B2 is single-node with no replication. **D-04-02-c**
(`strict:` off by default, additive) is already implemented in the compiler
(Phase 1, `compile.rs:88-90`); Phase 4 adds only its deploy-path author-time
warning (§1.5). **D-04-02-g** stays open and is untouched: nothing here
changes `CompiledSieve`'s caveat shape, and both pinning tests keep passing
unchanged.

---

## 1. The eight pieces

Ordered as they should be sequenced. Every line number below was read on the
current tree at `0edae7e`; §9's original numbers predate Phases 1-3 and are
not reused.

### 1.1 Rust manifest — `crates/app_orchestration/src/models.rs`

`ServiceConfig` is at [models.rs:210-229](../../../../crates/app_orchestration/src/models.rs#L210)
(`schema_path` at `:226`). There is **no `ServiceManifest` struct** — §12.6's
finding still holds; `ServiceConfig` is flattened into `ServiceSpec`
(`models.rs:232-238`), which `SynAppManifest.services` maps
(`models.rs:249-259`). Add, next to `schema_path`:

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub fdae: Option<FdaeManifest>,

/// Optional declarative ReBAC policy for this service (ADR-0017 §1).
/// `#[serde(default)]` on the field above keeps every existing manifest
/// parsing unchanged -- a service with no policy is unfiltered, which is
/// the policy layer's default-*absent* (ADR-0017 §2.1).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FdaeManifest {
    pub policy_path: String,
}
```

`ServiceConfig` derives no `Default`, so **every struct literal needs the new
field** — six sites, all `fdae: None`:
[catalog.rs:57](../../../../crates/app_orchestration/src/catalog.rs#L57),
[models.rs:507](../../../../crates/app_orchestration/src/models.rs#L507),
[models.rs:620](../../../../crates/app_orchestration/src/models.rs#L620),
[journal.rs:328](../../../../crates/app_orchestration/src/journal.rs#L328),
[reconcile.rs:157](../../../../crates/app_orchestration/src/reconcile.rs#L157),
[apps/roymctl/src/commands/app.rs:67](../../../../apps/roymctl/src/commands/app.rs#L67).

The manifest TOML shape stays exactly what `task.md`'s Migration Strategy
sketched (its `ServiceManifest` heading is the stale part, not the TOML):

```toml
[services.my-svc.fdae]
policy_path = "fdae-policy.json"
```

### 1.2 WIT manifest — `crates/wit_interfaces/wit/control-plane/control-plane.wit`

The record the deploy path actually receives is
[`service-config`, control-plane.wit:23-33](../../../../crates/wit_interfaces/wit/control-plane/control-plane.wit#L23)
(`schema-path` at `:31`). Add after it:

```wit
/// Path to this service's declarative FDAE ReBAC policy document
/// (ADR-0017 §1), read and validated at deploy. Absent = no row/column
/// filtering for this service.
fdae-policy-path: option<string>,
```

Shipping only §1.1 without this is unreachable code — §9.1's warning, which
still holds: the Rust field would be dropped at the WIT boundary and
`orchestration.rs` would never see it. Unlike `data-layer.wit`, this file has
**no symlinked copies** (`find` over `crates/wit_interfaces/wit` and
`test-components` returns exactly one `control-plane.wit`, and no test
component's `wit/deps` references it), so no guest is source-affected; the
change is wire-additive.

The generated `WitServiceConfig` gains `fdae_policy_path: Option<String>`, so
**every WIT literal needs the field** — 26 sites, all `fdae_policy_path: None`:

| File | Lines |
|---|---|
| `crates/sdk/src/lib.rs` | 501, 536, 569 |
| `crates/control_plane/src/service.rs` | 386, 640, 902, 1037, 1392 |
| `crates/control_plane/src/service/orchestration.rs` | 815, 891, 972, 1042, 1131, 1209, 1230 |
| `crates/substrate/tests/http_passthrough_e2e.rs` | 43, 62 |
| `crates/coordinator_iroh/tests/multi_hop_relay.rs` | 171 |
| `crates/sandbox_wasm/tests/data_layer_integration.rs` | 63 |
| `crates/sandbox_wasm/tests/lifecycle_hooks.rs` | 81 |
| `crates/sandbox_wasm/tests/messaging_integration.rs` | 75 |
| `crates/sandbox_wasm/tests/stream_integration.rs` | 126 |
| `crates/sandbox_wasm/benches/data_layer_bench.rs` | 170 |
| `crates/router/tests/proxy_dispatch.rs` | 58 |
| `crates/router/tests/service_ownership.rs` | 190 |
| `crates/router/tests/deploy_grant.rs` | 121 |

### 1.3 The mapper — `crates/sdk/src/mapper.rs`

`map_deployment_plan_to_wit` field-copies Rust → WIT at
[mapper.rs:15-28](../../../../crates/sdk/src/mapper.rs#L15) (`schema_path` copy
at `:23`). Add:

```rust
fdae_policy_path: svc.config.fdae.as_ref().map(|f| f.policy_path.clone()),
```

This is the real manifest-driven path: `roymctl app deploy` →
[app.rs:106](../../../../apps/roymctl/src/commands/app.rs#L106)
`map_deployment_plan_to_wit` → [sdk/src/lib.rs:590](../../../../crates/sdk/src/lib.rs#L590)
`deploy_plan` → [orchestration.rs:642](../../../../crates/control_plane/src/service/orchestration.rs#L642)
`deploy_plan` → `:690` `deploy`. The three `deploy_svc_*` SDK helpers
(`lib.rs:484/519/551`) build a hardcoded empty config and only need the
`None` from §1.2 — they carry no manifest.

### 1.4 Deploy-time read + validate — `crates/control_plane/src/service/orchestration.rs`

`deploy` is at [orchestration.rs:203](../../../../crates/control_plane/src/service/orchestration.rs#L203).
The `schema_path` read+validate is at `:287-323`, and `save_config_generation`
at `:331-336`.

**Do not mirror `schema_path`'s placement.** That block is nested inside
`if let Some(custom_config_str) = &manifest.config.custom_config {` (`:282`),
so today a `schema_path` set without a `custom_config` is silently ignored.
That is defensible for a *config* schema (there is nothing to validate) but
wrong for a policy, which is independent of `custom_config`. The FDAE block
goes **after** the `custom_config` block closes (`:326`) and **before**
`save_config_generation` (`:331`), at the same nesting level as `flat_config`:

```rust
let fdae_policy = if let Some(policy_path_str) = &manifest.config.fdae_policy_path {
    let policy_path = PathBuf::from(policy_path_str);
    // Same guard as schema_path (:291-299): a deploy is remote-triggerable,
    // so the path must not escape the substrate's working directory.
    if policy_path.components().any(|c| matches!(c, Component::ParentDir))
        || policy_path.is_absolute()
    { return Err(...); }
    let doc = task::spawn_blocking(move || fs::read_to_string(&policy_path)).await??;
    Some(Arc::new(fdae::parse_and_validate(&doc).map_err(|e| format!("..."))?))
} else {
    None
};
```

Notes that must be honored:

- **Validation is a hard deploy failure**, matching `schema_path`'s
  "Configuration validation failed" precedent and ADR-0017 §1's "validated at
  deploy… the Cedar lesson". `parse_and_validate` already does JSON-Schema +
  typed deserialization + semantics (relation shapes, path resolution,
  `principal_column` coverage, acyclic `includes`, collection ambiguity) —
  `policy.rs:150-210`. Phase 4 adds no validation logic of its own.
- **The policy document is JSON, not YAML.** `parse_and_validate` is
  `serde_json::from_str` (`policy.rs:151`); ADR-0017's examples are YAML for
  readability only. Say so in the doc update (§4) so an author does not ship a
  `.yaml` and get a schema error.
- **The file is read on the substrate's side**, relative to its working
  directory — identical to `schema_path` today, and the reason for the same
  traversal guard. Do not invent a new resolution base in this phase.
- **Ordering vs. persistence**: parse first, persist second (§1.6), so an
  invalid policy never reaches `fdae_policies`. Persist before
  `deploy_wasm_service` (`:340`), so the instantiation in the `init`/`migrate`
  lifecycle hook already sees the row.
- **Rollback**: the existing failure paths call `rollback_config_generation`
  (`:69-80`). Add the analogous `fdae_policies` cleanup only where a config
  generation is already rolled back; a re-deploy overwrites the row anyway
  (`INSERT … ON CONFLICT DO UPDATE`, §1.6), so there is no stale-generation
  ladder to maintain.
- `crates/control_plane/Cargo.toml` gains `syneroym-fdae.workspace = true`
  (it has `syneroym-data-db` at `:29` but no `fdae` dependency today).

### 1.5 The `strict:` author-time warning — same file, after the service is up

§9.2 says to warn "when the manifest declares collections a policy lacks
definitions for". **A manifest declares no collections** — `ServiceConfig`
(§1.1) has no collection list, and collections are created by the guest's
`init()` hook (`create-collection`/`execute-ddl`) or by native calls. So the
warning as literally written has no referent on this tree. Resolved, rather
than deferred:

**The service's own database is the collection inventory.** Add a listing
method and warn in both directions, after `deploy_wasm_service` /
`deploy_container_service` / `deploy_tcp_service` have run (`:338-349`), which
is the first point at which a first-deploy's `init()` has created its tables:

1. a table present in the service DB with **no** matching `definitions:` entry
   → warn *"collection `x` has no FDAE definition; it is unfiltered today and
   would be denied under `strict: true`"* — this is D-04-02-c's warning
   exactly;
2. a `definitions:` entry whose `table` is **absent** from the service DB →
   warn *"policy defines `x` but no such collection exists (yet)"* — catches
   the typo/rename that otherwise fails closed silently at query time
   (ADR-0017 Amendments: the physical-column check is deliberately a lazy
   query-time check, so this is the only place a name error surfaces early).

Both are `tracing::warn!`, never errors — D-04-02-c is explicit that this
warns and does not hard-fail, and direction 2 legitimately fires for a
TCP/container service whose collections are created lazily on first use. Word
the message so "not created yet" reads as an expected case.

This needs one small additive `ServiceStore` method:

```rust
/// Lists the service's collections (user tables), excluding SQLite
/// internals and the host's own `_vault`.
async fn list_collections(&self) -> Result<Vec<String>, host_store::DataLayerError>;
```

implemented on both impls —
[`SqliteServiceStore`, sqlite.rs:1461](../../../../crates/data_db/src/sqlite.rs#L1461)
and the [`Arc<SqliteServiceStore>` forwarder, sqlite.rs:1711](../../../../crates/data_db/src/sqlite.rs#L1711)
— as `SELECT name FROM sqlite_master WHERE type='table'` minus `sqlite_%` and
`_%` (the host vault table is `_vault`, `sqlite.rs:1188`; collections are
plain named tables, `sqlite.rs:112`). `ControlPlaneService` already holds the
`key_store` and `storage_provider` it needs to `open_service_db`
(used at `orchestration.rs:382-383`).

**Alternative considered and rejected:** a purely policy-internal warning (no
new plumbing). Rejected — nearly every intra-policy defect is already a
parse-time hard error (`policy.rs:173-210`), so it would warn about nothing.

### 1.6 Persist + load — `StorageProvider` + `fdae_policies`

Mirror the config-generation pair. Trait
([traits.rs:12-97](../../../../crates/data_db/src/traits.rs#L12); the analogs
are `save_config_generation` at `:43-47` and `get_latest_config_generation` at
`:64-67`):

```rust
/// Saves (replacing) the validated FDAE policy document for a service.
async fn save_fdae_policy(&self, service_id: &str, policy_json: &str) -> anyhow::Result<()>;

/// Loads a service's persisted FDAE policy document, if any.
async fn load_fdae_policy(&self, service_id: &str) -> anyhow::Result<Option<String>>;
```

Impl on `SqliteStorageProvider`
([sqlite.rs:1132](../../../../crates/data_db/src/sqlite.rs#L1132); model the
bodies on `save_config_generation` `:1261-1299` and
`get_latest_config_generation` `:1339-1360` — `spawn_blocking` +
`substrate_conn` mutex + `params!`). There is exactly **one**
`StorageProvider` implementation in the workspace, so there is no mock to
update.

Table, in `substrate.db`:

```sql
CREATE TABLE IF NOT EXISTS fdae_policies (
    service_id  TEXT PRIMARY KEY,
    policy_json TEXT NOT NULL,
    updated_at  INTEGER NOT NULL
)
```

**No generation ladder** — unlike `config_generations`, a policy is
last-write-wins (`ON CONFLICT (service_id) DO UPDATE`), because a grant that
names a policy binds late by design (ADR-0017 Consequences: "policy
*tightening* must be immediate", which is why grants may not pin a version).

Placement: the schema-init block runs `Self::run_m3a_migration` /
`run_m3b_migration` unconditionally at `sqlite.rs:854-855`, each a bag of
`CREATE TABLE IF NOT EXISTS` (`:874-908`). Add a third, named for its content
rather than a milestone — `run_fdae_migration(conn)` — called alongside. Leave
`SUBSTRATE_SCHEMA_VERSION` (`sqlite.rs:47`, `"m3b"`) **unchanged**: every
migration is idempotent and runs on every open, the constant only feeds a log
line and one assertion (`sqlite.rs:1851`), and bumping it would either encode a
planning ID in code (AGENTS.md) or invent a version scheme this phase has no
use for. The product is pre-release; the table is created in place, with no
compat shim.

### 1.7 Native dispatch enforcement — `crates/control_plane/src/synsvc_native.rs`

This is where a deployed policy becomes **real enforcement** for a verified
external caller. It is *not* enforcement for every caller that reaches this
code: `SynSvcNativeService::dispatch` has a second ingress that synthesizes a
capability-less identity — §2 covers both, and §2.2 is the one that is easy to
miss.

- `SynSvcNativeService` ([:49-57](../../../../crates/control_plane/src/synsvc_native.rs#L49))
  gains `fdae_policy: Option<Arc<Policy>>`; `new` ([:189-205](../../../../crates/control_plane/src/synsvc_native.rs#L189))
  gains the trailing param.
- The **one production construction site** is
  [orchestration.rs:380-387](../../../../crates/control_plane/src/service/orchestration.rs#L380),
  inside `deploy`, which by then holds the `Arc<Policy>` from §1.4 — no load,
  no cache, no parse on the hot path. A re-deploy reconstructs the service, so
  a policy edit takes effect with the deploy that carries it. The durable
  `fdae_policies` row (§1.6) is what any future re-hydration path reads; there
  is no startup re-deploy path in-tree today (`native_dispatch` is populated
  only from `deploy`).
- **11 test construction sites** pass `None`:
  `crates/router/tests/ucan_context.rs:94`;
  `crates/router/tests/native_dispatch_identity.rs:226, 286, 325, 360, 399,
  437, 515, 568, 776, 846`.
- Build the `QueryAuth` per invocation from the **router-verified** caller and
  pass it at the four read/delete sites, replacing today's `None`:
  `get` ([:301](../../../../crates/control_plane/src/synsvc_native.rs#L301)),
  `query` ([:317](../../../../crates/control_plane/src/synsvc_native.rs#L317)),
  `delete_many` ([:349](../../../../crates/control_plane/src/synsvc_native.rs#L349)),
  `aggregate` ([:462](../../../../crates/control_plane/src/synsvc_native.rs#L462)).
  A private helper mirroring `HostState::query_auth`
  (`host_capabilities.rs:175-181`) keeps the four sites identical:

  ```rust
  fn query_auth<'a>(&'a self, invocation: &'a NativeInvocation) -> Option<QueryAuth<'a>> {
      self.fdae_policy.as_ref().map(|policy| QueryAuth {
          policy,
          session: &invocation.caller.session,
          service_id: &self.service_id,
      })
  }
  ```

  On the **external** ingress the session is genuinely the caller's: the
  native arm of `dispatch_json_rpc_once` rejects anonymous callers and threads
  the verified `CallerContext` into `NativeInvocation.caller`
  ([dispatch.rs:99-105](../../../../crates/router/src/route_handler/dispatch.rs#L99)).
  On the **guest self-proxy** ingress it is a synthesized `service_system`
  context and the read comes back empty — see §2.2, which is the reason this
  helper must stay exactly as written.
- **No `AuthLevel` carve-out.** The helper deliberately does not branch on
  `AuthLevel::System` (or on `caller_did.starts_with("system:")`) to fall back
  to `auth = None`. Doing so would make a guest's self-proxy route *more*
  permissive than its direct WIT `store::Host` route under the same policy —
  i.e. a guest under a policy could proxy to itself to escape it. The empty
  result of §2.2 is over-restriction; the carve-out would be a bypass. Note
  this in the code comment so it is not "simplified" later.
- **`strip_record` needs no change** ([:124-130](../../../../crates/control_plane/src/synsvc_native.rs#L124)) —
  Phase 3 added the call at both read arms precisely so this phase is a
  construction-site change. Its doc comment ("`auth` stays `None` on this
  native-dispatch path… no policy source until Phase 4") is now stale and must
  be rewritten to describe live CLS.
- **Out of this piece:** no native `check-access` JSON-RPC method. Mode A is
  not exposed on the native dispatch surface today and §9 does not ask for it;
  adding a method is a new API, not plumbing. Record it, don't ship it.

### 1.8 WASM path — load at instantiation, `crates/sandbox_wasm/src/engine.rs`

`build_store_and_instantiate` ([:612-697](../../../../crates/sandbox_wasm/src/engine.rs#L612))
already loads the config generation from storage at `:638-646` and constructs
`HostState` at `:662-674`, whose last argument is today's literal `None`
(`:673`). Phase 4 replaces that `None` with the service's policy.

`HostState::new`'s signature does **not** change (Phase 3 already added the
param at `host_capabilities.rs:144`), so **no other `HostState::new` site
needs touching** — including the one inside `engine.rs`'s own
`#[cfg(test)] mod tests` ([:1200-1212](../../../../crates/sandbox_wasm/src/engine.rs#L1200),
module opens at `:1177`), which tests interface listing, never touches the
data layer, and correctly keeps passing `None`. The ~17 other test/bench sites
keep injecting `None` or a hand-built `Some(policy)` exactly as Phase 3 left
them.

**Do not parse per instantiation.** `build_store_and_instantiate` runs on
*every* guest invocation, and `parse_and_validate` compiles the embedded
JSON Schema and re-validates on each call (`policy.rs:158-164`) — that would
put schema compilation on the hot path and straight through the ≤ 25 ms p99
budget. Instead add a resolved-policy cache next to the component cache:

- new field on `AppSandboxEngine` beside `components`
  ([:93](../../../../crates/sandbox_wasm/src/engine.rs#L93)):
  `fdae_policies: DashMap<String, Option<Arc<Policy>>>`. The value is an
  `Option` so that *"resolved: this service has no policy"* — the common
  case — is cached too, instead of re-querying `substrate.db` per invocation.
- `build_store_and_instantiate` looks up, and on a miss calls
  `load_fdae_policy` + `parse_and_validate` + inserts. A parse failure at this
  point is fail-closed-**absent**: log an error and cache `None` (the deploy
  path is what rejects a bad policy; a row that fails to parse here means the
  DB was tampered with or the crate's schema moved, and the alternative —
  denying every read — would take a service down on a substrate upgrade).
  State this reasoning in the code comment.
- evict on `stop_wasm` ([:901](../../../../crates/sandbox_wasm/src/engine.rs#L901),
  beside `self.components.remove`) and on `compile_and_cache_wasm`
  ([:964](../../../../crates/sandbox_wasm/src/engine.rs#L964)) so a re-deploy
  re-resolves rather than serving the previous policy.

Because the load comes from `fdae_policies` (not from the in-memory deploy
result), the WASM path is correct across a substrate restart: `load_cached_wasm`
recompiles from disk and the next instantiation re-resolves the policy from the
DB.

---

## 2. Which ingresses Phase 4 makes live — native enforcement vs. the synthesized-identity limitation

FDAE's effect after Phase 4 is decided by **which ingress built the
`SessionContext`**, not by whether the callee is native or WASM. Three
ingresses reach a policy-covered store; one carries a real principal and two
synthesize a capability-less one. Phase 4 must ship that asymmetry openly
rather than let "Phase 4 ✅" imply FDAE is live everywhere.

### 2.1 Enforced — external caller → native dispatch

`dispatch.rs`'s native arm
rejects an anonymous caller and threads the router-verified `CallerContext`
into `NativeInvocation.caller`
([dispatch.rs:99-105](../../../../crates/router/src/route_handler/dispatch.rs#L99)),
so `caller.session` is a genuine `SessionContext` with the caller's
capabilities and `subject_did`. §1.7's wiring therefore produces real Tier-3
filtering: `compile_read` selects branches from the caller's `with`/`can`,
binds their `caveats` and `claims`, and SQLite prunes rows the caller's ReBAC
chain does not reach. This is the enforcement Phase 4 can honestly claim — and
**only** for this ingress, not for native dispatch as such (§2.2).

### 2.2 Not enforced (empty) — guest self-proxy → native dispatch

`SynSvcNativeService::dispatch` — the exact code §1.7 wires — has a **second
production ingress**, and it does not carry a verified caller.
`ProxyRouter::invoke_local`'s `NativeHostChannel` branch copies `req.caller`
verbatim into `NativeInvocation`
([proxy.rs:251-265](../../../../crates/router/src/proxy.rs#L251)), and a guest
reaches that branch by calling `syneroym:proxy` against **its own** service:
`proxy::Host::call` always builds
`caller: CallerContext::service_system(&self.component_id)` with
`origin: CallOrigin::Guest`
([host_capabilities.rs:633-682](../../../../crates/sandbox_wasm/src/host_capabilities.rs#L633)),
`"data-layer"` is one of the six `NATIVE_CAPABILITY_INTERFACES`
([local_registry.rs:39](../../../../crates/core/src/local_registry.rs#L39)),
and `check_native_capability_gate`'s same-service exception
([proxy.rs:224-231](../../../../crates/router/src/proxy.rs#L224)) **deliberately
permits** it — the gate compares the raw `component_id` precisely so that a
component's calls to its own service are not rejected. Those two — the
`dispatch_json_rpc_once` arm of §2.1 and this branch — are the only
`NativeInvocation` producers in production code.

So this ingress carries the same capability-less identity as §2.3, lands in
the code §1.7 makes policy-aware, and therefore **also returns empty** under a
deployed policy. Consequences to hold onto:

- It is a **behavior change on an already-legitimate, already-reachable
  path**: today (`auth = None`) a guest self-proxying a `data-layer` read gets
  unfiltered results; after Phase 4, for a policy-carrying service, it gets
  none. Nothing here is a *leak* — it fails toward over-restriction, and it is
  exactly consistent with what §2.3 does to the same guest's direct WIT read.
  That consistency is the argument for leaving it alone (§1.7's no-carve-out
  rule): the alternative would make self-proxy a bypass of the direct path's
  enforcement.
- It is **not** a B3 confused-deputy case. Cross-service self-proxy stays
  blocked by the gate; this is the same-service case, and its resolution is
  the same caller-threading work as §2.3 (D-04-02-h), not a separate decision.
- It is **untested today in either direction** — `proxy_dispatch.rs` has only
  `guest_to_guest_same_node_proxy_call_returns_typed_result` (`:191`) and
  `guest_cross_service_native_capability_through_proxy_is_permission_denied`
  (`:223`); nothing drives a same-service guest → native `data-layer` call. §3
  adds one, since otherwise Phase 4 changes this path's behavior with nothing
  watching.

### 2.3 Not enforced (empty) — guest → WIT host functions

`prepare_wasm_execution`
builds `CallerContext::service_system(service_id)` for an ordinary invocation
and `local_elevated` for `init`/`migrate`
([engine.rs:711-716](../../../../crates/sandbox_wasm/src/engine.rs#L711)) —
"the callee acts as itself", settled in M04A (B0's design §6.1.2, cited in
`dispatch.rs`; A1 explicitly deferred a caller-scoped guest identity to M04B,
and `proxy.rs:276-281` records the same boundary in code). The router's real
verified caller exists in `dispatch_json_rpc_once` but is never passed into
`execute_wasm_json` ([dispatch.rs:124](../../../../crates/router/src/route_handler/dispatch.rs#L124),
[proxy.rs:302](../../../../crates/router/src/proxy.rs#L302)).

`service_system` carries **empty `capabilities`** and `subject_did =
"system:<service_id>"` ([native.rs:107-118](../../../../crates/rpc/src/native.rs#L107)).
Fed to `compile_read`, no capability covers the collection resource,
`applicable` is empty, there is no `default:` fallback to reach, and the
compiler returns `deny_all()` — `0=1`
([compile.rs:97-113](../../../../crates/fdae/src/compile.rs#L97),
`:153-159`). Concretely, after Phase 4:

> **A service that ships a policy returns empty Mode-B results, and `false`
> from every Mode-A `check-access`, for every read a guest originates — by
> either route, §2.2's self-proxy or §2.3's direct WIT call — on any
> collection whose permissions require an external principal.** The mechanism
> is wired end-to-end; the principal it would filter against is not.

**Phase 4 deliberately does not fix either of them.** Threading the real caller into the
WASM path is a cross-cutting change through `crates/router` and the whole
`execute_wasm_json` → `execute_wasm_vals` → `prepare_wasm_execution` →
`build_store_and_instantiate` chain, and it collides head-on with the proxy's
intentional "callee acts as itself" semantics for cross-service calls — which
is the same original-principal question **Slice B3 is already solving** with
ADR-0015 A5's `anchor_did` (`task.md`, Slice B3: "a row policy on the
data-owning node must filter by the original principal (`anchor`), not the
proxying service (`caller`)"). Resolving it here would fork that design.

**Therefore:**

1. Ship §1.7's and §1.8's plumbing — the mechanism should exist and be
   exercised — but claim enforcement only for §2.1's ingress.
2. Record a new Decision Register entry **D-04-02-h** in `task.md` (next free
   slot; a-g are taken) covering **both** synthesized-identity ingresses,
   drafted in §4.
3. `status.md`'s Phase 4 section states the limitation as plainly as Phase 3
   stated *"`fdae_policy` is `None` in production"*, and states it as
   per-ingress — a reader who takes away "native is enforced" without the
   §2.2 qualifier has been misinformed by the write-up.
4. Pin both ingresses with tests (§3), so the empty result is recorded
   behavior rather than a surprise.

**Consequence for reference-scenario step 22.** Step 22 reads *"a
`data-layer::query` call is transparently filtered by FDAE's SQL pushdown
sieve — unauthorized rows never reach the WASM guest"*. Phase 4 can close the
**filtering** half on §2.1's ingress, end to end from a real verified caller
(§3's native end-to-end test), but **not** the "…never reach the WASM guest"
half — that requires a guest-originated read filtered by an external
principal, which §2.2 and §2.3 between them show is unreachable until caller
threading lands (D-04-02-h / B3). §5 records it as deferred rather than
claiming it.

The Playwright suite is not the vehicle for this and is untouched:
`crates/substrate/tests/e2e/tests/` contains only `webrtc.spec.ts` and
`multi-hop.spec.ts` (browser ↔ WebRTC ↔ TCP-passthrough mini-app; the fixture
is `miniapp-demo1-web`, a Rust HTTP backend with no data-layer use, and its
browser visitors are anonymous by design). Closing a reference step with a
Rust integration test rather than Playwright is the established convention —
M04A closed steps 20/21/24/25 exactly that way
([M04A task.md:647](../M04A-proxy-and-auth-foundation/task.md#L647)).
`mise run test:e2e` therefore has nothing new to exercise in Phase 4; record
the skip with this reasoning, as Phases 2 and 3 did.

---

## 3. Tests

- **`app_orchestration` (manifest round-trip)** — a TOML manifest with
  `[services.x.fdae] policy_path = "…"` parses into `Some(FdaeManifest)`
  through `SynAppManifest::from_toml` (`models.rs:262`), survives
  `to_toml`, and a manifest **without** the block still parses with
  `fdae: None` (the `#[serde(default)]` additivity claim).
- **`sdk` mapper** — `map_deployment_plan_to_wit` copies `fdae.policy_path`
  into `fdae_policy_path`; `None` maps to `None`. This is the §9.1
  "unreachable code" guard: without it the field is silently dropped, and no
  other test would notice.
- **`data_db` storage** (`sqlite.rs`'s existing private `tests` module, beside
  the config-generation tests): `save_fdae_policy` → `load_fdae_policy` round
  trip; a second save for the same `service_id` **replaces** (last-write-wins,
  one row); `load_fdae_policy` for an unknown service is `Ok(None)`;
  `list_collections` returns created collections and excludes `_vault` and
  `sqlite_%`.
- **`control_plane` deploy** (`orchestration.rs`'s `#[cfg(test)] mod tests`,
  modeled on `test_deploy_config_schema_rejection` at
  [:913-988](../../../../crates/control_plane/src/service/orchestration.rs#L913)):
  - a valid `fdae_policy_path` deploys and the document is readable back via
    `load_fdae_policy`;
  - a **malformed / schema-invalid** policy fails the deploy with a policy
    error, and **nothing is persisted** to `fdae_policies`;
  - `../` and absolute `fdae_policy_path` are rejected by the traversal guard
    (mirror `test_deploy_plan_path_traversal` / `test_deploy_plan_absolute_path`,
    `:756`/`:833`);
  - **a policy with no `custom_config` on the manifest still validates** —
    the regression test for §1.4's placement decision, which would silently
    pass if the FDAE block were nested like `schema_path`'s;
  - the `strict:` warning fires in both directions (assert via a
    `tracing` capture or, if that is heavier than it is worth, by unit-testing
    the extracted `warn_on_policy_collection_mismatch(policy, &collections)`
    helper directly on two name sets — preferred).
- **Native end-to-end — the phase's headline test.** New
  `crates/router/tests/native_fdae_enforcement.rs` (or a section of
  `native_dispatch_identity.rs`, whose harness this reuses): build a
  `SynSvcNativeService` with `Some(policy)`, register it, seed rows through
  `dispatch_json_rpc_once` as an elevated caller, then issue `query`/`get` as
  **two different verified callers** whose `SessionContext.capabilities` and
  `subject_did` differ, and assert each sees only their own reachable rows and
  that a CLS-masked field is absent from the returned payload. The harness
  already exists — `test_caller`/`admin_caller` at
  [native_dispatch_identity.rs:68-95](../../../../crates/router/tests/native_dispatch_identity.rs#L68)
  and the real `dispatch_json_rpc_once` path used at `:216-271`. This is what
  makes the reference-step-22 *filtering* claim true on the ingress that has a
  principal (§2.1).
- **Guest self-proxy ingress — new coverage, `crates/router/tests/proxy_dispatch.rs`.**
  Two tests, since this path has none today in either direction (§2.2):
  1. **Baseline, policy-absent** — a guest proxying to its **own** service's
     `data-layer` interface reaches `SynSvcNativeService` and reads normally.
     This pins the same-service exception (`proxy.rs:224-231`) as intended
     behavior and would catch a future tightening of the gate that broke it.
     It is worth having regardless of FDAE.
  2. **Policy-present pin** — the same call against a service constructed with
     `Some(policy)` returns **empty**, because `proxy::Host::call` synthesizes
     `service_system` (`host_capabilities.rs:670`). Comment it against
     D-04-02-h, mirroring the D-04-02-g pinning convention, and instruct
     whoever threads caller identity to flip it to the rows the real caller
     should see. Without this test Phase 4 changes a live path's behavior with
     nothing watching.
- **WASM instantiation** (`sandbox_wasm`): deploy a service, `save_fdae_policy`
  a valid document, invoke, and assert the instantiated `HostState` resolved a
  `Some(policy)` (via the engine's cache or a policy-shaped observable
  behavior); assert the cache is evicted on `stop_wasm`/re-deploy; assert a
  policy-absent service still resolves `None` and behaves byte-for-byte as
  before. **Also pin §2.3's limitation as a test**, with a comment naming
  D-04-02-h and instructing whoever threads caller identity to flip it: a
  guest read under a loaded policy returns **empty** because the synthesized
  `service_system` caller holds no capability — the same "pin today's
  undesired behavior explicitly" convention Phases 2 and 3 used for D-04-02-g.
- **Unchanged and must stay green**: the D-04-02-g pins
  (`tests_fdae.rs::two_capabilities_with_conflicting_caveats_currently_narrow_to_zero_rows`,
  `host_capabilities.rs::tests::fdae_d04_02_g_extra_caveated_capability_narrows_cls_strip`),
  every Phase 2/3 test, and all pre-existing deploy tests — the ~32 literal
  sites of §1.1/§1.2 are mechanical `None`s and must change no behavior.

---

## 4. Docs to update

- **`task.md`** —
  - Slice B2 status line (`:367`): add Phase 4, with the native-live /
    WASM-mechanism-only distinction stated in the same sentence, not a
    footnote.
  - **New Decision Register entry D-04-02-h — guest-originated reads carry no
    principal.** Draft: *"Every read a guest originates runs under a
    synthesized `CallerContext::service_system(service_id)` — 'the callee acts
    as itself', settled in M04A B0/A1 — whose `SessionContext` holds no
    capabilities, so `compile_read` falls to `deny_all()` and the read returns
    empty (Mode B) / `false` (Mode A) for any permission requiring an external
    principal. **Two ingresses**, and the distinction is per-ingress, not
    native-vs-WASM: (i) the WASM engine path, `prepare_wasm_execution`
    (`engine.rs:711-716`), reaching the store through `HostState`; and (ii) a
    guest's `syneroym:proxy` call into its **own** service's native
    `data-layer` — `proxy::Host::call` synthesizes the same identity
    (`host_capabilities.rs:670`), the proxy gate's same-service exception
    deliberately permits it (`proxy.rs:224-231`), and
    `ProxyRouter::invoke_local` hands it to `SynSvcNativeService`
    (`proxy.rs:251-265`). Ingress (ii) is a **behavior change** introduced by
    Phase 4 on a path that previously read unfiltered; it fails
    over-restrictive, never open. Phase 4 ships the deploy/persist/load
    mechanism but does not thread real caller identity: that is cross-cutting
    through `crates/router` and collides with the proxy's deliberate 'callee
    acts as itself' semantics for cross-service calls — the same
    original-principal question **Slice B3 is already solving via ADR-0015
    A5's `anchor_did`**. Expected to be resolved alongside B3's `anchor_did`
    work, not as a slice of its own. Deliberately **not** worked around by
    exempting `AuthLevel::System` callers from the sieve: that would make
    ingress (ii) a bypass of ingress (i)'s enforcement. What **is** enforced
    as of Phase 4 is an external, router-verified caller reaching native
    dispatch through `dispatch_json_rpc_once` (`dispatch.rs:99-105`) — that
    ingress, and only that one. Pinned by a `sandbox_wasm` test and a
    `proxy_dispatch.rs` test asserting today's empty result at each ingress."*
  - Migration Strategy: fix the stale `ServiceManifest` heading to
    `ServiceConfig` (§12.6), and note the policy document is **JSON**.
  - Reference Scenario step 22: mark the native-path filtering half closed by
    Phase 4, the "never reach the WASM guest" half open against D-04-02-h.
  - D-04-02-c: record that the compiler side shipped in Phase 1 and the
    deploy-path author-time warning shipped here, with the collection
    inventory it actually checks against (§1.5).
- **`status.md`** — a Phase 4 section in the established shape: what was
  delivered, tests, **the §2 limitation stated plainly and per-ingress**
  (including that §2.2's self-proxy path changes behavior), decisions carried
  (JSON-only policy documents; no generation ladder for policies; warning is
  warn-only in both directions; engine-side policy cache and why), explicit
  out-of-scope, and verification evidence including the `mise run test:e2e`
  skip and its reasoning.
- **`docs/developer-guide.md`** (or wherever the service-manifest keys are
  documented alongside `schema_path`) — the `[services.<name>.fdae]` block,
  that the path is read on the substrate side relative to its working
  directory, that the document is JSON validated at deploy, and that a service
  with no policy is unfiltered (ADR-0017 §2.1 default-*absent*).
- **ADR-0017** — no change. §1's "referenced from the manifest (`policy_path`,
  `#[serde(default)]`), versioned and JSON-Schema-validated at deploy" is
  exactly what §1.1/§1.4 implement; no new decision is taken here.

---

## 5. Explicitly out of scope (deferrals, recorded not dropped)

- **Threading real caller identity into guest-originated reads** — both
  ingresses, §2.2 and §2.3; D-04-02-h; expected alongside B3's `anchor_did`.
  Also explicitly **not** worked around by an `AuthLevel::System` sieve
  exemption (§1.7).
- **Reference-scenario step 22's "never reaches the WASM guest" half** —
  blocked on the above. The filtering half is closed on §2.1's ingress by §3's
  end-to-end test. No Playwright spec is added or modified.
- **Decision trace** (ADR-0017 §9) — **Phase 5**, and worth naming the
  discomfort rather than only the deferral: Phase 4 is the first phase in
  which a *real* caller can be denied by a *deployed* policy (§2.1), which is
  precisely the "denied — what do I go edit" scenario the ADR wrote the trace
  for, and it was already deferred once in Phase 2. Held at Phase 5 anyway,
  for two reasons: surfacing a `DecisionTrace` alongside `CompiledSieve` is an
  `fdae` (Phase 1) contract change, so pulling it forward reopens a phase this
  one treats as ground truth mid-flight; and Phase 5 follows immediately on
  the same branch/PR with nothing released in between, so the exposure is a
  review window, not a shipped gap. `status.md` must say that until Phase 5, a
  deny is diagnosable only from `RUST_LOG` tracing and the policy document
  itself — do not let "Phase 4 enforces" read as "Phase 4 explains".
- **Benchmarks** — the `criterion` FDAE pushdown bench and the < 25 ms p99
  perf-budget row are **Phase 5**.
- **Failure/Security matrix sign-off** (`task.md`) — **Phase 5**, including
  the watchdog/cycle rows. Phase 4 adds no new matrix row.
- **Native `check-access` JSON-RPC method** — Mode A is not exposed on the
  native dispatch surface; adding it is new API, not plumbing (§1.7).
- **Policy-configurable watchdog budget** — still the interim
  `FDAE_MAX_VM_OPS` constant; needs an `fdae` schema field, out of Phase 4.
- **Substrate-config-level FDAE settings** — none introduced; the policy's own
  document is the whole surface.
- **B3 `anchor` terminal, B4-fdae stage-4 ABAC, B5-fdae write-path gate
  (D-04-02-f), D-04-02-e native-admission TODO, `router/src/proxy.rs`'s
  interim gate (§12.13)** — later slices, untouched. In particular do **not**
  widen the proxy gate while touching adjacent code.
- **D-04-02-d** (stale relationship data) — re-confirmed M7, not a Phase 4
  gate (§0).
- **`query_raw` sieve-awareness** — the documented CLS gap from Phase 3's
  review stands, guarded by `data-layer/admin`; unchanged here.

---

## 6. Execution order + gates

1. **Manifest fields** — Rust `ServiceConfig` + `FdaeManifest` (§1.1), WIT
   `service-config` (§1.2), mapper copy (§1.3). Compile the workspace and fix
   the ~32 literal sites in one mechanical pass; nothing behaves differently
   yet.
2. **Storage** — `fdae_policies` + `save_fdae_policy`/`load_fdae_policy` +
   `list_collections` on both `ServiceStore` impls (§1.5, §1.6), with their
   unit tests. Independently landable.
3. **Deploy** — read + validate + persist in `orchestration.rs` (§1.4), plus
   the `syneroym-fdae` dependency on `control_plane`; then the `strict:`
   warning pass (§1.5). Deploy tests (§3) green here.
4. **Native enforcement** — `SynSvcNativeService` policy field, the 11 test
   `None`s, the four `QueryAuth` sites (no `AuthLevel` carve-out), and the
   `strip_record` doc-comment rewrite (§1.7). Then the native end-to-end test
   — the headline proof — **and** the two `proxy_dispatch.rs` self-proxy tests
   (§2.2, §3): land them in the same step, because this is the step that
   changes that path's behavior.
5. **WASM instantiation** — engine policy cache + eviction + the `None` at
   `engine.rs:673` (§1.8), its tests, and the D-04-02-h limitation pin.
6. **Docs** — `task.md` (status line, D-04-02-h, Migration Strategy fix, step
   22, D-04-02-c), `status.md` Phase 4, developer guide (§4).
7. **Gates**: `cargo +nightly fmt --all`; `cargo clippy --workspace
   --all-targets --all-features` clean; `cargo test --workspace` green (modulo
   the known env-only `coordinator-iroh` socket-bind failure); `wasm32-wasip2`
   unbroken — the WIT change is additive and touches no guest-imported
   interface, so no `test-components` rebuild is required, but confirm rather
   than assume; `mise run test:e2e` unchanged and recorded as a deliberate
   skip with §2's reasoning; import-hygiene pass over every edited file; no
   planning-doc IDs in code (note `run_fdae_migration`, deliberately not named
   after a milestone).

Committing needs `--no-verify` + sandbox-off (the pre-commit hook runs stable
`fmt`, which fails on the nightly-formatted tree, and gpg signing, which fails
in-sandbox).
