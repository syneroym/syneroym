use std::sync::Arc;

use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::TcpProbe as WitTcpProbe;

use super::{super::*, helpers::*};

#[tokio::test]
async fn status_reports_unknown_for_a_service_with_no_recorded_type() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;

    service
        .registry
        .register(
            "pre-a4-svc".to_string(),
            "main".to_string(),
            SubstrateEndpoint::TcpHostPort { host: "127.0.0.1".to_string(), port: 9 },
        )
        .await
        .unwrap();
    // Deliberately no `set_deploy_facts` call -- simulates a service
    // deployed by a pre-A4 binary.

    let phase = service.instance_phase("pre-a4-svc", None).await;
    assert!(
        matches!(phase, InstancePhase::Unknown(ref r) if r.contains("no service type recorded"))
    );
}

#[tokio::test]
async fn status_reports_not_found_for_an_id_this_substrate_has_no_endpoints_for() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;

    let status = service
        .status(vec!["never-deployed".to_string()], &node_wide_caller("owner"))
        .await
        .unwrap();
    assert_eq!(status.services.len(), 1);
    assert!(matches!(status.services[0].phase, InstancePhase::NotFound));
}

#[tokio::test]
async fn status_omits_a_service_the_caller_may_not_see_and_reports_not_found_when_named() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;

    service
        .deploy(
            "owned-by-alice".to_string(),
            tcp_manifest_with(9, None),
            &node_wide_caller("alice"),
        )
        .await
        .unwrap();

    let bob = scoped_deploy_caller("bob", "some-other-service");
    let swept = service.status(vec![], &bob).await.unwrap();
    assert!(
        swept.services.iter().all(|s| s.service_id != "owned-by-alice"),
        "bob must not see alice's service in an unnamed sweep"
    );

    // A4-10: named explicitly, it must read identically to an id that
    // was never deployed at all -- `not-found`, not `unauthorized`. Bob
    // holds no grant on "owned-by-alice" whatsoever; distinguishing the
    // two would let any verified caller probe for the existence of an
    // arbitrary DID on this node with no grant at all.
    let named = service.status(vec!["owned-by-alice".to_string()], &bob).await.unwrap();
    assert_eq!(named.services.len(), 1);
    assert!(matches!(named.services[0].phase, InstancePhase::NotFound));
}

/// `status` reports the epoch this substrate currently
/// serves for each of a service's own declared dependencies, read
/// from the per-dependent persisted binding row -- the per-dependent
/// binding convergence data.
#[tokio::test]
async fn status_reports_the_epoch_it_currently_serves_per_dependency() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let caller = node_wide_caller("did:key:zAlice");

    service
        .deploy_with_context(
            "frontend-svc".to_string(),
            tcp_manifest_with(9, None),
            Some(app_context(
                "app-1",
                "frontend",
                vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
            )),
            &caller,
        )
        .await
        .unwrap();
    service
        .write_bindings(
            BindingWrite {
                service_id: "frontend-svc".to_string(),
                app_instance_id: "app-1".to_string(),
                bindings: vec![DependencyBinding {
                    dependency_name: "backend".to_string(),
                    app_instance_id: "app-1".to_string(),
                    mode: WitTopologyMode::Singleton,
                    members: vec!["did:key:zNewBackendMember".to_string()],
                    epoch: 3,
                    cache_ttl_ms: 60_000,
                }],
                generation: 0,
            },
            &caller,
        )
        .await
        .unwrap();

    let status = service.status(vec!["frontend-svc".to_string()], &caller).await.unwrap();
    assert_eq!(status.services.len(), 1);
    assert_eq!(
        status.services[0].binding_epochs,
        vec![("backend".to_string(), 3)],
        "{:?}",
        status.services[0].binding_epochs
    );
}

/// A caller with no grant on a named id must not learn
/// anything about it, including what it depends on -- `not-found`
/// carries an empty `binding_epochs`, same as every other field.
#[tokio::test]
async fn status_reports_not_found_for_a_named_id_the_caller_may_not_see() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;

    service
        .deploy_with_context(
            "owned-by-alice".to_string(),
            tcp_manifest_with(9, None),
            Some(app_context(
                "app-1",
                "frontend",
                vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
            )),
            &node_wide_caller("alice"),
        )
        .await
        .unwrap();

    let bob = scoped_deploy_caller("bob", "some-other-service");
    let named = service.status(vec!["owned-by-alice".to_string()], &bob).await.unwrap();
    assert_eq!(named.services.len(), 1);
    assert!(matches!(named.services[0].phase, InstancePhase::NotFound));
    assert!(
        named.services[0].binding_epochs.is_empty(),
        "a caller with no grant must not learn what a service it cannot see depends on"
    );
}

/// A4-11: an unbounded `service_ids` list from any verified caller must
/// not be free to accept -- each named id that turns out not to exist
/// used to cost a full scan of every registered endpoint.
#[tokio::test]
async fn status_rejects_a_service_ids_list_over_the_cap() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;

    let too_many: Vec<String> = (0..=MAX_STATUS_SERVICE_IDS).map(|i| format!("svc-{i}")).collect();
    let err = service.status(too_many, &status_capable_caller("owner")).await.unwrap_err();
    assert!(err.contains("over the"), "{err}");
}

/// A4-09 (post-review): a duplicate id in one `service_ids` list used to
/// be a target twice over, racing itself inside the same `join_all`
/// (A4-05) and bypassing `probe_cached`'s cache entirely -- confirmed
/// empirically before this fix (8 copies of one id produced 8 concurrent
/// probes). One entry per distinct id now, regardless of how many times
/// the caller names it.
#[tokio::test]
async fn status_deduplicates_repeated_service_ids_before_probing() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let owner = status_capable_caller("owner");

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let manifest = tcp_manifest_with(
        port,
        Some(WitHealthCheck::TcpConnect(WitTcpProbe {
            interface_name: "main".to_string(),
            timeout_ms: 2000,
        })),
    );
    service.deploy("dup-svc".to_string(), manifest, &owner).await.unwrap();

    let repeated = vec!["dup-svc".to_string(); 8];
    let status = service.status(repeated, &owner).await.unwrap();
    assert_eq!(status.services.len(), 1, "{:?}", status.services);
    assert!(matches!(status.services[0].probe, ProbeStatus::Passing));
}

#[tokio::test]
async fn node_facts_are_absent_for_a_caller_without_node_wide_status() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;

    let bob = scoped_deploy_caller("bob", "bobs-service");
    service.deploy("bobs-service".to_string(), tcp_manifest_with(9, None), &bob).await.unwrap();

    let status = service.status(vec![], &bob).await.unwrap();
    assert!(status.node.is_none());
    assert_eq!(status.services.len(), 1);
    assert_eq!(status.services[0].service_id, "bobs-service");
}

#[tokio::test]
async fn node_facts_are_returned_for_the_substrate_owner() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;

    let status = service.status(vec![], &status_capable_caller("owner")).await.unwrap();
    assert!(status.node.is_some());
}

#[tokio::test]
async fn status_reports_the_compiled_in_service_types() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;

    let status = service.status(vec![], &status_capable_caller("owner")).await.unwrap();
    let node = status.node.unwrap();
    // Default features enable both sandboxes; `tcp` needs no engine.
    assert_eq!(node.service_types, vec!["container", "tcp", "wasm"]);
}

/// A4-06: the same gate `status`'s own `node` field applies, on the
/// standalone path -- a caller without node-wide `orchestrator/status`
/// must not read node facts through this narrower method either.
#[tokio::test]
async fn node_facts_is_none_without_node_wide_status() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let bob = scoped_deploy_caller("bob", "bobs-service");
    assert!(service.node_facts(&bob).await.is_none());
}

/// A4-06: `node_facts` must answer identically to `status(vec![])`'s
/// `node` field -- the whole point of splitting it out is a cheaper path
/// to the *same* facts, not a different set of them.
#[tokio::test]
async fn node_facts_answers_the_same_as_status_with_no_service_ids() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let owner = status_capable_caller("owner");

    let registry_client = Arc::new(syneroym_core::dht_registry::RegistryClient::new(
        true,
        Some("http://registry.example".to_string()),
    ));
    service.set_endpoint_publisher(Arc::new(
        syneroym_core::endpoint_publisher::EndpointPublisher::new(
            registry_client,
            temp_dir.path().to_path_buf(),
        ),
    ));

    let via_status = service.status(vec![], &owner).await.unwrap().node.unwrap();
    let via_node_facts = service.node_facts(&owner).await.unwrap();
    assert_eq!(via_status.registry_url, via_node_facts.registry_url);
    assert_eq!(via_status.dht_enabled, via_node_facts.dht_enabled);
    assert_eq!(via_status.node_did, via_node_facts.node_did);
    assert_eq!(via_status.service_types, via_node_facts.service_types);
}

#[tokio::test]
async fn status_reports_the_registry_this_node_publishes_into() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;

    let registry_client = Arc::new(syneroym_core::dht_registry::RegistryClient::new(
        true,
        Some("http://registry.example".to_string()),
    ));
    service.set_endpoint_publisher(Arc::new(
        syneroym_core::endpoint_publisher::EndpointPublisher::new(
            registry_client,
            temp_dir.path().to_path_buf(),
        ),
    ));

    let status = service.status(vec![], &status_capable_caller("owner")).await.unwrap();
    let node = status.node.unwrap();
    assert_eq!(node.registry_url.as_deref(), Some("http://registry.example"));
    assert!(node.dht_enabled);
}

#[tokio::test]
async fn a_probe_runs_for_a_tcp_service_whose_phase_is_unknown() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;

    // A `connect` completes once the OS accepts the SYN into its own
    // backlog queue -- the listener need not call `accept()` itself, so
    // just keeping it bound (and alive for the test's duration) is
    // enough for the probe to see a `Passing` connect.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    let manifest = tcp_manifest_with(
        port,
        Some(WitHealthCheck::TcpConnect(WitTcpProbe {
            interface_name: "main".to_string(),
            timeout_ms: 2000,
        })),
    );
    service
        .deploy("probed-tcp-svc".to_string(), manifest, &node_wide_caller("owner"))
        .await
        .unwrap();

    let status = service
        .status(vec!["probed-tcp-svc".to_string()], &node_wide_caller("owner"))
        .await
        .unwrap();
    assert_eq!(status.services.len(), 1);
    assert!(matches!(status.services[0].phase, InstancePhase::Unknown(_)));
    assert!(
        matches!(status.services[0].probe, ProbeStatus::Passing),
        "expected the probe to have run and passed, got {:?}",
        status.services[0].probe
    );
}
