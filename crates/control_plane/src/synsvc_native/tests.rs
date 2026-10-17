#![allow(clippy::cognitive_complexity)]

use syneroym_core::record_signer::SigningPrincipal;
use syneroym_data_blob::ObjectStoreBlobProvider;
use syneroym_data_db::{SqliteStorageProvider, vault_names};
use syneroym_mqtt_broker::MqttBrokerConfig;
use syneroym_rpc::{CallerContext, PERMISSION_DENIED_CODE};
use syneroym_wit_interfaces::host::syneroym::{
    data_layer::store::DataLayerError, vault::vault::VaultError,
};

use super::*;

/// `permission-denied`/`quota-exceeded` are not reachable end to end
/// through the HTTP bridge's own `get`/`query`/`put`/`patch`
/// routes (see the module doc on `data_layer_error`) -- unit-tested
/// directly here instead, since there is no end-to-end path that
/// produces them.
#[test]
fn data_layer_error_maps_every_variant_to_a_distinguishable_code() {
    // A `const` cannot appear in a match pattern, so this literal stays
    // in sync with `syneroym_rpc::PERMISSION_DENIED_CODE` by hand.
    assert!(matches!(
        data_layer_error(DataLayerError::PermissionDenied),
        RpcError::Custom(-32010, _, _)
    ));
    assert!(matches!(
        data_layer_error(DataLayerError::CollectionNotFound),
        RpcError::Custom(-32011, _, _)
    ));
    let RpcError::Custom(-32012, msg, _) =
        data_layer_error(DataLayerError::SchemaViolation("bad field".to_string()))
    else {
        panic!("schema-violation must map to Custom(-32012, ..)");
    };
    assert_eq!(msg, "bad field");
    assert!(matches!(
        data_layer_error(DataLayerError::QuotaExceeded),
        RpcError::Custom(-32013, _, _)
    ));
    assert!(matches!(
        data_layer_error(DataLayerError::Internal("boom".to_string())),
        RpcError::InternalError(_)
    ));
}

#[test]
fn parse_principal_handles_service_and_delegated() {
    let p1 = parse_principal(&serde_json::json!("service")).unwrap();
    assert!(matches!(p1, SigningPrincipal::Service));

    let p2 = parse_principal(&serde_json::json!({"delegated": "cert_json"})).unwrap();
    assert!(
        matches!(p2, SigningPrincipal::Delegated { delegation_json } if delegation_json == "cert_json")
    );

    assert!(parse_principal(&serde_json::json!("invalid")).is_err());
}

#[test]
fn admit_privileged_capability_admits_self_and_owner_refuses_stranger() {
    let node_id = Arc::new(Identity::generate().unwrap());
    let ks = Arc::new(KeyStore::new());
    let tmp = tempfile::tempdir().unwrap();
    let sp: Arc<dyn StorageProvider> =
        Arc::new(syneroym_data_db::SqliteStorageProvider::new(tmp.path(), true).unwrap());
    let bp: Arc<dyn BlobProvider> =
        Arc::new(syneroym_data_blob::ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let mb = Arc::new(MqttBroker::new(syneroym_mqtt_broker::MqttBrokerConfig::default()).unwrap());

    let svc = SynSvcNativeService::new(
        "did:key:zTestSvc".to_string(),
        ks,
        sp,
        bp,
        mb,
        None,
        node_id,
        "did:key:zOwner",
        empty_service_proxy(),
        syneroym_rpc::empty_row_authorizer(),
        None,
    );

    let system_caller = syneroym_rpc::CallerContext {
        caller_did: "system:did:key:zTestSvc".to_string(),
        app_instance: None,
        session: Default::default(),
        auth: syneroym_rpc::AuthLevel::Delegated,
        proof: None,
    };
    assert!(svc.admit_privileged_capability(&system_caller).is_ok());

    let stranger_caller = syneroym_rpc::CallerContext {
        caller_did: "did:key:zStranger".to_string(),
        app_instance: None,
        session: Default::default(),
        auth: syneroym_rpc::AuthLevel::Delegated,
        proof: None,
    };
    let err = svc.admit_privileged_capability(&stranger_caller).unwrap_err();
    assert!(matches!(err, RpcError::Custom(PERMISSION_DENIED_CODE, _, _)));
}

#[test]
fn signing_error_maps_every_variant_to_expected_rpc_error() {
    use syneroym_core::record_signer::SigningError as SE;
    assert!(matches!(
        signing_error(SE::NoDelegation("no cert".to_string())),
        RpcError::Custom(-32030, _, _)
    ));
    assert!(matches!(
        signing_error(SE::InvalidRecord("bad record".to_string())),
        RpcError::InvalidParams(_)
    ));
    assert!(matches!(
        signing_error(SE::PermissionDenied),
        RpcError::Custom(PERMISSION_DENIED_CODE, _, _)
    ));
    assert!(matches!(
        signing_error(SE::Internal("internal err".to_string())),
        RpcError::InternalError(_)
    ));
}

/// The substrate keeps some vault names for itself. The native guest door
/// answers `not-found` for them even when the row exists, and still reveals
/// an ordinary name.
#[tokio::test]
async fn vault_reveal_refuses_reserved_names_even_when_the_row_exists() {
    let service_id = "did:key:zTestSvc";
    let ks = Arc::new(KeyStore::new());
    let tmp = tempfile::tempdir().unwrap();
    let sp: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(tmp.path(), false).unwrap());
    let store = sp.open_service_db(service_id, &ks).await.unwrap();
    store.write_secret(vault_names::MAILBOX_SECRET, b"seed").await.unwrap();
    store.write_secret("ordinary", b"visible").await.unwrap();
    let bp: Arc<dyn BlobProvider> = Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let mb = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let svc = SynSvcNativeService::new(
        service_id.to_string(),
        ks,
        sp,
        bp,
        mb,
        None,
        Arc::new(Identity::generate().unwrap()),
        "did:key:zOwner",
        empty_service_proxy(),
        syneroym_rpc::empty_row_authorizer(),
        None,
    );
    let reveal = |key: &'static str| {
        svc.dispatch_vault(NativeInvocation {
            caller: CallerContext::service_system(service_id),
            interface: "vault".to_string(),
            method: "reveal".to_string(),
            params: serde_json::json!({ "key": key }),
        })
    };

    let reserved = reveal(vault_names::MAILBOX_SECRET).await.unwrap_err();
    assert!(
        matches!(&reserved, RpcError::InternalError(m) if m == &VaultError::NotFound.to_string()),
        "{reserved:?}"
    );
    let ordinary = reveal("ordinary").await.unwrap();
    assert_eq!(serde_json::from_value::<Vec<u8>>(ordinary.payload).unwrap(), b"visible".to_vec());
}
