//! Where the home relay of this substrate's Iroh endpoint comes from, and the
//! background task that sets it. Boot never waits for this task.

use std::time::{Duration, Instant};

use iroh::RelayUrl;
use syneroym_core::config::IrohParentConfig;
use syneroym_router::{HomeRelay, HomeRelayError, net_iroh};
use tracing::{debug, info, warn};

const FIXED_WARN_INTERVAL: Duration = Duration::from_secs(300);

/// The source of the home relay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum HomeRelaySource {
    /// `parent_coordinator.iroh.url` is set.
    Fixed { url: RelayUrl },
}

impl HomeRelaySource {
    /// The source for this config. `Ok(None)` means degraded mode: there is
    /// no `url`.
    ///
    /// # Errors
    /// When `url` is set and not a valid relay URL. Startup stops.
    pub(super) fn from_config(config: &IrohParentConfig) -> anyhow::Result<Option<Self>> {
        Ok(net_iroh::parent_relay_url(config)?.map(|url| Self::Fixed { url }))
    }
}

/// Connects to the home relay in the background, or logs the degraded
/// mode warning when there is no source. Never blocks.
pub(super) fn connect_home_relay(home_relay: HomeRelay, source: Option<HomeRelaySource>) {
    let Some(source) = source else {
        warn!(
            "parent_coordinator.iroh.url is not set: the Iroh endpoint has no home relay \
             (degraded mode). Local services and outbound calls work, but no peer can reach this \
             node through a relay, and its endpoint record is not published. Set \
             parent_coordinator.iroh.url and restart, or remove \"iroh\" from \
             substrate.communication_interfaces."
        );
        return;
    };
    tokio::spawn(run_source(home_relay, source));
}

async fn run_source(home_relay: HomeRelay, source: HomeRelaySource) {
    match source {
        HomeRelaySource::Fixed { url } => run_fixed(&home_relay, url).await,
    }
}

/// Sets a fixed relay and keeps waiting until iroh confirms it.
///
/// No sleep between tries: after the first try the relay stays in the map,
/// and iroh runs a net report on its own every 20 to 26 s. So when a relay
/// that was down comes back, it is confirmed within about 30 s.
async fn run_fixed(home_relay: &HomeRelay, url: RelayUrl) {
    let mut last_warn: Option<Instant> = None;
    loop {
        match home_relay.set(url.clone()).await {
            Ok(()) => {
                info!(relay_url = %url, "home relay set");
                return;
            }
            Err(HomeRelayError::EndpointClosed) => return,
            Err(e @ HomeRelayError::NotConfirmed { .. }) => {
                // Warn on the first failure, then at most every 5 minutes.
                if last_warn.is_none_or(|t| t.elapsed() >= FIXED_WARN_INTERVAL) {
                    warn!(error = %e, "home relay not confirmed yet; still waiting");
                    last_warn = Some(Instant::now());
                } else {
                    debug!(error = %e, "home relay not confirmed yet; still waiting");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
