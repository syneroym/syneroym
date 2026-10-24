use std::sync::Arc;

use syneroym_identity::{Identity, delegation::SCOPE_SERVICE_INSTANCE};

use super::*;
use crate::storage::MockStorage;

#[tokio::test]
async fn test_registry_lifecycle() {
    let storage = Arc::new(MockStorage::new());
    let registry = EndpointRegistry::new(storage.clone()).await.unwrap();

    // 1. Register
    let service = "test-service".to_string();
    let iface = "health".to_string();
    let endpoint = SubstrateEndpoint::WasmChannel { service_id: service.clone() };

    registry.register(service.clone(), iface.clone(), endpoint.clone()).await.unwrap();

    // 2. Lookup
    let (found, canonical) = registry.lookup(&service, &iface).unwrap();
    assert_eq!(canonical, iface);
    match found {
        SubstrateEndpoint::WasmChannel { service_id } => assert_eq!(service_id, service),
        _ => panic!("Wrong endpoint type"),
    }

    // 3. Persistence check (new registry instance with same storage)
    let registry2 = EndpointRegistry::new(storage).await.unwrap();
    assert!(registry2.lookup(&service, &iface).is_some());

    // 4. Remove
    registry2.remove(&service, &iface).await.unwrap();
    assert!(registry2.lookup(&service, &iface).is_none());
}

#[tokio::test]
async fn test_registry_empty_interface() {
    let storage = Arc::new(MockStorage::new());
    let registry = EndpointRegistry::new(storage).await.unwrap();

    let service = "empty-iface-service".to_string();
    let iface = String::new();
    let endpoint = SubstrateEndpoint::WasmChannel { service_id: service.clone() };

    registry.register(service.clone(), iface.clone(), endpoint.clone()).await.unwrap();

    let (found, canonical) = registry.lookup(&service, &iface).unwrap();
    assert_eq!(canonical, "");
    match found {
        SubstrateEndpoint::WasmChannel { service_id } => assert_eq!(service_id, service),
        _ => panic!("Wrong endpoint type"),
    }
}

/// An empty interface resolves to the one app-declared interface, with
/// the native-capability interfaces also registered -- which is what
/// makes the naive "only one endpoint" rule wrong.
#[tokio::test]
async fn an_empty_interface_resolves_to_the_only_app_declared_one() {
    let storage = Arc::new(MockStorage::new());
    let registry = EndpointRegistry::new(storage).await.unwrap();
    let service = "svc-with-one-declared-iface".to_string();
    let endpoint = SubstrateEndpoint::WasmChannel { service_id: service.clone() };

    for native in NATIVE_CAPABILITY_INTERFACES {
        registry.register(service.clone(), native.to_string(), endpoint.clone()).await.unwrap();
    }
    registry.register(service.clone(), "default".to_string(), endpoint.clone()).await.unwrap();

    let (found, canonical) = registry.lookup(&service, "").unwrap();
    assert_eq!(canonical, "default");
    match found {
        SubstrateEndpoint::WasmChannel { service_id } => assert_eq!(service_id, service),
        _ => panic!("Wrong endpoint type"),
    }
}

/// `NODE_NATIVE_INTERFACES` (`orchestrator`/`security`) is
/// filtered by the empty branch exactly like `NATIVE_CAPABILITY_INTERFACES`
/// -- neither is ever "the one app-declared interface" a caller with no
/// interface named actually meant, even though today `orchestrator`/
/// `security` are only ever registered under a node's own DID and never
/// under a deployed service's id (so this was previously unreachable in
/// practice, only in principle).
#[tokio::test]
async fn an_empty_interface_never_resolves_to_a_node_native_interface() {
    let storage = Arc::new(MockStorage::new());
    let registry = EndpointRegistry::new(storage).await.unwrap();
    let service = "svc-with-node-native-only".to_string();
    let endpoint = SubstrateEndpoint::WasmChannel { service_id: service.clone() };

    for native in NODE_NATIVE_INTERFACES {
        registry.register(service.clone(), native.to_string(), endpoint.clone()).await.unwrap();
    }
    assert!(registry.lookup(&service, "").is_none());

    // With one genuine app-declared interface alongside them, that one
    // still resolves -- the node-native pair is filtered out, not
    // counted toward the ambiguity check.
    registry.register(service.clone(), "default".to_string(), endpoint.clone()).await.unwrap();
    let (_, canonical) = registry.lookup(&service, "").unwrap();
    assert_eq!(canonical, "default");
}

/// The ambiguity and the empty-set halves, both `None`, never
/// a guess.
#[tokio::test]
async fn an_empty_interface_is_refused_when_two_are_declared_and_when_none_is() {
    let storage = Arc::new(MockStorage::new());
    let registry = EndpointRegistry::new(storage).await.unwrap();
    let endpoint_of = |id: &str| SubstrateEndpoint::WasmChannel { service_id: id.to_string() };

    // Zero app-declared interfaces (only native ones): refused.
    let none_declared = "svc-only-native".to_string();
    for native in NATIVE_CAPABILITY_INTERFACES {
        registry
            .register(none_declared.clone(), native.to_string(), endpoint_of(&none_declared))
            .await
            .unwrap();
    }
    assert!(registry.lookup(&none_declared, "").is_none());

    // Two app-declared interfaces: refused.
    let two_declared = "svc-two-declared".to_string();
    registry
        .register(two_declared.clone(), "default".to_string(), endpoint_of(&two_declared))
        .await
        .unwrap();
    registry
        .register(two_declared.clone(), "admin".to_string(), endpoint_of(&two_declared))
        .await
        .unwrap();
    assert!(registry.lookup(&two_declared, "").is_none());
}

/// The scoped form of the interface-resolution property -- at the hop that
/// resolves the service, the interface a downstream check sees is the
/// registered name, exactly as on the hash path (covered above by
/// `an_empty_interface_resolves_to_the_only_app_declared_one`). Its
/// paired assertion, here, is the relay case: for a service this
/// registry does not host, `lookup` (and `resolve_interface`) return
/// `None` regardless of the interface a caller named -- empty or
/// hashed alike -- which is exactly the condition
/// `RouteHandler::handle_stream` (`crates/router/src/route_handler/io.rs`)
/// uses to forward the original, uncanonicalized preamble to the next
/// hop rather than terminate it here. A relay hop does not host the
/// service and cannot know its interface names, so it must not guess
/// -- the same rule it already applies to an unresolved `short_hash`.
#[tokio::test]
async fn the_terminating_hop_canonicalizes_before_any_capability_check() {
    let storage = Arc::new(MockStorage::new());
    let registry = EndpointRegistry::new(storage).await.unwrap();
    let hosted = "svc-hosted-here".to_string();
    registry
        .register(
            hosted.clone(),
            "default".to_string(),
            SubstrateEndpoint::WasmChannel { service_id: hosted.clone() },
        )
        .await
        .unwrap();

    // Terminating: an empty interface canonicalizes to the one
    // app-declared interface (a property another test already pins;
    // repeated here as the paired half of this test's own claim).
    let (_, canonical) = registry.lookup(&hosted, "").unwrap();
    assert_eq!(canonical, "default");

    // Relay: a service this registry does not host resolves to
    // nothing, for every interface shape a caller might have named --
    // empty, a hash, or a literal name -- which is what makes
    // `handle_stream` take the relay branch and forward the original
    // preamble untouched instead of canonicalizing here.
    let unhosted = "svc-hosted-elsewhere";
    assert!(registry.resolve_interface(unhosted, "").is_none());
    assert!(registry.resolve_interface(unhosted, "default").is_none());
    assert!(registry.resolve_interface(unhosted, &util::short_hash("default")).is_none());
}

/// `set_owner`/`owner_of`/`remove_owner` round-trip, and
/// persist across a second `EndpointRegistry::new` on the same storage
/// (mirrors `test_registry_lifecycle`'s persistence step).
#[tokio::test]
async fn test_owner_round_trip_and_persistence() {
    let storage = Arc::new(MockStorage::new());
    let registry = EndpointRegistry::new(storage.clone()).await.unwrap();

    assert_eq!(registry.owner_of("svc-1"), None);

    registry.set_owner("svc-1".to_string(), "did:key:zOwner".to_string()).await.unwrap();
    assert_eq!(registry.owner_of("svc-1"), Some("did:key:zOwner".to_string()));

    let registry2 = EndpointRegistry::new(storage).await.unwrap();
    assert_eq!(registry2.owner_of("svc-1"), Some("did:key:zOwner".to_string()));

    registry2.remove_owner("svc-1").await.unwrap();
    assert_eq!(registry2.owner_of("svc-1"), None);
}

#[tokio::test]
async fn test_owner_of_unknown_service_is_none() {
    let storage = Arc::new(MockStorage::new());
    let registry = EndpointRegistry::new(storage).await.unwrap();
    assert_eq!(registry.owner_of("never-deployed"), None);
}

fn test_cert(master: &Identity, service_id: &str) -> DelegationCertificate {
    let instance = Identity::generate().unwrap();
    let mut cert = DelegationCertificate::issue(
        master,
        instance.public_key(),
        3600,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    cert.temporary_did = service_id.to_string();
    cert
}

#[tokio::test]
async fn an_instance_certificate_round_trips_through_storage() {
    let storage = Arc::new(MockStorage::new());
    let registry = EndpointRegistry::new(storage.clone()).await.unwrap();
    let master = Identity::generate().unwrap();

    assert_eq!(registry.instance_cert("svc-1"), None);

    let cert = test_cert(&master, "svc-1");
    registry.set_instance_cert("svc-1".to_string(), cert.clone()).await.unwrap();
    assert_eq!(registry.instance_cert("svc-1"), Some(cert.clone()));

    // Persists across a second `EndpointRegistry::new` on the same storage.
    let registry2 = EndpointRegistry::new(storage).await.unwrap();
    assert_eq!(registry2.instance_cert("svc-1"), Some(cert));
}

#[tokio::test]
async fn removing_a_service_forgets_its_instance_certificate() {
    let storage = Arc::new(MockStorage::new());
    let registry = EndpointRegistry::new(storage).await.unwrap();
    let master = Identity::generate().unwrap();

    let cert = test_cert(&master, "svc-1");
    registry.set_instance_cert("svc-1".to_string(), cert).await.unwrap();
    assert!(registry.instance_cert("svc-1").is_some());

    registry.remove_instance_cert("svc-1").await.unwrap();
    assert_eq!(registry.instance_cert("svc-1"), None);
}

#[tokio::test]
async fn all_instance_certs_returns_every_installed_certificate() {
    let storage = Arc::new(MockStorage::new());
    let registry = EndpointRegistry::new(storage).await.unwrap();
    assert!(registry.all_instance_certs().is_empty());

    let master = Identity::generate().unwrap();
    registry.set_instance_cert("svc-1".to_string(), test_cert(&master, "svc-1")).await.unwrap();
    registry.set_instance_cert("svc-2".to_string(), test_cert(&master, "svc-2")).await.unwrap();

    let mut all = registry.all_instance_certs();
    all.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].0, "svc-1");
    assert_eq!(all[1].0, "svc-2");
}

#[tokio::test]
async fn a_call_with_no_interface_never_picks_the_mailbox() {
    let storage = Arc::new(MockStorage::new());
    let registry = EndpointRegistry::new(storage).await.unwrap();
    let node_did = "did:key:zTestNode";
    let endpoint = SubstrateEndpoint::NativeHostChannel { service_id: "test".to_string() };

    for native in NODE_NATIVE_INTERFACES {
        registry
            .register(node_did.to_string(), native.to_string(), endpoint.clone())
            .await
            .unwrap();
    }
    registry
        .register(
            node_did.to_string(),
            crate::mailbox::MAILBOX_INTERFACE.to_string(),
            endpoint.clone(),
        )
        .await
        .unwrap();

    assert_eq!(registry.resolve_interface(node_did, ""), None);
}

#[tokio::test]
async fn the_mailbox_does_not_make_the_supervisor_ambiguous() {
    let storage = Arc::new(MockStorage::new());
    let registry = EndpointRegistry::new(storage).await.unwrap();
    let node_did = "did:key:zTestNode";
    let endpoint = SubstrateEndpoint::NativeHostChannel { service_id: "test".to_string() };

    registry
        .register(node_did.to_string(), "supervisor".to_string(), endpoint.clone())
        .await
        .unwrap();
    registry
        .register(
            node_did.to_string(),
            crate::mailbox::MAILBOX_INTERFACE.to_string(),
            endpoint.clone(),
        )
        .await
        .unwrap();

    assert_eq!(registry.resolve_interface(node_did, ""), Some("supervisor".to_string()));
}

#[tokio::test]
async fn a_mailbox_round_trips_and_persists_across_a_new_registry() {
    let storage = Arc::new(MockStorage::new());
    let registry = EndpointRegistry::new(storage.clone()).await.unwrap();
    assert_eq!(registry.mailbox_of("svc-1"), None);

    let mailbox = HostedMailbox {
        mailbox: "did:key:zBox".to_string(),
        max_ttl_secs: 100,
        methods: vec!["greeter#greet".to_string()],
    };
    registry.set_mailbox("svc-1".to_string(), mailbox.clone()).await.unwrap();
    assert_eq!(registry.mailbox_of("svc-1"), Some(mailbox.clone()));

    let reopened = EndpointRegistry::new(storage).await.unwrap();
    assert_eq!(reopened.mailbox_of("svc-1"), Some(mailbox));

    reopened.remove_mailbox("svc-1").await.unwrap();
    reopened.remove_mailbox("svc-1").await.unwrap();
    assert_eq!(reopened.mailbox_of("svc-1"), None);
}

#[tokio::test]
async fn hosted_mailboxes_lists_every_service_with_a_mailbox_sorted() {
    let registry = EndpointRegistry::new(Arc::new(MockStorage::new())).await.unwrap();
    let mailbox =
        HostedMailbox { mailbox: "did:key:zBox".to_string(), max_ttl_secs: 100, methods: vec![] };
    for id in ["svc-b", "svc-a"] {
        registry.set_mailbox(id.to_string(), mailbox.clone()).await.unwrap();
    }
    let ids: Vec<String> = registry.hosted_mailboxes().into_iter().map(|(id, _)| id).collect();
    assert_eq!(ids, ["svc-a", "svc-b"]);
}

#[test]
fn node_level_interfaces_match_by_name_and_by_short_hash() {
    for name in ["orchestrator", "security", "supervisor", "mailbox"] {
        assert!(is_node_level_interface(name), "{name}");
        assert!(is_node_level_interface(&util::short_hash(name)), "{name} hash");
    }
    assert!(!is_node_level_interface("data-layer"));
    assert!(!is_node_level_interface("greeter"));
}

#[tokio::test]
async fn a_lifetime_update_changes_only_a_fact_that_still_names_the_mailbox() {
    let storage = Arc::new(MockStorage::new());
    let registry = EndpointRegistry::new(storage.clone()).await.unwrap();
    assert!(!registry.set_mailbox_ttl("svc-1", "did:key:zBox", 50).await.unwrap(), "no fact");
    let mailbox = HostedMailbox {
        mailbox: "did:key:zBox".to_string(),
        max_ttl_secs: 100,
        methods: vec!["greeter#greet".to_string()],
    };
    registry.set_mailbox("svc-1".to_string(), mailbox.clone()).await.unwrap();

    assert!(!registry.set_mailbox_ttl("svc-1", "did:key:zOther", 50).await.unwrap());
    assert!(registry.set_mailbox_ttl("svc-1", "did:key:zBox", 50).await.unwrap());

    let expected = HostedMailbox { max_ttl_secs: 50, ..mailbox };
    assert_eq!(registry.mailbox_of("svc-1"), Some(expected.clone()));
    let reopened = EndpointRegistry::new(storage).await.unwrap();
    assert_eq!(reopened.mailbox_of("svc-1"), Some(expected));
}
