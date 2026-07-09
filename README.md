# Syneroym

Syneroym is a truly peer-to-peer foundation for group communication and trust, on which independent mini-apps (SynApps) — chat, marketplace, social, AI — plug in and work together as one experience, with no central server in the middle. No blockchains or cryptocurrency. The [thesis](THESIS.md) states the full bet, and what this is and is not.

Our flagship experience is **Roym** — mini-apps sharing one identity, one contact list, one set of groups, one trust model — starting with its first vertical, the Professional Services Guild. The substrate is generic — anyone can build their own SynApp on it.

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

- [Project thesis](THESIS.md)
- [Terminology](docs/TERMINOLOGY.md) — canonical names and phrasings
- [Product vision mapping ](docs/VISION.md) 
- [Requirement specifications](docs/system-requirements-spec.md) 
- [System Architecture](docs/system-architecture.md)
- [Developer Guide](docs/developer-guide.md) — Detailed setup, testing, and API examples

## Contributing

Contributions are welcome once the Phase 1 scope is locked. See `CONTRIBUTING.md` for guidelines.

## License

Syneroym is dual-licensed under **MPL 2.0** and a **Commercial License**.

- **MPL 2.0 (Mozilla Public License 2.0):** The core Syneroym substrate is open source under the MPL 2.0. This is a file-level copyleft license, which means you are free to link, embed, and use Syneroym in your proprietary and commercial products (including SynApps, SynSvcs, and WebAssembly components) without "infecting" your own code. You only need to share your source code if you modify the existing Syneroym files themselves.
- **Commercial License (Copyleft Exemption):** If your use case requires modifying the core Syneroym source files while keeping those modifications closed-source and proprietary, you can obtain a Commercial License that grants a full exemption from the MPL's copyleft obligations. (Note: This commercial option strictly provides copyleft exemption and does not include warranties, indemnification, or SLAs). 

For commercial licensing inquiries, please fill out our [Commercial License Inquiry Form](https://forms.gle/wztMPRTXkc1QioPo8).

See the [LICENSE](LICENSE) file for the full text of the MPL 2.0.
