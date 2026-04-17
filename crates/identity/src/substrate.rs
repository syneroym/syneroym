use anyhow::{Context, Result, anyhow};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::Identity;

/// Represents the substrate's verification status regarding its controller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubstrateIdentityStatus {
    Verified,
    Unverified,
    None,
}

/// The state of the substrate's identity and control.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubstrateIdentityState {
    pub did: String,
    pub controller: Option<String>,
    pub status: SubstrateIdentityStatus,
}

/// A proof within a ControllerAgreement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proof {
    #[serde(rename = "type")]
    pub proof_type: String,
    #[serde(rename = "verificationMethod")]
    pub verification_method: String,
    #[serde(rename = "proofPurpose")]
    pub proof_purpose: String,
    #[serde(rename = "proofValue")]
    pub proof_value: String,
}

/// ControllerAgreement binding a node DID to a controller DID.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControllerAgreement {
    #[serde(rename = "type")]
    pub agreement_type: String,
    pub controlled: String,
    pub controller: String,
    #[serde(rename = "issuedAt")]
    pub issued_at: String,
    #[serde(rename = "expiresAt")]
    pub expires_at: Option<String>,
    pub proof: Vec<Proof>,
}

impl ControllerAgreement {
    /// Attempt to parse a ControllerAgreement from JSON string.
    pub fn from_json(json: &str) -> Result<Self> {
        serde_json::from_str(json).map_err(Into::into)
    }
}

/// Derive a did:key from an ed25519 public key.
pub fn derive_did_key(pubkey: &VerifyingKey) -> String {
    // multicodec ed25519-pub is 0xed01
    let mut bytes = vec![0xed, 0x01];
    bytes.extend_from_slice(pubkey.as_bytes());
    format!("did:key:h{}", z32::encode(&bytes))
}

/// Resolve a z-base-32 encoded string from a did:key.
pub fn resolve_did_z32(did: &str) -> Result<&str> {
    if !did.starts_with("did:key:h") {
        return Err(anyhow!("DID is not a z-base-32 did:key: {}", did));
    }
    Ok(&did["did:key:h".len()..])
}

/// Resolve an ed25519 public key from a did:key.
pub fn resolve_did_key(did: &str) -> Result<VerifyingKey> {
    let z32_str = resolve_did_z32(did)?;

    // Decode z-base-32
    let bytes =
        z32::decode(z32_str.as_bytes()).map_err(|_| anyhow!("Invalid z-base-32 encoding"))?;

    // Check multicodec prefix
    if bytes.len() != 34 || bytes[0] != 0xed || bytes[1] != 0x01 {
        return Err(anyhow!("Invalid multicodec prefix for ed25519-pub"));
    }

    let pubkey_bytes: [u8; 32] =
        bytes[2..34].try_into().map_err(|_| anyhow!("Invalid public key length"))?;
    VerifyingKey::from_bytes(&pubkey_bytes).map_err(Into::into)
}

/// Canonicalize JSON per RFC 8785 (JSON Canonicalization Scheme).
/// This ensures deterministic, spec-compliant serialization:
/// - Keys are sorted lexicographically
/// - No extraneous whitespace
/// - UTF-8 encoded with sorted object keys at all nesting levels
fn canonicalize_json_value(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut sorted_map = serde_json::Map::new();
            let mut keys: Vec<_> = map.keys().collect();
            keys.sort();
            for key in keys {
                if let Some(val) = map.get(key) {
                    sorted_map.insert(key.to_string(), canonicalize_json_value(val));
                }
            }
            serde_json::Value::Object(sorted_map)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(canonicalize_json_value).collect())
        }
        other => other.clone(),
    }
}

/// Validate a signature against the agreement's canonicalized form using RFC 8785 (JSON Canonicalization Scheme).
/// This ensures deterministic, spec-compliant signature verification compatible with external systems.
fn verify_signature(
    agreement: &ControllerAgreement,
    proof: &Proof,
    pubkey: &VerifyingKey,
) -> Result<()> {
    if proof.proof_type != "Ed25519Signature2020" {
        return Err(anyhow!("Unsupported proof type: {}", proof.proof_type));
    }

    // Serialize agreement and apply RFC 8785 JSON Canonicalization Scheme
    let mut agreement_value = serde_json::to_value(agreement)?;
    agreement_value.as_object_mut().context("Agreement JSON must be an object")?.remove("proof");

    // Canonicalize JSON per RFC 8785 (sorted keys, no whitespace)
    let canonical_value = canonicalize_json_value(&agreement_value);
    let payload = serde_json::to_string(&canonical_value)
        .context("Failed to serialize canonicalized agreement JSON")?;

    let sig_bytes = z32::decode(proof.proof_value.as_bytes())
        .map_err(|_| anyhow!("Invalid z-base-32 signature encoding"))?;
    if sig_bytes.len() != 64 {
        return Err(anyhow!("Invalid signature length"));
    }
    let signature = Signature::from_slice(&sig_bytes)?;

    pubkey.verify(payload.as_bytes(), &signature).map_err(Into::into)
}

impl SubstrateIdentityState {
    /// Initialize the SubstrateIdentityState according to the boot flow rules.
    pub fn init(
        substrate_identity: &Identity,
        agreement: Option<&ControllerAgreement>,
        controller_flag: Option<&str>,
        require_agreement: bool,
    ) -> Result<Self> {
        let substrate_pubkey = substrate_identity.public_key();
        let substrate_did = derive_did_key(&substrate_pubkey);

        if let Some(agr) = agreement {
            if agr.controlled != substrate_did {
                if require_agreement {
                    return Err(anyhow!("Agreement controlled DID does not match substrate DID"));
                }
                return Ok(Self {
                    did: substrate_did,
                    controller: None,
                    status: SubstrateIdentityStatus::None,
                });
            }

            // Resolve controller pubkey
            let controller_pubkey = match resolve_did_key(&agr.controller) {
                Ok(pk) => pk,
                Err(e) => {
                    if require_agreement {
                        return Err(anyhow!("Failed to resolve controller DID: {}", e));
                    }
                    return Ok(Self {
                        did: substrate_did,
                        controller: Some(agr.controller.clone()),
                        status: SubstrateIdentityStatus::Unverified,
                    });
                }
            };

            // Validate signatures
            // We expect one proof from controller and one from substrate
            let mut controller_valid = false;
            let mut substrate_valid = false;

            for proof in &agr.proof {
                if proof.verification_method.starts_with(&agr.controller) {
                    if verify_signature(agr, proof, &controller_pubkey).is_ok() {
                        controller_valid = true;
                    }
                } else if proof.verification_method.starts_with(&substrate_did)
                    && verify_signature(agr, proof, &substrate_pubkey).is_ok()
                {
                    substrate_valid = true;
                }
            }

            if controller_valid && substrate_valid {
                if let Some(expires_at) = &agr.expires_at
                    && let Ok(dt) = chrono::DateTime::parse_from_rfc3339(expires_at)
                    && dt < chrono::Utc::now()
                {
                    if require_agreement {
                        return Err(anyhow!("Agreement expired"));
                    }
                    return Ok(Self {
                        did: substrate_did,
                        controller: Some(agr.controller.clone()),
                        status: SubstrateIdentityStatus::Unverified,
                    });
                }

                return Ok(Self {
                    did: substrate_did,
                    controller: Some(agr.controller.clone()),
                    status: SubstrateIdentityStatus::Verified,
                });
            } else {
                if require_agreement {
                    return Err(anyhow!("Agreement signatures invalid"));
                }
                return Ok(Self {
                    did: substrate_did,
                    controller: Some(agr.controller.clone()),
                    status: SubstrateIdentityStatus::Unverified,
                });
            }
        }

        if let Some(ctrl) = controller_flag {
            return Ok(Self {
                did: substrate_did,
                controller: Some(ctrl.to_string()),
                status: SubstrateIdentityStatus::Unverified,
            });
        }

        Ok(Self { did: substrate_did, controller: None, status: SubstrateIdentityStatus::None })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Identity;

    #[test]
    fn test_substrate_identity_state_no_agreement_no_controller() {
        let identity = Identity::generate().expect("Failed to generate identity");
        let state = SubstrateIdentityState::init(&identity, None, None, false).unwrap();

        assert_eq!(state.did, derive_did_key(&identity.public_key()));
        assert_eq!(state.controller, None);
        assert_eq!(state.status, SubstrateIdentityStatus::None);
    }

    #[test]
    fn test_substrate_identity_state_with_controller_flag_only() {
        let identity = Identity::generate().expect("Failed to generate identity");
        let controller_did = "did:key:hybndrfg8ejkmcpqx";
        let state =
            SubstrateIdentityState::init(&identity, None, Some(controller_did), false).unwrap();

        assert_eq!(state.did, derive_did_key(&identity.public_key()));
        assert_eq!(state.controller, Some(controller_did.to_string()));
        assert_eq!(state.status, SubstrateIdentityStatus::Unverified);
    }

    #[test]
    fn test_derive_and_resolve_did_key() {
        let identity = Identity::generate().expect("Failed to generate identity");
        let did = derive_did_key(&identity.public_key());

        assert!(did.starts_with("did:key:h"));

        let resolved_pubkey = resolve_did_key(&did).expect("Failed to resolve generated did:key");
        assert_eq!(identity.public_key().as_bytes(), resolved_pubkey.as_bytes());
    }

    #[test]
    fn test_invalid_did_key_resolution() {
        let invalid_did = "did:web:example.com";
        let result = resolve_did_key(invalid_did);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "DID is not a z-base-32 did:key: did:web:example.com"
        );
    }

    #[test]
    fn test_resolve_did_z32() {
        let identity = Identity::generate().expect("Failed to generate identity");
        let did = derive_did_key(&identity.public_key());
        let z32_str = resolve_did_z32(&did).unwrap();

        let mut bytes = vec![0xed, 0x01];
        bytes.extend_from_slice(identity.public_key().as_bytes());
        let expected_z32 = z32::encode(&bytes);

        assert_eq!(z32_str, expected_z32);
    }

    #[test]
    fn test_resolve_did_z32_invalid() {
        let invalid_did = "did:web:example.com";
        let result = resolve_did_z32(invalid_did);
        assert!(result.is_err());
    }
}
