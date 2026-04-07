#[allow(warnings)]
mod bindings;

use bindings::exports::component::introduce::introduce::{Guest, Person};

struct Component;

impl Guest for Component {
    fn greet(arg: Person) -> String {
        format!("hello {}, you are interested in {} topics", arg.name, arg.interests.len())
    }
}

bindings::export!(Component with_types_in bindings);
