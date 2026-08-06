#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Scheduled-task test guest component.
//!
//! Exports a `scheduled-driver` interface with `tick`/`tick-count`, backed
//! by its own data-layer collection, so an end-to-end test can assert "ran
//! exactly once" from the test process rather than the guest's own state.

use bindings::{
    Guest,
    exports::syneroym_test::scheduled_test::scheduled_driver::Guest as ScheduledDriverGuest,
    syneroym::data_layer::store::{self, CollectionSchema, RecordWriteValue},
};

mod bindings {
    wit_bindgen::generate!({
        path: "wit",
        world: "scheduled-test",
        with: {
            "syneroym:data-layer/store@0.1.0": generate,
        },
    });

    use super::ScheduledTestComponent;
    export!(ScheduledTestComponent);
}

const COLLECTION: &str = "ticks";
const COUNTER_ID: &str = "counter";

struct ScheduledTestComponent;

impl Guest for ScheduledTestComponent {
    fn init() -> Result<(), String> {
        store::create_collection(&CollectionSchema { name: COLLECTION.to_string(), indexes: vec![] })
            .map_err(|e| format!("{e:?}"))
    }
}

/// The stored payload is `{"count": N}`, not a bare number: the host's
/// `payload` column is declared `JSON`, which SQLite gives NUMERIC affinity
/// (its declared-type rules only special-case `INT`/`CHAR`/`CLOB`/`TEXT`/
/// `BLOB`/`REAL`) -- a bare JSON number round-trips as valid JSON but is
/// silently stored as an actual `INTEGER`, which the host's own text read
/// path then rejects. A JSON object never triggers that coercion, the same
/// convention every other data-layer fixture already follows.
fn read_count() -> Result<u32, String> {
    let record = store::get(COLLECTION, COUNTER_ID).map_err(|e| format!("{e:?}"))?;
    Ok(record
        .and_then(|r| {
            let text = String::from_utf8_lossy(&r.payload);
            text.split(':').nth(1)?.trim().trim_end_matches('}').trim().parse::<u32>().ok()
        })
        .unwrap_or(0))
}

impl ScheduledDriverGuest for ScheduledTestComponent {
    fn tick() -> Result<(), String> {
        let next = read_count()? + 1;
        store::put(
            COLLECTION,
            &RecordWriteValue {
                id: COUNTER_ID.to_string(),
                payload: format!("{{\"count\": {next}}}").into_bytes(),
            },
        )
        .map_err(|e| format!("{e:?}"))
    }

    fn tick_count() -> Result<u32, String> {
        read_count()
    }
}
