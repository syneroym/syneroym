//! Background maintenance worker for sweeps and record republishing.

use std::{sync::Arc, time::Duration};

use metrics::{counter, gauge};
use syneroym_core::dht_registry::RegistryClient;
use tokio::{
    select,
    sync::Notify,
    time::{self, MissedTickBehavior},
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::{error::MailboxError, store::MailboxStore};

#[cfg(test)]
mod tests;

/// Background loop that periodically sweeps expired resources and republishes
/// service records.
pub struct MailboxWorker {
    store: Arc<MailboxStore>,
    registry: Arc<RegistryClient>,
    wake: Arc<Notify>,
    sweep_every: Duration,
    republish_every_secs: u64,
}

impl std::fmt::Debug for MailboxWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MailboxWorker")
            .field("sweep_every", &self.sweep_every)
            .field("republish_every_secs", &self.republish_every_secs)
            .finish()
    }
}

impl MailboxWorker {
    pub(crate) fn new(
        store: Arc<MailboxStore>,
        registry: Arc<RegistryClient>,
        wake: Arc<Notify>,
        sweep_every: Duration,
        republish_every_secs: u64,
    ) -> Self {
        Self { store, registry, wake, sweep_every, republish_every_secs }
    }

    /// Runs the worker loop until cancelled.
    pub async fn run(self, cancel: CancellationToken) {
        let mut tick = time::interval(self.sweep_every);
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);

        loop {
            select! {
                () = cancel.cancelled() => return,
                _ = tick.tick() => {},
                () = self.wake.notified() => {}
            }

            let now = syneroym_passport::now_secs();
            match blocking(&self.store, move |s| s.sweep(now)).await {
                Ok(report) => {
                    if report.expired > 0 {
                        counter!("substrate.mailbox.expired").increment(u64::from(report.expired));
                    }
                    if report.over_claimed > 0 {
                        counter!("substrate.mailbox.closed_unacked")
                            .increment(u64::from(report.over_claimed));
                    }
                    if report.expired > 0 || report.over_claimed > 0 || report.queues_removed > 0 {
                        debug!(
                            expired = report.expired,
                            over_claimed = report.over_claimed,
                            queues_removed = report.queues_removed,
                            "mailbox sweep pass completed"
                        );
                    }
                }
                Err(e) => {
                    warn!(error = %e, "mailbox sweep failed");
                }
            }

            self.republish(now, &cancel).await;

            if let Ok(t) = blocking(&self.store, |s| s.totals()).await {
                gauge!("substrate.mailbox.queues").set(t.queues as f64);
                gauge!("substrate.mailbox.messages").set(t.messages as f64);
                gauge!("substrate.mailbox.bytes").set(t.bytes as f64);
            }
        }
    }

    async fn republish(&self, now: u64, cancel: &CancellationToken) {
        let due = match blocking(&self.store, move |s| s.records_due(now, 100)).await {
            Ok(d) => d,
            Err(e) => {
                warn!(error = %e, "failed to read due records for republish");
                return;
            }
        };

        for r in due {
            let reg_fut = self.registry.register(&r.record, false);
            let result = select! {
                () = cancel.cancelled() => return,
                res = reg_fut => res,
            };

            let next = if result.is_ok() {
                now.saturating_add(self.republish_every_secs)
            } else {
                counter!("substrate.mailbox.republish_failures").increment(1);
                now.saturating_add(300)
            };

            let service_id = r.service_id;
            if let Err(e) =
                blocking(&self.store, move |s| s.mark_published(&service_id, next)).await
            {
                warn!(error = %e, "failed to mark record publish deadline");
            }
        }
    }
}

async fn blocking<T: Send + 'static>(
    store: &Arc<MailboxStore>,
    f: impl FnOnce(&MailboxStore) -> Result<T, MailboxError> + Send + 'static,
) -> Result<T, MailboxError> {
    let s = store.clone();
    tokio::task::spawn_blocking(move || f(&s))
        .await
        .map_err(|e| MailboxError::Internal(format!("blocking worker task failed: {e}")))?
}
