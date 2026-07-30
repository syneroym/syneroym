//! Substrate-Controller agreements and boot-time identity flow
//!
//! Implements `did:key` resolution, RFC 8785 JSON Canonicalization,
//! controller agreement verification, and cryptographic status checks.

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

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

/// A proof within a `ControllerAgreement`.
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

/// The only `agreement_type` this tree issues or accepts.
pub const CONTROLLER_AGREEMENT_TYPE: &str = "ControllerAgreement";

/// `ControllerAgreement` binding a node DID to a controller DID.
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
    /// Attempt to parse a `ControllerAgreement` from JSON string.
    pub fn from_json(json: &str) -> Result<Self> {
        serde_json::from_str(json).map_err(Into::into)
    }

    /// Mint a mutually-signed agreement binding `node`'s DID (the
    /// `controlled`) to `controller`'s DID. Both proofs are produced here
    /// because both private keys are needed and neither can be supplied
    /// remotely: the node's key lives only on the node's filesystem.
    ///
    /// `expires_in_secs: None` issues an agreement with no expiry.
    pub fn issue(
        node: &Identity,
        controller: &Identity,
        expires_in_secs: Option<u64>,
    ) -> Result<Self> {
        let node_did = derive_did_key(&node.public_key());
        let controller_did = derive_did_key(&controller.public_key());

        if node_did == controller_did {
            bail!(
                "a substrate cannot be its own controller: the agreement's two proofs are \
                 indistinguishable when `controlled` == `controller`, so it can never verify (see \
                 SubstrateIdentityState::init). Create a separate operator identity with `roymctl \
                 identity create --name <name>`."
            );
        }

        let now = Utc::now();
        let issued_at = now.to_rfc3339_opts(SecondsFormat::Secs, true);
        let expires_at = expires_in_secs.map(|secs| {
            (now + Duration::seconds(secs as i64)).to_rfc3339_opts(SecondsFormat::Secs, true)
        });

        let unsigned = Self {
            agreement_type: CONTROLLER_AGREEMENT_TYPE.to_string(),
            controlled: node_did.clone(),
            controller: controller_did.clone(),
            issued_at,
            expires_at,
            proof: vec![],
        };

        // The canonical payload every proof signs over -- byte-for-byte what
        // `verify_signature` reconstructs: the agreement serialized with
        // `proof` removed, then RFC-8785 canonicalized.
        let mut value = serde_json::to_value(&unsigned)?;
        value.as_object_mut().context("agreement JSON must be an object")?.remove("proof");
        let payload = canonicalize_json_value(&value);

        let proofs = vec![
            proof_for(controller, &controller_did, &payload)?,
            proof_for(node, &node_did, &payload)?,
        ];

        Ok(Self { proof: proofs, ..unsigned })
    }
}

fn proof_for(signer: &Identity, signer_did: &str, payload: &Value) -> Result<Proof> {
    Ok(Proof {
        proof_type: "Ed25519Signature2020".to_string(),
        verification_method: format!("{signer_did}#key-1"),
        proof_purpose: "capabilityDelegation".to_string(),
        proof_value: signer.sign_json(payload)?,
    })
}

/// Derive a did:key from an ed25519 public key.
#[must_use]
pub fn derive_did_key(pubkey: &VerifyingKey) -> String {
    // multicodec ed25519-pub is 0xed01
    let mut bytes = vec![0xed, 0x01];
    bytes.extend_from_slice(pubkey.as_bytes());
    format!("did:key:h{}", z32::encode(&bytes))
}

/// Resolve a z-base-32 encoded string from a did:key.
pub fn resolve_did_z32(did: &str) -> Result<&str> {
    if !did.starts_with("did:key:h") {
        return Err(anyhow!("DID is not a z-base-32 did:key: {did}"));
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
pub fn canonicalize_json_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut sorted_map = Map::new();
            let mut keys: Vec<_> = map.keys().collect();
            keys.sort();
            for key in keys {
                if let Some(val) = map.get(key) {
                    sorted_map.insert(key.clone(), canonicalize_json_value(val));
                }
            }
            Value::Object(sorted_map)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(canonicalize_json_value).collect()),
        other => other.clone(),
    }
}

/// Verify a z-base-32 Ed25519 signature (as produced by `Identity::sign_json`)
/// over the RFC-8785 canonicalization of `value`, against the pubkey resolved
/// from `signer_did`. Mirrors `DelegationCertificate::verify`'s crypto steps,
/// exposed as a free function so `syneroym-ucan` can verify UCAN token
/// signatures without depending on `ed25519-dalek`/`z32` directly.
pub fn verify_json_signature(signer_did: &str, value: &Value, sig_z32: &str) -> Result<()> {
    let pubkey = resolve_did_key(signer_did).context("failed to resolve signer DID")?;
    let canonical = canonicalize_json_value(value);
    let payload =
        serde_json::to_string(&canonical).context("failed to serialize canonical payload")?;
    let sig_bytes = z32::decode(sig_z32.as_bytes())
        .map_err(|_| anyhow!("invalid z-base-32 signature encoding"))?;
    let signature = Signature::from_slice(&sig_bytes).context("invalid Ed25519 signature bytes")?;
    pubkey.verify(payload.as_bytes(), &signature).map_err(Into::into)
}

/// Validate a signature against the agreement's canonicalized form using RFC
/// 8785 (JSON Canonicalization Scheme). This ensures deterministic,
/// spec-compliant signature verification compatible with external systems.
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
    /// Initialize the `SubstrateIdentityState` according to the boot flow
    /// rules.
    pub fn init(
        substrate_identity: &Identity,
        agreement: Option<&ControllerAgreement>,
        controller_flag: Option<&str>,
        require_agreement: bool,
    ) -> Result<Self> {
        let substrate_pubkey = substrate_identity.public_key();
        let substrate_did = derive_did_key(&substrate_pubkey);

        if let Some(agr) = agreement {
            if agr.agreement_type != CONTROLLER_AGREEMENT_TYPE {
                if require_agreement {
                    return Err(anyhow!("unsupported agreement type '{}'", agr.agreement_type));
                }
                return Ok(Self {
                    did: substrate_did,
                    controller: None,
                    status: SubstrateIdentityStatus::None,
                });
            }

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
                        return Err(anyhow!("Failed to resolve controller DID: {e}"));
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
                if let Some(expires_at) = &agr.expires_at {
                    let parsed = DateTime::parse_from_rfc3339(expires_at);
                    match parsed {
                        Ok(dt) if dt < Utc::now() => {
                            if require_agreement {
                                return Err(anyhow!("Agreement expired"));
                            }
                            return Ok(Self {
                                did: substrate_did,
                                controller: Some(agr.controller.clone()),
                                status: SubstrateIdentityStatus::Unverified,
                            });
                        }
                        Ok(_) => {}
                        Err(_) => {
                            // A present-but-unparseable expiresAt used to be
                            // treated as "no expiry" -- fail-open on a
                            // hand-edited agreement. Now it is an error.
                            if require_agreement {
                                return Err(anyhow!("agreement expiresAt is not RFC-3339"));
                            }
                            return Ok(Self {
                                did: substrate_did,
                                controller: Some(agr.controller.clone()),
                                status: SubstrateIdentityStatus::Unverified,
                            });
                        }
                    }
                }

                return Ok(Self {
                    did: substrate_did,
                    controller: Some(agr.controller.clone()),
                    status: SubstrateIdentityStatus::Verified,
                });
            }
            if require_agreement {
                return Err(anyhow!("Agreement signatures invalid"));
            }
            return Ok(Self {
                did: substrate_did,
                controller: Some(agr.controller.clone()),
                status: SubstrateIdentityStatus::Unverified,
            });
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

    #[test]
    fn test_verify_json_signature_round_trip() {
        let signer = Identity::generate().unwrap();
        let signer_did = derive_did_key(&signer.public_key());
        let value = serde_json::json!({"b": 2, "a": 1});

        let sig = signer.sign_json(&value).unwrap();

        verify_json_signature(&signer_did, &value, &sig).unwrap();
    }

    #[test]
    fn test_verify_json_signature_rejects_tampered_value() {
        let signer = Identity::generate().unwrap();
        let signer_did = derive_did_key(&signer.public_key());
        let value = serde_json::json!({"a": 1});
        let sig = signer.sign_json(&value).unwrap();

        let tampered = serde_json::json!({"a": 2});
        assert!(verify_json_signature(&signer_did, &tampered, &sig).is_err());
    }

    #[test]
    fn test_verify_json_signature_rejects_wrong_signer() {
        let signer = Identity::generate().unwrap();
        let other = Identity::generate().unwrap();
        let other_did = derive_did_key(&other.public_key());
        let value = serde_json::json!({"a": 1});
        let sig = signer.sign_json(&value).unwrap();

        assert!(verify_json_signature(&other_did, &value, &sig).is_err());
    }

    #[test]
    fn issue_produces_a_mutually_signed_agreement_that_verifies() {
        let node = Identity::generate().unwrap();
        let controller = Identity::generate().unwrap();

        let agreement = ControllerAgreement::issue(&node, &controller, None).unwrap();
        let state = SubstrateIdentityState::init(&node, Some(&agreement), None, true).unwrap();

        assert_eq!(state.status, SubstrateIdentityStatus::Verified);
        assert_eq!(state.controller, Some(derive_did_key(&controller.public_key())));
    }

    #[test]
    fn issue_rejects_a_self_owned_agreement() {
        let node = Identity::generate().unwrap();
        let same_bytes = node.to_bytes();
        let same = Identity::from_bytes(&same_bytes);

        let err = ControllerAgreement::issue(&node, &same, None).unwrap_err();
        assert!(err.to_string().contains("cannot be its own controller"));
    }

    #[test]
    fn an_agreement_naming_another_node_is_not_verified() {
        let node = Identity::generate().unwrap();
        let other_node = Identity::generate().unwrap();
        let controller = Identity::generate().unwrap();

        let agreement = ControllerAgreement::issue(&other_node, &controller, None).unwrap();

        let err = SubstrateIdentityState::init(&node, Some(&agreement), None, true).unwrap_err();
        assert!(err.to_string().contains("does not match"));

        let state = SubstrateIdentityState::init(&node, Some(&agreement), None, false).unwrap();
        assert_eq!(state.status, SubstrateIdentityStatus::None);
    }

    #[test]
    fn an_expired_agreement_is_not_verified() {
        let node = Identity::generate().unwrap();
        let controller = Identity::generate().unwrap();
        let agreement = ControllerAgreement::issue(&node, &controller, Some(0)).unwrap();

        // The expiry timestamp is generated a moment before `Utc::now()` is
        // re-evaluated at init time, so `now + 0s` is already in the past.
        let err = SubstrateIdentityState::init(&node, Some(&agreement), None, true).unwrap_err();
        assert!(err.to_string().contains("expired"));

        let state = SubstrateIdentityState::init(&node, Some(&agreement), None, false).unwrap();
        assert_eq!(state.status, SubstrateIdentityStatus::Unverified);
    }

    #[test]
    fn an_agreement_with_an_unparseable_expiry_is_not_verified() {
        let node = Identity::generate().unwrap();
        let controller = Identity::generate().unwrap();
        let mut agreement = ControllerAgreement::issue(&node, &controller, None).unwrap();
        agreement.expires_at = Some("not-a-timestamp".to_string());

        // Re-sign so the tampered field still passes signature verification
        // and the test isolates the expiry-parse behavior specifically.
        let resigned = resign(&node, &controller, agreement);

        let err = SubstrateIdentityState::init(&node, Some(&resigned), None, true).unwrap_err();
        assert!(err.to_string().contains("RFC-3339"));

        let state = SubstrateIdentityState::init(&node, Some(&resigned), None, false).unwrap();
        assert_eq!(state.status, SubstrateIdentityStatus::Unverified);
    }

    #[test]
    fn an_agreement_with_an_unknown_type_is_not_verified() {
        let node = Identity::generate().unwrap();
        let controller = Identity::generate().unwrap();
        let mut agreement = ControllerAgreement::issue(&node, &controller, None).unwrap();
        agreement.agreement_type = "SomethingElse".to_string();
        let resigned = resign(&node, &controller, agreement);

        let err = SubstrateIdentityState::init(&node, Some(&resigned), None, true).unwrap_err();
        assert!(err.to_string().contains("unsupported agreement type"));

        let state = SubstrateIdentityState::init(&node, Some(&resigned), None, false).unwrap();
        assert_eq!(state.status, SubstrateIdentityStatus::None);
    }

    #[test]
    fn a_tampered_agreement_field_invalidates_both_proofs() {
        let node = Identity::generate().unwrap();
        let controller = Identity::generate().unwrap();
        let attacker = Identity::generate().unwrap();
        let mut agreement = ControllerAgreement::issue(&node, &controller, None).unwrap();

        // Flip the controller to someone else's DID without re-signing --
        // both proofs were computed over the original canonical payload.
        agreement.controller = derive_did_key(&attacker.public_key());

        let state = SubstrateIdentityState::init(&node, Some(&agreement), None, false).unwrap();
        assert_eq!(state.status, SubstrateIdentityStatus::Unverified);
    }

    /// Re-sign an agreement after a field was hand-edited in a test, so the
    /// edit under test does not get masked by a signature-verification
    /// failure instead.
    fn resign(
        node: &Identity,
        controller: &Identity,
        agreement: ControllerAgreement,
    ) -> ControllerAgreement {
        let mut value = serde_json::to_value(&agreement).unwrap();
        value.as_object_mut().unwrap().remove("proof");
        let payload = canonicalize_json_value(&value);
        let controller_did = derive_did_key(&controller.public_key());
        let node_did = derive_did_key(&node.public_key());
        let proofs = vec![
            proof_for(controller, &controller_did, &payload).unwrap(),
            proof_for(node, &node_did, &payload).unwrap(),
        ];
        ControllerAgreement { proof: proofs, ..agreement }
    }
}
