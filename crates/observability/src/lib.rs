#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Observability component for metrics, logging, and tracing.

mod engine;
pub mod metrics;
pub mod recorder;

pub use engine::ObservabilityEngine;
pub use recorder::{HistogramSnapshot, MemoryRecorder, MetricsSnapshot};
