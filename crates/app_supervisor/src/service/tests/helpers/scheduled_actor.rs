//! A scheduled-run actor for testing schedule executions.

use std::{sync::Mutex, time::Duration};

use syneroym_sdk::{BindingWrite, BindingWriteOutcome, SubstrateActor};

/// `Mutex` so a test can flip the outcome between two calls in the
/// same actor, the same shape `DurableTestActor` uses.
#[derive(Debug, Default)]
pub(in crate::service::tests) struct ScheduledActor {
    pub(in crate::service::tests) error: Mutex<Option<String>>,
    pub(in crate::service::tests) delay: Option<Duration>,
}

#[async_trait::async_trait]
impl SubstrateActor for ScheduledActor {
    async fn apply_plan(&self, _plan: syneroym_sdk::DeploymentPlan) -> Result<(), String> {
        unimplemented!("not exercised by schedule tests")
    }
    async fn write_bindings(
        &self,
        _write: BindingWrite,
    ) -> Result<Vec<BindingWriteOutcome>, String> {
        unimplemented!("not exercised by schedule tests")
    }
    async fn restart(&self, _service_id: String, _generation: u64) -> Result<(), String> {
        unimplemented!("not exercised by schedule tests")
    }
    async fn renew_cert(
        &self,
        _service_id: String,
        _generation: u64,
        _instance_certificate: String,
    ) -> Result<(), String> {
        unimplemented!("not exercised by schedule tests")
    }
    async fn instance_identity(
        &self,
        _service_id: &str,
    ) -> Result<syneroym_sdk::InstanceIdentity, String> {
        unimplemented!("not exercised by schedule tests")
    }
    async fn held_generation(&self, _app_instance_id: &str) -> Result<Option<u64>, String> {
        unimplemented!("not exercised by schedule tests")
    }
    async fn run_scheduled(
        &self,
        _service_id: String,
        _generation: u64,
        _interface: String,
        _method: String,
        _params_json: Option<String>,
    ) -> Result<(), String> {
        if let Some(delay) = self.delay {
            tokio::time::sleep(delay).await;
        }
        if let Some(e) = self.error.lock().unwrap().clone() {
            return Err(e);
        }
        Ok(())
    }
}
