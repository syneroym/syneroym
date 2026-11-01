use std::time::{Duration, Instant};

use wasmtime::{Store, UpdateDeadline};

use super::EPOCH_TICK_MS;
use crate::host_capabilities::{HostState, WaitCredit};

/// Tracks wall-clock execution time and credited wait time for a guest call,
/// determining whether an epoch deadline callback should extend the deadline
/// or trigger an interruption trap.
pub(crate) struct EpochBudget {
    started: Instant,
    budget: Duration,
    credited: Duration,
    #[allow(dead_code)]
    credit_cap: Duration,
}

impl EpochBudget {
    /// Creates a new budget with the specified maximum credit cap.
    /// The budget remains unarmed until [`Self::arm`] is called.
    pub(crate) fn new(credit_cap: Duration) -> Self {
        Self {
            started: Instant::now(),
            budget: Duration::ZERO,
            credited: Duration::ZERO,
            credit_cap,
        }
    }

    /// Arms the budget at the start of a guest call with the baseline time
    /// and allocated epoch ticks.
    pub(crate) fn arm(&mut self, now: Instant, ticks: u64) {
        self.started = now;
        self.budget = Duration::from_millis(ticks * EPOCH_TICK_MS);
        self.credited = Duration::ZERO;
    }

    /// Computes the remaining credit allowance available for outbound batches
    /// during this guest call.
    #[allow(dead_code)]
    pub(crate) fn allowance(&self) -> Duration {
        self.credit_cap.saturating_sub(self.credited)
    }

    /// Credits the elapsed wait time of a finished outbound batch against
    /// this guest call's epoch deadline, capped by the remaining allowance.
    #[allow(dead_code)]
    pub(crate) fn credit(&mut self, token: WaitCredit) {
        let max_additional = self.allowance();
        let add = token.duration().min(max_additional);
        self.credited = self.credited.saturating_add(add);
    }

    /// Evaluates whether the guest should be interrupted or allowed to continue
    /// when guest execution reaches an epoch boundary.
    pub(crate) fn on_deadline(&self, now: Instant) -> UpdateDeadline {
        if self.credited.is_zero() {
            return UpdateDeadline::Interrupt;
        }
        let elapsed = now.saturating_duration_since(self.started);
        let own = elapsed.saturating_sub(self.credited);
        if own >= self.budget {
            return UpdateDeadline::Interrupt;
        }
        let tick_nanos = Duration::from_millis(EPOCH_TICK_MS).as_nanos();
        let remaining_nanos = (self.budget - own).as_nanos();
        let ticks = remaining_nanos.div_ceil(tick_nanos).max(1);
        UpdateDeadline::Continue(ticks as u64)
    }
}

/// Arms the store's epoch deadline and initializes the host state's budget.
/// This is the sole mechanism used by production code to configure epoch
/// deadlines.
pub(crate) fn arm_epoch_deadline(store: &mut Store<HostState>, ticks: u64) {
    store.data_mut().epoch.arm(Instant::now(), ticks);
    store.set_epoch_deadline(ticks);
}

#[cfg(test)]
mod tests;
