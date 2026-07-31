# Slice A4 Implementation Plan — Health, Read-Only

**Status:** 📋 Planned (2026-07-30, revised 2026-07-31 after review). Not
started. Milestone: [task.md](task.md) slice **A4**. Design of record:
[ADR-0021](../../../decisions/0021-binding-propagation-and-app-supervisor.md)
§7/§8 (what the supervisor watches, and that the operator read surface is a
deliverable) and
[ADR-0020](../../../decisions/0020-stable-logical-service-identity.md) §3
(the attended posture, whose missed-renewal outage A4 must make visible).
Depends on **A0, A1, A2, A3, P0 — all Complete**. Gates A5.

**The one-sentence summary.** A manifest service may declare a readiness
probe; the substrate records what type each service is and gains a `status`
query reporting, per service, whether its instance is running and whether that
probe passes, plus facts about the node itself; a sweep in `crates/sdk` polls
every substrate an app instance is placed on, keeps the three failure signals
distinct, and writes alert rows a `roymctl app alerts` verb reads back.
Nothing is restarted, retried, or remediated.

**Review pass (2026-07-31), all eight findings incorporated.** Four changed
the design, and two of those were correctness gaps that would have shipped a
slice quietly missing one of its three required signals:

- **The substrate cannot tell a container from a TCP service** (§0.5). Both
  register `SubstrateEndpoint::TcpHostPort`
  ([orchestration.rs:426](../../../../crates/control_plane/src/service/orchestration.rs#L426),
  [:479](../../../../crates/control_plane/src/service/orchestration.rs#L479)),
  `list_impl` maps that to `"tcp"`, and `PodmanSocket` is never registered by
  orchestration at all — so the first draft's `"podman" => readyz` arm was
  dead code and every container would have reported `Unknown`. Nothing
  persists a service type. A4 now stores one, which also repairs the guess
  `readyz` makes today (D-A4-17).
- **A declared probe on a `tcp` service never ran** (§0.5), because the draft
  probed only a `Running` phase and made every `tcp` service `Unknown`. That
  narrowed `task.md`'s third signal to wasm `rpc` probes alone, silently, and
  made §9's e2e test 35 unbuildable. Phase and probe are now orthogonal
  (D-A4-7).
- **The alert store was assumed reusable by A5 and is not** (§0.12). A5's
  supervisor is a substrate role; the deployment journal is an operator-local
  file a substrate cannot open. Alerts move into a standalone `AlertStore`
  whose schema, types, and folding logic A5 points at its own database
  (D-A4-9).
- **Node facts were ungated** (§0.13), handing any verified caller the node
  DID, its compiled-in capabilities, and its registry configuration — a
  disclosure path back into the posture P0 had just closed (D-A4-18).

Plus four call-site and constant corrections: `EndpointStorage` has **four**
implementors, not two (§3.2); the dummy `ContainerEngine` needs a `readyz`
stub too (§4.5); the wire→model conversion §3.4 leaned on was never declared
(§3.5); and `CERT_NEAR_EXPIRY_SECS = 24h` would have fired on every
certificate from birth, since the default lifetime is 24 hours — the
substrate's own rule is **relative** (25% of lifetime remaining), and the
wire record did not even carry the `issued-at` needed to reproduce it
(D-A4-16).

**Verified by the same pass, recorded so it is not re-litigated:** the
70-sites/22-files and 14-sites/9-files literal counts in §8 are both exact,
and `test_wit_adherence`
([service.rs:582](../../../../crates/control_plane/src/service.rs#L582)) walks
every function in the WIT `orchestrator` interface against the dispatch table,
so a missing `"status"` arm fails on its own with no new test needed.

**Read §0 first.** Planning and review found **thirteen** places where
`task.md`'s A4 paragraph leaves a decision unmade, describes a component that
does not exist yet, or understates the work. **Six** of them change what A4
has to build. §1's decisions (D-A4-1 … D-A4-19) take §0's recommended
resolutions as given; where a decision is genuinely the requester's, it is
listed again in §12.

---

## §0 — What `task.md` leaves open, understates, or assumes

Same discipline as A0 §6 / A1 §6 / A2 §0 / A3 §0 / P0 §0: recorded here rather
than silently worked around, so `task.md` can carry a dated correction at
sign-off.

`task.md`'s entire A4 text is:

> Health-check declaration in `ServiceConfig` (absent today); a substrate-side
> per-instance status query; the supervisor's poll loop; three signals kept
> distinct because remediation differs per signal — substrate unreachable,
> instance not running, author-declared readiness probe failing. Alert events
> emitted and queryable. **No remediation yet**: watch the signal before acting
> on it.

Everything below is what those six lines do not say.

### 0.1 (Scope-changing) "the supervisor's poll loop" — there is no supervisor until A5

A4 is asked for "the supervisor's poll loop", but A5 is the slice that
introduces the **substrate role**, the **`supervisor` interface**, and the
persisted desired state (`task.md` A5). A4 therefore has no process to put a
loop in, and no interface to expose a read surface on.

Three shapes are possible:

1. **Build the substrate role early, in A4.** Pulls A5's role registration,
   config plumbing, and single-writer generation stamp forward. A resident
   loop with no remediation and no generation stamp is exactly the
   "second, competing mechanism that then has to be unwound" ADR-0021 §5
   warns against.
2. **Put the sweep in `roymctl` only.** Cheapest, but unreachable from any
   test that runs a real substrate — `roymctl` is bin-only (no `[lib]`
   target), which is why `multi_substrate_placement_e2e.rs` drives
   `sdk::deploy::apply_plan` instead of the CLI, and why A3 already carries
   an open backlog row about `app::handle` being untestable.
3. **Put the sweep in `crates/sdk` as a library function, driven one-shot by a
   `roymctl` verb.** `sdk` is a dev-dependency of `crates/substrate`, so the
   two-node e2e can call it directly — the identical decision A3 took for
   `apply_plan` (A3 §12 question 1, D-A3-14).

**Recommendation: (3).** It matches the precedent this milestone already set
one slice ago, it is testable over two real nodes, and A5 promotes `sdk` to a
real dependency and calls the *same function* from its reconcile loop instead
of reimplementing it. See **D-A4-1**, **D-A4-2**.

### 0.2 (Scope-changing) "Alert events … queryable" has no store and no read surface in A4

ADR-0021 §8 makes the operator read surface part of the `supervisor`
interface — A5's. `task.md`'s non-goals section further says alerting in this
milestone is "structured events, queryable through the operator read surface,
and published to an MQTT topic (the broker is already in-process)".

Two problems for A4:

- **"The operator read surface" does not exist yet.** A4 needs somewhere to
  put alerts. Where, exactly, is §0.12's problem — it is not as simple as
  "the journal".
- **"The broker is already in-process" is true of a *substrate*, not of A4's
  poller.** Under D-A4-1 the sweep runs in a client process, which holds no
  `MqttBroker` and has no publish path to a remote one (the `messaging`
  interface's `publish` is a *guest* host-function, and `SyneroymClient`
  exposes `subscribe` but no publish — `crates/sdk/src/lib.rs:463`).

**Recommendation:** `roymctl app alerts` is A4's read surface, over the store
§0.12 settles. **MQTT publication moves to A5**, where the supervisor *is* a
substrate role with the broker in-process. See **D-A4-10** and §12 question 2.

### 0.3 (Scope-changing) Nothing in the journal names the DID to poll

The sweep must ask a substrate about a service by `service_id`. The journal
cannot supply it. `app deploy` writes the record from `target_plan` — the
**pre-substitution** plan — and only then substitutes member-master DIDs into
a *copy* (`member_identity::substitute_and_certify_members`'s doc comment says
so outright: "the deployment journal records the plan *before* this
substitution runs, so it never holds master-DID-bearing data, only the
compiler's fabricated ids", `apps/roymctl/src/commands/app.rs:436-474`).

So for a `--mint-masters` deploy the journal's `service_id` is a fabricated id
that was never deployed anywhere, and for a plain deploy it *is* the deployed
id. `check_no_placement_change` already works around exactly this, re-deriving
the real DID from the local identities directory and falling back to the plan's
id (`apps/roymctl/src/commands/app.rs:256-260`).

**Recommendation:** extract that workaround into one shared helper,
`member_identity::deployed_service_id`, and have both the refusal and the
sweep call it — the same "two sites must not read this two ways" fix A3 made
for `current_placement`. See **D-A4-11**.

### 0.4 (Scope-changing) Adding a field to the WIT `service-config` touches 70 construction sites

`service-config` is a bindgen-generated **record**, so a new field breaks every
struct literal. There are **70** of them across **22** files (counted by
`fdae_policy:`, a field unique to that record):

| File | Sites |
|---|---|
| `crates/control_plane/src/service/orchestration.rs` | 35 |
| `crates/control_plane/src/service.rs` | 5 |
| `crates/sandbox_wasm/src/host_capabilities.rs` | 4 |
| `crates/control_plane/src/synsvc_native.rs` | 3 |
| `crates/sdk/src/lib.rs` | 3 |
| `crates/router/tests/proxy_dispatch.rs` | 3 |
| `crates/substrate/tests/http_passthrough_e2e.rs` | 2 |
| 15 further files | 1 each |

This is the same shape as A3 §8.3's undercount ("**13** call sites, not two").
It is mechanical — one `health_check: None,` line per site — but it must be
budgeted, not discovered.

The alternative is the `http_routes` precedent: carry the declaration inside
`custom_config` JSON and add **no** WIT surface at all
(`crates/control_plane/src/http_routes.rs`'s module doc argues for exactly
that). Rejected: a readiness probe is a first-class deploy-time contract the
supervisor depends on, in the same class as `schema` and `fdae-policy`, both of
which are typed fields on this record. `http_routes` earned its exemption by
being a large nested router-only concern. See **D-A4-4**.

The app-model `ServiceConfig` (`crates/app_orchestration/src/models.rs:253`)
has only **14** literal sites across 9 files — cheap by comparison.

### 0.5 (Scope-changing) The substrate does not know a container from a TCP service, so "instance not running" is unanswerable for both

This is the largest finding, and the one the first draft got wrong.

`task.md` names one signal; the substrate has three different truths behind it,
and **cannot currently tell which one applies**:

- **wasm** — nothing runs between calls. A component is compiled and cached in
  `AppSandboxEngine.components` (`crates/sandbox_wasm/src/engine.rs:98`) and
  instantiated per call. "Running" can only mean "a component is loaded".
- **container** — a real process, and `ContainerEngine::readyz` answers it via
  `podman inspect --format {{.State.Running}}`
  (`crates/sandbox_podman/src/engine.rs:459`).
- **tcp** — an *external* process the substrate merely routes to. It has no
  liveness signal at all without a declared probe.

**But a container and a TCP service are indistinguishable in the registry.**
Container deploy registers `SubstrateEndpoint::TcpHostPort`
(`orchestration.rs:479`) and so does TCP deploy (`orchestration.rs:426`);
`list_impl` maps that variant to the string `"tcp"` for both
(`orchestration.rs:1443`). `SubstrateEndpoint::PodmanSocket` exists as a
variant but **orchestration never registers it** — the only writers are the
storage round-trip and `router/src/proxy.rs`'s match. And no table in
`crates/data_db/src/registry_store.rs` records a service type: there are tables
for owners, certs, app context, and bindings, and nothing else.

Today's `readyz` papers over this with a guess — *any* `TcpHostPort` endpoint
means container (`orchestration.rs:524-537`) — which is wrong in the opposite
direction: it runs `podman inspect` against real TCP services and reports the
resulting failure as a readiness failure.

**Recommendation:** persist the service type at deploy, in the same row as the
health check (§3), and switch on that rather than on an endpoint variant that
cannot carry the distinction. `readyz`'s guess is repaired by the same fact
(**D-A4-17**) — leaving it would give one substrate two contradictory answers
about the same service.

**And phase must not gate the probe.** The first draft probed only when the
phase was `Running`, which — combined with `tcp` being honestly `Unknown` —
meant a declared probe on a TCP service *never ran*, narrowing `task.md`'s
third signal to wasm `rpc` probes alone. Phase and probe are orthogonal
signals: the substrate probes whenever the phase is `Running` **or**
`Unknown`, and skips only when it already knows the instance is down (probing
a stopped container produces a second alert for one fault). For a `tcp`
service the probe *is* the only liveness evidence there is. See **D-A4-7**.

### 0.6 (Understated) The status query is also the cheapest close for two A3 backlog rows

A3 §13 predicted this and two rows in `deferred-backlog.md` are targeted at
A4 for it:

- *"A3: substrate capabilities are operator-declared, not probed"* — the
  inventory's `capabilities` field is trusted from the operator's own TOML,
  because container support is a compile-time Cargo feature invisible on the
  wire (`crates/control_plane/Cargo.toml`'s `app_sandbox` / `podman_sandbox`).
  A status query can just report `cfg!(feature = …)`.
- *"A3: multi-substrate placement requires one registry namespace, verified
  only after the fact"* — `probe_registry_reachability`
  (`apps/roymctl/src/commands/app.rs:174`) is a heuristic over URLs `roymctl`
  was given, because nothing on the wire reports which registry a substrate
  actually publishes into. That value is
  `config.substrate.registry_url` + `enable_bep0044_dht`
  (`crates/core/src/config.rs:747`), and the substrate can simply say it.

Both are a handful of lines once a status query exists. Including them turns
A3's post-apply warning into a real **preflight refusal**. They are **not** in
`task.md`'s A4 sentence, so they are scoped as their own phase (§7.3) that can
be dropped. See **D-A4-15** and §12 question 4.

### 0.7 (Understated) `ControlPlaneService` cannot see its own config

The node facts §0.6 wants are not reachable: `ControlPlaneService` holds
engines, stores, identity, and directories, but **no `SubstrateConfig`**
(`crates/control_plane/src/service.rs:48-113`). `init` has many call sites, so
widening its signature is expensive.

**Recommendation:** read the registry facts off the `EndpointPublisher` that is
already wired in post-construction (`set_endpoint_publisher`,
`service.rs:173`) — it holds the `RegistryClient`
(`crates/core/src/endpoint_publisher.rs:47`), which just needs two accessors.
Service types come from `cfg!(feature = …)`, no config needed. No `init`
signature change. See **D-A4-14**.

### 0.8 (Understated) A probe the poller can call itself does not exist

A container's health endpoint is on a host-local port
(`SubstrateEndpoint::TcpHostPort { host: "127.0.0.1", … }`,
`orchestration.rs:479`), and a wasm probe needs the sandbox engine. Neither is
reachable from a remote poller. The probe therefore **runs on the substrate**,
inside the status query, and the result travels back on the wire — which is
what `task.md`'s phrase "substrate-side per-instance status query" implies but
does not state.

That creates a cost the performance budget names ("the supervisor's steady-state
poll must not be a meaningful load on a target substrate"): a chatty poller
would run a real probe per call, and a wasm `rpc` probe costs a component
instantiation. **Recommendation:** cache the last probe result per service for
a minimum interval and serve the cached one inside that window. See
**D-A4-8**.

### 0.9 (Stale) `roymctl svc start` / `svc stop` call orchestrator methods that do not exist

`apps/roymctl/src/commands/svc.rs:325-336` sends `orchestrator`.`start` and
`orchestrator`.`stop`. The dispatch table
(`crates/control_plane/src/service.rs:370-448`) handles only `readyz`,
`resolve-instance-identity`, `deploy`, `deploy-plan`, `undeploy`, `list` —
both calls fail with `MethodNotFound`.

Relevant because A4 introduces the notion of an instance being "not running",
which reads as if a start/stop lifecycle exists. It does not.

**Recommendation:** out of scope for a read-only slice — record a backlog row
rather than fixing it here, and do not let A4's status vocabulary imply a
lifecycle A5 has not built. See §11.

### 0.10 (Stale) The dummy sandbox has been incomplete since before this slice

`crates/control_plane/src/dummy_sandbox.rs` defines only `init` and
`exports_authorize_rows` for the no-`app_sandbox` build, and
`pub struct ContainerEngine;` with **no methods at all**, while
`orchestration.rs` calls `deploy_wasm`, `stop_wasm`, `remove_wasm`,
`unsubscribe_all`, and the container engine's `deploy`/`stop`/`remove`/`readyz`
unconditionally. So `--no-default-features` does not build today, and the gates
(`--all-features`) never exercise that path.

**Recommendation:** follow the existing convention — call the engine
unconditionally, add the mirroring dummy methods for the two accessors A4 adds
(`AppSandboxEngine::is_deployed`, `ContainerEngine::readyz`) for symmetry, and
do **not** attempt to repair the pre-existing breakage in this slice. Backlog
row.

### 0.11 (Ambiguous) Does a read-only command write to the alert store?

"Alert events emitted and queryable" requires persistence, but `app health`
reads like a pure query. A command that silently writes is surprising.

**Recommendation:** `app health` records by default — it *is* the poll loop
A4 was asked for, and A5's loop will record on exactly the same schedule —
with `--no-record` for a pure read. Named in the command's own help text. See
**D-A4-12** and §12 question 5.

### 0.12 (Scope-changing) The obvious alert store is one A5 cannot read

The first draft put alerts in the deployment journal. That does not survive
contact with A5: the journal is an **operator-local SQLite file**
(`--journal-path`, default `deployments.db` in the working directory, opened
only from `apps/roymctl/src/commands/app.rs`), while A5's supervisor is a
**substrate role** serving status and alerts over the `supervisor` interface
(`task.md` A5, ADR-0021 §8). A substrate cannot open a client-side file.

So "A5's read surface serves the same rows" was wrong, and building the store
inside `DeploymentJournal` would hand A5 a thing to unwind — the outcome
ADR-0021 §5 warns about and that §0.1 itself invokes when rejecting an early
resident loop.

**Recommendation:** what A5 reuses is the **schema, the types, and the folding
logic**, not the file. Alerts live in a standalone `AlertStore` in
`syneroym-app-orchestration` that wraps a `rusqlite::Connection` and owns its
own DDL. A4 constructs one against the journal's directory; A5 constructs one
against the supervisor's own database with no change above it, exactly as
`PlanApplier` let A6 swap a body without touching callers. See **D-A4-9** and
§12 question 6.

### 0.13 (Scope-changing) Node facts would be readable by any verified caller

`list_impl`'s visibility filter covers the *service list* and nothing else. The
first draft computed `node-facts` — node DID, compiled-in service types,
registry URL, DHT flag — unconditionally, so any caller who completes a
handshake learns what a node can run and which registry namespace it lives in.

P0 has just removed the bootstrap grant so an **unowned** substrate hands out
nothing at all; this would put a small disclosure path back, on a query
introduced to help operators.

**Recommendation:** gate node facts on **node-wide** `ORCHESTRATOR_STATUS` and
make the field `option<node-facts>` — a caller with only an app-scoped grant
still gets status for its own services and simply sees no node facts. This
couples to §7.3: `app deploy`'s preflight must handle `node: none` by saying it
could not check, not by passing silently. See **D-A4-18**.

---

## §1 — Decisions

| ID | Decision |
|---|---|
| **D-A4-1** | The sweep lives in **`crates/sdk/src/health.rs`**, behind a `StatusQuery` trait, mirroring A3's `PlanApplier` (§0.1). `SyneroymClient` is its production implementation; the e2e and unit tests use fakes. A5 calls the same `poll_once` from its reconcile loop. |
| **D-A4-2** | A4 ships a **one-shot sweep**, not a resident loop: `roymctl app health <instance>` polls once and exits, with `--watch <secs>` repeating in-process for an operator watching a screen. No daemon, no lease, no generation stamp — all A5's (ADR-0021 §4/§5). |
| **D-A4-3** | The status query takes a **list** of service ids and answers in one round trip. An empty list means "every service this caller may see", using `list_impl`'s existing visibility filter. A per-service RPC would violate the "health poll cost" budget at inventory scale. |
| **D-A4-4** | The health-check declaration is a **typed field** on both the app-model `ServiceConfig` and the WIT `service-config`, not a `custom_config` key (§0.4). Budget: 70 WIT literal sites + 14 app-model literal sites. |
| **D-A4-5** | Three probe kinds: **`tcp-connect`**, **`http-get`**, **`rpc`**. Each names an `interface` the service already registered, so a probe addresses a real endpoint rather than a second, separately-configured address. |
| **D-A4-6** | A probe kind incompatible with the service's type is a **deploy-time error** (`rpc` on a container, `http-get`/`tcp-connect` on wasm), not a runtime `failing`. A misconfiguration must not be indistinguishable from an outage. |
| **D-A4-7** | **The substrate records each service's type at deploy** (§0.5) and derives phase from it: wasm → `running` iff a component is loaded; container → `ContainerEngine::readyz`; tcp → `unknown` with a reason, because the process runs outside this substrate. A named service with no endpoints → `not-found`, kept distinct from `not-running` so a failed deploy is distinguishable from a stopped instance. A service with no recorded type (deployed by a pre-A4 binary) → `unknown`, naming the redeploy that fixes it. **Phase does not gate the probe**: the probe runs on `running` *and* `unknown`, and is skipped only for `not-running`/`not-found`, where it would report a second symptom of one fault. |
| **D-A4-8** | Probe results are **cached for `PROBE_MIN_INTERVAL_SECS` (5)** per service, and `service-status` reports `probe-checked-at` so a reader can see it got a cached answer (§0.8). |
| **D-A4-9** | Alerts live in a standalone **`AlertStore`** in `syneroym-app-orchestration`, wrapping its own `rusqlite::Connection` and owning its own DDL (§0.12). A4 opens one beside the journal; A5 opens one on the supervisor's own database. Rows carry an active/cleared lifecycle keyed on `(instance_id, logical_ref, substrate_did, kind)`. |
| **D-A4-10** | **MQTT publication of alerts is A5's**, recorded as a backlog row with its reason — A4's poller is a client process with no broker (§0.2). |
| **D-A4-11** | The DID to poll comes from **`member_identity::deployed_service_id`**, a new shared helper extracted from `check_no_placement_change`'s existing workaround: the local member-master identity if one exists, else the plan's own `service_id` (§0.3). It inherits that site's **member-index-0** assumption, which is correct today (nothing in a manifest can express more than one member) and becomes wrong the moment A5 scales a service — backlog row, and A5's own reference-scenario step 5. |
| **D-A4-12** | `app health` **records alerts by default**; `--no-record` opts out (§0.11). |
| **D-A4-13** | Substrate reachability is a **preflight per distinct substrate**, not a per-service failure: an unreachable node raises exactly one `substrate-unreachable` alert and marks every service placed there `SubstrateUnreachable`, raising no per-service alerts for them. This is what keeps `task.md`'s three signals distinct in practice. |
| **D-A4-14** | Node facts (`service-types`, `registry-url`, `dht-enabled`) are read off the wired-in `EndpointPublisher`'s `RegistryClient` plus `cfg!(feature = …)`, leaving `ControlPlaneService::init`'s signature untouched (§0.7). |
| **D-A4-15** | A3's two backlog rows (§0.6) are closed in their **own phase** (§7.3), separable from the rest of A4: declared-vs-reported capability mismatch **warns**; a split registry namespace across the inventory **refuses** at preflight, replacing A3's post-apply heuristic warning. |
| **D-A4-16** | A near-expiry instance certificate is a **fourth alert kind**, using the **same relative rule** as the substrate's own heartbeat sweep — remaining ≤ 25% of lifetime (`warn_on_near_expiry_instance_certs`, `crates/substrate/src/runtime.rs:781-804`) — not an absolute window. An absolute 24 h would fire on every certificate from birth, since `DEFAULT_INSTANCE_CERT_EXPIRES_HOURS` is 24 (`crates/sdk/src/deploy.rs:35`). The rule moves into `syneroym-identity` beside `DelegationCertificate` so the substrate and the sweep share one definition, and `service-status` carries `instance-certificate-issued-at` so the sweep can apply it. |
| **D-A4-17** | `readyz`'s "any `TcpHostPort` means container" guess (`orchestration.rs:524-537`) is **repaired** to read the recorded service type. Included because A4 creates the fact that makes the guess unnecessary, and leaving it would let one substrate give two contradictory answers about the same service. |
| **D-A4-18** | **`node-facts` is gated on node-wide `ORCHESTRATOR_STATUS`** and the field is `option<node-facts>` (§0.13). An app-scoped caller still gets its own services' status. |
| **D-A4-19** | `app health`'s exit code is driven by **faults**, not by unknowns: a `tcp` service that declared no probe is "I cannot tell", not "unhealthy", and must not make a routine sweep exit non-zero forever. `--strict` makes unknowns fail too. |

---

## §2 — Phase 1: the health-check declaration

### 2.1 `crates/app_orchestration/src/models.rs` — new types

Insert after `RotationPolicy` (`models.rs:237-241`).

```rust
/// Default readiness-probe timeout, in milliseconds. Short on purpose: a
/// probe is a liveness question, and a slow answer is already a bad one.
pub const DEFAULT_PROBE_TIMEOUT_MS: u32 = 2_000;

const fn default_probe_timeout_ms() -> u32 { DEFAULT_PROBE_TIMEOUT_MS }
const fn default_expect_status() -> u16 { 200 }

/// Author-declared readiness probe (ADR-0021 §7's active signal, applied to a
/// service the supervisor manages rather than a bound external one).
///
/// Absent means **liveness only**: the substrate reports whether the instance
/// is running and nothing more. Present means the substrate additionally runs
/// this probe and reports its outcome as a distinct signal, because
/// remediation differs between "not running" and "running but not ready".
/// For a `tcp` service, where the process runs outside the substrate
/// entirely, this is the *only* evidence of liveness there is.
///
/// Externally tagged, one struct per variant -- the shape `PlacementSelector`
/// already proved round-trips through TOML and JSON while nested inside a
/// `#[serde(flatten)]`ed `ServiceConfig`. An internally tagged (`tag = "kind"`)
/// enum reads better in TOML but has no such proof under `flatten`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HealthCheck {
    /// Open a TCP connection to the host:port `interface` is registered on.
    /// Valid for `tcp` and `container` services.
    TcpConnect(TcpProbe),
    /// HTTP GET `path` against the host:port `interface` is registered on.
    /// Valid for `tcp` and `container` services.
    HttpGet(HttpProbe),
    /// Invoke `method` on `interface` in the deployed component. Valid for
    /// `wasm` services. Any non-error return is a pass -- the probe asks
    /// whether the guest can run, not what it answers.
    Rpc(RpcProbe),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TcpProbe {
    pub interface: InterfaceName,
    #[serde(default = "default_probe_timeout_ms")]
    pub timeout_ms: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpProbe {
    pub interface: InterfaceName,
    pub path: String,
    #[serde(default = "default_expect_status")]
    pub expect_status: u16,
    #[serde(default = "default_probe_timeout_ms")]
    pub timeout_ms: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcProbe {
    pub interface: InterfaceName,
    pub method: String,
    #[serde(default = "default_probe_timeout_ms")]
    pub timeout_ms: u32,
}

impl HealthCheck {
    /// The service types this probe kind can address (D-A4-6). Read by the
    /// deploy-time validation, so a manifest error surfaces at deploy rather
    /// than as a permanently `failing` probe.
    #[must_use]
    pub const fn valid_for(&self) -> &'static [ServiceType] {
        match self {
            Self::TcpConnect(_) | Self::HttpGet(_) => {
                &[ServiceType::Tcp, ServiceType::Container]
            }
            Self::Rpc(_) => &[ServiceType::Wasm],
        }
    }

    #[must_use]
    pub fn interface(&self) -> &InterfaceName {
        match self {
            Self::TcpConnect(p) => &p.interface,
            Self::HttpGet(p) => &p.interface,
            Self::Rpc(p) => &p.interface,
        }
    }

    /// Kebab-case name of the variant, for error messages.
    #[must_use]
    pub const fn kind_name(&self) -> &'static str {
        match self {
            Self::TcpConnect(_) => "tcp-connect",
            Self::HttpGet(_) => "http-get",
            Self::Rpc(_) => "rpc",
        }
    }
}
```

Manifest form:

```toml
[services.backend]
service_type = "container"
source = "unused"
interfaces = ["http"]

[services.backend.health_check.http-get]
interface = "http"
path = "/healthz"
expect_status = 200
timeout_ms = 1500
```

### 2.2 `models.rs` — one field on `ServiceConfig`

Append to `ServiceConfig` (after `fdae`, `models.rs:272-273`):

```rust
    /// Author-declared readiness probe (M05A A4). Absent = liveness only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_check: Option<HealthCheck>,
```

Every literal of this struct gains `health_check: None,` — **14 sites**:

| File | Sites |
|---|---|
| `crates/app_orchestration/src/models.rs` (tests) | 4 |
| `crates/substrate/tests/multi_substrate_placement_e2e.rs` | 2 |
| `apps/roymctl/src/commands/app.rs` | 2 |
| `crates/app_orchestration/src/catalog.rs` | 1 |
| `crates/app_orchestration/src/journal.rs` (tests) | 1 |
| `crates/app_orchestration/src/reconcile.rs` (tests) | 1 |
| `crates/app_orchestration/src/substrate_inventory.rs` (tests) | 1 |
| `crates/sdk/src/mapper.rs` (tests) | 1 |
| `crates/sdk/src/deploy.rs` (tests) | 1 |

**Nothing needs to carry the field through by hand.** The compiler clones the
whole config (`crates/app_orchestration/src/compiler.rs:149`,
`config: spec.config.clone()`), so a manifest's declaration reaches
`PlannedService` for free. `catalog.rs:48` is the legacy single-WASM shim
building a fresh config from a bare path, so `health_check: None` there is
correct rather than a dropped value.

### 2.3 `crates/app_orchestration/src/lib.rs`

Re-export alongside the existing model types (`lib.rs:21`):

```rust
    HealthCheck, HttpProbe, PlannedService, RpcProbe, ServiceConfig, /* … */ TcpProbe,
```

### 2.4 `crates/wit_interfaces/wit/control-plane/control-plane.wit`

Insert before `record service-config` (`control-plane.wit:34`):

```wit
    record tcp-probe {
        interface-name: string,
        timeout-ms: u32,
    }

    record http-probe {
        interface-name: string,
        path: string,
        expect-status: u16,
        timeout-ms: u32,
    }

    record rpc-probe {
        interface-name: string,
        method: string,
        timeout-ms: u32,
    }

    /// Author-declared readiness probe, run by the *substrate* on behalf of a
    /// supervisor: a container's port is host-local and a wasm probe needs
    /// the sandbox engine, so neither is reachable from a remote poller.
    /// Absent leaves the service liveness-only -- and for a `tcp` service,
    /// which runs outside the substrate, absent leaves it with no liveness
    /// signal at all.
    variant health-check {
        tcp-connect(tcp-probe),
        http-get(http-probe),
        rpc(rpc-probe),
    }
```

and add to `service-config` (after `fdae-policy`, `control-plane.wit:49`):

```wit
        health-check: option<health-check>,
```

### 2.5 The 70 WIT `service-config` literal sites

Each gains `health_check: None,`. Full list in §8.1. This is mechanical; do it
in one commit of its own so review of the behavior changes is not buried in it.

### 2.6 `crates/sdk/src/mapper.rs` — map the declaration onto the wire

In `map_deployment_plan_to_wit`'s `wit_config` construction (`mapper.rs:117-138`),
after `fdae_policy`:

```rust
            health_check: svc.config.health_check.as_ref().map(map_health_check),
```

New private function, next to `map_mode` (`mapper.rs:70`):

```rust
/// Maps the app model's `HealthCheck` to the wire variant. Pure translation:
/// no defaulting, no validation -- serde already applied the field defaults
/// at parse time, and kind/type compatibility is the substrate's deploy-time
/// check (D-A4-6), so a client cannot smuggle a bad pairing past it.
fn map_health_check(check: &HealthCheck) -> WitHealthCheck {
    match check {
        HealthCheck::TcpConnect(p) => WitHealthCheck::TcpConnect(WitTcpProbe {
            interface_name: p.interface.to_string(),
            timeout_ms: p.timeout_ms,
        }),
        HealthCheck::HttpGet(p) => WitHealthCheck::HttpGet(WitHttpProbe {
            interface_name: p.interface.to_string(),
            path: p.path.clone(),
            expect_status: p.expect_status,
            timeout_ms: p.timeout_ms,
        }),
        HealthCheck::Rpc(p) => WitHealthCheck::Rpc(WitRpcProbe {
            interface_name: p.interface.to_string(),
            method: p.method.clone(),
            timeout_ms: p.timeout_ms,
        }),
    }
}
```

Imports: add `HealthCheck` to the `syneroym_app_orchestration::models` import
(`mapper.rs:6-12`) and the four WIT types to the generated-types import
(`mapper.rs:13-21`), aliased `WitHealthCheck` / `WitTcpProbe` / `WitHttpProbe` /
`WitRpcProbe` to match the file's existing `WitServiceConfig` convention.

### 2.7 `roymctl svc deploy` — deliberately unchanged

A standalone `svc deploy` builds its `DeployManifest` directly
(`crates/sdk/src/lib.rs:510-640`) and gets `health_check: None`. No CLI flag is
added: a probe belongs to an app's manifest, which is the thing a supervisor
manages. Recorded in §11 as an intentional gap, not an oversight.

---

## §3 — Phase 2: the substrate records what it deployed

Two facts must survive a restart and be re-readable at status time: **what type
the service is** (§0.5 — nothing records it today) and **its declared probe**.
They are written by the same deploy, read by the same query, and deleted by the
same undeploy, so they are **one row**, not two tables.

### 3.1 `crates/data_db/src/registry_store.rs` — new table

After the `service_app_context` table (`registry_store.rs:101-107`):

```rust
            // A4: what a deploy said this service *is*, and how to probe it.
            //
            // The type is not derivable after the fact: a container and a
            // TCP service both register `SubstrateEndpoint::TcpHostPort`
            // (orchestration.rs:426 and :479) and `PodmanSocket` is never
            // registered at all, so the endpoint variant cannot tell them
            // apart -- which is why `readyz` had to guess. One row per
            // service, upserted on redeploy, deleted on undeploy, mirroring
            // service_instance_certs.
            conn.execute(
                "CREATE TABLE IF NOT EXISTS service_deploy_facts (
                    service_id        TEXT PRIMARY KEY,
                    service_type      TEXT NOT NULL,
                    health_check_json TEXT,
                    created_at        INTEGER NOT NULL
                );",
                [],
            )?;
```

Unconditional creation, per the file's own standing note (`registry_store.rs:53-62`).

`service_type` is stored as the plain string `"wasm"` / `"container"` / `"tcp"`
and `health_check_json` as the serialized wire variant: `syneroym-core`, where
the registry lives, must not depend on the generated WIT types or on
`syneroym-app-orchestration`, so `control_plane` serializes on the way in and
parses on the way out. Same reasoning the certificate row already follows, in
reverse (a `DelegationCertificate` is a type core already has).

### 3.2 `crates/core/src/storage.rs` — three trait methods, **four** implementors

Add to `EndpointStorage`, after the cert methods (`storage.rs:42-47`):

```rust
    /// Every stored deploy fact, as `(service_id, service_type, health_check_json)`.
    async fn load_all_deploy_facts(&self) -> Result<Vec<(String, String, Option<String>)>>;
    async fn save_deploy_facts(
        &self,
        service_id: &str,
        service_type: &str,
        health_check_json: Option<&str>,
    ) -> Result<()>;
    async fn remove_deploy_facts(&self, service_id: &str) -> Result<()>;
```

**Four** implementors need updating, not two:

| Implementor | File | Work |
|---|---|---|
| `MockStorage` | `crates/core/src/storage.rs:124` | real in-memory impls, mirroring `save_cert`/`load_all_certs`/`remove_cert` (`storage.rs:151-160`) |
| `SqliteEndpointStorage` | `crates/data_db/src/registry_store.rs:153` | real impls against §3.1's table |
| `RemoveOwnerFailingStorage` | `crates/router/tests/service_ownership.rs:152` | three delegating pass-throughs |
| `FailingEndpointStorage` | `crates/control_plane/src/service/orchestration.rs:3240` | three delegating pass-throughs (it wraps a `MockStorage` in `inner`) |

### 3.3 `crates/core/src/local_registry.rs` — one map, three accessors

New field on `EndpointRegistry` (after `service_certs`, `local_registry.rs:81`):

```rust
    /// `service_id` -> `(service_type, health_check_json)` recorded at deploy
    /// (A4). Absent for a service deployed by a pre-A4 binary, which is why
    /// every reader treats a missing entry as "unknown" rather than guessing.
    service_deploy_facts: Arc<DashMap<String, (String, Option<String>)>>,
```

Initialize in `new` (`:109-117`) and `new_mock` (`:246-256`); replay in
`load_from_db` (after the app-context replay, `:152-156`):

```rust
        for (service_id, service_type, check) in self.storage.load_all_deploy_facts().await? {
            self.service_deploy_facts.insert(service_id, (service_type, check));
        }
```

Accessors, mirroring `set_instance_cert` / `instance_cert` / `remove_instance_cert`
(`:282-304`):

```rust
    pub async fn set_deploy_facts(
        &self,
        service_id: String,
        service_type: String,
        health_check_json: Option<String>,
    ) -> Result<()>;
    #[must_use]
    pub fn deploy_facts(&self, service_id: &str) -> Option<(String, Option<String>)>;
    pub async fn remove_deploy_facts(&self, service_id: &str) -> Result<()>;
```

### 3.4 `crates/control_plane/src/service/orchestration.rs` — install and clear

**Validate**, inside `deploy_with_context`, next to the existing manifest checks
and **before** any engine work (so a bad pairing costs nothing):

```rust
// D-A4-6: a probe kind that cannot address this service type is a manifest
// error. Accepting it would produce a permanently `failing` probe that is
// indistinguishable, at the supervisor, from a real outage.
let service_type = app_service_type(&manifest.service_type);   // §3.5
if let Some(check) = &manifest.config.health_check {
    let model = model_health_check(check);                     // §3.5
    if !model.valid_for().contains(&service_type) {
        return Err(format!(
            "health check '{}' cannot address a '{service_type:?}' service; it is valid for \
             {:?}",
            model.kind_name(),
            model.valid_for()
        ));
    }
    if let HealthCheck::HttpGet(p) = &model
        && !p.path.starts_with('/')
    {
        return Err(format!("http-get probe path '{}' must start with '/'", p.path));
    }
}
```

**Install**, immediately after the instance-certificate install block
(`orchestration.rs:1168-1182`), under the identical undeploy+rollback idiom.
Unlike the certificate there is no upsert-or-clear branch: the type is always
present, and a redeploy that drops the probe writes a row with a `NULL`
`health_check_json`, which clears it by construction.

```rust
let facts_result = self
    .registry
    .set_deploy_facts(
        service_id.clone(),
        service_type_str(service_type),
        manifest.config.health_check.as_ref().map(serde_json::to_string).transpose()?,
    )
    .await;
if let Err(e) = facts_result { /* undeploy + rollback_config_generation +
                                  rollback_fdae_policy, then return Err */ }
```

**Clear**, in `undeploy_impl` beside `remove_instance_cert` (`:1388`), together
with the service's cached probe result (§4.4):

```rust
if let Err(e) = self.registry.remove_deploy_facts(&service_id).await {
    tracing::warn!("Failed to remove deploy facts for {}: {}", service_id, e);
}
self.probe_cache.remove(&service_id);
```

### 3.5 `orchestration.rs` — the wire↔model conversions

Three small private functions, next to the existing `map_topology_mode`
(`orchestration.rs:70-76`), which is already precedent for a wire→model
conversion living in this file:

```rust
/// Wire `service-type` variant -> the app model's `ServiceType`. Only the
/// discriminant matters here; the payload is what the deploy already used.
const fn app_service_type(t: &WitServiceType) -> AppServiceType;

/// `ServiceType` -> the string stored in `service_deploy_facts` and reported
/// on the wire. The inverse parse lives beside it.
const fn service_type_str(t: AppServiceType) -> &'static str;
fn parse_service_type(s: &str) -> Option<AppServiceType>;

/// Wire `health-check` -> the app model's, so deploy-time validation can use
/// `HealthCheck::valid_for`/`kind_name` rather than restating the pairing
/// table on the wire type. The inverse of `sdk::mapper::map_health_check`.
fn model_health_check(c: &WitHealthCheck) -> HealthCheck;
```

`control_plane` already depends on `syneroym-app-orchestration`
(`crates/control_plane/Cargo.toml`) and already imports `TopologyMode` from it,
so this adds no dependency.

---

## §4 — Phase 3: the substrate-side status query

### 4.1 `control-plane.wit` — the wire types

Append after `deploy-plan` (`control-plane.wit:223`):

```wit
    /// Whether the substrate believes this service's instance is running.
    /// Deliberately not a bool: the supervisor's remediation differs per
    /// variant, and `unknown` must never be silently read as healthy.
    variant instance-phase {
        /// A wasm component is loaded, or the container reports Running.
        running,
        /// Deployed here, but the instance is down. Carries the reason.
        not-running(string),
        /// The substrate cannot know -- a `tcp` service runs outside it, or
        /// no service type was recorded (deployed by a pre-A4 binary).
        /// Carries which. A declared probe still runs in this state.
        unknown(string),
        /// This substrate has no endpoints registered for the id at all.
        /// Distinct from `not-running`: a deploy that never landed, versus a
        /// deploy that landed and stopped.
        not-found,
        /// The caller may not see this service. Only ever returned for an
        /// explicitly named id, never for an unnamed sweep.
        unauthorized,
    }

    variant probe-status {
        /// The service declared none; liveness only.
        not-declared,
        passing,
        failing(string),
    }

    record service-status {
        service-id: string,
        /// What the deploy said this service is: "wasm" / "container" /
        /// "tcp". Absent for a service deployed before this was recorded.
        /// Distinct from `endpoint-type`, which reports the *endpoint*
        /// variant and cannot tell a container from a TCP service.
        service-type: option<string>,
        endpoint-type: string,
        /// From the deploy's `app-context`, when it had one (A2).
        app-instance-id: option<string>,
        service-name: option<string>,
        phase: instance-phase,
        probe: probe-status,
        /// Both halves of the installed instance certificate's lifetime, so
        /// a reader can apply the same relative near-expiry rule the
        /// substrate's own heartbeat sweep uses (ADR-0020 §3) rather than
        /// inventing an absolute window.
        instance-certificate-issued-at: option<u64>,
        instance-certificate-expires-at: option<u64>,
        /// Unix seconds the probe result was produced. Earlier than
        /// `checked-at` when a cached result was served.
        probe-checked-at: option<u64>,
    }

    /// What this node is, as opposed to what is running on it. Answers two
    /// questions a deploying client could previously only guess at: what
    /// service types it can actually run, and which registry namespace it
    /// publishes endpoint records into.
    record node-facts {
        node-did: string,
        /// "wasm" / "container" / "tcp", per compiled-in sandbox features.
        service-types: list<string>,
        registry-url: option<string>,
        dht-enabled: bool,
    }

    record substrate-status {
        /// Present only for a caller holding node-wide `orchestrator/status`.
        /// An app-scoped caller still gets `services` for its own services;
        /// what a node can run and where it publishes is not theirs to read.
        node: option<node-facts>,
        checked-at: u64,
        services: list<service-status>,
    }

    /// Per-instance status for a supervisor's poll loop. An empty
    /// `service-ids` means "every service this caller may see", using the
    /// same visibility rule as `list`. A named id the caller may not see
    /// comes back `unauthorized` rather than being omitted, so a poller can
    /// tell "not permitted" from "not running".
    status: func(service-ids: list<string>) -> result<substrate-status, string>;
```

### 4.2 `orchestration.rs` — trait method

Add to `OrchestratorInterface` (`orchestration.rs:44-66`):

```rust
    async fn status(
        &self,
        service_ids: Vec<String>,
        caller: &CallerContext,
    ) -> Result<SubstrateStatus, String>;
```

Trait impl delegates to `status_impl`, matching `list`'s one-line shape
(`orchestration.rs:585-587`).

### 4.3 `service.rs` — dispatch arm

In the `match invocation.method.as_str()` block, after the `"list"` arm
(`service.rs:440-446`):

```rust
            "status" => {
                // Accepts `[[ids]]`, `[ids]`, and `{"service_ids": [...]}` --
                // the same three shapes `readyz` already tolerates
                // (service.rs:371-384), because JSON-RPC callers in this tree
                // are not consistent about positional-versus-named params.
                let service_ids: Vec<String> = parse_status_params(invocation.params);
                let status = self
                    .status(service_ids, &invocation.caller)
                    .await
                    .map_err(RpcError::InternalError)?;
                Ok(NativeResponse { payload: serde_json::to_value(status).unwrap_or(Value::Null) })
            }
```

No new adherence test is needed: `test_wit_adherence` (`service.rs:582`) already
parses the WIT `orchestrator` interface and asserts every function is
dispatchable, so omitting this arm fails an existing test.

### 4.4 `orchestration.rs` — `status_impl`

New field on `ControlPlaneService` (`service.rs:48-113`), initialized inside
`init`'s `Self { … }` so **no signature changes**:

```rust
    /// Last probe result per service, `(checked_at_secs, ProbeStatus)`.
    /// A supervisor polling every few seconds must not turn into probe load
    /// on the target (the milestone's "health poll cost" budget), and a wasm
    /// `rpc` probe costs a component instantiation. Entries are dropped on
    /// undeploy.
    probe_cache: DashMap<String, (u64, ProbeStatus)>,
```

`PROBE_MIN_INTERVAL_SECS: u64 = 5` as a module constant.

```
async fn status_impl(&self, service_ids, caller) -> Result<SubstrateStatus, String>:

    now = unix_seconds()

    # ---- node facts (D-A4-18) -------------------------------------------
    # Gated on node-wide authority, not on seeing any one service: what this
    # node can run and where it publishes is a property of the node.
    node = if self.has_node_wide_ability(caller, Ability::ORCHESTRATOR_STATUS):
               Some(NodeFacts {
                   node_did: self.node_did.clone(),
                   service_types: compiled_service_types(),        # §4.6
                   registry_url / dht_enabled: from endpoint_publisher (§4.6),
               })
           else: None

    # ---- which services -------------------------------------------------
    # `list_impl` already builds a visibility-filtered `Vec<DeployedService>`
    # and applies the ORCHESTRATOR_STATUS / owner filter. Reuse it verbatim
    # rather than re-deriving the filter -- two independently-maintained
    # visibility rules is how a disclosure bug gets introduced.
    visible = self.list_impl(caller).await?
    visible_by_id = index visible by service_id

    if service_ids.is_empty():
        targets = visible; named_missing = []
    else:
        targets, named_missing = partition service_ids on visible_by_id

    # ---- per service ----------------------------------------------------
    out = []
    for dep in targets:
        (service_type, check_json) = self.registry.deploy_facts(&dep.service_id).unzip()
        phase = self.instance_phase(&dep, service_type).await            # §4.5

        # D-A4-7: phase does NOT gate the probe. A `tcp` service is always
        # `Unknown` -- probing only `Running` would mean a declared probe
        # never runs for exactly the type that has no other signal. It is
        # skipped only where the instance is already known to be down, where
        # it would report a second symptom of one fault.
        (probe, probe_at) = match phase:
            Running | Unknown(_) => self.probe_cached(&dep.service_id, now).await   # §4.7
            _                    => (ProbeStatus::NotDeclared, None)

        cert = self.registry.instance_cert(&dep.service_id)
        app_ctx = self.registry.app_context_of(&dep.service_id)
        out.push(ServiceStatus {
            service_type, endpoint_type: dep.endpoint_type, phase, probe,
            probe_checked_at: probe_at,
            instance_certificate_issued_at:  cert.map(|c| c.issued_at_secs),
            instance_certificate_expires_at: cert.map(|c| c.expires_at_secs),
            app_instance_id / service_name from app_ctx, … })

    # A named id that is not visible: `unauthorized` if it exists on this
    # node but the caller may not see it, `not-found` otherwise. Both are
    # already inferable from `readyz`'s existing error text, so this leaks
    # nothing new -- and the distinction is what lets a poller tell a
    # credential problem from a deploy that never landed.
    for id in named_missing:
        exists = !self.registry.lookup_by_service(&id).is_empty()
        out.push(ServiceStatus { service_id: id,
                                 phase: if exists { Unauthorized } else { NotFound },
                                 probe: NotDeclared, … })

    sort out by service_id
    Ok(SubstrateStatus { node, checked_at: now, services: out })
```

### 4.5 `orchestration.rs` — `instance_phase`, and D-A4-17

```
async fn instance_phase(&self, dep, service_type: Option<String>) -> InstancePhase:
    Some(t) = service_type.as_deref().and_then(parse_service_type) else:
        # Two cases land here, both correctly "the substrate cannot say".
        # (a) Deployed by a pre-A4 binary: pre-release, there is no migration
        #     -- the row appears on the next deploy.
        # (b) The node's own `orchestrator`/`security` endpoints, which
        #     `list_impl` includes (it filters NATIVE_CAPABILITY_INTERFACES,
        #     not NODE_NATIVE_INTERFACES) and which no deploy ever created.
        # Reported honestly rather than guessed at, which is the mistake
        # `readyz` made.
        return Unknown("no service type recorded for this service; redeploy to record it")

    # Only the three types a deploy can produce reach here: the wire
    # `service-type` variant has no `native-host` case.
    match t:
        Wasm =>
            if self.app_sandbox_engine.is_deployed(&dep.service_id) { Running }
            else { NotRunning("no compiled component is loaded for this id") }
        Container =>
            match self.podman_sandbox_engine.readyz(&dep.service_id).await:
                Ok(())  => Running
                Err(e)  => NotRunning(e.to_string())
        Tcp =>
            # The process runs outside this substrate. A registration is not
            # liveness, and reporting it as `running` would be a lie the
            # supervisor then acts on. A declared probe still runs (§4.4).
            Unknown("tcp services run outside this substrate; a declared \
                     health check is their only liveness signal")
```

**D-A4-17 — repair `readyz`.** Replace the "any `TcpHostPort` means container"
guess (`orchestration.rs:524-537`) with the recorded type:

```rust
// Was: any TcpHostPort endpoint => run `podman inspect`, which fires against
// real TCP services too and reports the resulting failure as unreadiness.
if let Some((t, _)) = self.registry.deploy_facts(&service_id)
    && parse_service_type(&t) == Some(AppServiceType::Container)
{
    self.podman_sandbox_engine.readyz(&service_id).await.map_err(…)?;
}
```

A service with no recorded facts is no longer podman-inspected — matching
`status`'s `unknown`, so the two surfaces cannot disagree.

**New accessor** on `crates/sandbox_wasm/src/engine.rs`, beside
`exports_authorize_rows` (`:580`):

```rust
    /// Whether a compiled component is loaded for `service_id` -- the only
    /// liveness a wasm service has, since nothing runs between calls
    /// (M05A A4).
    #[must_use]
    pub fn is_deployed(&self, service_id: &str) -> bool {
        self.components.contains_key(service_id)
    }
```

**Two dummy stubs** in `crates/control_plane/src/dummy_sandbox.rs` (§0.10 — the
file's `ContainerEngine` has no methods at all today, so this adds an `impl`
block rather than a method):

```rust
#[cfg(not(feature = "app_sandbox"))]
impl AppSandboxEngine {
    /// A sandbox-less build never has a loaded component, mirroring
    /// `exports_authorize_rows`' own reasoning above.
    #[must_use]
    pub fn is_deployed(&self, _service_id: &str) -> bool { false }
}

#[cfg(not(feature = "podman_sandbox"))]
impl ContainerEngine {
    /// A build with no container engine cannot answer a readiness question
    /// about a container it could never have started.
    pub async fn readyz(&self, service_id: &str) -> anyhow::Result<()> {
        anyhow::bail!("container support is not compiled into this substrate ({service_id})")
    }
}
```

### 4.6 `orchestration.rs` — node facts (D-A4-14)

```rust
/// Service types this build can actually run. Container support is a
/// compile-time Cargo feature and invisible on the wire, which is why the A3
/// substrate inventory had to trust an operator-typed `capabilities` list
/// (deferred-backlog.md). `tcp` needs no engine and is always available.
fn compiled_service_types() -> Vec<String> {
    let mut types = vec!["tcp".to_string()];
    if cfg!(feature = "app_sandbox") { types.push("wasm".to_string()); }
    if cfg!(feature = "podman_sandbox") { types.push("container".to_string()); }
    types.sort();
    types
}
```

Registry facts come off the already-wired publisher, so `init` is untouched:

```rust
let (registry_url, dht_enabled) = match self.endpoint_publisher.get() {
    Some(p) => {
        let c = p.registry_client();
        (c.registry_url().map(str::to_string), c.dht_enabled())
    }
    // A substrate with no publisher wired (a test harness, or a node with
    // no registry role) reports "unknown", not "none" -- a caller must not
    // read an unwired publisher as a split-registry fleet.
    None => (None, false),
};
```

Two new accessors on `crates/core/src/dht_registry.rs`'s `RegistryClient`
(fields at `:194-201`):

```rust
    #[must_use]
    pub fn registry_url(&self) -> Option<&str> { self.registry_url.as_deref() }
    #[must_use]
    pub const fn dht_enabled(&self) -> bool { self.dht_client.is_some() }
```

### 4.7 `orchestration.rs` — running the probe

```
async fn probe_cached(&self, service_id, now) -> (ProbeStatus, Option<u64>):
    if let Some((at, status)) = self.probe_cache.get(service_id)
       and now.saturating_sub(at) < PROBE_MIN_INTERVAL_SECS:
        return (status.clone(), Some(at))
    status = self.run_probe(service_id).await
    self.probe_cache.insert(service_id.to_string(), (now, status.clone()))
    (status, Some(now))


async fn run_probe(&self, service_id) -> ProbeStatus:
    Some((_, Some(json))) = self.registry.deploy_facts(service_id) else return NotDeclared
    check: WitHealthCheck = serde_json::from_str(&json) or return
        Failing("stored health check is unreadable: {e}")

    # Resolve the declared interface to the endpoint it was registered on.
    # `lookup` handles both the exact name and the short hash.
    Some((endpoint, _)) = self.registry.lookup(service_id, check.interface_name())
        else return Failing("no endpoint registered for interface '{iface}'")

    match check:
        TcpConnect(p) =>
            SubstrateEndpoint::TcpHostPort { host, port } = endpoint
                else return Failing("interface '{iface}' is not a TCP endpoint")
            timeout(p.timeout_ms, TcpStream::connect((host, port))):
                Ok(Ok(_))  => Passing
                Ok(Err(e)) => Failing("connect failed: {e}")
                Err(_)     => Failing("connect timed out after {timeout_ms}ms")

        HttpGet(p) =>
            SubstrateEndpoint::TcpHostPort { host, port } = endpoint
                else return Failing("interface '{iface}' is not a TCP endpoint")
            url = "http://{host}:{port}{path}"     # path verified '/'-leading at deploy
            match reqwest GET url with timeout p.timeout_ms:
                Ok(r) if r.status().as_u16() == p.expect_status => Passing
                Ok(r)  => Failing("expected {expect}, got {actual}")
                Err(e) => Failing("http probe failed: {e}")

        Rpc(p) =>
            request = JsonRpcRequest { jsonrpc: "2.0", method: p.method,
                                       params: json!([]), id: Some(1) }
            # `caller: None` -- a substrate-originated probe, the same choice
            # `ProxyRouter::invoke_local` makes for a guest-to-guest call.
            match timeout(p.timeout_ms,
                          self.app_sandbox_engine
                              .execute_wasm_json(service_id, &p.interface_name,
                                                 &request, None)):
                Ok(Ok(_))  => Passing      # any non-error return passes
                Ok(Err(e)) => Failing("rpc probe failed: {e}")
                Err(_)     => Failing("rpc probe timed out after {timeout_ms}ms")
```

`crates/control_plane/Cargo.toml` gains `reqwest.workspace = true` (already a
workspace dependency, pulled in transitively today via `syneroym-core`, so no
new third-party weight).

### 4.8 `crates/sdk/src/lib.rs` — the client method

Beside `list_svcs` (`lib.rs:668-672`):

```rust
    /// A supervisor's poll: per-instance status for `service_ids`, or for
    /// every service this client may see when the list is empty (A4).
    pub async fn status(&self, service_ids: Vec<String>) -> Result<SubstrateStatus> {
        let res = self
            .request("orchestrator", "status", serde_json::json!({ "service_ids": service_ids }))
            .await?;
        Ok(serde_json::from_value(res.result)?)
    }
```

Re-export `SubstrateStatus`, `ServiceStatus`, `InstancePhase`, `ProbeStatus`,
`NodeFacts` from `crates/sdk/src/lib.rs` alongside the existing WIT re-exports.

---

## §5 — Phase 4: the alert store

### 5.1 The near-expiry rule moves to `syneroym-identity` (D-A4-16)

The substrate already has the right rule and the wrong home for sharing it:
`warn_on_near_expiry_instance_certs` (`crates/substrate/src/runtime.rs:781-804`)
computes *remaining ≤ 25% of lifetime*, which cannot be reached from `sdk`.
Move the predicate next to the type it is about, in
`crates/identity/src/delegation.rs`:

```rust
impl DelegationCertificate {
    /// Whether this certificate is within 25% of its lifetime of expiring --
    /// the renewal signal under the attended posture (ADR-0020 §3), where a
    /// missed cadence is an outage rather than a degradation.
    ///
    /// Relative, not an absolute window: `DEFAULT_INSTANCE_CERT_EXPIRES_HOURS`
    /// is 24 hours, so any absolute threshold at or above that fires on every
    /// certificate from the moment it is issued.
    #[must_use]
    pub const fn is_near_expiry(&self, now_secs: u64) -> bool {
        let lifetime = self.expires_at_secs.saturating_sub(self.issued_at_secs);
        if lifetime == 0 { return false; }
        self.expires_at_secs.saturating_sub(now_secs).saturating_mul(4) <= lifetime
    }
}
```

`runtime.rs`'s sweep calls it instead of inlining the arithmetic (its existing
test, `a_certificate_near_expiry_is_warned_about_on_the_heartbeat_sweep`, is the
regression guard); `sdk::health` applies the identical rule to the
`issued-at`/`expires-at` pair §4.1 now carries.

### 5.2 `crates/app_orchestration/src/alerts.rs` — new module (D-A4-9)

Standalone, **not** methods on `DeploymentJournal`: A4 opens it beside the
journal, and A5's substrate-side supervisor opens the same type against its own
database (§0.12). Nothing above `AlertStore` knows which.

```rust
//! Alerts raised by a health sweep, and their active/cleared lifecycle.
//!
//! Deliberately its own store rather than more tables on `DeploymentJournal`:
//! A4's sweep is an operator-local process writing beside `deployments.db`,
//! while A5's supervisor is a substrate role with its own database, and a
//! substrate cannot open a client-side file. What carries across is this
//! schema, these types, and `sdk::health::record_report`'s folding logic --
//! A5 changes only the `Connection` handed to `AlertStore::open`.

/// Why an alert was raised. One variant per distinct signal, because
/// remediation differs per signal (task.md A4) -- collapsing them would
/// erase the distinction A4 exists to establish.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AlertKind {
    /// The substrate itself did not answer. Raised once per substrate, never
    /// per service (D-A4-13).
    SubstrateUnreachable,
    /// The substrate answered and says the instance is down or absent.
    InstanceNotRunning,
    /// The declared readiness probe fails. Reachable from a `running` phase
    /// and from an `unknown` one -- a `tcp` service has no other signal.
    ProbeFailing,
    /// The installed instance certificate is within 25% of its lifetime of
    /// expiring (§5.1).
    CertificateNearExpiry,
}

impl fmt::Display for AlertKind { /* SCREAMING_SNAKE_CASE, matching
                                     DeploymentState/ActionState */ }
impl FromStr for AlertKind { /* … */ }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertRecord {
    pub id: i64,
    pub instance_id: AppInstanceId,
    /// `None` for a substrate-level alert, which belongs to no one service.
    pub logical_ref: Option<String>,
    pub substrate_alias: Option<String>,
    pub substrate_did: String,
    pub kind: AlertKind,
    pub detail: String,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
    /// `None` while the signal is still present.
    pub cleared_at: Option<i64>,
}

#[derive(Debug)]
pub struct AlertStore { conn: Connection }
```

Schema, created by `AlertStore::open`/`open_in_memory` (same unconditional
`CREATE TABLE IF NOT EXISTS` posture as `journal.rs:132-163`):

```sql
CREATE TABLE IF NOT EXISTS alerts (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    instance_id     TEXT NOT NULL,
    logical_ref     TEXT,
    substrate_alias TEXT,
    substrate_did   TEXT NOT NULL,
    kind            TEXT NOT NULL,
    detail          TEXT NOT NULL,
    first_seen_at   INTEGER NOT NULL,
    last_seen_at    INTEGER NOT NULL,
    cleared_at      INTEGER
);
CREATE INDEX IF NOT EXISTS idx_alerts_instance ON alerts(instance_id);
-- One *active* row per (instance, ref, substrate, kind). A partial unique
-- index rather than application-side checking: a second sweep must refresh
-- the existing row, never open a duplicate, and the same signal seen again
-- after being cleared is a genuinely new incident with its own row.
CREATE UNIQUE INDEX IF NOT EXISTS idx_alerts_active
    ON alerts(instance_id, IFNULL(logical_ref,''), substrate_did, kind)
    WHERE cleared_at IS NULL;
```

### 5.3 `AlertStore` API

```rust
    pub fn open<P: AsRef<Path>>(dir: P, db_name: &str) -> Result<Self>;
    pub fn open_in_memory() -> Result<Self>;

    /// Raises `kind`, or refreshes it if an active row already exists.
    /// Returns `true` when this call opened a **new** incident, so a caller
    /// can print only the transitions rather than the whole standing set on
    /// every sweep.
    pub fn raise(
        &self,
        instance_id: &AppInstanceId,
        logical_ref: Option<&str>,
        substrate_alias: Option<&str>,
        substrate_did: &str,
        kind: AlertKind,
        detail: &str,
    ) -> Result<bool>;

    /// Marks the matching active alert cleared. Returns `true` when one was
    /// actually cleared. Idempotent: clearing a signal that was never raised
    /// is a no-op, which is the ordinary case on a healthy sweep.
    pub fn clear(
        &self,
        instance_id: &AppInstanceId,
        logical_ref: Option<&str>,
        substrate_did: &str,
        kind: AlertKind,
    ) -> Result<bool>;

    /// Active alerts for an instance, oldest first.
    pub fn active(&self, instance_id: &AppInstanceId) -> Result<Vec<AlertRecord>>;
    /// Every alert for an instance including cleared ones, oldest first.
    pub fn all(&self, instance_id: &AppInstanceId) -> Result<Vec<AlertRecord>>;
```

`raise` pseudo-code:

```
now = Utc::now().timestamp()
updated = UPDATE alerts SET last_seen_at = now, detail = ?
           WHERE instance_id=? AND IFNULL(logical_ref,'')=IFNULL(?,'')
             AND substrate_did=? AND kind=? AND cleared_at IS NULL
if updated > 0: return Ok(false)
INSERT INTO alerts (..., first_seen_at=now, last_seen_at=now, cleared_at=NULL)
return Ok(true)
```

Re-export `AlertStore` / `AlertKind` / `AlertRecord` from
`crates/app_orchestration/src/lib.rs`.

---

## §6 — Phase 5: the sweep (`crates/sdk/src/health.rs`)

New module, registered in `crates/sdk/src/lib.rs` alongside `deploy`.

```rust
//! Polling the health of an app instance's services across the substrates
//! they are placed on (M05A Slice A4), read-only.
//!
//! `StatusQuery` is the read-side twin of `deploy::PlanApplier`, and lives
//! here for the same two reasons: the two-node e2e can drive it (`sdk` is a
//! dev-dependency of `crates/substrate`, `roymctl` is a binary that cannot be
//! linked from a test), and A5's reconcile loop calls this same function
//! rather than growing a second poller beside it.

/// One-shot readiness check per substrate before any status call is made,
/// matching `roymctl app deploy`'s own preflight budget.
pub const HEALTH_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

#[async_trait::async_trait]
pub trait StatusQuery: fmt::Debug + Send + Sync {
    async fn status(&self, service_ids: Vec<String>) -> Result<SubstrateStatus, String>;
}

#[async_trait::async_trait]
impl StatusQuery for SyneroymClient { /* delegates to Self::status */ }

/// A substrate to poll. Mirrors `deploy::DeployTarget` field for field.
#[derive(Debug, Clone)]
pub struct HealthTarget {
    pub alias: Option<SubstrateAlias>,
    pub substrate_did: String,
    pub query: Arc<dyn StatusQuery>,
}

/// One service the sweep expects to find, already resolved to the DID it was
/// actually deployed under (D-A4-11 -- the caller resolves it, because the
/// member-master identity files live under `roymctl`'s `--dir`, not here).
/// An empty `substrate_did` means the journal records no completed placement.
#[derive(Debug, Clone)]
pub struct ExpectedService {
    pub logical_ref: LogicalServiceRef,
    pub service_id: String,
    pub substrate_did: String,
}

/// The three signals `task.md` requires stay distinct, plus the two states
/// that are neither healthy nor a fault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signal {
    Healthy,
    /// The substrate did not answer. Never inferred from a service-level
    /// symptom (D-A4-13).
    SubstrateUnreachable(String),
    InstanceNotRunning(String),
    ProbeFailing(String),
    /// The substrate answered but cannot tell -- a `tcp` service that
    /// declared no probe, a service with no recorded type, or a caller
    /// without the grant. Not healthy, and not a fault (D-A4-19).
    Unknown(String),
    /// The journal has no completed placement for this service.
    NotDeployed,
}

impl Signal {
    /// Whether this is one of the three faults `task.md` names. `Unknown`
    /// and `NotDeployed` are deliberately excluded: "I cannot tell" must not
    /// drive a non-zero exit or an alert.
    #[must_use]
    pub const fn is_fault(&self) -> bool;
}

#[derive(Debug, Clone)]
pub struct ServiceHealth {
    pub logical_ref: LogicalServiceRef,
    pub service_id: String,
    pub alias: Option<SubstrateAlias>,
    pub substrate_did: String,
    pub signal: Signal,
    pub instance_certificate_issued_at: Option<u64>,
    pub instance_certificate_expires_at: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct SubstrateHealth {
    pub alias: Option<SubstrateAlias>,
    pub substrate_did: String,
    /// `None` when the substrate did not answer, or when this caller holds
    /// no node-wide `orchestrator/status` (D-A4-18) -- `error` distinguishes
    /// the two.
    pub node: Option<NodeFacts>,
    pub error: Option<String>,
}

#[derive(Debug, Default)]
pub struct HealthReport {
    pub substrates: Vec<SubstrateHealth>,
    pub services: Vec<ServiceHealth>,
}

impl HealthReport {
    /// Services reporting one of the three faults. Drives the exit code
    /// (D-A4-19).
    #[must_use]
    pub fn faults(&self) -> Vec<&ServiceHealth>;
    /// Services the sweep could not decide. Reported, never fatal unless the
    /// caller asked for `--strict`.
    #[must_use]
    pub fn unknowns(&self) -> Vec<&ServiceHealth>;
    /// No faults and nothing unknown.
    #[must_use]
    pub fn is_healthy(&self) -> bool;
}
```

### 6.1 `poll_once`

```
pub async fn poll_once(
    targets: &BTreeMap<String /* substrate_did */, HealthTarget>,
    expected: &[ExpectedService],
) -> HealthReport:

    report = HealthReport::default()
    by_substrate = group expected by substrate_did (skipping the empty one)

    for (did, services) in by_substrate:
        Some(target) = targets.get(did) else:
            # A placement whose substrate the caller built no target for --
            # an inventory that no longer lists the alias, say. Reported,
            # not skipped: silently dropping it would show a healthy sweep
            # for an app that is partly unwatched.
            report.substrates.push(SubstrateHealth { node: None,
                error: Some("no health target built for this substrate") })
            for s in services: push Unknown(same text)
            continue

        match target.query.status(services.map(service_id)).await:
            Err(e) =>
                # D-A4-13: one substrate-level fault, no per-service alerts.
                report.substrates.push(SubstrateHealth { node: None, error: Some(e) })
                for s in services: push ServiceHealth { signal: SubstrateUnreachable(e), … }
            Ok(status) =>
                report.substrates.push(SubstrateHealth { node: status.node, error: None })
                by_id = index status.services by service_id
                for s in services:
                    Some(st) = by_id.get(&s.service_id) else:
                        push Unknown("the substrate returned no status for this id"); continue
                    signal = match (st.phase, st.probe):
                        (NotRunning(r), _)      => InstanceNotRunning(r)
                        (NotFound, _)           => InstanceNotRunning(
                                                     "this substrate has no endpoints for the id")
                        (Unauthorized, _)       => Unknown("caller holds no orchestrator/status \
                                                            grant for this service")
                        # A failing probe is a fault whether or not the
                        # substrate could determine a phase -- for a `tcp`
                        # service it is the only signal there is (D-A4-7).
                        (Running | Unknown(_), Failing(r)) => ProbeFailing(r)
                        (Running, _)            => Healthy
                        (Unknown(_), Passing)   => Healthy
                        (Unknown(r), NotDeclared) => Unknown(r)
                    push ServiceHealth { signal, cert issued/expires from st, … }

    for s in expected where s.substrate_did.is_empty():
        push ServiceHealth { signal: NotDeployed, … }

    report.services.sort_by(logical_ref)
    report
```

### 6.2 `record_report`

```
/// Folds a report into the alert store: raises what is now failing, clears
/// what is no longer. Returns the alerts this sweep *opened*, so a caller
/// prints transitions rather than re-printing the standing set every time.
pub fn record_report(
    alerts: &AlertStore,
    instance_id: &AppInstanceId,
    report: &HealthReport,
    now: u64,
) -> Result<Vec<(AlertKind, String)>>:

    opened = []

    # ---- substrate level -------------------------------------------------
    for sub in &report.substrates:
        match &sub.error:
            Some(e) => if alerts.raise(instance_id, None, sub.alias, &sub.substrate_did,
                                       SubstrateUnreachable, e)?:
                           opened.push((SubstrateUnreachable, sub.substrate_did))
            None    => alerts.clear(instance_id, None, &sub.substrate_did,
                                    SubstrateUnreachable)?

    # ---- service level ---------------------------------------------------
    for svc in &report.services:
        l_ref = svc.logical_ref.to_string()
        # Exactly one of the two service-level kinds can be active at a time;
        # the other is cleared on every pass, so a service that moves from
        # "not running" to "probe failing" does not leave a stale alert.
        # SubstrateUnreachable is deliberately not re-raised per service --
        # it was already raised once above (D-A4-13).
        active = match svc.signal:
            InstanceNotRunning(r) => Some((InstanceNotRunning, r))
            ProbeFailing(r)       => Some((ProbeFailing, r))
            _                     => None
        for kind in [InstanceNotRunning, ProbeFailing]:
            if active.kind == Some(kind):
                if alerts.raise(instance_id, Some(&l_ref), svc.alias,
                                &svc.substrate_did, kind, &active.detail)?:
                    opened.push((kind, l_ref.clone()))
            else:
                alerts.clear(instance_id, Some(&l_ref), &svc.substrate_did, kind)?

        # ---- D-A4-16: the same relative rule the substrate uses ----------
        near = match (svc.instance_certificate_issued_at,
                      svc.instance_certificate_expires_at):
            (Some(issued), Some(expires)) =>
                DelegationCertificate::is_near_expiry_parts(issued, expires, now)
            _ => false          # no certificate installed: nothing to renew
        if near:
            if alerts.raise(…, CertificateNearExpiry,
                    "instance certificate expires at {expires}; renew with \
                     `roymctl identity certify-instance`")?:
                opened.push(…)
        else:
            alerts.clear(…, CertificateNearExpiry)?

    Ok(opened)
```

`is_near_expiry_parts(issued, expires, now)` is the free-function form of §5.1's
method, so the sweep can apply it to the two wire fields without reconstructing
a `DelegationCertificate` it never received.

---

## §7 — Phase 6: `roymctl` wiring

### 7.1 `AppCommands::Health` and `AppCommands::Alerts`

Added to `apps/roymctl/src/commands/app.rs:36-100`:

```rust
    /// Poll every substrate this app instance's services are placed on and
    /// report per-service health. Read-only: nothing is restarted, retried,
    /// or redeployed. Alerts are recorded unless `--no-record` is passed.
    /// Exits non-zero when any service reports a fault; a service the
    /// substrate could not decide about is reported but not fatal unless
    /// `--strict`.
    Health {
        instance_id: String,
        #[arg(long, default_value = "deployments.db")]
        journal_path: PathBuf,
        /// Alert store. Defaults to `alerts.db` beside the journal.
        #[arg(long)]
        alerts_path: Option<PathBuf>,
        #[arg(long)]
        inventory: Option<PathBuf>,
        /// Repeat every N seconds instead of polling once and exiting.
        #[arg(long, value_name = "SECS")]
        watch: Option<u64>,
        /// Poll and print without writing alert rows.
        #[arg(long)]
        no_record: bool,
        /// Treat an undetermined service as a failure too.
        #[arg(long)]
        strict: bool,
    },
    /// Show alerts recorded for an app instance by `app health`.
    Alerts {
        instance_id: String,
        #[arg(long)]
        alerts_path: Option<PathBuf>,
        #[arg(long, default_value = "deployments.db")]
        journal_path: PathBuf,
        /// Include alerts that have since cleared.
        #[arg(long)]
        all: bool,
    },
```

### 7.2 `handle`'s `Health` arm

```
AppCommands::Health { … }:
    instance_id = AppInstanceId::try_new(...)
    journal     = DeploymentJournal::open(parent_dir, db_name)
    alerts      = AlertStore::open(alerts_dir, alerts_name)     # default: alerts.db

    # ---- what to poll (D-A4-11, D-A4-13) --------------------------------
    Some(record) = journal.get_latest(&instance_id) else
        bail "no deployment record for {instance_id}"
    landed = journal.get_completed_actions_for_instance(&instance_id)

    expected = []; aliases = BTreeMap<String /*did*/, Option<SubstrateAlias>>
    for svc in &record.plan.services:
        l_ref = svc.logical_ref.to_string()
        match deploy::current_placement(&landed, &l_ref):
            None      => expected.push(ExpectedService { substrate_did: "".into(), … })
            Some(row) =>
                # The plan's `service_id` is the compiler's fabricated id
                # whenever the deploy minted masters (§0.3), so re-derive.
                id = member_identity::deployed_service_id(dir, svc)?
                expected.push(ExpectedService { service_id: id,
                                                substrate_did: row.substrate_did.clone(), … })
                aliases.insert(row.substrate_did.clone(), row.substrate_alias.map(…))

    # ---- one target per distinct substrate ------------------------------
    # Aliased substrates resolve through the inventory exactly as `app
    # deploy` does, including `resolve_credentials`' both-or-neither rule:
    # polling with a mismatched identity/token pair fails as a confusing
    # "holds no grant" instead of a credential error (D-A3-6's own hazard).
    inv = load inventory when any alias is present
    for (did, alias) in aliases:
        (api_url, identity, ucan) = per-alias entry, else the globals
        client = super::client_for(did.clone(), api_url, dir, identity, ucan)
        client.wait_for_ready(HEALTH_CONNECT_TIMEOUT):
            # NOT fatal, unlike `app deploy`'s preflight: an unreachable
            # substrate is the exact thing this command exists to report.
            Err(e) => targets.insert(did, HealthTarget { query:
                          Arc::new(UnreachableTarget(e.to_string())), … })
            Ok(()) => targets.insert(did, HealthTarget { query: Arc::new(client), … })

    # ---- sweep -----------------------------------------------------------
    loop:
        report = health::poll_once(&targets, &expected).await
        print_health_table(&report)
        for u in report.unknowns(): eprintln!("undetermined: {…}")
        if !no_record:
            for (kind, subject) in health::record_report(&alerts, &instance_id, &report, now)?:
                eprintln!("ALERT {kind}: {subject}")
        match watch: None => break, Some(secs) => sleep(secs)

    # D-A4-19: faults are fatal; "cannot tell" is not, unless --strict. A
    # `tcp` service that declared no probe is permanently undetermined, and
    # must not make every routine sweep exit non-zero.
    if !report.faults().is_empty() or (strict and !report.unknowns().is_empty()):
        bail "…"
```

`UnreachableTarget` is a tiny `StatusQuery` in `app.rs` that returns its stored
error — so "the connection never came up" and "the status call failed" take
**one** path into `poll_once` instead of two, and `SubstrateUnreachable` has a
single producer.

Table format, mirroring `svc list`'s column style
(`apps/roymctl/src/commands/svc.rs:308-324`):

```
SERVICE                SUBSTRATE   STATUS               DETAIL
inst-1/frontend        edge-a      HEALTHY              -
inst-1/backend         edge-b      PROBE_FAILING        expected 200, got 503
inst-1/legacy-tcp      edge-b      UNDETERMINED         no health check declared
```

### 7.3 A3's two backlog rows (D-A4-15, optional phase)

In `AppCommands::Deploy`'s existing preflight loop
(`apps/roymctl/src/commands/app.rs:366-388`), after `wait_for_ready`:

```
facts = c.status(vec![]).await.ok().and_then(|s| s.node)
match facts:
    None =>
        # D-A4-18: node facts need node-wide orchestrator/status. A deploy-only
        # or app-scoped credential legitimately cannot read them, and this must
        # say so rather than pass silently -- a skipped check that looks like a
        # passed one is worse than no check.
        eprintln!("note: cannot verify substrate '{alias}''s capabilities or registry \
                   configuration with this credential (needs node-wide \
                   orchestrator/status); falling back to the post-apply probe.")
    Some(facts) =>
        # (a) declared-vs-reported capabilities -- warn, not fail. The declared
        #     list is the operator's own record and may be deliberately narrower.
        for t in entry.capabilities not in facts.service_types:
            eprintln!("warning: substrate '{alias}' declares '{t:?}' in {inv_path} but \
                       reports it cannot run it")
        # (b) registry namespace -- collected now, decided after the loop.
        registry_facts.insert(alias, (facts.registry_url, facts.dht_enabled))

after the loop, if the plan places services on more than one substrate
and registry_facts covers every one of them:
    if not (all registry_url equal) and not (all dht_enabled):
        bail "substrates {a} and {b} publish endpoint records into different \
              registries ({url_a} vs {url_b}) and not every substrate has the \
              DHT enabled. Cross-substrate dependency calls cannot resolve. \
              Point them at one registry, or enable BEP0044 on all of them."
```

`probe_registry_reachability` **stays**: it still catches a registry that
accepted a write but cannot serve it, and it is the only check available when
the credential cannot read node facts. Its doc comment is updated to say the
namespace question is decided at preflight *when the credential allows*, and
this is the propagation check plus the fallback.

---

## §8 — Every call site that changes

### 8.1 WIT `service-config` literals — `health_check: None,` (70 sites, 22 files)

`crates/control_plane/src/service/orchestration.rs` (35) ·
`crates/control_plane/src/service.rs` (5) ·
`crates/sandbox_wasm/src/host_capabilities.rs` (4) ·
`crates/control_plane/src/synsvc_native.rs` (3) ·
`crates/sdk/src/lib.rs` (3) ·
`crates/router/tests/proxy_dispatch.rs` (3) ·
`crates/substrate/tests/http_passthrough_e2e.rs` (2) ·
and one site each in: `crates/substrate/tests/instance_identity_e2e.rs`,
`master_endpoint_record_e2e.rs`, `federated_fdae_e2e.rs`,
`multi_substrate_placement_e2e.rs`, `crates/sdk/src/mapper.rs`,
`crates/coordinator_iroh/tests/multi_hop_relay.rs`,
`crates/sandbox_wasm/tests/{messaging_integration,lifecycle_hooks,stream_integration,data_layer_integration,abac_integration}.rs`,
`crates/sandbox_wasm/benches/{data_layer_bench,abac_bench}.rs`,
`crates/router/tests/{deploy_grant,service_ownership}.rs`.

Locate with: `rg -n 'fdae_policy:' crates apps`.

### 8.2 App-model `ServiceConfig` literals — `health_check: None,` (14 sites, 9 files)

Listed in §2.2. Locate with: `rg -n 'fdae: (None|Some)' crates apps`.

### 8.3 `EndpointStorage` implementors — three new methods each (4 sites)

`crates/core/src/storage.rs:124` (`MockStorage`, real) ·
`crates/data_db/src/registry_store.rs:153` (`SqliteEndpointStorage`, real) ·
`crates/router/tests/service_ownership.rs:152` (`RemoveOwnerFailingStorage`,
delegating) · `crates/control_plane/src/service/orchestration.rs:3240`
(`FailingEndpointStorage`, delegating).

### 8.4 Non-mechanical edits

| File | Change |
|---|---|
| `crates/app_orchestration/src/models.rs` | §2.1 types, §2.2 field |
| `crates/app_orchestration/src/alerts.rs` | **new** — `AlertStore`, `AlertKind`, `AlertRecord` (§5.2, §5.3) |
| `crates/app_orchestration/src/lib.rs` | `pub mod alerts` + re-exports (§2.3, §5.3) |
| `crates/app_orchestration/src/catalog.rs` | one `health_check: None` in the legacy shim; no carry-through needed — `compiler.rs:149` clones the whole config |
| `crates/identity/src/delegation.rs` | `is_near_expiry` + `is_near_expiry_parts` (§5.1) |
| `crates/substrate/src/runtime.rs` | `warn_on_near_expiry_instance_certs` calls the moved predicate (§5.1) |
| `crates/wit_interfaces/wit/control-plane/control-plane.wit` | probe records + `health-check` (§2.4); status types + `status` func (§4.1) |
| `crates/core/src/storage.rs` | three `EndpointStorage` methods + `MockStorage` impls (§3.2) |
| `crates/data_db/src/registry_store.rs` | `service_deploy_facts` table + three impls (§3.1) |
| `crates/core/src/local_registry.rs` | field, replay, three accessors (§3.3) |
| `crates/core/src/dht_registry.rs` | `registry_url()`, `dht_enabled()` (§4.6) |
| `crates/sandbox_wasm/src/engine.rs` | `is_deployed` (§4.5) |
| `crates/control_plane/src/dummy_sandbox.rs` | `AppSandboxEngine::is_deployed` **and** `ContainerEngine::readyz` stubs (§4.5) |
| `crates/control_plane/Cargo.toml` | `reqwest.workspace = true` (§4.7) |
| `crates/control_plane/src/service.rs` | `probe_cache` field, `"status"` dispatch arm (§4.3, §4.4) |
| `crates/control_plane/src/service/orchestration.rs` | deploy-time validation + facts install + clear (§3.4); the three conversions (§3.5); `status` trait method, `status_impl`, `instance_phase`, `compiled_service_types`, `probe_cached`, `run_probe` (§4); `readyz` repaired (D-A4-17) |
| `crates/sdk/src/mapper.rs` | `map_health_check` + one field (§2.6) |
| `crates/sdk/src/lib.rs` | `status()` + re-exports (§4.8); `pub mod health` |
| `crates/sdk/src/health.rs` | **new** (§6) |
| `apps/roymctl/src/commands/member_identity.rs` | `deployed_service_id` (D-A4-11) |
| `apps/roymctl/src/commands/app.rs` | `Health`/`Alerts` variants + arms (§7.1, §7.2); `check_no_placement_change` switched to `deployed_service_id`; optional §7.3 preflight |

---

## §9 — Tests

**Unit — `crates/app_orchestration/src/models.rs`**
1. `a_health_check_round_trips_through_toml_and_json` — all three variants,
   nested inside the `#[serde(flatten)]`ed `ServiceConfig`. This is the test
   that would catch the flatten regression §2.1 designs around.
2. `an_absent_health_check_emits_no_key`.
3. `probe_defaults_apply_when_omitted` — `timeout_ms` and `expect_status`.
4. `valid_for_pairs_each_kind_with_its_service_types`.

**Unit — `crates/sdk/src/mapper.rs`**
5. `a_health_check_maps_onto_the_wire`, all three variants.
6. `no_health_check_maps_to_none`.

**Unit — `crates/identity/src/delegation.rs`**
7. `a_certificate_is_not_near_expiry_when_freshly_issued` — the exact bug the
   first draft's absolute 24 h constant would have shipped, against a 24-hour
   certificate.
8. `a_certificate_is_near_expiry_inside_the_last_quarter_of_its_lifetime`.
   `runtime.rs`'s existing
   `a_certificate_near_expiry_is_warned_about_on_the_heartbeat_sweep` is the
   regression guard for the move itself.

**Unit — `crates/control_plane/src/service/orchestration.rs`**
9. `a_probe_kind_that_cannot_address_the_service_type_is_rejected_at_deploy`
   (`rpc` on a container; `http-get` on wasm).
10. `an_http_probe_path_that_does_not_start_with_a_slash_is_rejected_at_deploy`.
11. `a_deploy_records_its_service_type_and_health_check_and_undeploy_removes_them`.
12. `a_redeploy_without_a_health_check_clears_the_stored_one`.
13. `a_container_and_a_tcp_service_are_distinguished_by_the_recorded_type` —
    the §0.5 finding, pinned: both register `TcpHostPort`, and `status` must
    still report `not-running`/`unknown` respectively.
14. `readyz_does_not_podman_inspect_a_tcp_service` (D-A4-17) — the pre-existing
    bug, now fixed.
15. `status_reports_unknown_for_a_service_with_no_recorded_type`.
16. `status_reports_not_found_for_an_id_this_substrate_has_no_endpoints_for`.
17. `status_omits_a_service_the_caller_may_not_see_and_reports_unauthorized_when_named`.
18. `node_facts_are_absent_for_a_caller_without_node_wide_status` (D-A4-18) —
    and its positive twin, `node_facts_are_returned_for_the_substrate_owner`.
19. `status_reports_the_compiled_in_service_types`.
20. `status_reports_the_registry_this_node_publishes_into`.
21. `a_probe_runs_for_a_tcp_service_whose_phase_is_unknown` — the §0.5 narrowing
    that the first draft would have shipped silently.
22. `a_probe_is_not_run_for_an_instance_that_is_not_running` — one fault, one
    alert.
23. `a_probe_result_is_cached_within_the_minimum_interval` — two back-to-back
    queries, one probe executed.

**Unit — `crates/app_orchestration/src/alerts.rs`**
24. `raising_the_same_alert_twice_refreshes_one_row_rather_than_opening_two`.
25. `clearing_an_alert_that_was_never_raised_is_a_no_op`.
26. `an_alert_raised_again_after_clearing_is_a_new_incident`.
27. `active_excludes_cleared_ones_and_all_includes_them`.

**Unit — `crates/sdk/src/health.rs`** (against a fake `StatusQuery`)
28. `an_unreachable_substrate_marks_its_services_unreachable_and_not_not_running`
    — the D-A4-13 distinctness claim.
29. `a_failing_probe_on_an_unknown_phase_reports_probe_failing` — a `tcp`
    service; the signal `task.md` requires and §0.5 rescued.
30. `a_stopped_instance_reports_instance_not_running_and_does_not_report_probe_failing`.
31. `an_undetermined_service_is_not_a_fault` (D-A4-19) — `faults()` empty,
    `unknowns()` non-empty, `is_healthy()` false.
32. `a_service_with_no_completed_placement_reports_not_deployed`.
33. `record_report_clears_a_service_level_alert_when_the_signal_changes_kind`
    — not-running → probe-failing must not leave two active rows.
34. `record_report_raises_one_substrate_alert_not_one_per_service`.
35. `record_report_does_not_raise_near_expiry_for_a_freshly_issued_certificate`
    (D-A4-16).

**Unit — `apps/roymctl`**
36. `test_app_health_command_parsing`, `test_app_alerts_command_parsing`,
    `health_help_lists_no_record_watch_and_strict`.
37. `deployed_service_id_prefers_the_local_member_master_and_falls_back_to_the_plan_id`
    (in `member_identity.rs`) — and `check_no_placement_change`'s existing tests
    must still pass unchanged after the extraction, which is the regression
    guard for D-A4-11.

**E2E — `crates/substrate/tests/health_monitoring_e2e.rs`** (new; two `Node`s
copied from `multi_substrate_placement_e2e.rs`, its own non-overlapping port
block per that file's convention)

38. `both_services_report_healthy_and_each_node_reports_its_own_registry` —
    the A3 two-substrate app deployed, one sweep, both `Healthy`, and node
    facts naming each node's actual registry URL (the §0.6 close).
39. `a_stopped_substrate_is_reported_unreachable_while_the_other_stays_healthy`
    — node B shut down; exactly one `SubstrateUnreachable`, node A unaffected.
    The reference scenario's step 3.
40. `a_failing_readiness_probe_is_distinct_from_a_stopped_instance` — a TCP
    service with a `tcp-connect` probe whose listener is closed: phase stays
    `unknown`, probe reports `failing`, signal is `ProbeFailing`. Buildable
    only because of §0.5's fix.
41. `alerts_are_recorded_deduplicated_and_cleared_across_three_sweeps` — sweep
    with the fault (row opened), sweep again (same row, `last_seen_at` moved),
    fix and sweep (row cleared) — read back through `AlertStore::active`/`all`.

**Not built, deliberately:** a `roymctl app health` CLI-level e2e. `roymctl`
is bin-only, the same blocker A3's open backlog row already records; tests
36-37 cover the wiring at unit level and 38-41 cover the behavior over two real
nodes, which is A3's own precedent for `apply_plan`.

**Failure/security matrix:** A4 adds no rows. Rows 11/13 name the operator read
surface and remediation, both A5's. Row 3's visibility half gains a second
piece of evidence (tests 7-8, 35 + D-A4-16) which should be appended to that
row at sign-off.

---

## §10 — Merge order

| # | Content | Independently mergeable? |
|---|---|---|
| 1 | §2 the declaration (models, WIT, mapper) **plus** §8.1/§8.2's mechanical literal edits | **Yes.** Pure schema addition; nothing reads the field yet. Keep the 70-site churn in its own commit |
| 2 | §5.1 the near-expiry predicate moved into `syneroym-identity` | **Yes.** Pure refactor with an existing regression test |
| 3 | §3 the deploy-facts row **plus** D-A4-17's `readyz` repair | **Yes**, and valuable alone: it fixes a live bug (`podman inspect` against TCP services) independently of anything else in A4 |
| 4 | §5.2/§5.3 the `AlertStore` | **Yes.** Self-contained; no callers |
| 5 | §4 the status query | **Yes**, once 1 and 3 land |
| 6 | §6 the sweep | **Yes**, once 5 lands |
| 7 | §7.1-7.2 `roymctl app health` / `app alerts` | **No** — lands with 6 |
| 8 | §7.3 A3's two backlog rows (D-A4-15) | **Yes**, once 5 lands. Separable and droppable |

E2E tests 38-41 land with 6-7 or immediately after.

---

## §11 — Docs and backlog

**Docs**
- `docs/developer-guide.md` — a health-check manifest example per probe kind;
  a `roymctl app health` / `app alerts` walkthrough next to the multi-substrate
  deploy section; the `orchestrator/status` grant a monitoring-only credential
  needs, and that a **node-wide** one is required to read node facts; and, if
  §7.3 ships, the new split-registry preflight refusal.
- `task.md` — dated corrections for §0.1-§0.13; A4's row flipped at sign-off.
- `status.md` — an A4 section with evidence, matching A0/A1/A2/A3/P0's shape.
- ADR-0021 — no amendment needed. §7/§8 are consistent with what A4 builds;
  A4 only defers the MQTT half of `task.md`'s alerting non-goal, which is a
  milestone-text correction, not an ADR one.

**Backlog rows to add**
- *Alerts are not published to MQTT* (D-A4-10, §0.2) — A4's poller is a client
  process with no in-process broker → **A5**.
- *The alert store is a local file A5 cannot read* (§0.12) — A5 reuses
  `AlertStore`'s schema, types, and folding logic against the supervisor's own
  database; the A4-era `alerts.db` beside `deployments.db` is not migrated →
  **A5**.
- *`deployed_service_id` assumes member index 0* (D-A4-11) — inherited from
  `check_no_placement_change` and `substitute_and_certify_members`, correct
  while nothing in a manifest can express more than one member. The sweep then
  watches member 0 only, and the reference scenario's step 5 (scale to two)
  breaks it → **A5**.
- *`SubstrateEndpoint` cannot distinguish a container from a TCP service*
  (§0.5) — A4 works around it with a recorded type rather than fixing the
  variant. `PodmanSocket` is never registered by orchestration, so
  `list_impl`'s `"podman"` arm and `DeployedService.endpoint_type`'s container
  case are both unreachable → **TBD**.
- *`roymctl svc start` / `svc stop` call orchestrator methods that do not
  exist* (§0.9) — `MethodNotFound` on both; predates A4 → **TBD**.
- *`--no-default-features` does not build `syneroym-control-plane`* (§0.10) —
  the dummy sandbox lacks most of the methods `orchestration.rs` calls
  unconditionally; the gates run `--all-features` so it is never exercised →
  **TBD**.
- *`svc deploy` cannot declare a health check* (§2.7) → **TBD**.
- *A wasm `rpc` probe costs a component instantiation* — bounded by the
  5-second cache (D-A4-8), but a large fleet on one node still pays it. If the
  poll-cost budget is missed, the answer is a cheaper wasm liveness signal, not
  a longer cache → **A5**, alongside the budget measurement.
- *`status` reports no binding state* — A5's read surface has to show
  per-dependent binding convergence (`task.md` exit criteria), and the epoch is
  already in the substrate's registry. Left out of A4 to keep the wire record
  small; adding a field to `service-status` later touches only
  `orchestration.rs` and its tests → **A5**.

**Backlog rows to resolve** (move to *Recently resolved*, only if §7.3 ships)
- *A3: substrate capabilities are operator-declared, not probed* — now
  reported by `status`, compared at preflight, warns on a mismatch. Note in the
  row that the check is skipped, with an explicit message, for a credential
  without node-wide `orchestrator/status` (D-A4-18).
- *A3: multi-substrate placement requires one registry namespace, verified
  only after the fact* — now a preflight refusal where the credential allows;
  `probe_registry_reachability` retained as the propagation check and the
  fallback.

If §7.3 is dropped, both rows stay open and are retargeted to **A5**.

**Backlog rows to update**
- *A3: `app::handle`'s own CLI-level orchestration has no test coverage* — its
  target says "A4 (or a dedicated follow-up before it)". A4 adds two more
  commands to the same untested `handle`, so the row grows rather than closes.
  Retarget to **A5** and note that A4 mitigated it the same way A3 did.
- *Stable member identity: unattended certificate renewal* — A4 adds the
  `CertificateNearExpiry` alert, so the attended posture's visibility half now
  has a third piece of evidence. The row stays open: visibility is not renewal.

---

## §12 — Questions for the requester

1. **§0.1 / D-A4-1, D-A4-2.** Confirm the sweep belongs in `crates/sdk` and is
   one-shot, with the substrate role and the resident loop staying in A5.
   Recommended as written.
2. **§0.2 / D-A4-10.** Confirm MQTT alert publication moves to A5. `task.md`'s
   non-goals text assumes an in-process broker, which is true of a substrate
   and not of A4's poller.
3. **§0.5 / D-A4-7.** Two parts. (a) Confirm a `tcp` service reports `unknown`
   rather than `running` — honest, and no longer noisy at the exit code, since
   D-A4-19 makes undetermined non-fatal. (b) Confirm D-A4-17: A4 repairs
   `readyz`'s existing container guess as part of recording the service type,
   rather than leaving two contradictory answers on one substrate.
4. **§0.6 / D-A4-15 / §7.3.** Build the two A3 backlog closes in A4, or leave
   them for A5? Cheap once `status` exists, but outside `task.md`'s A4
   sentence. Recommended: build them, as their own separable phase.
5. **§0.11 / D-A4-12.** Confirm `app health` records alerts by default.
6. **§0.12 / D-A4-9.** Confirm the standalone `AlertStore` over (a) no
   persistence in A4 at all — print and exit, leaving the only store to A5, or
   (b) tables on `DeploymentJournal`, which A5 provably cannot read.
   Recommended as written: A5 reuses the schema and the folding logic even
   though it cannot reuse the file.
7. **Probe kinds.** Three (`tcp-connect`, `http-get`, `rpc`) — is `rpc` wanted
   in A4 at all? It is the only probe a wasm service can have, and without it
   every wasm service is liveness-only; it is also the only one that costs a
   component instantiation per probe.

---

## §13 — What this hands A5

- **`sdk::health::poll_once` and `record_report`** — the reconcile loop calls
  both rather than growing a second poller. A5 changes *when* they run and
  *what happens next*, not *how a signal is read*.
- **Three distinct signals, already separated at the source.** A5's bounded
  remediation branches on `Signal`: restart-in-place for
  `InstanceNotRunning`, alert-and-wait for `SubstrateUnreachable` (nothing to
  restart on a node you cannot reach), and a policy choice for `ProbeFailing`.
  That branch is why `task.md` insisted the signals stay distinct.
- **`AlertStore` — the schema, types, and lifecycle, not the file.** A5 opens
  the same type on the supervisor's own database and serves those rows over
  the `supervisor` interface; the terminal-`Degraded`-after-max-attempts state
  (failure-matrix row 13) is one more `AlertKind` on the same table. What A5
  must **not** inherit is A4's local `alerts.db`, and §11 records that.
- **A recorded per-service type**, which A5 needs anyway: restart-in-place is a
  different operation for a container than for a wasm component, and until this
  slice nothing on the substrate could tell them apart.
- **`node-facts`** — A5's adopt/manage decisions can check that a substrate can
  actually run what it is about to be handed, before handing it.
- **A probe A5 can reuse as its own credential-free liveness check.** ADR-0021
  §7's passive attended-posture signal for a bound external dependency is
  structurally the same call `http-get`/`tcp-connect` already make; the active
  online-key probe is the one that needs the member master A5 introduces.

**One tension A5 must resolve, not discover:** A4's sweep reads health as the
*operator*, using whatever credential the inventory holds per substrate. A5's
supervisor reads it as *itself*, a distinct principal that must be granted
`orchestrator/status` — and, to read node facts at all, the **node-wide** form
(D-A4-18). Nothing in A4 issues that grant, and A3's inventory holds grants
rather than issuing them (A3's own retargeted backlog row). A supervisor
without it polls half-blind: services it owns report normally, node facts come
back empty, and A4 at least reports that honestly rather than as healthy.
