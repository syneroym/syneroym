//! The coordinator's own record in its community registry: register, then
//! register again on a timer so the record does not expire.

use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use iroh::{EndpointAddr, SecretKey};
use syneroym_core::{
    config::RetryPolicy,
    dht_registry::{
        DEFAULT_ENDPOINT_NOT_AFTER_SECS, DEFAULT_REGISTRY_TTL_SECS, EndpointInfo,
        EndpointMechanism, EndpointType, HEARTBEAT_INTERVAL_SECS, RegistryClient,
    },
    retry,
};
use syneroym_identity::{Identity, substrate};
use tokio::{sync::watch, task::JoinHandle, time};
use tracing::{debug, warn};

/// Wait after a successful registration.
pub(crate) const REGISTRATION_REFRESH: Duration = Duration::from_secs(HEARTBEAT_INTERVAL_SECS);
/// Wait after a registration that failed all its retries.
pub(crate) const REGISTRATION_RETRY: Duration = Duration::from_secs(60);

// The registry drops a record that is not refreshed within its TTL, so the
// refresh must come first.
const _: () = assert!(HEARTBEAT_INTERVAL_SECS < DEFAULT_REGISTRY_TTL_SECS);

/// The DID the coordinator registers under: `did:key` of its Iroh key.
pub(crate) fn coordinator_did(secret_key: &SecretKey) -> String {
    substrate::derive_did_key(&Identity::from_bytes(&secret_key.to_bytes()).public_key())
}

/// What one registration needs. Shared by every round of the loop.
#[derive(Debug)]
struct Registration {
    registry: RegistryClient,
    secret_key: SecretKey,
    relay_url: Option<String>,
    retry_policy: RetryPolicy,
}

/// Registers the coordinator's record and keeps it registered until the task
/// is aborted. `registered` holds the result of the last attempt.
pub(crate) fn spawn(
    registry_url: String,
    secret_key: SecretKey,
    relay_url: Option<String>,
    retry_policy: RetryPolicy,
    registered: watch::Sender<bool>,
) -> JoinHandle<()> {
    let registration = Arc::new(Registration {
        // The coordinator has never published to the DHT.
        registry: RegistryClient::new(false, Some(registry_url)),
        secret_key,
        relay_url,
        retry_policy,
    });
    tokio::spawn(keep_registered(
        move || {
            let registration = Arc::clone(&registration);
            async move { registration.register_with_retries().await }
        },
        registered,
    ))
}

/// Calls `register` now, then again after `REGISTRATION_REFRESH` when it
/// succeeded or `REGISTRATION_RETRY` when it failed. Never returns.
pub(crate) async fn keep_registered<F, Fut>(mut register: F, registered: watch::Sender<bool>)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    loop {
        let ok = register().await;
        registered.send_replace(ok);
        time::sleep(if ok { REGISTRATION_REFRESH } else { REGISTRATION_RETRY }).await;
    }
}

impl Registration {
    async fn register_with_retries(&self) -> bool {
        let res = retry::retry_with_backoff(&self.retry_policy, || self.register_once()).await;
        match &res {
            Ok(()) => debug!("coordinator registered in its community registry"),
            Err(e) => warn!("coordinator registration failed after all retries: {e}"),
        }
        res.is_ok()
    }

    /// Signs a fresh record (new timestamp, so the registry accepts it as
    /// newer) and posts it.
    async fn register_once(&self) -> Result<()> {
        let node_id = self.secret_key.public();
        let endpoint_addr_bytes = serde_json::to_vec(&EndpointAddr::new(node_id))?;
        let identity = Identity::from_bytes(&self.secret_key.to_bytes());
        let did = coordinator_did(&self.secret_key);
        let not_after = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
            .saturating_add(DEFAULT_ENDPOINT_NOT_AFTER_SECS);
        let info = EndpointInfo {
            service_id: did.clone(),
            substrate_id: did,
            endpoint_type: EndpointType::Substrate,
            nickname: Some(format!("coordinator-{}", &node_id.to_string()[..8])),
            mechanisms: vec![EndpointMechanism::Iroh {
                endpoint_addr_bytes,
                relay_url: self.relay_url.clone(),
            }],
            is_private: false,
            ttl: None,
            not_after,
            generation: 0,
        };
        self.registry.register(&info.sign(&identity)?, false).await
    }
}

#[cfg(test)]
mod tests;
