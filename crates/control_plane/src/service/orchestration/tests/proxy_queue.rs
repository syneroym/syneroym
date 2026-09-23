use std::sync::Arc;

use super::{super::*, helpers::*};

#[tokio::test]
async fn proxy_dead_letters_lists_what_the_services_queue_holds() {
    let temp_dir = tempfile::tempdir().unwrap();
    let queues = Arc::new(FakeProxyQueues::default());
    queues.dead.lock().unwrap().push(a_dead_letter(1, "msg-7"));
    let service = service_with_proxy_queues(temp_dir.path(), &queues).await;

    let listed = service
        .proxy_dead_letters("svc-a".to_string(), &status_capable_caller("owner"))
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].idempotency_key, "msg-7");
    assert_eq!(listed[0].last_error, "target unreachable");
}

/// The verb B1 shipped without and had to add afterwards, because its
/// e2e could not otherwise assert that an item was queued, survived a
/// restart, and then left.
#[tokio::test]
async fn proxy_outbox_lists_an_item_before_it_lands() {
    let temp_dir = tempfile::tempdir().unwrap();
    let queues = Arc::new(FakeProxyQueues::default());
    queues.queued.lock().unwrap().push(QueuedCallInfo {
        id: 1,
        idempotency_key: "msg-7".to_string(),
        attempts: 2,
    });
    let service = service_with_proxy_queues(temp_dir.path(), &queues).await;

    let listed =
        service.proxy_outbox("svc-a".to_string(), &status_capable_caller("owner")).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].idempotency_key, "msg-7");
    assert_eq!(listed[0].attempts, 2);
}

#[tokio::test]
async fn replay_re_enqueues_and_does_not_execute_inline() {
    let temp_dir = tempfile::tempdir().unwrap();
    let queues = Arc::new(FakeProxyQueues::default());
    queues.dead.lock().unwrap().push(a_dead_letter(1, "msg-7"));
    let service = service_with_proxy_queues(temp_dir.path(), &queues).await;

    service.proxy_replay("svc-a".to_string(), 1, &status_capable_caller("owner")).await.unwrap();

    assert!(queues.dead.lock().unwrap().is_empty(), "the dead letter must be consumed");
    assert_eq!(
        queues.queued.lock().unwrap().len(),
        1,
        "and reappear in the outbox for the worker to pick up"
    );
    assert_eq!(
        queues.delivered.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "replay must not deliver anything itself"
    );
}

/// The proxy verbs extend the gate their neighbours already
/// use rather than inventing a second authority to hold.
#[tokio::test]
async fn the_new_verbs_are_refused_without_the_gate_their_neighbours_use() {
    use syneroym_rpc::{AuthLevel, SessionContext};

    let temp_dir = tempfile::tempdir().unwrap();
    let queues = Arc::new(FakeProxyQueues::default());
    queues.dead.lock().unwrap().push(a_dead_letter(1, "msg-7"));
    let service = service_with_proxy_queues(temp_dir.path(), &queues).await;

    let ungranted = CallerContext {
        caller_did: "did:key:zStranger".to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: "did:key:zStranger".to_string(),
            ..Default::default()
        },
        auth: AuthLevel::Delegated,
        proof: None,
    };

    assert!(service.proxy_outbox("svc-a".to_string(), &ungranted).await.is_err());
    assert!(service.proxy_dead_letters("svc-a".to_string(), &ungranted).await.is_err());
    assert!(service.proxy_replay("svc-a".to_string(), 1, &ungranted).await.is_err());
    assert_eq!(
        queues.dead.lock().unwrap().len(),
        1,
        "a refused replay must not have consumed the dead letter"
    );
}

/// Replay re-enqueues a call the worker then sends, so it is a
/// lifecycle write and must not be reachable with the read grant the
/// listing verbs use. `status_capable_caller` holds both, so this
/// drives a caller holding *only* the read one.
#[tokio::test]
async fn proxy_replay_is_not_reachable_with_only_the_read_grant() {
    use syneroym_rpc::{AuthLevel, Capability, SessionContext};

    let temp_dir = tempfile::tempdir().unwrap();
    let queues = Arc::new(FakeProxyQueues::default());
    queues.dead.lock().unwrap().push(a_dead_letter(1, "msg-7"));
    let service = service_with_proxy_queues(temp_dir.path(), &queues).await;

    let read_only = CallerContext {
        caller_did: "did:key:zReader".to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: "did:key:zReader".to_string(),
            capabilities: vec![Capability {
                with: ResourceUri::substrate("did:key:zTestNode"),
                can: Ability(Ability::ORCHESTRATOR_STATUS.to_string()),
                caveats: None,
            }],
            ..Default::default()
        },
        auth: AuthLevel::Delegated,
        proof: None,
    };

    // The listings are reads and stay reachable.
    assert!(service.proxy_outbox("svc-a".to_string(), &read_only).await.is_ok());
    assert!(service.proxy_dead_letters("svc-a".to_string(), &read_only).await.is_ok());

    // The write is not.
    assert!(
        service.proxy_replay("svc-a".to_string(), 1, &read_only).await.is_err(),
        "a read grant must not let a caller make a service emit calls"
    );
    assert_eq!(queues.dead.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn sagas_lists_what_the_services_log_holds() {
    let temp_dir = tempfile::tempdir().unwrap();
    let queues = Arc::new(FakeProxyQueues::default());
    queues.sagas.lock().unwrap().push(a_saga("saga-1", "unreachable"));
    let service = service_with_proxy_queues(temp_dir.path(), &queues).await;

    let listed = service.sagas("svc-a".to_string(), &status_capable_caller("owner")).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].saga_id, "saga-1");
    assert_eq!(listed[0].name, "checkout");
}

#[tokio::test]
async fn sagas_is_refused_without_a_status_grant() {
    use syneroym_rpc::{AuthLevel, SessionContext};

    let temp_dir = tempfile::tempdir().unwrap();
    let queues = Arc::new(FakeProxyQueues::default());
    queues.sagas.lock().unwrap().push(a_saga("saga-1", "unreachable"));
    let service = service_with_proxy_queues(temp_dir.path(), &queues).await;

    let ungranted = CallerContext {
        caller_did: "did:key:zStranger".to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: "did:key:zStranger".to_string(),
            ..Default::default()
        },
        auth: AuthLevel::Delegated,
        proof: None,
    };

    assert!(service.sagas("svc-a".to_string(), &ungranted).await.is_err());
}

/// `saga-compensate` causes calls to leave the node, so it takes the
/// write gate, not the listing's read gate -- the same rule
/// `proxy-replay` follows.
#[tokio::test]
async fn saga_compensate_is_not_reachable_with_only_the_read_grant() {
    use syneroym_rpc::{AuthLevel, Capability, SessionContext};

    let temp_dir = tempfile::tempdir().unwrap();
    let queues = Arc::new(FakeProxyQueues::default());
    queues.sagas.lock().unwrap().push(a_saga("saga-1", "unreachable"));
    let service = service_with_proxy_queues(temp_dir.path(), &queues).await;

    let read_only = CallerContext {
        caller_did: "did:key:zReader".to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: "did:key:zReader".to_string(),
            capabilities: vec![Capability {
                with: ResourceUri::substrate("did:key:zTestNode"),
                can: Ability(Ability::ORCHESTRATOR_STATUS.to_string()),
                caveats: None,
            }],
            ..Default::default()
        },
        auth: AuthLevel::Delegated,
        proof: None,
    };

    assert!(service.sagas("svc-a".to_string(), &read_only).await.is_ok());
    assert!(
        service
            .saga_compensate("svc-a".to_string(), "saga-1".to_string(), &read_only)
            .await
            .is_err(),
        "a read grant must not let a caller make a service emit undos"
    );
    assert!(queues.rearmed.lock().unwrap().is_empty());

    service
        .saga_compensate("svc-a".to_string(), "saga-1".to_string(), &status_capable_caller("owner"))
        .await
        .unwrap();
    assert_eq!(queues.rearmed.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn readyz_does_not_podman_inspect_a_tcp_service() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;

    service
        .registry
        .register(
            "tcp-readyz-svc".to_string(),
            "main".to_string(),
            SubstrateEndpoint::TcpHostPort { host: "127.0.0.1".to_string(), port: 9 },
        )
        .await
        .unwrap();
    service
        .registry
        .set_deploy_facts("tcp-readyz-svc".to_string(), "tcp".to_string(), None, None, None)
        .await
        .unwrap();

    // This once called `podman inspect` against a real TCP
    // service and reported the resulting failure as unreadiness.
    let result =
        service.readyz("tcp-readyz-svc".to_string(), &status_capable_caller("owner")).await;
    assert!(result.is_ok(), "{result:?}");
}
