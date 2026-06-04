//! Constants used exclusively for testing across the Syneroym workspace.

/// The interface name for the greeter test component.
pub const GREETER_INTERFACE_NAME: &str = "syneroym-test:greeter/greet@0.1.0";

/// Returns the workspace-relative path to the greeter component WASM module.
pub fn greeter_wasm_path() -> std::path::PathBuf {
    // The CARGO_MANIFEST_DIR for syneroym-core is `crates/core`
    // We navigate up to the workspace root and then to the test-components/greeter target
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../../test-components/greeter/target/wasm32-wasip2/release/syneroym_test_greeter.wasm",
    )
}
