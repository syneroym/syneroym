//! Picks the home relay from the coordinator list of the configured registry,
//! keeps it while it works, and picks again when it does not.
//!
//! The task owns the `HomeRelay` handle: nothing else calls `set` on a node
//! with this source. It never stops a `set` half way (the handle's own doc
//! comment says why), so `set_relay` is never inside a `select!`.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    mem,
    path::PathBuf,
    time::Duration,
};

use syneroym_core::{
    coordinator_info::{CoordinatorInfo, CoordinatorStatus},
    curated_list::{self, CoordinatorEntry, CuratedList},
    retry,
};
use syneroym_router::{HomeRelay, HomeRelayError, net_iroh};
use tokio::time::{self, Instant};
use tracing::{debug, info, warn};

use super::{
    WarnLimiter,
    cache::{self, CoordinatorCache},
    io::{LiveIo, ResolverIo},
};

/// How often the chosen coordinator is probed.
const HEALTH_INTERVAL: Duration = Duration::from_secs(60);
/// This many failed probes within `FAILURE_WINDOW` start a new pick.
const FAILURE_LIMIT: usize = 3;
const FAILURE_WINDOW: Duration = Duration::from_secs(300);
/// Retry delays when there is no relay, or a kept relay waits for a
/// replacement.
const RETRY_INITIAL: Duration = Duration::from_secs(5);
const RETRY_MAX: Duration = Duration::from_secs(600);
/// Retry delays for a failed list refresh while the relay works.
const REFRESH_RETRY_INITIAL: Duration = Duration::from_secs(60);
const REFRESH_RETRY_MAX: Duration = Duration::from_secs(3_600);
/// Bounds on the wait between two refreshes, whatever `max_age_secs` says.
/// The floor stops a tight loop on `max_age_secs = 0`; the ceiling keeps the
/// deadline inside what a clock can hold.
const MIN_REFRESH: Duration = Duration::from_secs(10);
const MAX_REFRESH: Duration = Duration::from_secs(7 * 24 * 3_600);
/// How long an entry whose relay did not confirm is left alone while a
/// working relay is kept.
const CONFIRM_COOLDOWN: Duration = Duration::from_secs(3_600);

/// Warns through `limiter`, or logs at debug level when it is too soon.
macro_rules! warn_limited {
    ($limiter:expr, $($fields:tt)+) => {
        if $limiter.should_warn() {
            warn!($($fields)+);
        } else {
            debug!($($fields)+);
        }
    };
}

/// The endpoint closed; the task ends.
struct Closed;

/// What one info probe says about an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProbeVerdict {
    /// `healthy` with an online relay: may be picked.
    Open,
    /// `at_capacity` with an online relay: may be kept, not picked.
    Full,
    /// No answer, no online relay, or a status this build does not know.
    Down,
}

impl ProbeVerdict {
    fn of(answer: anyhow::Result<CoordinatorInfo>) -> Self {
        let Ok(info) = answer else { return Self::Down };
        if !info.relay.is_some_and(|r| r.online) {
            return Self::Down;
        }
        match info.status {
            CoordinatorStatus::Healthy => Self::Open,
            CoordinatorStatus::AtCapacity => Self::Full,
            CoordinatorStatus::Unknown => Self::Down,
        }
    }

    fn can_keep(self) -> bool {
        matches!(self, Self::Open | Self::Full)
    }
}

/// Failed probes of the chosen relay.
#[derive(Debug, Default)]
struct FailureWindow {
    times: VecDeque<Instant>,
}

impl FailureWindow {
    /// Adds a failure at `now`, forgets those older than `FAILURE_WINDOW`,
    /// and returns true when `FAILURE_LIMIT` are left.
    fn record(&mut self, now: Instant) -> bool {
        self.times.push_back(now);
        while self.times.front().is_some_and(|t| now.duration_since(*t) > FAILURE_WINDOW) {
            self.times.pop_front();
        }
        self.times.len() >= FAILURE_LIMIT
    }
}

/// A delay that doubles on every use, up to `max`, with ±10 % jitter.
#[derive(Debug)]
struct Backoff {
    initial: Duration,
    max: Duration,
    next: Duration,
}

impl Backoff {
    fn new(initial: Duration, max: Duration) -> Self {
        Self { initial, max, next: initial }
    }

    /// The delay to wait now. The one after it is twice as long.
    fn next(&mut self) -> Duration {
        let millis = u64::try_from(self.next.as_millis()).unwrap_or(u64::MAX);
        let delay = Duration::from_millis(retry::calculate_jittered_backoff(millis));
        self.next = (self.next * 2).min(self.max);
        delay
    }

    fn reset(&mut self) {
        self.next = self.initial;
    }
}

struct Resolver<I> {
    io: I,
    /// The substrate DID. The list is ranked with it.
    chooser: String,
    registry_url: String,
    cache_path: PathBuf,
    cache: Option<CoordinatorCache>,
    /// Delay for "no relay" and for "a kept relay waits for a replacement".
    /// Reset when a pick succeeds.
    retry: Backoff,
    /// When a kept relay that is off the list or failing gets its next
    /// replacement try. `None` when no try is due.
    repick_at: Option<Instant>,
    /// The last fetch error, shown in the "no home relay" warning instead of
    /// its own log line.
    last_fetch_error: Option<String>,
    /// Entries whose relay did not confirm, and when.
    not_confirmed: HashMap<String, Instant>,
    /// The last probe of the current relay passed. A successful pick sets it
    /// (the entry was just probed), and so does every passing health probe.
    /// Starts `false`: there is no relay yet.
    kept_relay_ok: bool,
    /// Failed probes of the current relay. Lives here, not in `keep`, so a
    /// failed replacement does not make the next probe start from zero.
    failures: FailureWindow,
    /// When the current relay is probed next.
    next_health: Instant,
    warn_degraded: WarnLimiter,
    warn_refresh: WarnLimiter,
    /// "Coordinator did not answer" and "relay not confirmed".
    warn_relay: WarnLimiter,
}

/// Entry point, called by `connect_home_relay`.
pub(super) async fn run(
    home_relay: HomeRelay,
    registry_url: String,
    cache_path: PathBuf,
    chooser: String,
) {
    let io = match LiveIo::new(home_relay) {
        Ok(io) => io,
        Err(e) => {
            warn!(error = %e, "cannot resolve a home relay (degraded mode)");
            return;
        }
    };
    Resolver::new(io, registry_url, cache_path, chooser).run().await;
}

impl<I: ResolverIo> Resolver<I> {
    fn new(io: I, registry_url: String, cache_path: PathBuf, chooser: String) -> Self {
        let cache = cache::load(&cache_path).filter(|c| {
            let same = c.registry_url == registry_url;
            if !same {
                info!("ignoring coordinators.json from another registry");
            }
            same
        });
        Self {
            io,
            chooser,
            registry_url,
            cache_path,
            cache,
            retry: Backoff::new(RETRY_INITIAL, RETRY_MAX),
            repick_at: None,
            last_fetch_error: None,
            not_confirmed: HashMap::new(),
            kept_relay_ok: false,
            failures: FailureWindow::default(),
            next_health: Instant::now() + HEALTH_INTERVAL,
            warn_degraded: WarnLimiter::default(),
            warn_refresh: WarnLimiter::default(),
            warn_relay: WarnLimiter::default(),
        }
    }

    async fn run(mut self) {
        let mut skip = None;
        let mut outside_logged = false;
        loop {
            let Ok(picked) = self.resolve_once(skip.take()).await else { return };
            if picked {
                self.retry.reset();
                self.repick_at = None;
            }
            if let Some(chosen) = self.kept_choice() {
                if !picked {
                    // Stay on the relay we have and try again on the backoff.
                    self.repick_at = Some(Instant::now() + self.retry.next());
                    info!("no other listed relay works; keeping the current home relay for now");
                }
                match self.keep(chosen).await {
                    Ok(url) => skip = Some(url),
                    Err(Closed) => return,
                }
                continue;
            }
            if !self.wait_without_choice(&mut outside_logged).await {
                return;
            }
        }
    }

    /// The chosen entry, when the endpoint also has a relay.
    fn kept_choice(&self) -> Option<CoordinatorEntry> {
        self.io.current_relay()?;
        self.chosen().cloned()
    }

    /// No relay of ours: degraded mode, or a relay set by someone else.
    /// Waits for the next try; returns false when the endpoint closed.
    async fn wait_without_choice(&mut self, outside_logged: &mut bool) -> bool {
        let delay = self.retry.next();
        if self.io.current_relay().is_some() {
            if !mem::replace(outside_logged, true) {
                info!("the home relay was set outside the coordinator list; leaving it alone");
            }
        } else {
            self.io.clear_unconfirmed().await;
            self.log_degraded(delay);
        }
        tokio::select! {
            () = time::sleep(delay) => true,
            () = self.io.closed() => false,
        }
    }

    fn log_degraded(&mut self, delay: Duration) {
        let last_error = self.last_fetch_error.as_deref().unwrap_or("none");
        warn_limited!(
            self.warn_degraded,
            "no home relay: the coordinator list of {} gave no usable relay (degraded mode). \
             Peers cannot reach this node through a relay, and its endpoint record is not \
             published. Last list error: {last_error}. Trying again in {delay:?}.",
            self.registry_url
        );
    }

    fn chosen(&self) -> Option<&CoordinatorEntry> {
        self.cache.as_ref()?.chosen.as_ref()
    }

    fn lists(&self, url: &str) -> bool {
        self.cache.as_ref().is_some_and(|c| c.lists(url))
    }

    /// One pass, in this order: the saved choice, a fresh saved list, the
    /// registry, and last the saved list at any age. `skip` is a relay that
    /// has just failed. Returns true when a relay is now confirmed.
    async fn resolve_once(&mut self, skip: Option<String>) -> Result<bool, Closed> {
        let mut tried: HashSet<String> = skip.into_iter().collect();
        if self.try_saved_choice(&mut tried).await? {
            return Ok(true);
        }
        if let Some(entries) = self.fresh_entries()
            && self.pick_from(entries, &mut tried).await?
        {
            return Ok(true);
        }
        match self.io.fetch_list(&self.registry_url).await {
            Ok(list) => {
                self.last_fetch_error = None;
                let entries = list.entries.clone();
                self.store_list(list);
                self.pick_from(entries, &mut tried).await
            }
            Err(e) => {
                self.last_fetch_error = Some(format!("{e:#}"));
                match self.saved_entries() {
                    Some(entries) => self.pick_from(entries, &mut tried).await,
                    None => Ok(false),
                }
            }
        }
    }

    /// The saved choice, while the saved list still lists it and its
    /// coordinator answers. A relay off the list gets no new connection.
    async fn try_saved_choice(&mut self, tried: &mut HashSet<String>) -> Result<bool, Closed> {
        let Some(entry) =
            self.chosen().filter(|e| !tried.contains(&e.url) && self.lists(&e.url)).cloned()
        else {
            return Ok(false);
        };
        tried.insert(entry.url.clone());
        if !ProbeVerdict::of(self.io.probe(&entry.info_url).await).can_keep() {
            return Ok(false);
        }
        self.try_set(&entry).await
    }

    /// The entries of the saved list, when it is fresh and not empty.
    fn fresh_entries(&self) -> Option<Vec<CoordinatorEntry>> {
        let now = self.io.now_secs();
        self.cache
            .as_ref()
            .filter(|c| c.is_fresh(now) && !c.list.entries.is_empty())
            .map(|c| c.list.entries.clone())
    }

    /// The entries of the saved list, whatever its age, when it is not empty.
    fn saved_entries(&self) -> Option<Vec<CoordinatorEntry>> {
        self.cache.as_ref().filter(|c| !c.list.entries.is_empty()).map(|c| c.list.entries.clone())
    }

    /// Tries the entries in rank order. Returns true when one is confirmed.
    async fn pick_from(
        &mut self,
        entries: Vec<CoordinatorEntry>,
        tried: &mut HashSet<String>,
    ) -> Result<bool, Closed> {
        for entry in curated_list::rank(&self.chooser, entries) {
            if !tried.insert(entry.url.clone()) {
                continue;
            }
            // A failed switch costs a gap with no working relay. Do not
            // repeat one soon while the relay we have still works. A dead
            // relay gets no cool-down: a failed switch costs nothing then.
            if self.io.current_relay().is_some()
                && self.kept_relay_ok
                && self.recently_not_confirmed(&entry.url)
            {
                continue;
            }
            match ProbeVerdict::of(self.io.probe(&entry.info_url).await) {
                ProbeVerdict::Open => {}
                ProbeVerdict::Full => {
                    info!(relay_url = %entry.url, "skipping a full coordinator");
                    continue;
                }
                ProbeVerdict::Down => {
                    info!(relay_url = %entry.url, "skipping a coordinator that does not answer");
                    continue;
                }
            }
            if self.try_set(&entry).await? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn recently_not_confirmed(&mut self, url: &str) -> bool {
        self.not_confirmed.retain(|_, at| at.elapsed() < CONFIRM_COOLDOWN);
        self.not_confirmed.contains_key(url)
    }

    async fn try_set(&mut self, entry: &CoordinatorEntry) -> Result<bool, Closed> {
        // A normalised URL is http(s) with a host, so this only fails on a
        // value that `normalize_url` lets through and `RelayUrl` does not.
        let Ok(url) = net_iroh::parse_relay_url("coordinator list entry", &entry.url) else {
            debug!(relay_url = %entry.url, "skipping an entry that is not a relay URL");
            return Ok(false);
        };
        match self.io.set_relay(url).await {
            Ok(()) => {
                info!(
                    relay_url = %entry.url,
                    operator = %entry.operator,
                    "home relay chosen from the coordinator list"
                );
                self.save_choice(entry.clone());
                self.kept_relay_ok = true;
                self.failures = FailureWindow::default();
                self.next_health = Instant::now() + HEALTH_INTERVAL;
                Ok(true)
            }
            Err(e @ HomeRelayError::NotConfirmed { .. }) => {
                self.not_confirmed.insert(entry.url.clone(), Instant::now());
                warn_limited!(self.warn_relay, error = %e, "relay from the coordinator list was not confirmed");
                Ok(false)
            }
            Err(HomeRelayError::EndpointClosed) => Err(Closed),
        }
    }

    /// Keeps the cache with the new list and the old choice. The choice
    /// stays because `keep` needs its `info_url` for health probes while the
    /// endpoint uses it.
    fn store_list(&mut self, list: CuratedList<CoordinatorEntry>) {
        let chosen = self.cache.take().and_then(|c| c.chosen);
        self.cache = Some(CoordinatorCache {
            registry_url: self.registry_url.clone(),
            fetched_at_secs: self.io.now_secs(),
            list,
            chosen,
        });
        self.persist();
    }

    fn save_choice(&mut self, entry: CoordinatorEntry) {
        // The entry came from a list, so a cache exists.
        if let Some(cache) = self.cache.as_mut() {
            cache.chosen = Some(entry);
            self.persist();
        }
    }

    /// Saving is best effort: the state in memory still works without it.
    fn persist(&self) {
        let Some(cache) = &self.cache else { return };
        if let Err(e) = cache::save(&self.cache_path, cache) {
            warn!(error = %format!("{e:#}"), "cannot save the coordinator cache");
        }
    }

    /// How long until the saved list is due for a refresh.
    fn refresh_delay(&self) -> Duration {
        let now = self.io.now_secs();
        let secs = self.cache.as_ref().map_or(0, |c| c.expires_at_secs().saturating_sub(now));
        Duration::from_secs(secs).clamp(MIN_REFRESH, MAX_REFRESH)
    }

    /// Watches the chosen relay. Returns its URL when a new pick is needed:
    /// it left the list, or it failed too often.
    ///
    /// A new list that ranks another entry first does not start a new pick.
    async fn keep(&mut self, chosen: CoordinatorEntry) -> Result<String, Closed> {
        let mut refresh_retry = Backoff::new(REFRESH_RETRY_INITIAL, REFRESH_RETRY_MAX);
        let mut refresh_at = Instant::now() + self.refresh_delay();
        loop {
            let next_health = self.next_health;
            let repick_at = self.repick_at;
            tokio::select! {
                () = self.io.closed() => return Err(Closed),
                () = time::sleep_until(next_health) => {
                    if self.on_health_tick(&chosen).await {
                        return Ok(chosen.url);
                    }
                }
                () = time::sleep_until(refresh_at) => {
                    if self.on_refresh(&chosen, &mut refresh_at, &mut refresh_retry).await {
                        return Ok(chosen.url);
                    }
                }
                () = time::sleep_until(repick_at.unwrap_or(refresh_at)), if repick_at.is_some() => {
                    if self.on_repick_due(&chosen) {
                        return Ok(chosen.url);
                    }
                }
            }
        }
    }

    /// Probes the chosen coordinator. True when it has failed often enough
    /// to pick again.
    async fn on_health_tick(&mut self, chosen: &CoordinatorEntry) -> bool {
        self.next_health = Instant::now() + HEALTH_INTERVAL;
        self.kept_relay_ok = ProbeVerdict::of(self.io.probe(&chosen.info_url).await).can_keep();
        if self.kept_relay_ok {
            return false;
        }
        warn_limited!(
            self.warn_relay,
            relay_url = %chosen.url,
            "the coordinator of the home relay did not answer"
        );
        if !self.failures.record(Instant::now()) {
            return false;
        }
        warn_limited!(self.warn_relay, "3 failures within 5 minutes; choosing another relay");
        self.failures = FailureWindow::default();
        true
    }

    /// Asks the registry for the list again. True when the chosen relay is
    /// no longer listed and no replacement try is already planned.
    async fn on_refresh(
        &mut self,
        chosen: &CoordinatorEntry,
        refresh_at: &mut Instant,
        refresh_retry: &mut Backoff,
    ) -> bool {
        match self.io.fetch_list(&self.registry_url).await {
            Ok(list) => {
                refresh_retry.reset();
                self.store_list(list);
                *refresh_at = Instant::now() + self.refresh_delay();
                // With a try already planned, its timer decides. Returning
                // here would skip the backoff when `max_age_secs` is short.
                let gone = !self.lists(&chosen.url) && self.repick_at.is_none();
                if gone {
                    info!(
                        relay_url = %chosen.url,
                        "home relay left the coordinator list; choosing again"
                    );
                }
                gone
            }
            Err(e) => {
                warn_limited!(
                    self.warn_refresh,
                    error = %format!("{e:#}"),
                    "cannot refresh the coordinator list; keeping the saved list"
                );
                *refresh_at = Instant::now() + refresh_retry.next();
                false
            }
        }
    }

    /// A planned replacement try is due. True when it should run; false when
    /// the relay is fine again and the tries stop.
    fn on_repick_due(&mut self, chosen: &CoordinatorEntry) -> bool {
        if self.kept_relay_ok && self.lists(&chosen.url) {
            self.repick_at = None;
            self.retry.reset();
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests;
