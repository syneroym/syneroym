use std::sync::{Arc, Mutex};

use serde_json::Value;
use syneroym_rpc::{NOT_HOSTED_HERE_RPC_CODE, ProxyError};

use crate::proxy::tests::helpers::{
    MockHop, MockOutcome, base_request, empty_registry, synthetic_addr, test_router,
};

#[tokio::test]
async fn not_hosted_then_success_at_new_address_non_idempotent() {
    let hop = Arc::new(MockHop::with_outcomes(vec![
        MockOutcome::Callee {
            code: NOT_HOSTED_HERE_RPC_CODE,
            message: "not hosted here".to_string(),
        },
        MockOutcome::Success(Value::String("ok".to_string())),
    ]));
    let router = test_router(hop.clone(), empty_registry());
    let req = base_request("target-svc", "op");

    let addr1 = synthetic_addr();
    let addr2 = synthetic_addr();
    let addrs = Arc::new(Mutex::new(vec![addr1, addr2]));

    let res = router
        .invoke_rehoming(&req, || {
            let addrs = addrs.clone();
            async move { Ok(addrs.lock().unwrap().remove(0)) }
        })
        .await
        .unwrap();

    assert_eq!(res, Value::String("ok".to_string()));
    assert_eq!(hop.call_count(), 2);
}

#[tokio::test]
async fn not_hosted_then_success_at_new_address_idempotent() {
    let hop = Arc::new(MockHop::with_outcomes(vec![
        MockOutcome::Callee {
            code: NOT_HOSTED_HERE_RPC_CODE,
            message: "not hosted here".to_string(),
        },
        MockOutcome::Success(Value::String("ok".to_string())),
    ]));
    let router = test_router(hop.clone(), empty_registry());
    let mut req = base_request("target-svc", "op");
    req.idempotent = true;

    let addr1 = synthetic_addr();
    let addr2 = synthetic_addr();
    let addrs = Arc::new(Mutex::new(vec![addr1, addr2]));

    let res = router
        .invoke_rehoming(&req, || {
            let addrs = addrs.clone();
            async move { Ok(addrs.lock().unwrap().remove(0)) }
        })
        .await
        .unwrap();

    assert_eq!(res, Value::String("ok".to_string()));
    assert_eq!(hop.call_count(), 2);
}

#[tokio::test]
async fn resolver_returns_same_endpoint_id_fails_service_not_found() {
    let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Callee {
        code: NOT_HOSTED_HERE_RPC_CODE,
        message: "not hosted here".to_string(),
    }]));
    let router = test_router(hop.clone(), empty_registry());
    let req = base_request("target-svc", "op");

    let addr = synthetic_addr();
    let addrs = Arc::new(Mutex::new(vec![addr.clone(), addr]));

    let err = router
        .invoke_rehoming(&req, || {
            let addrs = addrs.clone();
            async move { Ok(addrs.lock().unwrap().remove(0)) }
        })
        .await
        .unwrap_err();

    assert!(matches!(err, ProxyError::ServiceNotFound(svc) if svc == "target-svc"));
    assert_eq!(hop.call_count(), 1);
}

#[tokio::test]
async fn not_hosted_twice_fails_service_not_found() {
    let hop = Arc::new(MockHop::with_outcomes(vec![
        MockOutcome::Callee {
            code: NOT_HOSTED_HERE_RPC_CODE,
            message: "not hosted here".to_string(),
        },
        MockOutcome::Callee {
            code: NOT_HOSTED_HERE_RPC_CODE,
            message: "not hosted here again".to_string(),
        },
    ]));
    let router = test_router(hop.clone(), empty_registry());
    let req = base_request("target-svc", "op");

    let addr1 = synthetic_addr();
    let addr2 = synthetic_addr();
    let addrs = Arc::new(Mutex::new(vec![addr1, addr2]));

    let err = router
        .invoke_rehoming(&req, || {
            let addrs = addrs.clone();
            async move { Ok(addrs.lock().unwrap().remove(0)) }
        })
        .await
        .unwrap_err();

    assert!(matches!(err, ProxyError::ServiceNotFound(svc) if svc == "target-svc"));
    assert_eq!(hop.call_count(), 2);
}

#[tokio::test]
async fn transport_error_is_not_rehomed() {
    let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Transport]));
    let router = test_router(hop.clone(), empty_registry());
    let req = base_request("target-svc", "op");

    let addr1 = synthetic_addr();
    let addr2 = synthetic_addr();
    let addrs = Arc::new(Mutex::new(vec![addr1, addr2]));

    let err = router
        .invoke_rehoming(&req, || {
            let addrs = addrs.clone();
            async move { Ok(addrs.lock().unwrap().remove(0)) }
        })
        .await
        .unwrap_err();

    assert!(matches!(err, ProxyError::Transport(_)));
    assert_eq!(hop.call_count(), 1);
}
