use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use serde_json::Map;
use syneroym_core::{
    dht_registry::{MasterAnchorPayload, MasterAnchorResolver},
    host_identity::HostIdentity,
    mailbox::MAX_CHILD_TOKEN_BYTES,
};
use syneroym_identity::{Identity, substrate};
use syneroym_passport::{PassportId, RELAY_HOME, facts, issue_passport};

use super::*;
use crate::test_support::{Anchors, Member, Operator, child_for};

const LIFE: u64 = 3_600;

fn did(identity: &Identity) -> String {
    substrate::derive_did_key(&identity.public_key())
}

struct Fixture {
    operator: Operator,
    owner: Identity,
    anchors: Arc<Anchors>,
    passports: MailboxPassports,
    now: u64,
}

impl Fixture {
    fn new(operator: Operator) -> Self {
        let anchors = Arc::new(Anchors::new());
        let passports = MailboxPassports::in_memory(&operator.host, anchors.clone()).unwrap();
        Self {
            operator,
            owner: Identity::generate().unwrap(),
            anchors,
            passports,
            now: syneroym_passport::now_secs(),
        }
    }

    fn passport(&self, facts: &[(&str, u64)]) -> CapabilityToken {
        self.operator.passport(&self.owner, facts, LIFE, self.now)
    }

    fn child(&self, passport: &CapabilityToken, member: &Member) -> String {
        child_for(&self.owner, passport, member, LIFE, self.now)
    }

    async fn admit(&self, member: &Member, token: &str) -> Result<Lease, MailboxError> {
        self.passports.admit(&member.instance(), member.cert.clone(), token, self.now).await
    }
}

fn code_of(result: Result<Lease, MailboxError>) -> &'static str {
    match result.unwrap_err() {
        MailboxError::PassportRefused { code, .. } => code,
        other => panic!("expected a passport refusal, got {other:?}"),
    }
}

#[tokio::test]
async fn a_valid_child_token_grants_a_lease_for_the_master() {
    let f = Fixture::new(Operator::unowned());
    let member = Member::new(LIFE);
    let passport = f.passport(&[(facts::MAX_MESSAGES, 9)]);
    let lease = f.admit(&member, &f.child(&passport, &member)).await.unwrap();

    assert_eq!(lease.subject, member.service_id());
    assert_eq!(lease.owner_did, did(&f.owner));
    assert_eq!(lease.limit(facts::MAX_MESSAGES), Some(9));
    assert_eq!(f.passports.leases().lease_for(&member.service_id(), f.now), Some(lease));
}

#[tokio::test]
async fn a_host_with_a_controller_accepts_the_controller_passport() {
    let f = Fixture::new(Operator::controlled());
    let member = Member::new(LIFE);
    let passport = f.passport(&[]);
    assert!(f.admit(&member, &f.child(&passport, &member)).await.is_ok());
}

#[tokio::test]
async fn a_child_token_for_another_master_is_not_bound() {
    let f = Fixture::new(Operator::unowned());
    let (member, other) = (Member::new(LIFE), Member::new(LIFE));
    let passport = f.passport(&[]);
    let token_for_other = f.child(&passport, &other);
    assert_eq!(code_of(f.admit(&member, &token_for_other).await), "not_bound");
}

#[tokio::test]
async fn a_leaked_child_token_alone_is_not_enough() {
    // The thief has an instance certificate of their own master, not of the
    // master the token names.
    let f = Fixture::new(Operator::unowned());
    let (victim, thief) = (Member::new(LIFE), Member::new(LIFE));
    let passport = f.passport(&[]);
    let leaked = f.child(&passport, &victim);
    assert_eq!(code_of(f.admit(&thief, &leaked).await), "not_bound");

    // The thief's instance key with the victim's certificate: the
    // certificate names another instance key.
    let result = f.passports.admit(&thief.instance(), victim.cert.clone(), &leaked, f.now).await;
    assert_eq!(code_of(result), "not_bound");
}

#[tokio::test]
async fn a_passport_from_another_root_is_untrusted() {
    let f = Fixture::new(Operator::unowned());
    let member = Member::new(LIFE);
    let stranger = Operator::unowned();
    let passport = issue_passport(
        &stranger.root,
        &f.operator.host.node_did,
        &MAILBOX_USE,
        &did(&f.owner),
        Map::new(),
        LIFE,
        f.now,
    )
    .unwrap();
    assert_eq!(code_of(f.admit(&member, &f.child(&passport, &member)).await), "untrusted_root");
}

#[tokio::test]
async fn a_host_with_a_controller_refuses_a_node_key_passport() {
    let f = Fixture::new(Operator::controlled());
    let member = Member::new(LIFE);
    // Signed by the node key, not by the controller.
    let node_key = Operator::unowned();
    let passport = issue_passport(
        &node_key.root,
        &f.operator.host.node_did,
        &MAILBOX_USE,
        &did(&f.owner),
        Map::new(),
        LIFE,
        f.now,
    )
    .unwrap();
    assert_eq!(code_of(f.admit(&member, &f.child(&passport, &member)).await), "untrusted_root");
}

#[tokio::test]
async fn an_expired_passport_is_refused() {
    let f = Fixture::new(Operator::unowned());
    // The certificate outlives the passport, so the passport is what ends.
    let member = Member::new(LIFE * 2);
    let passport = f.passport(&[]);
    let token = f.child(&passport, &member);
    let late = f.now + LIFE + 1;
    let result = f.passports.admit(&member.instance(), member.cert.clone(), &token, late).await;
    assert_eq!(code_of(result), "expired");
}

#[tokio::test]
async fn an_expired_child_token_is_refused() {
    let f = Fixture::new(Operator::unowned());
    let member = Member::new(LIFE * 2);
    let passport = f.passport(&[]);
    let token = child_for(&f.owner, &passport, &member, 60, f.now);
    let result =
        f.passports.admit(&member.instance(), member.cert.clone(), &token, f.now + 61).await;
    assert_eq!(code_of(result), "expired");
}

#[tokio::test]
async fn a_relay_passport_is_the_wrong_capability() {
    let f = Fixture::new(Operator::unowned());
    let member = Member::new(LIFE);
    let passport = issue_passport(
        &f.operator.root,
        &f.operator.host.node_did,
        &RELAY_HOME,
        &did(&f.owner),
        Map::new(),
        LIFE,
        f.now,
    )
    .unwrap();
    assert_eq!(code_of(f.admit(&member, &f.child(&passport, &member)).await), "wrong_capability");
}

#[tokio::test]
async fn max_services_admits_one_master_and_renews_it() {
    let f = Fixture::new(Operator::unowned());
    let (first, second) = (Member::new(LIFE), Member::new(LIFE));
    let passport = f.passport(&[(facts::MAX_SERVICES, 1)]);

    f.admit(&first, &f.child(&passport, &first)).await.unwrap();
    assert_eq!(code_of(f.admit(&second, &f.child(&passport, &second)).await), "limit_reached");
    f.admit(&first, &f.child(&passport, &first)).await.expect("the same master renews");
}

#[tokio::test]
async fn a_revoked_owner_to_master_edge_is_refused() {
    let f = Fixture::new(Operator::unowned());
    let member = Member::new(LIFE);
    let passport = f.passport(&[]);
    f.anchors.revoke(&did(&f.owner), &member.service_id());
    assert_eq!(code_of(f.admit(&member, &f.child(&passport, &member)).await), "revoked");
    assert!(f.passports.leases().lease_for(&member.service_id(), f.now).is_none());
}

/// A resolver that never answers in time.
#[derive(Debug)]
struct SlowResolver;

#[async_trait::async_trait]
impl MasterAnchorResolver for SlowResolver {
    async fn resolve_master_anchor(&self, _: &str) -> Result<MasterAnchorPayload, anyhow::Error> {
        tokio::time::sleep(Duration::from_secs(60)).await;
        Ok(MasterAnchorPayload::default())
    }
}

#[tokio::test(start_paused = true)]
async fn a_revocation_lookup_that_times_out_still_grants() {
    let operator = Operator::unowned();
    let passports = MailboxPassports::in_memory(&operator.host, Arc::new(SlowResolver)).unwrap();
    let (owner, member) = (Identity::generate().unwrap(), Member::new(LIFE));
    let now = syneroym_passport::now_secs();
    let passport = operator.passport(&owner, &[], LIFE, now);
    let token = child_for(&owner, &passport, &member, LIFE, now);
    let lease = passports.admit(&member.instance(), member.cert.clone(), &token, now).await;
    assert!(lease.is_ok(), "{lease:?}");
}

#[tokio::test]
async fn a_denied_passport_is_refused() {
    let f = Fixture::new(Operator::unowned());
    let member = Member::new(LIFE);
    let passport = f.passport(&[]);
    let id = PassportId::of(&passport).unwrap();
    f.passports.leases().deny(&id, f.now).unwrap();
    assert_eq!(code_of(f.admit(&member, &f.child(&passport, &member)).await), "denied");
}

#[tokio::test]
async fn a_token_over_the_size_limit_or_with_bad_json_is_malformed() {
    let f = Fixture::new(Operator::unowned());
    let member = Member::new(LIFE);
    assert_eq!(code_of(f.admit(&member, "{not json").await), "malformed");
    let huge = format!("\"{}\"", "a".repeat(MAX_CHILD_TOKEN_BYTES));
    assert_eq!(code_of(f.admit(&member, &huge).await), "malformed");
}

#[tokio::test]
async fn a_lease_ends_with_the_passport() {
    let f = Fixture::new(Operator::unowned());
    let member = Member::new(LIFE);
    let passport = f.passport(&[]);
    let lease = f.admit(&member, &f.child(&passport, &member)).await.unwrap();
    assert_eq!(lease.lease_ends_at, f.now + LIFE);
    assert!(f.passports.leases().lease_for(&member.service_id(), lease.lease_ends_at).is_none());
}

#[tokio::test]
async fn a_revoked_edge_ends_the_running_lease() {
    let f = Fixture::new(Operator::unowned());
    let member = Member::new(LIFE);
    let passport = f.passport(&[(facts::MAX_MESSAGES, 9)]);
    let token = f.child(&passport, &member);
    f.admit(&member, &token).await.unwrap();
    assert!(f.passports.leases().lease_for(&member.service_id(), f.now).is_some());

    f.anchors.revoke(&did(&f.owner), &member.service_id());
    assert_eq!(code_of(f.admit(&member, &token).await), "revoked");

    assert!(
        f.passports.leases().lease_for(&member.service_id(), f.now).is_none(),
        "the refused master must not keep the limits of its old lease"
    );
}

#[tokio::test]
async fn a_changed_root_ends_the_lease_of_the_old_passport() {
    let f = Fixture::new(Operator::unowned());
    let member = Member::new(LIFE);
    let passport = f.passport(&[]);
    let token = f.child(&passport, &member);
    f.admit(&member, &token).await.unwrap();

    // The host gains a verified controller: the node-key passport is
    // untrusted now, but its lease is still in the file.
    let controller = did(&Identity::generate().unwrap());
    let host = HostIdentity { verified_controller: Some(controller), ..f.operator.host.clone() };
    let restarted =
        MailboxPassports::with_leases(&host, f.anchors.clone(), f.passports.leases().clone())
            .unwrap();
    let result = restarted.admit(&member.instance(), member.cert.clone(), &token, f.now).await;

    assert_eq!(code_of(result), "untrusted_root");
    assert!(restarted.leases().lease_for(&member.service_id(), f.now).is_none());
}

#[tokio::test]
async fn a_refusal_that_is_not_definitive_keeps_the_lease() {
    let f = Fixture::new(Operator::unowned());
    let (member, thief) = (Member::new(LIFE), Member::new(LIFE));
    let passport = f.passport(&[]);
    let token = f.child(&passport, &member);
    f.admit(&member, &token).await.unwrap();

    // Another master shows the token: not bound, and the lease stays.
    assert_eq!(code_of(f.admit(&thief, &token).await), "not_bound");

    assert!(f.passports.leases().lease_for(&member.service_id(), f.now).is_some());
}

#[derive(Debug, Default)]
struct CountingResolver {
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl MasterAnchorResolver for CountingResolver {
    async fn resolve_master_anchor(&self, _: &str) -> Result<MasterAnchorPayload, anyhow::Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(MasterAnchorPayload::default())
    }
}

#[tokio::test]
async fn a_denied_passport_is_refused_before_the_network_lookup() {
    let operator = Operator::unowned();
    let resolver = Arc::new(CountingResolver::default());
    let passports = MailboxPassports::in_memory(&operator.host, resolver.clone()).unwrap();
    let (owner, member) = (Identity::generate().unwrap(), Member::new(LIFE));
    let now = syneroym_passport::now_secs();
    let passport = operator.passport(&owner, &[], LIFE, now);
    passports.leases().deny(&PassportId::of(&passport).unwrap(), now).unwrap();
    let token = child_for(&owner, &passport, &member, LIFE, now);

    let result = passports.admit(&member.instance(), member.cert.clone(), &token, now).await;

    assert_eq!(code_of(result), "denied");
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 0, "no lookup for a denied passport");
}
