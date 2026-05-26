//! Iroh Ecosystem Coordinator
//!
//! Establishes secure peer-to-peer tunnels using the Iroh protocol, hosting
//! the local Iroh endpoint and relay server.

use anyhow::{Context, Result};
use axum::{Router, routing::get};
use iroh::{RelayMap, RelayMode, RelayUrl, SecretKey};
use iroh_relay::server::Server;
use std::net::SocketAddr;
use std::sync::Arc;
use syneroym_core::config::SubstrateConfig;
use syneroym_identity::Identity;
use syneroym_identity::substrate::derive_did_key;
use syneroym_router::RouteHandler;
use tokio::net::TcpListener;
use tracing::{debug, info, warn};

use crate::config::build_relay_config;
use crate::info_endpoint::{CoordinatorInfo, InfoState, get_info};

pub struct CoordinatorIroh {
    relay_server: Option<Server>,
    iroh_router: Option<iroh::protocol::Router>,
    http_info_handle: Option<tokio::task::JoinHandle<()>>,
    info_addr: Option<SocketAddr>,
}

impl CoordinatorIroh {
    pub async fn init(config: &SubstrateConfig) -> Result<Self> {
        info!("Initializing Coordinator IROH");
        let config_roles = config.roles.coordinator.clone();

        let mut relay_server = None;
        let mut iroh_router = None;
        let mut http_info_handle = None;
        let mut info_addr = None;

        if let Some(role) = &config_roles {
            if role.iroh.as_ref().map(|i| i.enable_relay).unwrap_or(false) {
                let server_config = build_relay_config(role).await?;
                debug!("Iroh Relay Config built: {:?}", server_config.relay.is_some());
                relay_server = Some(
                    Server::spawn(server_config)
                        .await
                        .context("failed to spawn iroh relay server")?,
                );
            }

            if let Some(iroh_cfg) = &role.iroh {
                let community_registry_url = iroh_cfg.community_registry_url.clone();

                // 1. Build Iroh Endpoint
                let secret_key = SecretKey::generate(&mut rand::rng());
                let mut ep_bldr = iroh::Endpoint::builder(iroh::endpoint::presets::N0);

                let mut chosen_relay_url_str = None;
                if let Some(parent_url) = config.uplink.iroh.as_ref().map(|u| u.relay_url.clone()) {
                    chosen_relay_url_str = Some(parent_url);
                } else if iroh_cfg.enable_relay {
                    let actual_http_addr = relay_server.as_ref().and_then(|s| s.http_addr());
                    if let Some(addr) = actual_http_addr {
                        chosen_relay_url_str = Some(format!("http://{}", addr));
                    } else {
                        chosen_relay_url_str =
                            Some(format!("http://{}", iroh_cfg.http_bind_address));
                    }
                }

                if let Some(relay_url_str) = chosen_relay_url_str {
                    if let Ok(relay_url) = relay_url_str.parse::<RelayUrl>() {
                        ep_bldr = iroh::Endpoint::empty_builder()
                            .relay_mode(RelayMode::Custom(RelayMap::from(relay_url)));
                    }
                }
                let iroh_endpoint = ep_bldr.secret_key(secret_key.clone()).bind().await?;
                iroh_endpoint.online().await;

                let endpoint_addr = iroh_endpoint.addr();
                let node_id = endpoint_addr.id;
                debug!("Coordinator Iroh endpoint bound: {}", node_id);

                let parent_relay_url = config.uplink.iroh.as_ref().map(|u| u.relay_url.clone());
                // 2. Build RouteHandler
                let route_handler = RouteHandler::new_coordinator(
                    iroh_endpoint.clone(),
                    community_registry_url.clone(),
                    parent_relay_url,
                );

                // 3. Spawn Iroh Router
                let router = iroh::protocol::Router::builder(iroh_endpoint)
                    .accept(syneroym_router::SYNEROYM_ALPN, route_handler)
                    .spawn();
                iroh_router = Some(router);

                // 4. Start Axum /v1/info HTTP Server
                let mut http_info_addr: SocketAddr =
                    iroh_cfg.http_bind_address.parse().context("invalid http_bind_address")?;
                // Add 10 to the port to avoid conflict, or bind to port 0 for dynamic port in tests
                if http_info_addr.port() == 0 {
                    http_info_addr.set_port(0);
                } else {
                    http_info_addr.set_port(http_info_addr.port() + 10);
                }

                let listener = TcpListener::bind(http_info_addr).await?;
                let local_addr = listener.local_addr()?;
                info_addr = Some(local_addr);
                info!("Coordinator /v1/info listening on {}", local_addr);

                let endpoint_addr_payload = iroh::EndpointAddr::new(endpoint_addr.id);
                let endpoint_addr_bytes = serde_json::to_vec(&endpoint_addr_payload)?;
                let actual_http_addr = relay_server.as_ref().and_then(|s| s.http_addr());
                let local_relay_url = if iroh_cfg.enable_relay {
                    if let Some(addr) = actual_http_addr {
                        Some(format!("http://{}", addr))
                    } else {
                        Some(format!("http://{}", iroh_cfg.http_bind_address))
                    }
                } else {
                    None
                };

                let parent_relay_url = config.uplink.iroh.as_ref().map(|u| u.relay_url.clone());

                let info_state = Arc::new(InfoState {
                    info: CoordinatorInfo {
                        endpoint_addr_bytes,
                        node_id: node_id.to_string(),
                        relay_url: local_relay_url,
                        parent_relay_url,
                    },
                });

                let app = Router::new().route("/v1/info", get(get_info)).with_state(info_state);

                let handle = tokio::spawn(async move {
                    if let Err(e) = axum::serve(listener, app).await {
                        warn!("HTTP info server error: {}", e);
                    }
                });
                http_info_handle = Some(handle);

                // 5. Register in Global Registry if requested
                if iroh_cfg.share_in_registry && iroh_cfg.community_registry_url.is_some() {
                    let registry_url = iroh_cfg.community_registry_url.clone().unwrap();
                    let node_id_str = node_id.to_string();
                    let secret_key_bytes = secret_key.to_bytes();

                    let endpoint_addr_payload = iroh::EndpointAddr::new(endpoint_addr.id);
                    let actual_http_addr = relay_server.as_ref().and_then(|s| s.http_addr());
                    let relay_url_payload = if let Some(ref parent_url) =
                        config.uplink.iroh.as_ref().map(|u| &u.relay_url)
                    {
                        Some((*parent_url).clone())
                    } else if iroh_cfg.enable_relay {
                        if let Some(addr) = actual_http_addr {
                            Some(format!("http://{}", addr))
                        } else {
                            Some(format!("http://{}", iroh_cfg.http_bind_address))
                        }
                    } else {
                        None
                    };

                    tokio::spawn(async move {
                        let mut attempts = 0;
                        while attempts < 30 {
                            match register_coordinator_in_registry(
                                &registry_url,
                                &node_id_str,
                                &endpoint_addr_payload,
                                relay_url_payload.clone(),
                                &secret_key_bytes,
                            )
                            .await
                            {
                                Ok(()) => {
                                    info!("Coordinator successfully registered in global registry");
                                    return;
                                }
                                Err(e) => {
                                    warn!(
                                        "Failed to register coordinator in registry (attempt {}): {}",
                                        attempts + 1,
                                        e
                                    );
                                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                    attempts += 1;
                                }
                            }
                        }
                    });
                }
            }
        }

        Ok(Self { relay_server, iroh_router, http_info_handle, info_addr })
    }

    pub fn info_addr(&self) -> Option<SocketAddr> {
        self.info_addr
    }

    pub fn endpoint_addr(&self) -> Option<iroh::EndpointAddr> {
        self.iroh_router.as_ref().map(|r| r.endpoint().addr())
    }

    pub async fn run(&mut self) -> Result<()> {
        info!("Running Coordinator IROH");

        let mut relay_fut = std::pin::pin!(async {
            if let Some(server) = &mut self.relay_server {
                server.task_handle().await.context("iroh relay server task panicked")??;
                Ok(())
            } else {
                std::future::pending::<Result<()>>().await
            }
        });

        let mut router_fut = std::pin::pin!(async {
            if let Some(router) = &self.iroh_router {
                router.endpoint().closed().await;
                Ok(())
            } else {
                std::future::pending::<Result<()>>().await
            }
        });

        tokio::select! {
            res = &mut relay_fut => {
                res?;
            }
            res = &mut router_fut => {
                res?;
            }
        }

        Ok(())
    }

    pub async fn shutdown(&mut self) -> Result<()> {
        info!("Shutting down Coordinator IROH");
        if let Some(handle) = self.http_info_handle.take() {
            handle.abort();
        }
        if let Some(router) = self.iroh_router.take() {
            let _ = router.shutdown().await;
        }
        if let Some(server) = self.relay_server.take() {
            server.shutdown().await.context("failed to cleanly shutdown iroh relay server")?;
        }
        Ok(())
    }
}

async fn register_coordinator_in_registry(
    registry_url: &str,
    node_id_str: &str,
    endpoint_addr: &iroh::EndpointAddr,
    relay_url: Option<String>,
    secret_key_bytes: &[u8; 32],
) -> Result<()> {
    let endpoint_addr_bytes = serde_json::to_vec(endpoint_addr)
        .map_err(|e| anyhow::anyhow!("Failed to serialize endpoint addr: {}", e))?;

    let identity = Identity::from_bytes(secret_key_bytes);
    let did = derive_did_key(&identity.public_key());

    let info = syneroym_core::community_registry::EndpointInfo {
        service_id: did.clone(),
        substrate_id: did.clone(),
        endpoint_type: syneroym_core::community_registry::EndpointType::Substrate,
        nickname: Some(format!("coordinator-{}", &node_id_str[..8])),
        mechanisms: vec![syneroym_core::community_registry::EndpointMechanism::Iroh {
            endpoint_addr_bytes,
            relay_url,
        }],
        is_private: false,
    };

    let signature_z32 = identity.sign_json(&serde_json::to_value(&info)?)?;
    let signed_info =
        syneroym_core::community_registry::SignedEndpointInfo { info, signature: signature_z32 };

    let client = reqwest::Client::new();
    let url = format!("{}/register", registry_url);
    let res = client.post(&url).json(&signed_info).send().await?;

    if !res.status().is_success() {
        return Err(anyhow::anyhow!("Registry registration returned status: {}", res.status()));
    }

    Ok(())
}
