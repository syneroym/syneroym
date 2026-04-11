#[allow(warnings)]
#[rustfmt::skip]

wit_bindgen::generate!({
    world: "host-environment",
});

use exports::syneroym::host::app::Guest;

struct Component;

impl Guest for Component {
    fn run() -> String {
        "hello from introduce component".to_string()
    }
}

export!(Component);
