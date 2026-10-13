//! Where the home relay of this substrate's Iroh endpoint comes from, and the
//! background task that sets it. Boot never waits for this task.

use std::{path::PathBuf, time::Duration};

use anyhow::Context;
use iroh::RelayUrl;
use syneroym_core::{config::SubstrateConfig, curated_list};
use syneroym_router::{HomeRelay, HomeRelayError, net_iroh};
use tokio::time::Instant;
use tracing::{debug, info, warn};

mod cache;
mod io;
mod resolver;

/// The file under `storage.db_dir` that keeps the coordinator list and the
/// choice across restarts.
const COORDINATOR_CACHE_FILE: &str = "coordinators.json";

/// A warning that repeats is logged at most this often.
const WARN_INTERVAL: Duration = Duration::from_secs(300);

/// The source of the home relay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum HomeRelaySource {
    /// `parent_coordinator.iroh.url` is set.
    Fixed { url: RelayUrl },
    /// `url` is not set and `substrate.registry_url` is: pick a relay from
    /// that registry's coordinator list.
    Registry {
        /// Normalised with `curated_list::normalize_url`.
        registry_url: String,
        cache_path: PathBuf,
    },
}

impl HomeRelaySource {
    /// The source for this config. `Ok(None)` means degraded mode with
    /// nothing to retry: no `url`, and no `substrate.registry_url` (or no
    /// Iroh endpoint at all, when the caller never uses the source).
    ///
    /// # Errors
    /// When `url` is set and not a valid relay URL, or when the registry
    /// source would be used and `substrate.registry_url` is not a valid
    /// registry URL. Startup stops.
    pub(super) fn from_config(config: &SubstrateConfig) -> anyhow::Result<Option<Self>> {
        // A bad `url` stops startup on every node.
        if let Some(url) = net_iroh::parent_relay_url(&config.parent_coordinator.iroh)? {
            return Ok(Some(Self::Fixed { url }));
        }
        // No Iroh endpoint means no home relay, so `registry_url` is not read
        // here: a WebRTC-only node keeps working with any value.
        let uses_iroh = config.substrate.communication_interfaces.iter().any(|c| c == "iroh");
        let Some(raw) = config.substrate.registry_url.as_deref().filter(|_| uses_iroh) else {
            return Ok(None);
        };
        let registry_url = curated_list::normalize_url(raw)
            .with_context(|| format!("substrate.registry_url = {raw:?}"))?;
        let cache_path = config.storage.db_dir.join(COORDINATOR_CACHE_FILE);
        Ok(Some(Self::Registry { registry_url, cache_path }))
    }
}

/// Connects to the home relay in the background, or logs the degraded
/// mode warning when there is no source. Never blocks.
///
/// `chooser` is the substrate DID: the registry list is ranked with it.
pub(super) fn connect_home_relay(
    home_relay: HomeRelay,
    source: Option<HomeRelaySource>,
    chooser: &str,
) {
    match source {
        None => warn!(
            "parent_coordinator.iroh.url and substrate.registry_url are not set: the Iroh \
             endpoint has no home relay (degraded mode). Local services and outbound calls work, \
             but no peer can reach this node through a relay, and its endpoint record is not \
             published. Set one of them and restart, or remove \"iroh\" from \
             substrate.communication_interfaces."
        ),
        Some(HomeRelaySource::Fixed { url }) => {
            tokio::spawn(run_fixed(home_relay, url));
        }
        Some(HomeRelaySource::Registry { registry_url, cache_path }) => {
            tokio::spawn(resolver::run(home_relay, registry_url, cache_path, chooser.to_owned()));
        }
    }
}

/// Warns the first time, then at most once every `WARN_INTERVAL`; the caller
/// logs at debug level in between.
#[derive(Debug, Default)]
pub(super) struct WarnLimiter {
    /// `tokio::time::Instant`, so paused-time tests drive it. Outside paused
    /// tests it reads the same clock as `std::time::Instant`.
    last: Option<Instant>,
}

impl WarnLimiter {
    /// True when this message should be a warning.
    pub(super) fn should_warn(&mut self) -> bool {
        let now = Instant::now();
        if self.last.is_none_or(|t| now.duration_since(t) >= WARN_INTERVAL) {
            self.last = Some(now);
            return true;
        }
        false
    }
}

/// Sets a fixed relay and keeps waiting until iroh confirms it.
///
/// No sleep between tries: after the first try the relay stays in the map,
/// and iroh runs a net report on its own every 20 to 26 s. So when a relay
/// that was down comes back, it is confirmed within about 30 s.
async fn run_fixed(home_relay: HomeRelay, url: RelayUrl) {
    let mut limiter = WarnLimiter::default();
    loop {
        match home_relay.set(url.clone()).await {
            Ok(()) => {
                info!(relay_url = %url, "home relay set");
                return;
            }
            Err(HomeRelayError::EndpointClosed) => return,
            Err(e @ HomeRelayError::NotConfirmed { .. }) => {
                if limiter.should_warn() {
                    warn!(error = %e, "home relay not confirmed yet; still waiting");
                } else {
                    debug!(error = %e, "home relay not confirmed yet; still waiting");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
