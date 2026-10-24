use serde_json::{Map, json};
use syneroym_identity::{DelegationCertificate, Identity, delegation::SCOPE_SERVICE_INSTANCE};
use syneroym_passport::{
    Lease, LeaseStore, MAILBOX_USE, PassportId, Proof, RELAY_HOME, Registrar, SignerBinding,
    wire::LeaseInfo,
};
use syneroym_rpc::RpcError;

fn did(id: &Identity) -> String {
    syneroym_identity::substrate::derive_did_key(&id.public_key())
}

fn grant_test_lease(store: &LeaseStore) -> (PassportId, Lease) {
    let operator = Identity::generate().unwrap();
    let host = did(&operator);
    let owner = Identity::generate().unwrap();
    let node = Identity::generate().unwrap();
    let now = syneroym_passport::now_secs();
    let passport = syneroym_passport::issue_passport(
        &operator,
        &host,
        &RELAY_HOME,
        &did(&owner),
        Map::new(),
        3600,
        now,
    )
    .unwrap();
    let passport_id = PassportId::of(&passport).unwrap();
    let token = syneroym_passport::child_token(&owner, &passport, &did(&node), 3600, now).unwrap();
    let registrar =
        Registrar::new(RELAY_HOME, host.clone(), vec![host.clone()], store.clone()).unwrap();
    let challenge = registrar.issue_challenge(now).unwrap();
    let signature =
        syneroym_passport::sign_challenge(&node, &host, &RELAY_HOME, &challenge, &token).unwrap();
    let proof = Proof::Challenge { challenge, signer_did: did(&node), signature };
    let checked = registrar.check(&token, &proof, &SignerBinding::SameKey, now).unwrap();
    let lease = registrar.grant(checked, now).unwrap();
    (passport_id, lease)
}

#[tokio::test]
async fn leases_lists_granted_lease() {
    let store = LeaseStore::open_in_memory(&RELAY_HOME).unwrap();
    let (passport_id, lease) = grant_test_lease(&store);
    let res = super::leases(super::RELAY, Some(&store)).unwrap();
    let list: Vec<LeaseInfo> = serde_json::from_value(res.payload).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].passport_id, passport_id);
    assert_eq!(list[0].owner_did, lease.owner_did);
    assert_eq!(list[0].subject, lease.subject);
    assert_eq!(list[0].lease_ends_at, lease.lease_ends_at);
    assert_eq!(list[0].registered_at, lease.registered_at);
}

#[tokio::test]
async fn deny_revokes_lease_and_records_denial() {
    let store = LeaseStore::open_in_memory(&RELAY_HOME).unwrap();
    let (passport_id, _) = grant_test_lease(&store);
    let params = json!([passport_id.as_str()]);
    let res = super::deny(super::RELAY, Some(&store), params).await.unwrap();
    assert_eq!(res.payload, json!(1));
    let leases_res = super::leases(super::RELAY, Some(&store)).unwrap();
    let list: Vec<LeaseInfo> = serde_json::from_value(leases_res.payload).unwrap();
    assert!(list.is_empty());
    assert!(store.is_denied(&passport_id));
}

#[tokio::test]
async fn deny_with_malformed_id_is_invalid_params() {
    let store = LeaseStore::open_in_memory(&RELAY_HOME).unwrap();
    let params = json!(["not-a-valid-hex-passport-id"]);
    let err = super::deny(super::RELAY, Some(&store), params).await.unwrap_err();
    match err {
        RpcError::InvalidParams(msg) => {
            assert!(msg.contains("relay-passport-deny"));
        }
        other => panic!("expected InvalidParams, got {other:?}"),
    }
}

#[tokio::test]
async fn methods_without_store_return_method_not_found() {
    let leases_err = super::leases(super::RELAY, None).unwrap_err();
    match leases_err {
        RpcError::MethodNotFound(msg) => {
            assert_eq!(msg, super::NO_PASSPORT_RELAY);
        }
        other => panic!("expected MethodNotFound, got {other:?}"),
    }

    let deny_err = super::deny(super::RELAY, None, json!(["00".repeat(32)])).await.unwrap_err();
    match deny_err {
        RpcError::MethodNotFound(msg) => {
            assert_eq!(msg, super::NO_PASSPORT_RELAY);
        }
        other => panic!("expected MethodNotFound, got {other:?}"),
    }
}

/// A lease of the mailbox use, granted the way the mailbox grants it: the
/// transport proves the instance key and its certificate binds it to the
/// master.
fn grant_mailbox_lease(store: &LeaseStore) -> PassportId {
    let operator = Identity::generate().unwrap();
    let host = did(&operator);
    let owner = Identity::generate().unwrap();
    let (master, instance) = (Identity::generate().unwrap(), Identity::generate().unwrap());
    let now = syneroym_passport::now_secs();
    let passport = syneroym_passport::issue_passport(
        &operator,
        &host,
        &MAILBOX_USE,
        &did(&owner),
        Map::new(),
        3600,
        now,
    )
    .unwrap();
    let token =
        syneroym_passport::child_token(&owner, &passport, &did(&master), 3600, now).unwrap();
    let cert = DelegationCertificate::issue(
        &master,
        instance.public_key(),
        3600,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    let registrar = Registrar::new(MAILBOX_USE, host.clone(), vec![host], store.clone()).unwrap();
    let proof = Proof::Transport { verified_did: did(&instance) };
    let checked =
        registrar.check(&token, &proof, &SignerBinding::InstanceCertificate(cert), now).unwrap();
    registrar.grant(checked, now).unwrap();
    PassportId::of(&passport).unwrap()
}

#[tokio::test]
async fn the_mailbox_pair_lists_and_denies_a_lease() {
    let store = LeaseStore::open_in_memory(&MAILBOX_USE).unwrap();
    let passport_id = grant_mailbox_lease(&store);

    let res = super::leases(super::MAILBOX, Some(&store)).unwrap();
    let list: Vec<LeaseInfo> = serde_json::from_value(res.payload).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].passport_id, passport_id);

    let params = json!([passport_id.as_str()]);
    let res = super::deny(super::MAILBOX, Some(&store), params).await.unwrap();
    assert_eq!(res.payload, json!(1));
    let res = super::leases(super::MAILBOX, Some(&store)).unwrap();
    let list: Vec<LeaseInfo> = serde_json::from_value(res.payload).unwrap();
    assert!(list.is_empty());
    assert!(store.is_denied(&passport_id));
}

#[tokio::test]
async fn the_mailbox_pair_without_a_mailbox_is_method_not_found() {
    let err = super::leases(super::MAILBOX, None).unwrap_err();
    assert!(matches!(&err, RpcError::MethodNotFound(m) if m == super::NO_PASSPORT_MAILBOX));
    let err = super::deny(super::MAILBOX, None, json!(["00".repeat(32)])).await.unwrap_err();
    assert!(matches!(&err, RpcError::MethodNotFound(m) if m == super::NO_PASSPORT_MAILBOX));
}

#[tokio::test]
async fn a_malformed_mailbox_deny_names_its_own_verb() {
    let store = LeaseStore::open_in_memory(&MAILBOX_USE).unwrap();
    let err = super::deny(super::MAILBOX, Some(&store), json!(["nope"])).await.unwrap_err();
    assert!(matches!(&err, RpcError::InvalidParams(m) if m.contains("mailbox-passport-deny")));
}
