//! Shared test fixtures for the `app` command tests, split by subcommand
//! into sibling files (each under the ~500-line test-module guideline).

use clap::Parser;
use syneroym_app_orchestration::models::{ServiceId, TopologyMode};
use syneroym_sdk::{
    BindingWrite, BindingWriteOutcome, DeploymentPlan as WitDeploymentPlan, deploy::SubstrateActor,
};

use super::*;

mod deploy;
mod health;
mod reconcile;
mod resolve;
mod shared;

#[derive(Parser)]
struct DummyCli {
    #[command(subcommand)]
    command: AppCommands,
}

// The placement-change refusal never calls the actor -- it only reads
// `DeployTarget`'s own fields -- so a fake that panics if ever invoked is
// enough to keep these tests free of any live substrate.
#[derive(Debug)]
struct NoopApplier;

#[async_trait::async_trait]
impl SubstrateActor for NoopApplier {
    async fn apply_plan(&self, _plan: WitDeploymentPlan) -> Result<(), String> {
        unimplemented!("check_no_placement_change must never call apply_plan()")
    }

    async fn write_bindings(
        &self,
        _write: BindingWrite,
    ) -> Result<Vec<BindingWriteOutcome>, String> {
        unimplemented!("check_no_placement_change must never call write_bindings()")
    }

    async fn restart(&self, _service_id: String, _generation: u64) -> Result<(), String> {
        unimplemented!("check_no_placement_change must never call restart()")
    }

    async fn renew_cert(
        &self,
        _service_id: String,
        _generation: u64,
        _instance_certificate: String,
    ) -> Result<(), String> {
        unimplemented!("check_no_placement_change must never call renew_cert()")
    }

    async fn instance_identity(
        &self,
        _service_id: &str,
    ) -> Result<syneroym_sdk::InstanceIdentity, String> {
        unimplemented!("check_no_placement_change must never call instance_identity()")
    }

    async fn held_generation(&self, _app_instance_id: &str) -> Result<Option<u64>, String> {
        unimplemented!("check_no_placement_change must never call held_generation()")
    }
}

fn dummy_config() -> ServiceConfig {
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
        visibility: Default::default(),
    }
}

fn planned_service(
    logical_ref: LogicalServiceRef,
    service_id: &str,
    alias: &str,
) -> PlannedService {
    PlannedService {
        service_id: ServiceId::new(service_id),
        logical_ref,
        substrate: Some(SubstrateAlias::new(alias)),
        config: dummy_config(),
        resolved_dependencies: BTreeMap::new(),
        topology_mode: TopologyMode::Singleton,
        member_index: 0,
        schedule: None,
        sharding_strategy: None,
        topology_visibility: Default::default(),
    }
}

fn deploy_target(did: &str, alias: &str) -> DeployTarget {
    DeployTarget {
        alias: Some(SubstrateAlias::new(alias)),
        substrate_did: did.to_string(),
        actor: Arc::new(NoopApplier),
    }
}

fn dummy_deployment_plan(instance_id: &AppInstanceId, svc: PlannedService) -> DeploymentPlan {
    DeploymentPlan {
        app_instance_id: instance_id.clone(),
        blueprint_id: AppBlueprintId::new("syneroym:test"),
        version: Version::new(1, 0, 0),
        services: vec![svc],
    }
}
