//! A fake `ResolverIo` and the harness the resolver tests share. Every test
//! runs on a paused clock, so the 60 s probes and the backoff cost no real
//! time.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use anyhow::anyhow;
use iroh::RelayUrl;
use syneroym_core::{
    coordinator_info::{CoordinatorInfo, CoordinatorStatus, RelayInfo},
    curated_list::{self, CoordinatorEntry, CoordinatorTransport, CuratedList},
};
use syneroym_router::HomeRelayError;
use tempfile::TempDir;
use tokio::{
    task::JoinHandle,
    time::{self, Instant},
};
use tokio_util::sync::CancellationToken;

use super::{Backoff, FailureWindow, ProbeVerdict, Resolver, ResolverIo};
use crate::runtime::home_relay::cache::{self as cache_file, CoordinatorCache};

mod boot;
mod keep;
mod logs;

pub(super) const REGISTRY: &str = "http://registry.test";
const CHOOSER: &str = "did:key:htest";
const START_SECS: u64 = 1_000_000;

pub(super) fn entry(n: u32) -> CoordinatorEntry {
    CoordinatorEntry {
        transport: CoordinatorTransport::IrohRelay,
        url: format!("http://relay-{n}.test"),
        info_url: format!("http://info-{n}.test"),
        operator: format!("operator {n}"),
    }
}

/// The entries in the order the resolver tries them.
pub(super) fn ranked(entries: &[CoordinatorEntry]) -> Vec<CoordinatorEntry> {
    curated_list::rank(CHOOSER, entries.iter().cloned())
}

pub(super) fn info(status: CoordinatorStatus, relay_online: bool) -> CoordinatorInfo {
    CoordinatorInfo {
        endpoint_addr_bytes: Vec::new(),
        substrate_id: "test".into(),
        relay_url: None,
        parent_coordinator_url: None,
        status,
        identity: None,
        connections: None,
        tls: None,
        relay: Some(RelayInfo { online: relay_online, clients: None, max_clients: None }),
        registry: None,
    }
}

/// A saved cache: `age_secs` old, lists `entries`, and chose `chosen`.
pub(super) fn cache(
    entries: &[CoordinatorEntry],
    max_age_secs: u64,
    age_secs: u64,
    chosen: Option<&CoordinatorEntry>,
) -> CoordinatorCache {
    CoordinatorCache {
        registry_url: REGISTRY.into(),
        fetched_at_secs: START_SECS - age_secs,
        list: CuratedList { max_age_secs, entries: entries.to_vec() },
        chosen: chosen.cloned(),
    }
}

pub(super) async fn secs(n: u64) {
    time::sleep(Duration::from_secs(n)).await;
}

/// Sleeps until `s` seconds after `t0`.
pub(super) async fn at(t0: Instant, s: u64) {
    time::sleep_until(t0 + Duration::from_secs(s)).await;
}

#[derive(Clone)]
pub(super) struct FakeIo(Arc<FakeState>);

struct FakeState {
    /// The answer of `fetch_list`; `None` is an error.
    list: Mutex<Option<CuratedList<CoordinatorEntry>>>,
    fetches: AtomicUsize,
    /// `info_url` to answer; a missing key is a probe error.
    infos: Mutex<HashMap<String, CoordinatorInfo>>,
    probes: Mutex<HashMap<String, usize>>,
    /// Normalised relay URLs that `set_relay` confirms.
    confirms: Mutex<HashSet<String>>,
    /// Every `set_relay` call, normalised.
    sets: Mutex<Vec<String>>,
    current: Mutex<Option<RelayUrl>>,
    clears: AtomicUsize,
    closed: CancellationToken,
    started: Instant,
}

impl FakeIo {
    pub(super) fn new() -> Self {
        Self(Arc::new(FakeState {
            list: Mutex::new(None),
            fetches: AtomicUsize::new(0),
            infos: Mutex::new(HashMap::new()),
            probes: Mutex::new(HashMap::new()),
            confirms: Mutex::new(HashSet::new()),
            sets: Mutex::new(Vec::new()),
            current: Mutex::new(None),
            clears: AtomicUsize::new(0),
            closed: CancellationToken::new(),
            started: Instant::now(),
        }))
    }

    /// The registry answers with this list.
    pub(super) fn serve(&self, entries: &[CoordinatorEntry], max_age_secs: u64) {
        let list = CuratedList { max_age_secs, entries: entries.to_vec() };
        *self.0.list.lock().unwrap() = Some(list);
    }

    /// The registry does not answer.
    pub(super) fn registry_down(&self) {
        *self.0.list.lock().unwrap() = None;
    }

    pub(super) fn answer(&self, e: &CoordinatorEntry, answer: Option<CoordinatorInfo>) {
        let mut infos = self.0.infos.lock().unwrap();
        match answer {
            Some(info) => infos.insert(e.info_url.clone(), info),
            None => infos.remove(&e.info_url),
        };
    }

    /// The coordinator of `e` answers `healthy`.
    pub(super) fn up(&self, e: &CoordinatorEntry) {
        self.answer(e, Some(info(CoordinatorStatus::Healthy, true)));
    }

    /// The coordinator of `e` does not answer.
    pub(super) fn down(&self, e: &CoordinatorEntry) {
        self.answer(e, None);
    }

    /// `set_relay` confirms the relay of `e`.
    pub(super) fn confirm(&self, e: &CoordinatorEntry) {
        self.0.confirms.lock().unwrap().insert(e.url.clone());
    }

    /// Sets the home relay as if someone else had done it.
    pub(super) fn set_current(&self, e: &CoordinatorEntry) {
        *self.0.current.lock().unwrap() = Some(e.url.parse().unwrap());
    }

    pub(super) fn fetches(&self) -> usize {
        self.0.fetches.load(Ordering::SeqCst)
    }

    pub(super) fn clears(&self) -> usize {
        self.0.clears.load(Ordering::SeqCst)
    }

    pub(super) fn probes(&self, e: &CoordinatorEntry) -> usize {
        self.0.probes.lock().unwrap().get(&e.info_url).copied().unwrap_or(0)
    }

    pub(super) fn sets(&self) -> Vec<String> {
        self.0.sets.lock().unwrap().clone()
    }

    /// How many times `set_relay` was called for `e`.
    pub(super) fn set_count(&self, e: &CoordinatorEntry) -> usize {
        self.sets().iter().filter(|u| **u == e.url).count()
    }

    pub(super) fn current(&self) -> Option<String> {
        let current = self.0.current.lock().unwrap();
        current.as_ref().map(|u| curated_list::normalize_url(u.as_str()).unwrap())
    }

    pub(super) fn close(&self) {
        self.0.closed.cancel();
    }
}

impl ResolverIo for FakeIo {
    async fn fetch_list(
        &self,
        _registry_url: &str,
    ) -> anyhow::Result<CuratedList<CoordinatorEntry>> {
        self.0.fetches.fetch_add(1, Ordering::SeqCst);
        self.0.list.lock().unwrap().clone().ok_or_else(|| anyhow!("registry is down"))
    }

    async fn probe(&self, info_url: &str) -> anyhow::Result<CoordinatorInfo> {
        *self.0.probes.lock().unwrap().entry(info_url.to_owned()).or_default() += 1;
        self.0.infos.lock().unwrap().get(info_url).cloned().ok_or_else(|| anyhow!("no answer"))
    }

    async fn set_relay(&self, url: RelayUrl) -> Result<(), HomeRelayError> {
        let normal = curated_list::normalize_url(url.as_str()).unwrap();
        self.0.sets.lock().unwrap().push(normal.clone());
        if self.0.confirms.lock().unwrap().contains(&normal) {
            *self.0.current.lock().unwrap() = Some(url);
            return Ok(());
        }
        // The real handle rolls back to the old relay, so `current` stays.
        Err(HomeRelayError::NotConfirmed { url, timeout: Duration::from_secs(15) })
    }

    fn current_relay(&self) -> Option<RelayUrl> {
        self.0.current.lock().unwrap().clone()
    }

    async fn clear_unconfirmed(&self) {
        self.0.clears.fetch_add(1, Ordering::SeqCst);
    }

    async fn closed(&self) {
        self.0.closed.cancelled().await;
    }

    fn now_secs(&self) -> u64 {
        START_SECS + self.0.started.elapsed().as_secs()
    }
}

/// A running resolver and the cache file it uses.
pub(super) struct Run {
    pub(super) io: FakeIo,
    pub(super) path: PathBuf,
    task: JoinHandle<()>,
    _dir: TempDir,
}

/// Starts the resolver with `saved` already on disk, if any.
pub(super) fn start(io: &FakeIo, saved: Option<CoordinatorCache>) -> Run {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("coordinators.json");
    if let Some(saved) = saved {
        cache_file::save(&path, &saved).unwrap();
    }
    let resolver = Resolver::new(io.clone(), REGISTRY.into(), path.clone(), CHOOSER.into());
    Run { io: io.clone(), path, task: tokio::spawn(resolver.run()), _dir: dir }
}

impl Run {
    /// Closes the endpoint and checks that the task ends.
    pub(super) async fn stop(self) {
        self.io.close();
        time::timeout(Duration::from_secs(5), self.task)
            .await
            .expect("the task ends when the endpoint closes")
            .unwrap();
    }
}

#[test]
fn probe_verdict_reads_status_and_relay() {
    let answer = |status, online| ProbeVerdict::of(Ok(info(status, online)));
    assert_eq!(answer(CoordinatorStatus::Healthy, true), ProbeVerdict::Open);
    assert_eq!(answer(CoordinatorStatus::AtCapacity, true), ProbeVerdict::Full);
    assert_eq!(answer(CoordinatorStatus::Unknown, true), ProbeVerdict::Down);
    assert_eq!(answer(CoordinatorStatus::Healthy, false), ProbeVerdict::Down);
    assert_eq!(ProbeVerdict::of(Err(anyhow!("timeout"))), ProbeVerdict::Down);

    let mut no_relay = info(CoordinatorStatus::Healthy, true);
    no_relay.relay = None;
    assert_eq!(ProbeVerdict::of(Ok(no_relay)), ProbeVerdict::Down);

    assert!(ProbeVerdict::Open.can_keep());
    assert!(ProbeVerdict::Full.can_keep());
    assert!(!ProbeVerdict::Down.can_keep());
}

#[tokio::test(start_paused = true)]
async fn failure_window_needs_three_failures_inside_five_minutes() {
    let t0 = Instant::now();
    let mut window = FailureWindow::default();
    assert!(!window.record(t0));
    assert!(!window.record(t0 + Duration::from_secs(100)));
    assert!(window.record(t0 + Duration::from_secs(200)));

    // The first failure is now more than 300 s old, so it no longer counts.
    let mut window = FailureWindow::default();
    assert!(!window.record(t0));
    assert!(!window.record(t0 + Duration::from_secs(200)));
    assert!(!window.record(t0 + Duration::from_secs(400)));
    assert!(window.record(t0 + Duration::from_secs(450)));
}

#[test]
fn backoff_doubles_up_to_its_cap_and_resets() {
    let within = |d: Duration, want: u64| {
        let want = Duration::from_secs(want);
        d >= want.mul_f64(0.89) && d <= want.mul_f64(1.11)
    };
    let mut backoff = Backoff::new(Duration::from_secs(5), Duration::from_secs(20));
    for want in [5, 10, 20, 20, 20] {
        let got = backoff.next();
        assert!(within(got, want), "{got:?} is not about {want} s");
    }
    backoff.reset();
    assert!(within(backoff.next(), 5));
}
