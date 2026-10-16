use serde_json::{Map, json};
use syneroym_identity::Identity;
use syneroym_passport::{
    Lease, LeaseStore, PassportId, Proof, RELAY_HOME, Registrar, SignerBinding, wire::LeaseInfo,
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
    let res = super::leases(Some(&store)).unwrap();
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
    let res = super::deny(Some(&store), params).await.unwrap();
    assert_eq!(res.payload, json!(1));
    let leases_res = super::leases(Some(&store)).unwrap();
    let list: Vec<LeaseInfo> = serde_json::from_value(leases_res.payload).unwrap();
    assert!(list.is_empty());
    assert!(store.is_denied(&passport_id));
}

#[tokio::test]
async fn deny_with_malformed_id_is_invalid_params() {
    let store = LeaseStore::open_in_memory(&RELAY_HOME).unwrap();
    let params = json!(["not-a-valid-hex-passport-id"]);
    let err = super::deny(Some(&store), params).await.unwrap_err();
    match err {
        RpcError::InvalidParams(msg) => {
            assert!(msg.contains("relay-passport-deny"));
        }
        other => panic!("expected InvalidParams, got {other:?}"),
    }
}

#[tokio::test]
async fn methods_without_store_return_method_not_found() {
    let leases_err = super::leases(None).unwrap_err();
    match leases_err {
        RpcError::MethodNotFound(msg) => {
            assert_eq!(msg, super::NO_PASSPORT_RELAY);
        }
        other => panic!("expected MethodNotFound, got {other:?}"),
    }

    let deny_err = super::deny(None, json!(["00".repeat(32)])).await.unwrap_err();
    match deny_err {
        RpcError::MethodNotFound(msg) => {
            assert_eq!(msg, super::NO_PASSPORT_RELAY);
        }
        other => panic!("expected MethodNotFound, got {other:?}"),
    }
}
