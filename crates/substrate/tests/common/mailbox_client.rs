//! Plain helpers for talking to a mailbox from a test: the clients and the
//! calls, with no receiver behaviour.

#![allow(dead_code)]

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;
use syneroym_core::{dht_registry::DEFAULT_ENDPOINT_NOT_AFTER_SECS, mailbox::MAILBOX_INTERFACE};
use syneroym_identity::{DelegationCertificate, Identity, substrate};
use syneroym_rpc::JsonRpcError;
use syneroym_sdk::SyneroymClient;

use super::SubstrateNode;

pub fn did_of(identity: &Identity) -> String {
    substrate::derive_did_key(&identity.public_key())
}

pub fn not_after() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .saturating_add(DEFAULT_ENDPOINT_NOT_AFTER_SECS)
}

/// A client that presents `instance` and its certificate to `node`.
pub async fn instance_client(
    node: &SubstrateNode,
    instance: &Identity,
    cert: &DelegationCertificate,
) -> SyneroymClient {
    let mut client =
        node.client_as(Identity::from_bytes(&instance.to_bytes())).with_delegation(cert.clone());
    client.connect().await.expect("failed to connect instance client");
    client
}

/// One mailbox method, with the JSON-RPC error split out as `(code, data)`.
pub async fn call(
    client: &SyneroymClient,
    method: &str,
    params: Value,
) -> Result<Value, (i32, Value)> {
    match client.request(MAILBOX_INTERFACE, method, params).await {
        Ok(res) => Ok(res.result),
        Err(e) => match e.downcast_ref::<JsonRpcError>() {
            Some(rpc_err) => Err((rpc_err.code, rpc_err.data.clone().unwrap_or(Value::Null))),
            None => panic!("unexpected transport or parse error: {e:?}"),
        },
    }
}
