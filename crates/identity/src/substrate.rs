use anyhow::{Result, anyhow};
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

    let pubkey_bytes: [u8; 32] = bytes[2..34].try_into().unwrap();
    VerifyingKey::from_bytes(&pubkey_bytes).map_err(Into::into)
}

/// Validate a signature against the agreement's canonicalized form.
/// TODO: Simplified canonicalization: strip the proof field from the JSON before signing.
/// In a real implementation, JCS (JSON Canonicalization Scheme) should be used.
fn verify_signature(
    agreement: &ControllerAgreement,
    proof: &Proof,
    pubkey: &VerifyingKey,
) -> Result<()> {
    if proof.proof_type != "Ed25519Signature2020" {
        return Err(anyhow!("Unsupported proof type: {}", proof.proof_type));
    }

    // Create a copy without proofs for canonicalization
    let mut unsigned_agreement = serde_json::to_value(agreement)?;
    unsigned_agreement.as_object_mut().unwrap().remove("proof");

    // Simple serialization
    let payload = serde_json::to_string(&unsigned_agreement)?;

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
                // TODO: Ignore expiresAt for now (requires parsing dates)
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
        let identity = Identity::generate();
        let state = SubstrateIdentityState::init(&identity, None, None, false).unwrap();

        assert_eq!(state.did, derive_did_key(&identity.public_key()));
        assert_eq!(state.controller, None);
        assert_eq!(state.status, SubstrateIdentityStatus::None);
    }

    #[test]
    fn test_substrate_identity_state_with_controller_flag_only() {
        let identity = Identity::generate();
        let controller_did = "did:key:hybndrfg8ejkmcpqx";
        let state =
            SubstrateIdentityState::init(&identity, None, Some(controller_did), false).unwrap();

        assert_eq!(state.did, derive_did_key(&identity.public_key()));
        assert_eq!(state.controller, Some(controller_did.to_string()));
        assert_eq!(state.status, SubstrateIdentityStatus::Unverified);
    }

    #[test]
    fn test_derive_and_resolve_did_key() {
        let identity = Identity::generate();
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
        let identity = Identity::generate();
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
