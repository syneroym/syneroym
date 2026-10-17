use syneroym_core::dht_registry::EndpointMailbox;
use syneroym_mailbox_format::MailboxSecretKey;

use super::*;

fn record_of(master: &Identity, claimed_id: Option<&str>, mailbox: bool) -> SignedEndpointInfo {
    let own_id = substrate::derive_did_key(&master.public_key());
    EndpointInfo {
        service_id: claimed_id.unwrap_or(&own_id).to_string(),
        substrate_id: "did:key:zSubstrate".to_string(),
        endpoint_type: EndpointType::Service,
        mechanisms: vec![],
        nickname: None,
        is_private: false,
        ttl: None,
        not_after: u64::MAX / 2,
        generation: 0,
        mailbox: mailbox.then(|| EndpointMailbox {
            service_id: own_id.clone(),
            key: MailboxSecretKey::derive_from_master(master).public_key(),
        }),
    }
    .sign(master)
    .unwrap()
}

#[test]
fn no_current_record_is_fine() {
    assert!(refuse_if_mailbox("did:key:zAny", None).is_ok());
}

#[test]
fn a_record_without_a_mailbox_is_fine() {
    let master = Identity::generate().unwrap();
    let id = substrate::derive_did_key(&master.public_key());
    assert!(refuse_if_mailbox(&id, Some(&record_of(&master, None, false))).is_ok());
}

#[test]
fn a_record_with_a_mailbox_is_refused() {
    let master = Identity::generate().unwrap();
    let id = substrate::derive_did_key(&master.public_key());
    let err = refuse_if_mailbox(&id, Some(&record_of(&master, None, true))).unwrap_err();
    assert!(err.to_string().contains("has a mailbox"), "{err}");
}

#[test]
fn a_record_for_another_service_is_ignored() {
    let master = Identity::generate().unwrap();
    let other = Identity::generate().unwrap();
    let other_id = substrate::derive_did_key(&other.public_key());
    assert!(refuse_if_mailbox(&other_id, Some(&record_of(&master, None, true))).is_ok());
}

#[test]
fn a_record_that_does_not_verify_is_ignored() {
    let master = Identity::generate().unwrap();
    let id = substrate::derive_did_key(&master.public_key());
    let mut record = record_of(&master, None, true);
    record.info.nickname = Some("changed after signing".to_string());
    assert!(refuse_if_mailbox(&id, Some(&record)).is_ok());
}
