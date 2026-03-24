//! Observability component for metrics, logging, and tracing.

use anyhow::{Context, Result};
use std::fs;
use syneroym_core::config::{LogFormat, LogLevel, LogTarget, SubstrateConfig};
use tracing::info;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

pub struct ObservabilityEngine {
    log_guard: Option<WorkerGuard>,
}

impl ObservabilityEngine {
    /// Initializes global tracing subscribers and metrics registries.
    pub fn init(config: &SubstrateConfig) -> Result<Self> {
        let filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new(default_directive(&config.logging.level)));

        let log_guard = match config.logging.target {
            LogTarget::Stdout => {
                let subscriber =
                    tracing_subscriber::registry().with(filter).with(stdout_layer(config));
                subscriber.try_init().context("failed to initialize stdout tracing subscriber")?;
                None
            }
            LogTarget::File => {
                fs::create_dir_all(&config.app_log_dir).with_context(|| {
                    format!("failed to create log directory at {}", config.app_log_dir.display())
                })?;

                let file_appender =
                    tracing_appender::rolling::daily(&config.app_log_dir, "syneroym.log");
                let (writer, guard) = tracing_appender::non_blocking(file_appender);
                let subscriber =
                    tracing_subscriber::registry().with(filter).with(file_layer(config, writer));
                subscriber.try_init().context("failed to initialize file tracing subscriber")?;
                Some(guard)
            }
        };

        info!(
            level = %default_directive(&config.logging.level),
            format = ?config.logging.format,
            target = ?config.logging.target,
            log_dir = %config.app_log_dir.display(),
            "observability initialized"
        );

        Ok(Self { log_guard })
    }

    /// Flushes remaining telemetry data before the application exits.
    pub async fn shutdown(&self) -> Result<()> {
        let _ = &self.log_guard;
        info!("flushing observability data");
        Ok(())
    }
}

fn default_directive(level: &LogLevel) -> &'static str {
    match level {
        LogLevel::Error => "error",
        LogLevel::Warn => "warn",
        LogLevel::Info => "info",
        LogLevel::Debug => "debug",
        LogLevel::Trace => "trace",
    }
}

fn stdout_layer<S>(config: &SubstrateConfig) -> Box<dyn tracing_subscriber::Layer<S> + Send + Sync>
where
    S: tracing::Subscriber + for<'span> tracing_subscriber::registry::LookupSpan<'span>,
{
    match config.logging.format {
        LogFormat::Json => Box::new(fmt::layer().json()),
        LogFormat::Pretty => Box::new(fmt::layer().pretty()),
    }
}

fn file_layer<S>(
    config: &SubstrateConfig,
    writer: tracing_appender::non_blocking::NonBlocking,
) -> Box<dyn tracing_subscriber::Layer<S> + Send + Sync>
where
    S: tracing::Subscriber + for<'span> tracing_subscriber::registry::LookupSpan<'span>,
{
    match config.logging.format {
        LogFormat::Json => Box::new(fmt::layer().json().with_writer(writer)),
        LogFormat::Pretty => Box::new(fmt::layer().pretty().with_writer(writer)),
    }
}
