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
- `product/` — Product specs, vision mapping, and acceptance criteria
- `libs/` — Language-specific substrate libraries (identity, discovery, transport, execution)
- `apps/` — User-facing applications (CLI, desktop/mobile shells, signaling surfaces)
- `examples/` — Demo integrations and reference apps
- `docs/` — Supporting documentation

## Scope Boundary: Substrate vs. Mini-Apps
The `libs/` directory is the shared substrate library layer that applications use.
Mini-apps that run on top of the substrate are out of scope for this repo and are
expected to be built independently (e.g., HTML/CSS/JS or WASM).

## Getting Started
This repo is being bootstrapped. For now:
- Read the vision in `product/00-vision.md`
- Read the spec pack in `product/01-spec-pack.md`

## Contributing
Contributions are welcome once the Phase 1 scope is locked. See
`CONTRIBUTING.md` for guidelines.

## License
MIT OR Apache-2.0
