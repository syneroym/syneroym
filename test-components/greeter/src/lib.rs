#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Greeter test guest component
//!
//! Standard mock WASM component utilizing generated bindings to respond to greeting requests.

use bindings::exports::syneroym_test::greeter::greet::Guest;
mod bindings {
    // The line below will be expanded as Rust code containing
    wit_bindgen::generate!({
        path: "wit/world.wit",
    });

    // In the lines below we use the generated `export!()` macro re-use and
    use super::GreeterComponent;
    export!(GreeterComponent);
}

struct GreeterComponent;

impl Guest for GreeterComponent {
    fn greet(name: String) -> String {
        format!("Hello, {}! Greetings from greeter::greet::greet", name)
    }
}
