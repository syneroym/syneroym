//! Svc sandbox deployment and lifecycle subcommands
//!
//! Commands to package, deploy, start, list, and terminate sandboxed guest
//! svcs.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use clap::Subcommand;

pub mod deploy;
pub mod lifecycle;
pub mod proxy;
pub mod sagas;

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) use std::fs;

#[cfg(test)]
pub(crate) use deploy::{
    parse_container_port_mapping, parse_container_volume_mapping, parse_interfaces,
    parse_visibility, signed_export_record, validate_container_flags, validate_container_ports,
};
#[cfg(test)]
pub(crate) use lifecycle::format_expiry;
#[cfg(test)]
pub(crate) use syneroym_core::dht_registry::SignedEndpointInfo;
#[cfg(test)]
pub(crate) use syneroym_identity::{Identity, substrate};
#[cfg(test)]
pub(crate) use syneroym_sdk::{Visibility, mapper::DEFAULT_INTERFACE_NAME};

// `Deploy` carries every flag across all three deploy kinds (WASM, TCP,
// container) at once, so it is unavoidably far larger than `Remove`/`Start`/
// `Stop`'s single `svc_id`. This is a one-shot, parsed-once CLI arg struct,
// not a value stored in bulk, so the size difference the perf lint warns
// about has no runtime cost here.
#[allow(clippy::large_enum_variant)]
#[derive(Subcommand, Debug, Clone)]
pub enum SvcCommands {
    /// Deploy a new `SynSvc` via API
    Deploy {
        /// The DID-key for the service
        #[arg(long)]
        svc_id: String,
        /// Comma-separated list of interfaces to register
        #[arg(long)]
        interfaces: String,
        /// Path to the WASM component binary
        #[arg(long, conflicts_with_all = ["tcp", "image"])]
        wasm: Option<PathBuf>,
        /// TCP host:port for an existing service (e.g. "localhost:8080")
        #[arg(long, conflicts_with_all = ["wasm", "image"])]
        tcp: Option<String>,
        /// Container image for a Podman-backed service (e.g.
        /// "docker.io/library/nginx:alpine"). Mutually exclusive with
        /// `--wasm`/`--tcp`; needs at least one `--port` to be reachable.
        #[arg(long, conflicts_with_all = ["wasm", "tcp"])]
        image: Option<String>,
        /// Container port mapping, repeatable:
        /// "interface:container_port[:host_port][:protocol]". `protocol` is
        /// "tcp" (default) or "udp" -- only `tcp` mappings are reachable
        /// through the substrate today, though Podman will still publish a
        /// `udp` one on the host. Each interface name here must also appear
        /// in `--interfaces`. Only meaningful alongside `--image` (checked
        /// at runtime, not by clap -- see `validate_container_flags`).
        #[arg(long = "port")]
        ports: Vec<String>,
        /// Container volume mapping, repeatable: "host_path:container_path".
        /// Docker-style mount options (e.g. a trailing ":ro") are not
        /// supported. In-volume file materialization is not exposed by this
        /// flag -- use a `SynApp` manifest's `files` list instead. Only
        /// meaningful alongside `--image` (see `--port` above).
        #[arg(long = "volume")]
        volumes: Vec<String>,
        /// Optional identity name for signing the published endpoint
        /// record. The self-signed publish route for a service with no
        /// member master -- named identity's own DID must equal `--svc-id`.
        /// With `--master`, that identity signs instead: a member's
        /// endpoint record must be signed by its master key (ADR-0020 §3),
        /// since the hosting substrate never holds it and cannot produce
        /// this signature itself.
        #[arg(long)]
        identity: Option<String>,
        /// Optional nickname for the registry
        #[arg(long)]
        nickname: Option<String>,
        /// Name of a local member master identity (ADR-0020 §1). When
        /// present, `--svc-id` must equal that identity's DID. Signs the
        /// published endpoint record (above); separately, the substrate is
        /// queried for the instance key it would derive and a
        /// `service-instance` certificate is issued and installed for
        /// outbound-call authentication. Absent leaves the service its own
        /// master, exactly as before this flag existed.
        #[arg(long, conflicts_with = "instance_certificate")]
        master: Option<String>,
        /// Path to a JSON `DelegationCertificate` already minted with
        /// `identity certify-instance` -- installed as-is instead of this
        /// command minting a fresh one itself. The one path that lets an
        /// operator pick a non-default `--expires-hours`, or install a
        /// certificate signed on a different machine than this deploy runs
        /// from. Mutually exclusive with `--master`.
        #[arg(long, conflicts_with = "master")]
        instance_certificate: Option<PathBuf>,
        /// Community registry URL to publish/refresh the master's anchor at
        /// when `--master` mints a fresh certificate. Ignored on
        /// the `--instance-certificate` path, since that certificate was
        /// minted (and its anchor published, if at all) elsewhere. Without
        /// it, a certificate minted here is unusable on the wire until an
        /// anchor exists some other way (`roymctl identity publish-anchor`).
        #[arg(long)]
        registry_url: Option<String>,
        /// Path to a gzip-compressed tar archive of static assets, served
        /// straight from blob storage without instantiating the component.
        /// Only meaningful alongside `--wasm`.
        #[arg(long, requires = "wasm")]
        assets: Option<PathBuf>,
        /// Who may fetch `--assets` with no signature or delegation:
        /// "public", "internal", or "private" (default). `internal` and
        /// `private` are identical to no `--assets` at all -- there is no
        /// middle tier.
        #[arg(long, default_value = "private", requires = "assets")]
        asset_visibility: String,
        /// Path to a JSON file used verbatim as the service's
        /// `custom_config` -- the reserved `http_routes` key inside it is
        /// what declares HTTP routes. Only meaningful alongside `--wasm`.
        #[arg(long, requires = "wasm")]
        custom_config: Option<PathBuf>,
        /// Whether this service's endpoint record is published (ADR-0018):
        /// "public" (registered and propagated), "internal" (registered
        /// with this substrate's registry only), or "private" (never
        /// registered; the default when no `--identity`/`--master` is
        /// given). `public`/`internal` require `--identity` or `--master`,
        /// since only the service's own key can sign a record the registry
        /// will admit. When `--identity`/`--master` is given, this flag has
        /// no default and must be stated explicitly -- a deploy that could
        /// sign a record but was not told to must refuse rather than
        /// silently publish nothing.
        #[arg(long)]
        visibility: Option<String>,
        /// Write the signed endpoint record to this path instead of relying
        /// on the registry (ADR-0018 §2). The file is a `SignedEndpointInfo`
        /// -- self-contained and independently verifiable -- to hand to
        /// whoever should be able to reach a `private` service.
        #[arg(long)]
        record_out: Option<PathBuf>,
    },
    /// Remove an installed `SynSvc` via API
    Remove {
        #[arg(long)]
        svc_id: String,
    },
    /// List installed `SynSvcs` via API
    List,
    /// Restart a deployed `SynSvc` in place, without reinstalling it.
    /// Replaces an earlier `start`/`stop` pair, which called orchestrator
    /// methods that never existed.
    Restart {
        #[arg(long)]
        svc_id: String,
    },
    /// Show the calls waiting in a service's durable proxy outbox.
    ///
    /// Prefixed `proxy-` because `supervisor outbox`/`dead-letters`/
    /// `replay` already exist and are keyed by app instance. These three
    /// are per-service and node-local.
    ProxyOutbox {
        #[arg(long)]
        svc_id: String,
    },
    /// Show the queued calls a service gave up on delivering.
    ProxyDeadLetters {
        #[arg(long)]
        svc_id: String,
    },
    /// Re-enqueue one dead letter for another delivery attempt. It is
    /// never executed inline, and the receiver deduplicates it against the
    /// original if that one did land.
    ProxyReplay {
        #[arg(long)]
        svc_id: String,
        #[arg(long)]
        dead_letter_id: u64,
    },
    /// Show the sagas a service's own log holds.
    Sagas {
        #[arg(long)]
        svc_id: String,
    },
    /// Re-arm a `failed` saga back to `compensating`. It never walks
    /// inline; the worker picks it up on its next tick.
    SagaCompensate {
        #[arg(long)]
        svc_id: String,
        #[arg(long)]
        saga_id: String,
    },
}

/// Handle `SynSvc` management subcommands
pub async fn handle(
    command: &SvcCommands,
    api_url: &str,
    substrate_did: String,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> anyhow::Result<()> {
    let mut client = super::client_for(substrate_did.clone(), api_url, dir, run_as, ucan_path)?;
    client.wait_for_ready(Duration::from_secs(5)).await?;

    match command {
        SvcCommands::Deploy {
            svc_id,
            interfaces,
            wasm,
            tcp,
            image,
            ports,
            volumes,
            identity,
            nickname,
            master,
            instance_certificate,
            registry_url,
            assets,
            asset_visibility,
            custom_config,
            visibility,
            record_out,
        } => {
            deploy::handle_deploy(
                &mut client,
                &substrate_did,
                svc_id,
                interfaces,
                wasm,
                tcp,
                image,
                ports,
                volumes,
                identity,
                nickname,
                master,
                instance_certificate,
                registry_url,
                assets,
                asset_visibility,
                custom_config,
                visibility,
                record_out,
                dir,
            )
            .await
        }
        SvcCommands::Remove { svc_id } => lifecycle::handle_remove(&mut client, svc_id).await,
        SvcCommands::List => lifecycle::handle_list(&mut client).await,
        SvcCommands::Restart { svc_id } => lifecycle::handle_restart(&mut client, svc_id).await,
        SvcCommands::ProxyOutbox { svc_id } => proxy::handle_outbox(&mut client, svc_id).await,
        SvcCommands::ProxyDeadLetters { svc_id } => {
            proxy::handle_dead_letters(&mut client, svc_id).await
        }
        SvcCommands::ProxyReplay { svc_id, dead_letter_id } => {
            proxy::handle_replay(&mut client, svc_id, *dead_letter_id).await
        }
        SvcCommands::Sagas { svc_id } => sagas::handle_sagas(&mut client, svc_id).await,
        SvcCommands::SagaCompensate { svc_id, saga_id } => {
            sagas::handle_saga_compensate(&mut client, svc_id, saga_id).await
        }
    }
}
