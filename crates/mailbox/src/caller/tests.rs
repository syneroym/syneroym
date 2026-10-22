use syneroym_identity::{DelegationCertificate, Identity, delegation::SCOPE_ROUTING};
use syneroym_rpc::{AuthLevel, CallerContext, CallerProof, SessionContext};

use super::*;
use crate::test_support::{Anchors, Member};

#[tokio::test]
async fn valid_instance_passes_all_checks() {
    let member = Member::new(3600);
    let caller = member.caller();
    let instance = instance_of(&caller, member.cert.issued_at_secs + 10).unwrap();
    assert_eq!(instance.master_did, member.service_id());
    assert_eq!(instance.instance_did, member.cert.temporary_did);
    assert_eq!(instance.issued_at_secs, member.cert.issued_at_secs);

    let anchors = Anchors::new();
    anchors.publish(&member.service_id());
    check_not_revoked(&anchors, &instance).await.unwrap();
}

#[test]
fn system_auth_is_unauthenticated() {
    let member = Member::new(3600);
    let mut caller = member.caller();
    caller.auth = AuthLevel::System;
    assert_eq!(
        instance_of(&caller, member.cert.issued_at_secs + 10).unwrap_err(),
        MailboxError::Unauthenticated
    );
}

#[test]
fn missing_proof_is_not_an_instance() {
    let member = Member::new(3600);
    let mut caller = member.caller();
    caller.proof = None;
    assert!(matches!(
        instance_of(&caller, member.cert.issued_at_secs + 10).unwrap_err(),
        MailboxError::NotAnInstance(_)
    ));
}

#[test]
fn missing_delegation_json_is_not_an_instance() {
    let member = Member::new(3600);
    let mut caller = member.caller();
    caller.proof = Some(CallerProof {
        pubkey_hex: hex::encode(member.instance.public_key().to_bytes()),
        delegation_json: None,
        ..Default::default()
    });
    assert!(matches!(
        instance_of(&caller, member.cert.issued_at_secs + 10).unwrap_err(),
        MailboxError::NotAnInstance(_)
    ));
}

#[test]
fn unreadable_certificate_json_is_not_an_instance() {
    let member = Member::new(3600);
    let mut caller = member.caller();
    caller.proof = Some(CallerProof {
        pubkey_hex: hex::encode(member.instance.public_key().to_bytes()),
        delegation_json: Some("not json".to_string()),
        ..Default::default()
    });
    assert!(matches!(
        instance_of(&caller, member.cert.issued_at_secs + 10).unwrap_err(),
        MailboxError::NotAnInstance(_)
    ));
}

#[test]
fn routing_scope_is_not_an_instance() {
    let master = Identity::generate().unwrap();
    let instance = Identity::generate().unwrap();
    let cert = DelegationCertificate::issue(
        &master,
        instance.public_key(),
        3600,
        SCOPE_ROUTING.to_string(),
    )
    .unwrap();
    let master_did = substrate::derive_did_key(&master.public_key());
    let caller = CallerContext {
        caller_did: master_did.clone(),
        app_instance: None,
        session: SessionContext {
            subject_did: master_did,
            capabilities: Vec::new(),
            ..Default::default()
        },

        auth: AuthLevel::Delegated,
        proof: Some(CallerProof {
            pubkey_hex: hex::encode(instance.public_key().to_bytes()),
            delegation_json: Some(cert.to_json().unwrap()),
            ..Default::default()
        }),
        on_behalf_did: None,
    };
    assert!(matches!(
        instance_of(&caller, cert.issued_at_secs + 10).unwrap_err(),
        MailboxError::NotAnInstance(_)
    ));
}

#[test]
fn certificate_for_another_master_is_refused() {
    let member = Member::new(3600);
    let mut caller = member.caller();
    caller.caller_did = "did:key:zOtherMaster".to_string();
    assert!(matches!(
        instance_of(&caller, member.cert.issued_at_secs + 10).unwrap_err(),
        MailboxError::NotAnInstance(_)
    ));
}

#[test]
fn mismatched_pubkey_is_refused() {
    let member = Member::new(3600);
    let mut caller = member.caller();
    let other_key = Identity::generate().unwrap();
    caller.proof = Some(CallerProof {
        pubkey_hex: hex::encode(other_key.public_key().to_bytes()),
        delegation_json: Some(member.cert.to_json().unwrap()),
        ..Default::default()
    });
    assert!(matches!(
        instance_of(&caller, member.cert.issued_at_secs + 10).unwrap_err(),
        MailboxError::NotAnInstance(_)
    ));
}

#[test]
fn expired_certificate_is_refused() {
    let member = Member::new(3600);
    let caller = member.caller();
    assert!(matches!(
        instance_of(&caller, member.cert.expires_at_secs + 1).unwrap_err(),
        MailboxError::NotAnInstance(_)
    ));
}

#[tokio::test]
async fn revoked_instance_key_is_refused() {
    let member = Member::new(3600);
    let instance = member.instance();
    let anchors = Anchors::new();
    anchors.publish(&member.service_id());
    anchors.revoke(&member.service_id(), &instance.instance_did);
    assert!(matches!(
        check_not_revoked(&anchors, &instance).await.unwrap_err(),
        MailboxError::NotAnInstance(_)
    ));
}

#[tokio::test]
async fn unreadable_anchor_returns_internal_error() {
    let member = Member::new(3600);
    let instance = member.instance();
    let anchors = Anchors::new();
    // Do not publish master anchor -> resolve fails
    assert!(matches!(
        check_not_revoked(&anchors, &instance).await.unwrap_err(),
        MailboxError::Internal(_)
    ));
}

#[test]
fn sender_of_checks_auth_level() {
    let member = Member::new(3600);
    let mut caller = member.caller();
    let now = member.cert.issued_at_secs + 10;
    assert_eq!(sender_of(&caller, now).unwrap().master_did, member.service_id());

    caller.auth = AuthLevel::Ucan;
    assert_eq!(sender_of(&caller, now).unwrap().master_did, member.service_id());

    caller.auth = AuthLevel::System;
    assert_eq!(sender_of(&caller, now).unwrap_err(), MailboxError::Unauthenticated);
}
