#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Saga compensation test guest component (M05B Slice B4).
//!
//! One component, deployed twice under two service ids: once as the
//! orchestrating driver (`saga-driver`) and once as the participant that
//! records what was reserved and what was undone, in that order
//! (`saga-participant`). Nothing is declared anywhere -- the participant's
//! `saga-undo-reserve` export is the whole of its participation (§0.4a).

use bindings::{
    Guest,
    exports::syneroym_test::saga_test::{
        saga_driver::Guest as SagaDriverGuest, saga_participant::Guest as SagaParticipantGuest,
    },
    syneroym::{
        data_layer::store::{self, CollectionSchema, RecordWriteValue},
        proxy::saga,
    },
};

mod bindings {
    wit_bindgen::generate!({
        path: "wit",
        world: "saga-test",
        with: {
            "syneroym:data-layer/store@0.1.0": generate,
            "syneroym:proxy/proxy@0.1.0": generate,
            "syneroym:proxy/saga@0.1.0": generate,
        },
    });

    use super::SagaTestComponent;
    export!(SagaTestComponent);
}

/// The fully-qualified interface name the driver addresses on its peer.
/// Both halves of this fixture are the same binary, so the driver knows
/// its own participant export's name at compile time -- nothing is
/// resolved or declared at runtime.
const PARTICIPANT_INTERFACE: &str = "syneroym-test:saga-test/saga-participant@0.1.0";

const LEDGER_COLLECTION: &str = "ledger";
const LEDGER_ID: &str = "log";

struct SagaTestComponent;

impl Guest for SagaTestComponent {
    fn init() -> Result<(), String> {
        store::create_collection(&CollectionSchema {
            name: LEDGER_COLLECTION.to_string(),
            indexes: vec![],
        })
        .map_err(|e| format!("{e:?}"))
    }
}

/// The stored payload is a JSON *object* (`{"log": "a,b,c"}`), never a bare
/// scalar: the host's `payload` column is declared `JSON`, which SQLite
/// gives NUMERIC affinity, so a bare value can be silently coerced and the
/// host's own text read then fails (the same lesson `scheduled-test`
/// already recorded). Parsed and built by hand, matching `data-layer-test`'s
/// own no-`serde_json` convention.
fn read_ledger() -> Result<Vec<String>, String> {
    let record = store::get(LEDGER_COLLECTION, LEDGER_ID).map_err(|e| format!("{e:?}"))?;
    Ok(record
        .and_then(|r| {
            let text = String::from_utf8_lossy(&r.payload).to_string();
            let rest = text.split("\"log\":\"").nth(1)?;
            let joined = rest.strip_suffix("\"}")?;
            if joined.is_empty() {
                Some(Vec::new())
            } else {
                Some(joined.split(',').map(str::to_string).collect())
            }
        })
        .unwrap_or_default())
}

fn append_ledger(entry: &str) -> Result<(), String> {
    let mut log = read_ledger()?;
    log.push(entry.to_string());
    let joined = log.join(",");
    store::put(
        LEDGER_COLLECTION,
        &RecordWriteValue {
            id: LEDGER_ID.to_string(),
            payload: format!("{{\"log\":\"{joined}\"}}").into_bytes(),
        },
    )
    .map_err(|e| format!("{e:?}"))
}

impl SagaParticipantGuest for SagaTestComponent {
    fn reserve(item: String) -> Result<String, String> {
        append_ledger(&format!("reserve:{item}"))?;
        Ok(format!("reserved-{item}"))
    }

    /// May be called for an operation that never happened (§0.5): the
    /// intent is written before the forward call, so a crashed substrate
    /// can compensate a `reserve` whose result never came back. Recording
    /// `undo:<item>` regardless is what makes the ledger's *order* the
    /// assertion, not just its final membership.
    fn saga_undo_reserve(item: String, forward_result: Option<String>) -> Result<(), String> {
        let _ = forward_result;
        append_ledger(&format!("undo:{item}"))
    }

    fn ledger() -> Result<String, String> {
        Ok(read_ledger()?.join(", "))
    }
}

impl SagaDriverGuest for SagaTestComponent {
    fn begin_workflow(deadline_secs: u64) -> Result<String, String> {
        saga::begin("saga-test-workflow", Some(deadline_secs)).map_err(|e| format!("{e:?}"))
    }

    fn add_step(saga_id: String, peer: String, item: String) -> Result<String, String> {
        let params = format!("{{\"item\":\"{item}\"}}");
        saga::step(&saga_id, &saga::CallTarget::Service(peer), PARTICIPANT_INTERFACE, "reserve", &params, None)
            .map_err(|e| format!("{e:?}"))
    }

    fn finish_workflow(saga_id: String, outcome: String) -> Result<(), String> {
        match outcome.as_str() {
            "commit" => saga::commit(&saga_id).map_err(|e| format!("{e:?}")),
            "compensate" => saga::compensate(&saga_id).map_err(|e| format!("{e:?}")),
            other => Err(format!("unknown finish-workflow outcome '{other}'")),
        }
    }
}
