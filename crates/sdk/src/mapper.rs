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
/// rather than a URL or a local path (M05A A5b): a plan applied by the App
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
/// hex-encoded bytes (D-A5-7's remote-submit inlining), and anything else is
/// read as a local path off this process's working directory. Shared by the
/// Wasm component's `source` and M06A A1's asset bundle `archive` -- the two
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

/// Maps the app model's `AssetBundle` (M06A A1) to the wire record. Absent
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

/// Maps the app model's `TopologyMode` to the wire `topology-mode` variant
/// (A2). No `sharding-strategy` on the wire yet -- `sharded` means hash
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
/// check (D-A4-6), so a client cannot smuggle a bad pairing past it.
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
/// `binding_epochs` is keyed by `MemberRef`, not `LogicalServiceRef` (M05A
/// A5e, D-A5e-2): the epoch belongs to the dependent *member*, since each
/// member holds its own `service_bindings` row on the substrate.
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
                // access to the operator's local filesystem (D-A5-7), so
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
            // D-A2-16: without member-master substitution these members are
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
                        // Intra-app only (D-A2-2).
                        app_instance_id: plan_instance_id.clone(),
                        mode: map_mode(target_modes.get(name).copied().unwrap_or_default()),
                        members: members.iter().map(ToString::to_string).collect(),
                        // M05A A5c §19.3/D-A5c-4: the epoch belongs to the
                        // *dependent* service, not the dependency -- one
                        // counter per (app_instance_id, dependent
                        // logical_ref), shared by every one of that
                        // service's bindings. `0` (an absent entry) means
                        // "no supervisor has written here", which is also
                        // what every caller through A5b still means by it.
                        epoch: binding_epochs.get(&svc.member_ref()).copied().unwrap_or(0),
                        cache_ttl_ms: DEFAULT_BINDING_CACHE_TTL_MS,
                    })
                    .collect()
            } else {
                Vec::new()
            },
            // ADR-0021 §4 (M05A A5a): 0 for every caller through A5a --
            // the supervisor that presents a real, `adopt`-minted
            // generation does not exist yet.
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
mod tests {
    use std::collections::BTreeMap;

    use semver::Version;
    use syneroym_app_orchestration::models::{
        AppBlueprintId, AppInstanceId, FdaeManifest, HttpProbe, InterfaceName, LogicalServiceName,
        LogicalServiceRef, PlannedService, RpcProbe, ServiceConfig, ServiceId, ServiceType,
        TcpProbe, TopologyMode,
    };

    use super::*;

    fn base_config() -> ServiceConfig {
        ServiceConfig {
            service_type: ServiceType::Tcp,
            source: "127.0.0.1:9000".to_string(),
            hash: None,
            interfaces: vec![],
            env: BTreeMap::new(),
            args: vec![],
            custom_config: None,
            quota: None,
            schema: None,
            rotation_policy: Default::default(),
            fdae: None,
            health_check: None,
            assets: None,
            visibility: syneroym_app_orchestration::Visibility::Private,
        }
    }

    fn plan_with_config(config: ServiceConfig) -> DeploymentPlan {
        DeploymentPlan {
            app_instance_id: AppInstanceId::new("inst-1"),
            blueprint_id: AppBlueprintId::new("syneroym:test-app"),
            version: Version::parse("0.1.0").unwrap(),
            services: vec![PlannedService {
                service_id: ServiceId::new("did:key:h123"),
                logical_ref: LogicalServiceRef {
                    app_instance_id: AppInstanceId::new("inst-1"),
                    service_name: LogicalServiceName::new("svc"),
                },
                substrate: None,
                config,
                resolved_dependencies: BTreeMap::new(),
                topology_mode: TopologyMode::Singleton,
                member_index: 0,
                schedule: None,
                sharding_strategy: None,
                topology_visibility: syneroym_app_orchestration::TopologyVisibility::Restricted,
            }],
        }
    }

    /// Whole-plan mapping, which is what every test but the subset one wants.
    fn map_all(
        plan: &DeploymentPlan,
        instance_certificates: &BTreeMap<ServiceId, String>,
        registry_certificates: &BTreeMap<ServiceId, String>,
        emit_bindings: bool,
    ) -> anyhow::Result<WitDeploymentPlan> {
        let all: Vec<&PlannedService> = plan.services.iter().collect();
        // Unmanaged (M05A A5a): none of these tests exercise the
        // generation gate, which is `map_deployment_plan_to_wit`'s own
        // concern to unit-test. Epoch defaults to empty (every binding maps
        // at 0) for the same reason -- the epoch map is its own test's
        // concern.
        map_deployment_plan_to_wit(
            plan,
            &all,
            instance_certificates,
            registry_certificates,
            emit_bindings,
            0,
            &BTreeMap::new(),
        )
    }

    /// The point of the whole change: a bare manifest path is resolved here,
    /// on the client, so the deploy call carries the document and the
    /// substrate needs nothing pre-staged.
    #[test]
    fn local_document_ref_is_read_and_shipped_inline() {
        let dir = tempfile::tempdir().unwrap();
        let policy = dir.path().join("fdae-policy.json");
        std::fs::write(&policy, r#"{"version":"fdae/v1"}"#).unwrap();

        let mut config = base_config();
        config.fdae = Some(FdaeManifest {
            policy: DocumentRef::Local(policy.to_string_lossy().into_owned()),
        });

        let wit_plan =
            map_all(&plan_with_config(config), &BTreeMap::new(), &BTreeMap::new(), true).unwrap();
        match &wit_plan.services[0].manifest.config.fdae_policy {
            Some(DocumentSource::Inline(content)) => {
                assert_eq!(content, r#"{"version":"fdae/v1"}"#);
            }
            other => panic!("expected inline content, got {other:?}"),
        }
    }

    #[test]
    fn remote_document_ref_passes_through_for_the_substrate_to_resolve() {
        let mut config = base_config();
        config.fdae = Some(FdaeManifest {
            policy: DocumentRef::Remote { remote_path: "policies/shared.json".to_string() },
        });

        let wit_plan =
            map_all(&plan_with_config(config), &BTreeMap::new(), &BTreeMap::new(), true).unwrap();
        match &wit_plan.services[0].manifest.config.fdae_policy {
            Some(DocumentSource::Path(path)) => assert_eq!(path, "policies/shared.json"),
            other => panic!("expected a host path, got {other:?}"),
        }
    }

    #[test]
    fn local_document_ref_missing_file_fails_the_deploy() {
        let mut config = base_config();
        config.fdae =
            Some(FdaeManifest { policy: DocumentRef::Local("does-not-exist.json".to_string()) });

        assert!(
            map_all(&plan_with_config(config), &BTreeMap::new(), &BTreeMap::new(), true,).is_err()
        );
    }

    #[test]
    fn map_deployment_plan_to_wit_maps_absent_fdae_to_none() {
        let wit_plan =
            map_all(&plan_with_config(base_config()), &BTreeMap::new(), &BTreeMap::new(), true)
                .unwrap();
        assert!(wit_plan.services[0].manifest.config.fdae_policy.is_none());
    }

    #[test]
    fn no_health_check_maps_to_none() {
        let wit_plan =
            map_all(&plan_with_config(base_config()), &BTreeMap::new(), &BTreeMap::new(), true)
                .unwrap();
        assert!(wit_plan.services[0].manifest.config.health_check.is_none());
    }

    /// A TCP service with no declared interfaces gets `DEFAULT_INTERFACE_NAME`
    /// -- the one name this mapper and `roymctl svc deploy`'s own
    /// `--interfaces` fallback now share, rather than each independently
    /// picking a different string.
    #[test]
    fn a_tcp_service_with_no_declared_interfaces_gets_the_shared_default_name() {
        let wit_plan =
            map_all(&plan_with_config(base_config()), &BTreeMap::new(), &BTreeMap::new(), true)
                .unwrap();
        match &wit_plan.services[0].manifest.service_type {
            WitServiceType::Tcp(m) => {
                assert_eq!(m.endpoints.len(), 1);
                assert_eq!(m.endpoints[0].interface_name, DEFAULT_INTERFACE_NAME);
            }
            other => panic!("expected a TCP manifest, got {other:?}"),
        }
    }

    /// An explicitly declared interface is used verbatim, never overridden
    /// by the default.
    #[test]
    fn a_tcp_services_declared_interface_name_is_used_verbatim() {
        let mut config = base_config();
        config.interfaces = vec![InterfaceName::new("admin")];
        let wit_plan =
            map_all(&plan_with_config(config), &BTreeMap::new(), &BTreeMap::new(), true).unwrap();
        match &wit_plan.services[0].manifest.service_type {
            WitServiceType::Tcp(m) => {
                assert_eq!(m.endpoints.len(), 1);
                assert_eq!(m.endpoints[0].interface_name, "admin");
            }
            other => panic!("expected a TCP manifest, got {other:?}"),
        }
    }

    #[test]
    fn a_health_check_maps_onto_the_wire() {
        let mut tcp_config = base_config();
        tcp_config.health_check = Some(HealthCheck::TcpConnect(TcpProbe {
            interface: InterfaceName::new("main"),
            timeout_ms: 1234,
        }));
        let wit_plan =
            map_all(&plan_with_config(tcp_config), &BTreeMap::new(), &BTreeMap::new(), true)
                .unwrap();
        match wit_plan.services[0].manifest.config.health_check.as_ref().unwrap() {
            WitHealthCheck::TcpConnect(p) => {
                assert_eq!(p.interface_name, "main");
                assert_eq!(p.timeout_ms, 1234);
            }
            other => panic!("expected TcpConnect, got {other:?}"),
        }

        let mut http_config = base_config();
        http_config.health_check = Some(HealthCheck::HttpGet(HttpProbe {
            interface: InterfaceName::new("http"),
            path: "/healthz".to_string(),
            expect_status: 204,
            timeout_ms: 1500,
        }));
        let wit_plan =
            map_all(&plan_with_config(http_config), &BTreeMap::new(), &BTreeMap::new(), true)
                .unwrap();
        match wit_plan.services[0].manifest.config.health_check.as_ref().unwrap() {
            WitHealthCheck::HttpGet(p) => {
                assert_eq!(p.interface_name, "http");
                assert_eq!(p.path, "/healthz");
                assert_eq!(p.expect_status, 204);
                assert_eq!(p.timeout_ms, 1500);
            }
            other => panic!("expected HttpGet, got {other:?}"),
        }

        let mut rpc_config = base_config();
        rpc_config.health_check = Some(HealthCheck::Rpc(RpcProbe {
            interface: InterfaceName::new("rpc"),
            method: "ping".to_string(),
            timeout_ms: 2000,
        }));
        let wit_plan =
            map_all(&plan_with_config(rpc_config), &BTreeMap::new(), &BTreeMap::new(), true)
                .unwrap();
        match wit_plan.services[0].manifest.config.health_check.as_ref().unwrap() {
            WitHealthCheck::Rpc(p) => {
                assert_eq!(p.interface_name, "rpc");
                assert_eq!(p.method, "ping");
                assert_eq!(p.timeout_ms, 2000);
            }
            other => panic!("expected Rpc, got {other:?}"),
        }
    }

    fn container_config(custom: &str) -> ServiceConfig {
        let mut config = base_config();
        config.service_type = ServiceType::Container;
        config.source = "docker.io/library/nginx:1.27".to_string();
        config.custom_config = Some(custom.to_string());
        config
    }

    fn container_manifest_of(plan: &WitDeploymentPlan) -> &ContainerManifest {
        match &plan.services[0].manifest.service_type {
            WitServiceType::Container(m) => m,
            other => panic!("expected a container manifest, got {other:?}"),
        }
    }

    /// Guards the field names the developer guide documents: a mismatch here
    /// would only surface at a live deploy.
    #[test]
    fn container_volume_files_are_parsed_and_inlined() {
        let dir = tempfile::tempdir().unwrap();
        let conf = dir.path().join("nginx.conf");
        std::fs::write(&conf, "server { listen 80; }").unwrap();

        let custom = format!(
            r#"{{"volumes":[{{"host_path":"conf","container_path":"/etc/nginx/conf.d",
                 "files":[{{"relative_path":"default.conf","content":{}}}]}}]}}"#,
            serde_json::to_string(&conf.to_string_lossy().into_owned()).unwrap()
        );

        let wit_plan = map_all(
            &plan_with_config(container_config(&custom)),
            &BTreeMap::new(),
            &BTreeMap::new(),
            true,
        )
        .expect("volumes should parse");
        let volumes = &container_manifest_of(&wit_plan).volumes;

        assert_eq!(volumes.len(), 1);
        assert_eq!(volumes[0].host_path, "conf");
        assert_eq!(volumes[0].container_path, "/etc/nginx/conf.d");
        assert_eq!(volumes[0].files.len(), 1);
        assert_eq!(volumes[0].files[0].relative_path, "default.conf");
        match &volumes[0].files[0].content {
            DocumentSource::Inline(c) => assert_eq!(c, "server { listen 80; }"),
            other => panic!("expected inline content, got {other:?}"),
        }
    }

    /// A volume that only wants an empty directory stays as terse as it was
    /// before `files` existed.
    #[test]
    fn container_volume_without_files_still_parses() {
        let custom = r#"{"volumes":[{"host_path":"data","container_path":"/data"}]}"#;
        let wit_plan = map_all(
            &plan_with_config(container_config(custom)),
            &BTreeMap::new(),
            &BTreeMap::new(),
            true,
        )
        .unwrap();
        let volumes = &container_manifest_of(&wit_plan).volumes;

        assert_eq!(volumes.len(), 1);
        assert!(volumes[0].files.is_empty());
    }

    /// Both sibling keys fail loudly. Silently dropping either one deploys a
    /// container that is broken in a way nothing reports.
    #[test]
    fn malformed_volumes_and_ports_both_fail_the_deploy() {
        let bad_volumes = r#"{"volumes":[{"host_path":"data"}]}"#;
        let err = map_all(
            &plan_with_config(container_config(bad_volumes)),
            &BTreeMap::new(),
            &BTreeMap::new(),
            true,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("invalid container volumes"), "{err}");

        let bad_ports = r#"{"ports":[{"interface_name":"default","port":80,"protocol":"tcp"}]}"#;
        let err = map_all(
            &plan_with_config(container_config(bad_ports)),
            &BTreeMap::new(),
            &BTreeMap::new(),
            true,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("invalid container ports"), "{err}");
    }

    #[test]
    fn oversize_local_document_fails_before_the_deploy_call() {
        let dir = tempfile::tempdir().unwrap();
        let big = dir.path().join("big-policy.json");
        std::fs::write(&big, "x".repeat(deploy_docs::MAX_DEPLOY_DOCUMENT_BYTES as usize + 1))
            .unwrap();

        let mut config = base_config();
        config.fdae =
            Some(FdaeManifest { policy: DocumentRef::Local(big.to_string_lossy().into_owned()) });

        let err = map_all(&plan_with_config(config), &BTreeMap::new(), &BTreeMap::new(), true)
            .unwrap_err()
            .to_string();
        assert!(err.contains("exceeding the"), "{err}");
    }

    /// A plan with `frontend` depending on `backend`, `backend` deployed
    /// `Redundant` with two members -- so a binding assertion exercises
    /// both "one binding per `depends_on` entry" and "the mode is the
    /// *target's* own topology mode, not the dependent's" (§3.3's landmine:
    /// every service in this fixture is otherwise `Singleton`).
    fn plan_with_a_dependency() -> DeploymentPlan {
        let app_instance_id = AppInstanceId::new("inst-1");
        let backend_ref = LogicalServiceRef {
            app_instance_id: app_instance_id.clone(),
            service_name: LogicalServiceName::new("backend"),
        };
        let frontend_ref = LogicalServiceRef {
            app_instance_id: app_instance_id.clone(),
            service_name: LogicalServiceName::new("frontend"),
        };
        DeploymentPlan {
            app_instance_id,
            blueprint_id: AppBlueprintId::new("syneroym:test-app"),
            version: Version::parse("0.1.0").unwrap(),
            services: vec![
                PlannedService {
                    service_id: ServiceId::new("did:key:hBackend"),
                    logical_ref: backend_ref,
                    substrate: None,
                    config: base_config(),
                    resolved_dependencies: BTreeMap::new(),
                    topology_mode: TopologyMode::Redundant,
                    member_index: 0,
                    schedule: None,
                    sharding_strategy: None,
                    topology_visibility: syneroym_app_orchestration::TopologyVisibility::Restricted,
                },
                PlannedService {
                    service_id: ServiceId::new("did:key:hFrontend"),
                    logical_ref: frontend_ref,
                    substrate: None,
                    config: base_config(),
                    resolved_dependencies: BTreeMap::from([(
                        LogicalServiceName::new("backend"),
                        vec![
                            ServiceId::new("did:key:hBackendMember1"),
                            ServiceId::new("did:key:hBackendMember2"),
                        ],
                    )]),
                    topology_mode: TopologyMode::Singleton,
                    member_index: 0,
                    schedule: None,
                    sharding_strategy: None,
                    topology_visibility: syneroym_app_orchestration::TopologyVisibility::Restricted,
                },
            ],
        }
    }

    #[test]
    fn the_app_context_carries_one_binding_per_depends_on_entry_with_the_targets_mode() {
        let wit_plan =
            map_all(&plan_with_a_dependency(), &BTreeMap::new(), &BTreeMap::new(), true).unwrap();

        let frontend =
            wit_plan.services.iter().find(|s| s.logical_ref.ends_with("frontend")).unwrap();
        let ctx = frontend.app_context.as_ref().expect("frontend has an app context");
        assert_eq!(ctx.app_instance_id, "inst-1");
        assert_eq!(ctx.service_name, "frontend");
        assert_eq!(ctx.bindings.len(), 1);
        let binding = &ctx.bindings[0];
        assert_eq!(binding.dependency_name, "backend");
        assert_eq!(binding.app_instance_id, "inst-1");
        assert!(
            matches!(binding.mode, WitTopologyMode::Redundant),
            "the binding's mode must be the *target's* topology mode, not the dependent's -- \
             backend is Redundant, frontend (the dependent) is Singleton"
        );
        assert_eq!(binding.members, vec!["did:key:hBackendMember1", "did:key:hBackendMember2"]);

        let backend =
            wit_plan.services.iter().find(|s| s.logical_ref.ends_with("backend")).unwrap();
        assert!(
            backend.app_context.as_ref().unwrap().bindings.is_empty(),
            "a service with no depends_on entry gets an empty binding list, not one for itself"
        );
    }

    /// This is the latent bug §5.1 exists to fix: `backend`'s topology mode
    /// must come from the *whole* plan, not from the subset being mapped.
    /// Mapping only `frontend` (as A3's per-substrate deploy call does when
    /// `backend` is placed elsewhere) must still emit `backend`'s real mode
    /// on the binding -- a naive "filter the plan, then map" shape would
    /// silently default it to `Singleton` since `backend` itself is absent
    /// from the subset.
    #[test]
    fn mapping_one_service_resolves_a_dependencys_mode_from_the_whole_plan() {
        let plan = plan_with_a_dependency();
        let frontend_only: Vec<&PlannedService> = plan
            .services
            .iter()
            .filter(|s| s.logical_ref.service_name.as_str() == "frontend")
            .collect();

        let wit_plan = map_deployment_plan_to_wit(
            &plan,
            &frontend_only,
            &BTreeMap::new(),
            &BTreeMap::new(),
            true,
            0,
            &BTreeMap::new(),
        )
        .unwrap();

        assert_eq!(wit_plan.services.len(), 1);
        let ctx = wit_plan.services[0].app_context.as_ref().unwrap();
        let binding = &ctx.bindings[0];
        assert!(
            matches!(binding.mode, WitTopologyMode::Redundant),
            "backend's mode must be resolved from the whole plan even though only frontend was \
             mapped"
        );
    }

    /// M05A A5c §19.3/D-A5c-4: the epoch is keyed by the *dependent*
    /// member's own ref, not by the dependency name -- frontend's one entry
    /// in the map must land on every one of frontend's bindings.
    #[test]
    fn a_plan_mapped_at_a_nonzero_epoch_emits_that_epoch_on_every_binding() {
        let plan = plan_with_a_dependency();
        let frontend_member_ref = plan.services[1].member_ref();
        let epochs = BTreeMap::from([(frontend_member_ref, 7u64)]);

        let all: Vec<&PlannedService> = plan.services.iter().collect();
        let wit_plan = map_deployment_plan_to_wit(
            &plan,
            &all,
            &BTreeMap::new(),
            &BTreeMap::new(),
            true,
            0,
            &epochs,
        )
        .unwrap();

        let frontend =
            wit_plan.services.iter().find(|s| s.logical_ref.ends_with("frontend")).unwrap();
        let ctx = frontend.app_context.as_ref().unwrap();
        assert_eq!(ctx.bindings.len(), 1);
        assert_eq!(ctx.bindings[0].epoch, 7);

        // backend has no entry in the map, so it must fall back to 0 --
        // meaning "no supervisor has written here" -- rather than
        // inheriting frontend's value or panicking on a missing key.
        let backend =
            wit_plan.services.iter().find(|s| s.logical_ref.ends_with("backend")).unwrap();
        assert!(backend.app_context.as_ref().unwrap().bindings.is_empty());
    }

    #[test]
    fn a_plan_with_no_dependencies_emits_an_empty_binding_list() {
        let wit_plan =
            map_all(&plan_with_config(base_config()), &BTreeMap::new(), &BTreeMap::new(), true)
                .unwrap();
        assert!(wit_plan.services[0].app_context.as_ref().unwrap().bindings.is_empty());
    }

    /// D-A2-16: without `--mint-masters`, `resolved_dependencies` still
    /// holds the compiler's fabricated `did:key:h...` ids, which are not
    /// real keys. Publishing them would let `dependency(...)` resolve and
    /// then fail one layer down as `service-not-found`, destroying the
    /// distinction `dependency-not-bound` exists to draw -- so
    /// `emit_bindings: false` must publish no bindings at all, not the
    /// fabricated ones.
    #[test]
    fn emit_bindings_false_publishes_no_fabricated_member_dids() {
        let wit_plan =
            map_all(&plan_with_a_dependency(), &BTreeMap::new(), &BTreeMap::new(), false).unwrap();

        for svc in &wit_plan.services {
            assert!(
                svc.app_context.as_ref().unwrap().bindings.is_empty(),
                "emit_bindings: false must publish no bindings for '{}', fabricated or otherwise",
                svc.logical_ref
            );
        }
    }
}
