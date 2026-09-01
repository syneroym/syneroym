use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Digest;
use syneroym_identity::substrate;

pub const ENVELOPE_VERSION: u32 = 1;
pub const MAX_PAYLOAD_BYTES: usize = 64 * 1024;
pub const MAX_PAYLOAD_DEPTH: usize = 32;
pub const MAX_RECORD_TYPE_LEN: usize = 64;
pub const MAX_SUBJECT_LEN: usize = 256;
pub const RECORD_ID_PREFIX: &str = "rec_";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct RecordDraft {
    pub version: u32,
    #[serde(alias = "record_type")]
    pub record_type: String,
    pub subject: String,
    pub payload: Value,
    #[serde(alias = "expires_at_secs")]
    pub expires_at_secs: Option<u64>,
    pub supersedes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DraftError {
    #[error("a record with version 0 does not exist")]
    ZeroVersion,
    #[error("record type '{0}' is not lowercase ascii letters, digits and '-', 1..={max} bytes", max = MAX_RECORD_TYPE_LEN)]
    RecordType(String),
    #[error("subject is {0} bytes, over the {max}-byte maximum", max = MAX_SUBJECT_LEN)]
    Subject(usize),
    #[error("payload is not a JSON object")]
    PayloadNotObject,
    #[error("payload is {bytes} bytes, over the {max}-byte maximum", max = MAX_PAYLOAD_BYTES)]
    PayloadTooLarge { bytes: usize, max: usize },
    #[error("payload nests {depth} deep, over the {max} maximum", max = MAX_PAYLOAD_DEPTH)]
    PayloadTooDeep { depth: usize, max: usize },
    #[error("payload field '{0}' holds a number that is not an integer")]
    PayloadNonIntegerNumber(String),
    #[error("supersedes '{0}' is not a record id")]
    Supersedes(String),
    #[error("expires_at_secs {expires_at_secs} is already past (now {now_secs})")]
    ExpiryInPast { expires_at_secs: u64, now_secs: u64 },
}

impl RecordDraft {
    pub fn validate(&self, now_secs: u64) -> Result<(), DraftError> {
        if self.version == 0 {
            return Err(DraftError::ZeroVersion);
        }

        if self.record_type.is_empty()
            || self.record_type.len() > MAX_RECORD_TYPE_LEN
            || !self.record_type.bytes().all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'-'))
        {
            return Err(DraftError::RecordType(self.record_type.clone()));
        }

        if self.subject.len() > MAX_SUBJECT_LEN {
            return Err(DraftError::Subject(self.subject.len()));
        }

        match &self.payload {
            Value::Object(_) => (),
            _ => return Err(DraftError::PayloadNotObject),
        }

        fn walk(val: &Value, depth: usize, path: &str) -> Result<(), DraftError> {
            if depth > MAX_PAYLOAD_DEPTH {
                return Err(DraftError::PayloadTooDeep { depth, max: MAX_PAYLOAD_DEPTH });
            }

            match val {
                Value::Number(n) => {
                    if !n.is_i64() && !n.is_u64() {
                        return Err(DraftError::PayloadNonIntegerNumber(path.to_string()));
                    }
                }
                Value::Object(map) => {
                    for (k, v) in map {
                        let field_path =
                            if path.is_empty() { k.clone() } else { format!("{path}.{k}") };
                        walk(v, depth + 1, &field_path)?;
                    }
                }
                Value::Array(arr) => {
                    for (i, v) in arr.iter().enumerate() {
                        let elem_path = format!("{path}[{i}]");
                        walk(v, depth + 1, &elem_path)?;
                    }
                }
                _ => (),
            }

            Ok(())
        }

        walk(&self.payload, 1, "")?;

        let canonical_payload = substrate::canonicalize_json_value(&self.payload);
        let bytes =
            serde_json::to_vec(&canonical_payload).map_err(|_| DraftError::PayloadNotObject)?.len();
        if bytes > MAX_PAYLOAD_BYTES {
            return Err(DraftError::PayloadTooLarge { bytes, max: MAX_PAYLOAD_BYTES });
        }

        if let Some(s) = &self.supersedes {
            if !s.starts_with(RECORD_ID_PREFIX) {
                return Err(DraftError::Supersedes(s.clone()));
            }
            let rest = &s[RECORD_ID_PREFIX.len()..];
            let decoded =
                z32::decode(rest.as_bytes()).map_err(|_| DraftError::Supersedes(s.clone()))?;
            if decoded.len() != 32 {
                return Err(DraftError::Supersedes(s.clone()));
            }
        }

        if let Some(e) = self.expires_at_secs
            && e <= now_secs
        {
            return Err(DraftError::ExpiryInPast { expires_at_secs: e, now_secs });
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    pub envelope_version: u32,
    pub version: u32,
    pub record_type: String,
    pub issuer: String,
    pub subject: String,
    pub issued_at_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<String>,
    pub payload: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegation: Option<String>,
    pub signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EnvelopeError {
    #[error("envelope json: {0}")]
    Json(String),
    #[error("signature already attached")]
    AlreadySigned,
}

impl Envelope {
    pub fn unsigned(
        draft: RecordDraft,
        issuer: String,
        delegation: Option<String>,
        issued_at_secs: u64,
    ) -> Result<(Self, Vec<u8>), DraftError> {
        draft.validate(issued_at_secs)?;
        let env = Envelope {
            envelope_version: ENVELOPE_VERSION,
            version: draft.version,
            record_type: draft.record_type,
            issuer,
            subject: draft.subject,
            issued_at_secs,
            expires_at_secs: draft.expires_at_secs,
            supersedes: draft.supersedes,
            payload: substrate::canonicalize_json_value(&draft.payload),
            delegation,
            signature: String::new(),
        };
        let bytes = env.signing_bytes().map_err(|_| DraftError::PayloadNotObject)?;
        Ok((env, bytes))
    }

    pub fn attach_signature(&mut self, signature_z32: String) -> Result<(), EnvelopeError> {
        if !self.signature.is_empty() {
            return Err(EnvelopeError::AlreadySigned);
        }
        self.signature = signature_z32;
        Ok(())
    }

    pub fn signing_bytes(&self) -> Result<Vec<u8>, EnvelopeError> {
        let mut unsigned = self.clone();
        unsigned.signature = String::new();
        let value =
            serde_json::to_value(&unsigned).map_err(|e| EnvelopeError::Json(e.to_string()))?;
        let canonical = substrate::canonicalize_json_value(&value);
        serde_json::to_vec(&canonical).map_err(|e| EnvelopeError::Json(e.to_string()))
    }

    pub fn record_id(&self) -> Result<String, EnvelopeError> {
        let value = serde_json::to_value(self).map_err(|e| EnvelopeError::Json(e.to_string()))?;
        let canonical = substrate::canonicalize_json_value(&value);
        let bytes =
            serde_json::to_vec(&canonical).map_err(|e| EnvelopeError::Json(e.to_string()))?;
        let digest = sha2::Sha256::digest(&bytes);
        Ok(format!("{RECORD_ID_PREFIX}{}", z32::encode(&digest)))
    }

    pub fn to_json(&self) -> Result<String, EnvelopeError> {
        serde_json::to_string(self).map_err(|e| EnvelopeError::Json(e.to_string()))
    }

    pub fn from_json(s: &str) -> Result<Self, EnvelopeError> {
        serde_json::from_str(s).map_err(|e| EnvelopeError::Json(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn sample_draft() -> RecordDraft {
        RecordDraft {
            version: 1,
            record_type: "listing".to_string(),
            subject: "sub_123".to_string(),
            payload: json!({"title": "test item", "price": 100}),
            expires_at_secs: None,
            supersedes: None,
        }
    }

    #[test]
    fn draft_zero_version_refused() {
        let mut d = sample_draft();
        d.version = 0;
        assert_eq!(d.validate(100), Err(DraftError::ZeroVersion));
    }

    #[test]
    fn draft_invalid_record_type_refused() {
        let mut d = sample_draft();
        d.record_type = "Invalid_Type!".to_string();
        assert_eq!(d.validate(100), Err(DraftError::RecordType("Invalid_Type!".to_string())));

        d.record_type = "".to_string();
        assert_eq!(d.validate(100), Err(DraftError::RecordType("".to_string())));
    }

    #[test]
    fn draft_subject_too_long_refused() {
        let mut d = sample_draft();
        d.subject = "a".repeat(MAX_SUBJECT_LEN + 1);
        assert_eq!(d.validate(100), Err(DraftError::Subject(MAX_SUBJECT_LEN + 1)));
    }

    #[test]
    fn draft_payload_not_object_refused() {
        let mut d = sample_draft();
        d.payload = json!(["array", "payload"]);
        assert_eq!(d.validate(100), Err(DraftError::PayloadNotObject));
    }

    #[test]
    fn draft_float_in_payload_refused() {
        let mut d = sample_draft();
        d.payload = json!({"price": 10.5});
        assert_eq!(d.validate(100), Err(DraftError::PayloadNonIntegerNumber("price".to_string())));
    }

    #[test]
    fn draft_integer_valued_float_in_payload_refused() {
        let mut d = sample_draft();
        d.payload = serde_json::from_str(r#"{"price": 1.0}"#).unwrap();
        assert_eq!(d.validate(100), Err(DraftError::PayloadNonIntegerNumber("price".to_string())));
    }

    #[test]
    fn draft_payload_too_deep_refused() {
        let mut d = sample_draft();
        let mut deep = json!({"val": 1});
        for _ in 0..MAX_PAYLOAD_DEPTH + 1 {
            deep = json!({"nested": deep});
        }
        d.payload = deep;
        assert!(matches!(d.validate(100), Err(DraftError::PayloadTooDeep { .. })));
    }

    #[test]
    fn draft_invalid_supersedes_refused() {
        let mut d = sample_draft();
        d.supersedes = Some("invalid_prefix".to_string());
        assert_eq!(d.validate(100), Err(DraftError::Supersedes("invalid_prefix".to_string())));

        d.supersedes = Some(format!("{RECORD_ID_PREFIX}short"));
        assert!(matches!(d.validate(100), Err(DraftError::Supersedes(_))));
    }

    #[test]
    fn draft_expiry_in_past_refused() {
        let mut d = sample_draft();
        d.expires_at_secs = Some(100);
        assert_eq!(
            d.validate(100),
            Err(DraftError::ExpiryInPast { expires_at_secs: 100, now_secs: 100 })
        );
    }

    #[test]
    fn record_id_is_stable_across_serialize_deserialize() {
        let draft = sample_draft();
        let (env, _) = Envelope::unsigned(draft, "did:key:z6M123".to_string(), None, 100).unwrap();
        let id1 = env.record_id().unwrap();
        let json_str = env.to_json().unwrap();
        let parsed = Envelope::from_json(&json_str).unwrap();
        let id2 = parsed.record_id().unwrap();
        assert_eq!(id1, id2);
        assert!(id1.starts_with(RECORD_ID_PREFIX));
    }

    #[test]
    fn record_id_changes_when_any_field_changes() {
        let draft = sample_draft();
        let (mut env, _) =
            Envelope::unsigned(draft, "did:key:z6M123".to_string(), None, 100).unwrap();
        env.attach_signature("sig123".to_string()).unwrap();
        let base_id = env.record_id().unwrap();

        let mut e = env.clone();
        e.version = 2;
        assert_ne!(base_id, e.record_id().unwrap());

        let mut e = env.clone();
        e.record_type = "quote".to_string();
        assert_ne!(base_id, e.record_id().unwrap());

        let mut e = env.clone();
        e.issuer = "did:key:z6M999".to_string();
        assert_ne!(base_id, e.record_id().unwrap());

        let mut e = env.clone();
        e.subject = "sub_other".to_string();
        assert_ne!(base_id, e.record_id().unwrap());

        let mut e = env.clone();
        e.issued_at_secs = 101;
        assert_ne!(base_id, e.record_id().unwrap());

        let mut e = env.clone();
        e.expires_at_secs = Some(200);
        assert_ne!(base_id, e.record_id().unwrap());

        let mut e = env.clone();
        e.payload = json!({"title": "test item", "price": 101});
        assert_ne!(base_id, e.record_id().unwrap());

        let mut e = env.clone();
        e.signature = "sig456".to_string();
        assert_ne!(base_id, e.record_id().unwrap());
    }

    #[test]
    fn envelope_without_version_fails_to_parse() {
        let json_str = r#"{"envelope_version":1,"record_type":"listing","issuer":"did:key:z6M123","subject":"","issued_at_secs":100,"payload":{},"signature":""}"#;
        assert!(Envelope::from_json(json_str).is_err());
    }

    #[test]
    fn envelope_without_envelope_version_fails_to_parse() {
        let json_str = r#"{"version":1,"record_type":"listing","issuer":"did:key:z6M123","subject":"","issued_at_secs":100,"payload":{},"signature":""}"#;
        assert!(Envelope::from_json(json_str).is_err());
    }

    #[test]
    fn record_draft_deserializes_kebab_and_snake_case() {
        let kebab = r#"{"version":1,"record-type":"listing","subject":"s1","payload":{},"expires-at-secs":100}"#;
        let d1: RecordDraft = serde_json::from_str(kebab).unwrap();
        assert_eq!(d1.record_type, "listing");
        assert_eq!(d1.expires_at_secs, Some(100));

        let snake = r#"{"version":1,"record_type":"listing","subject":"s1","payload":{},"expires_at_secs":100}"#;
        let d2: RecordDraft = serde_json::from_str(snake).unwrap();
        assert_eq!(d2.record_type, "listing");
        assert_eq!(d2.expires_at_secs, Some(100));
    }
}
