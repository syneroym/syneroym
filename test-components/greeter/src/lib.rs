#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Greeter test guest component
//!
//! Standard mock WASM component utilizing generated bindings to respond to greeting requests.

use bindings::{exports::syneroym_test::greeter::greet::Guest, syneroym::host::context};
mod bindings {
    // The line below will be expanded as Rust code containing
    wit_bindgen::generate!({
        path: "wit",
        world: "greeter",
        with: {
            "syneroym:host/context": generate,
        },
    });

    // In the lines below we use the generated `export!()` macro re-use and
    use super::GreeterComponent;
    export!(GreeterComponent);
}

struct GreeterComponent;

impl Guest for GreeterComponent {
    fn greet(name: String) -> String {
        // `get-test-context` echoes back this instance's own `component_id`
        // (its deployed `service_id`) -- the one thing that differs between
        // two `replicas` members of an otherwise byte-identical deploy, and
        // otherwise unobservable from a caller resolving through the
        // dependency-binding host capability (M05A Slice A5e reference
        // scenario, step 5).
        let ctx = context::get_test_context("");
        format!("Hello, {name}! Greetings from greeter::greet::greet ({ctx})")
    }
}
