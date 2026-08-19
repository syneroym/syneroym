//! Svc sandbox deployment and lifecycle subcommands
//!
//! Commands to package, deploy, start, list, and terminate sandboxed guest
//! svcs.

use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use chrono::DateTime;
use clap::Subcommand;
use syneroym_core::dht_registry::{DEFAULT_ENDPOINT_NOT_AFTER_SECS, EndpointInfo, EndpointType};
use syneroym_identity::{DelegationCertificate, Identity, substrate};
use syneroym_sdk::{
    ArtifactSource, AssetBundle, ContainerPortMapping, ContainerVolumeMapping, DeploySvcOptions,
    NetworkEndpoint, Publication, Visibility, deploy, mapper::DEFAULT_INTERFACE_NAME,
};

use super::member_identity;

/// The attended posture's default certificate lifetime for a deploy-time
/// certification via `svc deploy --master` -- `identity certify-instance`
/// is the dedicated renewal command for a longer- or shorter-lived one.
const DEFAULT_INSTANCE_CERT_EXPIRES_HOURS: u64 = 24;

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
        /// Path to a gzip-compressed tar archive of static assets (M06A
        /// A1), served straight from blob storage without instantiating
        /// the component. Only meaningful alongside `--wasm`.
        #[arg(long, requires = "wasm")]
        assets: Option<PathBuf>,
        /// Who may fetch `--assets` with no signature or delegation:
        /// "public", "internal", or "private" (default). `internal` and
        /// `private` are identical to no `--assets` at all -- A1 has no
        /// middle tier.
        #[arg(long, default_value = "private", requires = "assets")]
        asset_visibility: String,
        /// Path to a JSON file used verbatim as the service's
        /// `custom_config` -- the reserved `http_routes` key inside it is
        /// what declares HTTP routes (M3B Slice 7, M06A A2). Only
        /// meaningful alongside `--wasm`.
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
    /// Restart a deployed `SynSvc` in place, without reinstalling it (M05A
    /// A5a). Replaces the pre-A5a `start`/`stop` pair, which called
    /// orchestrator methods that never existed.
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
            validate_container_flags(image, ports, volumes)?;
            let ifaces: Vec<String> = parse_interfaces(interfaces)?;
            let stated_visibility =
                visibility.as_deref().map(|v| parse_visibility(v, "--visibility")).transpose()?;

            // The record the substrate publishes and replays verbatim: it
            // holds no key of its own that could ever produce this
            // signature for a `--master` deploy (ADR-0020 §3), so this is
            // the *only* place a member's endpoint record is ever signed,
            // and it must be signed on every `--master` deploy, not only
            // when a nickname is given -- unlike an earlier design, where the
            // substrate re-signed with a delegated instance key and this
            // blob's own signature was never trusted.
            //
            // Bound owned, chosen by reference: `Identity` is not `Clone`,
            // and the `--master` arm below needs the same key again.
            let named_identity = match identity {
                Some(name) => {
                    let id = load_identity(dir, name)?;
                    let did = substrate::derive_did_key(&id.public_key());
                    if did != *svc_id {
                        anyhow::bail!(
                            "--identity resolves to {did}, which is not --svc-id {svc_id}; the \
                             registry resolves a record's signing key from its own service_id, so \
                             this record could never be admitted"
                        );
                    }
                    Some(id)
                }
                None => None,
            };
            let master_identity = match master {
                Some(name) => Some(member_identity::resolve_member_master(dir, name)?),
                None => None,
            };

            // `--identity` wins if both are somehow given (clap does not
            // forbid it, since neither conflicts with the other); otherwise
            // `--master` is the record's signer. Neither present means
            // `--instance-certificate` alone: there is no local key that
            // could sign a record which would verify under `svc_id`, so
            // deploy proceeds without one.
            let signing_identity: Option<&Identity> =
                named_identity.as_ref().or(master_identity.as_ref());

            // ADR-0018 §5: a deploy that *can* sign a record but was not
            // told whether to publish it must fail loudly, not fall back to
            // `private` and succeed having published nothing. Silence is
            // only safe when there is no signing identity to publish with
            // in the first place.
            if signing_identity.is_some() && stated_visibility.is_none() {
                anyhow::bail!(
                    "--identity/--master can sign a published endpoint record, so --visibility \
                     must be given explicitly (\"public\", \"internal\", or \"private\") -- it no \
                     longer defaults silently to private"
                );
            }
            let parsed_visibility = stated_visibility.unwrap_or(Visibility::Private);

            // Nothing to sign with, but a nickname was given: it is silently
            // dropped (unchanged from before this flag existed). Warn rather
            // than fail: the deploy itself still succeeds, and a
            // silently-lost nickname is confusing to debug.
            if nickname.is_some() && signing_identity.is_none() {
                eprintln!(
                    "Warning: --nickname has no effect without --identity or --master -- it will \
                     not be published."
                );
            }

            let not_after = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
                .saturating_add(DEFAULT_ENDPOINT_NOT_AFTER_SECS);

            let publication = match (parsed_visibility, signing_identity) {
                (Visibility::Private, Some(id)) if record_out.is_some() => {
                    let record = EndpointInfo {
                        service_id: svc_id.clone(),
                        substrate_id: substrate_did.clone(),
                        endpoint_type: EndpointType::Service,
                        mechanisms: vec![],
                        nickname: nickname.clone(),
                        // A record exported for a `private` service must
                        // never be admitted by a registry: `is_private` is
                        // signed into the payload, so this is the only
                        // chance to say so. `RegistryClient::register`'s
                        // DHT gate (D-B2-16) trusts this flag verbatim.
                        is_private: true,
                        ttl: None,
                        not_after,
                        generation: 0,
                    }
                    .sign(id)?;
                    if let Some(path) = record_out {
                        fs::write(path, serde_json::to_string_pretty(&record)?).map_err(|e| {
                            anyhow::anyhow!(
                                "failed to write --record-out at {}: {e}",
                                path.display()
                            )
                        })?;
                        println!("Wrote the signed endpoint record to {}", path.display());
                    }
                    Publication::Private
                }
                (Visibility::Private, _) => {
                    if record_out.is_some() {
                        eprintln!(
                            "Warning: --record-out has no effect without --identity or --master"
                        );
                    }
                    Publication::Private
                }
                (v, None) => {
                    let v_str = v.as_str();
                    anyhow::bail!(
                        "--visibility '{v_str}' needs --identity or --master: only the service's \
                         own key can sign a record the registry will admit"
                    );
                }
                (v, Some(id)) => {
                    let record = EndpointInfo {
                        service_id: svc_id.clone(),
                        substrate_id: substrate_did.clone(),
                        endpoint_type: EndpointType::Service,
                        mechanisms: vec![],
                        nickname: nickname.clone(),
                        is_private: v == Visibility::Internal,
                        ttl: None,
                        not_after,
                        generation: 0,
                    }
                    .sign(id)?;
                    if let Some(path) = record_out {
                        fs::write(path, serde_json::to_string_pretty(&record)?).map_err(|e| {
                            anyhow::anyhow!(
                                "failed to write --record-out at {}: {e}",
                                path.display()
                            )
                        })?;
                        println!("Wrote the signed endpoint record to {}", path.display());
                    }
                    if v == Visibility::Public {
                        Publication::Public(record)
                    } else {
                        Publication::Internal(record)
                    }
                }
            };

            let instance_cert = match (&master_identity, instance_certificate) {
                (Some(resolved_master), _) => {
                    let master_did = substrate::derive_did_key(&resolved_master.public_key());
                    if master_did != *svc_id {
                        anyhow::bail!(
                            "--master '{}' resolves to {master_did}, which does not match \
                             --svc-id {svc_id} -- an install-time certificate for this pair would \
                             be rejected",
                            master.as_deref().unwrap_or("?")
                        );
                    }
                    let cert = deploy::certify_instance(
                        &client,
                        resolved_master,
                        svc_id,
                        DEFAULT_INSTANCE_CERT_EXPIRES_HOURS,
                    )
                    .await?;
                    member_identity::refresh_anchor_or_warn(
                        registry_url.as_deref(),
                        resolved_master,
                    )
                    .await?;
                    Some(cert)
                }
                (None, Some(path)) => {
                    let cert_json = fs::read_to_string(path).map_err(|e| {
                        anyhow::anyhow!(
                            "failed to read --instance-certificate at {}: {e}",
                            path.display()
                        )
                    })?;
                    Some(DelegationCertificate::from_json(&cert_json)?)
                }
                (None, None) => None,
            };

            if let Some(wasm_path) = wasm {
                let wasm_bytes = fs::read(wasm_path)?;
                let asset_bundle = match assets {
                    Some(assets_path) => {
                        let archive = fs::read(assets_path).map_err(|e| {
                            anyhow::anyhow!(
                                "failed to read --assets at {}: {e}",
                                assets_path.display()
                            )
                        })?;
                        Some(AssetBundle {
                            archive: ArtifactSource::Binary(archive),
                            hash: None,
                            visibility: Some(parse_visibility(
                                asset_visibility,
                                "--asset-visibility",
                            )?),
                        })
                    }
                    None => None,
                };
                let custom_config_json = match custom_config {
                    Some(path) => Some(fs::read_to_string(path).map_err(|e| {
                        anyhow::anyhow!("failed to read --custom-config at {}: {e}", path.display())
                    })?),
                    None => None,
                };
                client
                    .deploy_svc_wasm_with_options(
                        svc_id.clone(),
                        ifaces,
                        wasm_bytes,
                        DeploySvcOptions {
                            publication,
                            instance_certificate: instance_cert,
                            assets: asset_bundle,
                            custom_config: custom_config_json,
                        },
                    )
                    .await?;
                println!("Successfully deployed WASM svc {svc_id}");
            } else if let Some(tcp_addr) = tcp {
                let (host, port) = get_host_port_from_tcp_addr(tcp_addr)?;
                // One `NetworkEndpoint` per declared interface, all naming
                // the same backend: a TCP passthrough has nothing to
                // dispatch on, so every declared interface is just another
                // registered name for the identical `(host, port)`.
                let endpoints = ifaces
                    .into_iter()
                    .map(|interface_name| NetworkEndpoint {
                        interface_name,
                        host: host.clone(),
                        port,
                    })
                    .collect();
                client
                    .deploy_svc_tcp(svc_id.clone(), endpoints, publication, instance_cert)
                    .await?;
                println!("Successfully deployed TCP service {svc_id}");
            } else if let Some(image) = image {
                let port_mappings = ports
                    .iter()
                    .map(|p| parse_container_port_mapping(p))
                    .collect::<anyhow::Result<Vec<_>>>()?;
                validate_container_ports(&ifaces, &port_mappings)?;
                let volume_mappings = volumes
                    .iter()
                    .map(|v| parse_container_volume_mapping(v))
                    .collect::<anyhow::Result<Vec<_>>>()?;
                client
                    .deploy_container(
                        svc_id.clone(),
                        image.clone(),
                        port_mappings,
                        volume_mappings,
                        publication,
                        instance_cert,
                    )
                    .await?;
                println!("Successfully deployed container service {svc_id}");
            } else {
                anyhow::bail!("Either --wasm, --tcp, or --image must be provided for deployment");
            }
        }
        SvcCommands::Remove { svc_id } => {
            // Unmanaged (M05A A5a): an operator-driven `svc remove` always
            // presents generation 0, the same convention `svc deploy` uses.
            client.undeploy(svc_id.clone(), 0).await?;
            println!("Successfully removed svc {svc_id}");
        }
        SvcCommands::List => {
            // Lists all installed SynSvcs registered in the local substrate registry.
            let services = client.list_svcs().await?;
            println!(
                "{:<50} {:<10} {:<12} {:<30} {:<50}",
                "SERVICE ID", "TYPE", "VISIBILITY", "INSTANCE CERT EXPIRES", "INTERFACES"
            );
            println!("{:-<158}", "");
            for svc in services {
                let vis_str = svc.visibility.map_or("-", Visibility::as_str);
                println!(
                    "{:<50} {:<10} {:<12} {:<30} {:<50}",
                    svc.service_id,
                    svc.endpoint_type,
                    vis_str,
                    format_expiry(svc.instance_certificate_expires_at),
                    svc.interfaces.join(", ")
                );
            }
        }
        SvcCommands::ProxyOutbox { svc_id } => {
            let items = client.proxy_outbox(svc_id.clone()).await?;
            if items.is_empty() {
                println!("No queued proxy calls for {svc_id}");
            } else {
                println!("{:<8} {:<10} {:<50}", "ID", "ATTEMPTS", "IDEMPOTENCY KEY");
                println!("{:-<70}", "");
                for item in items {
                    println!("{:<8} {:<10} {:<50}", item.id, item.attempts, item.idempotency_key);
                }
            }
        }
        SvcCommands::ProxyDeadLetters { svc_id } => {
            let items = client.proxy_dead_letters(svc_id.clone()).await?;
            if items.is_empty() {
                println!("No proxy dead letters for {svc_id}");
            } else {
                println!(
                    "{:<8} {:<10} {:<30} {:<40} {:<40}",
                    "ID", "ATTEMPTS", "CREATED", "IDEMPOTENCY KEY", "LAST ERROR"
                );
                println!("{:-<130}", "");
                for item in items {
                    println!(
                        "{:<8} {:<10} {:<30} {:<40} {:<40}",
                        item.id,
                        item.attempts,
                        DateTime::from_timestamp_millis(item.created_at)
                            .map_or_else(|| "-".to_string(), |dt| dt.to_rfc3339()),
                        item.idempotency_key,
                        item.last_error
                    );
                }
            }
        }
        SvcCommands::ProxyReplay { svc_id, dead_letter_id } => {
            client.proxy_replay(svc_id.clone(), *dead_letter_id).await?;
            println!("Re-enqueued dead letter {dead_letter_id} for {svc_id}");
        }
        SvcCommands::Sagas { svc_id } => {
            let items = client.sagas(svc_id.clone()).await?;
            if items.is_empty() {
                println!("No sagas for {svc_id}");
            } else {
                println!(
                    "{:<38} {:<20} {:<13} {:<10} {:<40}",
                    "SAGA ID", "NAME", "STATE", "STEPS", "LAST ERROR"
                );
                println!("{:-<125}", "");
                for item in items {
                    println!(
                        "{:<38} {:<20} {:<13?} {:<10} {:<40}",
                        item.saga_id,
                        item.name,
                        item.state,
                        format!("{}/{}", item.compensated_steps, item.steps),
                        item.last_error.as_deref().unwrap_or("-")
                    );
                }
            }
        }
        SvcCommands::SagaCompensate { svc_id, saga_id } => {
            client.saga_compensate(svc_id.clone(), saga_id.clone()).await?;
            println!("Re-armed saga {saga_id} for {svc_id}");
        }
        SvcCommands::Restart { svc_id } => {
            // Unmanaged (M05A A5a): an operator-driven `svc restart`
            // always presents generation 0, the same convention `svc
            // deploy`/`svc remove` use.
            client.restart(svc_id.clone(), 0).await?;
            println!("Successfully restarted svc {svc_id}");
        }
    }
    Ok(())
}

/// `-` for a service with no installed instance certificate; otherwise an
/// RFC 3339 timestamp, so "when does this fall over" is answerable without
/// reading logs (ADR-0020 §3).
fn format_expiry(expires_at_secs: Option<u64>) -> String {
    match expires_at_secs {
        Some(secs) => DateTime::from_timestamp(secs as i64, 0)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_else(|| "-".to_string()),
        None => "-".to_string(),
    }
}

/// Parses visibility values (e.g. `--visibility`, `--asset-visibility`).
fn parse_visibility(value: &str, flag_name: &str) -> anyhow::Result<Visibility> {
    match value.to_lowercase().as_str() {
        "public" => Ok(Visibility::Public),
        "internal" => Ok(Visibility::Internal),
        "private" => Ok(Visibility::Private),
        other => {
            anyhow::bail!("{flag_name} '{other}' is not one of: public, internal, private")
        }
    }
}

/// Parses `--interfaces`' comma-separated value into a non-empty,
/// non-blank interface name list. A blank `--interfaces` value (the
/// common case: an operator with one interface and no reason to name it)
/// falls back to `DEFAULT_INTERFACE_NAME` -- the
/// same name a manifest-driven deploy's own equivalent fallback uses
/// (`sdk::mapper`'s TCP mapping), so "what does an unnamed interface get
/// called" has one answer regardless of which deploy path minted it.
///
/// This used to be `if ifaces.is_empty() { vec!["default"] } else {
/// ifaces }` applied *after* splitting -- dead code, since
/// `"".split(',')` yields one empty-string element, never zero elements,
/// so the fallback could never fire. `--interfaces ""` silently
/// registered a service under the literal interface name `""` instead.
/// A comma-separated value with a genuinely blank *segment* (a stray
/// comma, e.g. `"http,,admin"`) is different from an omitted value
/// entirely and is refused rather than guessed at.
fn parse_interfaces(interfaces: &str) -> anyhow::Result<Vec<String>> {
    if interfaces.trim().is_empty() {
        return Ok(vec![DEFAULT_INTERFACE_NAME.to_string()]);
    }
    let ifaces: Vec<String> = interfaces.split(',').map(|s| s.trim().to_string()).collect();
    if let Some(pos) = ifaces.iter().position(|s| s.is_empty()) {
        anyhow::bail!(
            "--interfaces '{interfaces}' has a blank interface name at position {}; remove the \
             stray comma, or leave --interfaces empty entirely for a single interface named '{}'",
            pos + 1,
            DEFAULT_INTERFACE_NAME
        );
    }
    Ok(ifaces)
}

fn get_host_port_from_tcp_addr(tcp_addr: &str) -> anyhow::Result<(String, u16)> {
    let parts: Vec<&str> = tcp_addr.split(':').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid TCP address format. Expected host:port");
    }
    let host = parts[0].to_string();
    let port = parts[1].parse::<u16>()?;
    Ok((host, port))
}

/// Podman's `-p`/`--publish` accepts only these two protocol suffixes.
const CONTAINER_PORT_PROTOCOLS: [&str; 2] = ["tcp", "udp"];

/// Parses a `--port` value of the form
/// "interface:container_port[:host_port][:protocol]". `host_port` may be
/// left empty (e.g. "iface:80::udp") to pick `protocol` without pinning a
/// host port.
fn parse_container_port_mapping(spec: &str) -> anyhow::Result<ContainerPortMapping> {
    let parts: Vec<&str> = spec.split(':').collect();
    if !(2..=4).contains(&parts.len()) {
        anyhow::bail!(
            "Invalid --port '{spec}'. Expected interface:container_port[:host_port][:protocol]"
        );
    }
    if parts[0].is_empty() {
        anyhow::bail!("Invalid --port '{spec}': interface name must not be empty");
    }
    let interface_name = parts[0].to_string();
    let container_port = parts[1]
        .parse::<u16>()
        .map_err(|e| anyhow::anyhow!("Invalid container_port in --port '{spec}': {e}"))?;
    let host_port = match parts.get(2) {
        Some(&"") | None => None,
        Some(raw) => Some(
            raw.parse::<u16>()
                .map_err(|e| anyhow::anyhow!("Invalid host_port in --port '{spec}': {e}"))?,
        ),
    };
    let protocol = match parts.get(3) {
        None => "tcp".to_string(),
        Some(p) => {
            if !CONTAINER_PORT_PROTOCOLS.contains(p) {
                anyhow::bail!(
                    "Invalid --port '{spec}': protocol '{p}' is not one of {}",
                    CONTAINER_PORT_PROTOCOLS.join(", ")
                );
            }
            p.to_string()
        }
    };
    Ok(ContainerPortMapping { interface_name, host_port, container_port, protocol })
}

/// Parses a `--volume` value of the form "host_path:container_path". The
/// in-volume file materialization (`ContainerVolumeMapping::files`) has no
/// CLI flag yet, so it is always empty here.
fn parse_container_volume_mapping(spec: &str) -> anyhow::Result<ContainerVolumeMapping> {
    let parts: Vec<&str> = spec.split(':').collect();
    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        anyhow::bail!(
            "Invalid --volume '{spec}'. Expected host_path:container_path -- Docker-style mount \
             options (e.g. a trailing ':ro') are not supported"
        );
    }
    Ok(ContainerVolumeMapping {
        host_path: parts[0].to_string(),
        container_path: parts[1].to_string(),
        files: vec![],
    })
}

/// `--port`/`--volume` are only meaningful alongside `--image`. clap's own
/// `requires` cannot fully guard this: `--image` also `conflicts_with`
/// `--wasm`/`--tcp`, and when one of those is present clap treats `--image`
/// as unreachable and silently skips enforcing anything that requires it --
/// so `--tcp ... --port ...` (no `--image`) parses fine at the clap layer.
/// This is checked again here, at runtime, for every combination.
fn validate_container_flags(
    image: &Option<String>,
    ports: &[String],
    volumes: &[String],
) -> anyhow::Result<()> {
    if image.is_none() && (!ports.is_empty() || !volumes.is_empty()) {
        anyhow::bail!("--port/--volume require --image");
    }
    Ok(())
}

/// Every deployed container needs at least one reachable port, and every
/// `--port`'s interface must be one `--interfaces` actually declared --
/// otherwise a typo registers a phantom interface with no warning.
fn validate_container_ports(
    ifaces: &[String],
    port_mappings: &[ContainerPortMapping],
) -> anyhow::Result<()> {
    if port_mappings.is_empty() {
        anyhow::bail!("--image requires at least one --port, or the container is unreachable");
    }
    for mapping in port_mappings {
        if !ifaces.contains(&mapping.interface_name) {
            anyhow::bail!(
                "--port names interface '{}', which is not in --interfaces ({})",
                mapping.interface_name,
                ifaces.join(",")
            );
        }
    }
    Ok(())
}

fn load_identity(dir: &Path, name: &str) -> anyhow::Result<Identity> {
    let key_path = dir.join("identities").join(format!("{name}.key"));
    if !key_path.exists() {
        anyhow::bail!("Identity '{}' not found at {}", name, key_path.display());
    }
    Identity::load_from_path(&key_path)
}

#[cfg(test)]
mod tests {
    use clap::{Parser, error::ErrorKind};
    use syneroym_core::dht_registry::SignedEndpointInfo;
    use syneroym_sdk::SyneroymClient;

    use super::*;

    #[derive(Debug, Parser)]
    struct Wrapper {
        #[command(subcommand)]
        cmd: SvcCommands,
    }

    /// `--instance-certificate` installs an already-minted certificate
    /// as-is; `--master` mints and installs a fresh one itself.
    /// Together they're ambiguous about which certificate actually gets
    /// installed, so clap must reject the combination before either flag's
    /// handler ever runs.
    #[test]
    fn deploy_rejects_master_and_instance_certificate_together() {
        let err = Wrapper::try_parse_from([
            "svc",
            "deploy",
            "--svc-id",
            "did:key:zTest",
            "--interfaces",
            "default",
            "--tcp",
            "localhost:1",
            "--master",
            "m",
            "--instance-certificate",
            "/tmp/cert.json",
        ])
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ArgumentConflict);
    }

    /// `--wasm` and `--image` pick two different deploy kinds for the same
    /// service -- clap must reject the combination, not silently prefer one
    /// via if/else precedence in the handler.
    #[test]
    fn deploy_rejects_wasm_and_image_together() {
        let err = Wrapper::try_parse_from([
            "svc",
            "deploy",
            "--svc-id",
            "did:key:zTest",
            "--interfaces",
            "default",
            "--wasm",
            "/tmp/app.wasm",
            "--image",
            "docker.io/library/nginx:alpine",
        ])
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ArgumentConflict);
    }

    /// Same as `deploy_rejects_wasm_and_image_together`, for the other pair
    /// of deploy-kind flags.
    #[test]
    fn deploy_rejects_tcp_and_image_together() {
        let err = Wrapper::try_parse_from([
            "svc",
            "deploy",
            "--svc-id",
            "did:key:zTest",
            "--interfaces",
            "default",
            "--tcp",
            "localhost:1",
            "--image",
            "docker.io/library/nginx:alpine",
        ])
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ArgumentConflict);
    }

    /// `--port`/`--volume` only mean anything alongside `--image`. clap
    /// cannot reject this combination itself (see `validate_container_flags`
    /// for why), so `--tcp ... --port ...` parses fine at the clap layer --
    /// it is `validate_container_flags`, called from the handler before
    /// either arm runs, that must catch it instead.
    #[test]
    fn deploy_with_tcp_and_port_parses_but_is_not_a_valid_combination() {
        let cmd = Wrapper::try_parse_from([
            "svc",
            "deploy",
            "--svc-id",
            "did:key:zTest",
            "--interfaces",
            "default",
            "--tcp",
            "localhost:1",
            "--port",
            "default:80:8080",
        ])
        .expect("clap itself does not reject --tcp with --port");
        let SvcCommands::Deploy { image, ports, .. } = cmd.cmd else {
            panic!("expected SvcCommands::Deploy");
        };
        assert!(image.is_none());
        assert_eq!(
            validate_container_flags(&image, &ports, &[]).unwrap_err().to_string(),
            "--port/--volume require --image"
        );
    }

    #[test]
    fn validate_container_flags_rejects_ports_without_image() {
        let ports = vec!["default:80:8080".to_string()];
        let err = validate_container_flags(&None, &ports, &[]).unwrap_err();
        assert!(err.to_string().contains("--port/--volume require --image"));
    }

    #[test]
    fn validate_container_flags_rejects_volumes_without_image() {
        let volumes = vec!["html:/usr/share/nginx/html".to_string()];
        let err = validate_container_flags(&None, &[], &volumes).unwrap_err();
        assert!(err.to_string().contains("--port/--volume require --image"));
    }

    #[test]
    fn validate_container_flags_accepts_ports_and_volumes_with_image() {
        let image = Some("docker.io/library/nginx:alpine".to_string());
        let ports = vec!["default:80:8080".to_string()];
        let volumes = vec!["html:/usr/share/nginx/html".to_string()];
        validate_container_flags(&image, &ports, &volumes).unwrap();
    }

    #[test]
    fn validate_container_flags_accepts_image_with_no_ports_or_volumes() {
        let image = Some("docker.io/library/nginx:alpine".to_string());
        validate_container_flags(&image, &[], &[]).unwrap();
    }

    /// `--master` plus `--nickname` needs no `--identity` -- the master
    /// identity already loaded on this arm carries the envelope.
    #[test]
    fn deploy_with_a_master_and_a_nickname_needs_no_identity_flag() {
        let cmd = Wrapper::try_parse_from([
            "svc",
            "deploy",
            "--svc-id",
            "did:key:zTest",
            "--interfaces",
            "default",
            "--tcp",
            "localhost:1",
            "--master",
            "m",
            "--nickname",
            "alice",
        ])
        .expect("--master with --nickname and no --identity must parse");
        let SvcCommands::Deploy { master, nickname, identity, .. } = cmd.cmd else {
            panic!("expected SvcCommands::Deploy");
        };
        assert_eq!(master.as_deref(), Some("m"));
        assert_eq!(nickname.as_deref(), Some("alice"));
        assert!(identity.is_none());
    }

    /// `--instance-certificate` alone (no `--identity`/`--master`) parses
    /// fine with a `--nickname`, even though there is no local key that
    /// could sign a record verifying under `svc_id` -- the nickname is
    /// silently dropped with a warning at runtime, not rejected at parse
    /// time.
    #[test]
    fn deploy_with_an_instance_certificate_and_a_nickname_needs_no_identity_flag() {
        let cmd = Wrapper::try_parse_from([
            "svc",
            "deploy",
            "--svc-id",
            "did:key:zTest",
            "--interfaces",
            "default",
            "--tcp",
            "localhost:1",
            "--instance-certificate",
            "/tmp/cert.json",
            "--nickname",
            "alice",
        ])
        .expect("--instance-certificate with --nickname and no --identity must parse");
        let SvcCommands::Deploy { instance_certificate, nickname, identity, .. } = cmd.cmd else {
            panic!("expected SvcCommands::Deploy");
        };
        assert_eq!(instance_certificate, Some(PathBuf::from("/tmp/cert.json")));
        assert_eq!(nickname.as_deref(), Some("alice"));
        assert!(identity.is_none());
    }

    /// `--image` (with repeatable `--port`/`--volume`) parses and reaches
    /// the container arm, mirroring `--wasm`/`--tcp`'s own presence-based
    /// dispatch -- `wasm`/`tcp` stay `None` since only `--image` was given.
    #[test]
    fn deploy_with_image_port_and_volume_reaches_the_container_arm() {
        let cmd = Wrapper::try_parse_from([
            "svc",
            "deploy",
            "--svc-id",
            "did:key:zTest",
            "--interfaces",
            "default",
            "--image",
            "docker.io/library/nginx:alpine",
            "--port",
            "default:80:8080:tcp",
            "--volume",
            "html:/usr/share/nginx/html",
        ])
        .expect("--image with --port and --volume must parse");
        let SvcCommands::Deploy { image, ports, volumes, wasm, tcp, .. } = cmd.cmd else {
            panic!("expected SvcCommands::Deploy");
        };
        assert_eq!(image.as_deref(), Some("docker.io/library/nginx:alpine"));
        assert_eq!(ports, vec!["default:80:8080:tcp".to_string()]);
        assert_eq!(volumes, vec!["html:/usr/share/nginx/html".to_string()]);
        assert!(wasm.is_none());
        assert!(tcp.is_none());
    }

    /// `--master` with `--image` parses, mirroring
    /// `deploy_with_a_master_and_a_nickname_needs_no_identity_flag` for the
    /// TCP arm -- a member master applies identically regardless of which
    /// service type is being deployed.
    #[test]
    fn deploy_with_a_master_and_an_image_parses() {
        let cmd = Wrapper::try_parse_from([
            "svc",
            "deploy",
            "--svc-id",
            "did:key:zTest",
            "--interfaces",
            "default",
            "--image",
            "docker.io/library/nginx:alpine",
            "--master",
            "m",
        ])
        .expect("--master with --image must parse");
        let SvcCommands::Deploy { master, image, .. } = cmd.cmd else {
            panic!("expected SvcCommands::Deploy");
        };
        assert_eq!(master.as_deref(), Some("m"));
        assert_eq!(image.as_deref(), Some("docker.io/library/nginx:alpine"));
    }

    #[test]
    fn parse_container_port_mapping_rejects_malformed_input() {
        let err = parse_container_port_mapping("default").unwrap_err();
        assert!(err.to_string().contains("Invalid --port"));
    }

    #[test]
    fn parse_container_port_mapping_defaults_host_port_and_protocol() {
        let mapping = parse_container_port_mapping("default:80").unwrap();
        assert_eq!(mapping.interface_name, "default");
        assert_eq!(mapping.container_port, 80);
        assert_eq!(mapping.host_port, None);
        assert_eq!(mapping.protocol, "tcp");
    }

    #[test]
    fn parse_container_port_mapping_parses_host_port_and_protocol() {
        let mapping = parse_container_port_mapping("default:80:8080:udp").unwrap();
        assert_eq!(mapping.host_port, Some(8080));
        assert_eq!(mapping.protocol, "udp");
    }

    #[test]
    fn parse_container_port_mapping_allows_an_empty_host_port_with_a_protocol() {
        let mapping = parse_container_port_mapping("default:80::udp").unwrap();
        assert_eq!(mapping.host_port, None);
        assert_eq!(mapping.protocol, "udp");
    }

    #[test]
    fn parse_container_port_mapping_rejects_an_empty_interface_name() {
        let err = parse_container_port_mapping(":80:8080").unwrap_err();
        assert!(err.to_string().contains("interface name must not be empty"));
    }

    #[test]
    fn parse_container_port_mapping_rejects_an_unsupported_protocol() {
        let err = parse_container_port_mapping("default:80:8080:tpc").unwrap_err();
        assert!(err.to_string().contains("protocol 'tpc' is not one of"));
    }

    #[test]
    fn parse_container_port_mapping_rejects_too_many_segments() {
        let err = parse_container_port_mapping("default:80:8080:tcp:extra").unwrap_err();
        assert!(err.to_string().contains("Invalid --port"));
    }

    #[test]
    fn parse_container_port_mapping_rejects_a_non_numeric_container_port() {
        let err = parse_container_port_mapping("default:not-a-port").unwrap_err();
        assert!(err.to_string().contains("Invalid container_port"));
    }

    #[test]
    fn parse_container_volume_mapping_rejects_malformed_input() {
        let err = parse_container_volume_mapping("no-colon-here").unwrap_err();
        assert!(err.to_string().contains("Invalid --volume"));
    }

    #[test]
    fn parse_container_volume_mapping_parses_host_and_container_path() {
        let mapping = parse_container_volume_mapping("html:/usr/share/nginx/html").unwrap();
        assert_eq!(mapping.host_path, "html");
        assert_eq!(mapping.container_path, "/usr/share/nginx/html");
        assert!(mapping.files.is_empty());
    }

    /// Docker/Podman users commonly type a trailing mount option like
    /// `:ro` -- rejecting it with a clear message beats silently folding it
    /// into `container_path` (`/var/lib/data:ro`, which podman would then
    /// fail on with a much more confusing error).
    #[test]
    fn parse_container_volume_mapping_rejects_a_docker_style_mount_option() {
        let err = parse_container_volume_mapping("/data:/var/lib/data:ro").unwrap_err();
        assert!(err.to_string().contains("mount options"));
    }

    #[test]
    fn validate_container_ports_rejects_no_ports() {
        let ifaces = vec!["default".to_string()];
        let err = validate_container_ports(&ifaces, &[]).unwrap_err();
        assert!(err.to_string().contains("at least one --port"));
    }

    #[test]
    fn validate_container_ports_rejects_a_port_interface_absent_from_interfaces() {
        let ifaces = vec!["default".to_string()];
        let mapping = parse_container_port_mapping("other:80").unwrap();
        let err = validate_container_ports(&ifaces, &[mapping]).unwrap_err();
        assert!(err.to_string().contains("not in --interfaces"));
    }

    #[test]
    fn validate_container_ports_accepts_a_port_naming_a_declared_interface() {
        let ifaces = vec!["default".to_string()];
        let mapping = parse_container_port_mapping("default:80").unwrap();
        validate_container_ports(&ifaces, &[mapping]).unwrap();
    }

    /// A blank `--interfaces` composes correctly with `--port` for a
    /// container deploy too, not only `--wasm`/`--tcp`: `parse_interfaces`
    /// falls back to `["default"]`, and a `--port default:80` mapping
    /// names exactly that.
    #[test]
    fn a_blank_interfaces_value_composes_with_a_default_named_container_port() {
        let ifaces = parse_interfaces("").unwrap();
        let mapping = parse_container_port_mapping("default:80").unwrap();
        validate_container_ports(&ifaces, &[mapping]).unwrap();
    }

    /// The bug this replaces: `"".split(',')` yields one empty-string
    /// element, never zero, so a length check could never see this case
    /// as "empty" -- `--interfaces ""` used to silently register a
    /// service under the literal interface name `""`.
    #[test]
    fn parse_interfaces_falls_back_to_the_shared_default_name_when_blank() {
        assert_eq!(parse_interfaces("").unwrap(), vec![DEFAULT_INTERFACE_NAME.to_string()]);
        assert_eq!(parse_interfaces("   ").unwrap(), vec![DEFAULT_INTERFACE_NAME.to_string()]);
    }

    #[test]
    fn parse_interfaces_splits_and_trims_a_real_list() {
        assert_eq!(
            parse_interfaces("http, admin").unwrap(),
            vec!["http".to_string(), "admin".to_string()]
        );
    }

    /// A blank *segment* amid otherwise real names (a stray comma) is a
    /// different mistake than an omitted value entirely, and is refused
    /// rather than silently coerced to the default.
    #[test]
    fn parse_interfaces_rejects_a_blank_segment_in_an_otherwise_real_list() {
        let err = parse_interfaces("http,,admin").unwrap_err();
        assert!(err.to_string().contains("position 2"), "{err}");

        let err = parse_interfaces(",admin").unwrap_err();
        assert!(err.to_string().contains("position 1"), "{err}");
    }

    #[test]
    fn format_expiry_shows_a_dash_for_no_certificate() {
        assert_eq!(format_expiry(None), "-");
    }

    #[test]
    fn format_expiry_shows_an_rfc3339_timestamp() {
        // 2024-01-01T00:00:00Z
        assert_eq!(format_expiry(Some(1_704_067_200)), "2024-01-01T00:00:00+00:00");
    }

    #[test]
    fn parse_visibility_accepts_the_three_declared_values_case_insensitively() {
        assert_eq!(parse_visibility("public", "--visibility").unwrap(), Visibility::Public);
        assert_eq!(parse_visibility("Internal", "--visibility").unwrap(), Visibility::Internal);
        assert_eq!(parse_visibility("PRIVATE", "--visibility").unwrap(), Visibility::Private);
    }

    #[test]
    fn parse_visibility_rejects_an_unknown_value_naming_the_flag() {
        let err = parse_visibility("hidden", "--visibility").unwrap_err();
        assert!(err.to_string().contains("--visibility"), "{err}");
        assert!(err.to_string().contains("hidden"), "{err}");

        let err = parse_visibility("hidden", "--asset-visibility").unwrap_err();
        assert!(err.to_string().contains("--asset-visibility"), "{err}");
    }

    /// Plan test 45 (ADR-0018 §2, `D-B2-8`): the `--record-out` -> file ->
    /// `new_with_record` round trip. `new_with_record_verifies_signature_
    /// and_sets_fields` (`crates/sdk/src/lib.rs`) already covers the
    /// signature-verification half against an in-memory record; what is
    /// missing is the file itself -- `svc deploy` cannot be driven from a
    /// test (`roymctl` is a binary, not linkable), so this builds the exact
    /// record shape the private-with-export deploy arm signs (`is_private:
    /// true`, since that record must never be admitted by a registry),
    /// writes it to a temp file the way `--record-out` does, and reads it
    /// back before connecting.
    #[test]
    fn a_record_out_file_round_trips_through_new_with_record() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("svc.record.json");

        let identity = Identity::generate().unwrap();
        let svc_id = substrate::derive_did_key(&identity.public_key());
        let record = EndpointInfo {
            service_id: svc_id.clone(),
            substrate_id: "did:key:zSubstrate".to_string(),
            endpoint_type: EndpointType::Service,
            mechanisms: vec![],
            nickname: Some("my-private-svc".to_string()),
            is_private: true,
            ttl: None,
            not_after: 9_999_999_999,
            generation: 0,
        }
        .sign(&identity)
        .unwrap();

        fs::write(&path, serde_json::to_string_pretty(&record).unwrap()).unwrap();

        let read_back: SignedEndpointInfo =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let client =
            SyneroymClient::new_with_record(read_back, "http://127.0.0.1:9999".to_string())
                .unwrap();
        assert_eq!(client.service_id(), svc_id);
    }
}
