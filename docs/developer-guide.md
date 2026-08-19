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
roymctl registry lookup "alice-<SERVICE_DID_SHORTHASH>"
```

### Discovering Services

Lookup a specific service by its DID:
```bash
# Returns signed endpoint info
curl http://localhost:7961/lookup/did:key:z6MkhaXn...
```

### Managing Applications (Orchestrator)

The Orchestrator is a native service running inside the substrate. You can interact with it via the Client Gateway (Port 7960).

> **The `curl`-via-gateway examples below require a claimed substrate and an
> active controller person session, or direct `roymctl` invocation.** Without an
> active person session token, the client gateway presents the *node's own* DID as caller,
> never the controller's (which fails closed for management requests). When a controller
> logs in via `roymctl session login --as <owner>`, requests carrying the session cookie or
> bearer token present the delegated person identity through the gateway. Direct `roymctl`
> commands (e.g. `roymctl --dir <DIR> --as owner svc deploy ...`) sign requests directly.

#### List Deployed Services
```bash
# Replace <NICKNAME> and <SUBSTRATE_DID_SHORTHASH>. `-iorchestrator` names
# the interface literally, not by hash -- the parser accepts either.
curl -X POST http://localhost:7960/ \
  -H "Host: <NICKNAME>-s<SUBSTRATE_DID_SHORTHASH>-iorchestrator.localhost" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "list",
    "params": {},
    "id": 1
  }'
```

> [!NOTE]
> **"Interface" means two different things depending on the service type.**
> For a WASM component, an interface is a WIT-exported namespace of
> functions — part of the component itself, in the WASM component-model
> sense. For a TCP or container service, there is no component to export
> anything: "interface" here just names one of possibly several endpoints
> the substrate registers for the same backing `(host, port)` or process —
> closer to "a named auxiliary port" (e.g. a metrics or readiness port
> alongside the main one) than to a WASM interface. Both senses share the
> same `--interfaces` flag and the same hostname `-i<hash>` segment because
> the mechanism that registers and resolves them (`EndpointRegistry`) does
> not need to know which sense applies — but a reader should not assume a
> TCP/container "interface" means the same thing a WASM one does.
>
> When a service declares exactly one interface -- including via
> `--interfaces` left blank, which `roymctl svc deploy` (any of `--wasm`,
> `--tcp`, `--image`) treats as "one interface, name it for me" -- the
> destination resolves it automatically; a hostname can omit `-i` too, and
> gets the same answer. That single, unnamed interface is always called
> `default`, not `""` or `main` -- both other spellings existed briefly, in
> different code paths, before being unified.

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

##### Declaring Publication Visibility (ADR-0018)

`svc deploy` publishes nothing by default. Whether the service's endpoint
record reaches the community registry, and how far, is a declaration, not a
side effect of which flags happened to be passed:

```bash
roymctl --dir <DIR> --as owner svc deploy \
  --svc-id did:key:my-app-did \
  --interfaces my-interface:v1 \
  --wasm ./app.wasm \
  --identity my-service-key \
  --visibility internal
```

`--visibility` is one of `public` (registered and propagated to parent
registries), `internal` (registered with this substrate's community
registry only — the value a multi-substrate app's own members need to
resolve each other, not `public`), or `private` (never registered; the
default when neither `--identity` nor `--master` is given). `public`/`internal`
require `--identity` or `--master`, since only the service's own
key can sign a record the registry will admit; a mismatched
`--identity`/`--svc-id` pair is refused before anything is built. **Once
`--identity` or `--master` is given, `--visibility` has no default and must
be stated explicitly** — a deploy that could sign a record but was not told
whether to publish it is refused rather than silently deploying unpublished.
`--record-out <path>` writes the signed record to a file instead of (or as
well as) handing it to the substrate, for sharing a `private` service's
record out of band with `SyneroymClient::new_with_record`.

An app deployed through a `SynAppManifest` declares the same thing per
service, in TOML (`ServiceSpec`'s `config` is flattened, so `visibility`
sits directly under the service, like `fdae` above):
```toml
[services.my-svc]
visibility = "internal"
```
Omitting it means `private` — a member with no declaration is deployed but
never published, and a cross-node call to it fails to resolve. A logical
service's `topology_visibility = "open"` (ADR-0022 §5, below) still needs a
registered `visibility` for an outside caller to actually reach any of its
members.

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
topology_document_not_after_secs = 3600       # 1h; how long a signed Tier-2 topology
                                               # document stays usable after signing --
                                               # see "Resolving an app's logical service
                                               # from outside it" below
topology_document_cache_ttl_secs = 300        # 5m; advisory re-ask interval carried
                                               # inside the signed document
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

**What this DID is for.** The Logical Service Discovery Overlay
(ADR-0022) resolves it in two tiers: Tier 1 (the app DID → the supervisor
holding it) is the resident loop's own periodic publish to
`substrate.registry_url`, once this instance is adopted; Tier 2 (one
logical service → its current member set) is the `supervisor resolve`
verb documented below, and `roymctl app resolve` is the outside caller's
own path into it. Tier 3 (a member DID → an address) is the ordinary
registry lookup every other DID in this system already goes through.
`status`'s `app_record_expires_at` field reports how long the Tier-1
record has left before it needs another refresh.

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

##### Resolving an app's logical service from outside it (ADR-0022 §3)

A caller that is not part of an app instance — a different app, an
operator's own tooling — can still find out who currently answers for one
of that app's logical services, without ever talking to that app's own
supervisor node directly first. `roymctl app resolve` walks all three
tiers:

```bash
roymctl --api-url http://<registry> --as <caller-identity> --ucan <grant.json> \
  app resolve did:key:<app-instance-master-did> backend
# -> app: did:key:...  service: backend
#    mode: Redundant  epoch: 2
#    members:
#      did:key:...
#      did:key:...
```

It looks the app DID up in the registry (Tier 1) to find the supervising
node, fetches that supervisor's signed topology document over `supervisor
resolve` (Tier 2), and verifies the document's signature against the app
DID it resolved in Tier 1 — never against whatever the document itself
claims, and never trusting the connection it arrived over. Turning a
printed member DID into an address is Tier 3, an ordinary registry lookup
unaffected by any of this.

**The document is signed once per `(service, epoch)` and cached in the
supervisor's own memory, not once per request** — a burst of callers
resolving the same service costs one signature, not one per caller. A
caller that has already fetched and verified a document keeps routing
from its own cache — no further network call to the supervisor — until
`topology_document_not_after_secs` after it was signed; a supervisor that
is down does not stop an already-resolved caller from continuing to
route, only from resolving something for the first time.

**`resolve` is gated by a capability, not by `substrate/admin`.** An
operator who owns the supervisor's node already has it (a bare
`substrate/admin` grant covers everything on that node), but any other
caller needs `supervisor/resolve` on `synapp:<app-did>` specifically:

```bash
roymctl --dir <DIR> --as <supervisor-node-owner> identity issue-grant \
  --to did:key:<caller-did> \
  --resource "synapp:did:key:<app-instance-master-did>" \
  --can supervisor/resolve \
  --out grants/resolve-guild-instance-1.json
```

An unknown app DID and a caller holding no grant for a real one are
refused identically — the error names neither the app's members nor
whether it exists, so a caller with no grant cannot use `resolve` to
probe which apps this supervisor manages.

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

##### Scheduling a service (`schedule`)

```toml
[services.reports]
service_type = "wasm"
source = "reports.wasm"
interfaces = ["scheduled-driver"]

[services.reports.schedule]
cron = "0 3 * * *"
interface = "scheduled-driver"
method = "generate-nightly-report"
# params = "[\"optional\", \"json\", \"args\"]"   # absent sends []
# timeout_ms = 10000                              # default; ceiling is 30000
```

`schedule` tells the supervisor to fire `method` on `interface` on exactly
one member of that logical service, on `cron`'s own cadence — there is no
lease and no cluster scheduler anywhere in this design (ADR-0023 §6); the
supervisor that already owns the instance is the one that decides "when"
and dispatches locally on the substrate that hosts the picked member.
`cron` is a standard five-field crontab expression (`min hour dom mon dow`),
**evaluated in UTC** — there is no per-schedule or per-manifest time zone
setting, so a nightly job written as `"0 3 * * *"` fires at 03:00 UTC
regardless of where the substrate or the operator sits. `interface` must be
one the service's own `interfaces` list already declares; `method` is not
otherwise checked against the deployed component, since the manifest names
an artifact, not a parsed WIT world. At run time the hosting substrate
refuses a tick whose `(service, interface)` pair it has no local endpoint
for, so a schedule naming an interface the component does not actually
export fails fast on that substrate and shows up in `last-error`, instead
of being resolved as if the service lived on some other node. Up to 16
services may declare a schedule per app instance, the same shape and the
same reason as `replicas`'s own cap.

Absent `params` sends an empty positional array to `method`, not `null` —
the same shape the substrate's own `rpc` readiness probe sends a
no-argument guest method. `timeout_ms` (default 10s) is this run's own
budget, and must be between 1ms and the 30s ceiling — a manifest asking
for anything outside that range is refused at validation, rather than
silently clamped. The ceiling exists because the call is awaited inline
inside a reconcile pass that runs app instances one at a time — every
second here is a second every *other* instance this supervisor manages
waits its turn. A budget above the guest's own execution limit (5s,
`dispatch_epoch_timeout_secs`) buys no extra work, only a longer wait on a
substrate that is not answering.

**A missed tick is skipped, never run late.** If this supervisor was down,
paused, or its substrate was unreachable when a tick was due, that
occurrence is gone — the next tick is the retry, not a burst of catch-up
runs on the pass after recovery. A tick still fires if the pass that would
have caught it lands late by no more than the *grace window*: two poll
intervals, or the real gap between this supervisor's last two sweeps if
that is longer. The second half matters in practice — a sweep reconnects
to every managed substrate and routinely takes longer than its nominal
interval, and a window sized only from the configured interval would drop
ticks while the supervisor was awake the whole time. The gap is measured
inside one process, so a restart resets it and downtime never widens the
window. A schedule finer than `poll_interval_secs` (default 30s) collapses
to at most one run per pass, silently — a `cron` firing every 10 seconds
still runs at most once every 30.

**A scheduled run is never queued and never dead-letters.** Unlike a
binding push, a failed or timed-out run does not go through the durable
outbox (ADR-0023 §3): a tick for 03:00 delivered late at 06:00 is work
whose window has already passed, and the next night's tick is a better
retry than a delayed delivery. A failure instead raises a standing
`SCHEDULED_RUN_FAILED` alert (one row per logical service, regardless of
which member's substrate actually ran the failing tick), cleared by the
next successful run.

```bash
roymctl --substrate this-node supervisor schedules guild-instance-1
```

lists every schedule this instance declares, alongside `evaluated-at` (this
pass's own watermark), `last-run-at` and `last-member-index` (the most
recent actual run — both absent until one has happened), and `last-error`.
`last-run-at` that has stopped advancing while `evaluated-at` keeps moving
is the visible form of a substrate this supervisor cannot reach — the
honest cost of running with no lease (ADR-0023 §6), rather than something
worked around.

`roymctl app deploy` has no supervisor behind it, and the supervisor is the
one thing that runs a schedule — deploying a manifest that declares one
this way **warns and still deploys**, the same posture already taken for a
registry that cannot resolve every member: the deploy itself is valid, one
declared behavior just never happens. Use `roymctl supervisor submit` for
any manifest that carries a schedule you actually want to fire.

##### Compensating a workflow (sagas)

A service that drives a multi-service workflow records each step as it
takes it, so a failure -- or this substrate dying mid-workflow -- can walk
the record backwards, undoing what already ran. The platform never chooses
the forward path and never decides a workflow failed on its own; it only
orders the backward walk, delivers each undo under a fence it mints itself,
retries on a backoff, and shows an operator what it could not undo
([ADR-0023](decisions/0023-durable-async-primitives.md) §7, as amended
2026-08-07).

```wit
// The forward operation, on whatever interface the service already exports.
reserve: func(item: string) -> result<string, string>;

// Its compensation: same parameters, plus one optional trailing member
// carrying the forward call's own return value.
saga-undo-reserve: func(item: string, forward-result: option<string>)
    -> result<_, string>;
```

`saga-undo-<method>` is the whole convention. **Nothing is declared in the
manifest** -- a service with no compensation is the ordinary case (an
idempotent or read-only operation has nothing to undo), not an author who
forgot, so absence already means "takes no part in any saga". The deploy
path checks the compiled component's own exports instead: every
`saga-undo-<x>` it exports must have an `<x>` beside it on the same
interface, or the deploy is refused. `saga-` is a reserved prefix precisely
so this check is sound -- a bare `undo-` would be an ordinary business verb
(`undo-last-update` is a legal, unrelated export) and could not be checked
this way without either refusing legal names or letting the walk call a
business function by mistake.

The driving side calls a second WIT interface to record steps as it takes
them:

```wit
let saga = saga::begin("checkout", /* deadline-secs */ some(3600))?;
let result = saga::step(saga, target, "reserve", "reserve", params, none)?;
// ... more steps ...
saga::commit(saga)?;      // the workflow reached its goal
// or, on failure:
saga::compensate(saga)?;  // give up -- the walk starts on the next tick
```

`step` is `call` plus a durable write: the intent is recorded *before* the
call is dispatched, and the outcome after. **This means a `saga-undo-<op>`
may be called for an operation that never happened** -- if the substrate
died between the call leaving and its answer arriving, the step is still
compensated, because the host cannot tell "never ran" from "ran but the
answer never came back". Write every undo as "ensure this is not in
effect" rather than "reverse this", and it handles both cases for free.

`compensate` returns as soon as the saga is marked -- **not** once the walk
finishes. A workflow of N steps is N remote undos, comfortably longer than
a guest's own dispatch budget, so nothing waits for it inline. Poll
`status`, or the operator verb below, for the outcome. The walk itself
undoes the newest step first, one at a time, on the async worker's own
tick -- the same worker that drains the guest outbox.

**The other way a saga starts compensating: its own deadline.** `begin`'s
`deadline-secs` (absent takes the node's default; the node also enforces a
ceiling) is the *only* way a workflow interrupted by a crash is ever
unwound, because a WASM component does not exist between calls -- nothing
else can notice it died mid-workflow. A service whose instance certificate
is not being renewed cannot compensate past that certificate's own expiry;
`begin` warns when the requested deadline outlives the caller's current
certificate, but does not refuse the call.

```bash
roymctl svc sagas --svc-id <SERVICE-DID>
```

lists every saga the service's own log holds: state (`open` /
`compensating` / `compensated` / `failed`), how many of its steps are
compensated so far, and its last error. A saga that reaches `failed` --
its undo attempt budget exhausted, or a step's target answered in a way
that settles the question rather than one worth retrying -- keeps its
step history and stays visible here; nothing is silently dropped.

```bash
roymctl svc saga-compensate --svc-id <SERVICE-DID> --saga-id <SAGA-ID>
```

re-arms a `failed` saga: it returns to `compensating` with its current
step's attempt count reset, and the worker picks it up on its next tick.
Like `proxy-replay`, this never walks inline -- it only re-queues the
attempt.

**A substrate whose vault is locked compensates nothing**, and says so: the
sweep logs a warning per service per tick rather than silently skipping,
distinguishing a locked node from one that is merely idle. Every saga is
intact on disk and resumes exactly where it left off once an operator
injects the KEK.

**A queued (`enqueue`d) call can never be a saga step.** `enqueue` returns
"accepted for delivery", never "delivered" -- its own outcome is unknown to
the caller by construction, so it cannot be recorded as a completed or
failed step. A saga step is always a synchronous `saga::step` call.

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

#### The gateway hostname scheme (ADR-0022 §7)

One grammar, two forms. Everything after the first label is ignored (it
doesn't have to be `localhost` -- any domain works unchanged):

```text
<nickname>-s<service-did-hash>[-i<interface-hash>].<domain>          # a service outside any app instance
<nickname>-a<app-did-hash>-s<service-name-hash>[-i<interface-hash>].<domain>  # a logical service of an app instance
```

`-s` is the only required segment. `-a`'s presence decides whether `-s`
carries a hash of a concrete service DID (reversed by the registry's alias
index) or a hash of a logical service name inside an app (reversed by the
app's own supervisor). Omitting `-i` means "the service's one
app-declared interface" and is resolved at the destination — it fails
loudly if the service declares zero or more than one.

`roymctl alias <SERVICE_ID> --nickname <NICKNAME> [--interface <NAME>]`
builds the unscoped form; adding `--service <LOGICAL_SERVICE_NAME>` (which
also requires `--nickname`, since it must be the app's own
`AppInstanceId`) builds the app-scoped form against the app's own master
DID.

An app-scoped hostname carries a routing key, when the caller has one, as
the `X-Syneroym-Routing-Key` request header rather than a hostname
segment — the gateway forwards it unmodified. A `Redundant` service picks
the same member for the same key; a `Sharded` service requires one.

The member is selected once, from the first HTTP request on a TCP
connection, and every later request on that same connection (an HTTP
keep-alive reusing the socket) rides the same tunnel — the header on a
second request on a reused connection has no effect. A client that needs
per-request routing-key accuracy must open a fresh connection per request
(most HTTP clients do this by default when the `Host` changes; a client
pooling connections to one gateway across different keys does not).
Tracked in `deferred-backlog.md`.

Resolving an app-scoped host needs the gateway (or WebRTC coordinator) to
be authorized for `supervisor/resolve` on the target app. Two config
keys, in `[iam]` and `[roles.client_gateway]` (or `[roles.coordinator]`)
respectively:

```toml
[iam]
# Grants this node's own DID `supervisor/resolve`, node-wide -- covers
# every app supervised on this node with no credential file. Off by
# default.
grant_resolve_to_node_did = true

[roles.client_gateway]
# A CapabilityToken file granting `supervisor/resolve` on apps supervised
# by *other* nodes -- the same file `roymctl app resolve --ucan` takes.
resolve_ucan = "/path/to/resolve-token.json"
```

With neither set, every app-scoped hostname is refused by the supervisor
it reaches (a startup warning names both keys); unscoped (`-s` only)
hostnames are unaffected either way. A third way in needs neither
key: an app that declares a logical service `topology_visibility = "open"`
(ADR-0022 §5) answers any verified caller with no capability at all, so a
gateway with no credential for that app still resolves it. `[iam]
grant_resolve_to_node_did` stays the *operator*-side answer for a node's
own apps' `restricted` services — the two are independent, not
alternatives for the same case.

#### Call a JSON-RPC method on a WASM app via HTTP Proxy

> [!TIP]
> You can use `roymctl alias <APP_DID> --nickname <NICKNAME> --interface <INTERFACE_NAME>` to get the full Host header.

```bash
# Host header format: <NICKNAME>-s<APP_DID_HASH>[-i<INTERFACE_HASH>].localhost
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
  -H "Host: my-tcp-service-s<APP_DID_HASH>-i<INTERFACE_HASH>.localhost"
```

#### Person Sessions at the Client Gateway (ADR-0016 §0.5)

By default, unauthenticated requests arriving at the client gateway are proxied presenting the **node's own DID** as caller, which downstream handlers and WASM guests see as `self-asserted`.

To act as a verified person identity, a client opens a session with the gateway:

1. **Login with `roymctl`**:
   ```bash
   roymctl session login --as alice --gateway-url http://localhost:7960 --registry-url http://localhost:7961
   ```
   This issues a challenge to the gateway, signs a delegation certificate from Alice's master identity to the gateway's node identity (with `routing` scope), publishes Alice's master anchor, and saves the session token to `<config-dir>/sessions/<sanitized-gateway-url>.json` (mode 0600 on Unix).

2. **Inspect Session Status**:
   ```bash
   roymctl session status --gateway-url http://localhost:7960
   ```

3. **Use Session with `curl`**:
   Present the session token via either the `syneroym_session` cookie or `Authorization: Bearer` (Cookie takes priority when both are present):
   ```bash
   # Via Cookie
   curl http://localhost:7960/echo \
     -H "Host: $(roymctl alias <SERVICE_DID>)" \
     -H "Cookie: syneroym_session=$(roymctl session token --gateway-url http://localhost:7960)"

   # Or via Authorization Bearer
   curl http://localhost:7960/echo \
     -H "Host: $(roymctl alias <SERVICE_DID>)" \
     -H "Authorization: Bearer $(roymctl session token --gateway-url http://localhost:7960)"
   ```
   The gateway strips the session credential before proxying to the destination service, attaching the delegation certificate so downstream services see `auth = delegated` and `did = alice_did`.

4. **Logout**:
   ```bash
   roymctl session logout --gateway-url http://localhost:7960
   ```

##### Reserved Endpoints (`/_syneroym/session/*`)

The gateway intercepts paths beginning with `/_syneroym/` locally and never proxies them:
- `POST /_syneroym/session/challenge`: Returns a cryptographic nonce, the gateway's node DID, and challenge expiry.
- `POST /_syneroym/session/login`: Validates the signed challenge and delegation certificate, returning a session token and setting `Set-Cookie: syneroym_session=...; HttpOnly; SameSite=Strict`.
- `GET /_syneroym/session/whoami`: Returns the active session's person DID and expiration.
- `POST /_syneroym/session/logout`: Terminates the session and clears the cookie (`Max-Age=0`).

##### Configuration

```toml
[roles.client_gateway]
http_port = 7960
# Session lifetime in seconds (defaults to 28800 = 8 hours)
session_ttl_secs = 28800
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

