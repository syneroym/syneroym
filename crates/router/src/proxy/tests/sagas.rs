use super::helpers::*;

#[tokio::test]
async fn a_step_records_its_intent_before_the_call_lands() {
    let node = saga_node(5).await;
    let saga_id = begun_saga(&node).await;

    node.router.saga_step(saga_step_request(&saga_id, SAGA_TARGET)).await.unwrap();

    let log = node.sagas.existing_log_for(SAGA_CALLER).await.unwrap().unwrap();
    let step = log.next_uncompensated_step(&saga_id).unwrap().unwrap();
    assert_eq!(step.idx, 0);
    assert_eq!(step.method, "reserve");
    assert_eq!(node.target.invoked.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_step_whose_call_fails_is_recorded_as_failed_and_still_returns_its_error() {
    let node = saga_node(5).await;
    node.target.fail_with.store(true, Ordering::SeqCst);
    let saga_id = begun_saga(&node).await;

    let result = node.router.saga_step(saga_step_request(&saga_id, SAGA_TARGET)).await;
    assert!(result.is_err(), "the caller must still see the failure");

    let log = node.sagas.existing_log_for(SAGA_CALLER).await.unwrap().unwrap();
    assert!(
        log.next_uncompensated_step(&saga_id).unwrap().is_none(),
        "a failed step is never compensated"
    );
}

#[tokio::test]
async fn a_step_on_an_unknown_saga_is_refused() {
    let node = saga_node(5).await;
    let result = node.router.saga_step(saga_step_request("no-such-saga", SAGA_TARGET)).await;
    assert!(result.is_err(), "unexpected success");
}

#[tokio::test]
async fn a_step_against_a_node_level_interface_is_refused() {
    let node = saga_node(5).await;
    let saga_id = begun_saga(&node).await;

    for interface in ["orchestrator", "security", "supervisor"] {
        let mut req = saga_step_request(&saga_id, SAGA_TARGET);
        req.interface = interface.to_string();
        let result = node.router.saga_step(req).await;
        assert!(
            matches!(result, Err(ProxyError::PermissionDenied(_))),
            "interface '{interface}' must be refused, got {result:?}"
        );
    }
}

#[tokio::test]
async fn a_step_against_the_callers_own_service_is_refused() {
    let node = saga_node(5).await;
    let saga_id = begun_saga(&node).await;

    let result = node.router.saga_step(saga_step_request(&saga_id, SAGA_CALLER)).await;
    assert!(
        matches!(result, Err(ProxyError::UnsupportedTarget(_))),
        "a self-target must be refused, got {result:?}"
    );
}

#[tokio::test]
async fn a_step_with_no_timeout_takes_a_budget_inside_the_guests_epoch_not_the_proxys_default() {
    // The node's epoch is 5s, so the derived step budget is 4s -- well
    // under `DEFAULT_PROXY_CALL_TIMEOUT` (30s). A target that blocks
    // past 4s but under 30s must still time out, which only holds if
    // the derived budget -- not the proxy default -- was applied.
    let node = saga_node(5).await;
    let saga_id = begun_saga(&node).await;
    node.target.hold.lock().unwrap().replace(Arc::new(tokio::sync::Notify::new()));

    let started = Instant::now();
    let result = node.router.saga_step(saga_step_request(&saga_id, SAGA_TARGET)).await;
    assert!(matches!(result, Err(ProxyError::Timeout(_))), "got {result:?}");
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "the step must not wait out the proxy's own 30s default"
    );
}

#[tokio::test]
async fn a_step_with_an_explicit_timeout_keeps_it() {
    let node = saga_node(5).await;
    let saga_id = begun_saga(&node).await;
    let mut req = saga_step_request(&saga_id, SAGA_TARGET);
    req.timeout_ms = Some(50);
    node.target.hold.lock().unwrap().replace(Arc::new(tokio::sync::Notify::new()));

    let started = Instant::now();
    let result = node.router.saga_step(req).await;
    assert!(matches!(result, Err(ProxyError::Timeout(_))), "got {result:?}");
    assert!(started.elapsed() < Duration::from_secs(2), "the explicit 50ms budget must apply");
}

#[tokio::test]
async fn begin_is_refused_for_a_service_with_no_unexpired_instance_certificate() {
    let node = saga_node(5).await;
    node.registry.remove_instance_cert(SAGA_CALLER).await.unwrap();

    let result = node
        .router
        .saga_begin(SagaBegin {
            caller_service_id: SAGA_CALLER.to_string(),
            app_instance_id: None,
            name: "wf".to_string(),
            deadline_secs: None,
        })
        .await;
    assert!(
        matches!(result, Err(ProxyError::PermissionDenied(_))),
        "an uncertified caller must be refused, got {result:?}"
    );
}

/// A managed instance's certificate is renewed on every supervisor
/// pass, so its own current expiry cannot decide whether a long
/// deadline is sound -- `begin` warns rather than refusing.
#[tokio::test]
async fn begin_warns_but_proceeds_when_the_deadline_outlives_the_certificate() {
    let node = saga_node(5).await;
    let cert = DelegationCertificate::issue(
        &Identity::generate().unwrap(),
        Identity::generate().unwrap().public_key(),
        10,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    node.registry.set_instance_cert(SAGA_CALLER.to_string(), cert).await.unwrap();

    let result = node
        .router
        .saga_begin(SagaBegin {
            caller_service_id: SAGA_CALLER.to_string(),
            app_instance_id: None,
            name: "wf".to_string(),
            deadline_secs: Some(60),
        })
        .await;
    assert!(
        result.is_ok(),
        "a deadline outliving the certificate must warn, not refuse: {result:?}"
    );
}

#[tokio::test]
async fn begin_is_refused_above_the_deadline_ceiling() {
    let node = saga_node(5).await;
    let result = node
        .router
        .saga_begin(SagaBegin {
            caller_service_id: SAGA_CALLER.to_string(),
            app_instance_id: None,
            name: "wf".to_string(),
            deadline_secs: Some(999_999_999),
        })
        .await;
    assert!(
        matches!(result, Err(ProxyError::PermissionDenied(_))),
        "a deadline above the ceiling must be refused rather than clamped, got {result:?}"
    );
}

#[tokio::test]
async fn a_deadline_of_none_takes_the_node_default() {
    let node = saga_node(5).await;
    let saga_id = begun_saga(&node).await;
    let info = node.router.saga_status(SAGA_CALLER, &saga_id).await.unwrap();
    let expected = saga_config(5).default_deadline_ms;
    assert_eq!(info.deadline_at - info.created_at, expected);
}

#[tokio::test]
async fn commit_drops_the_log_so_a_later_compensate_is_refused() {
    let node = saga_node(5).await;
    let saga_id = begun_saga(&node).await;

    node.router.saga_commit(SAGA_CALLER, &saga_id).await.unwrap();

    let status = node.router.saga_status(SAGA_CALLER, &saga_id).await;
    assert!(status.is_err(), "a committed saga must be gone");
    let compensate = node.router.saga_compensate(SAGA_CALLER, &saga_id).await;
    assert!(compensate.is_err(), "compensate must be refused after commit");
}

#[tokio::test]
async fn a_saga_id_is_minted_by_the_host_and_is_unique_per_begin() {
    let node = saga_node(5).await;
    let a = begun_saga(&node).await;
    let b = begun_saga(&node).await;
    assert_ne!(a, b);
}

#[tokio::test]
async fn two_concurrent_begins_through_a_cold_cache_share_one_log_handle() {
    let node = saga_node(5).await;
    let router_a = node.router.clone();
    let router_b = node.router.clone();
    let (a, b) = tokio::join!(
        router_a.saga_begin(SagaBegin {
            caller_service_id: SAGA_CALLER.to_string(),
            app_instance_id: None,
            name: "wf-a".to_string(),
            deadline_secs: None,
        }),
        router_b.saga_begin(SagaBegin {
            caller_service_id: SAGA_CALLER.to_string(),
            app_instance_id: None,
            name: "wf-b".to_string(),
            deadline_secs: None,
        }),
    );
    let (a, b) = (a.unwrap(), b.unwrap());
    assert_ne!(a, b);
    // Both sagas are visible through the same log -- if two separate
    // connections had been opened, one write could be invisible to a
    // read through the other handle.
    assert!(node.router.saga_status(SAGA_CALLER, &a).await.is_ok());
    assert!(node.router.saga_status(SAGA_CALLER, &b).await.is_ok());
}

// -- merging the forward result into an undo's parameters -----------

#[test]
fn merge_forward_result_adds_a_member_to_an_object_and_an_element_to_an_array() {
    let object = serde_json::json!({"item": "a"});
    let merged = merge_forward_result(&object, Some(&Value::String("id-1".to_string())));
    assert_eq!(merged, serde_json::json!({"item": "a", "forward-result": "id-1"}));

    let array = serde_json::json!(["a", 1]);
    let merged = merge_forward_result(&array, Some(&Value::String("id-1".to_string())));
    assert_eq!(merged, serde_json::json!(["a", 1, "id-1"]));
}

#[test]
fn merge_forward_result_makes_an_object_when_the_forward_params_were_null() {
    let merged = merge_forward_result(&Value::Null, Some(&Value::String("id-1".to_string())));
    assert_eq!(merged, serde_json::json!({"forward-result": "id-1"}));
}

#[test]
fn a_forward_call_that_returned_nothing_sends_no_forward_result() {
    let object = serde_json::json!({"item": "a"});
    let merged = merge_forward_result(&object, None);
    assert_eq!(merged, object);
}

// -- bookkeeping shortens the step budget ----------------------------

#[test]
fn bookkeeping_before_the_call_shortens_the_step_budget_rather_than_extending_the_epoch() {
    // The whole rule in one number: a slower bookkeeping phase (the log
    // open plus the intent write) must leave the call *less* time, not
    // the same amount tacked on top of it -- otherwise a guest's own
    // epoch could be overrun by exactly the bookkeeping cost this
    // subtraction exists to protect against.
    let fast = step_call_budget_ms(4_000, Duration::from_millis(10));
    let slow = step_call_budget_ms(4_000, Duration::from_millis(1_500));
    assert_eq!(fast, 3_990);
    assert_eq!(slow, 2_500);
    assert!(
        slow < fast,
        "more bookkeeping time must leave a smaller call budget, not a larger one"
    );
}

#[test]
fn the_step_budget_floors_at_the_minimum_rather_than_going_negative() {
    // Bookkeeping that ate the whole epoch (a very cold open, or a tiny
    // configured epoch) must not produce a zero or negative budget --
    // `saturating_sub` alone would floor at zero, which is not a call
    // budget, it is an instant refusal.
    let budget = step_call_budget_ms(4_000, Duration::from_secs(10));
    assert_eq!(budget, syneroym_async_queue::MIN_STEP_CALL_BUDGET_MS);
}
