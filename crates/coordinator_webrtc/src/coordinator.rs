//! WebRTC Ecosystem Coordinator
//!
//! Orchestrates signaling and peer-to-peer transport bridging using WebRTC,
//! handling peer discovery and connection routing.

use std::{
    collections::HashMap,
    fmt::{self, Debug, Formatter},
    sync::Arc,
};

use anyhow::Result;
use iroh::Endpoint;
use syneroym_app_orchestration::{LogicalResolver, StaticInventory};
use syneroym_core::{
    config::SubstrateConfig, dht_registry::RegistryClient, local_registry::EndpointRegistry,
    util::load_or_generate_node_identity,
};
use syneroym_data_db::registry_store;
use syneroym_router::net_iroh;
use syneroym_sdk::topology::{
    AppHostResolver, RegistryTier1Lookup, RegistryTopologyFetcher, Tier2Fetch, credential_warning,
};
use tokio::{net::TcpListener, sync::Mutex};
use tracing::{debug, info, warn};

use crate::{
    bootstrap::{self, BootstrapState},
    signalling,
};

pub struct CoordinatorWebRtc {
    bootstrap_listener: Option<TcpListener>,
    signalling_listener: Option<TcpListener>,
    bootstrap_state: Arc<BootstrapState>,
}

impl Debug for CoordinatorWebRtc {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("CoordinatorWebRtc")
            .field(
                "bootstrap_listener",
                &self.bootstrap_listener.as_ref().map(|l| l.local_addr().ok()),
            )
            .field(
                "signalling_listener",
                &self.signalling_listener.as_ref().map(|l| l.local_addr().ok()),
            )
            .field("bootstrap_state", &self.bootstrap_state)
            .finish()
    }
}

impl CoordinatorWebRtc {
    pub async fn init(config: &SubstrateConfig) -> Result<Self> {
        info!("Initializing coordinator webrtc");

        let webrtc_config = config
            .roles
            .coordinator
            .as_ref()
            .and_then(|c| c.webrtc.as_ref())
            .ok_or_else(|| anyhow::anyhow!("WebRTC coordinator configuration missing"))?;

        let bootstrap_listener =
            TcpListener::bind(&webrtc_config.bootstrap_page_bind_address).await?;
        let signalling_listener = TcpListener::bind(&webrtc_config.signalling_bind_address).await?;

        let actual_signalling_port = signalling_listener.local_addr()?.port();

        let iroh_relay_url = config.parent_coordinator.iroh.as_ref().map(|c| c.url.clone());
        let endpoint = net_iroh::build_iroh_endpoint(iroh_relay_url, None, None).await?;

        let data_store = registry_store::init_store(config).await?;
        let registry = EndpointRegistry::new(data_store).await?;

        let registry_client = RegistryClient::new(true, config.substrate.registry_url.clone());

        // S3, D-S3-11: the coordinator resolves app-scoped (`-a…-s…`)
        // bootstrap hosts the same way the client gateway does, over its
        // own `AppHostResolver` (D-S3-7 -- never shared). The blind tunnel
        // has no identity of its own; `resolve` is authorized, so it needs
        // one, and it reuses the node's own key (not a fresh one) so
        // `[iam].grant_resolve_to_node_did` covers this component with the
        // same config key the client gateway uses, and a coordinator
        // restart does not invalidate an operator's `resolve_ucan` token.
        let identity = load_or_generate_node_identity(config)?;
        let registry_url = config.substrate.registry_url.clone().unwrap_or_default();
        let resolve_ucan_path =
            config.roles.coordinator.as_ref().and_then(|c| c.resolve_ucan.as_ref());
        let fetcher = if registry_url.is_empty() {
            None
        } else {
            let mut f = RegistryTopologyFetcher::new(registry_url.clone()).with_identity(&identity);
            if let Some(path) = resolve_ucan_path {
                let raw = std::fs::read_to_string(path)?;
                f = f.with_ucan(serde_json::from_str(&raw)?);
            }
            match credential_warning(
                resolve_ucan_path.is_some(),
                config.iam.grant_resolve_to_node_did,
            ) {
                Some(syneroym_sdk::topology::CredentialWarning::NeitherConfigured) => warn!(
                    "coordinator has neither `roles.coordinator.resolve_ucan` nor \
                     `iam.grant_resolve_to_node_did`; app-scoped (-a…-s…) bootstrap hosts will be \
                     refused by any supervisor they reach. Unscoped (-s only) hosts are \
                     unaffected."
                ),
                Some(syneroym_sdk::topology::CredentialWarning::OnlyTheSameNodeGate) => debug!(
                    "coordinator has no `resolve_ucan`; app-scoped bootstrap hosts will resolve \
                     only for apps supervised by this node"
                ),
                None => {}
            }
            Some(Box::new(f) as Box<dyn Tier2Fetch>)
        };
        let tier1 = Box::new(RegistryTier1Lookup::new(registry_url.clone()));
        let app_host_resolver = AppHostResolver::new(
            tier1,
            fetcher,
            LogicalResolver::new(Arc::new(StaticInventory::new())),
        );

        let bootstrap_state = Arc::new(BootstrapState {
            iroh: endpoint,
            external_host: webrtc_config.external_host.clone(),
            signaling_port: actual_signalling_port,
            registry,
            registry_url: config.substrate.registry_url.clone(),
            registry_client,
            connection_cache: Mutex::new(HashMap::new()),
            app_host_resolver,
        });

        Ok(Self {
            bootstrap_listener: Some(bootstrap_listener),
            signalling_listener: Some(signalling_listener),
            bootstrap_state,
        })
    }

    pub fn bootstrap_port(&self) -> u16 {
        self.bootstrap_listener.as_ref().and_then(|l| l.local_addr().ok()).map_or(0, |a| a.port())
    }

    pub fn signalling_port(&self) -> u16 {
        self.signalling_listener.as_ref().and_then(|l| l.local_addr().ok()).map_or(0, |a| a.port())
    }

    pub fn endpoint(&self) -> Endpoint {
        self.bootstrap_state.iroh.clone()
    }

    pub async fn run(&mut self) -> Result<()> {
        info!("Running coordinator webrtc");

        let bootstrap_listener = self
            .bootstrap_listener
            .take()
            .ok_or_else(|| anyhow::anyhow!("Bootstrap listener already taken"))?;
        let signalling_listener = self
            .signalling_listener
            .take()
            .ok_or_else(|| anyhow::anyhow!("Signalling listener already taken"))?;

        let bootstrap_fut = bootstrap::start(bootstrap_listener, self.bootstrap_state.clone());
        let signalling_fut = signalling::start(signalling_listener);

        tokio::try_join!(bootstrap_fut, signalling_fut)?;

        Ok(())
    }

    pub async fn shutdown(&mut self) -> Result<()> {
        info!("Shutting down coordinator webrtc");
        self.bootstrap_state.iroh.close().await;
        Ok(())
    }
}
