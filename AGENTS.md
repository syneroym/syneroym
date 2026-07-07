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
- **Mandatory Import Cleanup**: Before finishing any coding task, you MUST perform a dedicated final pass over the files you edited to clean up imports. You must strictly enforce the import rules (Types via standard `use`, Functions qualified by parent module) and proactively remove inline fully-qualified paths (lines with multiple `::`).
- **Prompt Clarity**: This is very important. If you don't understand what I am saying in the prompt clearly, it seems vague or confusing, please say so. Ask more questions, or explain how you would like me to rephrase the prompt. Or write out your understanding of the ask and request me to confirm before going ahead.
- **Interaction Style**: Respond concisely and directly. Use structured markdown for outputs, including code blocks, lists, and links to files/lines. Avoid verbose explanations unless requested.
- **Output Quality**: Ensure responses are accurate, idiomatic Rust code. Link to relevant files using workspace-relative paths (e.g., [src/main.rs](src/main.rs#L10)). Provide runnable code snippets with minimal setup instructions.
- **Security and Dependencies**: Do not exfiltrate secrets. Use minimal, pinned, widely-used libraries. Update manifests appropriately.
- **Git Commit Messages (Conventional Commits + 50/72 Rule)**: Prefix the subject line with a [Conventional Commits](https://www.conventionalcommits.org/) type (`feat`, `fix`, `docs`, `refactor`, `test`, `chore`, `perf`, `build`, `ci`, `style`, `revert`), plus an optional scope, e.g. `fix(data-layer): ...`. The description after the colon is lowercase, in the imperative mood, with no trailing period, and the whole subject line stays at or under 50 characters where practical. The second line must be empty. The body (lines 3+) must be wrapped at 72 characters and explain the what and why, not the how.
- **Pull Request CLA Checkbox**: `.github/workflows/cla-enforcer.yml` fails any PR whose description doesn't contain this exact line, checked and unmodified, on its own line: `- [x] I have read and agree to the [Syneroym CLA](https://github.com/syneroym/syneroym/blob/main/CLA.md).` `gh pr create --body` overrides `.github/PULL_REQUEST_TEMPLATE.md` entirely, so always include this line verbatim in any PR body you author.

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
- **Build and Test**: Use `cargo` for Rust builds. WASM targets (`wasm32-wasip2`) require special handling. Ensure `cargo +nightly fmt --all`, `clippy`, and `test` pass before completion.
