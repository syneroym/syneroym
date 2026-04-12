mod bindings {
    // The line below will be expanded as Rust code containing
    wit_bindgen::generate!({
        path: "wit/world.wit",
    });

    // In the lines below we use the generated `export!()` macro re-use and
    use super::IntroducerComponent;
    export!(IntroducerComponent);
}

struct IntroducerComponent;

impl bindings::exports::syneroym_test::introducer::greet::Guest for IntroducerComponent {
    fn greet(name: String) -> String {
        format!("hello, {}! Greetings from the introducer component", name)
    }
}
