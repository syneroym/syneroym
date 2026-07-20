//! Per-request FDAE authorization context threaded into the read/delete
//! paths (ADR-0017, M04B Slice B2 Phase 2).

use syneroym_fdae::Policy;
use syneroym_ucan::SessionContext;

/// Per-request policy + caller context threaded into the read/delete paths.
/// `None` at a call site preserves today's unfiltered behavior
/// (policy-absent services, native dispatch, benches, tests).
#[derive(Debug)]
pub struct QueryAuth<'a> {
    pub policy: &'a Policy,
    pub session: &'a SessionContext,
    pub service_id: &'a str,
}

/// A read result plus the CLS field-mask the host must apply as its final
/// projection (host-side, Phase 3 -- this crate never strips fields itself,
/// per the stage-4 ordering contract). `masked_fields` is always empty on
/// the policy-absent path.
#[derive(Debug)]
pub struct ReadOutcome<T> {
    pub value: T,
    pub masked_fields: Vec<String>,
}
