//! Where the home relay of this substrate's Iroh endpoint comes from, and the
//! background task that sets it. Boot never waits for this task.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::Context;
use iroh::RelayUrl;
use syneroym_control_plane::HomeRelayControl;
use syneroym_core::{config::SubstrateConfig, curated_list, retry};
use syneroym_router::{HomeRelay, HomeRelayError, net_iroh};
use tokio::{sync::oneshot, time::Instant};
use tracing::{debug, info, warn};

mod cache;
mod io;
mod passport;
mod resolver;

#[cfg(test)]
mod test_logs;

/// The file under `storage.db_dir` that keeps the coordinator list and the
/// choice across restarts.
const COORDINATOR_CACHE_FILE: &str = "coordinators.json";

/// The file under `storage.db_dir` that keeps the home-relay passport.
const PASSPORT_FILE: &str = "home_relay_passport.json";

/// A warning that repeats is logged at most this often.
const WARN_INTERVAL: Duration = Duration::from_secs(300);

/// The source of the home relay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum HomeRelaySource {
    /// `parent_coordinator.iroh.url` is set.
    Fixed {
        url: RelayUrl,
        passport_path: PathBuf,
        /// This node also runs a coordinator Iroh role, which takes the same
        /// `url` as its parent relay.
        coordinator_on_parent: bool,
    },
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
            let passport_path = config.storage.db_dir.join(PASSPORT_FILE);
            let coordinator_on_parent =
                config.roles.coordinator.as_ref().is_some_and(|c| c.iroh.is_some());
            return Ok(Some(Self::Fixed { url, passport_path, coordinator_on_parent }));
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
/// Starts the home-relay task of `source`. Returns the passport control
/// when the source is `Fixed`; the caller wires it into the control plane.
pub(super) fn connect_home_relay(
    home_relay: HomeRelay,
    source: Option<HomeRelaySource>,
    node_did: &str,
    secret_key: [u8; 32],
) -> Option<Arc<dyn HomeRelayControl>> {
    match source {
        None => {
            warn!(
                "parent_coordinator.iroh.url and substrate.registry_url are not set: the Iroh \
                 endpoint has no home relay (degraded mode). Local services and outbound calls \
                 work, but no peer can reach this node through a relay, and its endpoint record \
                 is not published. Set one of them and restart, or remove \"iroh\" from \
                 substrate.communication_interfaces."
            );
            None
        }
        Some(HomeRelaySource::Fixed { url, passport_path, coordinator_on_parent }) => {
            let (boot_done_tx, boot_done_rx) = oneshot::channel();
            let control = passport::start(passport::Start {
                home_relay: home_relay.clone(),
                fixed_url: url.clone(),
                path: passport_path,
                node_did: node_did.to_owned(),
                secret_key,
                coordinator_on_parent,
                boot_done: boot_done_tx,
            });
            tokio::spawn(async move {
                // The agent registers before the relay goes into the map, so
                // iroh's reconnect backoff does not delay a granted lease. A
                // dropped sender (the agent ended) also releases the wait.
                let _ = boot_done_rx.await;
                run_fixed(home_relay, url).await;
            });
            Some(control)
        }
        Some(HomeRelaySource::Registry { registry_url, cache_path }) => {
            tokio::spawn(resolver::run(home_relay, registry_url, cache_path, node_did.to_owned()));
            None
        }
    }
}

/// A delay that doubles on every use, up to `max`, with ±10 % jitter.
#[derive(Debug)]
pub(super) struct Backoff {
    initial: Duration,
    max: Duration,
    next: Duration,
}

impl Backoff {
    pub(super) fn new(initial: Duration, max: Duration) -> Self {
        Self { initial, max, next: initial }
    }

    /// The delay to wait now. The one after it is twice as long.
    pub(super) fn next(&mut self) -> Duration {
        let millis = u64::try_from(self.next.as_millis()).unwrap_or(u64::MAX);
        let delay = Duration::from_millis(retry::calculate_jittered_backoff(millis));
        self.next = (self.next * 2).min(self.max);
        delay
    }

    pub(super) fn reset(&mut self) {
        self.next = self.initial;
    }
}

/// Writes atomic file with mode 0600 on Unix.
pub(super) fn write_file_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create directory {}", parent.display()))?;
    }
    let tmp = path.with_extension("tmp");
    let mut file = {
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        opts.open(&tmp).with_context(|| format!("cannot open {}", tmp.display()))?
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .with_context(|| format!("cannot set permissions on {}", tmp.display()))?;
    }
    file.write_all(bytes).with_context(|| format!("cannot write {}", tmp.display()))?;
    file.sync_all().with_context(|| format!("cannot sync {}", tmp.display()))?;
    drop(file);
    fs::rename(&tmp, path)
        .with_context(|| format!("cannot rename {} to {}", tmp.display(), path.display()))?;
    Ok(())
}

/// Reads HTTP response body up to `max` bytes.
pub(super) async fn read_capped(
    mut response: reqwest::Response,
    max: usize,
) -> anyhow::Result<Vec<u8>> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        anyhow::ensure!(body.len() + chunk.len() <= max, "the answer is longer than {max} bytes");
        body.extend_from_slice(&chunk);
    }
    Ok(body)
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
