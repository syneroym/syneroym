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

impl bindings::exports::syneroym_test::greeter::greet::Guest for GreeterComponent {
    fn greet(name: String) -> String {
        format!("hello, {}! Greetings from the greeter component", name)
    }
}
