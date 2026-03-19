# Syneroym

> Status: Early-stage, under active development
>
> This repository is the primary product build for Syneroym. It is **not
> production-ready**. Expect breaking changes, incomplete features, and
> evolving architecture.

For a look at the earlier feasibility work and verification, check out the [Syneroym Prototype](https://github.com/syneroym/syneroym-prototype) repository.

## What This Is
Syneroym is an identity-native, user-space peer substrate enabling direct
peer-to-peer interaction among independently owned entities, without
central authorities or global consensus. The substrate is designed for
intermittent connectivity, explicit trust boundaries, and portable execution.

## Repository Layout
- `docs/` — Vision mapping, Product specs, architecture, and requirements docs
- `crates/` — Rust crates for the core substrate, network protocols, identity, CLI, and relay
- `wit/` — WebAssembly Interface Types for component interactions
- `apps/` — User-facing applications (CLI, desktop/mobile shells, signaling surfaces)
- `examples/` — Demo integrations and reference apps

## Scope Boundary: Substrate vs. Mini-Apps
The `crates/` directory contains the core substrate components and libraries that applications use.
Mini-apps that run on top of the substrate are out of scope for this repo and are
expected to be built independently (e.g., HTML/CSS/JS or WASM).

## Getting Started

This repo is being bootstrapped. For now:
- Read the vision in `docs/VISION.md`
- Read the requirement spec in `docs/requirements.md`
- Read the architecture design in `docs/architecture.md`

### Prerequisites

We recommend using [mise](https://mise.jdx.dev/) to manage development tools.

```bash
# Install tools specified in mise.toml (Rust, Node, wasm-tools, etc.)
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

Run the Rust test suite:

```bash
cargo test
```

## Contributing
Contributions are welcome once the Phase 1 scope is locked. See `CONTRIBUTING.md` for guidelines.

## License
MIT OR Apache-2.0
