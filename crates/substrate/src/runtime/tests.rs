use std::sync::Arc;

use syneroym_app_orchestration::{AppScope, ServiceId, TopologyEpoch, TopologyMode};
use syneroym_core::storage::MockStorage;
use syneroym_identity::{DelegationCertificate, delegation::SCOPE_SERVICE_INSTANCE};

use super::*;

/// A certificate within 25% of its lifetime of expiring is flagged;
/// one nowhere near expiry is not.
#[tokio::test]
async fn a_certificate_near_expiry_is_warned_about_on_the_heartbeat_sweep() {
    let registry = EndpointRegistry::new(Arc::new(MockStorage::new())).await.unwrap();
    let master = Identity::generate().unwrap();
    let instance = Identity::generate().unwrap();
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();

    // 1000s lifetime, 100s (10%) remaining -- inside the 25% window.
    let mut near_expiry = DelegationCertificate::issue(
        &master,
        instance.public_key(),
        1000,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    near_expiry.issued_at_secs = now - 900;
    near_expiry.expires_at_secs = now + 100;
    registry.set_instance_cert("near-expiry-svc".to_string(), near_expiry).await.unwrap();

    // Freshly issued, nowhere near its 3600s expiry.
    let mut fresh = DelegationCertificate::issue(
        &master,
        instance.public_key(),
        3600,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    fresh.issued_at_secs = now;
    fresh.expires_at_secs = now + 3600;
    registry.set_instance_cert("fresh-svc".to_string(), fresh).await.unwrap();

    let warned = warn_on_near_expiry_instance_certs(&registry);
    assert_eq!(warned, vec!["near-expiry-svc".to_string()]);
}

/// A persisted binding row that would panic `LogicalServiceName::new`
/// (a `/` in the dependency name) or fails to parse as JSON must be
/// warned and skipped, not crash substrate startup -- and a good row
/// alongside it must still replay.
#[tokio::test]
async fn an_unreadable_persisted_binding_is_skipped_not_fatal() {
    let storage = Arc::new(MockStorage::new());
    let registry = EndpointRegistry::new(storage.clone()).await.unwrap();

    registry.save_binding("svc-slash", "app-1", "bad/name", r#"{"fake":"entry"}"#).await.unwrap();
    registry.save_binding("svc-badjson", "app-1", "backend", "not json").await.unwrap();
    let good_entry = TopologyEntry {
        mode: TopologyMode::Singleton,
        members: vec![ServiceId::new("did:key:zGoodMember")],
        sharding_strategy: None,
        epoch: TopologyEpoch::default(),
        cache_ttl: Duration::from_secs(60),
        not_after: None,
    };
    registry
        .save_binding("svc-good", "app-1", "good-dep", &serde_json::to_string(&good_entry).unwrap())
        .await
        .unwrap();

    let app_registry = replay_persisted_bindings(&registry).await.unwrap();

    assert!(
        app_registry
            .get(&TopologyKey::local(
                AppInstanceId::new("app-1"),
                LogicalServiceName::new("good-dep")
            ))
            .is_some(),
        "the well-formed row alongside the corrupt ones must still replay"
    );
    assert_eq!(app_registry.list(&AppScope::Local(AppInstanceId::new("app-1"))).len(), 1);
}
