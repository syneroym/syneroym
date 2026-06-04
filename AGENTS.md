# Project: Syneroym

## General Instructions
- Focus religiously on these code aspects: Simplicity, performance, readability, testability, overall beauty, robustness, scalability, reliability.
- Follow standard Rust `clippy` guidelines. Before completion, confirm that `cargo fmt --all`, `cargo clippy --workspace --all-targets --all-features`, and `cargo test --workspace` succeed.
- Try to use the latest stable versions of any library added.
- Have extensive integration and end to end tests for end user facing interfaces.
- Have solid unit tests for internal code if it is complex and delicate, even if it is not user facing.
- If new tools are needed in the build pipeline, add them to `mise.toml` too, so other dev environments easily get it.
- Do not commit code changes, and also do not add code changes to git index.
- Files with `scratch-notes` in the name, as well as the `docs/archive` folder, contain temporary or archived ideas and should be ignored by the agent. They might not contain reliable information.

## Project & Rust Specifics
- Given the presence of WASM component configurations (`wasm32-wasip2`), maintain clean `wit` file boundaries and consider cross-compilation constraints.
- Emphasize idiomatic Rust formatting, leveraging the language's strong typing to guarantee correctness. Ensure imports are idiomatic: (1) for types (structs, enums, traits), use standard `use` statements at the top of the file; (2) for function calls, import the parent module at the top of the file and call the function qualified by that module; (3) for name conflicts, either import the parent module and qualify, or alias the imports.
- Ensure any added dependencies reflect widely supported community standards.

## Functionality, Architecture Documents
- The [vision doc](docs/VISION.md) contains the vision for Syneroym
- The [requirements](docs/requirements.md) contains high-level function requirements
- The [architecture](docs/architecture.md) contains high-level architecture
- The above docs are starting points for the implementation. It is likely that during implementation we deviate and improvise from those, and later get them in sync.

## AI Agent Guidelines
- **Prompt Clarity**: This is very important. If you don't understand what I am saying in the prompt clearly, it seems vague or confusing, please say so. Ask more questions, or explain how you would like me to rephrase the prompt. Or write out your understanding of the ask and request me to confirm before going ahead.
- **Interaction Style**: Respond concisely and directly. Use structured markdown for outputs, including code blocks, lists, and links to files/lines. Avoid verbose explanations unless requested.
- **Output Quality**: Ensure responses are accurate, idiomatic Rust code. Link to relevant files using workspace-relative paths (e.g., [src/main.rs](src/main.rs#L10)). Provide runnable code snippets with minimal setup instructions.
- **Security and Dependencies**: Do not exfiltrate secrets. Use minimal, pinned, widely-used libraries. Update manifests appropriately.
- **Git Commit Messages (50/72 Rule)**: When auto-generating or suggesting git commit messages, strictly enforce the 50/72 rule. The subject line (first line) must be capitalized, in the imperative mood, and no more than 50 characters, with no trailing period. The second line must be empty. The body (lines 3+) must be wrapped at 72 characters and explain the what and why, not the how.

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
- **Build and Test**: Use `cargo` for Rust builds. WASM targets (`wasm32-wasip2`) require special handling. Ensure `cargo fmt`, `clippy`, and `test` pass before completion.
