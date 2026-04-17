use anyhow::{Context, Result};
use iroh_relay::server::{self as relay, QuicConfig, ServerConfig};
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use syneroym_core::config::{AccessControl, CoordinatorRole};

fn load_certs(filename: impl AsRef<Path>) -> Result<Vec<CertificateDer<'static>>> {
    let certs: Vec<_> = CertificateDer::pem_file_iter(filename.as_ref())
        .with_context(|| {
            format!("failed to open certificate file at {}", filename.as_ref().display())
        })?
        .collect::<Result<Vec<_>, _>>()
        .context("failed to parse certificate")?;
    Ok(certs)
}

fn load_secret_key(filename: impl AsRef<Path>) -> Result<PrivateKeyDer<'static>> {
    let key = PrivateKeyDer::from_pem_file(filename.as_ref()).with_context(|| {
        format!("failed to read or parse private key file at {}", filename.as_ref().display())
    })?;
    Ok(key)
}

pub(crate) async fn build_relay_config(
    role: &CoordinatorRole,
) -> Result<ServerConfig<std::io::Error>> {
    let iroh_cfg = role.iroh.clone().unwrap_or_default();

    let http_bind_addr: SocketAddr =
        iroh_cfg.http_bind_address.parse().context("invalid iroh http_bind_address")?;

    let quic_bind_addr: SocketAddr =
        iroh_cfg.quic_bind_address.parse().context("invalid iroh quic_bind_address")?;

    let relay_tls = if let Some(tls) = &role.tls {
        let cert_path = tls.cert_path.clone();
        let key_path = tls.key_path.clone();

        let (private_key, certs) = tokio::task::spawn_blocking(move || {
            let key = load_secret_key(&key_path)?;
            let certs = load_certs(&cert_path)?;
            Ok::<_, anyhow::Error>((key, certs))
        })
        .await
        .context("join blocking cert load")??;

        let server_config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(|e| anyhow::anyhow!("protocols supported by ring: {}", e))?
        .with_no_client_auth()
        .with_single_cert(certs.clone(), private_key)
        .context("failed to create rustls ServerConfig")?;

        Some(relay::TlsConfig {
            https_bind_addr: http_bind_addr,
            cert: relay::CertConfig::Manual { certs },
            server_config,
            quic_bind_addr,
        })
    } else {
        None
    };

    let mut quic_config = None;
    if relay_tls.is_some()
        && let Some(ref tls) = relay_tls
    {
        quic_config = Some(QuicConfig {
            server_config: tls.server_config.clone(),
            bind_addr: tls.quic_bind_addr,
        });
    }

    let access_config = match &role.access {
        AccessControl::String(s) if s == "everyone" => relay::AccessConfig::Everyone,
        AccessControl::List(l) => {
            let mut list = Vec::new();
            for s in l {
                let id: iroh_base::EndpointId =
                    s.parse().context("invalid endpoint ID in access list")?;
                list.push(id);
            }
            let list = Arc::new(list);
            relay::AccessConfig::Restricted(Box::new(move |endpoint_id| {
                let list = list.clone();
                Box::pin(async move {
                    if list.contains(&endpoint_id) {
                        relay::Access::Allow
                    } else {
                        relay::Access::Deny
                    }
                })
            }))
        }
        _ => relay::AccessConfig::Everyone,
    };

    let relay_config = if iroh_cfg.enable_relay {
        Some(relay::RelayConfig {
            http_bind_addr,
            tls: relay_tls,
            limits: Default::default(),
            key_cache_capacity: None,
            access: access_config,
        })
    } else {
        None
    };

    Ok(ServerConfig { relay: relay_config, quic: quic_config, metrics_addr: None })
}

#[cfg(test)]
mod tests {
    use super::*;
    use syneroym_core::config::CoordinatorIrohConfig;

    #[tokio::test]
    async fn test_build_relay_config_enable_relay() -> Result<()> {
        let role = CoordinatorRole {
            iroh: Some(CoordinatorIrohConfig {
                enable_relay: true,
                http_bind_address: "127.0.0.1:8080".to_string(),
                quic_bind_address: "127.0.0.1:8081".to_string(),
                enable_signalling: false,
            }),
            ..Default::default()
        };

        let config = build_relay_config(&role).await?;
        assert!(config.relay.is_some());

        let relay_cfg = config.relay.unwrap();
        assert_eq!(relay_cfg.http_bind_addr.to_string(), "127.0.0.1:8080");
        Ok(())
    }

    #[tokio::test]
    async fn test_build_relay_config_disable_relay() -> Result<()> {
        let role = CoordinatorRole {
            iroh: Some(CoordinatorIrohConfig {
                enable_relay: false,
                http_bind_address: "127.0.0.1:8080".to_string(),
                quic_bind_address: "127.0.0.1:8081".to_string(),
                enable_signalling: false,
            }),
            ..Default::default()
        };

        let config = build_relay_config(&role).await?;
        assert!(config.relay.is_none());
        Ok(())
    }
}
