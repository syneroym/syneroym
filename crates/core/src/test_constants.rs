//! Constants used exclusively for testing across the Syneroym workspace.

/// The interface name for the greeter test component.
use std::path::PathBuf;
pub const GREETER_INTERFACE_NAME: &str = "syneroym-test:greeter/greet@0.1.0";

/// Returns the workspace-relative path to the greeter component WASM module.
pub fn greeter_wasm_path() -> PathBuf {
    // The CARGO_MANIFEST_DIR for syneroym-core is `crates/core`
    // We navigate up to the workspace root and then to the test-components/greeter
    // target
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../../test-components/greeter/target/wasm32-wasip2/release/syneroym_test_greeter.wasm",
    )
}

/// Returns the workspace-relative path to the data-layer-test component WASM
/// module (imports `syneroym:data-layer/store`, exports `init`/`migrate` and
/// CRUD test-driver functions -- see `test-components/data-layer-test`).
pub fn data_layer_test_wasm_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../../test-components/data-layer-test/target/wasm32-wasip2/release/\
         syneroym_test_data_layer.wasm",
    )
}

/// Returns the workspace-relative path to the messaging-pubsub-test
/// component WASM module (imports `syneroym:messaging/host-api`, exports
/// `syneroym:messaging/guest-api::handle-message` and a `test-driver`
/// interface for subscribing/publishing/reading back received messages --
/// see `test-components/messaging-pubsub-test`).
pub fn messaging_pubsub_test_wasm_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../../test-components/messaging-pubsub-test/target/wasm32-wasip2/release/\
         syneroym_test_messaging_pubsub.wasm",
    )
}

/// The `test-driver` interface name for the M3B Slice 6B stream-test
/// component.
pub const STREAM_TEST_DRIVER_INTERFACE: &str = "syneroym-test:stream-test/test-driver@0.1.0";

/// Returns the workspace-relative path to the stream-test component WASM
/// module (imports `syneroym:messaging/host-api`, exports
/// `syneroym:messaging/stream-types`/`guest-api` -- `handle-stream-request`/
/// `accept-stream-upload` -- and a `test-driver` interface for
/// registering/reading back uploaded content -- see
/// `test-components/stream-test`).
pub fn stream_test_wasm_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../../test-components/stream-test/target/wasm32-wasip2/release/syneroym_test_stream.wasm",
    )
}

/// The `test-driver` interface name for the M04A Slice A1 proxy-test
/// component.
pub const PROXY_TEST_DRIVER_INTERFACE: &str = "syneroym-test:proxy-test/test-driver@0.1.0";

/// Returns the workspace-relative path to the proxy-test component WASM
/// module (imports `syneroym:proxy/proxy`, exports a `test-driver` interface
/// with a single `call-peer` method that originates a Universal Proxy call --
/// see `test-components/proxy-test`).
pub fn proxy_test_wasm_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../../test-components/proxy-test/target/wasm32-wasip2/release/syneroym_test_proxy.wasm",
    )
}
