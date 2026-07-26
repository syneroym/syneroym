#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Stage-4 ABAC (ADR-0017 §7) test fixture. Switchable behavior via
//! `app-config`'s `mode` key, so one compiled component covers every
//! Failure/Security matrix row 7-9 case plus the read-only/lookup escape
//! hatch, instead of one fixture per scenario.

use bindings::{
    Guest,
    exports::syneroym::data_layer::authorizer::{
        AuthContext, CandidateRow, Guest as AuthorizerGuest, RowDecision,
    },
    syneroym::{
        app_config::app_config,
        data_layer::store::{self, CollectionSchema, RecordWriteValue},
    },
};

mod bindings {
    wit_bindgen::generate!({
        path: "wit",
        world: "abac-test",
        with: {
            "syneroym:data-layer/store@0.1.0": generate,
            "syneroym:data-layer/authorizer@0.1.0": generate,
            "syneroym:app-config/app-config@0.1.0": generate,
        },
    });

    use super::AbacTestComponent;
    export!(AbacTestComponent);
}

struct AbacTestComponent;

/// `app-config`'s `mode` key selects this fixture's behavior; absent
/// defaults to `allow_all` (the baseline -- restrict-only, never widens).
fn read_mode() -> String {
    app_config::get("mode").ok().flatten().unwrap_or_else(|| "allow_all".to_string())
}

impl Guest for AbacTestComponent {
    fn init() -> Result<(), String> {
        store::create_collection(&CollectionSchema { name: "profiles".to_string(), indexes: vec![] })
            .map_err(|e| format!("{e:?}"))?;
        store::create_collection(&CollectionSchema {
            name: "lookup_targets".to_string(),
            indexes: vec![],
        })
        .map_err(|e| format!("{e:?}"))?;
        // Non-empty payloads (`classification`/`ssn`/`note`) so the same two
        // rows also serve `stage4_cls_mask_unions_with_the_after_step_
        // redact_set` (review finding B4-08): `classification` is
        // CLS-masked by `lookup_targets`'s own policy permission, `ssn` is
        // redacted by the `authorize-rows` after-step in `redact` mode, and
        // `note` must survive both to prove the union subtracts, never
        // adds.
        let lookup_row_payload = serde_json::json!({
            "classification": "secret",
            "ssn": "999-99-9999",
            "note": "public-lookup-row"
        })
        .to_string()
        .into_bytes();
        store::put(
            "lookup_targets",
            &RecordWriteValue { id: "target-1".to_string(), payload: lookup_row_payload.clone() },
        )
        .map_err(|e| format!("{e:?}"))?;
        // A second row so the recursion-bound mode below can assert it saw
        // *both*, unfiltered -- not just that a single-row `get` succeeded.
        store::put(
            "lookup_targets",
            &RecordWriteValue { id: "target-2".to_string(), payload: lookup_row_payload },
        )
        .map_err(|e| format!("{e:?}"))
    }
}

impl AuthorizerGuest for AbacTestComponent {
    fn authorize_rows(
        _ctx: AuthContext,
        rows: Vec<CandidateRow>,
    ) -> Result<Vec<RowDecision>, String> {
        match read_mode().as_str() {
            // Row 8: an unbounded loop burns fuel/epoch ticks until the
            // host's budget (`abac_max_instructions`/`abac_epoch_timeout_secs`)
            // traps the call -- proving the after-step cannot run unmetered.
            "spin" => loop {},
            // Row 7's structural guard: one fewer decision than rows, which
            // `apply_stage4` must treat as a whole-batch deny (arity
            // mismatch), never as "the missing row is implicitly denied."
            "bad_arity" => Ok(rows.into_iter().skip(1).map(|_| RowDecision::Allow).collect()),
            // The motivating case: deny a row whose payload marks it
            // `classification: "secret"`.
            "deny_by_field" => Ok(rows
                .into_iter()
                .map(|row| {
                    let is_secret = serde_json::from_slice::<serde_json::Value>(&row.payload)
                        .ok()
                        .and_then(|v| {
                            v.get("classification")
                                .and_then(|c| c.as_str().map(|s| s == "secret"))
                        })
                        .unwrap_or(false);
                    if is_secret { RowDecision::Deny } else { RowDecision::Allow }
                })
                .collect()),
            // Row 9: redact a field on every row.
            "redact" => {
                Ok(rows.into_iter().map(|_| RowDecision::Redact(vec!["ssn".to_string()])).collect())
            }
            // Read-only enforcement: this after-step instance's `HostState.
            // read_only` must hard-deny the write regardless of what the
            // guest decides here -- the host test asserts nothing was
            // actually persisted, not what this call returns.
            "write_attempt" => {
                let _ = store::put(
                    "profiles",
                    &RecordWriteValue {
                        id: "guest-write-attempt".to_string(),
                        payload: b"{}".to_vec(),
                    },
                );
                Ok(rows.into_iter().map(|_| RowDecision::Allow).collect())
            }
            // ADR-0017 §7's read-only lookup escape hatch: reads this
            // service's own data unfiltered (no `QueryAuth` reaches a
            // `LocalReadOnly` caller) to decide every row in the batch.
            "lookup" => {
                let found = store::get("lookup_targets", "target-1").ok().flatten().is_some();
                let decision = if found { RowDecision::Allow } else { RowDecision::Deny };
                Ok(rows.into_iter().map(|_| match &decision {
                    RowDecision::Allow => RowDecision::Allow,
                    _ => RowDecision::Deny,
                })
                .collect())
            }
            // D-B4-4's recursion bound. `lookup_targets`'s own `view`
            // permission is stage-4-gated *and* unconditionally public
            // (`paths: []`), so it matches for *any* caller identity,
            // including this after-step's own synthetic `LocalReadOnly`
            // one -- guaranteeing the nested query below is non-empty and
            // would therefore actually re-enter this same function (and
            // recurse without bound, since every entry runs the identical
            // logic) if the `LocalReadOnly` sieve exemption were ever
            // narrowed. Today it completes fast and sees both seeded rows
            // unfiltered, because the exemption means this nested read
            // never consults a sieve -- carries no `QueryAuth` -- at all.
            "nested_query_recurses_if_unexempted" => {
                let opts = store::QueryOptions { filter: None, limit: None, cursor: None };
                let saw_both = store::query("lookup_targets", &opts)
                    .map(|r| r.records.len() == 2)
                    .unwrap_or(false);
                Ok(rows
                    .into_iter()
                    .map(|_| if saw_both { RowDecision::Allow } else { RowDecision::Deny })
                    .collect())
            }
            // Baseline: cannot widen access beyond what the sieve already
            // admitted -- every row the sieve excluded never reaches this
            // function at all, so `allow`-everything here is still
            // restrict-only by construction.
            _ => Ok(rows.into_iter().map(|_| RowDecision::Allow).collect()),
        }
    }
}
