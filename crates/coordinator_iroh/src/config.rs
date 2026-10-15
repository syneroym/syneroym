//! Configuration for the Iroh Transport Coordinator
//!
//! Structs and validation for signaling/relay address bindings and keypaths.

use std::{io, net::SocketAddr, path::Path, sync::Arc};

use anyhow::{Context, Result, bail, ensure};
use iroh_base::EndpointId;
use iroh_relay::server::{self as relay, QuicConfig, ServerConfig};
use relay::{Access, AccessConfig, CertConfig, RelayConfig, TlsConfig};
use rustls::{ServerConfig as RustlsServerConfig, crypto::ring};
use rustls_pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};
use syneroym_core::config::{CoordinatorAccess, CoordinatorIrohConfig, CoordinatorRole};
use syneroym_passport::LeaseStore;
use tokio::task;
use tracing::warn;

use crate::passport;

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

/// Checks `[roles.coordinator] access` at startup.
///
/// # Errors
/// For a list entry that is not an Iroh endpoint id, and for a passport
/// form without `[tls]` (`CoordinatorAccess::check_tls`).
pub fn check_access(access: &CoordinatorAccess, has_tls: bool, relay_on: bool) -> Result<()> {
    let ids = parse_endpoint_ids(access.list())?;
    if ids.is_empty() && matches!(access, CoordinatorAccess::List(_)) {
        warn!(
            "[roles.coordinator] access is an empty list; only the coordinator's own endpoint and \
             the host node may use this relay"
        );
    }
    access.check_tls(has_tls)?;
    if access.accepts_passports() && !relay_on {
        warn!(
            "[roles.coordinator] access accepts passports, but enable_relay is false; it has no \
             effect"
        );
    }
    Ok(())
}

fn parse_endpoint_ids(ids: &[String]) -> Result<Vec<EndpointId>> {
    ids.iter()
        .map(|s| {
            s.parse::<EndpointId>().with_context(|| {
                format!("[roles.coordinator] access: {s:?} is not an Iroh endpoint id")
            })
        })
        .collect()
}

/// Builds the relay access check from `[roles.coordinator] access`.
/// `always`: endpoints every restricted form admits (the coordinator's own
/// endpoint and the host node). `leases`: the lease store; it must be
/// `Some` exactly when `access` accepts passports.
///
/// # Errors
/// For a list entry that is not an Iroh endpoint id, or a lease store that
/// does not match `access`.
pub fn relay_access(
    access: &CoordinatorAccess,
    always: &[EndpointId],
    leases: Option<LeaseStore>,
) -> Result<AccessConfig> {
    if *access == CoordinatorAccess::Everyone {
        return Ok(AccessConfig::Everyone);
    }
    ensure!(
        access.accepts_passports() == leases.is_some(),
        "the relay lease store does not match [roles.coordinator] access"
    );
    let mut admitted = parse_endpoint_ids(access.list())?;
    admitted.extend_from_slice(always);
    let admitted = Arc::new(admitted);
    Ok(AccessConfig::Restricted(Box::new(move |id| {
        let allow =
            admitted.contains(&id) || leases.as_ref().is_some_and(|l| passport::is_leased(l, &id));
        Box::pin(async move { if allow { Access::Allow } else { Access::Deny } })
    })))
}

/// Checks `[roles.coordinator.iroh] max_relay_clients`.
///
/// # Errors
/// For `0`, which would always report `at_capacity`.
pub fn check_relay_cap(iroh_cfg: &CoordinatorIrohConfig, has_parent_relay: bool) -> Result<()> {
    match iroh_cfg.max_relay_clients {
        Some(0) => bail!("[roles.coordinator.iroh] max_relay_clients must be at least 1"),
        Some(1) if iroh_cfg.enable_relay && !has_parent_relay => {
            warn!(
                "[roles.coordinator.iroh] max_relay_clients is 1 and there is no parent relay; \
                 the coordinator's own endpoint occupies this slot, so the relay will always \
                 report at_capacity"
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

pub async fn build_relay_config(
    role: &CoordinatorRole,
    always: &[EndpointId],
    leases: Option<LeaseStore>,
) -> Result<ServerConfig<io::Error>> {
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

    let relay_config = if iroh_cfg.enable_relay {
        let access_config = relay_access(&role.access, always, leases)?;
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
    use syneroym_passport::RELAY_HOME;

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

        let config = build_relay_config(&role, &[], None).await?;
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

        let config = build_relay_config(&role, &[], None).await?;
        assert!(config.relay.is_none());
        Ok(())
    }

    #[test]
    fn everyone_admits_any_endpoint() -> Result<()> {
        let access = relay_access(&CoordinatorAccess::Everyone, &[], None)?;
        assert!(matches!(access, AccessConfig::Everyone));
        Ok(())
    }

    #[test]
    fn everyone_ignores_always_list() -> Result<()> {
        let key = SecretKey::generate(&mut rand::rng());
        let access = relay_access(&CoordinatorAccess::Everyone, &[key.public()], None)?;
        assert!(matches!(access, AccessConfig::Everyone));
        Ok(())
    }

    #[tokio::test]
    async fn list_admits_listed_and_always_admitted_endpoints() -> Result<()> {
        let key1 = SecretKey::generate(&mut rand::rng());
        let key2 = SecretKey::generate(&mut rand::rng());
        let always_key = SecretKey::generate(&mut rand::rng());
        let id1 = key1.public();
        let id2 = key2.public();
        let always_id = always_key.public();

        let access =
            relay_access(&CoordinatorAccess::List(vec![id1.to_string()]), &[always_id], None)?;
        let AccessConfig::Restricted(check) = access else {
            panic!("expected Restricted access config");
        };

        assert_eq!(check(id1).await, Access::Allow);
        assert_eq!(check(always_id).await, Access::Allow);
        assert_eq!(check(id2).await, Access::Deny);
        Ok(())
    }

    #[tokio::test]
    async fn passport_access_admits_leased_and_always_admitted_endpoints() -> Result<()> {
        let store = LeaseStore::open_in_memory(&RELAY_HOME)?;
        let always_key = SecretKey::generate(&mut rand::rng());
        let always_id = always_key.public();
        let random_key = SecretKey::generate(&mut rand::rng());
        let random_id = random_key.public();

        let access = relay_access(&CoordinatorAccess::Passport, &[always_id], Some(store))?;
        let AccessConfig::Restricted(check) = access else {
            panic!("expected Restricted access config");
        };

        assert_eq!(check(always_id).await, Access::Allow);
        assert_eq!(check(random_id).await, Access::Deny);
        Ok(())
    }

    #[tokio::test]
    async fn table_form_admits_listed_or_leased() -> Result<()> {
        let store = LeaseStore::open_in_memory(&RELAY_HOME)?;
        let listed_key = SecretKey::generate(&mut rand::rng());
        let listed_id = listed_key.public();
        let random_key = SecretKey::generate(&mut rand::rng());
        let random_id = random_key.public();

        let access = relay_access(
            &CoordinatorAccess::ListOrPassport(vec![listed_id.to_string()]),
            &[],
            Some(store),
        )?;
        let AccessConfig::Restricted(check) = access else {
            panic!("expected Restricted access config");
        };

        assert_eq!(check(listed_id).await, Access::Allow);
        assert_eq!(check(random_id).await, Access::Deny);
        Ok(())
    }

    #[test]
    fn check_access_rejects_bad_ids() {
        let err = check_access(&CoordinatorAccess::List(vec!["not an id".into()]), false, true)
            .unwrap_err()
            .to_string();
        assert!(err.contains("not an Iroh endpoint id"), "expected error: {err}");

        let err2 = relay_access(&CoordinatorAccess::List(vec!["not an id".into()]), &[], None)
            .unwrap_err()
            .to_string();
        assert!(err2.contains("not an Iroh endpoint id"), "expected error: {err2}");
    }

    #[test]
    fn lease_store_must_match_access() {
        let store = LeaseStore::open_in_memory(&RELAY_HOME).unwrap();
        assert!(relay_access(&CoordinatorAccess::Passport, &[], None).is_err());
        assert!(relay_access(&CoordinatorAccess::List(vec![]), &[], Some(store.clone())).is_err());
        assert!(relay_access(&CoordinatorAccess::ListOrPassport(vec![]), &[], None).is_err());
    }

    #[test]
    fn check_access_requires_tls_for_passports() {
        assert!(check_access(&CoordinatorAccess::Passport, false, true).is_err());
        assert!(check_access(&CoordinatorAccess::Passport, true, true).is_ok());
        assert!(check_access(&CoordinatorAccess::ListOrPassport(vec![]), false, true).is_err());
        assert!(check_access(&CoordinatorAccess::ListOrPassport(vec![]), true, true).is_ok());
        assert!(check_access(&CoordinatorAccess::Everyone, false, true).is_ok());
        assert!(check_access(&CoordinatorAccess::List(vec![]), false, true).is_ok());
    }

    #[test]
    fn cap_of_one_warns_for_every_access_form() {
        let cfg = CoordinatorIrohConfig {
            enable_relay: true,
            max_relay_clients: Some(1),
            ..Default::default()
        };
        assert!(check_relay_cap(&cfg, false).is_ok());
        assert!(check_relay_cap(&cfg, true).is_ok());

        let zero_cfg = CoordinatorIrohConfig {
            enable_relay: true,
            max_relay_clients: Some(0),
            ..Default::default()
        };
        assert!(check_relay_cap(&zero_cfg, false).is_err());
    }
}
