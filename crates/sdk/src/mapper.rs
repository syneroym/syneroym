use std::path::{Path, PathBuf};

use syneroym_app_orchestration::models::{
    DeploymentPlan, DocumentRef, RotationPolicy, ServiceType,
};
use syneroym_core::util;
use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
    ArtifactSource, ContainerManifest, ContainerPortMapping, ContainerVolumeFile,
    ContainerVolumeMapping, DeployManifest, DeploymentPlan as WitDeploymentPlan, DocumentSource,
    NetworkEndpoint, PlannedService, ResourceQuota, RotationPolicy as WitRotationPolicy,
    ServiceConfig as WitServiceConfig, ServiceType as WitServiceType, TcpManifest, WasmManifest,
};

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
            let content = String::from_utf8(bytes).map_err(|e| {
                anyhow::anyhow!("{field_name} at {path} is not valid UTF-8 text: {e}")
            })?;
            Ok(DocumentSource::Inline(content))
        }
        DocumentRef::Remote { remote_path } => Ok(DocumentSource::Path(remote_path.clone())),
    }
}

pub fn map_deployment_plan_to_wit(plan: DeploymentPlan) -> anyhow::Result<WitDeploymentPlan> {
    let mut services = Vec::new();
    for svc in plan.services {
        let wit_config = WitServiceConfig {
            env: svc.config.env.into_iter().collect(),
            args: svc.config.args,
            custom_config: svc.config.custom_config.clone(),
            quota: svc.config.quota.map(|q| ResourceQuota {
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
        };

        let service_type = match svc.config.service_type {
            ServiceType::Wasm => {
                let source = if svc.config.source.starts_with("http://")
                    || svc.config.source.starts_with("https://")
                {
                    ArtifactSource::Url(svc.config.source.clone())
                } else {
                    let path = PathBuf::from(&svc.config.source);
                    let bytes = util::read_local_artifact(&path)?;
                    ArtifactSource::Binary(bytes)
                };
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
                                    "main".to_string()
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
                    if let Some(p) = cfg.get("ports")
                        && let Ok(p_vec) =
                            serde_json::from_value::<Vec<ContainerPortMapping>>(p.clone())
                    {
                        ports = p_vec;
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
        services.push(PlannedService {
            service_id: svc.service_id.to_string(),
            logical_ref: svc.logical_ref.to_string(),
            manifest: DeployManifest {
                config: wit_config,
                service_type,
                registry_certificate: None,
            },
        });
    }

    Ok(WitDeploymentPlan {
        app_instance_id: plan.app_instance_id.to_string(),
        blueprint_id: plan.blueprint_id.to_string(),
        version: plan.version.to_string(),
        services,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use semver::Version;
    use syneroym_app_orchestration::models::{
        AppBlueprintId, AppInstanceId, FdaeManifest, LogicalServiceName, LogicalServiceRef,
        PlannedService, ServiceConfig, ServiceId, ServiceType, TopologyMode,
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
                config,
                resolved_dependencies: vec![],
                topology_mode: TopologyMode::Singleton,
            }],
        }
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

        let wit_plan = map_deployment_plan_to_wit(plan_with_config(config)).unwrap();
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

        let wit_plan = map_deployment_plan_to_wit(plan_with_config(config)).unwrap();
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

        assert!(map_deployment_plan_to_wit(plan_with_config(config)).is_err());
    }

    #[test]
    fn map_deployment_plan_to_wit_maps_absent_fdae_to_none() {
        let wit_plan = map_deployment_plan_to_wit(plan_with_config(base_config())).unwrap();
        assert!(wit_plan.services[0].manifest.config.fdae_policy.is_none());
    }
}
