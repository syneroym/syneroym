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
/// with a single `call-peer` method that originates a Universal Proxy call,
/// taking an extra `target-kind` ("service" or "dependency") argument
/// selecting which `call-target` variant it builds -- see
/// `test-components/proxy-test`).
pub fn proxy_test_wasm_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../../test-components/proxy-test/target/wasm32-wasip2/release/syneroym_test_proxy.wasm",
    )
}

/// Returns the workspace-relative path to the abac-test component WASM
/// module (imports `syneroym:data-layer/store` and `syneroym:app-config/
/// app-config`, exports `init` and `syneroym:data-layer/authorizer` --
/// Slice B4-fdae's stage-4 ABAC fixture, ADR-0017 §7. Behavior switches on
/// the deployed `app-config`'s `mode` key -- see
/// `test-components/abac-test`).
pub fn abac_test_wasm_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../../test-components/abac-test/target/wasm32-wasip2/release/syneroym_test_abac.wasm",
    )
}

/// The `scheduled-driver` interface name for the scheduled-test component.
pub const SCHEDULED_TEST_DRIVER_INTERFACE: &str =
    "syneroym-test:scheduled-test/scheduled-driver@0.1.0";

/// Returns the workspace-relative path to the scheduled-test component WASM
/// module (imports `syneroym:data-layer/store`, exports `init` and a
/// `scheduled-driver` interface -- `tick`/`tick-count` -- backed by its own
/// persisted counter, so a scheduled-task e2e test can assert "ran exactly
/// once" from the test process. See `test-components/scheduled-test`).
pub fn scheduled_test_wasm_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../../test-components/scheduled-test/target/wasm32-wasip2/release/\
         syneroym_test_scheduled.wasm",
    )
}

/// The `saga-driver` interface name for the saga-test component.
pub const SAGA_TEST_DRIVER_INTERFACE: &str = "syneroym-test:saga-test/saga-driver@0.1.0";

/// The `saga-participant` interface name for the saga-test component.
pub const SAGA_TEST_PARTICIPANT_INTERFACE: &str = "syneroym-test:saga-test/saga-participant@0.1.0";

/// Returns the workspace-relative path to the saga-test component WASM
/// module (imports `syneroym:data-layer/store` and `syneroym:proxy/saga`,
/// exports `init` plus `saga-driver` (`begin-workflow`/`add-step`/
/// `finish-workflow`) and `saga-participant` (`reserve`/`saga-undo-reserve`/
/// `ledger`) -- the same binary deployed twice, once under each role. See
/// `test-components/saga-test`).
pub fn saga_test_wasm_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../../test-components/saga-test/target/wasm32-wasip2/release/syneroym_test_saga.wasm",
    )
}

/// The `test-driver` interface name for the M06A A2 http-guest-test
/// component.
pub const HTTP_GUEST_TEST_DRIVER_INTERFACE: &str =
    "syneroym-test:http-guest-test/test-driver@0.1.0";

/// Returns the workspace-relative path to the http-guest-test component
/// WASM module (imports `syneroym:data-layer/store`, exports
/// `syneroym:http/incoming-handler#handle-request` and a `test-driver`
/// interface -- `last-request` -- for reading back the shape of the most
/// recently received request from a separate call. See
/// `test-components/http-guest-test`).
pub fn http_guest_test_wasm_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../../test-components/http-guest-test/target/wasm32-wasip2/release/\
         syneroym_test_http_guest.wasm",
    )
}

/// Returns the workspace-relative path to the websocket-guest-test component
/// WASM module.
pub fn websocket_guest_test_wasm_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../../test-components/websocket-guest-test/target/wasm32-wasip2/release/\
         syneroym_test_websocket_guest.wasm",
    )
}

/// Returns the workspace-relative path to the miniapp-demo1-wasm component
/// WASM module (M06A A4).
pub fn miniapp_demo1_wasm_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../../test-components/miniapp-demo1-wasm/target/wasm32-wasip2/release/\
         syneroym_test_miniapp_demo1_wasm.wasm",
    )
}

/// Default declared http_routes JSON for miniapp-demo1-wasm.
pub const MINIAPP_DEMO1_WASM_ROUTES_JSON: &str = r#"{
  "http_routes": [
    {"method": "GET", "path": "/comments", "target": "guest", "operation": "handle-request", "public": true},
    {"method": "GET", "path": "/api/comments", "target": "guest", "operation": "handle-request", "public": true},
    {"method": "POST", "path": "/api/comments", "target": "guest", "operation": "handle-request", "public": true},
    {"method": "GET", "path": "/api/files", "target": "guest", "operation": "handle-request", "public": true},
    {"method": "GET", "path": "/api/files/{filename}", "target": "stream", "operation": "accept-download", "protocol": "file-download"},
    {"method": "POST", "path": "/api/files", "target": "stream", "operation": "accept-upload", "protocol": "file-upload"},
    {"method": "GET", "path": "/api/events", "target": "messaging", "operation": "subscribe-sse", "topic": "comment-updates"},
    {"method": "GET", "path": "/ws", "target": "websocket", "operation": "handle-upgrade", "topic": "comment-updates", "public": true}
  ]
}"#;
