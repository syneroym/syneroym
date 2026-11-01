use std::{sync::Arc, time::Duration};

use serde_json::json;
use syneroym_wit_interfaces::host::syneroym::proxy::proxy;
use tokio::sync::Semaphore;

use super::{ScriptedProxy, req, test_host};
use crate::host_capabilities::{CallAll, CallAllLimits, tests::test_service_proxy};

#[tokio::test]
async fn a_read_only_instance_cannot_run_a_batch() {
    let proxy = Arc::new(ScriptedProxy::default());
    let temp_dir = tempfile::tempdir().unwrap();
    let mut host = test_host("svc", &proxy, true, temp_dir.path(), None, None);

    let err = proxy::Host::call_all(&mut host, vec![req("svc", "echo", json!("1"), None)])
        .await
        .unwrap_err();
    let proxy::ProxyError::Internal(msg) = err else {
        panic!("expected Internal, got {err:?}");
    };
    assert_eq!(msg, "stage-4 after-step instances may not originate proxy calls");
}

#[tokio::test]
async fn a_missing_proxy_is_refused_with_the_text_of_call() {
    let temp_dir = tempfile::tempdir().unwrap();
    let proxy = Arc::new(ScriptedProxy::default());
    let mut host = test_host("svc", &proxy, false, temp_dir.path(), None, None);
    host.service_proxy = test_service_proxy();

    let err = proxy::Host::call_all(&mut host, vec![req("svc", "echo", json!("1"), None)])
        .await
        .unwrap_err();
    let proxy::ProxyError::Internal(msg) = err else {
        panic!("expected Internal, got {err:?}");
    };
    assert_eq!(msg, "proxy unavailable");
}

#[tokio::test]
async fn refusal_texts_follow_wit_contract() {
    let proxy = Arc::new(ScriptedProxy::default());
    let temp_dir = tempfile::tempdir().unwrap();

    let mut host1 = test_host("svc", &proxy, true, temp_dir.path(), None, None);
    let err1 = proxy::Host::call_all(&mut host1, vec![req("svc", "echo", json!("1"), None)])
        .await
        .unwrap_err();
    let proxy::ProxyError::Internal(msg1) = err1 else {
        panic!("expected Internal, got {err1:?}");
    };
    assert_eq!(msg1, "stage-4 after-step instances may not originate proxy calls");

    let mut host2 = test_host("svc", &proxy, false, temp_dir.path(), None, None);
    host2.service_proxy = test_service_proxy();
    let err2 = proxy::Host::call_all(&mut host2, vec![req("svc", "echo", json!("1"), None)])
        .await
        .unwrap_err();
    let proxy::ProxyError::Internal(msg2) = err2 else {
        panic!("expected Internal, got {err2:?}");
    };
    assert_eq!(msg2, "proxy unavailable");

    let mut host3 = test_host("svc", &proxy, false, temp_dir.path(), None, None);
    let sem = Arc::new(Semaphore::new(0));
    host3.call_all = CallAll::new(CallAllLimits::default(), 0);
    let err3 = proxy::Host::call_all(&mut host3, vec![req("svc", "echo", json!("1"), None)])
        .await
        .unwrap_err();
    let proxy::ProxyError::Internal(msg3) = err3 else {
        panic!("expected Internal, got {err3:?}");
    };
    assert!(msg3.starts_with("call-all busy:"));
    drop(sem);

    let mut host4 = test_host("svc", &proxy, false, temp_dir.path(), None, None);
    host4.epoch = crate::engine::EpochBudget::new(Duration::from_millis(500));
    let err4 = proxy::Host::call_all(&mut host4, vec![req("svc", "echo", json!("1"), None)])
        .await
        .unwrap_err();
    let proxy::ProxyError::Internal(msg4) = err4 else {
        panic!("expected Internal, got {err4:?}");
    };
    assert!(msg4.starts_with("call-all busy:"));
}

#[tokio::test]
async fn the_waiter_limit_refuses_at_once_and_a_permit_is_released_after_the_batch() {
    let proxy = Arc::new(ScriptedProxy::default());
    let temp_dir = tempfile::tempdir().unwrap();
    let mut host = test_host("svc", &proxy, false, temp_dir.path(), None, None);
    let limits = CallAllLimits::default();
    let call_all = CallAll::new(limits, 1);
    host = host.with_call_all(call_all.clone());

    let permit = call_all.waiters.clone().try_acquire_owned().unwrap();
    let err = proxy::Host::call_all(&mut host, vec![req("svc", "echo", json!("1"), None)])
        .await
        .unwrap_err();
    assert!(matches!(err, proxy::ProxyError::Internal(_)));
    drop(permit);

    let res =
        proxy::Host::call_all(&mut host, vec![req("svc", "echo", json!("1"), None)]).await.unwrap();
    assert_eq!(res.len(), 1);
    assert_eq!(res[0].as_deref().unwrap(), "1");
}

#[tokio::test]
async fn a_nested_batch_at_the_waiter_limit_is_refused_and_does_not_hang() {
    let proxy = Arc::new(ScriptedProxy::default());
    let temp_dir = tempfile::tempdir().unwrap();

    let limits = CallAllLimits::default();
    let shared_call_all = CallAll::new(limits, 1);

    let host2 = Arc::new(tokio::sync::Mutex::new(
        test_host("svc2", &proxy, false, temp_dir.path(), None, None)
            .with_call_all(shared_call_all.clone()),
    ));

    let host2_clone = host2.clone();
    *proxy.nested_hook.lock().unwrap() = Some(Arc::new(Box::new(move || {
        let h = host2_clone.clone();
        Box::pin(async move {
            let mut guard = h.lock().await;
            let calls = vec![req("svc2", "echo", json!("nested"), None)];
            match proxy::Host::call_all(&mut *guard, calls).await {
                Ok(_) => Ok(serde_json::Value::String("nested_ok".to_string())),
                Err(e) => Err(syneroym_rpc::ProxyError::Internal(format!("nested refused: {e:?}"))),
            }
        })
    })));

    let mut host1 = test_host("svc1", &proxy, false, temp_dir.path(), None, None)
        .with_call_all(shared_call_all.clone());

    let res = proxy::Host::call_all(&mut host1, vec![req("svc1", "nested", json!({}), None)])
        .await
        .unwrap();

    assert_eq!(res.len(), 1);
    let Err(proxy::ProxyError::Internal(msg)) = &res[0] else {
        panic!("expected nested refusal, got {:?}", res[0]);
    };
    assert!(msg.contains("nested refused"));
}

#[tokio::test]
async fn the_wait_allowance_shrinks_the_total_time_and_runs_out() {
    let proxy = Arc::new(ScriptedProxy::default());
    let temp_dir = tempfile::tempdir().unwrap();
    let limits = CallAllLimits {
        total_timeout: Duration::from_millis(2500),
        default_call_timeout: Duration::from_millis(2000),
        max_call_timeout: Duration::from_millis(2000),
        ..Default::default()
    };
    let mut host = test_host("svc", &proxy, false, temp_dir.path(), None, None)
        .with_call_all(CallAll::new(limits, 4));

    let res1 =
        proxy::Host::call_all(&mut host, vec![req("svc", "sleep", json!({"ms": 1600}), None)])
            .await
            .unwrap();
    assert_eq!(res1.len(), 1);
    assert_eq!(res1[0].as_deref().unwrap(), "slept");

    let allowance = host.epoch.allowance();
    assert!(allowance < Duration::from_secs(1));

    let err = proxy::Host::call_all(&mut host, vec![req("svc", "sleep", json!({"ms": 10}), None)])
        .await
        .unwrap_err();
    let proxy::ProxyError::Internal(msg) = err else {
        panic!("expected Internal, got {err:?}");
    };
    assert_eq!(msg, "call-all busy: this guest call has no wait time left in call-all");
}

#[tokio::test]
async fn a_refused_batch_gives_no_credit_and_a_timed_out_batch_does_give_it() {
    let proxy = Arc::new(ScriptedProxy::default());
    let temp_dir = tempfile::tempdir().unwrap();
    let limits = CallAllLimits {
        max_calls: 1,
        total_timeout: Duration::from_millis(2000),
        default_call_timeout: Duration::from_millis(50),
        max_call_timeout: Duration::from_millis(500),
        ..Default::default()
    };
    let mut host = test_host("svc", &proxy, false, temp_dir.path(), None, None)
        .with_call_all(CallAll::new(limits, 4));

    let initial_allowance = host.epoch.allowance();

    let _ = proxy::Host::call_all(
        &mut host,
        vec![req("svc", "echo", json!("1"), None), req("svc", "echo", json!("2"), None)],
    )
    .await;
    assert_eq!(host.epoch.allowance(), initial_allowance);

    let _ =
        proxy::Host::call_all(&mut host, vec![req("svc", "sleep", json!({"ms": 500}), Some(50))])
            .await;
    assert!(host.epoch.allowance() < initial_allowance);
}

#[tokio::test]
async fn allowance_below_min_batch_allowance_refuses() {
    let proxy = Arc::new(ScriptedProxy::default());
    let temp_dir = tempfile::tempdir().unwrap();
    let mut host = test_host("svc", &proxy, false, temp_dir.path(), None, None);
    host.epoch = crate::engine::EpochBudget::new(Duration::from_millis(999));

    let err = proxy::Host::call_all(&mut host, vec![req("svc", "echo", json!("1"), None)])
        .await
        .unwrap_err();
    let proxy::ProxyError::Internal(msg) = err else {
        panic!("expected Internal, got {err:?}");
    };
    assert_eq!(msg, "call-all busy: this guest call has no wait time left in call-all");
}
