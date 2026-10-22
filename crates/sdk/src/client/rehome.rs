use std::sync::{PoisonError, atomic::Ordering};

use anyhow::Result;
use iroh::{Endpoint, EndpointAddr, RelayMap, RelayMode, RelayUrl};
use syneroym_core::dht_registry::EndpointMechanism;
use syneroym_router::{E2eError, SYNEROYM_ALPN};
use syneroym_rpc::{JsonRpcError, NOT_HOSTED_HERE_RPC_CODE};
use tokio::time;

use super::{SyneroymClient, TransportConnection, spawn_background_close};

impl SyneroymClient {
    pub(crate) fn can_rehome(&self) -> bool {
        self.provided_mechanisms.is_none() && !self.registry_url.is_empty()
    }

    pub(crate) async fn dial(
        &self,
        mechanisms: Vec<EndpointMechanism>,
        looked_up_host: Option<&str>,
    ) -> Result<TransportConnection> {
        // Try mechanisms. Currently only Iroh is implemented.
        for mechanism in mechanisms {
            match mechanism {
                EndpointMechanism::Iroh { endpoint_addr_bytes, relay_url } => {
                    let mut endpoint_addr: EndpointAddr =
                        serde_json::from_slice(&endpoint_addr_bytes)?;

                    let mut ep_bldr = Endpoint::empty_builder();
                    if let Some(relay) = relay_url
                        && let Ok(url) = relay.parse::<RelayUrl>()
                    {
                        ep_bldr =
                            ep_bldr.relay_mode(RelayMode::Custom(RelayMap::from(url.clone())));
                        endpoint_addr = endpoint_addr.with_relay_url(url);
                    }

                    let endpoint = ep_bldr.bind().await?;
                    let dial = endpoint.connect(endpoint_addr, SYNEROYM_ALPN);
                    match time::timeout(self.connect_timeout, dial).await {
                        Ok(Ok(conn)) => {
                            let host_key = self
                                .host_key_for(looked_up_host, conn.remote_id().as_bytes())
                                .await;
                            return Ok(TransportConnection::Iroh { endpoint, conn, host_key });
                        }
                        Ok(Err(e)) => {
                            self.push_pending_close(spawn_background_close(endpoint));
                            return Err(e.into());
                        }
                        Err(_) => {
                            self.push_pending_close(spawn_background_close(endpoint));
                            return Err(anyhow::anyhow!(
                                "connect to {} timed out after {:?}",
                                self.service_id,
                                self.connect_timeout
                            ));
                        }
                    }
                }
                EndpointMechanism::WebRtc { .. } => {
                    // Not implemented
                }
            }
        }

        Err(anyhow::anyhow!("No supported communication mechanism found for {}", self.service_id))
    }

    #[doc(hidden)]
    pub fn rehome_count(&self) -> usize {
        self.rehome_count.load(Ordering::SeqCst)
    }

    /// Swaps in a connection to the service's current host, unless another
    /// request already did since `failed` was used.
    #[doc(hidden)]
    pub async fn rehome(&self, failed: &TransportConnection) -> Result<()> {
        let _serial = self.rehome_lock.lock().await;
        if self.current_stable_id() != Some(failed.stable_id()) {
            return Ok(());
        }
        let target = self.registry_lookup_override.as_deref().unwrap_or(&self.service_id);
        let info = self.registry_client().lookup(target, true).await?.info;
        let conn = self.dial(info.mechanisms, Some(&info.substrate_id)).await?;
        self.rehome_count.fetch_add(1, Ordering::SeqCst);
        let old = self.connection.lock().unwrap_or_else(PoisonError::into_inner).replace(conn);
        if let Some(TransportConnection::Iroh { endpoint, .. }) = old {
            self.push_pending_close(spawn_background_close(endpoint));
        }
        Ok(())
    }
}

pub(crate) fn is_not_hosted_here(e: &anyhow::Error) -> bool {
    e.downcast_ref::<JsonRpcError>().is_some_and(|j| j.code == NOT_HOSTED_HERE_RPC_CODE)
}

pub(crate) fn is_bad_host(e: &anyhow::Error) -> bool {
    e.downcast_ref::<E2eError>().is_some_and(|x| matches!(x, E2eError::BadHostSignature { .. }))
}
