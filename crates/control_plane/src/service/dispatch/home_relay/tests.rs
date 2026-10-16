use std::sync::{Arc, Mutex};

use serde_json::json;
use syneroym_passport::wire::{HomeRelaySetRequest, HomeRelayStatus};
use syneroym_rpc::{RpcError, RpcResult};

use super::*;
use crate::home_relay::HomeRelayControl;

#[derive(Debug, Default)]
struct FakeHomeRelayControl {
    status: Mutex<Option<HomeRelayStatus>>,
}

#[async_trait::async_trait]
impl HomeRelayControl for FakeHomeRelayControl {
    async fn set_passport(&self, _request: HomeRelaySetRequest) -> RpcResult<HomeRelayStatus> {
        let guard = self.status.lock().unwrap();
        guard.clone().ok_or_else(|| RpcError::InvalidParams("no fake status configured".into()))
    }

    fn status(&self) -> Option<HomeRelayStatus> {
        self.status.lock().unwrap().clone()
    }
}

fn sample_status() -> HomeRelayStatus {
    HomeRelayStatus {
        relay_url: "https://relay.example.org".into(),
        info_url: "https://info.example.org".into(),
        active: true,
        passport_id: "a".repeat(64).parse().unwrap(),
        host_node_did: "did:key:host".into(),
        owner_did: "did:key:owner".into(),
        access_ends_at: 1000,
        lease_ends_at: Some(500),
        last_attempt_at: Some(100),
        last_error: None,
        next_attempt_at: Some(400),
    }
}

#[tokio::test]
async fn no_control_returns_method_not_found() {
    let err_set = set(None, json!([])).await.unwrap_err();
    assert!(
        matches!(err_set, RpcError::MethodNotFound(msg) if msg.contains("no fixed home relay"))
    );

    let err_status = status(None).unwrap_err();
    assert!(
        matches!(err_status, RpcError::MethodNotFound(msg) if msg.contains("no fixed home relay"))
    );
}

#[tokio::test]
async fn bad_params_returns_invalid_params() {
    let control: Arc<dyn HomeRelayControl> = Arc::new(FakeHomeRelayControl::default());
    let err = set(Some(&control), json!("not an array")).await.unwrap_err();
    assert!(matches!(err, RpcError::InvalidParams(_)));
}

#[tokio::test]
async fn set_returns_status() {
    let fake = FakeHomeRelayControl::default();
    let expected = sample_status();
    *fake.status.lock().unwrap() = Some(expected.clone());
    let control: Arc<dyn HomeRelayControl> = Arc::new(fake);

    let dummy_request = json!([{
        "token": {
            "issuer_did": "did:key:owner",
            "audience_did": "did:key:subject",
            "capabilities": [],
            "facts": {},
            "not_before_secs": 0,
            "expires_at_secs": 1000,
            "proofs": [],
            "signature": ""
        },
        "info_url": "https://info.example.org"
    }]);

    let res = set(Some(&control), dummy_request).await.unwrap();
    let back: HomeRelayStatus = serde_json::from_value(res.payload).unwrap();
    assert_eq!(back, expected);
}

#[test]
fn status_returns_null_when_none() {
    let control: Arc<dyn HomeRelayControl> = Arc::new(FakeHomeRelayControl::default());
    let res = status(Some(&control)).unwrap();
    assert_eq!(res.payload, serde_json::Value::Null);
}

#[test]
fn status_returns_status_when_some() {
    let fake = FakeHomeRelayControl::default();
    let expected = sample_status();
    *fake.status.lock().unwrap() = Some(expected.clone());
    let control: Arc<dyn HomeRelayControl> = Arc::new(fake);

    let res = status(Some(&control)).unwrap();
    let back: HomeRelayStatus = serde_json::from_value(res.payload).unwrap();
    assert_eq!(back, expected);
}
