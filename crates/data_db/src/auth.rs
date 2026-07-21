//! Per-request FDAE authorization context threaded into the read/delete
//! paths (ADR-0017, M04B Slice B2 Phase 2).

use serde_json::Value;
use syneroym_fdae::Policy;
use syneroym_ucan::SessionContext;

use crate::host_store::DataLayerError;

/// Per-request policy + caller context threaded into the read/delete paths.
/// `None` at a call site preserves today's unfiltered behavior
/// (policy-absent services, native dispatch, benches, tests).
#[derive(Debug)]
pub struct QueryAuth<'a> {
    pub policy: &'a Policy,
    pub session: &'a SessionContext,
    pub service_id: &'a str,
}

/// A read result plus the CLS field-mask the host must apply as its final
/// projection (host-side, Phase 3 -- this crate never strips fields itself,
/// per the stage-4 ordering contract). `masked_fields` is always empty on
/// the policy-absent path.
#[derive(Debug)]
pub struct ReadOutcome<T> {
    pub value: T,
    pub masked_fields: Vec<String>,
}

/// Removes each top-level key in `masked` from a JSON-object payload
/// (host-side CLS projection, Phase 3). `masked_fields` are always flat
/// top-level keys -- `compile_cls` copies `fields.deny` verbatim, no path
/// parsing -- so a top-level `Map::remove` is sufficient.
///
/// Fail-closed: a payload that won't parse as a JSON object while a
/// non-empty mask applies is an error (returning it unstripped would leak
/// the masked field), never a pass-through. An empty mask returns the
/// payload untouched without parsing it.
pub fn strip_masked_fields(payload: Vec<u8>, masked: &[String]) -> Result<Vec<u8>, DataLayerError> {
    if masked.is_empty() {
        return Ok(payload);
    }
    let mut value: Value = serde_json::from_slice(&payload)
        .map_err(|e| DataLayerError::SchemaViolation(format!("payload is not valid JSON: {e}")))?;
    let Value::Object(map) = &mut value else {
        return Err(DataLayerError::SchemaViolation(
            "payload is not a JSON object; cannot apply CLS field mask".into(),
        ));
    };
    for field in masked {
        map.remove(field);
    }
    serde_json::to_vec(&value)
        .map_err(|e| DataLayerError::Internal(format!("failed to re-serialize payload: {e}")))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn strips_a_named_top_level_key() {
        let payload = br#"{"name":"alice","ssn":"123-45-6789"}"#.to_vec();
        let stripped = strip_masked_fields(payload, &["ssn".to_string()]).unwrap();
        let value: Value = serde_json::from_slice(&stripped).unwrap();
        assert_eq!(value, serde_json::json!({"name": "alice"}));
    }

    #[test]
    fn leaves_sibling_fields_untouched() {
        let payload = br#"{"name":"alice","age":30,"ssn":"123-45-6789"}"#.to_vec();
        let stripped = strip_masked_fields(payload, &["ssn".to_string()]).unwrap();
        let value: Value = serde_json::from_slice(&stripped).unwrap();
        assert_eq!(value, serde_json::json!({"name": "alice", "age": 30}));
    }

    #[test]
    fn empty_mask_returns_payload_untouched_without_parsing() {
        let payload = b"not even json".to_vec();
        let stripped = strip_masked_fields(payload.clone(), &[]).unwrap();
        assert_eq!(stripped, payload);
    }

    #[test]
    fn non_json_payload_with_non_empty_mask_fails_closed() {
        let payload = b"not json".to_vec();
        let err = strip_masked_fields(payload, &["ssn".to_string()]).unwrap_err();
        assert!(matches!(err, DataLayerError::SchemaViolation(_)));
    }

    #[test]
    fn mask_naming_an_absent_key_is_a_no_op_success() {
        let payload = br#"{"name":"alice"}"#.to_vec();
        let stripped = strip_masked_fields(payload, &["ssn".to_string()]).unwrap();
        let value: Value = serde_json::from_slice(&stripped).unwrap();
        assert_eq!(value, serde_json::json!({"name": "alice"}));
    }
}
