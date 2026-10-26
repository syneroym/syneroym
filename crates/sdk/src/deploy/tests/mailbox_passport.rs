//! Mailbox passports at deploy: reading a passport and signing child tokens.

use serde_json::Map;
use syneroym_app_orchestration::models::MailboxChoice;
use syneroym_passport::{
    CapabilityToken, MAILBOX_USE, RELAY_HOME, child_token, inspect_child_token, issue_passport,
};

use super::*;

const LIFE: u64 = 3_600;

fn did_of(identity: &Identity) -> String {
    substrate::derive_did_key(&identity.public_key())
}

fn now() -> u64 {
    syneroym_passport::now_secs()
}

struct World {
    operator: Identity,
    owner: Identity,
    host: String,
}

impl World {
    fn new() -> Self {
        let operator = Identity::generate().unwrap();
        let host = did_of(&operator);
        Self { operator, owner: Identity::generate().unwrap(), host }
    }

    fn passport_json(&self) -> String {
        let passport = issue_passport(
            &self.operator,
            &self.host,
            &MAILBOX_USE,
            &did_of(&self.owner),
            Map::new(),
            LIFE,
            now(),
        )
        .unwrap();
        serde_json::to_string(&passport).unwrap()
    }

    fn passport(&self) -> MailboxPassport {
        MailboxPassport::parse(&self.passport_json(), &did_of(&self.owner), now()).unwrap()
    }
}

/// A member with a real master DID, so its child token can be made.
fn member_on(host: Option<&str>) -> PlannedService {
    let master = Identity::generate().unwrap();
    let mut svc = service("member", None);
    svc.service_id = ServiceId::new(did_of(&master));
    svc.mailbox = host.map(|h| MailboxChoice::Service(ServiceId::new(h)));
    svc
}

#[test]
fn parse_reads_a_passport_for_its_owner() {
    let w = World::new();
    let passport = w.passport();
    assert_eq!(passport.host(), w.host);
}

#[test]
fn parse_refuses_another_owner() {
    let w = World::new();
    let stranger = did_of(&Identity::generate().unwrap());
    let err = MailboxPassport::parse(&w.passport_json(), &stranger, now()).unwrap_err();
    assert!(err.to_string().contains("not to the owner"), "{err}");
}

#[test]
fn parse_refuses_a_relay_passport() {
    let w = World::new();
    let relay = issue_passport(
        &w.operator,
        &w.host,
        &RELAY_HOME,
        &did_of(&w.owner),
        Map::new(),
        LIFE,
        now(),
    )
    .unwrap();
    let json = serde_json::to_string(&relay).unwrap();
    let err = MailboxPassport::parse(&json, &did_of(&w.owner), now()).unwrap_err();
    assert!(err.to_string().contains("not for"), "{err}");
}

#[test]
fn parse_refuses_an_expired_passport() {
    let w = World::new();
    let err = MailboxPassport::parse(&w.passport_json(), &did_of(&w.owner), now() + LIFE + 1)
        .unwrap_err();
    assert!(err.to_string().contains("expired"), "{err}");
}

#[test]
fn parse_refuses_a_child_token_in_place_of_a_passport() {
    let w = World::new();
    let passport: CapabilityToken = serde_json::from_str(&w.passport_json()).unwrap();
    let child =
        child_token(&w.owner, &passport, &did_of(&Identity::generate().unwrap()), LIFE, now())
            .unwrap();
    let json = serde_json::to_string(&child).unwrap();
    let err = MailboxPassport::parse(&json, &did_of(&w.owner), now()).unwrap_err();
    assert!(err.to_string().contains("not a mailbox passport"), "{err}");
}

#[test]
fn parse_refuses_text_that_is_not_a_token() {
    let err = MailboxPassport::parse("{nope", "did:key:hx", now()).unwrap_err();
    assert!(err.to_string().contains("does not parse"), "{err}");
}

#[test]
fn the_child_token_passes_the_offline_check_and_ends_with_the_passport() {
    let w = World::new();
    let master = Identity::generate().unwrap();
    let master_did = did_of(&master);
    let now = now();

    let json = w.passport().child_token_for(&w.owner, &master_did, now).unwrap();

    let token: CapabilityToken = serde_json::from_str(&json).unwrap();
    let presented = inspect_child_token(&token, &MAILBOX_USE, &master_did, now).unwrap();
    assert_eq!(presented.host_node_did, w.host);
    assert_eq!(token.expires_at_secs, token.proofs[0].expires_at_secs);
}

#[test]
fn one_token_is_made_for_each_service_of_the_matching_mailbox() {
    let w = World::new();
    let other_host = did_of(&Identity::generate().unwrap());
    let (a, b, elsewhere, none) = (
        member_on(Some(&w.host)),
        member_on(Some(&w.host)),
        member_on(Some(&other_host)),
        member_on(None),
    );
    let plan = plan(vec![a.clone(), b.clone(), elsewhere, none]);

    let tokens = child_tokens_for(&plan, &w.owner, &[w.passport()], now()).unwrap();

    assert_eq!(tokens.keys().collect::<Vec<_>>().len(), 2);
    for member in [&a, &b] {
        let token: CapabilityToken = serde_json::from_str(&tokens[&member.service_id]).unwrap();
        assert_eq!(token.audience_did, member.service_id.as_str());
    }
}

#[test]
fn a_passport_that_matches_no_service_is_refused() {
    let w = World::new();
    let other_host = did_of(&Identity::generate().unwrap());
    let plan = plan(vec![member_on(Some(&other_host))]);
    let err = child_tokens_for(&plan, &w.owner, &[w.passport()], now()).unwrap_err();
    assert!(err.to_string().contains("matches no service"), "{err}");
}

#[test]
fn two_passports_for_one_mailbox_are_refused() {
    let w = World::new();
    let plan = plan(vec![member_on(Some(&w.host))]);
    let err = child_tokens_for(&plan, &w.owner, &[w.passport(), w.passport()], now()).unwrap_err();
    assert!(err.to_string().contains("two --mailbox-passport files"), "{err}");
}

#[test]
fn an_unpinned_auto_mailbox_is_refused() {
    let w = World::new();
    let mut auto = member_on(None);
    auto.mailbox = Some(MailboxChoice::Auto);
    let err = child_tokens_for(&plan(vec![auto]), &w.owner, &[w.passport()], now()).unwrap_err();
    assert!(err.to_string().contains("pin it"), "{err}");
}

#[test]
fn a_child_token_by_another_identity_than_the_owner_is_refused() {
    let w = World::new();
    let thief = Identity::generate().unwrap();
    let err = w
        .passport()
        .child_token_for(&thief, &did_of(&Identity::generate().unwrap()), now())
        .unwrap_err();
    assert!(err.to_string().contains("was issued to"), "{err}");
}
