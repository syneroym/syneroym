use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use syneroym_app_orchestration::{
    AppInstanceId, ServiceId, ShardingStrategy, StaticInventory, TopologyDocument, TopologyEpoch,
    TopologyMode,
};
use syneroym_core::dht_registry::{EndpointInfo, EndpointType, SignedEndpointInfo};
use syneroym_identity::substrate::{self, derive_did_key};

use super::*;

#[derive(Debug)]
struct CountingFetcher {
    calls: AtomicUsize,
    signed: SignedTopologyDocument,
}

#[async_trait::async_trait]
impl TopologyFetcher for CountingFetcher {
    async fn fetch(
        &self,
        _app_did: &AppDid,
        _service_name: &LogicalServiceName,
    ) -> Result<SignedTopologyDocument> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.signed.clone())
    }
}

/// The two performance properties this exercises, measured directly
/// rather than as timings. One `fetch_and_register`, then N `resolve`
/// calls, asserts `fetch_calls == 1` -- resolution after the first
/// fetch makes no network call. `register_verified` (the only caller
/// of `verify`) having run exactly once by construction here shows
/// `verify` runs once per fetch, not once per resolve.
#[tokio::test]
async fn one_fetch_and_register_serves_every_later_resolve() {
    let master = Identity::generate().unwrap();
    let app_did = AppDid::new(substrate::derive_did_key(&master.public_key()));
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let doc = TopologyDocument {
        app_instance_id: AppInstanceId::new("inst-1"),
        app_did: app_did.clone(),
        service_name: LogicalServiceName::new("backend"),
        mode: TopologyMode::Singleton,
        members: vec![ServiceId::new("did:key:zMember")],
        sharding_strategy: None,
        epoch: TopologyEpoch(1),
        generation: 0,
        issued_at: now,
        not_after: now + 3600,
        cache_ttl_ms: 60_000,
    };
    let signed = doc.sign(&master).unwrap();
    let fetcher = CountingFetcher { calls: AtomicUsize::new(0), signed };

    let resolver = LogicalResolver::new(Arc::new(StaticInventory::new()));
    let key =
        fetch_and_register(&fetcher, &resolver, &app_did, &LogicalServiceName::new("backend"))
            .await
            .unwrap();

    for _ in 0..5 {
        assert!(resolver.resolve(&key, None).is_ok());
    }
    assert_eq!(
        fetcher.calls.load(Ordering::SeqCst),
        1,
        "budget 1: no network call after the first fetch"
    );
}

// ── `AppHostResolver` ──────────────────────────────────────────────

#[derive(Debug)]
struct FakeTier1 {
    calls: AtomicUsize,
    response: SignedEndpointInfo,
}

#[async_trait::async_trait]
impl Tier1Lookup for FakeTier1 {
    async fn lookup(&self, _alias: &str) -> Result<SignedEndpointInfo> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.response.clone())
    }
}

#[derive(Debug, Default)]
struct FakeTier2 {
    calls: AtomicUsize,
    responses: StdMutex<VecDeque<SignedTopologyDocument>>,
}

#[async_trait::async_trait]
impl Tier2Fetch for FakeTier2 {
    async fn fetch_via(
        &self,
        _supervisor_did: &str,
        _app_did: &AppDid,
        _service_name: &LogicalServiceName,
    ) -> Result<SignedTopologyDocument> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow::anyhow!("FakeTier2 has no more queued responses"))
    }
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn app_master() -> (Identity, AppDid) {
    let master = Identity::generate().unwrap();
    let app_did = AppDid::new(derive_did_key(&master.public_key()));
    (master, app_did)
}

fn signed_tier1_record(
    app_did: &AppDid,
    supervisor_did: &str,
    master: &Identity,
) -> SignedEndpointInfo {
    EndpointInfo {
        service_id: app_did.as_str().to_string(),
        substrate_id: supervisor_did.to_string(),
        endpoint_type: EndpointType::Service,
        mechanisms: vec![],
        nickname: Some("my-chat-app".to_string()),
        is_private: false,
        ttl: None,
        not_after: now_secs() + 3600,
        generation: 0,
    }
    .sign(master)
    .unwrap()
}

#[allow(clippy::too_many_arguments)]
fn signed_topology_doc(
    app_did: &AppDid,
    master: &Identity,
    service_name: &str,
    mode: TopologyMode,
    members: Vec<&str>,
    not_after_offset_secs: u64,
    sharding_strategy: Option<ShardingStrategy>,
) -> SignedTopologyDocument {
    let now = now_secs();
    let doc = TopologyDocument {
        app_instance_id: AppInstanceId::new("my-chat-app"),
        app_did: app_did.clone(),
        service_name: LogicalServiceName::new(service_name),
        mode,
        members: members.into_iter().map(ServiceId::new).collect(),
        sharding_strategy,
        epoch: TopologyEpoch(1),
        generation: 0,
        issued_at: now,
        not_after: now + not_after_offset_secs,
        cache_ttl_ms: 60_000,
    };
    doc.sign(master).unwrap()
}

fn app_host_resolver(tier1: Arc<FakeTier1>, fetcher: Option<Arc<FakeTier2>>) -> AppHostResolver {
    AppHostResolver::new(
        Box::new(tier1),
        fetcher.map(|f| Box::new(f) as Box<dyn Tier2Fetch>),
        LogicalResolver::new(Arc::new(StaticInventory::new())),
    )
}

/// The no-registry path (`fetcher: None`, an empty `registry_url`) is
/// what `ClientGateway::init` builds when `[substrate].registry_url`
/// is unset -- an app-scoped host must be refused with a message
/// naming Tier 1, not left to the panic/hang a missing fetcher would
/// otherwise produce.
#[tokio::test]
async fn an_app_scoped_host_is_refused_with_no_registry_configured() {
    let (master, app_did) = app_master();
    let a_hash = util::short_hash(app_did.as_str());
    let record = signed_tier1_record(&app_did, "did:key:zSupervisor", &master);
    let tier1 = Arc::new(FakeTier1 { calls: AtomicUsize::new(0), response: record });
    let resolver = app_host_resolver(tier1, None);
    let s_hash = util::short_hash("backend");

    let err = resolver.resolve_app_host("my-chat-app", &a_hash, &s_hash, None).await.unwrap_err();
    assert!(err.to_string().contains("no community registry configured"), "{err}");
}

/// The alias half of the binding check: a registry answering an alias
/// with another app's perfectly valid, self-signed record is refused.
#[tokio::test]
async fn a_tier1_record_whose_hash_does_not_match_the_a_segment_is_refused() {
    let (master, app_did) = app_master();
    let record = signed_tier1_record(&app_did, "did:key:zSupervisor", &master);
    let tier1 = Arc::new(FakeTier1 { calls: AtomicUsize::new(0), response: record });
    let resolver = app_host_resolver(tier1, Some(Arc::new(FakeTier2::default())));

    // A wrong `a_hash`, not the one this app's DID actually hashes to.
    let err = resolver
        .resolve_app_host("my-chat-app-wronghash", "wronghash", "anyhash", None)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("wronghash"), "{err}");
}

/// The document half of the binding check: a document naming a
/// different service than the `-s` segment is refused.
#[tokio::test]
async fn a_document_naming_a_different_service_than_the_s_segment_is_refused() {
    let (master, app_did) = app_master();
    let a_hash = util::short_hash(app_did.as_str());
    let record = signed_tier1_record(&app_did, "did:key:zSupervisor", &master);
    let tier1 = Arc::new(FakeTier1 { calls: AtomicUsize::new(0), response: record });
    let doc = signed_topology_doc(
        &app_did,
        &master,
        "backend",
        TopologyMode::Singleton,
        vec!["did:key:zMember0"],
        3600,
        None,
    );
    let fetcher = Arc::new(FakeTier2::default());
    fetcher.responses.lock().unwrap().push_back(doc);
    let resolver = app_host_resolver(tier1, Some(fetcher));

    // A wrong `s_hash`, not `short_hash("backend")`.
    let err = resolver
        .resolve_app_host(&format!("my-chat-app-{a_hash}"), &a_hash, "wronghash", None)
        .await
        .unwrap_err();
    // Pins the Tier-2 check specifically: a correct `a_hash` means
    // Tier 1 must succeed here, so an `OR` against either segment's
    // error text would just as readily pass on a Tier-1 regression as
    // on the Tier-2 binding check this test exists to cover.
    assert!(
        err.to_string().contains("supervisor answered '-swronghash' with service 'backend'"),
        "{err}"
    );
}

/// A second request for the same app-scoped host makes no network
/// call -- checked as a fetch count, not a timing.
#[tokio::test]
async fn a_second_request_for_the_same_app_scoped_host_makes_no_network_call() {
    let (master, app_did) = app_master();
    let a_hash = util::short_hash(app_did.as_str());
    let record = signed_tier1_record(&app_did, "did:key:zSupervisor", &master);
    let tier1 = Arc::new(FakeTier1 { calls: AtomicUsize::new(0), response: record });
    let doc = signed_topology_doc(
        &app_did,
        &master,
        "backend",
        TopologyMode::Singleton,
        vec!["did:key:zMember0"],
        3600,
        None,
    );
    let fetcher = Arc::new(FakeTier2::default());
    fetcher.responses.lock().unwrap().push_back(doc);
    let resolver = app_host_resolver(tier1.clone(), Some(fetcher.clone()));
    let s_hash = util::short_hash("backend");

    let first = resolver.resolve_app_host("my-chat-app", &a_hash, &s_hash, None).await.unwrap();
    assert_eq!(first, "did:key:zMember0");

    let fetcher_calls_before = fetcher.calls.load(Ordering::SeqCst);
    let tier1_calls_before = tier1.calls.load(Ordering::SeqCst);
    let second = resolver.resolve_app_host("my-chat-app", &a_hash, &s_hash, None).await.unwrap();
    assert_eq!(second, "did:key:zMember0");
    assert_eq!(fetcher.calls.load(Ordering::SeqCst), fetcher_calls_before, "no new Tier-2 fetch");
    assert_eq!(tier1.calls.load(Ordering::SeqCst), tier1_calls_before, "no new Tier-1 lookup");
}

/// One Tier-1 lookup per cold app-scoped resolve, not two, and zero
/// on a warm one.
#[tokio::test]
async fn a_cold_resolve_makes_exactly_one_tier1_lookup() {
    let (master, app_did) = app_master();
    let a_hash = util::short_hash(app_did.as_str());
    let record = signed_tier1_record(&app_did, "did:key:zSupervisor", &master);
    let tier1 = Arc::new(FakeTier1 { calls: AtomicUsize::new(0), response: record });
    let doc = signed_topology_doc(
        &app_did,
        &master,
        "backend",
        TopologyMode::Singleton,
        vec!["did:key:zMember0"],
        3600,
        None,
    );
    let fetcher = Arc::new(FakeTier2::default());
    fetcher.responses.lock().unwrap().push_back(doc);
    let resolver = app_host_resolver(tier1.clone(), Some(fetcher));
    let s_hash = util::short_hash("backend");

    resolver.resolve_app_host("my-chat-app", &a_hash, &s_hash, None).await.unwrap();
    assert_eq!(
        tier1.calls.load(Ordering::SeqCst),
        1,
        "exactly one Tier-1 lookup on a cold resolve"
    );

    resolver.resolve_app_host("my-chat-app", &a_hash, &s_hash, None).await.unwrap();
    assert_eq!(tier1.calls.load(Ordering::SeqCst), 1, "zero more Tier-1 lookups on a warm resolve");
}

/// The Tier-1 cache is keyed on the full `app_lookup_alias`, not
/// `a_hash` alone -- a second host carrying a
/// *different* nickname over the same app hash must repeat its own
/// Tier-1 alias lookup rather than silently reuse the first alias's
/// warm entry, which would let the cache accept a nickname the
/// registry was never actually asked to bind.
#[tokio::test]
async fn a_different_nickname_over_the_same_app_hash_repeats_the_tier1_lookup() {
    let (master, app_did) = app_master();
    let a_hash = util::short_hash(app_did.as_str());
    let record = signed_tier1_record(&app_did, "did:key:zSupervisor", &master);
    let tier1 = Arc::new(FakeTier1 { calls: AtomicUsize::new(0), response: record });
    let doc = signed_topology_doc(
        &app_did,
        &master,
        "backend",
        TopologyMode::Singleton,
        vec!["did:key:zMember0"],
        3600,
        None,
    );
    let fetcher = Arc::new(FakeTier2::default());
    fetcher.responses.lock().unwrap().push_back(doc);
    let resolver = app_host_resolver(tier1.clone(), Some(fetcher));
    let s_hash = util::short_hash("backend");

    resolver.resolve_app_host("my-chat-app", &a_hash, &s_hash, None).await.unwrap();
    assert_eq!(tier1.calls.load(Ordering::SeqCst), 1);

    // Same `a_hash`, a different nickname -- a different alias.
    resolver
        .resolve_app_host("a-totally-different-nickname", &a_hash, &s_hash, None)
        .await
        .unwrap();
    assert_eq!(
        tier1.calls.load(Ordering::SeqCst),
        2,
        "a different alias over the same hash must not reuse the first alias's cache entry"
    );
}

/// ADR-0022 §3's "on expiry try to refresh" -- an expired cache entry
/// triggers exactly one refetch rather than a failure.
#[tokio::test]
async fn an_expired_entry_triggers_one_refetch_rather_than_a_failure() {
    let (master, app_did) = app_master();
    let a_hash = util::short_hash(app_did.as_str());
    let record = signed_tier1_record(&app_did, "did:key:zSupervisor", &master);
    let tier1 = Arc::new(FakeTier1 { calls: AtomicUsize::new(0), response: record });
    // A 3s margin, not 1s: every wall-clock-boundary-adjacent
    // `not_after` in this codebase uses 3s for the same reason -- a
    // `not_after` computed one second before a real second boundary
    // leaves under 1ms of actual margin.
    let short_lived = signed_topology_doc(
        &app_did,
        &master,
        "backend",
        TopologyMode::Singleton,
        vec!["did:key:zMember0"],
        3,
        None,
    );
    let fresh = signed_topology_doc(
        &app_did,
        &master,
        "backend",
        TopologyMode::Singleton,
        vec!["did:key:zMember1"],
        3600,
        None,
    );
    let fetcher = Arc::new(FakeTier2::default());
    fetcher.responses.lock().unwrap().push_back(short_lived);
    fetcher.responses.lock().unwrap().push_back(fresh);
    let resolver = app_host_resolver(tier1, Some(fetcher.clone()));
    let s_hash = util::short_hash("backend");

    let first = resolver.resolve_app_host("my-chat-app", &a_hash, &s_hash, None).await.unwrap();
    assert_eq!(first, "did:key:zMember0");

    tokio::time::sleep(Duration::from_millis(3500)).await;

    let second = resolver.resolve_app_host("my-chat-app", &a_hash, &s_hash, None).await.unwrap();
    assert_eq!(second, "did:key:zMember1", "must have refetched the fresh document");
    assert_eq!(fetcher.calls.load(Ordering::SeqCst), 2, "exactly one refetch, not a failure");
}

/// Over a `Redundant` document -- the same routing key twice returns
/// the same member, and no header returns members in round-robin.
#[tokio::test]
async fn a_routing_key_header_selects_a_member_and_its_absence_does_not() {
    let (master, app_did) = app_master();
    let a_hash = util::short_hash(app_did.as_str());
    let record = signed_tier1_record(&app_did, "did:key:zSupervisor", &master);
    let tier1 = Arc::new(FakeTier1 { calls: AtomicUsize::new(0), response: record });
    let doc = signed_topology_doc(
        &app_did,
        &master,
        "backend",
        TopologyMode::Redundant,
        vec!["did:key:zMember0", "did:key:zMember1", "did:key:zMember2"],
        3600,
        None,
    );
    let fetcher = Arc::new(FakeTier2::default());
    fetcher.responses.lock().unwrap().push_back(doc);
    let resolver = app_host_resolver(tier1, Some(fetcher));
    let s_hash = util::short_hash("backend");
    let key = b"routing-key-alice";

    let first =
        resolver.resolve_app_host("my-chat-app", &a_hash, &s_hash, Some(key)).await.unwrap();
    for _ in 0..5 {
        let repeat =
            resolver.resolve_app_host("my-chat-app", &a_hash, &s_hash, Some(key)).await.unwrap();
        assert_eq!(repeat, first, "the same routing key must select the same member");
    }

    // The other half of this test's own title: with no header at all,
    // a `Redundant` topology round-robins rather than pinning one
    // member.
    let mut seen = std::collections::HashSet::new();
    for _ in 0..6 {
        seen.insert(
            resolver.resolve_app_host("my-chat-app", &a_hash, &s_hash, None).await.unwrap(),
        );
    }
    assert!(seen.len() > 1, "an unkeyed resolve must spread across members, got {seen:?}");
}

/// ADR-0022 §7's closing sentence -- a `Sharded` service with no
/// routing key fails with the resolver's own, specific error.
#[tokio::test]
async fn a_sharded_service_with_no_routing_key_fails_with_the_resolvers_own_error() {
    let (master, app_did) = app_master();
    let a_hash = util::short_hash(app_did.as_str());
    let record = signed_tier1_record(&app_did, "did:key:zSupervisor", &master);
    let tier1 = Arc::new(FakeTier1 { calls: AtomicUsize::new(0), response: record });
    let doc = signed_topology_doc(
        &app_did,
        &master,
        "backend",
        TopologyMode::Sharded,
        vec!["did:key:zMember0", "did:key:zMember1"],
        3600,
        Some(ShardingStrategy::HashSharding),
    );
    let fetcher = Arc::new(FakeTier2::default());
    fetcher.responses.lock().unwrap().push_back(doc);
    let resolver = app_host_resolver(tier1, Some(fetcher));
    let s_hash = util::short_hash("backend");

    let err = resolver.resolve_app_host("my-chat-app", &a_hash, &s_hash, None).await.unwrap_err();
    assert!(
        err.to_string().contains("routing_key") || err.to_string().contains("Sharded"),
        "{err}"
    );
}

/// A permanent selection failure (here, a `Sharded` topology with no
/// routing key) must not be treated as a cache miss. `FakeTier2` is
/// seeded with exactly one response, so a second refetch would surface
/// as "no more queued responses" instead of the resolver's own error
/// -- the discriminator this test relies on -- and `fetcher.calls`
/// pins it directly. Before the fix, every repeat call refetched Tier
/// 2, making a network call for a caller stuck in this permanent
/// state.
#[tokio::test]
async fn a_permanent_selection_failure_is_not_treated_as_a_cache_miss() {
    let (master, app_did) = app_master();
    let a_hash = util::short_hash(app_did.as_str());
    let record = signed_tier1_record(&app_did, "did:key:zSupervisor", &master);
    let tier1 = Arc::new(FakeTier1 { calls: AtomicUsize::new(0), response: record });
    let doc = signed_topology_doc(
        &app_did,
        &master,
        "backend",
        TopologyMode::Sharded,
        vec!["did:key:zMember0", "did:key:zMember1"],
        3600,
        Some(ShardingStrategy::HashSharding),
    );
    let fetcher = Arc::new(FakeTier2::default());
    fetcher.responses.lock().unwrap().push_back(doc);
    let resolver = app_host_resolver(tier1, Some(fetcher.clone()));
    let s_hash = util::short_hash("backend");

    for _ in 0..3 {
        let err =
            resolver.resolve_app_host("my-chat-app", &a_hash, &s_hash, None).await.unwrap_err();
        assert!(err.to_string().contains("routing_key"), "{err}");
    }
    assert_eq!(
        fetcher.calls.load(Ordering::SeqCst),
        1,
        "a permanent selection error must not trigger a refetch"
    );
}

/// A failed cold resolve is remembered for `NEGATIVE_CACHE_TTL`, so a
/// caller repeating the same bad host (the WebRTC bootstrap listener
/// that also calls this is public and unauthenticated) does not repeat
/// a Tier-1 round trip for every repeat. Uses a wrong `a_hash` for the
/// failure itself; what this test pins is that the *second* identical
/// failure costs no further lookup.
#[tokio::test]
async fn a_recent_failure_is_served_from_the_negative_cache_without_a_repeat_lookup() {
    let (master, app_did) = app_master();
    let record = signed_tier1_record(&app_did, "did:key:zSupervisor", &master);
    let tier1 = Arc::new(FakeTier1 { calls: AtomicUsize::new(0), response: record });
    let resolver = app_host_resolver(tier1.clone(), Some(Arc::new(FakeTier2::default())));

    let first = resolver
        .resolve_app_host("my-chat-app-wronghash", "wronghash", "anyhash", None)
        .await
        .unwrap_err();
    assert!(first.to_string().contains("wronghash"), "{first}");
    assert_eq!(tier1.calls.load(Ordering::SeqCst), 1);

    let second = resolver
        .resolve_app_host("my-chat-app-wronghash", "wronghash", "anyhash", None)
        .await
        .unwrap_err();
    assert_eq!(second.to_string(), first.to_string(), "the remembered failure must be identical");
    assert_eq!(
        tier1.calls.load(Ordering::SeqCst),
        1,
        "a fresh negative-cache hit must not repeat the Tier-1 lookup"
    );
}

/// The negative cache had no sweep -- an entry was only ever removed
/// by a later *success* for that exact key, so on the public,
/// unauthenticated bootstrap listener it grew by one entry per
/// distinct bad `Host` header forever. A new failure now sweeps every
/// expired entry on its way in.
#[tokio::test]
async fn a_new_failure_sweeps_every_expired_negative_cache_entry() {
    let (master, app_did) = app_master();
    let record = signed_tier1_record(&app_did, "did:key:zSupervisor", &master);
    let tier1 = Arc::new(FakeTier1 { calls: AtomicUsize::new(0), response: record });
    let resolver = app_host_resolver(tier1, Some(Arc::new(FakeTier2::default())));

    let _ = resolver.resolve_app_host("host-one", "wronghash", "anyhash", None).await;
    let _ = resolver.resolve_app_host("host-two", "wronghash", "anyhash", None).await;
    assert_eq!(resolver.negative_cache.len(), 2);

    tokio::time::sleep(NEGATIVE_CACHE_TTL + Duration::from_millis(500)).await;

    // A third, distinct failure sweeps the first two (now stale) on
    // its way in, leaving only itself.
    let _ = resolver.resolve_app_host("host-three", "wronghash", "anyhash", None).await;
    assert_eq!(
        resolver.negative_cache.len(),
        1,
        "expired entries must be swept, not accumulate forever"
    );
}

/// A `Tier1Lookup` whose artificial delay is what makes every
/// concurrent caller in `concurrent_cold_resolves_for_the_same_host_
/// share_one_fetch` still be waiting when the first one starts its
/// real lookup -- without it, the race the test exists to exercise
/// would only happen by chance.
#[derive(Debug)]
struct SlowTier1 {
    calls: AtomicUsize,
    response: SignedEndpointInfo,
}

#[async_trait::async_trait]
impl Tier1Lookup for SlowTier1 {
    async fn lookup(&self, _alias: &str) -> Result<SignedEndpointInfo> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(200)).await;
        Ok(self.response.clone())
    }
}

/// N concurrent callers for the same not-yet-cached host share one
/// Tier-1-then-Tier-2 round trip rather than each starting an
/// independent one. `FakeTier2` is seeded with exactly one
/// response, so this also fails loudly ("no more queued responses")
/// if the single-flight lock lets more than one caller through to the
/// real fetch.
#[tokio::test]
async fn concurrent_cold_resolves_for_the_same_host_share_one_fetch() {
    let (master, app_did) = app_master();
    let a_hash = util::short_hash(app_did.as_str());
    let record = signed_tier1_record(&app_did, "did:key:zSupervisor", &master);
    let tier1 = Arc::new(SlowTier1 { calls: AtomicUsize::new(0), response: record });
    let doc = signed_topology_doc(
        &app_did,
        &master,
        "backend",
        TopologyMode::Singleton,
        vec!["did:key:zMember0"],
        3600,
        None,
    );
    let fetcher = Arc::new(FakeTier2::default());
    fetcher.responses.lock().unwrap().push_back(doc);
    let resolver = AppHostResolver::new(
        Box::new(tier1.clone()),
        Some(Box::new(fetcher.clone()) as Box<dyn Tier2Fetch>),
        LogicalResolver::new(Arc::new(StaticInventory::new())),
    );
    let s_hash = util::short_hash("backend");

    let (r1, r2, r3, r4) = tokio::join!(
        resolver.resolve_app_host("my-chat-app", &a_hash, &s_hash, None),
        resolver.resolve_app_host("my-chat-app", &a_hash, &s_hash, None),
        resolver.resolve_app_host("my-chat-app", &a_hash, &s_hash, None),
        resolver.resolve_app_host("my-chat-app", &a_hash, &s_hash, None),
    );
    for r in [&r1, &r2, &r3, &r4] {
        assert_eq!(r.as_ref().unwrap().as_str(), "did:key:zMember0", "{r:?}");
    }
    assert_eq!(
        tier1.calls.load(Ordering::SeqCst),
        1,
        "one Tier-1 lookup shared by every concurrent caller"
    );
    assert_eq!(
        fetcher.calls.load(Ordering::SeqCst),
        1,
        "one Tier-2 fetch shared by every concurrent caller"
    );
}
