use anyhow::{Result, anyhow};
use syneroym_core::config::{AppSandboxRole, RetryPolicy};

/// Ceiling on one step's stored params, and on the stored forward result. A
/// crate constant rather than a config field, matching this crate's own
/// `MAX_QUEUED_PAYLOAD_BYTES` in shape and in value -- it bounds a row this
/// crate writes, and no deployment has a reason to tune it.
pub const MAX_SAGA_PAYLOAD_BYTES: usize = 256 * 1024;

/// The retry curve plus this log's own bounds. Compensation delivery reuses
/// the sandbox role's `queue_*` retry budget rather than a second policy --
/// an undo is a delivery, and a second budget would only disagree with the
/// first.
#[derive(Debug, Clone)]
pub struct SagaConfig {
    pub retry: RetryPolicy,
    pub max_open: u32,
    pub max_steps: u32,
    pub max_terminal_rows: u32,
    pub default_deadline_ms: i64,
    pub max_deadline_ms: i64,
    /// One step's own call budget when the guest names none. Derived from
    /// `dispatch_epoch_timeout_secs`, never from the proxy's own 30 s
    /// default call timeout -- a step that outlives the guest's epoch traps
    /// the guest instead of returning an error its workflow logic can act
    /// on.
    pub step_timeout_ms: u64,
}

/// A step call budget one second below `dispatch_epoch_timeout_secs`,
/// clamped to at least this floor -- a slow log open must shorten the call
/// rather than leave nothing for it at all.
pub const MIN_STEP_CALL_BUDGET_MS: u64 = 500;

impl From<&AppSandboxRole> for SagaConfig {
    fn from(role: &AppSandboxRole) -> Self {
        let defaults = RetryPolicy::default();
        let step_timeout_ms = role
            .dispatch_epoch_timeout_secs
            .saturating_mul(1000)
            .saturating_sub(1000)
            .max(MIN_STEP_CALL_BUDGET_MS);
        Self {
            retry: RetryPolicy {
                max_attempts: role.queue_max_attempts.max(1),
                initial_backoff_ms: defaults.initial_backoff_ms,
                backoff_multiplier: defaults.backoff_multiplier,
                max_backoff_ms: role.queue_max_backoff_secs.saturating_mul(1000),
            },
            max_open: role.saga_max_open.max(1),
            max_steps: role.saga_max_steps.max(1),
            max_terminal_rows: role.saga_max_terminal_rows.max(1),
            default_deadline_ms: i64::try_from(
                role.saga_default_deadline_secs.saturating_mul(1000),
            )
            .unwrap_or(i64::MAX),
            max_deadline_ms: i64::try_from(role.saga_max_deadline_secs.saturating_mul(1000))
                .unwrap_or(i64::MAX),
            step_timeout_ms,
        }
    }
}

/// A saga's own lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SagaState {
    Open,
    Compensating,
    Compensated,
    Failed,
}

impl SagaState {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Compensating => "compensating",
            Self::Compensated => "compensated",
            Self::Failed => "failed",
        }
    }

    pub(crate) fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "open" => Self::Open,
            "compensating" => Self::Compensating,
            "compensated" => Self::Compensated,
            "failed" => Self::Failed,
            other => return Err(anyhow!("unknown saga state '{other}' in storage")),
        })
    }
}

/// One step's own lifecycle state, distinct from the saga's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepState {
    Pending,
    Done,
    Failed,
    Compensated,
}

impl StepState {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Compensated => "compensated",
        }
    }
}

/// What [`SagaLog::record_step_intent`] stores before the forward call is
/// dispatched.
#[derive(Debug, Clone)]
pub struct StepIntent {
    /// JSON `QueuedTarget`, opaque to this crate.
    pub target: String,
    pub routing_key: Option<String>,
    pub interface: String,
    pub method: String,
    pub params: Vec<u8>,
}

/// What the walk reads back for one step.
#[derive(Debug, Clone)]
pub struct StepRow {
    pub idx: u32,
    pub target: String,
    pub routing_key: Option<String>,
    pub interface: String,
    pub method: String,
    pub params: Vec<u8>,
    pub result: Option<Vec<u8>>,
    pub attempts: u32,
}

/// The operator/guest listing shape for one saga.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SagaInfo {
    pub saga_id: String,
    pub name: String,
    pub state: String,
    pub steps: u32,
    pub compensated_steps: u32,
    pub created_at: i64,
    pub deadline_at: i64,
    pub last_error: Option<String>,
}

/// Just enough to drive the sweep: which saga, and which app instance its
/// undos should carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SagaHead {
    pub saga_id: String,
    pub app_instance_id: Option<String>,
}

/// What a failed undo attempt means for the saga.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompensationOutcome {
    Retry { next_attempt_at: i64 },
    Failed,
}
