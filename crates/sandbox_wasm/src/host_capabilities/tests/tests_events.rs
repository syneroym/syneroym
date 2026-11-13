use std::{path::Path, sync::Arc};

use syneroym_core::{event_bus::EventBus, local_registry::EndpointRegistry, storage::MockStorage};
use syneroym_data_db::SqliteStorageProvider;
use syneroym_data_keystore::KeyStore;
use syneroym_rpc::{AuthLevel, CallerContext, SessionContext};
use syneroym_wit_interfaces::events_host::syneroym::events::events::{EventsError, ReadResult};

use super::{
    test_blob_provider, test_messaging_context, test_service_proxy, test_streaming_context,
};
use crate::host_capabilities::HostState;

pub(super) fn test_host_state(
    component_id: &str,
    app_instance_id: Option<String>,
    read_only: bool,
    event_bus: Option<Arc<EventBus>>,
    db_dir: &Path,
) -> HostState {
    let caller = CallerContext {
        caller_did: "did:key:zTest".to_string(),
        app_instance: app_instance_id.clone(),
        session: SessionContext::default(),
        auth: AuthLevel::Ucan,
        proof: None,
        on_behalf_did: None,
    };
    HostState::new(
        component_id.to_string(),
        None,
        Arc::new(KeyStore::new()),
        Arc::new(SqliteStorageProvider::new(db_dir, false).unwrap()),
        test_blob_provider(),
        caller,
        0,
        test_messaging_context(),
        test_streaming_context(),
        test_service_proxy(),
        None,
        read_only,
        syneroym_rpc::empty_row_authorizer(),
        app_instance_id,
        syneroym_app_orchestration::empty_resolver(),
    )
    .with_event_bus(event_bus)
}

#[tokio::test]
async fn record_in_a_read_only_call_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));
    let bus = EventBus::new(registry);
    let host = test_host_state("svc_a", Some("app_a".into()), true, Some(bus), temp.path());

    let res = host.record_event("test.kind".into(), "sub".into(), None);
    assert!(matches!(res, Err(EventsError::ReadOnly)));
}

#[tokio::test]
async fn record_without_an_app_instance_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));
    let bus = EventBus::new(registry);
    let host = test_host_state("svc_a", None, false, Some(bus), temp.path());

    let res = host.record_event("test.kind".into(), "sub".into(), None);
    assert!(matches!(res, Err(EventsError::NoAppInstance)));
}

#[tokio::test]
async fn record_without_a_bus_is_no_app_instance() {
    let temp = tempfile::tempdir().unwrap();
    let host = test_host_state("svc_a", Some("app_a".into()), false, None, temp.path());

    let res = host.record_event("test.kind".into(), "sub".into(), None);
    assert!(matches!(res, Err(EventsError::NoAppInstance)));
}

#[tokio::test]
async fn read_reads_only_the_callers_app_instance() {
    let temp = tempfile::tempdir().unwrap();
    let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));
    registry.set_app_context("svc_a".into(), "app_a".into(), "conversation".into()).await.unwrap();
    registry.set_app_context("svc_b".into(), "app_b".into(), "conversation".into()).await.unwrap();

    let bus = EventBus::new(registry);
    let host_a =
        test_host_state("svc_a", Some("app_a".into()), false, Some(bus.clone()), temp.path());
    let host_b =
        test_host_state("svc_b", Some("app_b".into()), false, Some(bus.clone()), temp.path());

    let cursor_a0 = bus.head("app_a");

    host_a.record_event("test.a".into(), "sub_a".into(), None).unwrap();
    host_b.record_event("test.b".into(), "sub_b".into(), None).unwrap();

    let read_res = host_a.read_events(Some(cursor_a0.to_string()), 100).unwrap();
    match read_res {
        ReadResult::Events((events, _)) => {
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].kind, "test.a");
            assert_eq!(events[0].subject, "sub_a");
        }
        ReadResult::Reload(_) => panic!("unexpected reload"),
    }
}

#[tokio::test]
async fn read_caps_the_limit_at_five_hundred() {
    let temp = tempfile::tempdir().unwrap();
    let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));
    registry.set_app_context("svc_a".into(), "app_a".into(), "conversation".into()).await.unwrap();

    let bus = EventBus::new(registry);
    let host =
        test_host_state("svc_a", Some("app_a".into()), false, Some(bus.clone()), temp.path());
    let cursor_0 = bus.head("app_a");

    for i in 0..600 {
        host.record_event("test.kind".into(), format!("sub_{i}"), None).unwrap();
    }

    let read_res = host.read_events(Some(cursor_0.to_string()), 1000).unwrap();
    match read_res {
        ReadResult::Events((events, _)) => {
            assert_eq!(events.len(), 500);
        }
        ReadResult::Reload(_) => panic!("unexpected reload"),
    }
}
