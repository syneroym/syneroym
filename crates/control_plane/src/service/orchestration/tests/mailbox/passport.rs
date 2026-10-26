//! The owner's child token for a mailbox passport: checked at deploy, kept in
//! the service vault, and part of the deploy hash.

use serde_json::Map;
use syneroym_core::mailbox::MAX_CHILD_TOKEN_BYTES;
use syneroym_passport::{MAILBOX_USE, PassportUse, RELAY_HOME, child_token, issue_passport};

use super::*;

const LIFE: u64 = 3_600;

fn now() -> u64 {
    syneroym_passport::now_secs()
}

/// A mailbox host that issues passports to one owner.
struct Operator {
    key: Identity,
    host: String,
    owner: Identity,
}

impl Operator {
    fn new() -> Self {
        let key = Identity::generate().unwrap();
        let host = derive_did_key(&key.public_key());
        Self { key, host, owner: Identity::generate().unwrap() }
    }

    fn route(&self, key: &MailboxSecretKey) -> EndpointMailbox {
        EndpointMailbox { service_id: self.host.clone(), key: key.public_key() }
    }

    fn passport(&self, use_: &PassportUse) -> CapabilityToken {
        issue_passport(
            &self.key,
            &self.host,
            use_,
            &derive_did_key(&self.owner.public_key()),
            Map::new(),
            LIFE,
            now(),
        )
        .unwrap()
    }

    /// A fresh child token of `passport` for `subject`, as JSON.
    fn child(&self, passport: &CapabilityToken, subject: &str, life: u64) -> String {
        let token = child_token(&self.owner, passport, subject, life, now()).unwrap();
        serde_json::to_string(&token).unwrap()
    }
}

fn credentials(key: &MailboxSecretKey, child_token: Option<String>) -> MailboxCredentials {
    MailboxCredentials { child_token, ..credentials_for(key) }
}

fn validate(
    member: &Member,
    operator: &Operator,
    token: Option<String>,
) -> Result<Option<ValidatedMailbox>, String> {
    let key = member.mailbox_key();
    let record = member.record(Some(operator.route(&key)), false);
    validate_mailbox(
        &member.service_id,
        Some(&credentials(&key, token)),
        AppVisibility::Public,
        Some(&record),
        true,
    )
}

// -- validate_mailbox ------------------------------------------------------

#[test]
fn a_good_child_token_is_accepted() {
    let (member, operator) = (Member::new(), Operator::new());
    let token = operator.child(&operator.passport(&MAILBOX_USE), &member.service_id, LIFE);

    let accepted = validate(&member, &operator, Some(token.clone())).unwrap().unwrap();

    assert_eq!(accepted.child_token, Some(token));
}

#[test]
fn a_deploy_without_a_child_token_has_none() {
    let (member, operator) = (Member::new(), Operator::new());
    assert_eq!(validate(&member, &operator, None).unwrap().unwrap().child_token, None);
}

#[test]
fn a_child_token_refusal_never_shows_the_token() {
    let (member, operator) = (Member::new(), Operator::new());
    let other = derive_did_key(&Identity::generate().unwrap().public_key());
    let passport = operator.passport(&MAILBOX_USE);
    let relay = operator.passport(&RELAY_HOME);
    let cases = [
        ("another master", operator.child(&passport, &other, LIFE)),
        ("a relay passport", operator.child(&relay, &member.service_id, LIFE)),
        ("not json", "{not json".to_string()),
        ("too large", "x".repeat(MAX_CHILD_TOKEN_BYTES + 1)),
    ];
    for (name, token) in cases {
        let err = validate(&member, &operator, Some(token.clone())).unwrap_err();
        assert!(err.contains("child token of"), "{name}: {err}");
        assert!(!err.contains(&token[..token.len().min(40)]), "{name}: {err}");
    }
}

#[test]
fn a_child_token_for_another_mailbox_is_refused() {
    let (member, operator) = (Member::new(), Operator::new());
    let elsewhere = Operator::new();
    let token = elsewhere.child(&elsewhere.passport(&MAILBOX_USE), &member.service_id, LIFE);

    let err = validate(&member, &operator, Some(token)).unwrap_err();

    assert!(err.contains("is for mailbox"), "{err}");
    assert!(err.contains(&elsewhere.host) && err.contains(&operator.host), "{err}");
}

#[test]
fn an_expired_child_token_is_refused() {
    let (member, operator) = (Member::new(), Operator::new());
    let passport = operator.passport(&MAILBOX_USE);
    // Signed an hour ago and lived 60 s.
    let mut token = child_token(&operator.owner, &passport, &member.service_id, 60, now()).unwrap();
    token.not_before_secs -= LIFE;
    token.expires_at_secs -= LIFE;
    token.sign(&operator.owner).unwrap();

    let err =
        validate(&member, &operator, Some(serde_json::to_string(&token).unwrap())).unwrap_err();

    assert!(err.contains("refused: expired"), "{err}");
}

// -- the vault -------------------------------------------------------------

async fn deploy_with_token(
    service: &ControlPlaneService,
    member: &Member,
    operator: &Operator,
    token: Option<String>,
) -> Result<(), String> {
    let caller = caller();
    let key = member.mailbox_key();
    let mut manifest = member.manifest(&caller, Some(operator.route(&key)), Some(&key));
    manifest.mailbox = Some(credentials(&key, token));
    service.deploy(member.service_id.clone(), manifest, &caller).await.map(|_| ())
}

#[tokio::test]
async fn a_deploy_stores_the_child_token_and_a_redeploy_without_one_clears_it() {
    let dir = tempfile::tempdir().unwrap();
    let (member, operator) = (Member::new(), Operator::new());
    let service = host_of(&member, dir.path()).await;
    let token = operator.child(&operator.passport(&MAILBOX_USE), &member.service_id, LIFE);

    deploy_with_token(&service, &member, &operator, Some(token.clone())).await.unwrap();
    assert_eq!(service.mailbox_passport(&member.service_id).await.unwrap(), Some(token));

    deploy_with_token(&service, &member, &operator, None).await.unwrap();
    assert_eq!(service.mailbox_passport(&member.service_id).await.unwrap(), None);
    assert!(service.mailbox_key(&member.service_id).await.unwrap().is_some(), "the key stays");
}

#[tokio::test]
async fn an_undeploy_clears_the_child_token() {
    let dir = tempfile::tempdir().unwrap();
    let (member, operator) = (Member::new(), Operator::new());
    let service = host_of(&member, dir.path()).await;
    let token = operator.child(&operator.passport(&MAILBOX_USE), &member.service_id, LIFE);
    deploy_with_token(&service, &member, &operator, Some(token)).await.unwrap();

    service.undeploy(member.service_id.clone(), 0, &caller()).await.unwrap();

    assert_eq!(service.mailbox_passport(&member.service_id).await.unwrap(), None);
}

#[tokio::test]
async fn a_refused_child_token_stores_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (member, operator) = (Member::new(), Operator::new());
    let service = host_of(&member, dir.path()).await;
    let other = derive_did_key(&Identity::generate().unwrap().public_key());
    let token = operator.child(&operator.passport(&MAILBOX_USE), &other, LIFE);

    let err = deploy_with_token(&service, &member, &operator, Some(token)).await.unwrap_err();

    assert!(err.contains("child token of"), "{err}");
    assert!(service.mailbox_key(&member.service_id).await.unwrap().is_none());
    assert_eq!(service.mailbox_passport(&member.service_id).await.unwrap(), None);
}

// -- the deploy hash -------------------------------------------------------

#[tokio::test]
async fn a_redeploy_with_a_new_token_of_the_same_passport_is_a_noop() {
    let dir = tempfile::tempdir().unwrap();
    let (member, operator) = (Member::new(), Operator::new());
    let service = host_of(&member, dir.path()).await;
    let passport = operator.passport(&MAILBOX_USE);
    let first = operator.child(&passport, &member.service_id, LIFE);
    // Signed again a second earlier: the same passport, subject and expiry,
    // but other text.
    let mut again: CapabilityToken = serde_json::from_str(&first).unwrap();
    again.not_before_secs -= 1;
    again.sign(&operator.owner).unwrap();
    let second = serde_json::to_string(&again).unwrap();
    assert_ne!(first, second);

    deploy_with_token(&service, &member, &operator, Some(first.clone())).await.unwrap();
    deploy_with_token(&service, &member, &operator, Some(second)).await.unwrap();

    let stored = service.mailbox_passport(&member.service_id).await.unwrap();
    assert_eq!(stored, Some(first), "the second deploy was skipped as unchanged");
}

#[tokio::test]
async fn a_redeploy_with_a_new_passport_is_not_a_noop() {
    let dir = tempfile::tempdir().unwrap();
    let (member, operator) = (Member::new(), Operator::new());
    let service = host_of(&member, dir.path()).await;
    let first = operator.child(&operator.passport(&MAILBOX_USE), &member.service_id, LIFE);
    // A passport with other facts has another id.
    let mut renewed = operator.passport(&MAILBOX_USE);
    renewed.facts.insert("max_messages".to_string(), 7.into());
    renewed.sign(&operator.key).unwrap();
    let second = operator.child(&renewed, &member.service_id, LIFE);

    deploy_with_token(&service, &member, &operator, Some(first)).await.unwrap();
    deploy_with_token(&service, &member, &operator, Some(second.clone())).await.unwrap();

    assert_eq!(service.mailbox_passport(&member.service_id).await.unwrap(), Some(second));
}

#[test]
fn the_stable_text_of_a_child_token_falls_back_to_the_raw_text() {
    assert_eq!(stable_child_token_for_hash("{not json"), "{not json");
}
