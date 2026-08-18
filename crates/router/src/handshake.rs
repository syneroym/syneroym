//! Handshake Authorization and Preamble Verification
//!
//! Confirms the client's temporary identity key is authorized by their master
//! DID.

use std::time::Duration;

use anyhow::{Result, anyhow};
pub use syneroym_core::dht_registry::MasterAnchorResolver;
use syneroym_identity::{delegation::TRANSPORT_SCOPES, substrate};
use tokio::time;

use crate::RoutePreamble;

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

            // `master_did` is read from the certificate itself, so this
            // first argument is a no-op confused-deputy check on this path --
            // the connection asserts "I am delegated by M" and M is whatever
            // the certificate says; binding to a target is resolved
            // downstream on `master_did`. The scope check is what actually
            // enforces something here: both a person's delegated device key
            // and a service instance route a connection under a master's
            // identity, so the ingress accepts either transport scope and
            // leaves narrowing to whoever admits the connection for a
            // specific purpose.
            cert.verify(master_did, &TRANSPORT_SCOPES)?;

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

    use syneroym_core::dht_registry::MasterAnchorPayload;
    use syneroym_identity::{DelegationCertificate, Identity};

    use super::*;
    use crate::RoutePreamble;

    #[derive(Debug)]
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

    #[tokio::test]
    async fn a_routing_scoped_certificate_is_accepted_on_a_connection() {
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

        let ident = HandshakeVerifier::verify_preamble(&preamble, &resolver)
            .await
            .expect("a routing-scoped certificate is unchanged, existing behavior");
        assert_eq!(ident.master_did, master_did);
    }

    #[tokio::test]
    async fn a_service_instance_scoped_certificate_is_accepted_on_a_connection() {
        let member_master = Identity::generate().unwrap();
        let instance = Identity::generate().unwrap();
        let member_master_did = substrate::derive_did_key(&member_master.public_key());
        let instance_pubkey = instance.public_key();
        let instance_pubkey_hex = hex::encode(instance_pubkey.to_bytes());

        let cert = DelegationCertificate::issue(
            &member_master,
            instance_pubkey,
            3600,
            "service-instance".to_string(),
        )
        .unwrap();
        let cert_hex = hex::encode(cert.to_json().unwrap());

        let preamble = RoutePreamble::parse(&format!(
            "json-rpc://service?pubkey={instance_pubkey_hex}&delegation={cert_hex}"
        ))
        .unwrap();
        let resolver = MockResolver { anchor: RwLock::new(MasterAnchorPayload::default()) };

        let ident = HandshakeVerifier::verify_preamble(&preamble, &resolver)
            .await
            .expect("a service-instance-scoped certificate must be admitted at the ingress");
        assert_eq!(ident.master_did, member_master_did);
        assert_eq!(ident.temporary_did, substrate::derive_did_key(&instance_pubkey));
    }

    #[tokio::test]
    async fn a_certificate_scoped_outside_transport_is_rejected_at_the_handshake() {
        let master = Identity::generate().unwrap();
        let temp = Identity::generate().unwrap();
        let temp_pubkey = temp.public_key();
        let temp_pubkey_hex = hex::encode(temp_pubkey.to_bytes());

        let cert =
            DelegationCertificate::issue(&master, temp_pubkey, 3600, "vault-unseal".to_string())
                .unwrap();
        let cert_hex = hex::encode(cert.to_json().unwrap());

        let preamble = RoutePreamble::parse(&format!(
            "json-rpc://service?pubkey={temp_pubkey_hex}&delegation={cert_hex}"
        ))
        .unwrap();
        let resolver = MockResolver { anchor: RwLock::new(MasterAnchorPayload::default()) };

        let res = HandshakeVerifier::verify_preamble(&preamble, &resolver).await;
        assert!(res.is_err(), "a certificate scoped outside TRANSPORT_SCOPES must be rejected");
    }

    #[tokio::test]
    async fn an_expired_instance_certificate_fails_the_handshake_closed() {
        let member_master = Identity::generate().unwrap();
        let instance = Identity::generate().unwrap();
        let instance_pubkey = instance.public_key();
        let instance_pubkey_hex = hex::encode(instance_pubkey.to_bytes());

        // expires_in_secs = 0 -> already expired by the time verify() runs.
        let cert = DelegationCertificate::issue(
            &member_master,
            instance_pubkey,
            0,
            "service-instance".to_string(),
        )
        .unwrap();
        let cert_hex = hex::encode(cert.to_json().unwrap());

        let preamble = RoutePreamble::parse(&format!(
            "json-rpc://service?pubkey={instance_pubkey_hex}&delegation={cert_hex}"
        ))
        .unwrap();
        let resolver = MockResolver { anchor: RwLock::new(MasterAnchorPayload::default()) };

        let res = HandshakeVerifier::verify_preamble(&preamble, &resolver).await;
        assert!(res.is_err(), "an expired service-instance certificate must fail closed");
    }

    /// Matrix row 14's testable half: an instance certificate is revocable
    /// without touching the member master. The anchor revokes instance key 1
    /// (e.g. after a compromise or a planned rotation); the same master then
    /// certifies a fresh instance key 2, and that connection still verifies.
    #[tokio::test]
    async fn a_revoked_instance_key_is_rejected_while_the_member_master_still_certifies_a_new_one()
    {
        let member_master = Identity::generate().unwrap();
        let instance1 = Identity::generate().unwrap();
        let instance1_pubkey_hex = hex::encode(instance1.public_key().to_bytes());
        let instance1_did = substrate::derive_did_key(&instance1.public_key());

        let cert1 = DelegationCertificate::issue(
            &member_master,
            instance1.public_key(),
            3600,
            "service-instance".to_string(),
        )
        .unwrap();
        let cert1_hex = hex::encode(cert1.to_json().unwrap());
        let preamble1 = RoutePreamble::parse(&format!(
            "json-rpc://service?pubkey={instance1_pubkey_hex}&delegation={cert1_hex}"
        ))
        .unwrap();

        let resolver = MockResolver { anchor: RwLock::new(MasterAnchorPayload::default()) };
        resolver.anchor.write().unwrap().revoked_keys.push(instance1_did);

        let res1 = HandshakeVerifier::verify_preamble(&preamble1, &resolver).await;
        assert!(res1.is_err(), "a revoked instance key must be rejected");

        let instance2 = Identity::generate().unwrap();
        let instance2_pubkey_hex = hex::encode(instance2.public_key().to_bytes());
        let cert2 = DelegationCertificate::issue(
            &member_master,
            instance2.public_key(),
            3600,
            "service-instance".to_string(),
        )
        .unwrap();
        let cert2_hex = hex::encode(cert2.to_json().unwrap());
        let preamble2 = RoutePreamble::parse(&format!(
            "json-rpc://service?pubkey={instance2_pubkey_hex}&delegation={cert2_hex}"
        ))
        .unwrap();

        let res2 = HandshakeVerifier::verify_preamble(&preamble2, &resolver).await;
        assert!(
            res2.is_ok(),
            "a fresh instance key certified by the same master must still verify -- revoking one \
             instance key must not touch the member's identity"
        );
    }

    /// The reference scenario's step 4, at unit scale: a member
    /// reinstantiated under a new instance key (a restart, or relocation to
    /// another node) presents a different `temporary_did` but resolves to
    /// the *same* `master_did` -- the identity a dependent's binding names,
    /// and the identity `caller_did` ends up carrying.
    #[tokio::test]
    async fn a_second_instance_under_the_same_master_presents_the_same_authorization_identity() {
        let member_master = Identity::generate().unwrap();
        let member_master_did = substrate::derive_did_key(&member_master.public_key());

        let mut master_dids = Vec::new();
        for _ in 0..2 {
            let instance = Identity::generate().unwrap();
            let instance_pubkey_hex = hex::encode(instance.public_key().to_bytes());
            let cert = DelegationCertificate::issue(
                &member_master,
                instance.public_key(),
                3600,
                "service-instance".to_string(),
            )
            .unwrap();
            let cert_hex = hex::encode(cert.to_json().unwrap());
            let preamble = RoutePreamble::parse(&format!(
                "json-rpc://service?pubkey={instance_pubkey_hex}&delegation={cert_hex}"
            ))
            .unwrap();
            let resolver = MockResolver { anchor: RwLock::new(MasterAnchorPayload::default()) };

            let verified = HandshakeVerifier::verify_preamble(&preamble, &resolver).await.unwrap();
            master_dids.push(verified.master_did);
        }

        assert_eq!(master_dids[0], master_dids[1]);
        assert_eq!(master_dids[0], member_master_did);
    }
}
