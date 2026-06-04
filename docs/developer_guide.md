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
> For a full design, layout of the metrics, leak detection heuristics, and results gating, refer to the comprehensive [Performance & Robustness Testing Report](performance_and_robustness_report.md).


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
```bash
# Note: WASM binary bytes are usually sent as a base64-encoded array or via a URL.
curl -X POST http://localhost:7960/ \
  -H "Host: <NICKNAME>-p<SUBSTRATE_DID_SHORTHASH>-iorchestrator.localhost" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "deploy",
    "params": [
      "did:key:my-app-did",
      ["my-interface:v1"],
      {
        "config": { "env": [], "args": [], "custom_config": null },
        "service_type": {
          "wasm": {
            "source": { "url": "http://example.com/app.wasm" },
            "hash": "sha256:..."
          }
        }
      }
    ],
    "id": 1
  }'
```

#### Deploy a TCP Service (Passthrough)
```bash
curl -X POST http://localhost:7960/ \
  -H "Host: <NICKNAME>-p<SUBSTRATE_DID_SHORTHASH>-iorchestrator.localhost" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "deploy",
    "params": [
      "did:key:my-tcp-service",
      ["default"],
      {
        "config": { "env": [], "args": [], "custom_config": null },
        "service_type": {
          "tcp": {
            "host": "localhost",
            "port": 8080
          }
        }
      }
    ],
    "id": 1
  }'
```

#### Deploy a Container Service (Podman)
```bash
curl -X POST http://localhost:7960/ \
  -H "Host: <NICKNAME>-p<SUBSTRATE_DID_SHORTHASH>-iorchestrator.localhost" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "deploy",
    "params": [
      "did:key:my-container-service",
      ["default"],
      {
        "config": { "env": [], "args": [], "custom_config": null },
        "service_type": {
          "container": {
            "source": { "binary": [] },
            "hash": null,
            "image": "docker.io/library/nginx:alpine",
            "ports": [
              {
                "interface_name": "default",
                "host_port": null,
                "container_port": 80,
                "protocol": "tcp"
              }
            ],
            "volumes": [
              {
                "host_path": "html",
                "container_path": "/usr/share/nginx/html"
              }
            ]
          }
        }
      }
    ],
    "id": 1
  }'
```

#### Developing Podman Services Locally
When developing a Podman container service for Syneroym:
1. **Rootless:** Ensure the container can run rootless. Syneroym uses Podman in rootless mode by default.
2. **Build:** Build your image locally (`podman build -t my-app:latest .`).
3. **Reference:** During the orchestrator `deploy` call, reference `localhost/my-app:latest` or `docker.io/library/nginx:alpine` in the `image` field.
4. **Debug:** Use standard tools (`podman ps`, `podman logs <container-id>`) on your host to inspect the container if it fails to bind or start via the orchestrator.

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
