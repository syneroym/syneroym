use std::sync::RwLock;

use syneroym_core::dht_registry::MasterAnchorPayload;
use syneroym_identity::{DelegationCertificate, Identity, substrate};

use super::*;
use crate::RoutePreamble;

mod on_behalf;
mod proof;

#[derive(Debug)]
pub(crate) struct MockResolver {
    pub(crate) anchor: RwLock<MasterAnchorPayload>,
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
    let mut preamble = RoutePreamble::binary_json_rpc("service", "");
    preamble.sign(&client, now_secs());

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

    let cert =
        DelegationCertificate::issue(&master, temp_pubkey, 3600, "routing".to_string()).unwrap();

    let mut preamble = RoutePreamble::binary_json_rpc("service", "");
    preamble.delegation = Some(cert);
    preamble.sign(&temp, now_secs());

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

    let cert =
        DelegationCertificate::issue(&master, temp.public_key(), 3600, "routing".to_string())
            .unwrap();

    let mut preamble = RoutePreamble::binary_json_rpc("service", "");
    preamble.delegation = Some(cert);
    preamble.sign(&untrusted, now_secs());

    let resolver = MockResolver { anchor: RwLock::new(MasterAnchorPayload::default()) };

    let res = HandshakeVerifier::verify_preamble(&preamble, &resolver).await;
    assert!(res.is_err());
}

#[tokio::test]
async fn test_handshake_failed_expired_cert() {
    let master = Identity::generate().unwrap();
    let temp = Identity::generate().unwrap();

    let cert =
        DelegationCertificate::issue(&master, temp.public_key(), 0, "routing".to_string()).unwrap();

    let mut preamble = RoutePreamble::binary_json_rpc("service", "");
    preamble.delegation = Some(cert);
    preamble.sign(&temp, now_secs());

    let resolver = MockResolver { anchor: RwLock::new(MasterAnchorPayload::default()) };

    let res = HandshakeVerifier::verify_preamble(&preamble, &resolver).await;
    assert!(res.is_err());
}

#[tokio::test]
async fn test_handshake_passive_revocation() {
    let master = Identity::generate().unwrap();
    let temp = Identity::generate().unwrap();
    let temp_pubkey = temp.public_key();

    let cert =
        DelegationCertificate::issue(&master, temp_pubkey, 3600, "routing".to_string()).unwrap();

    let mut preamble = RoutePreamble::binary_json_rpc("service", "");
    preamble.delegation = Some(cert);
    preamble.sign(&temp, now_secs());

    let resolver = MockResolver { anchor: RwLock::new(MasterAnchorPayload::default()) };

    let res = HandshakeVerifier::verify_preamble(&preamble, &resolver).await;
    assert!(res.is_ok());

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

    let cert =
        DelegationCertificate::issue(&master, temp_pubkey, 3600, "routing".to_string()).unwrap();

    let mut preamble = RoutePreamble::binary_json_rpc("service", "");
    preamble.delegation = Some(cert);
    preamble.sign(&temp, now_secs());

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

    let cert = DelegationCertificate::issue(
        &member_master,
        instance_pubkey,
        3600,
        "service-instance".to_string(),
    )
    .unwrap();

    let mut preamble = RoutePreamble::binary_json_rpc("service", "");
    preamble.delegation = Some(cert);
    preamble.sign(&instance, now_secs());

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

    let cert =
        DelegationCertificate::issue(&master, temp.public_key(), 3600, "vault-unseal".to_string())
            .unwrap();

    let mut preamble = RoutePreamble::binary_json_rpc("service", "");
    preamble.delegation = Some(cert);
    preamble.sign(&temp, now_secs());

    let resolver = MockResolver { anchor: RwLock::new(MasterAnchorPayload::default()) };

    let res = HandshakeVerifier::verify_preamble(&preamble, &resolver).await;
    assert!(res.is_err(), "a certificate scoped outside TRANSPORT_SCOPES must be rejected");
}

#[tokio::test]
async fn an_expired_instance_certificate_fails_the_handshake_closed() {
    let member_master = Identity::generate().unwrap();
    let instance = Identity::generate().unwrap();

    let cert = DelegationCertificate::issue(
        &member_master,
        instance.public_key(),
        0,
        "service-instance".to_string(),
    )
    .unwrap();

    let mut preamble = RoutePreamble::binary_json_rpc("service", "");
    preamble.delegation = Some(cert);
    preamble.sign(&instance, now_secs());

    let resolver = MockResolver { anchor: RwLock::new(MasterAnchorPayload::default()) };

    let res = HandshakeVerifier::verify_preamble(&preamble, &resolver).await;
    assert!(res.is_err(), "an expired service-instance certificate must fail closed");
}

#[tokio::test]
async fn a_revoked_instance_key_is_rejected_while_the_member_master_still_certifies_a_new_one() {
    let member_master = Identity::generate().unwrap();
    let instance1 = Identity::generate().unwrap();
    let instance1_did = substrate::derive_did_key(&instance1.public_key());

    let cert1 = DelegationCertificate::issue(
        &member_master,
        instance1.public_key(),
        3600,
        "service-instance".to_string(),
    )
    .unwrap();
    let mut preamble1 = RoutePreamble::binary_json_rpc("service", "");
    preamble1.delegation = Some(cert1);
    preamble1.sign(&instance1, now_secs());

    let resolver = MockResolver { anchor: RwLock::new(MasterAnchorPayload::default()) };
    resolver.anchor.write().unwrap().revoked_keys.push(instance1_did);

    let res1 = HandshakeVerifier::verify_preamble(&preamble1, &resolver).await;
    assert!(res1.is_err(), "a revoked instance key must be rejected");

    let instance2 = Identity::generate().unwrap();
    let cert2 = DelegationCertificate::issue(
        &member_master,
        instance2.public_key(),
        3600,
        "service-instance".to_string(),
    )
    .unwrap();
    let mut preamble2 = RoutePreamble::binary_json_rpc("service", "");
    preamble2.delegation = Some(cert2);
    preamble2.sign(&instance2, now_secs());

    let res2 = HandshakeVerifier::verify_preamble(&preamble2, &resolver).await;
    assert!(res2.is_ok(), "a fresh instance key certified by the same master must still verify");
}

#[tokio::test]
async fn a_second_instance_under_the_same_master_presents_the_same_authorization_identity() {
    let member_master = Identity::generate().unwrap();
    let member_master_did = substrate::derive_did_key(&member_master.public_key());

    let mut master_dids = Vec::new();
    for _ in 0..2 {
        let instance = Identity::generate().unwrap();
        let cert = DelegationCertificate::issue(
            &member_master,
            instance.public_key(),
            3600,
            "service-instance".to_string(),
        )
        .unwrap();
        let mut preamble = RoutePreamble::binary_json_rpc("service", "");
        preamble.delegation = Some(cert);
        preamble.sign(&instance, now_secs());

        let resolver = MockResolver { anchor: RwLock::new(MasterAnchorPayload::default()) };

        let verified = HandshakeVerifier::verify_preamble(&preamble, &resolver).await.unwrap();
        master_dids.push(verified.master_did);
    }

    assert_eq!(master_dids[0], master_dids[1]);
    assert_eq!(master_dids[0], member_master_did);
}
