//! ReBAC -> SQL compiler (ADR-0017 §3-§4): compiles a parsed [`Policy`] and a
//! caller's `SessionContext` into a parameterized `WHERE EXISTS`/`WITH
//! RECURSIVE` row-security block (RLS) plus CLS field-masking metadata.

pub mod emit;
pub mod plan;
pub mod types;

#[cfg(test)]
mod tests;

pub use emit::{definition_has_abac, definition_table, resolve_structural};
pub use plan::{compile_read, finalize, plan_read};
pub use types::{
    CompiledSieve, FetchResult, FetchSlot, MAX_FETCH_IDS, Mode, PendingSieve, ReadPlan,
    RemoteFetch, StructuralQuery,
};
