//! The mailbox in member records, and the per-service certificate lifetime.

use syneroym_core::dht_registry::EndpointMailbox;
use syneroym_mailbox_format::MailboxSecretKey;

use super::*;

fn route_for(master: &Identity) -> EndpointMailbox {
    EndpointMailbox {
        service_id: substrate::derive_did_key(&Identity::generate().unwrap().public_key()),
        key: MailboxSecretKey::derive_from_master(master).public_key(),
    }
}

#[test]
fn a_public_member_record_carries_its_mailbox() {
    let master = Identity::generate().unwrap();
    let service_id = substrate::derive_did_key(&master.public_key());
    let route = route_for(&master);

    let json = member_registry_record(
        Visibility::Public,
        &service_id,
        "did:key:zSub",
        &master,
        9_999_999_999,
        Some(route.clone()),
    )
    .unwrap()
    .expect("public must mint a record");

    let signed: SignedEndpointInfo = serde_json::from_str(&json).unwrap();
    assert!(signed.verify().is_ok());
    assert_eq!(signed.info.mailbox, Some(route));
}

#[test]
fn a_mailbox_on_a_member_that_is_not_public_is_refused() {
    let master = Identity::generate().unwrap();
    let service_id = substrate::derive_did_key(&master.public_key());
    for visibility in [Visibility::Internal, Visibility::Private] {
        let err = member_registry_record(
            visibility,
            &service_id,
            "did:key:zSub",
            &master,
            9_999_999_999,
            Some(route_for(&master)),
        )
        .unwrap_err();
        assert!(err.to_string().contains("only a 'public' service"), "{err}");
    }
}

#[test]
fn the_certificate_lifetime_is_the_services_own_else_the_default() {
    let mut member = service("web", None);
    assert_eq!(member_cert_hours(&member, 24), 24);
    member.instance_cert_expires_hours = Some(168);
    assert_eq!(member_cert_hours(&member, 24), 168);
}
