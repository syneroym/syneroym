//! Configuration for the Iroh Transport Coordinator
//!
//! Structs and validation for signaling/relay address bindings and keypaths.

use std::{io, net::SocketAddr, path::Path, sync::Arc};

use anyhow::{Context, Result, bail};
use iroh_base::EndpointId;
use iroh_relay::server::{self as relay, QuicConfig, ServerConfig};
use relay::{Access, AccessConfig, CertConfig, RelayConfig, TlsConfig};
use rustls::{ServerConfig as RustlsServerConfig, crypto::ring};
use rustls_pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};
use syneroym_core::config::{CoordinatorAccess, CoordinatorIrohConfig, CoordinatorRole};
use tokio::task;
use tracing::warn;

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

/// Builds the relay access check from `[roles.coordinator] access`.
///
/// # Errors
/// For the passport forms, which this build cannot enforce yet, and for a
/// list entry that is not an Iroh endpoint id.
pub fn relay_access(access: &CoordinatorAccess) -> Result<AccessConfig> {
    match access {
        CoordinatorAccess::Everyone => Ok(AccessConfig::Everyone),
        CoordinatorAccess::List(ids) => restricted_to(ids),
        CoordinatorAccess::Passport => bail!(
            "[roles.coordinator] access = \"passport\" is not supported yet; use \"everyone\" or \
             a list of endpoint ids"
        ),
        CoordinatorAccess::ListOrPassport(_) => bail!(
            "[roles.coordinator] access = {{ list = [...], passport = true }} is not supported \
             yet; use the plain list form access = [...]"
        ),
    }
}

/// Checks `[roles.coordinator.iroh] max_relay_clients`.
///
/// # Errors
/// For `0`, which would always report `at_capacity`.
pub fn check_relay_cap(
    iroh_cfg: &CoordinatorIrohConfig,
    access: &CoordinatorAccess,
    has_parent_relay: bool,
) -> Result<()> {
    match iroh_cfg.max_relay_clients {
        Some(0) => bail!("[roles.coordinator.iroh] max_relay_clients must be at least 1"),
        Some(1)
            if iroh_cfg.enable_relay
                && *access == CoordinatorAccess::Everyone
                && !has_parent_relay =>
        {
            warn!(
                "[roles.coordinator.iroh] max_relay_clients is 1 with access = \"everyone\" and \
                 no parent relay; the coordinator's own endpoint occupies this slot, so the relay \
                 will always report at_capacity"
            );
        }
        Some(_) if !iroh_cfg.enable_relay => warn!(
            "[roles.coordinator.iroh] max_relay_clients is set, but enable_relay is false; it has \
             no effect"
        ),
        _ => {}
    }
    Ok(())
}

fn restricted_to(ids: &[String]) -> Result<AccessConfig> {
    if ids.is_empty() {
        warn!(
            "[roles.coordinator] access is configured with an empty list; all endpoints will be \
             denied access to this relay"
        );
    }
    let list = ids
        .iter()
        .map(|s| {
            s.parse::<EndpointId>().with_context(|| {
                format!("[roles.coordinator] access: {s:?} is not an Iroh endpoint id")
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let list = Arc::new(list);
    Ok(AccessConfig::Restricted(Box::new(move |endpoint_id| {
        let list = list.clone();
        Box::pin(
            async move { if list.contains(&endpoint_id) { Access::Allow } else { Access::Deny } },
        )
    })))
}

pub async fn build_relay_config(role: &CoordinatorRole) -> Result<ServerConfig<io::Error>> {
    let iroh_cfg = role.iroh.clone().unwrap_or_default();

    let http_bind_addr: SocketAddr =
        iroh_cfg.http_bind_address.parse().context("invalid iroh http_bind_address")?;

    let quic_bind_addr: SocketAddr =
        iroh_cfg.quic_bind_address.parse().context("invalid iroh quic_bind_address")?;

    let relay_tls = if let Some(tls) = &role.tls {
        let cert_path = tls.cert_path.clone();
        let key_path = tls.key_path.clone();

        let (private_key, certs) = task::spawn_blocking(move || {
            let key = load_secret_key(&key_path)?;
            let certs = load_certs(&cert_path)?;
            Ok::<_, anyhow::Error>((key, certs))
        })
        .await
        .context("join blocking cert load")??;

        let server_config =
            RustlsServerConfig::builder_with_provider(Arc::new(ring::default_provider()))
                .with_safe_default_protocol_versions()
                .map_err(|e| anyhow::anyhow!("protocols supported by ring: {e}"))?
                .with_no_client_auth()
                .with_single_cert(certs.clone(), private_key)
                .context("failed to create rustls ServerConfig")?;

        Some(TlsConfig {
            https_bind_addr: http_bind_addr,
            cert: CertConfig::Manual { certs },
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

    let access_config = relay_access(&role.access)?;

    let relay_config = if iroh_cfg.enable_relay {
        Some(RelayConfig {
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
    use iroh::SecretKey;
    use syneroym_core::config::CoordinatorIrohConfig;

    use super::*;

    #[tokio::test]
    async fn test_build_relay_config_enable_relay() -> Result<()> {
        let role = CoordinatorRole {
            iroh: Some(CoordinatorIrohConfig {
                enable_relay: true,
                http_bind_address: "127.0.0.1:8080".to_string(),
                quic_bind_address: "127.0.0.1:8081".to_string(),
                enable_signalling: false,
                community_registry_url: None,
                idle_timeout_secs: None,
                share_in_registry: false,
                max_connections: None,
                ..Default::default()
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
                community_registry_url: None,
                idle_timeout_secs: None,
                share_in_registry: false,
                max_connections: None,
                ..Default::default()
            }),
            ..Default::default()
        };

        let config = build_relay_config(&role).await?;
        assert!(config.relay.is_none());
        Ok(())
    }

    #[test]
    fn everyone_admits_any_endpoint() -> Result<()> {
        let access = relay_access(&CoordinatorAccess::Everyone)?;
        assert!(matches!(access, AccessConfig::Everyone));
        Ok(())
    }

    #[tokio::test]
    async fn list_admits_only_listed_endpoints() -> Result<()> {
        let key1 = SecretKey::generate(&mut rand::rng());
        let key2 = SecretKey::generate(&mut rand::rng());
        let id1 = key1.public();
        let id2 = key2.public();

        let access = relay_access(&CoordinatorAccess::List(vec![id1.to_string()]))?;
        let AccessConfig::Restricted(check) = access else {
            panic!("expected Restricted access config");
        };

        assert_eq!(check(id1).await, Access::Allow);
        assert_eq!(check(id2).await, Access::Deny);
        Ok(())
    }

    #[test]
    fn list_with_bad_id_is_an_error() {
        let err = relay_access(&CoordinatorAccess::List(vec!["not an id".into()]))
            .unwrap_err()
            .to_string();
        assert!(err.contains("not an Iroh endpoint id"), "expected error: {err}");
    }

    #[test]
    fn passport_access_is_an_error() {
        let err = relay_access(&CoordinatorAccess::Passport).unwrap_err().to_string();
        assert!(err.contains("not supported yet"), "expected error: {err}");
    }

    #[test]
    fn table_access_is_an_error() {
        let err = relay_access(&CoordinatorAccess::ListOrPassport(vec![])).unwrap_err().to_string();
        assert!(err.contains("not supported yet"), "expected error: {err}");
    }

    #[tokio::test]
    async fn build_relay_config_rejects_passport_access() {
        let role = CoordinatorRole {
            access: CoordinatorAccess::Passport,
            iroh: Some(CoordinatorIrohConfig { enable_relay: true, ..Default::default() }),
            ..Default::default()
        };
        let res = build_relay_config(&role).await;
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("not supported yet"));
    }
}
