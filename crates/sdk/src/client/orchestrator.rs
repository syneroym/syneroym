//! Orchestrator and administrative operations on SyneroymClient.

use anyhow::Result;
use syneroym_identity::DelegationCertificate;
use syneroym_rpc::{DeadLetterInfo, QueuedCallInfo, SagaInfo};
use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
    ArtifactSource, BindingWrite, ContainerManifest, ContainerPortMapping, ContainerVolumeMapping,
    DeployManifest, DeploymentPlan, InstanceIdentity, NetworkEndpoint, ServiceConfig, ServiceType,
    TcpManifest, WasmManifest,
};

use super::SyneroymClient;
use crate::types::{
    BindingWriteOutcome, DeploySvcOptions, DeployedService, NodeFacts, Publication,
    SigningIdentityInfo, SubstrateStatus,
};

impl SyneroymClient {
    pub async fn deploy_svc_wasm(
        &self,
        service_id: String,
        interfaces: Vec<String>,
        wasm_bytes: Vec<u8>,
        publication: Publication,
        instance_certificate: Option<DelegationCertificate>,
    ) -> Result<()> {
        self.deploy_svc_wasm_with_options(
            service_id,
            interfaces,
            wasm_bytes,
            DeploySvcOptions { publication, instance_certificate, ..Default::default() },
        )
        .await
    }

    /// [`deploy_svc_wasm`](Self::deploy_svc_wasm), plus everything optional
    /// about a WASM deploy: a static asset bundle and a `custom_config`
    /// JSON blob, whose reserved `http_routes` key declares a service's
    /// HTTP route table for a guest target. Replaces
    /// `deploy_svc_wasm_with_assets`, which had exactly one call site --
    /// a third optional field would have made the next one a fourth
    /// `deploy_svc_wasm_*` method instead of growing this one.
    pub async fn deploy_svc_wasm_with_options(
        &self,
        service_id: String,
        interfaces: Vec<String>,
        wasm_bytes: Vec<u8>,
        options: DeploySvcOptions,
    ) -> Result<()> {
        let (visibility, registry_certificate) = options.publication.split()?;
        let instance_certificate = options
            .instance_certificate
            .map(|c| c.to_json())
            .transpose()
            .map_err(|e| anyhow::anyhow!("Failed to serialize instance certificate: {e}"))?;
        let manifest = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: options.custom_config,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: None,
                health_check: None,
                assets: options.assets,
                visibility: Some(visibility),
            },
            service_type: ServiceType::Wasm(WasmManifest {
                source: ArtifactSource::Binary(wasm_bytes),
                hash: None,
                interfaces,
            }),
            registry_certificate,
            instance_certificate,
        };
        let params = serde_json::to_value((service_id, manifest))?;
        let res = self.request("orchestrator", "deploy", params).await?;
        if res.result == serde_json::json!({"status": "deployed"}) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("Deployment failed: {:?}", res.result))
        }
    }

    pub async fn deploy_svc_tcp(
        &self,
        service_id: String,
        endpoints: Vec<NetworkEndpoint>,
        publication: Publication,
        instance_certificate: Option<DelegationCertificate>,
    ) -> Result<()> {
        let (visibility, registry_certificate) = publication.split()?;
        let instance_certificate = instance_certificate
            .map(|c| c.to_json())
            .transpose()
            .map_err(|e| anyhow::anyhow!("Failed to serialize instance certificate: {e}"))?;
        let manifest = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: None,
                health_check: None,
                assets: None,
                visibility: Some(visibility),
            },
            service_type: ServiceType::Tcp(TcpManifest { endpoints }),
            registry_certificate,
            instance_certificate,
        };
        let params = serde_json::to_value((service_id, manifest))?;
        let res = self.request("orchestrator", "deploy", params).await?;
        if res.result == serde_json::json!({"status": "deployed"}) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("Deployment failed: {:?}", res.result))
        }
    }

    pub async fn deploy_container(
        &self,
        service_id: String,
        image: String,
        ports: Vec<ContainerPortMapping>,
        volumes: Vec<ContainerVolumeMapping>,
        publication: Publication,
        instance_certificate: Option<DelegationCertificate>,
    ) -> Result<()> {
        let (visibility, registry_certificate) = publication.split()?;
        let instance_certificate = instance_certificate
            .map(|c| c.to_json())
            .transpose()
            .map_err(|e| anyhow::anyhow!("Failed to serialize instance certificate: {e}"))?;
        let manifest = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: None,
                health_check: None,
                assets: None,
                visibility: Some(visibility),
            },
            service_type: ServiceType::Container(ContainerManifest {
                source: ArtifactSource::Binary(vec![]),
                hash: None,
                image,
                ports,
                volumes,
            }),
            registry_certificate,
            instance_certificate,
        };
        let params = serde_json::to_value((service_id, manifest))?;
        let res = self.request("orchestrator", "deploy", params).await?;
        if res.result == serde_json::json!({"status": "deployed"}) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("Deployment failed: {:?}", res.result))
        }
    }

    /// The instance signing key the substrate would derive for `service_id`
    /// under this client's own identity -- answerable before the service is
    /// deployed (ADR-0020 §3), which is what lets a member master be
    /// certified without the substrate ever holding it.
    pub async fn instance_identity(&self, service_id: &str) -> Result<InstanceIdentity> {
        let params = serde_json::to_value((service_id,))?;
        let res = self.request("orchestrator", "resolve-instance-identity", params).await?;
        serde_json::from_value(res.result)
            .map_err(|e| anyhow::anyhow!("Failed to parse instance identity response: {e}"))
    }

    pub async fn signing_identity(&self, service_id: &str) -> Result<SigningIdentityInfo> {
        let params = serde_json::to_value((service_id,))?;
        let res = self.request("signing", "identity", params).await?;
        serde_json::from_value(res.result)
            .map_err(|e| anyhow::anyhow!("Failed to parse signing identity response: {e}"))
    }

    pub async fn deploy_plan(&self, plan: DeploymentPlan) -> Result<()> {
        let params = serde_json::to_value((plan,))?;
        let res = self.request("orchestrator", "deploy-plan", params).await?;
        if res.result == serde_json::json!({"status": "deployed_plan"}) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("Deployment of plan failed: {:?}", res.result))
        }
    }

    /// `generation` is checked against the app instance's recorded
    /// management stamp when the service has one (ADR-0021 §4);
    /// send 0 for a standalone service.
    pub async fn undeploy(&self, service_id: String, generation: u64) -> Result<()> {
        let params = serde_json::to_value((service_id, generation))?;
        let res = self.request("orchestrator", "undeploy", params).await?;
        if res.result == serde_json::json!({"status": "undeployed"}) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("Undeployment failed: {:?}", res.result))
        }
    }

    /// Epoch-guarded binding write (ADR-0021 §3) -- the only
    /// path that changes a dependent's resolution without redeploying it.
    /// One outcome per binding, in the order sent.
    pub async fn write_bindings(&self, write: BindingWrite) -> Result<Vec<BindingWriteOutcome>> {
        let params = serde_json::to_value((write,))?;
        let res = self.request("orchestrator", "write-bindings", params).await?;
        Ok(serde_json::from_value(res.result)?)
    }

    /// Restart a deployed service in place, without reinstalling it.
    /// `generation` is checked against the app instance's recorded
    /// management stamp when the service has one; send 0 for a standalone
    /// service.
    pub async fn restart(&self, service_id: String, generation: u64) -> Result<()> {
        let params = serde_json::to_value((service_id, generation))?;
        let res = self.request("orchestrator", "restart", params).await?;
        if res.result == serde_json::json!({"status": "restarted"}) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("Restart failed: {:?}", res.result))
        }
    }

    /// Install a freshly-issued instance certificate on an already-deployed
    /// service, without reinstalling it -- the unattended-renewal path.
    /// `generation` follows `restart`'s rule; send 0 for a standalone
    /// service.
    pub async fn renew_cert(
        &self,
        service_id: String,
        generation: u64,
        instance_certificate: String,
    ) -> Result<()> {
        let params = serde_json::to_value((service_id, generation, instance_certificate))?;
        let res = self.request("orchestrator", "renew-cert", params).await?;
        if res.result == serde_json::json!({"status": "cert_renewed"}) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("Certificate renewal failed: {:?}", res.result))
        }
    }

    /// Clear an app instance's management stamp:
    /// `supervisor_did` back to `None`, `generation` back to 0. Without
    /// this, an adopted instance can never be hand-deployed again.
    pub async fn release_app_instance(
        &self,
        app_instance_id: String,
        generation: u64,
    ) -> Result<()> {
        let params = serde_json::to_value((app_instance_id, generation))?;
        let res = self.request("orchestrator", "release-app-instance", params).await?;
        if res.result == serde_json::json!({"status": "released"}) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("Releasing the app instance failed: {:?}", res.result))
        }
    }

    /// Every call waiting in `service_id`'s durable proxy outbox.
    pub async fn proxy_outbox(&self, service_id: String) -> Result<Vec<QueuedCallInfo>> {
        let res = self
            .request("orchestrator", "proxy-outbox", serde_json::to_value((service_id,))?)
            .await?;
        Ok(serde_json::from_value(res.result)?)
    }

    /// Every dead letter `service_id`'s durable proxy outbox holds.
    pub async fn proxy_dead_letters(&self, service_id: String) -> Result<Vec<DeadLetterInfo>> {
        let res = self
            .request("orchestrator", "proxy-dead-letters", serde_json::to_value((service_id,))?)
            .await?;
        Ok(serde_json::from_value(res.result)?)
    }

    /// Re-enqueues one dead letter. It never executes inline.
    pub async fn proxy_replay(&self, service_id: String, dead_letter_id: u64) -> Result<()> {
        let params = serde_json::to_value((service_id, dead_letter_id))?;
        let res = self.request("orchestrator", "proxy-replay", params).await?;
        if res.result == serde_json::json!({"status": "replayed"}) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("Replay failed: {:?}", res.result))
        }
    }

    /// Every saga `service_id`'s own log holds, oldest first.
    pub async fn sagas(&self, service_id: String) -> Result<Vec<SagaInfo>> {
        let res =
            self.request("orchestrator", "sagas", serde_json::to_value((service_id,))?).await?;
        Ok(serde_json::from_value(res.result)?)
    }

    /// Re-arms a `failed` saga back to `compensating`. It never walks
    /// inline.
    pub async fn saga_compensate(&self, service_id: String, saga_id: String) -> Result<()> {
        let params = serde_json::to_value((service_id, saga_id))?;
        let res = self.request("orchestrator", "saga-compensate", params).await?;
        if res.result == serde_json::json!({"status": "compensating"}) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("Saga compensate failed: {:?}", res.result))
        }
    }

    /// Forces this substrate to publish its own endpoint record and every
    /// hosted service's record to its community registry right now,
    /// instead of waiting for the next hourly heartbeat -- e.g. after a
    /// registry this node's records were wiped from (an in-memory registry
    /// that itself restarted) comes back up.
    pub async fn republish(&self) -> Result<()> {
        let res = self.request("orchestrator", "republish", serde_json::json!({})).await?;
        if res.result == serde_json::json!({"status": "republished"}) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("Republish failed: {:?}", res.result))
        }
    }

    pub async fn list_svcs(&self) -> Result<Vec<DeployedService>> {
        let res = self.request("orchestrator", "list", serde_json::json!({})).await?;
        let services: Vec<DeployedService> = serde_json::from_value(res.result)?;
        Ok(services)
    }

    /// A supervisor's poll: per-instance status for `service_ids`, or for
    /// every service this client may see when the list is empty.
    pub async fn status(&self, service_ids: Vec<String>) -> Result<SubstrateStatus> {
        let res = self
            .request("orchestrator", "status", serde_json::json!({ "service_ids": service_ids }))
            .await?;
        Ok(serde_json::from_value(res.result)?)
    }

    /// `status`'s `node` field alone, with none of `status`'s
    /// per-service work -- for a caller that wants only what this node is,
    /// not what is running on it (e.g. `app deploy`'s preflight).
    /// `None` for a caller without node-wide `orchestrator/status`,
    /// the same as `status`'s own `node` field.
    pub async fn node_facts(&self) -> Result<Option<NodeFacts>> {
        let res = self.request("orchestrator", "node-facts-only", serde_json::json!({})).await?;
        Ok(serde_json::from_value(res.result)?)
    }

    pub async fn inject_kek(&self, kek_hex: String) -> Result<()> {
        let params = serde_json::to_value((kek_hex,))?;
        let res = self.request("security", "inject-kek", params).await?;
        if res.result == serde_json::json!({"status": "injected"}) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("KEK injection failed: {:?}", res.result))
        }
    }

    pub async fn rotate_kek(&self, new_kek_hex: String) -> Result<()> {
        let params = serde_json::to_value((new_kek_hex,))?;
        let res = self.request("security", "rotate-kek", params).await?;
        if res.result == serde_json::json!({"status": "rotated"}) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("KEK rotation failed: {:?}", res.result))
        }
    }

    pub async fn set_secret(&self, service_id: String, key: String, value: Vec<u8>) -> Result<()> {
        let params = serde_json::to_value((service_id, key, value))?;
        let res = self.request("security", "set-secret", params).await?;
        if res.result == serde_json::json!({"status": "secret_set"}) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("Secret setting failed: {:?}", res.result))
        }
    }
}
