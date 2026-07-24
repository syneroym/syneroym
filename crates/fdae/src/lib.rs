#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! FDAE (Federated Data-Aware Authorization Engine) policy model and
//! ReBAC -> SQL compiler (ADR-0017).

mod compile;
mod policy;
mod trace;

pub use compile::{
    CompiledSieve, FetchResult, FetchSlot, MAX_FETCH_IDS, Mode, PendingSieve, ReadPlan,
    RemoteFetch, StructuralQuery, compile_read, definition_table, finalize, plan_read,
    resolve_structural,
};
pub use policy::{
    CondOp, Condition, Definition, FieldsPolicy, Operator, Permission, Policy, PolicyError,
    Relation, parse_and_validate,
};
pub use trace::DecisionTrace;
