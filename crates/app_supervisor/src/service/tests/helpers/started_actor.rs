//! A scheduled-run actor that checks the watermark from inside the call.

use syneroym_sdk::{BindingWrite, SubstrateActor};

use super::super::super::*;

/// A direct proof of the watermark's ordering: `record_schedule_started`
/// must have already landed by the time the target is called, not after --
/// checked from *inside* the call itself, before its own outcome is
/// known.
#[derive(Debug)]
pub(in crate::service::tests) struct AssertsStartedBeforeCallActor {
    pub(in crate::service::tests) store: SupervisorStore,
    pub(in crate::service::tests) expected_run_at: i64,
}

#[async_trait::async_trait]
impl SubstrateActor for AssertsStartedBeforeCallActor {
    async fn apply_plan(&self, _plan: syneroym_sdk::DeploymentPlan) -> Result<(), String> {
        unimplemented!("not exercised by this test")
    }
    async fn write_bindings(
        &self,
        _write: BindingWrite,
    ) -> Result<Vec<BindingWriteOutcome>, String> {
        unimplemented!("not exercised by this test")
    }
    async fn restart(&self, _service_id: String, _generation: u64) -> Result<(), String> {
        unimplemented!("not exercised by this test")
    }
    async fn renew_cert(
        &self,
        _service_id: String,
        _generation: u64,
        _instance_certificate: String,
    ) -> Result<(), String> {
        unimplemented!("not exercised by this test")
    }
    async fn instance_identity(
        &self,
        _service_id: &str,
    ) -> Result<syneroym_sdk::InstanceIdentity, String> {
        unimplemented!("not exercised by this test")
    }
    async fn held_generation(&self, _app_instance_id: &str) -> Result<Option<u64>, String> {
        unimplemented!("not exercised by this test")
    }
    async fn run_scheduled(
        &self,
        _service_id: String,
        _generation: u64,
        _interface: String,
        _method: String,
        _params_json: Option<String>,
    ) -> Result<(), String> {
        let states = self.store.schedule_states("inst-1").unwrap();
        let state = states
            .get("inst-1/worker")
            .expect("record_schedule_started must have written before the call");
        assert_eq!(state.last_run_at, Some(self.expected_run_at));
        Ok(())
    }
}
