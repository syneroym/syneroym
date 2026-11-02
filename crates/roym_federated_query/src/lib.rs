//! Federated query library: fan-out, verification, and result merging across
//! sources.

use serde_json as _;
use syneroym_app_host as _;

pub mod merge;

#[cfg(test)]
mod tests;

pub use merge::{HitSource, MergedHit, MergedRows, RefusedHit, Row, merge_rows};
