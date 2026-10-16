use syneroym_ucan::{Ability, Capability, ResourceUri};

use super::*;

const HOST: &str = "did:key:hhost";

fn cap(with: &str, can: &str) -> Capability {
    Capability { with: ResourceUri(with.to_string()), can: Ability(can.to_string()), caveats: None }
}

#[test]
fn resource_is_the_selector_form() {
    assert_eq!(RELAY_HOME.resource(HOST).0, "substrate:did:key:hhost/relay");
    assert_eq!(MAILBOX_USE.resource(HOST).0, "substrate:did:key:hhost/mailbox");
}

#[test]
fn names_exactly_accepts_its_own_capability() {
    assert!(RELAY_HOME.names_exactly(&RELAY_HOME.capability(HOST), HOST));
    assert!(MAILBOX_USE.names_exactly(&MAILBOX_USE.capability(HOST), HOST));
}

#[test]
fn names_exactly_refuses_everything_else() {
    let relay = "substrate:did:key:hhost/relay";
    let refused = [
        cap(relay, Ability::SUBSTRATE_ADMIN),
        cap("substrate:did:key:hhost", Ability::RELAY_HOME),
        cap("substrate:did:key:hhost", Ability::SUBSTRATE_ADMIN),
        cap("substrate:did:key:hother/relay", Ability::RELAY_HOME),
        cap("substrate:did:key:hhost/relay/x", Ability::RELAY_HOME),
        cap(relay, Ability::MAILBOX_USE),
        cap("substrate:did:key:hhost/mailbox", Ability::RELAY_HOME),
    ];
    for c in &refused {
        assert!(!RELAY_HOME.names_exactly(c, HOST), "{c:?} must not name relay/home exactly");
    }
}

#[test]
fn with_max_lease_secs_changes_only_the_lease_limit() {
    let short = RELAY_HOME.with_max_lease_secs(60);
    assert_eq!(short.max_lease_secs, 60);
    assert_eq!(short.ability, RELAY_HOME.ability);
}

#[test]
fn host_of_extracts_host_did() {
    let res = RELAY_HOME.resource(HOST);
    assert_eq!(RELAY_HOME.host_of(&res), Some(HOST));
    assert_eq!(MAILBOX_USE.host_of(&MAILBOX_USE.resource(HOST)), Some(HOST));

    assert_eq!(RELAY_HOME.host_of(&ResourceUri("substrate:did:key:hhost".to_string())), None);
    assert_eq!(RELAY_HOME.host_of(&MAILBOX_USE.resource(HOST)), None);
    assert_eq!(RELAY_HOME.host_of(&ResourceUri("substrate:/relay".to_string())), None);
}
