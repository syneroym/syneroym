use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use syneroym_app_orchestration::{
    DEFAULT_BINDING_CACHE_TTL_MS,
    models::{
        AssetBundle, DeploymentPlan, DocumentRef, HealthCheck, LogicalServiceName, MemberRef,
        PlannedService, RotationPolicy, ServiceId, ServiceType, TopologyMode,
        Visibility as ModelVisibility,
    },
};
use syneroym_core::{deploy_docs, util};
use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
    AppContext as WitAppContext, ArtifactSource, AssetBundle as WitAssetBundle, ContainerManifest,
    ContainerPortMapping, ContainerVolumeFile, ContainerVolumeMapping,
    DependencyBinding as WitDependencyBinding, DeployManifest, DeploymentPlan as WitDeploymentPlan,
    DocumentSource, HealthCheck as WitHealthCheck, HttpProbe as WitHttpProbe, NetworkEndpoint,
    PlannedService as WitPlannedService, ResourceQuota, RotationPolicy as WitRotationPolicy,
    RpcProbe as WitRpcProbe, ServiceConfig as WitServiceConfig, ServiceType as WitServiceType,
    TcpManifest, TcpProbe as WitTcpProbe, TopologyMode as WitTopologyMode,
    Visibility as WitVisibility, WasmManifest,
};

/// Marks a `ServiceConfig.source` value as hex-encoded artifact bytes
/// rather than a URL or a local path: a plan applied by the App
/// Supervisor runs on a remote substrate with no access to the operator's
/// filesystem, so `roymctl supervisor submit` inlines each Wasm artifact
/// into `source` itself before the plan is ever sent -- `hex`, not
/// base64, to avoid a new dependency for a one-shot RPC payload.
pub const INLINE_ARTIFACT_PREFIX: &str = "data:hex,";

/// The interface name a single-interface TCP/container/WASM service gets
/// when its author declares none explicitly. The one name both this
/// mapper (a manifest-driven deploy's own fallback, below) and `roymctl
/// svc deploy` (an ad-hoc deploy's `--interfaces` fallback,
/// `apps/roymctl/src/commands/svc.rs`) use, so "what does an unnamed
/// interface get called" has one answer across both deploy paths --
/// before this constant existed the two independently picked different
/// strings ("main" here, and a "default" this mapper never saw, since
/// `svc.rs`'s own check for it was dead code).
pub const DEFAULT_INTERFACE_NAME: &str = "default";

/// Author-side container volume, mirroring the wire record but with `files`
/// optional (so a volume that only needs an empty directory stays as terse as
/// it was) and file contents still unresolved.
#[derive(serde::Deserialize)]
struct VolumeSpec {
    host_path: String,
    container_path: String,
    #[serde(default)]
    files: Vec<VolumeFileSpec>,
}

#[derive(serde::Deserialize)]
struct VolumeFileSpec {
    relative_path: String,
    content: DocumentRef,
}

/// Resolves an author-side document reference for the wire.
///
/// A bare path is read here, client-side, and travels inline -- the same
/// treatment `source` already gets for a Wasm component just below, and the
/// reason a deploy works against a substrate with nothing pre-staged. An
/// explicit `remote_path` is passed through untouched for the substrate to
/// resolve against its own filesystem.
fn map_document_ref(doc: &DocumentRef, field_name: &str) -> anyhow::Result<DocumentSource> {
    match doc {
        DocumentRef::Local(path) => {
            let bytes = util::read_local_artifact(Path::new(path))?;
            // Checked here, before the UTF-8 copy and the RPC round-trip, so
            // an oversized document is an instant local error rather than a
            // payload the substrate rejects after receiving all of it.
            deploy_docs::check_inline_size_bytes(bytes.len(), field_name)
                .map_err(|e| anyhow::anyhow!("{path}: {e}"))?;
            let content = String::from_utf8(bytes).map_err(|e| {
                anyhow::anyhow!("{field_name} at {path} is not valid UTF-8 text: {e}")
            })?;
            Ok(DocumentSource::Inline(content))
        }
        DocumentRef::Remote { remote_path } => Ok(DocumentSource::Path(remote_path.clone())),
    }
}

/// Resolves a `source`-shaped field into a wire `ArtifactSource`: a URL
/// passes through, `INLINE_ARTIFACT_PREFIX`-prefixed content decodes as
/// hex-encoded bytes (the remote-submit inlining path), and anything else is
/// read as a local path off this process's working directory. Shared by the
/// Wasm component's `source` and the asset bundle `archive` -- the two
/// fields carrying this same three-way shape.
fn resolve_artifact_source(source: &str, what: &str) -> anyhow::Result<ArtifactSource> {
    if source.starts_with("http://") || source.starts_with("https://") {
        Ok(ArtifactSource::Url(source.to_string()))
    } else if let Some(hex_bytes) = source.strip_prefix(INLINE_ARTIFACT_PREFIX) {
        let bytes = hex::decode(hex_bytes)
            .map_err(|e| anyhow::anyhow!("invalid inline {what} encoding: {e}"))?;
        Ok(ArtifactSource::Binary(bytes))
    } else {
        let path = PathBuf::from(source);
        let bytes = util::read_local_artifact(&path)?;
        Ok(ArtifactSource::Binary(bytes))
    }
}

const fn map_visibility(v: ModelVisibility) -> WitVisibility {
    match v {
        ModelVisibility::Public => WitVisibility::Public,
        ModelVisibility::Internal => WitVisibility::Internal,
        ModelVisibility::Private => WitVisibility::Private,
    }
}

/// Maps the app model's `AssetBundle` to the wire record. Absent
/// `visibility` is never produced here -- the model field already defaults
/// to `Private` at parse time (`#[serde(default)]`), so the wire always
/// carries an explicit value.
fn map_asset_bundle(bundle: &AssetBundle, what: &str) -> anyhow::Result<WitAssetBundle> {
    Ok(WitAssetBundle {
        archive: resolve_artifact_source(&bundle.archive, what)?,
        hash: bundle.hash.clone(),
        visibility: Some(map_visibility(bundle.visibility)),
    })
}

/// Maps the app model's `TopologyMode` to the wire `topology-mode` variant.
/// No `sharding-strategy` on the wire yet -- `sharded` means hash
/// sharding until a manifest can express otherwise.
fn map_mode(mode: TopologyMode) -> WitTopologyMode {
    match mode {
        TopologyMode::Singleton => WitTopologyMode::Singleton,
        TopologyMode::Redundant => WitTopologyMode::Redundant,
        TopologyMode::Sharded => WitTopologyMode::Sharded,
    }
}

/// Maps the app model's `HealthCheck` to the wire variant. Pure translation:
/// no defaulting, no validation -- serde already applied the field defaults
/// at parse time, and kind/type compatibility is the substrate's deploy-time
/// check, so a client cannot smuggle a bad pairing past it.
fn map_health_check(check: &HealthCheck) -> WitHealthCheck {
    match check {
        HealthCheck::TcpConnect(p) => WitHealthCheck::TcpConnect(WitTcpProbe {
            interface_name: p.interface.to_string(),
            timeout_ms: p.timeout_ms,
        }),
        HealthCheck::HttpGet(p) => WitHealthCheck::HttpGet(WitHttpProbe {
            interface_name: p.interface.to_string(),
            path: p.path.clone(),
            expect_status: p.expect_status,
            timeout_ms: p.timeout_ms,
        }),
        HealthCheck::Rpc(p) => WitHealthCheck::Rpc(WitRpcProbe {
            interface_name: p.interface.to_string(),
            method: p.method.clone(),
            timeout_ms: p.timeout_ms,
        }),
    }
}

/// Maps exactly the services in `services`, while computing every
/// dependency's topology mode from the **whole** `plan`.
///
/// The split matters: a dependency's `mode` belongs to the dependency, which
/// may be placed on a different substrate and therefore absent from
/// `services`. Deriving modes from the subset would silently default every
/// cross-substrate dependency to `Singleton`.
///
/// `instance_certificates` maps a `PlannedService.service_id` (post
/// member-master substitution, if any) to the JSON-serialized
/// `DelegationCertificate` to install for it, and `registry_certificates` the
/// same key to the JSON-serialized, master-signed `SignedEndpointInfo` to
/// publish. Both are empty for a plan run without member masters -- every
/// service then maps to `None` for both fields, exactly as before either
/// parameter existed. The mapper only *translates* values that already
/// exist; it never mints or signs anything itself.
///
/// `PlannedService.substrate` is not mapped onto the wire: a substrate has no
/// use for the placement of services it is not hosting, and publishing it
/// would hand every node a partial topology map of the app for nothing.
///
/// `binding_epochs` is keyed by `MemberRef`, not `LogicalServiceRef`: the
/// epoch belongs to the dependent *member*, since each member holds its own
/// `service_bindings` row on the substrate.
pub fn map_deployment_plan_to_wit(
    plan: &DeploymentPlan,
    services: &[&PlannedService],
    instance_certificates: &BTreeMap<ServiceId, String>,
    registry_certificates: &BTreeMap<ServiceId, String>,
    emit_bindings: bool,
    generation: u64,
    binding_epochs: &BTreeMap<MemberRef, u64>,
) -> anyhow::Result<WitDeploymentPlan> {
    let plan_instance_id = plan.app_instance_id.to_string();
    // `mode` belongs to the *target* of a dependency, not the dependent --
    // build the lookup once, over every service in the whole plan, before the
    // per-service loop needs it. A dependency may be placed on a different
    // substrate and therefore absent from `services`.
    let target_modes: BTreeMap<LogicalServiceName, TopologyMode> = plan
        .services
        .iter()
        .map(|svc| (svc.logical_ref.service_name.clone(), svc.topology_mode))
        .collect();

    let mut wit_services = Vec::new();
    for svc in services {
        let wit_config = WitServiceConfig {
            env: svc.config.env.clone().into_iter().collect(),
            args: svc.config.args.clone(),
            custom_config: svc.config.custom_config.clone(),
            quota: svc.config.quota.clone().map(|q| ResourceQuota {
                max_instructions: q.max_instructions,
                max_memory_bytes: q.max_memory_bytes,
            }),
            schema: svc
                .config
                .schema
                .as_ref()
                .map(|d| map_document_ref(d, "schema"))
                .transpose()?,
            rotation_policy: Some(match svc.config.rotation_policy {
                RotationPolicy::RestartOnRotation => WitRotationPolicy::RestartOnRotation,
                RotationPolicy::None => WitRotationPolicy::None,
            }),
            fdae_policy: svc
                .config
                .fdae
                .as_ref()
                .map(|f| map_document_ref(&f.policy, "fdae policy"))
                .transpose()?,
            health_check: svc.config.health_check.as_ref().map(map_health_check),
            assets: svc
                .config
                .assets
                .as_ref()
                .map(|a| {
                    map_asset_bundle(a, &format!("asset bundle archive for {}", svc.service_id))
                })
                .transpose()?,
            visibility: Some(map_visibility(svc.config.visibility)),
        };

        let service_type = match svc.config.service_type {
            ServiceType::Wasm => {
                // A supervisor's `submit` runs on a remote substrate with no
                // access to the operator's local filesystem, so
                // `roymctl supervisor submit` inlines the artifact into
                // `source` itself before sending the plan -- the
                // `INLINE_ARTIFACT_PREFIX` arm below is what a
                // *substrate-side* mapping call (the supervisor's own apply
                // path) then decodes, never reading a local path at all.
                let source = resolve_artifact_source(
                    &svc.config.source,
                    &format!("wasm artifact for {}", svc.service_id),
                )?;
                WitServiceType::Wasm(WasmManifest {
                    source,
                    hash: svc.config.hash.clone(),
                    interfaces: svc.config.interfaces.iter().map(|i| i.to_string()).collect(),
                })
            }
            ServiceType::Tcp => {
                let mut endpoints = vec![];
                if let Some(custom) = &svc.config.custom_config
                    && let Ok(eps) = serde_json::from_str::<Vec<NetworkEndpoint>>(custom)
                {
                    endpoints = eps;
                }
                if endpoints.is_empty() {
                    let parts: Vec<&str> = svc.config.source.split(':').collect();
                    if parts.len() == 2 {
                        let host = parts[0].to_string();
                        if let Ok(port) = parts[1].parse::<u16>() {
                            endpoints.push(NetworkEndpoint {
                                interface_name: if svc.config.interfaces.is_empty() {
                                    DEFAULT_INTERFACE_NAME.to_string()
                                } else {
                                    svc.config.interfaces[0].to_string()
                                },
                                host,
                                port,
                            });
                        }
                    }
                }
                WitServiceType::Tcp(TcpManifest { endpoints })
            }
            ServiceType::Container => {
                let mut image = svc.config.source.clone();
                let mut ports = vec![];
                let mut volumes = vec![];

                if let Some(custom) = &svc.config.custom_config
                    && let Ok(cfg) = serde_json::from_str::<serde_json::Value>(custom)
                {
                    if let Some(img) = cfg.get("image").and_then(|v| v.as_str()) {
                        image = img.to_string();
                    }
                    // Strict, like `volumes` below: silently discarding a
                    // mistyped port list deploys a container that is simply
                    // unreachable, with nothing anywhere saying why.
                    if let Some(p) = cfg.get("ports") {
                        ports = serde_json::from_value::<Vec<ContainerPortMapping>>(p.clone())
                            .map_err(|e| anyhow::anyhow!("invalid container ports: {e}"))?;
                    }
                    if let Some(v) = cfg.get("volumes") {
                        let specs: Vec<VolumeSpec> = serde_json::from_value(v.clone())
                            .map_err(|e| anyhow::anyhow!("invalid container volumes: {e}"))?;
                        volumes = specs
                            .into_iter()
                            .map(|spec| {
                                Ok(ContainerVolumeMapping {
                                    host_path: spec.host_path,
                                    container_path: spec.container_path,
                                    files: spec
                                        .files
                                        .iter()
                                        .map(|f| {
                                            Ok(ContainerVolumeFile {
                                                relative_path: f.relative_path.clone(),
                                                content: map_document_ref(
                                                    &f.content,
                                                    "volume file",
                                                )?,
                                            })
                                        })
                                        .collect::<anyhow::Result<Vec<_>>>()?,
                                })
                            })
                            .collect::<anyhow::Result<Vec<_>>>()?;
                    }
                }

                WitServiceType::Container(ContainerManifest {
                    source: ArtifactSource::Binary(vec![]),
                    hash: svc.config.hash.clone(),
                    image,
                    ports,
                    volumes,
                })
            }
            ServiceType::NativeHost => {
                return Err(anyhow::anyhow!(
                    "NativeHost service type is not supported in deployment plans"
                ));
            }
        };
        let instance_certificate = instance_certificates.get(&svc.service_id).cloned();
        let registry_certificate = registry_certificates.get(&svc.service_id).cloned();
        let app_context = Some(WitAppContext {
            app_instance_id: plan_instance_id.clone(),
            service_name: svc.logical_ref.service_name.to_string(),
            // Without member-master substitution these members are
            // the compiler's fabricated `did:key:h...` ids, which resolve to
            // no key. Publishing them would make `dependency(...)` resolve
            // and then fail a layer down as `service-not-found`; an empty
            // list gives the guest the true answer,
            // `dependency-not-bound`.
            bindings: if emit_bindings {
                svc.resolved_dependencies
                    .iter()
                    .map(|(name, members)| WitDependencyBinding {
                        dependency_name: name.to_string(),
                        // Intra-app only.
                        app_instance_id: plan_instance_id.clone(),
                        mode: map_mode(target_modes.get(name).copied().unwrap_or_default()),
                        members: members.iter().map(ToString::to_string).collect(),
                        // The epoch belongs to the *dependent* service, not
                        // the dependency -- one counter per (app_instance_id,
                        // dependent logical_ref), shared by every one of that
                        // service's bindings. `0` (an absent entry) means
                        // "no supervisor has written here".
                        epoch: binding_epochs.get(&svc.member_ref()).copied().unwrap_or(0),
                        cache_ttl_ms: DEFAULT_BINDING_CACHE_TTL_MS,
                    })
                    .collect()
            } else {
                Vec::new()
            },
            // ADR-0021 §4: the management generation this apply writes at,
            // forwarded from the request unchanged.
            generation,
        });
        wit_services.push(WitPlannedService {
            service_id: svc.service_id.to_string(),
            logical_ref: svc.logical_ref.to_string(),
            manifest: DeployManifest {
                config: wit_config,
                service_type,
                registry_certificate,
                instance_certificate,
            },
            app_context,
        });
    }

    Ok(WitDeploymentPlan {
        app_instance_id: plan.app_instance_id.to_string(),
        blueprint_id: plan.blueprint_id.to_string(),
        version: plan.version.to_string(),
        services: wit_services,
    })
}

#[cfg(test)]
#[cfg(test)]
mod tests;
