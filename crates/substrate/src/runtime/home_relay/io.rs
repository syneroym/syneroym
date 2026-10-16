//! Everything the resolver does outside itself. The resolver is timing-heavy
//! (60 s probes, 5-minute windows, backoff), so its unit tests replace this
//! trait with a fake and run on a paused clock.

use std::{
    future::Future,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use iroh::RelayUrl;
use reqwest::redirect;
use syneroym_core::{
    coordinator_info::{CoordinatorInfo, INFO_PATH},
    curated_list::{self, CoordinatorEntry, CoordinatorFilter, CoordinatorTransport, CuratedList},
    registry_http,
};
use syneroym_router::{HomeRelay, HomeRelayError};
use tokio::time;
use tracing::debug;

/// How long an info probe may take.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// The most bytes of a `/v1/info` answer that are read. The real answer is
/// well below 2 KiB; a coordinator operator must not make us read more.
const MAX_PROBE_BODY_BYTES: usize = 64 * 1024;

/// Waits between fetch tries. The first fetch can race the registry of the
/// same node, which binds its port after the resolver starts.
const FETCH_TRY_WAITS: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(2)];

pub(super) trait ResolverIo: Send + Sync + 'static {
    /// `GET <registry_url>/coordinators?transport=iroh-relay&hops=0`.
    fn fetch_list(
        &self,
        registry_url: &str,
    ) -> impl Future<Output = anyhow::Result<CuratedList<CoordinatorEntry>>> + Send;
    /// `GET <info_url>/v1/info`. `info_url` is normalised (no trailing slash).
    fn probe(&self, info_url: &str)
    -> impl Future<Output = anyhow::Result<CoordinatorInfo>> + Send;
    /// `HomeRelay::set`.
    fn set_relay(&self, url: RelayUrl) -> impl Future<Output = Result<(), HomeRelayError>> + Send;
    /// `HomeRelay::current_url`.
    fn current_relay(&self) -> Option<RelayUrl>;
    /// `HomeRelay::clear_unconfirmed`.
    fn clear_unconfirmed(&self) -> impl Future<Output = ()> + Send;
    /// `HomeRelay::closed`.
    fn closed(&self) -> impl Future<Output = ()> + Send;
    /// Unix seconds now. A trait method so tests can move the clock.
    fn now_secs(&self) -> u64;
}

/// The HTTP half of `LiveIo`, which needs no relay handle and so can be
/// tested alone.
pub(super) struct Http {
    client: reqwest::Client,
    /// For info probes. `info_url` is written by a coordinator operator, a
    /// third party, so a probe must not follow a redirect to another address
    /// of this host.
    probe_client: reqwest::Client,
    /// A list request has ended with something other than a refused
    /// connection: an HTTP answer of any status, or a timeout. Until then a
    /// refused connection is tried again quickly.
    registry_reached: AtomicBool,
}

impl Http {
    pub(super) fn new() -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(registry_http::HTTP_REQUEST_TIMEOUT)
            .build()
            .context("building the coordinator-list HTTP client")?;
        let probe_client = reqwest::Client::builder()
            .timeout(PROBE_TIMEOUT)
            .redirect(redirect::Policy::none())
            .build()
            .context("building the coordinator info HTTP client")?;
        Ok(Self { client, probe_client, registry_reached: AtomicBool::new(false) })
    }

    /// Tries up to 3 times, but only on a refused connection and only until
    /// the registry has answered once. Failed tries are logged at debug
    /// level only: the resolver's own warning limit would otherwise be
    /// broken by one line per try.
    pub(super) async fn fetch_list(
        &self,
        registry_url: &str,
    ) -> anyhow::Result<CuratedList<CoordinatorEntry>> {
        let filter = CoordinatorFilter { transport: CoordinatorTransport::IrohRelay };
        let mut waits = FETCH_TRY_WAITS.iter();
        loop {
            let answer =
                curated_list::fetch::<CoordinatorEntry>(&self.client, registry_url, &filter, 0)
                    .await;
            let refused = answer.as_ref().err().is_some_and(is_connect_error);
            if !refused {
                self.registry_reached.store(true, Ordering::Relaxed);
            }
            let wait =
                waits.next().filter(|_| refused && !self.registry_reached.load(Ordering::Relaxed));
            let Some(wait) = wait else { return answer };
            debug!("coordinator list request was refused; trying again");
            time::sleep(*wait).await;
        }
    }

    pub(super) async fn probe(&self, info_url: &str) -> anyhow::Result<CoordinatorInfo> {
        let response = self
            .probe_client
            .get(format!("{info_url}{INFO_PATH}"))
            .send()
            .await?
            .error_for_status()?;
        let body = super::read_capped(response, MAX_PROBE_BODY_BYTES).await?;
        serde_json::from_slice(&body).context("the info answer is not valid")
    }
}

fn is_connect_error(e: &anyhow::Error) -> bool {
    e.downcast_ref::<reqwest::Error>().is_some_and(reqwest::Error::is_connect)
}

pub(super) struct LiveIo {
    http: Http,
    home_relay: HomeRelay,
}

impl LiveIo {
    /// # Errors
    /// When the HTTP client cannot be built.
    pub(super) fn new(home_relay: HomeRelay) -> anyhow::Result<Self> {
        Ok(Self { http: Http::new()?, home_relay })
    }
}

impl ResolverIo for LiveIo {
    async fn fetch_list(
        &self,
        registry_url: &str,
    ) -> anyhow::Result<CuratedList<CoordinatorEntry>> {
        self.http.fetch_list(registry_url).await
    }

    async fn probe(&self, info_url: &str) -> anyhow::Result<CoordinatorInfo> {
        self.http.probe(info_url).await
    }

    async fn set_relay(&self, url: RelayUrl) -> Result<(), HomeRelayError> {
        self.home_relay.set(url).await
    }

    fn current_relay(&self) -> Option<RelayUrl> {
        self.home_relay.current_url()
    }

    async fn clear_unconfirmed(&self) {
        self.home_relay.clear_unconfirmed().await;
    }

    async fn closed(&self) {
        self.home_relay.closed().await;
    }

    fn now_secs(&self) -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
    }
}

#[cfg(test)]
mod tests;
