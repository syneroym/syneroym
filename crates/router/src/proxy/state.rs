use super::{saga::rpc_saga_info_from, *};

/// The two durable per-service stores the operator verbs read, behind one
/// handle: a service's durable proxy state is one question to an operator,
/// not two.
pub struct ProxyState {
    pub(super) outbox: Arc<ProxyOutbox>,
    pub(super) sagas: Arc<SagaStore>,
}

impl std::fmt::Debug for ProxyState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxyState").finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl ProxyQueueInspector for ProxyState {
    async fn queued_calls(&self, service_id: &str) -> Result<Vec<QueuedCallInfo>, String> {
        self.outbox.queued_calls(service_id).await
    }

    async fn dead_letters(&self, service_id: &str) -> Result<Vec<DeadLetterInfo>, String> {
        self.outbox.dead_letters(service_id).await
    }

    async fn replay_dead_letter(&self, service_id: &str, id: u64) -> Result<(), String> {
        self.outbox.replay_dead_letter(service_id, id).await
    }

    async fn sagas(&self, service_id: &str) -> Result<Vec<SagaInfo>, String> {
        let Some(log) = self.sagas.existing_log_for(service_id).await.map_err(|e| e.to_string())?
        else {
            return Ok(Vec::new());
        };
        let items = task::spawn_blocking(move || log.list())
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        Ok(items.into_iter().map(rpc_saga_info_from).collect())
    }

    async fn rearm_saga(&self, service_id: &str, saga_id: &str) -> Result<(), String> {
        let Some(log) = self.sagas.existing_log_for(service_id).await.map_err(|e| e.to_string())?
        else {
            return Err(format!("service '{service_id}' has no durable saga log"));
        };
        let now = proxy_outbox::now_ms();
        let id = saga_id.to_string();
        let rearmed = task::spawn_blocking(move || log.rearm(&id, now))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        if rearmed {
            Ok(())
        } else {
            Err(format!(
                "saga '{saga_id}' is not failed -- only a failed saga can be re-armed to \
                 compensate"
            ))
        }
    }
}
