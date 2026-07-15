#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Universal Proxy guest test component (M04A Slice A1).
//!
//! Exercises `syneroym:proxy/proxy::call` from guest code end to end -- the
//! only way a real component can originate a cross-service call, as opposed
//! to a Rust-level `ProxyRouter::invoke` unit test.

use bindings::{
    exports::syneroym_test::proxy_test::test_driver::Guest as TestDriverGuest, syneroym::proxy::proxy,
};

mod bindings {
    wit_bindgen::generate!({
        path: "wit",
        world: "proxy-test",
        with: {
            "syneroym:proxy/proxy@0.1.0": generate,
        },
    });

    use super::ProxyTestComponent;
    export!(ProxyTestComponent);
}

struct ProxyTestComponent;

impl TestDriverGuest for ProxyTestComponent {
    fn call_peer(
        service: String,
        interface: String,
        method: String,
        params: String,
    ) -> Result<String, String> {
        proxy::call(&service, &interface, &method, &params, None).map_err(|e| format!("{e:?}"))
    }
}
