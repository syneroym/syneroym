# Project: Syneroym

## General Instructions
- Focus religiously on these code aspects: Simplicity, performance, readability, testability, overall beauty, robustness, scalability, reliability.
- Follow standard Rust `clippy` guidelines. Before completion, confirm that `cargo +nightly fmt --all`, `cargo clippy --workspace --all-targets --all-features`, `cargo test --workspace`, and `mise run test:e2e` succeed.
- Try to use the latest stable versions of any library added.
- Have extensive integration and end to end tests for end user facing interfaces.
- Have solid unit tests for internal code if it is complex and delicate, even if it is not user facing.
- If new tools are needed in the build pipeline, add them to `mise.toml` too, so other dev environments easily get it.
- Do not commit code changes, and also do not add code changes to git index, when the current branch is `main`. On any other (feature) branch, staging and committing is allowed.
- Files with `scratch-notes` in the name, as well as the `docs/archive` folder, contain temporary or archived ideas and should be ignored by the agent. They might not contain reliable information.

## Commands
```bash
# Build the workspace
cargo build

# Format (nightly required — stable cargo fmt silently ignores the unstable
# import-grouping options this repo relies on)
cargo +nightly fmt --all

# Lint (must be clean; correctness/suspicious lints are deny-level workspace-wide)
cargo clippy --workspace --all-targets --all-features

# Full Rust test suite
cargo test --workspace
# Single crate / single test
cargo test -p syneroym-router routing::tests::some_test_name
# Via mise (recommended, matches CI)
mise run test:rust

# Playwright WebRTC end-to-end tests (crates/substrate/tests/e2e)
mise run test:e2e
mise run test:e2e-ui   # interactive/headed mode

# Run everything (Rust + e2e)
mise run test:all

# Smoke tests against a running local or remote coordinator/registry
mise run test:smoke
mise run test:smoke -- --coordinator-url https://syneroym.xyz

# Benchmarks (Criterion micro, latency, concurrency, soak)
mise run bench:micro
mise run bench:latency
mise run bench:concurrency
mise run bench:soak
cargo xtask perf-summary   # runs all of the above and appends to PERF_SUMMARY.md

# Run the substrate / CLI locally
cargo run --bin roymctl -- --help
cargo run --bin syneroym-substrate -- run --config <path>

# After `cargo update`, iroh's pre-release crypto chain needs re-pinning:
mise run deps:update

# Prune stale target/ artifacts (untouched 14+ days) instead of `cargo clean`
mise run clean:sweep
```
Crate names are `syneroym-<dir>` (e.g. `crates/data_db` → `syneroym-data-db`, `crates/coordinator_iroh` → `syneroym-coordinator-iroh`) — use these with `cargo test -p` / `cargo build -p`.

## Project & Rust Specifics
- Given the presence of WASM component configurations (`wasm32-wasip2`), maintain clean `wit` file boundaries and consider cross-compilation constraints.
- Emphasize idiomatic Rust formatting, leveraging the language's strong typing to guarantee correctness.
- Ensure any added dependencies reflect widely supported community standards.

## Functionality, Architecture Documents
- The [vision doc](docs/VISION.md) contains the vision for Syneroym
- The [requirements](docs/system-requirements-spec.md) contains high-level function requirements
- The [architecture](docs/system-architecture.md) contains high-level architecture
- The above docs are starting points for the implementation. It is likely that during implementation we deviate and improvise from those, and later get them in sync.

## AI Agent Guidelines
- **Mandatory Import Cleanup**: Before finishing any coding task, you MUST perform a dedicated final pass over the files you edited to clean up imports. You must strictly enforce the import rules (Types via standard `use`, Functions qualified by parent module) and proactively remove inline fully-qualified paths (lines with multiple `::`). For conflicting types like `Result` or `Error`, import their parent module (e.g., `use std::fmt;`) and use `fmt::Result` to avoid multiple `::`.
- **Prompt Clarity**: This is very important. If you don't understand what I am saying in the prompt clearly, it seems vague or confusing, please say so. Ask more questions, or explain how you would like me to rephrase the prompt. Or write out your understanding of the ask and request me to confirm before going ahead.
- **Interaction Style**: Respond concisely and directly. Use structured markdown for outputs, including code blocks, lists, and links to files/lines. Avoid verbose explanations unless requested.
- **Output Quality**: Ensure responses are accurate, idiomatic Rust code. Link to relevant files using workspace-relative paths (e.g., [src/main.rs](src/main.rs#L10)). Provide runnable code snippets with minimal setup instructions.
- **Security and Dependencies**: Do not exfiltrate secrets. Use minimal, pinned, widely-used libraries. Update manifests appropriately.
- **Git Commit Messages (Conventional Commits + 50/72 Rule)**: Prefix the subject line with a [Conventional Commits](https://www.conventionalcommits.org/) type (`feat`, `fix`, `docs`, `refactor`, `test`, `chore`, `perf`, `build`, `ci`, `style`, `revert`), plus an optional scope, e.g. `fix(data-db): ...`. The description after the colon is lowercase, in the imperative mood, with no trailing period, and the whole subject line stays at or under 50 characters where practical. The second line must be empty. The body (lines 3+) must be wrapped at 72 characters and explain the what and why, not the how.
- **Pull Request CLA Checkbox**: `.github/workflows/cla-enforcer.yml` fails any PR whose description doesn't contain this exact line, checked and unmodified, on its own line: `- [x] I have read and agree to the [Syneroym CLA](https://github.com/syneroym/syneroym/blob/main/CLA.md).` `gh pr create --body` overrides `.github/PULL_REQUEST_TEMPLATE.md` entirely, so always include this line verbatim in any PR body you author.

## Repository Structure and Key Components
- **Workspace Layout**: This is a Rust workspace with multiple crates in `crates/`, apps in `apps/`, documentation in `docs/`, and test components in `test-components/`. Key files include `Cargo.toml` (workspace config) and `mise.toml` (tool versions).
- **Apps**: `roymctl/`: Command-line tool.

### Architecture
The workspace builds one long-running binary, **`syneroym-substrate`** (`crates/substrate`, entry point `main.rs` → `runtime::init`/`run`), which hosts several independently-toggleable components selected by `SubstrateConfig.roles` and Cargo features. `RuntimeServices` (`crates/substrate/src/runtime.rs`) owns them and races their `run()` futures in a single `tokio::select!` alongside the connection router, health, and metrics endpoints; any component can be absent from a given deployment profile.

Core components wired together at startup:
- **`syneroym-router`** (`ConnectionRouter`) — the heart of the substrate. Accepts inbound streams from Iroh (QUIC/relay) and WebRTC transports, parses a **route preamble** (`<scheme>://<interface>.<service_id>[?enc=...]`, see `crates/router/src/preamble.rs`) that names a transport/protocol/service, and dispatches to either a local native service or a WASM sandbox instance. Also performs the E2E ECDH-P256 + AES-GCM handshake and DID-based access control (`handshake.rs`, `routing.rs`).
- **`syneroym-coordinator`** (+ `coordinator_iroh`, `coordinator_webrtc`) — helps peers discover each other and relays data/signaling when direct connection isn't possible (federated, multi-hop capable).
- **`syneroym-community-registry`** — service discovery: signed `EndpointInfo` records published to a registry (and optionally a BEP0044 DHT) so peers can look up how to reach a DID.
- **`syneroym-client-gateway`** — local HTTP proxy (port 7960) that maps `Host:` headers (`<nickname>-p<did-hash>-i<interface-hash>.localhost`) to the right local endpoint/service, used by external HTTP clients and `roymctl`.
- **`syneroym-app-orchestration`** + **`syneroym-sandbox-wasm`** — the "orchestrator" native service: manages the deployed-app catalog/lifecycle (`catalog.rs`, `compiler.rs`, `reconcile.rs`, `resolver.rs`) and runs user WASM components (via Wasmtime) or delegates to `syneroym-sandbox-podman` for OCI container services.
- **`syneroym-control-plane`** — service definitions/types for deploying apps and controlling running services, exposed as JSON-RPC over the client gateway / native RPC dispatch.
- **`syneroym-wit-interfaces`** — generates Rust host/guest types from the WIT interfaces under `crates/wit_interfaces/wit/` (`host`, `data-layer`, `blob-store`, `app-config`, `control-plane`, `vault`) via `wit-bindgen`/`bindgen!`; this is the WASM component boundary — host-side crates (e.g. `syneroym-data-db`, `syneroym-data-blob`) speak these generated types directly with no separate conversion layer.
- **`syneroym-data-db`**, **`syneroym-data-blob`** — host-implemented storage capabilities (SQLite-backed structured data, content-addressed encrypted blob storage) exposed to WASM guests through the WIT interfaces above.
- **`syneroym-identity`**, **`syneroym-data-keystore`** — DID-based cryptographic identity (ed25519), delegation certificates, and KEK/DEK key management.
- **`syneroym-observability`** — metrics (Prometheus `/metrics`), health (`/health`), logging/tracing, wired in as plain Axum routes inside `runtime.rs` rather than a separate service.
- **`syneroym-rpc`** — shared RPC framing/serialization/transport-adapter layer used across JSON-RPC 2.0 traffic today; the `wrpc://` scheme/native wRPC component protocol is reserved but **not yet implemented** (JSON-RPC 2.0 is the actual current wire protocol everywhere the docs mention wRPC).

Config is a single `SubstrateConfig` (TOML) covering identity, coordinator roles, TLS (with SIGUSR1 hot-reload), and per-component enable flags. Ports follow a normalized `796x` scheme documented in [docs/developer-guide.md](docs/developer-guide.md) (gateway 7960, registry 7961, WebRTC bootstrap 7962, WebRTC signaling 7963, Iroh coordinator HTTP/QUIC 7964/7965, health/metrics 7966/7967).

`test-components/` holds minimal WASM guest components (`greeter`, `data-layer-test`, `miniapp-demo1-web`) used as fixtures for sandbox/e2e tests — they're excluded from the main workspace build graph (see root `Cargo.toml` `exclude`) since they cross-compile to `wasm32-wasip2`.
