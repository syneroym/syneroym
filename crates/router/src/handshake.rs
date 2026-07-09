//! Handshake Authorization and Preamble Verification
//!
//! Confirms the client's temporary identity key is authorized by their master
//! DID.

use std::time::Duration;

use anyhow::{Result, anyhow};
use syneroym_core::dht_registry::{MasterAnchorPayload, RegistryClient};
use syneroym_identity::substrate;
use tokio::time;

use crate::RoutePreamble;

#[async_trait::async_trait]
pub trait MasterAnchorResolver: Send + Sync {
    async fn resolve_master_anchor(
        &self,
        master_id: &str,
    ) -> Result<MasterAnchorPayload, anyhow::Error>;
}

#[async_trait::async_trait]
impl MasterAnchorResolver for RegistryClient {
    async fn resolve_master_anchor(
        &self,
        master_id: &str,
    ) -> Result<MasterAnchorPayload, anyhow::Error> {
        self.resolve_master_anchor(master_id, None).await
    }
}

#[derive(Debug)]
pub struct VerifiedIdentity {
    pub master_did: String,
    pub temporary_did: String,
}

#[derive(Debug)]
pub struct HandshakeVerifier;

impl HandshakeVerifier {
    pub async fn verify_preamble(
        preamble: &RoutePreamble,
        resolver: &dyn MasterAnchorResolver,
    ) -> Result<VerifiedIdentity, anyhow::Error> {
        let source_pubkey_hex = preamble
            .pubkey
            .as_ref()
            .ok_or_else(|| anyhow!("Missing client public key (pubkey) in preamble"))?;

        let source_pubkey_bytes = hex::decode(source_pubkey_hex)
            .map_err(|e| anyhow!("Invalid hex in client pubkey: {e}"))?;

        let source_pubkey = ed25519_dalek::VerifyingKey::from_bytes(
            &source_pubkey_bytes.try_into().map_err(|_| anyhow!("Invalid client pubkey length"))?,
        )
        .map_err(|e| anyhow!("Invalid client pubkey: {e}"))?;

        let temporary_did = substrate::derive_did_key(&source_pubkey);

        if let Some(cert) = &preamble.delegation {
            let master_did = &cert.master_did;

            // Verify certificate (expiry and signature match master)
            cert.verify(master_did)?;

            if cert.temporary_did != temporary_did {
                return Err(anyhow!(
                    "Delegation certificate temporary_did does not match preamble pubkey DID"
                ));
            }

            // Resolve master anchor from DHT / HTTP Registry to check for revocation
            let anchor =
                time::timeout(Duration::from_secs(5), resolver.resolve_master_anchor(master_did))
                    .await
                    .map_err(|_| anyhow!("Timeout resolving master anchor"))??;

            if anchor.revoked_keys.contains(&temporary_did) {
                return Err(anyhow!(
                    "Temporary DID {temporary_did} has been revoked by master {master_did}"
                ));
            }

            Ok(VerifiedIdentity { master_did: master_did.clone(), temporary_did })
        } else {
            // If no master_did is specified, fall back: the source key is the master key
            // itself.
            Ok(VerifiedIdentity { master_did: temporary_did.clone(), temporary_did })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::RwLock;

    use syneroym_identity::{DelegationCertificate, Identity};

    use super::*;
    use crate::RoutePreamble;

    struct MockResolver {
        anchor: RwLock<MasterAnchorPayload>,
    }

    #[async_trait::async_trait]
    impl MasterAnchorResolver for MockResolver {
        async fn resolve_master_anchor(
            &self,
            _master_id: &str,
        ) -> Result<MasterAnchorPayload, anyhow::Error> {
            Ok(self.anchor.read().unwrap().clone())
        }
    }

    #[tokio::test]
    async fn test_handshake_success_direct_master() {
        let client = Identity::generate().unwrap();
        let client_pubkey_hex = hex::encode(client.public_key().to_bytes());

        let preamble =
            RoutePreamble::parse(&format!("json-rpc://service?pubkey={client_pubkey_hex}"))
                .unwrap();
        let resolver = MockResolver { anchor: RwLock::new(MasterAnchorPayload::default()) };

        let res = HandshakeVerifier::verify_preamble(&preamble, &resolver).await;
        assert!(res.is_ok());
        let ident = res.unwrap();
        let expected_did = substrate::derive_did_key(&client.public_key());
        assert_eq!(ident.master_did, expected_did);
        assert_eq!(ident.temporary_did, expected_did);
    }

    #[tokio::test]
    async fn test_handshake_success_delegated() {
        let master = Identity::generate().unwrap();
        let temp = Identity::generate().unwrap();

        let master_did = substrate::derive_did_key(&master.public_key());
        let temp_pubkey = temp.public_key();
        let temp_pubkey_hex = hex::encode(temp_pubkey.to_bytes());

        let cert = DelegationCertificate::issue(&master, temp_pubkey, 3600, "routing".to_string())
            .unwrap();
        let cert_hex = hex::encode(cert.to_json().unwrap());

        let preamble = RoutePreamble::parse(&format!(
            "json-rpc://service?pubkey={temp_pubkey_hex}&delegation={cert_hex}"
        ))
        .unwrap();

        let resolver = MockResolver { anchor: RwLock::new(MasterAnchorPayload::default()) };

        let res = HandshakeVerifier::verify_preamble(&preamble, &resolver).await;
        assert!(res.is_ok());
        let ident = res.unwrap();
        assert_eq!(ident.master_did, master_did);
        assert_eq!(ident.temporary_did, substrate::derive_did_key(&temp_pubkey));
    }

    #[tokio::test]
    async fn test_handshake_failed_unauthorized() {
        let master = Identity::generate().unwrap();
        let temp = Identity::generate().unwrap();
        let untrusted = Identity::generate().unwrap();

        let untrusted_pubkey_hex = hex::encode(untrusted.public_key().to_bytes());

        let cert =
            DelegationCertificate::issue(&master, temp.public_key(), 3600, "routing".to_string())
                .unwrap();
        let cert_hex = hex::encode(cert.to_json().unwrap());

        let preamble = RoutePreamble::parse(&format!(
            "json-rpc://service?pubkey={untrusted_pubkey_hex}&delegation={cert_hex}"
        ))
        .unwrap();

        let resolver = MockResolver { anchor: RwLock::new(MasterAnchorPayload::default()) };

        let res = HandshakeVerifier::verify_preamble(&preamble, &resolver).await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn test_handshake_failed_expired_cert() {
        let master = Identity::generate().unwrap();
        let temp = Identity::generate().unwrap();

        let temp_pubkey = temp.public_key();
        let temp_pubkey_hex = hex::encode(temp_pubkey.to_bytes());

        // Issue an expired certificate (expires_in = 0, will fail verification)
        let cert =
            DelegationCertificate::issue(&master, temp_pubkey, 0, "routing".to_string()).unwrap();
        let cert_hex = hex::encode(cert.to_json().unwrap());

        let preamble = RoutePreamble::parse(&format!(
            "json-rpc://service?pubkey={temp_pubkey_hex}&delegation={cert_hex}"
        ))
        .unwrap();

        let resolver = MockResolver { anchor: RwLock::new(MasterAnchorPayload::default()) };

        let res = HandshakeVerifier::verify_preamble(&preamble, &resolver).await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn test_handshake_passive_revocation() {
        let master = Identity::generate().unwrap();
        let temp = Identity::generate().unwrap();

        let temp_pubkey = temp.public_key();
        let temp_pubkey_hex = hex::encode(temp_pubkey.to_bytes());

        let cert = DelegationCertificate::issue(&master, temp_pubkey, 3600, "routing".to_string())
            .unwrap();
        let cert_hex = hex::encode(cert.to_json().unwrap());

        let preamble = RoutePreamble::parse(&format!(
            "json-rpc://service?pubkey={temp_pubkey_hex}&delegation={cert_hex}"
        ))
        .unwrap();

        let resolver = MockResolver { anchor: RwLock::new(MasterAnchorPayload::default()) };

        // 1. Initially verified successfully
        let res = HandshakeVerifier::verify_preamble(&preamble, &resolver).await;
        assert!(res.is_ok());

        // 2. Revocation: update anchor to revoke cert, verification must now fail
        {
            let temp_did = substrate::derive_did_key(&temp_pubkey);
            let mut anchor_guard = resolver.anchor.write().unwrap();
            anchor_guard.revoked_keys.push(temp_did);
        }

        let res2 = HandshakeVerifier::verify_preamble(&preamble, &resolver).await;
        assert!(res2.is_err());
    }
}
