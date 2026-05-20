# Syneroym

> ⚠️ **Status: Exploratory / Unstable**
>
> This project is **WORK IN PROGRESS** and is under active exploration and development. The features, architecture, APIs, data models, and overall direction are subject to frequent change. Nothing here should be considered stable or production-ready at this stage. The repository is public for transparency and ease of sharing, not as an invitation for general use or contribution at this time.

For a look at the earlier feasibility work and verification, check out the [Syneroym Prototype](https://github.com/syneroym/syneroym-prototype) repository.

## What This Is
Syneroym is an identity-native, user-space peer substrate enabling direct
peer-to-peer interaction among independently owned entities, without
central authorities or global consensus. The substrate is designed for
intermittent connectivity, explicit trust boundaries, and portable execution.

## Repository Layout
- `docs/` — Vision mapping, Product specs, architecture, and requirements docs
- `crates/` — Rust crates for the core substrate, network protocols, identity, and relay
    - `substrate/` — Entry point for the substrate
- `apps/` — User-facing applications (CLI, desktop/mobile shells)
    - `roymctl/` — CLI for accessing services of the local substrate as well as other ecosystem services
- `examples/` — Demo integrations and reference apps

## Scope Boundary: Substrate vs. Mini-Apps
The `crates/` directory contains the core substrate components and libraries that applications use.
Mini-apps that run on top of the substrate are out of scope for this repo and are expected to be built independently (e.g., WASM components for backend logic, and HTML/CSS/JS loaded in WebViews for frontend UI).

## Getting Started

This repo is being bootstrapped. For now:
- Read the vision in `docs/VISION.md`
- Read the requirement spec in `docs/requirements.md`
- Read the architecture design in `docs/architecture.md`

### Prerequisites

We recommend using [mise](https://mise.jdx.dev/) to manage development tools.

```bash
# Install tools specified in mise.toml (Rust, wasm-tools, etc.)
mise install
```

Alternatively, you can manually install the required versions of [Rust](https://rustup.rs/) and [Node.js](https://nodejs.org/).

### Building

Install dependencies and build the project:

```bash
# Install Node/frontend dependencies
pnpm install

# Build all Rust crates
cargo build
```

### Running

You can run individual binaries using Cargo. For example, to run the CLI (`syneroym`):

```bash
cargo run --bin syneroym -- --help
```


### Testing

We organize tests into two suites: the **Rust suite** (for unit/integration tests) and the **Playwright E2E suite** (for browser-automation and WebRTC data plane scenarios).

#### Using Mise (Recommended)

If you are using `mise`, you can run self-documenting workspace tasks:

```bash
# Run both the Rust and browser E2E test suites sequentially
mise run test:all

# Run only Rust unit and integration tests
mise run test:rust

# Run only Playwright E2E browser tests
mise run test:e2e
```

#### Using raw commands

If you're not using `mise`, you can run the suites individually using standard toolchains:

```bash
# Run Rust unit/integration tests
cargo test --workspace

# Run Playwright E2E tests
cd crates/substrate/tests/e2e
npm install
npm test
```


## Contributing
Contributions are welcome once the Phase 1 scope is locked. See `CONTRIBUTING.md` for guidelines.

## License
MIT OR Apache-2.0
