# Project: Syneroym

## General Instructions
- Focus religiously on these code aspects: Simplicity, performance, readability, testability, and overall beauty.
- Follow standard Rust `clippy` guidelines. Before completion, confirm that `cargo fmt --all`, `cargo clippy --workspace --all-targets --all-features`, and `cargo test --workspace` succeed.
- Try to use the latest stable versions of any library added.
- Have extensive integration and end to end tests for end user facing interfaces.
- Have solid unit tests for internal code if it is complex and delicate, even if it is not user facing.
- Don't lose focus of non functional aspects throughout the development cycle. Aspects such as:
    - Performance, Reliability, Maintainability, Scalability, Testability, Documentation
- If new tools are needed in the build pipeline, add them to `mise.toml` too, so other dev environments easily get it.
- Do not commit code changes.

## Project & Rust Specifics
- Given the presence of WASM component configurations (`wasm32-wasip1`), maintain clean `wit` file boundaries and consider cross-compilation constraints.
- Emphasize idiomatic Rust formatting, leveraging the language's strong typing to guarantee correctness.
- Ensure any added dependencies reflect widely supported community standards.

## Functionality, Architecture Documents
- The [vision doc](docs/VISION.md) contains the vision for Syneroym
- The [requirements](docs/requirements.md) contains high-level function requirements
- The [architecture](docs/architecture.md) contains high-level architecture
- The above docs are starting points for the implementation. It is likely that during implementation we deviate and improvise from those, and later get them in sync.

## AI Agent Guidelines
- **Interaction Style**: Respond concisely and directly. Use structured markdown for outputs, including code blocks, lists, and links to files/lines. Avoid verbose explanations unless requested.
- **Output Quality**: Ensure responses are accurate, idiomatic Rust code. Link to relevant files using workspace-relative paths (e.g., [src/main.rs](src/main.rs#L10)). Provide runnable code snippets with minimal setup instructions.
- **Security and Dependencies**: Do not exfiltrate secrets. Use minimal, pinned, widely-used libraries. Update manifests appropriately.

## Repository Structure and Key Components
- **Workspace Layout**: This is a Rust workspace with multiple crates in `crates/`, apps in `apps/`, documentation in `docs/`, and test components in `test-components/`. Key files include `Cargo.toml` (workspace config) and `mise.toml` (tool versions).
- **Core Crates**:
  - `core/`: Fundamental types and utilities.
  - `coordinator/`: Coordination logic, with variants like `coordinator_iroh/` and `coordinator_webrtc/`.
  - `router/`: Routing functionality.
  - `rpc/`: Remote procedure calls.
  - `identity/`: Identity management.
  - `substrate/`: Substrate-related code, including WASM bindings.
  - `bindings/`: WASM/WebAssembly interfaces.
- **Apps**: `roymctl/`: Command-line tool.
- **Other**: `observability/`, `control_plane/`, `client_gateway/`, etc., for monitoring, control, and client interactions.
- **Build and Test**: Use `cargo` for Rust builds. WASM targets (`wasm32-wasip1`) require special handling. Ensure `cargo fmt`, `clippy`, and `test` pass before completion.
