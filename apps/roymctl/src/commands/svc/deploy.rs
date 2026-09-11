//! `svc deploy` -- WASM, TCP, and container service deployment, plus the
//! endpoint-record signing and instance-certificate handling it needs.

use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use syneroym_core::dht_registry::{
    DEFAULT_ENDPOINT_NOT_AFTER_SECS, EndpointInfo, EndpointType, SignedEndpointInfo,
};
use syneroym_identity::{DelegationCertificate, Identity, substrate};
use syneroym_sdk::{
    ArtifactSource, AssetBundle, ContainerPortMapping, ContainerVolumeMapping, DeploySvcOptions,
    NetworkEndpoint, Publication, SyneroymClient, Visibility, deploy,
    mapper::DEFAULT_INTERFACE_NAME,
};

use crate::commands::member_identity;

/// The attended posture's default certificate lifetime for a deploy-time
/// certification via `svc deploy --master` -- `identity certify-instance`
/// is the dedicated renewal command for a longer- or shorter-lived one.
const DEFAULT_INSTANCE_CERT_EXPIRES_HOURS: u64 = 24;

#[allow(clippy::too_many_arguments)]
pub(super) async fn handle_deploy(
    client: &mut SyneroymClient,
    substrate_did: &str,
    svc_id: &str,
    interfaces: &str,
    wasm: &Option<PathBuf>,
    tcp: &Option<String>,
    image: &Option<String>,
    ports: &[String],
    volumes: &[String],
    identity: &Option<String>,
    nickname: &Option<String>,
    master: &Option<String>,
    instance_certificate: &Option<PathBuf>,
    registry_url: &Option<String>,
    assets: &Option<PathBuf>,
    asset_visibility: &str,
    custom_config: &Option<PathBuf>,
    visibility: &Option<String>,
    record_out: &Option<PathBuf>,
    dir: &Path,
) -> anyhow::Result<()> {
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
            if did != svc_id {
                anyhow::bail!(
                    "--identity resolves to {did}, which is not --svc-id {svc_id}; the registry \
                     resolves a record's signing key from its own service_id, so this record \
                     could never be admitted"
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
    let signing_identity: Option<&Identity> = named_identity.as_ref().or(master_identity.as_ref());

    // ADR-0018 §5: a deploy that *can* sign a record but was not
    // told whether to publish it must fail loudly, not fall back to
    // `private` and succeed having published nothing. Silence is
    // only safe when there is no signing identity to publish with
    // in the first place.
    if signing_identity.is_some() && stated_visibility.is_none() {
        anyhow::bail!(
            "--identity/--master can sign a published endpoint record, so --visibility must be \
             given explicitly (\"public\", \"internal\", or \"private\") -- it no longer defaults \
             silently to private"
        );
    }
    let parsed_visibility = stated_visibility.unwrap_or(Visibility::Private);

    // Nothing to sign with, but a nickname was given: it is silently
    // dropped (unchanged from before this flag existed). Warn rather
    // than fail: the deploy itself still succeeds, and a
    // silently-lost nickname is confusing to debug.
    if nickname.is_some() && signing_identity.is_none() {
        eprintln!(
            "Warning: --nickname has no effect without --identity or --master -- it will not be \
             published."
        );
    }

    let not_after = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .saturating_add(DEFAULT_ENDPOINT_NOT_AFTER_SECS);

    let publication = match (parsed_visibility, signing_identity) {
        (Visibility::Private, Some(id)) if record_out.is_some() => {
            let record = signed_export_record(
                Visibility::Private,
                svc_id,
                substrate_did,
                nickname.clone(),
                not_after,
                id,
            )?;
            if let Some(path) = record_out {
                fs::write(path, serde_json::to_string_pretty(&record)?).map_err(|e| {
                    anyhow::anyhow!("failed to write --record-out at {}: {e}", path.display())
                })?;
                println!("Wrote the signed endpoint record to {}", path.display());
            }
            Publication::Private
        }
        (Visibility::Private, _) => {
            if record_out.is_some() {
                eprintln!("Warning: --record-out has no effect without --identity or --master");
            }
            Publication::Private
        }
        (v, None) => {
            let v_str = v.as_str();
            anyhow::bail!(
                "--visibility '{v_str}' needs --identity or --master: only the service's own key \
                 can sign a record the registry will admit"
            );
        }
        (v, Some(id)) => {
            let record =
                signed_export_record(v, svc_id, substrate_did, nickname.clone(), not_after, id)?;
            if let Some(path) = record_out {
                fs::write(path, serde_json::to_string_pretty(&record)?).map_err(|e| {
                    anyhow::anyhow!("failed to write --record-out at {}: {e}", path.display())
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
            if master_did != svc_id {
                anyhow::bail!(
                    "--master '{}' resolves to {master_did}, which does not match --svc-id \
                     {svc_id} -- an install-time certificate for this pair would be rejected",
                    master.as_deref().unwrap_or("?")
                );
            }
            let cert = deploy::certify_instance(
                client,
                resolved_master,
                svc_id,
                DEFAULT_INSTANCE_CERT_EXPIRES_HOURS,
            )
            .await?;
            member_identity::refresh_anchor_or_warn(registry_url.as_deref(), resolved_master)
                .await?;
            Some(cert)
        }
        (None, Some(path)) => {
            let cert_json = fs::read_to_string(path).map_err(|e| {
                anyhow::anyhow!("failed to read --instance-certificate at {}: {e}", path.display())
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
                    anyhow::anyhow!("failed to read --assets at {}: {e}", assets_path.display())
                })?;
                Some(AssetBundle {
                    archive: ArtifactSource::Binary(archive),
                    hash: None,
                    visibility: Some(parse_visibility(asset_visibility, "--asset-visibility")?),
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
                svc_id.to_string(),
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
            .map(|interface_name| NetworkEndpoint { interface_name, host: host.clone(), port })
            .collect();
        client.deploy_svc_tcp(svc_id.to_string(), endpoints, publication, instance_cert).await?;
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
                svc_id.to_string(),
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

    Ok(())
}

/// Parses visibility values (e.g. `--visibility`, `--asset-visibility`).
pub(crate) fn parse_visibility(value: &str, flag_name: &str) -> anyhow::Result<Visibility> {
    match value.to_lowercase().as_str() {
        "public" => Ok(Visibility::Public),
        "internal" => Ok(Visibility::Internal),
        "private" => Ok(Visibility::Private),
        other => {
            anyhow::bail!("{flag_name} '{other}' is not one of: public, internal, private")
        }
    }
}

/// Builds and signs the endpoint record `svc deploy` produces for a
/// signable `visibility` -- both the private-with-`--record-out` export arm
/// and the public/internal arm share this. `public` signs `is_private:
/// false`; `internal` and `private` both sign `is_private: true`, since a
/// record exported for a `private` service must never be admitted by a
/// registry, and `is_private` lives inside the signature, so this is the
/// only chance to say so. `RegistryClient::register`'s DHT gate trusts
/// this flag verbatim.
pub(crate) fn signed_export_record(
    visibility: Visibility,
    svc_id: &str,
    substrate_did: &str,
    nickname: Option<String>,
    not_after: u64,
    identity: &Identity,
) -> anyhow::Result<SignedEndpointInfo> {
    EndpointInfo {
        service_id: svc_id.to_string(),
        substrate_id: substrate_did.to_string(),
        endpoint_type: EndpointType::Service,
        mechanisms: vec![],
        nickname,
        is_private: visibility != Visibility::Public,
        ttl: None,
        not_after,
        generation: 0,
    }
    .sign(identity)
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
pub(crate) fn parse_interfaces(interfaces: &str) -> anyhow::Result<Vec<String>> {
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
pub(crate) fn parse_container_port_mapping(spec: &str) -> anyhow::Result<ContainerPortMapping> {
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
pub(crate) fn parse_container_volume_mapping(spec: &str) -> anyhow::Result<ContainerVolumeMapping> {
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
pub(crate) fn validate_container_flags(
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
pub(crate) fn validate_container_ports(
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
