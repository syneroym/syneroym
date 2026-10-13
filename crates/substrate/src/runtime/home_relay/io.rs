//! Everything the resolver does outside itself. The resolver is timing-heavy
//! (60 s probes, 5-minute windows, backoff), so its unit tests replace this
//! trait with a fake and run on a paused clock.

use std::{
    future::Future,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use iroh::RelayUrl;
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

/// Waits between fetch tries. The first fetch can race the co-located
/// registry's bind, so a refused connection is tried again quickly.
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

pub(super) struct LiveIo {
    client: reqwest::Client,
    home_relay: HomeRelay,
}

impl LiveIo {
    /// # Errors
    /// When the HTTP client cannot be built.
    pub(super) fn new(home_relay: HomeRelay) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(registry_http::HTTP_REQUEST_TIMEOUT)
            .build()
            .context("building the coordinator-list HTTP client")?;
        Ok(Self { client, home_relay })
    }
}

impl ResolverIo for LiveIo {
    /// Up to 3 tries, logged at debug level only: the resolver's own warning
    /// limit would otherwise be broken by one line per try.
    async fn fetch_list(
        &self,
        registry_url: &str,
    ) -> anyhow::Result<CuratedList<CoordinatorEntry>> {
        let filter = CoordinatorFilter { transport: CoordinatorTransport::IrohRelay };
        let mut waits = FETCH_TRY_WAITS.iter();
        loop {
            let answer =
                curated_list::fetch::<CoordinatorEntry>(&self.client, registry_url, &filter, 0)
                    .await;
            let (Err(e), Some(wait)) = (&answer, waits.next()) else { return answer };
            debug!(error = %e, "coordinator list request failed; trying again");
            time::sleep(*wait).await;
        }
    }

    async fn probe(&self, info_url: &str) -> anyhow::Result<CoordinatorInfo> {
        let info = self
            .client
            .get(format!("{info_url}{INFO_PATH}"))
            .timeout(PROBE_TIMEOUT)
            .send()
            .await?
            .error_for_status()?
            .json::<CoordinatorInfo>()
            .await?;
        Ok(info)
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
