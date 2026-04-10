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
