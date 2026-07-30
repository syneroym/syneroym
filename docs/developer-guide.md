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

