//! Supervisor role initialization and handle types.

use std::sync::Arc;

use syneroym_core::config::SubstrateConfig;

use super::router::SharedNodeHandles;

/// The literal `native_dispatch` key the supervisor's `NativeService`
/// registers under, independent of this node's own DID: unlike
/// `orchestrator`/`security`, which are the node addressing *itself*,
/// `supervisor` is dispatched to for the *same* connection preamble
/// (`<scheme>://supervisor.<node-did>`) but must not share `native_dispatch`'s
/// entry with `ControlPlaneService`, which is already registered under the
/// node's own DID by `RouteHandler::init`. Sourced from
/// `syneroym_control_plane`, which is also the crate that refuses a deploy
/// under this name, so the reserved word cannot drift between the two.
pub(super) const SUPERVISOR_DISPATCH_ID: &str =
    syneroym_control_plane::SUPERVISOR_RESERVED_SERVICE_ID;

#[cfg(feature = "supervisor")]
pub(super) type SupervisorHandle = syneroym_app_supervisor::SupervisorService;
#[cfg(not(feature = "supervisor"))]
pub(super) type SupervisorHandle = ();

#[cfg(feature = "supervisor")]
pub(super) async fn init_supervisor(
    config: &SubstrateConfig,
    service_id: &str,
    shared: &SharedNodeHandles,
) -> anyhow::Result<Arc<SupervisorHandle>> {
    use syneroym_app_supervisor::{
        MasterVault, RegistryAnchorWriter, RegistryTier1Writer, SupervisorService,
        store::SupervisorStore,
    };
    use syneroym_core::dht_registry::RegistryClient;
    use syneroym_rpc::NativeService;
    use tracing::warn;

    let role = config
        .roles
        .supervisor
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("init_supervisor called with no [roles.supervisor]"))?;

    std::fs::create_dir_all(&config.app_data_dir)?;
    let store = SupervisorStore::open_with_role(&config.app_data_dir, &role.db_name, role)?;
    let backup_dir = config.app_data_dir.join(&role.master_backup_dir);
    let vault = MasterVault::new(
        shared.storage_provider.clone(),
        shared.key_store.clone(),
        SUPERVISOR_DISPATCH_ID.to_string(),
        backup_dir,
    );
    // The node's own registry, the same one every other publisher on this
    // host uses. Built once and shared (by `Arc` clone) between the anchor
    // and Tier-1 writers below, rather than each building its own -- with
    // the DHT enabled that would otherwise be a second pkarr client and
    // socket per supervisor. `None` when no registry is configured, which
    // both writers' own docs explain the consequence of.
    let registry_client: Option<Arc<RegistryClient>> =
        config.substrate.registry_url.as_deref().map(|url| {
            Arc::new(RegistryClient::new(
                config.substrate.enable_bep0044_dht,
                Some(url.to_string()),
            ))
        });
    let supervisor = Arc::new(SupervisorService::new(
        service_id.to_string(),
        store,
        vault,
        &shared.client_identity,
        config.substrate.enable_bep0044_dht,
        shared.messaging_broker.clone(),
        role.alert_topic.clone(),
        role.poll_interval_secs,
        role.max_restart_attempts,
        role.restart_backoff_secs,
        role.renewed_cert_expires_hours,
        role.max_renewals_per_pass,
        role.master_anchor_refresh_interval_secs,
        RegistryAnchorWriter::from_registry_client(registry_client.clone()),
        RegistryTier1Writer::from_registry_client(registry_client),
        role.queue_tick_secs,
        role.topology_document_not_after_secs,
        role.topology_document_cache_ttl_secs,
    ));
    shared
        .native_dispatch
        .insert(SUPERVISOR_DISPATCH_ID.to_string(), supervisor.clone() as Arc<dyn NativeService>);

    if !shared.key_store.kek_is_loaded() {
        warn!(
            "supervisor role is enabled but its vault is LOCKED: no KEK has been injected, so it              cannot mint, certify, or renew member masters. Inject one with: roymctl --substrate              {service_id} security inject-kek --kek-hex <...>"
        );
    }
    if config.substrate.registry_url.is_none() {
        warn!(
            "supervisor role is enabled but this node has no substrate.registry_url configured:              the Tier-1 registry record for every app instance this supervisor manages cannot be              published, so callers outside those apps will not be able to discover them              (ADR-0022). Intra-app service discovery is unaffected."
        );
    }

    Ok(supervisor)
}

#[cfg(not(feature = "supervisor"))]
pub(super) async fn init_supervisor(
    _config: &SubstrateConfig,
    _service_id: &str,
    _shared: &SharedNodeHandles,
) -> anyhow::Result<Arc<SupervisorHandle>> {
    Err(anyhow::anyhow!(
        "[roles.supervisor] is configured but this binary was built without the `supervisor`          feature"
    ))
}
