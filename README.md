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

## Getting Started (Quickstart)

This repository uses [mise](https://mise.jdx.dev/) to automatically manage toolchains.

```bash
# 1. Install toolchains
mise install

# 2. Install dependencies & build
pnpm install
cargo build

# 3. Run all tests (Rust unit/integration & E2E)
mise run test:all
```

For more detailed workflows, port references, test suites (unit, e2e, and micro-benchmarks), and API examples, refer to the [Developer Guide](docs/developer-guide.md).

## Related Documentation

- [Product vision mapping ](docs/VISION.md) 
- [Requirement specifications](docs/system-requirements-spec.md) 
- [System Architecture](docs/system-architecture-design.md)
- [Developer Guide](docs/developer-guide.md) — Detailed setup, testing, and API examples

## Contributing

Contributions are welcome once the Phase 1 scope is locked. See `CONTRIBUTING.md` for guidelines.

## License

Apache-2.0
