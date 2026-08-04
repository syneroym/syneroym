# Syneroym Developer Guide

Welcome to the Syneroym developer guide! This document consolidates all essential development procedures, repository layouts, testing workflows, and API interaction examples into a single, unified reference.

---

## 1. Development Workflows & Setup

### Prerequisites

We recommend using [mise](https://mise.jdx.dev/) to automatically manage project tool versions (Rust stable, wasm-tools, Node.js 20, etc.).

```bash
# Automatically install tools configured in mise.toml
mise install
```

Alternatively, you can manually install:
- **Rust**: The latest stable compiler via [rustup](https://rustup.rs/)
- **Node.js**: Version 20+ via [nodejs.org](https://nodejs.org/)

### Building the Project

Ensure all dependencies are installed and build the Rust workspace:

```bash
# Install frontend / E2E dependencies
pnpm install

# Build all workspace crates and applications
cargo build
```

### Formatting the Project

To ensure all files adhere to the project's strict idiomatic guidelines—including grouping and separating imports by module, crate, and external libraries—use the nightly formatting command. Note that standard stable `cargo fmt` will ignore these unstable features, which can lead to disjointed newlines.

```bash
# Aggressively format all Rust code, merging and strictly grouping imports
cargo +nightly fmt --all
```

### Run Commands

To run local CLI and substrate nodes:

```bash
# Show CLI options for roymctl
cargo run --bin roymctl -- --help
```

---

## 2. Testing & Benchmarking Reference

Syneroym has a tiered testing and benchmarking strategy across three different suites:

### Automated Performance Summary
You can run all benchmarking suites (Criterion, Latency, Concurrency, and Soak) sequentially and automatically append a summarized report—including your machine hardware specifications—to `PERF_SUMMARY.md` via a single xtask command:

* **Run via Cargo:**
  ```bash
  cargo xtask perf-summary
  ```

### Suite 1: Rust Unit & Integration Tests (SUT)
These verify code correctness in isolation. Unit tests cover individual helper modules, and integration tests cover complex multi-system flows (e.g. substrate lifecycles).

* **Run via Mise (Recommended):**
  ```bash
  mise run test:rust
  ```
* **Run via Cargo:**
  ```bash
  cargo test --workspace
  ```

### Suite 2: Playwright WebRTC End-to-End Tests
These verify fully integrated WebRTC signaling and client gateway browser scenarios.

* **Run via Mise (Recommended):**
  ```bash
  mise run test:e2e
  ```
* **Run Interactive UI Mode:**
  ```bash
  mise run test:e2e-ui
  ```
* **Run via npm:**
  ```bash
  cd crates/substrate/tests/e2e
  npm install
  npm test
  ```

* **Run Everything Together**
To execute all Rust and E2E suites sequentially:
```bash
mise run test:all
```

### Suite 3: Criterion Micro-Benchmarks
These capture performance baselines for hotpaths under CPU stress in isolation, including preamble parsing, crypto (ECDH and AES-GCM), length-prefixed framing, and WASM sandbox store creation and instantiation.

* **Run via Mise (Recommended):**
  ```bash
  mise run bench:micro
  ```
* **Run via Cargo:**
  ```bash
  cargo bench --workspace
  ```

### Suite 4: Latency Overhead Tests
These tests spin up a local substrate process, load applications onto it, and benchmark the latency difference between direct execution and execution routed via the substrate framework.

* **Run via Mise (Recommended):**
  ```bash
  mise run bench:latency
  ```
* **Run via Cargo:**
  ```bash
  cargo run -p syneroym-perf -- latency
  ```

### Suite 5: Concurrency & Resource Profiling Tests
These tests flooding the substrate under high-concurrency, sudden spike load, pool exhaustion, and long-term client connections to verify resource boundaries.

* **Run via Mise (Recommended):**
  ```bash
  mise run bench:concurrency
  ```
* **Run via Cargo:**
  ```bash
  cargo run -p syneroym-perf -- concurrency
  ```

### Suite 6: Soak / Endurance Tests
These run long-duration endurance scenarios to detect slow memory/FD/task/cache leaks under concurrent sustained workloads.

* **Run via Mise (Recommended):**
  ```bash
  mise run bench:soak
  ```
* **Run via Cargo:**
  ```bash
  cargo run -p syneroym-perf -- soak --duration 1800
  ```

> [!NOTE]
> For a full design, layout of the metrics, leak detection heuristics, and results gating, refer to the comprehensive [Performance & Robustness Testing Report](performance-and-robustness-spec.md).


---

## 3. Port Reference (Normalized 796x)

- **7960**: Client Gateway (HTTP Proxy)
- **7961**: Community Registry (HTTP)
- **7962**: WebRTC Bootstrap Page (HTTP)
- **7963**: WebRTC Signaling Server (WebSocket)
- **7964**: Iroh Coordinator (HTTP Signaling)
- **7965**: Iroh Coordinator (QUIC Data)

---

## 4. API & Interaction Examples

This section details how to interact with the local substrate gateway and registries using the CLI (`roymctl`) and standard tools like `curl`.

### Identifying your Substrate

To interact with services, you need your Substrate's **Short Hash**. You can compute it from your DID using this command:

```bash
roymctl shorthash "<DID>"
```

### Claiming a Substrate

A freshly initialized substrate is **unowned**, and an unowned substrate now
fails closed: no caller can deploy, undeploy, check status, or reach the
`security` interface (KEK injection/rotation, vault secrets) until someone
claims it. Claiming binds the node's own DID to a **controller** identity
you hold, with a mutually-signed `ControllerAgreement` — from then on, only
that controller (or a caller it delegates to) holds any node-wide
capability.

```bash
# 1. Initialize the node (writes <dir>/substrate.key)
roymctl substrate init --dir <DIR>

# 2. Create the identity that will become the controller
roymctl --dir <DIR> identity create --name owner

# 3. Claim the substrate -- must run on the substrate host, since it signs
#    with the node's own private key, which never leaves that filesystem
roymctl --dir <DIR> substrate claim --controller owner

# 4. Start (or restart) the substrate -- it reads the agreement once, at
#    boot. If it uses <DIR> as its app_data_dir (e.g. via a [identity].key
#    setting in a --config file that points there), <DIR>/agreement.json is
#    discovered automatically with no further flag. Otherwise, point it at
#    the files directly:
syneroym-substrate run --key <DIR>/substrate.key --agreement <DIR>/agreement.json
```

From then on, control it with `roymctl --dir <DIR> --as owner ...` (or
`--ucan <token>` for a narrower, delegated grant — see §5.1 below). There is
no remote claim: the tool needs the node's own key file, so provisioning a
fleet of substrates means running `claim` on each host (or shipping
`agreement.json` out of band).

### Managing Identities

Before registering a service, you need to create a local identity (private key) that will be used to sign the registration.

```bash
# Create a new identity named 'my-service'
roymctl identity create --name my-service
```

### Registering a Service in the Community Registry

Once you have an identity, you can register it against a substrate DID. This links your service DID to the substrate that hosts it.

```bash
# Register 'my-service' against a substrate DID with an optional nickname
roymctl registry register \
  --identity my-service \
  --substrate "did:key:h..." \
  --nickname "alice"
```

You can verify the registration using the lookup command:

```bash
# Look up by DID or alias (nickname + shorthash)
roymctl registry lookup "alice-p<SERVICE_DID_SHORTHASH>"
```

### Discovering Services

Lookup a specific service by its DID:
```bash
# Returns signed endpoint info
curl http://localhost:7961/lookup/did:key:z6MkhaXn...
```

### Managing Applications (Orchestrator)

The Orchestrator is a native service running inside the substrate. You can interact with it via the Client Gateway (Port 7960).

> **The `curl`-via-gateway examples below need a claimed substrate, and even
> then are denied for anything but `list` today.** The client gateway
> presents the *node's own* DID as caller, never the controller's (a
> standing gap, see the deferred backlog's *Gateway caller = substrate-owner
> DID threading* row) — so `deploy`/`undeploy`/`status` are always denied
> through the gateway on a claimed substrate, and everything is denied on an
> unowned one. `curl` also cannot present a signed operator identity at all.
> For a real deploy, use `roymctl` directly against the substrate instead,
> e.g. `roymctl --dir <DIR> --as owner svc deploy --svc-id <DID> --interfaces
> <name> --tcp <host:port>` (see `roymctl svc deploy --help` for the WASM
> and container forms) — only `roymctl` can sign as the claimed substrate's
> controller.

#### List Deployed Services
```bash
# Replace <NICKNAME> and <SUBSTRATE_DID_SHORTHASH>
curl -X POST http://localhost:7960/ \
  -H "Host: <NICKNAME>-p<SUBSTRATE_DID_SHORTHASH>-iorchestrator.localhost" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "list",
    "params": {},
    "id": 1
  }'
```

#### Deploy a WASM Component
Only `roymctl` can sign as the claimed substrate's controller (see the note
above) -- the client gateway has no way to present that identity, so this is
not a `curl` example:
```bash
roymctl --dir <DIR> --as owner svc deploy \
  --svc-id did:key:my-app-did \
  --interfaces my-interface:v1 \
  --wasm ./app.wasm
```

##### Declaring an FDAE Policy (Row/Column-Level Security)

Any deployed service (WASM, TCP, or container) may declare a `config.fdae`
block naming a declarative ReBAC policy document, validated at deploy:
```toml
[services.my-svc.fdae]
policy = "fdae-policy.json"
```
A bare path is read **client-side** and the document travels inside the
deploy call — the same treatment a bare `source` already gets for a Wasm
component. That is what lets a deploy against a remote substrate work
with nothing pre-staged on it.

Like `source`, a relative path resolves against **your shell's working
directory**, not the manifest's location — so run `roymctl` from the
directory the paths are written relative to (usually the app's root).

To point at a document the substrate already holds instead — a large or
shared asset, or an operator-managed policy directory — say so explicitly
under the same key:
```toml
[services.my-svc.fdae]
policy = { remote_path = "/etc/syneroym/policies/guild.json" }
```
That path is resolved on the substrate's side, relative to its working
directory, under a path-traversal guard. `config.schema` (the JSON Schema
validating `custom_config`) takes exactly the same two forms.

In a raw `deploy` JSON-RPC call's `config`, the two arms are tagged:
```json
"config": { "env": [], "args": [], "custom_config": null,
            "fdae_policy": { "inline": "{\"version\":\"fdae/v1\", ...}" } }
```
```json
"config": { "env": [], "args": [], "custom_config": null,
            "fdae_policy": { "path": "/etc/syneroym/policies/guild.json" } }
```
Either way the document itself
must be **JSON** (ADR-0017's own examples are YAML for readability only; the
compiler is `serde_json::from_str`). A malformed or schema-invalid policy is
a hard deploy failure, so an author finds out at deploy time, not the first
time a caller is unexpectedly denied. A service with **no** `fdae` block is
unfiltered — every row and column reachable exactly as before FDAE existed
(ADR-0017 §2.1's default-absent). See
[ADR-0017](decisions/0017-fdae-policy-schema-and-compilation.md) for the
policy schema itself.

#### Calling Another Service from a WASM Component (Universal Proxy)
A deployed WASM component reaches another service — local or on another node —
through the `syneroym:proxy/proxy` WIT import (M04A Slice A1), without knowing
where the target actually lives:
```wit
import syneroym:proxy/proxy@0.1.0;
// call(service, interface, method, params, options) -> result<string, proxy-error>
```
`service` is the target's DID (or a registry alias), `interface` is a WIT
interface name the target registered at deploy time, and `params`/the success
value are JSON text — the callee binds them against its real WIT signature, so
the call is typed at the dispatch boundary even though the wire is JSON-RPC.
Set `options.idempotent = true` only for calls safe to retry on transport
failure; a callee-returned error is never retried. A component cannot use this
import to reach *another* service's native capabilities (`data-layer`, `vault`,
`app-config`, `blob-store`, `messaging`) — that's refused with
`permission-denied` — only its own, via its regular host imports.

#### Deploy a TCP Service (Passthrough)
```bash
roymctl --dir <DIR> --as owner svc deploy \
  --svc-id did:key:my-tcp-service \
  --interfaces default \
  --tcp localhost:8080
```

#### Deploy a Container Service (Podman)
```bash
roymctl --dir <DIR> --as owner svc deploy \
  --svc-id did:key:my-container-service \
  --interfaces default \
  --image docker.io/library/nginx:alpine \
  --port default:80:8080 \
  --volume html:/usr/share/nginx/html
```
`--port` is repeatable: `interface:container_port[:host_port][:protocol]` —
leaving `host_port` empty (e.g. `default:80`) lets Podman pick one
dynamically. `protocol` is `tcp` (default) or `udp`, but only `tcp` mappings
are reachable through the substrate's own routing today; a `udp` one is
still published by Podman on the host, it just isn't reachable through
`roymctl`/the client gateway. Each interface name must also appear in
`--interfaces`.
`--volume` is repeatable: `host_path:container_path` — Docker-style mount
options like a trailing `:ro` are not supported. In-volume file
materialization (see below) has no CLI flag yet; use a `SynApp` manifest's
`files` list instead (below).

##### Mounting Configuration Files into a Container

Many off-the-shelf images read their configuration from a file rather than
from environment variables (which is all `custom_config` gives them — it is
flattened into `-e KEY=VALUE`). Such a service is deployed by giving the
volume a `files` list; the substrate writes each entry into the volume
before the container starts:

```json
"volumes": [
  {
    "host_path": "conf",
    "container_path": "/etc/nginx/conf.d",
    "files": [
      {
        "relative_path": "default.conf",
        "content": { "inline": "server { listen 80; }" }
      }
    ]
  }
]
```

`content` takes the same two arms as `fdae_policy` above — `inline` for a
document carried in the deploy call, `path` for one the substrate already
holds. In a `SynApp` manifest the client resolves a bare path for you:

```toml
files = [ { relative_path = "default.conf", content = "./nginx.conf" } ]
```

Three behaviors worth knowing:
- A volume with a non-empty `files` list is mounted **read-only** — it is
  configuration the substrate owns, not scratch space.
- Such a volume is materialized fresh on every deploy, so a file dropped
  from the manifest disappears from the mount.
- A volume with an empty `files` list is left alone entirely — an empty,
  writable directory on first deploy, and untouched on later ones, so a
  container's own data survives a redeploy. The one edge this leaves: if a
  volume *had* files and a later deploy drops the list, the old files stay
  on disk and the mount reverts to writable. Use a different `host_path`,
  or undeploy first, when converting a config volume back to scratch.

#### Developing Podman Services Locally
When developing a Podman container service for Syneroym:
1. **Rootless:** Ensure the container can run rootless. Syneroym uses Podman in rootless mode by default.
2. **Build:** Build your image locally (`podman build -t my-app:latest .`).
3. **Reference:** During the orchestrator `deploy` call, reference `localhost/my-app:latest` or `docker.io/library/nginx:alpine` in the `image` field.
4. **Debug:** Use standard tools (`podman ps`, `podman logs <container-id>`) on your host to inspect the container if it fails to bind or start via the orchestrator.

#### Deploying a Multi-Substrate App (`roymctl app deploy`)

A `SynApp` manifest's services can each declare which substrate they run
on, so one app can span more than one operator-controlled node (M05A Slice
A3). A manifest with no `[placement]` at all keeps working exactly as
before — every service goes to the substrate the deploy was aimed at.

Placement names an **alias**, never a bare DID — the alias is what lets the
same manifest deploy against different operators' fleets. The alias is
resolved against a **substrate inventory** file, `<roymctl --dir>/substrates.toml`
by default (override with `--inventory`):

```toml
# substrates.toml
#
# Precondition: every substrate listed here must publish and resolve
# endpoint records through the SAME registry namespace -- one shared HTTP
# registry, or BEP0044 DHT enabled on all of them. A substrate publishes
# through its own configured registry, not through the api_url below, and
# nothing on the wire reports which one that is, so roymctl cannot check
# this before deploying -- it only warns afterward if a member doesn't
# resolve.
[substrates.edge-1]
did = "did:key:z6MkExampleNodeA"
# How roymctl reaches this substrate; overrides the global --api-url.
# NOT the registry this substrate publishes into, and unrelated to
# --registry-url (below), which is where member master anchors go.
api_url = "http://localhost:7961"
# Local identity to act as against this substrate; overrides --as.
identity = "operator"
# Signed CapabilityToken JSON (see `identity issue-grant`); overrides
# --ucan. Requires `identity` above. The grant needs orchestrator/deploy,
# orchestrator/undeploy, AND orchestrator/status together: a failed
# deploy's own rollback path calls undeploy with the same caller, and
# --mint-masters calls resolve-instance-identity (status-gated) before
# every deploy.
ucan = "grants/edge-1.json"
# Optional. Service types this substrate can run. Absent = unconstrained
# (nothing reports this over the wire, so it is operator-declared).
capabilities = ["wasm", "tcp"]

[substrates.edge-2]
did = "did:key:z6MkExampleNodeB"
identity = "operator"
capabilities = ["wasm", "container"]
```

A manifest names these aliases with `[placement]`, at the manifest level
(the default) or per service (the override):

```toml
id = "syneroym:guild-app"
version = "0.1.0"

[placement]
substrate = "edge-1"

[services.frontend]
service_type = "wasm"
source = "frontend.wasm"

[services.backend]
service_type = "wasm"
source = "backend.wasm"

[services.backend.placement]
substrate = "edge-2"
```

Deploying resolves every alias, connects to each substrate (failing clean,
before any deploy call, if one is unreachable or the inventory doesn't
define it), and applies one deploy call per (service, substrate):

```bash
roymctl --dir <DIR> --as owner app deploy guild-instance-1 guild-app.toml \
  --mint-masters --registry-url http://localhost:7961 \
  --inventory substrates.toml
```

What's different from a single-substrate deploy:
- **Partial failure does not roll back.** If some services deploy and
  others fail (a substrate goes down mid-run, say), the app is left
  `DEGRADED` and the command exits non-zero, naming exactly which services
  failed and where. Re-running the same command **resumes** — it skips
  whatever already landed and only retries the failures.
- **Moving a service to a different substrate is refused**, not silently
  relocated: undeploy it from the old substrate first (the error names both
  substrates and the service id to remove), *then* clear the placement
  record with `roymctl app forget <instance> --service <name>` — the error
  names this step too. `svc remove` only stops the instance; it has no
  concept of an app instance or a journal, so it can't clear the
  bookkeeping the refusal reads, and skipping `app forget` leaves the next
  deploy hitting the identical refusal. Once both steps are done, redeploy.
  Otherwise the old instance would keep running and keep republishing its
  endpoint record — a live conflict with the new one.
- **A fully-placed app needs no `--substrate`/`substrate.key`** — the
  default substrate is only touched by a service with no placement.
- **Every substrate in the fleet must share one registry namespace** (or
  all run the BEP0044 DHT). This can't be checked before deploying — a
  substrate publishes only through its own configured registry, which
  nothing on the wire reports — so after a multi-substrate deploy,
  `roymctl` probes every member through every `api_url` it was given and
  **warns** (does not fail) if one doesn't resolve there. Fix by pointing
  every substrate's `[substrate] registry_url` at the same registry.
- **Deleting `deployments.db` may be needed after upgrading**: the journal
  schema changed in place (pre-release, no migration ladder) — an older
  database's `deployment_actions` table is missing the columns this slice
  added.

#### Operating an App with a Supervisor (`roymctl supervisor`)

The **App Supervisor** (M05A Slices A5b/A5c,
[ADR-0021](decisions/0021-binding-propagation-and-app-supervisor.md) §8) is a
substrate role that holds desired state for the app instances it manages and
**reconciles it on a resident loop**, on top of answering direct RPC calls
over its own `supervisor` interface. Two postures exist for who deploys and
renews certificates: manual (`roymctl app deploy`, described above) or
supervised (`roymctl supervisor …`, below). Pick one per app instance; a
supervisor's `submit` and a plain `app deploy` are not meant to alternate
against the same instance.

##### Enabling the role

```toml
# substrate.toml
[roles.supervisor]
poll_interval_secs = 30       # the resident loop's tick period
db_name = "supervisor.db"     # desired state, journal, alerts, and remediation -- one file
max_restart_attempts = 3      # bounded restart-in-place ceiling per service (see "Remediation" below)
restart_backoff_secs = 30     # minimum wait between two restart attempts for one service --
                               # equal to poll_interval_secs by default, so at defaults a
                               # service is restarted at most once per pass, deliberately
alert_topic = "supervisor/alerts"  # MQTT topic prefix; see "Alerts over MQTT" below
master_backup_dir = "master-backups"  # relative to app_data_dir; see below
renewed_cert_expires_hours = 4        # lifetime of EVERY instance certificate this supervisor
                                      # mints -- the first one and every renewal; see below
max_renewals_per_pass = 5             # ceiling on renewals attempted in one pass
master_anchor_refresh_interval_secs = 43200   # 12h; anchors stop verifying after 24h
```

##### The resident loop

Every `poll_interval_secs`, the loop sweeps every non-paused, non-retired
instance: a health poll, then a **filtered** redeploy of only the services
that changed since the last fully-landed plan or that never landed at all
(never a full redeploy of everything on every pass), then one bounded
restart attempt for any landed service the sweep found not running. A pass
that outruns the interval drops the tick it overran rather than queuing a
burst.

**Remediation is restart-in-place only, and only for a landed service found
not running.** A declared readiness probe failing (`ProbeFailing`) is
alert-only — there is no way today to tell "still starting" from "genuinely
broken" (`HealthCheck` has no initial-delay or failure-threshold field), so
restarting on it would burn the attempt budget on a service that was never
actually broken. A substrate that does not answer at all
(`SubstrateUnreachable`) never triggers a restart either — restarting cannot
fix a substrate the loop cannot reach. After `max_restart_attempts`, a
service goes **terminal**: the loop stops restarting it, an alert
(`RemediationExhausted`) names the way out, and only `supervisor
force-reconcile` or `supervisor adopt` clears the terminal flag (both are a
fresh start by construction) — an out-of-band recovery (an operator
restarting the container themselves) still clears it the ordinary way too,
through the next healthy sweep.

**A resubmit that would move an already-landed service to a different
substrate is refused, permanently — the loop never retries it.**
Relocating a running member safely needs an `undeploy` on the old substrate,
an ordering rule between that and the new deploy, and durable retry across
both — none of which exist yet, so the supervisor refuses rather than risk
two live copies of one member. The manual path today: `svc remove` on the
old substrate, `roymctl app forget` to clear the stale placement record,
then resubmit.

**A service the resubmitted plan no longer names is never undeployed.** The
loop raises `OrphanedService` and leaves it running — undeploying a stateful
service because a manifest was edited is destructive, and `retire` is
deliberately not a teardown either. Remove it by hand (`svc remove`) if
that is what you actually want.

##### Unattended certificate renewal

The same pass also **renews instance certificates**. Each pass, any member
whose installed certificate is inside the last 25% of its lifetime is
reissued: the supervisor mints a fresh certificate from that member's master
in its own vault, installs it with the `renew-cert` verb — a
certificate-only write, no reinstall, no artifact over the wire — and then,
only if that member's `rotation_policy` is `restart-on-rotation`, restarts
it. A failure at any step skips the remaining steps for that member and is
retried on the next pass; it never fails the rest of the instance.

**A restart-on-rotation failure gets its own alert.** If the mint and
install both land but the restart itself fails, the certificate is not
stalled — the health poll sees a fresh window on the very next pass, which
would otherwise clear a `CertificateNearExpiry`/`CertificateExpired` alert
right out from under the real problem. This case raises
`RotationRestartPending` instead, backed by a persisted marker independent
of the certificate window, and it is retried every pass (regardless of
whether the member is due for renewal again) until the restart actually
succeeds.

At most `max_renewals_per_pass` members are renewed in one pass. Renewal is
the one thing the loop does whose work arrives all at once by construction:
every member of an instance is minted in the same call at the same lifetime,
so they all reach their near-expiry window in the same pass, every cycle.
The remainder simply rolls to the next pass — the near-expiry window is
hours wide against a 30-second tick, so there is a lot of room.

**Two different certificate lifetimes, for two different purposes.**
`renewed_cert_expires_hours` (4 hours) is what the *supervisor* mints, for
its first certificate and every renewal alike, so a managed member has one
lifetime for its whole life. Short is affordable precisely because renewal
is automated, and it bounds what a leaked instance key is worth. Separately,
every substrate enforces a **30-day ceiling** on any instance certificate
offered to it, on `deploy` and `renew-cert` alike. That is a backstop
against an unbounded mint, not a policy: `roymctl svc deploy
--expires-hours`'s own attended-posture default (24h), and any reasonable
manual cadence, are nowhere near it.

> **The restart trade, stated plainly.** The supervisor's vault is locked
> after every restart — the KEK arrives by `security inject-kek` and does not
> survive one — and nothing renews while it is locked. With
> `renewed_cert_expires_hours = 4`, the window to re-inject the KEK before
> managed members start failing handshakes closed is **between roughly 1 and
> 4 hours**, not the 24 it used to be: a member expires 4 hours after *its
> own last renewal*, not 4 hours after the restart, so a restart landing at
> the start of a member's near-expiry window leaves about an hour. The
> `VaultLocked` alert fires on the first pass that finds a member due for
> renewal with the vault shut, once per affected member, and its text names
> `inject-kek`. Treat it as page-worthy, not informational.

##### Master anchors are refreshed on the same tick

A master anchor stops verifying at every consumer 24 hours after it was
signed, and every certificate that master issues is unusable on the wire
without one. The loop republishes each managed master's anchor when
`master_anchor_refresh_interval_secs` (12 hours, 2× margin) has elapsed
since the last successful publication — evaluated on the ordinary pass tick
against a persisted fact, not on a timer of its own. It needs
`substrate.registry_url` configured on the supervisor's node; without one
the supervisor holds no anchor writer at all, since an anchor published
nowhere is worse than none.

The read-modify-write this performs has no compare-and-set behind it. It
does not need one under the topology this tree supports: mint-in-place means
exactly one vault ever holds a given master, and `export-master`/
`import-master` move a *file*, not concurrent live access — so there is
structurally one writer. Running two supervisors over one imported master
would break that assumption; it is not supported today.

##### Revoking one member's instance key

```bash
roymctl --substrate <supervisor-node-did> supervisor revoke-instance \
  guild-instance-1 guild-instance-1/frontend
```

Two writes in one action, because either alone is not a revocation. First
the member master's anchor gains that member's derived instance DID in its
`revoked_keys` list, so every handshake presenting that key fails from then
on. Second, the placement is recorded revoked locally, so **nothing
re-certifies it on any write path — the resident loop's renewal, `submit`,
and `force-reconcile` alike**. Without the second half, the very next
ordinary resubmit would silently mint a fresh certificate and reinstate the
key you just revoked.

Scoped to one member, not the whole instance: everything else keeps
reconciling, and an `InstanceRevoked` alert names the skip on every pass
that would otherwise have touched it. Deliberately not a teardown either —
the member's process keeps running until you issue `svc remove` (or
`supervisor retire`) separately.

There is no un-revoke verb. Bringing a revoked member back under management
needs a decided semantics for removing a DID from an anchor's revoked list,
which this milestone does not settle.

##### Custody: the supervisor mints its own member masters

Unlike `roymctl app deploy --mint-masters`, which mints into files under the
*operator's* `--dir`, the supervisor mints each member master **directly
into its own encrypted vault** and never receives one over the wire — there
is no `--upload-masters` flag and no unmastered submit path. This is why:
no client in this tree can produce the end-to-end encrypted transport
(`?enc=`) a key would need to cross the wire safely, so the design keeps the
key off the wire entirely rather than sending it plaintext. See
[ADR-0020](decisions/0020-stable-logical-service-identity.md)'s 2026-08-01
amendment for the full reasoning.

**The vault is locked until you inject a KEK, and the KEK does not survive a
restart.** Boot order matters:

```bash
# 1. Boot the substrate (the supervisor role starts, but its vault is locked --
#    a startup warning names the fix).
# 2. Inject the KEK -- required before the FIRST submit, and again after
#    every restart:
roymctl --substrate <supervisor-node-did> security inject-kek --kek-hex <64-hex-chars>
# 3. Now `supervisor submit` can mint.
```

A member master minted this way is **not backed up automatically**. The
moment `submit` mints one, it prints the DID and the export command:

```bash
roymctl --substrate <supervisor-node-did> supervisor export-master member-guild-instance-1#frontend-0
# -> Wrote master 'member-guild-instance-1#frontend-0' to <supervisor's app_data_dir>/master-backups/member-guild-instance-1#frontend-0.key
```

> **The vault-key name changed shape in M05A Slice A5e.** Before A5e, a
> member master's computable name was `member-<app-instance-id>-<service-
> name>-<index>`; A5e changed the `app-instance-id`/`service-name` boundary
> to `#` (`member-<app-instance-id>#<service-name>-<index>`) to close a
> collision between two different (instance, service) pairs that could
> otherwise stem to the identical name. `get_or_mint` finds nothing under
> the new name for an instance deployed before this change, so its **next
> submit re-identifies every member with a fresh master DID** rather than
> failing — the old master, its anchor, and its `revoke-instance` rows are
> left behind, orphaned. Pre-release policy accepts this the same way it
> accepts every other in-place schema change (no migration, no compat
> shim); if you have a real deployment predating A5e, back up and
> re-`import-master` each member under its new name (or just accept the
> re-identification) before its next submit.

That path is on the **supervisor's own host** — collecting the file from
there (a mounted volume, a backup agent, `scp`) is the operator's own
arrangement, the same "operator duty this milestone documents rather than
automates" `app deploy`'s masters already are. `import-master` is the
reverse: place an A0-A4 deployment's `<dir>/identities/member-*.key` into
that same `master_backup_dir`, then ask the supervisor to adopt it by name.
Neither verb ever carries key bytes in its request or response — only a
name (in) or a path (out).

##### Custody: the app instance also has a master of its own (M05A A7)

Beside every member master, the vault also holds one key per **app
instance** — its own master DID, distinct from any member's and from the
supervisor node's own identity. It is minted (or, on every later `adopt`,
resolved) the moment you `adopt` the instance, under the computable name
`app-<app-instance-id>`:

```bash
roymctl --substrate <supervisor-node-did> supervisor adopt guild-instance-1
# -> Adopted 'guild-instance-1' at generation 1.
#   app master: did:key:... -- back it up with `roymctl supervisor export-master app-guild-instance-1`
```

`roymctl supervisor status` reports it too, once adopted:

```bash
roymctl --substrate <supervisor-node-did> supervisor status guild-instance-1
# -> { ..., "app_master_did": "did:key:...", ... }
```

**It is not backed up automatically, exactly like a member master.**
`export-master`/`import-master` move it by the same name `adopt` printed —
neither verb changed shape for this: they already took a bare name, and
`app-<app-instance-id>` is simply one more name they accept.

> **An instance adopted before this slice gets its app master at its
> *next* `adopt`, and nowhere else** — not on `submit`, `force-reconcile`,
> a loop pass, or `status`. Until then, `status` reports `app_master_did`
> absent, the same way `member_master_name`'s A5e boundary change reads on
> an instance that predates it: pre-release, no migration, and the visible
> cost is one generation bump, which is what `adopt` already means.

**Handover order: `import-master` before `adopt`.** Moving an app instance
to a new supervisor follows the same sequence as any other handover
(`submit`, `import-master`, `adopt`), but for the app master the order is
load-bearing. Running `adopt` *before* `import-master` mints a **second**
app identity under the same name — the generation fence does not catch
this, since it fences two writers over *one* record, and a wrong-order
adopt produces two records that never meet. Re-running `adopt` after the
correct `import-master` repairs it: `adopt` always resolves-and-records
from whatever the vault currently holds, so the row ends up agreeing with
the vault either way — just later than necessary if the order was wrong.

**What this DID does not do yet.** Nothing publishes it to a registry,
nothing resolves it to an address, and no caller outside this supervisor
can use it for anything — that is the Logical Service Discovery Overlay's
slice S1, outside this milestone. What lands here is the identity itself:
where it lives, how it moves, and that it survives a supervisor handover
unchanged.

##### The grant a supervisor needs on every substrate it manages

The supervisor's own credential against each managed substrate travels
inside `submit`'s `inventory-json` (an alias's resolved `CapabilityToken`,
not a local file path — meaningless once it crosses the wire). Nothing
issues this grant automatically; an operator mints it once per managed
substrate, audienced to the **supervisor node's own DID** (the identity it
presents when it connects out, per ADR-0021 §8 — it is a client of
substrates, not a server to services):

```bash
roymctl --dir <DIR> --as <managed-substrate-owner> identity issue-grant \
  --to did:key:<supervisor-node-did> \
  --resource "substrate:did:key:<managed-substrate-did>" \
  --can orchestrator/deploy --can orchestrator/status \
  --out grants/supervisor-on-edge-1.json
```

**Both abilities, node-wide, not app-scoped.** `claim-app-instance`/
`release-app-instance` need `orchestrator/deploy`; `resolve-instance-identity`
(certification, on the path of every deploy the supervisor performs) and
`app-instance-management-of` (`adopt`'s read half, before any claim exists
to match ownership against) need `orchestrator/status` instead — the two
abilities are deliberately flat, so holding one does not cover the other.
An app-scoped selector is not enough either: claiming or releasing an app
instance is a node-scoped act, because the instance spans services.

> **A node-wide-granted supervisor that is not a member's recorded owner
> re-keys it on the first renewal.** `renew-cert` verifies the submitted
> certificate against the *renewing caller's* own derived identity, the
> same rule `deploy` uses — so a supervisor admitted only through the
> node-wide `orchestrator/deploy` grant above, on a member it did not
> itself deploy (adopted but not yet redeployed under its own identity,
> say), installs a certificate for *its own* derived key on that member's
> very next unattended renewal. The previous owner's key is not revoked by
> this; it simply stops being current, silently. Grant node-wide
> `orchestrator/deploy` only to the supervisor that actually owns — or
> will immediately redeploy and thereby take ownership of — every member
> on that substrate.

##### Submitting, adopting, and reading status

```bash
# Compile a manifest and hand it to the supervisor as desired state. The
# supervisor resolves-or-mints its own masters and substitutes them; the
# plan this sends still carries the compiler's fabricated ids.
roymctl --substrate <supervisor-node-did> supervisor submit \
  guild-instance-1 guild-app.toml --inventory substrates.toml

# Claim management: reads the held generation from every substrate the
# instance is placed on and writes held + 1 (ADR-0021 §4). The only minter
# -- a supervisor never self-increments.
roymctl --substrate <supervisor-node-did> supervisor adopt guild-instance-1

# Health, per-service signals, and a delivery note (best-effort synchronous
# push, not a durability guarantee):
roymctl --substrate <supervisor-node-did> supervisor status guild-instance-1

# Active alerts (add --all for the full history including cleared ones):
roymctl --substrate <supervisor-node-did> supervisor alerts guild-instance-1
```

`pause`/`resume` mark an instance in the supervisor's own store without
touching anything on the substrates it manages — `pause` stops the resident
loop from touching this instance automatically, and nothing else; every
other verb stays allowed while paused.

> **Pausing stops renewal too, not just redeploys.** `all_active` (the
> resident loop's own work list) excludes a paused instance entirely, so a
> paused instance gets no health poll, no certificate renewal, and no
> master-anchor refresh — the same "and nothing else" above covers those.
> With `renewed_cert_expires_hours = 4`, every member of a paused instance
> fails its handshake closed within about 4 hours, and its master anchors
> stop verifying within 24 — both silently, since the alert passes that
> would otherwise name it never run either. This did not matter at A5c's
> 24-hour certificates, where a paused instance's operator had a full day
> to notice and reissue by hand; at A5d's default it is a short, real
> clock. Resume before it, or renew by hand with `roymctl identity
> certify-instance` if the pause needs to outlast it.

`release` hands an instance back to
manual operation by clearing the management stamp on every placed substrate
(does **not** undeploy); `retire` does the same and additionally makes the
supervisor's own desired-state row terminal, refusing a later `submit`
until the instance is adopted again. `force-reconcile` re-runs the
mint/certify/apply pipeline against whatever is currently stored — the same
work the resident loop itself does when the instance's plan changes since
the last landed one, run immediately rather than waiting for the next tick.

##### Scaling a service (`replicas`)

```toml
[services.backend]
service_type = "wasm"
source = "backend.wasm"
replicas = 2
```

`replicas` (default `1`) tells the compiler to emit N members of that
service instead of one, each with its own master DID minted through the
same custody path as an unscaled service. Above `1`, the compiled topology
mode becomes `Redundant`: an unkeyed call from another service round-robins
across the members, and a keyed call (a WASM guest passing a routing key)
uses rendezvous hashing so the same key keeps landing on the same member.
`Sharded` mode is not reachable from a manifest yet — it needs a
`ShardingStrategy` surface a later slice adds.

A member is a full peer for every other supervisor mechanic: its own
placement, its own certificate lifecycle, its own restart budget, and its
own row on `supervisor status` (`<app-instance-id>/<service-name>#<index>` —
the `#<index>` suffix is what distinguishes member 1 from member 0 on every
read surface, including the argument `supervisor revoke-instance` takes).
Scaling a service that other services depend on is a **binding push, not a
redeploy**: a resubmit that only changes which member DIDs a dependency
resolves to reaches every dependent member through the same
`write-bindings` path A5c built, with no reinstall and no artifact
re-sent — `roymctl supervisor status`'s `bindings` array shows written vs.
observed epoch converging per dependent member.

**What `replicas` does not do.** All N members of a scaled service still
share one placement (`ServiceSpec.placement` is a single selector), so a
scale-out does not survive losing the node it is placed on — that needs a
placement-selector design a later slice will add, tracked in the deferred
backlog. And **each member is its own `service_id` and therefore its own
database** (`syneroym-data-db` opens one SQLite file per `service_id`):
`replicas` is for **stateless members only**, until M7's state replication
lands. The compiler refuses `replicas > 1` on a service that also declares
a `schema` (the manifest's own marker for "this service uses the structured
data layer"), naming M7 as the reason the refusal will relax. It cannot see
a service that uses the data layer without declaring a `schema` — that
residual case is on the operator to avoid; declaring it whenever a service
does use structured data is what makes the refusal actually catch this.
Above the cap of 16 members, `replicas` is refused outright at manifest
validation, before anything is deployed.

**Convergence, measured.** Two different numbers answer "how long until a
scale-out is actually being called":

- A `submit`-driven membership change applies in-call: the clock starts
  when the RPC is received, and stops when every reachable dependent
  member's `write-bindings` call has returned `Applied`/`NoOp`. Measured
  against a fake substrate answering immediately, this lands in
  microseconds — the write itself, not a poll.
- A change the resident loop discovers on its own (nothing resubmitted)
  waits up to one `poll_interval_secs` (default 30s) before the first push
  is even attempted, since that is when the loop's own diff next runs.

Both are inside ADR-0021 §6's 5-second budget for the first clause
(reachable dependents); the second clause — an unreachable dependent
converging within one poll interval of coming back — is bounded by
`poll_interval_secs` and, until A6 ships durable delivery, by the absence
of an outbox to hold the push in until then. Neither miss implicates the
push model itself, so §6's redesign trigger has not fired; see ADR-0021's
own §6 amendment and ADR-0022 §11, which already draws this line for
callers outside the app instance.

`roymctl supervisor status`'s own `bindings` array is a **third**, slower
number: it reflects what the last health poll observed, so it lags a real
push by up to `poll_interval_secs` even after that push has already landed
-- the operator-facing confirmation, not the write's own latency.

##### Alerts over MQTT

Every alert a pass newly opens is published as it happens, in addition to
being queryable through `supervisor alerts`. Subscribe over the supervisor
node's own `messaging` capability, targeting the supervisor node's own DID:

```bash
# The topic an operator subscribes to is unnamespaced -- `<alert_topic>/
# <app-instance-id>`, matching `[roles.supervisor] alert_topic` above.
# A SyneroymClient (or any messaging/subscribe caller) targeting the
# supervisor node's own DID:
client.subscribe("messaging", "supervisor/alerts/guild-instance-1")
```

Each message is the alert's `app_instance_id`, `kind`, and `label` as JSON.
**Messages are not retained** — a subscriber connecting after an alert
opened does not receive it; `supervisor alerts` (or `--all` for the full
history including cleared ones) is the durable, replayable read. Any
verified (non-anonymous) caller may subscribe; there is no capability gate
on this yet (tracked in `deferred-backlog.md`).

### Interacting with Applications

#### Call a JSON-RPC method on a WASM app via HTTP Proxy

> [!TIP]
> You can use `roymctl alias <APP_DID> --nickname <NICKNAME> --interface <INTERFACE_NAME>` to get the full Host header.

```bash
# Host header format: <NICKNAME>-p<APP_DID_HASH>-i<INTERFACE_HASH>.localhost
curl -X POST http://localhost:7960/ \
  -H "Host: $(roymctl alias <APP_DID> --nickname <NICKNAME> --interface <INTERFACE_NAME>)" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "greet",
    "params": ["Syneroym User"],
    "id": 1
  }'
```

#### Call a TCP service via HTTP Proxy
```bash
# Simple GET request
curl http://localhost:7960/api/data \
  -H "Host: my-tcp-service-p<APP_DID_HASH>-i<INTERFACE_HASH>.localhost"
```

### Health and Metrics

#### Health Check
```bash
curl http://localhost:7966/health
```

#### Prometheus Metrics
```bash
curl http://localhost:7967/metrics
```

---

## 5. Milestone 2 Operational Runbook

This runbook outlines the operational procedures for managing cryptographic identities, hot-reloading TLS certificates, and verifying deployments using smoke tests.

### 5.1. Cryptographic Identity Delegation

To generate a Master Identity and delegate access to a Temporary Identity:

1. **Create the Master Identity**:
   ```bash
   roymctl identity create --name master-key
   ```
   Note the generated DID (e.g. `did:key:z6Mkha...`).

2. **Create the Temporary Identity**:
   ```bash
   roymctl identity create --name temp-key
   ```
   Note the generated Temporary DID.

3. **Issue a Delegation Certificate**:
   ```bash
   roymctl identity delegate \
     --master master-key \
     --temp-did <TEMP_DID> \
     --expires-days 90 \
     --scope routing
   ```
   This will output the JSON-encoded `DelegationCertificate`.

4. **Publish Master Anchor**:
   ```bash
   roymctl identity publish-anchor \
     --master master-key \
     --registry-url http://localhost:7961
   ```

> **A member master's instance certificate is a separate flow:**
> `roymctl identity certify-instance --master <name> --substrate <did>`
> (ADR-0020 §1) queries the target substrate over
> `orchestrator/resolve-instance-identity`, which is gated the same as every
> other `orchestrator/*` method (M05A Slice P0) — pass `--as <controller>`
> (or a `--ucan <token>` covering this app) once the substrate is claimed.
> It is denied outright on an unowned substrate.

### 5.2. TLS Setup & Zero-Downtime Reload

Milestone 2 supports hot-reloading TLS configurations (such as Let's Encrypt certificates generated by certbot) without restarting the substrate process.

1. **Configure TLS in `syneroym.toml`**:
   ```toml
   [tls]
   cert_path = "/etc/letsencrypt/live/example.com/fullchain.pem"
   key_path  = "/etc/letsencrypt/live/example.com/privkey.pem"
   reload_on_sigusr1 = true
   ```

2. **Hot-Reloading via SIGUSR1**:
   When certbot renews the certificate on disk, trigger a hot-reload by sending `SIGUSR1` to the substrate process:
   ```bash
   kill -USR1 $(pgrep syneroym-substrate)
   ```
   Check the substrate logs to verify the reload was successful:
   ```text
   Received SIGUSR1. Reloading TLS certificates from ...
   Successfully reloaded TLS certificates
   ```

### 5.3. Running Smoke Tests

Smoke tests can be run to verify the end-to-end functionality of the transport, registry, and sandbox layers.

1. **Run against a local deployment**:
   ```bash
   mise run test:smoke
   ```

2. **Run against a remote coordinator (e.g., staging/production)**:
   ```bash
   mise run test:smoke -- --coordinator-url https://syneroym.xyz
   ```

