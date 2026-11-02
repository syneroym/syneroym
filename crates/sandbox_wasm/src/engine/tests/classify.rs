use wasmtime::{Config, Engine, Instance, Module, Store, Trap};

use crate::engine::{CallFailure, classify_call_failure};

/// Pins `classify_call_failure`'s taxonomy: the downcast
/// `Trap::OutOfFuel` and the two fuel substrings both
/// classify as `OutOfFuel`, memory/epoch substrings classify as
/// expected, and an unrelated error falls through to `Other`.
#[test]
fn classify_call_failure_matches_taxonomy() {
    let fuel_trap = wasmtime::Error::from(Trap::OutOfFuel);
    assert!(matches!(classify_call_failure(&fuel_trap), CallFailure::OutOfFuel));

    let fuel_string = wasmtime::Error::msg("all fuel consumed by WebAssembly code");
    assert!(matches!(classify_call_failure(&fuel_string), CallFailure::OutOfFuel));

    let fuel_string_alt = wasmtime::Error::msg("trap: out of fuel");
    assert!(matches!(classify_call_failure(&fuel_string_alt), CallFailure::OutOfFuel));

    let memory_string = wasmtime::Error::msg("instance exceeded its memory limits");
    assert!(matches!(classify_call_failure(&memory_string), CallFailure::MemoryFault));

    let memory_string_alt = wasmtime::Error::msg("host trap: MemoryFault");
    assert!(matches!(classify_call_failure(&memory_string_alt), CallFailure::MemoryFault));

    let epoch_string = wasmtime::Error::msg("epoch deadline reached while executing");
    assert!(matches!(classify_call_failure(&epoch_string), CallFailure::Deadline));

    let other = wasmtime::Error::msg("some unrelated wasm trap");
    assert!(matches!(classify_call_failure(&other), CallFailure::Other));
}

#[test]
fn classify_real_out_of_fuel_trap() {
    let mut config = Config::new();
    config.consume_fuel(true);
    let engine = Engine::new(&config).expect("build engine");
    let module = Module::new(&engine, r#"(module (func (export "spin") (loop (br 0))))"#)
        .expect("compile module");
    let mut store = Store::new(&engine, ());
    store.set_fuel(100).expect("set fuel");
    let instance = Instance::new(&mut store, &module, &[]).expect("instantiate");
    let spin = instance.get_typed_func::<(), ()>(&mut store, "spin").expect("get func");
    let err = spin.call(&mut store, ()).expect_err("should run out of fuel");
    assert!(
        matches!(classify_call_failure(&err), CallFailure::OutOfFuel),
        "expected OutOfFuel, got {err:?}"
    );
}

#[test]
fn classify_real_epoch_interrupt_trap() {
    let mut config = Config::new();
    config.epoch_interruption(true);
    let engine = Engine::new(&config).expect("build engine");
    let module = Module::new(&engine, r#"(module (func (export "spin") (loop (br 0))))"#)
        .expect("compile module");
    let mut store = Store::new(&engine, ());
    store.epoch_deadline_trap();
    store.set_epoch_deadline(1);
    engine.increment_epoch();
    let instance = Instance::new(&mut store, &module, &[]).expect("instantiate");
    let spin = instance.get_typed_func::<(), ()>(&mut store, "spin").expect("get func");
    let err = spin.call(&mut store, ()).expect_err("should trap on epoch");
    assert!(
        matches!(classify_call_failure(&err), CallFailure::Deadline),
        "expected Deadline, got {err:?}"
    );
}
