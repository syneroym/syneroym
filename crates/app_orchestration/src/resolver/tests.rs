use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};

use super::*;
use crate::models::{AppDid, AppInstanceId, LogicalServiceName, TopologyMode};

// ── Helper builders ──────────────────────────────────────

fn inst(s: &str) -> AppInstanceId {
    AppInstanceId::new(s)
}

fn svc_name(s: &str) -> LogicalServiceName {
    LogicalServiceName::new(s)
}

fn svc_id(s: &str) -> ServiceId {
    ServiceId::new(format!("did:key:{s}"))
}

fn did(s: &str) -> AppDid {
    AppDid::new(format!("did:key:{s}"))
}

fn local_key(inst_id: &str, name: &str) -> TopologyKey {
    TopologyKey::local(inst(inst_id), svc_name(name))
}

fn foreign_key(app_did: &str, name: &str) -> TopologyKey {
    TopologyKey::foreign(did(app_did), svc_name(name))
}

fn make_entry(
    mode: TopologyMode,
    members: Vec<ServiceId>,
    strategy: Option<ShardingStrategy>,
) -> TopologyEntry {
    TopologyEntry {
        mode,
        members,
        sharding_strategy: strategy,
        epoch: TopologyEpoch::default(),
        cache_ttl: Duration::from_secs(60),
        not_after: None,
    }
}

fn registry_with(entries: Vec<(TopologyKey, TopologyEntry)>) -> Arc<StaticInventory> {
    let reg = Arc::new(StaticInventory::new());
    for (key, entry) in entries {
        reg.register(key, entry);
    }
    reg
}

// ── StaticInventory ──────────────────────────────────────

#[test]
fn test_static_inventory_register_and_get() {
    let inv = StaticInventory::new();
    let key = local_key("app-1", "auth");
    let entry = make_entry(TopologyMode::Singleton, vec![svc_id("abc")], None);

    inv.register(key.clone(), entry.clone());

    let got = inv.get(&key).expect("should be present");
    assert_eq!(got.mode, TopologyMode::Singleton);
    assert_eq!(got.members, vec![svc_id("abc")]);
}

#[test]
fn test_static_inventory_list() {
    let inv = StaticInventory::new();
    let id = inst("app-1");
    inv.register(
        local_key("app-1", "auth"),
        make_entry(TopologyMode::Singleton, vec![svc_id("a")], None),
    );
    inv.register(
        local_key("app-1", "cache"),
        make_entry(TopologyMode::Redundant, vec![svc_id("b")], None),
    );
    // Different app — should not be listed.
    inv.register(
        local_key("other", "auth"),
        make_entry(TopologyMode::Singleton, vec![svc_id("c")], None),
    );

    let mut names = inv.list(&AppScope::Local(id));
    names.sort();
    assert_eq!(names, vec![svc_name("auth"), svc_name("cache")]);
}

#[test]
fn test_static_inventory_update_replaces_entry() {
    let inv = StaticInventory::new();
    let key = local_key("app-1", "auth");

    inv.register(key.clone(), make_entry(TopologyMode::Singleton, vec![svc_id("old")], None));
    inv.register(
        key.clone(),
        TopologyEntry {
            epoch: TopologyEpoch(1),
            ..make_entry(TopologyMode::Redundant, vec![svc_id("new1"), svc_id("new2")], None)
        },
    );

    let got = inv.get(&key).unwrap();
    assert_eq!(got.mode, TopologyMode::Redundant);
    assert_eq!(got.epoch, TopologyEpoch(1));
    assert_eq!(got.members.len(), 2);
}

#[test]
fn test_static_inventory_get_missing() {
    let inv = StaticInventory::new();
    assert!(inv.get(&local_key("app-x", "nonexistent")).is_none());
}

// ── Rendezvous hashing ───────────────────────────────────

#[test]
fn test_rendezvous_select_deterministic() {
    let members = vec![svc_id("alpha"), svc_id("beta"), svc_id("gamma")];
    let app_domain = b"app-instance-1";
    let svc_domain = b"svc-1";
    let key = b"user-42";

    let first = rendezvous_select(&members, app_domain, svc_domain, key);
    let second = rendezvous_select(&members, app_domain, svc_domain, key);
    assert_eq!(first, second, "rendezvous selection must be deterministic");
}

#[test]
fn test_rendezvous_select_different_keys_can_differ() {
    let members = vec![svc_id("alpha"), svc_id("beta"), svc_id("gamma")];
    let app_domain = b"app-instance-1";
    let svc_domain = b"svc-1";

    let results: Vec<_> = (0u64..20)
        .map(|i| rendezvous_select(&members, app_domain, svc_domain, &i.to_be_bytes()))
        .collect();

    let distinct: HashSet<_> =
        results.into_iter().flatten().map(|s| s.as_str().to_string()).collect();
    // With 20 keys and 3 members, expect at least 2 distinct selections.
    assert!(distinct.len() >= 2, "rendezvous should distribute across members");
}

#[test]
fn test_rendezvous_select_single_member() {
    let members = vec![svc_id("only")];
    let result = rendezvous_select(&members, b"app", b"svc", b"any-key");
    assert_eq!(result, Some(&svc_id("only")));
}

#[test]
fn test_rendezvous_select_empty() {
    let members: Vec<ServiceId> = vec![];
    assert!(rendezvous_select(&members, b"app", b"svc", b"key").is_none());
}

#[test]
fn test_rendezvous_domain_separator_changes_result() {
    let members = vec![svc_id("alpha"), svc_id("beta"), svc_id("gamma")];
    let key = b"same-routing-key";

    // Different domain separators (AppInstanceIds/LogicalServiceNames) must produce
    // independent hash spaces.  Collect multiple results and confirm they
    // are not all identical across different domain separators.
    let results_by_domain: Vec<Option<&ServiceId>> = [
        (b"app-a".as_ref(), b"svc-1".as_ref()),
        (b"app-b".as_ref(), b"svc-1".as_ref()),
        (b"app-c".as_ref(), b"svc-1".as_ref()),
        (b"app-a".as_ref(), b"svc-2".as_ref()),
    ]
    .iter()
    .map(|(app, svc)| rendezvous_select(&members, app, svc, key))
    .collect();

    let distinct: HashSet<_> =
        results_by_domain.into_iter().flatten().map(|s| s.as_str().to_string()).collect();
    // With 4 different domain separators and the same routing key, we
    // expect at least 2 distinct selected members.
    assert!(
        distinct.len() >= 2,
        "different domain separators should produce independent hash spaces"
    );
}

// ── LogicalResolver — Singleton ──────────────────────────

#[test]
fn test_resolve_singleton() {
    let reg = registry_with(vec![(
        local_key("app-1", "auth"),
        make_entry(TopologyMode::Singleton, vec![svc_id("sole-member")], None),
    )]);
    let resolver = LogicalResolver::new(reg);
    let key = local_key("app-1", "auth");

    let id = resolver.resolve(&key, None).unwrap();
    assert_eq!(id, svc_id("sole-member"));
}

#[test]
fn test_resolve_unregistered_returns_error() {
    let reg = Arc::new(StaticInventory::new());
    let resolver = LogicalResolver::new(reg);
    let err = resolver.resolve(&local_key("ghost-app", "missing"), None);
    assert!(err.is_err());
    assert!(err.unwrap_err().to_string().contains("No topology registered"));
}

#[test]
fn test_resolve_empty_members_returns_error() {
    let reg = registry_with(vec![(
        local_key("app-1", "empty"),
        make_entry(TopologyMode::Singleton, vec![], None),
    )]);
    let resolver = LogicalResolver::new(reg);
    let err = resolver.resolve(&local_key("app-1", "empty"), None);
    assert!(err.is_err());
    assert!(err.unwrap_err().to_string().contains("no eligible members"));
}

// ── LogicalResolver — Redundant ──────────────────────────

#[test]
fn test_resolve_redundant_round_robin() {
    let members = vec![svc_id("r0"), svc_id("r1"), svc_id("r2")];
    let reg = registry_with(vec![(
        local_key("app-1", "cache"),
        make_entry(TopologyMode::Redundant, members.clone(), None),
    )]);
    let resolver = LogicalResolver::new(reg);
    let key = local_key("app-1", "cache");

    // With no routing key, round-robin through members.
    let r0 = resolver.resolve(&key, None).unwrap();
    let r1 = resolver.resolve(&key, None).unwrap();
    let r2 = resolver.resolve(&key, None).unwrap();
    let r3 = resolver.resolve(&key, None).unwrap(); // wraps back

    assert_eq!(r0, members[0]);
    assert_eq!(r1, members[1]);
    assert_eq!(r2, members[2]);
    assert_eq!(r3, members[0]); // wrapped
}

#[test]
fn test_resolve_redundant_keyed_is_deterministic() {
    let members = vec![svc_id("r0"), svc_id("r1"), svc_id("r2")];
    let reg = registry_with(vec![(
        local_key("app-1", "cache"),
        make_entry(TopologyMode::Redundant, members, None),
    )]);
    let resolver = LogicalResolver::new(reg);
    let key = local_key("app-1", "cache");

    let a = resolver.resolve(&key, Some(b"key-abc")).unwrap();
    let b = resolver.resolve(&key, Some(b"key-abc")).unwrap();
    assert_eq!(a, b, "keyed redundant resolve must be deterministic");
}

// ── LogicalResolver — Sharded ────────────────────────────

#[test]
fn test_resolve_sharded_requires_routing_key() {
    let reg = registry_with(vec![(
        local_key("app-1", "store"),
        make_entry(
            TopologyMode::Sharded,
            vec![svc_id("s0"), svc_id("s1")],
            Some(ShardingStrategy::HashSharding),
        ),
    )]);
    let resolver = LogicalResolver::new(reg);
    let key = local_key("app-1", "store");

    let err = resolver.resolve(&key, None);
    assert!(err.is_err());
    assert!(err.unwrap_err().to_string().contains("routing_key"));
}

#[test]
fn test_resolve_sharded_hash_deterministic() {
    let members = vec![svc_id("s0"), svc_id("s1"), svc_id("s2")];
    let reg = registry_with(vec![(
        local_key("app-1", "store"),
        make_entry(TopologyMode::Sharded, members, Some(ShardingStrategy::HashSharding)),
    )]);
    let resolver = LogicalResolver::new(reg);
    let key = local_key("app-1", "store");

    let a = resolver.resolve(&key, Some(b"user:42")).unwrap();
    let b_res = resolver.resolve(&key, Some(b"user:42")).unwrap();
    assert_eq!(a, b_res);
}

#[test]
fn test_resolve_sharded_entity_tag_uses_partition_key() {
    // EntityTagSharding: only the bytes before the first NUL matter.
    let members = vec![svc_id("s0"), svc_id("s1"), svc_id("s2")];
    let reg = registry_with(vec![(
        local_key("app-1", "ts"),
        make_entry(TopologyMode::Sharded, members, Some(ShardingStrategy::EntityTagSharding)),
    )]);
    let resolver = LogicalResolver::new(reg);
    let key = local_key("app-1", "ts");

    // Same partition key, different item keys → same shard.
    let mut key1 = b"tenant-99\0item-1".to_vec();
    let mut key2 = b"tenant-99\0item-2".to_vec();
    let _ = &mut key1; // suppress unused warning
    let _ = &mut key2;
    let r1 = resolver.resolve(&key, Some(&key1)).unwrap();
    let r2 = resolver.resolve(&key, Some(&key2)).unwrap();
    assert_eq!(r1, r2, "same partition key must map to same shard");
}

#[test]
fn test_resolve_sharded_distribution() {
    let members = vec![svc_id("s0"), svc_id("s1"), svc_id("s2")];
    let reg = registry_with(vec![(
        local_key("app-1", "store"),
        make_entry(TopologyMode::Sharded, members.clone(), Some(ShardingStrategy::HashSharding)),
    )]);
    let resolver = LogicalResolver::new(reg);
    let key = local_key("app-1", "store");

    let mut counts = BTreeMap::new();
    for i in 0u64..300 {
        let routing_key = i.to_be_bytes();
        let selected = resolver.resolve(&key, Some(&routing_key)).unwrap();
        *counts.entry(selected.to_string()).or_insert(0u64) += 1;
    }
    // All 3 members should be selected at least once with 300 distinct keys.
    assert_eq!(counts.len(), 3, "all shards should receive traffic");
}

// ── LogicalResolver — cache invalidation ─────────────────

#[test]
fn test_cache_hit_bypasses_registry() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Debug)]
    struct MockRegistry {
        call_count: AtomicUsize,
        entry: TopologyEntry,
    }

    impl AppRegistry for MockRegistry {
        fn register(&self, _: TopologyKey, _: TopologyEntry) {}
        fn get(&self, _: &TopologyKey) -> Option<TopologyEntry> {
            self.call_count.fetch_add(1, Ordering::Relaxed);
            Some(self.entry.clone())
        }
        fn invalidate(&self, _: &TopologyKey) {}
        fn list(&self, _: &AppScope) -> Vec<LogicalServiceName> {
            vec![]
        }
    }

    let mock = Arc::new(MockRegistry {
        call_count: AtomicUsize::new(0),
        entry: make_entry(TopologyMode::Singleton, vec![svc_id("sole")], None),
    });

    let resolver = LogicalResolver::new(mock.clone());
    let key = local_key("app-1", "auth");

    // First resolve -> miss -> calls get
    resolver.resolve(&key, None).unwrap();
    assert_eq!(mock.call_count.load(Ordering::Relaxed), 1);

    // Second resolve -> hit -> should NOT call get
    resolver.resolve(&key, None).unwrap();
    assert_eq!(mock.call_count.load(Ordering::Relaxed), 1, "Cache hit must bypass registry");
}

#[test]
fn test_explicit_invalidate_clears_cache() {
    let inv = Arc::new(StaticInventory::new());
    let key = local_key("app-1", "auth");
    inv.register(key.clone(), make_entry(TopologyMode::Singleton, vec![svc_id("v1")], None));

    let resolver = LogicalResolver::new(inv.clone());

    // Populate cache.
    let _ = resolver.resolve(&key, None).unwrap();

    // Update registry (same epoch — TTL still valid, would not normally
    // refresh).  After explicit invalidate the new value should be seen.
    inv.register(key.clone(), make_entry(TopologyMode::Singleton, vec![svc_id("v2")], None));
    resolver.invalidate(&key);

    // Same epoch → cache was just evicted, re-fetch from registry.
    let got = resolver.resolve(&key, None).unwrap();
    assert_eq!(got, svc_id("v2"), "explicit invalidate should evict cache");
}

#[test]
fn register_through_the_resolver_evicts_the_cached_topology() {
    let inv = Arc::new(StaticInventory::new());
    let key = local_key("app-1", "backend");
    inv.register(key.clone(), make_entry(TopologyMode::Singleton, vec![svc_id("v1")], None));

    let resolver = LogicalResolver::new(inv);

    // Populate the cache with a long TTL, so a plain TTL expiry could
    // never explain a refresh below.
    let got = resolver.resolve(&key, None).unwrap();
    assert_eq!(got, svc_id("v1"));

    // A scale-out: two members now, written through the resolver's own
    // `register`, not the registry directly.
    resolver.register(
        key.clone(),
        make_entry(TopologyMode::Redundant, vec![svc_id("v1"), svc_id("v2")], None),
    );

    // Visible immediately -- not after `cache_ttl` -- because `register`
    // evicted the stale cached copy in the same step.
    let all = resolver.resolve_all(&key).unwrap();
    assert_eq!(all.members, vec![svc_id("v1"), svc_id("v2")]);
}

#[test]
fn test_ttl_expiry_triggers_refresh() {
    // Use a zero-TTL entry to simulate instant expiry.
    let inv = Arc::new(StaticInventory::new());
    let key = local_key("app-1", "auth");
    inv.register(
        key.clone(),
        TopologyEntry {
            cache_ttl: Duration::ZERO,
            ..make_entry(TopologyMode::Singleton, vec![svc_id("v1")], None)
        },
    );

    let resolver = LogicalResolver::new(inv.clone());

    // Populate cache (with zero TTL it immediately expires).
    let _ = resolver.resolve(&key, None).unwrap();

    // Update registry.
    inv.register(
        key.clone(),
        TopologyEntry {
            cache_ttl: Duration::ZERO,
            ..make_entry(TopologyMode::Singleton, vec![svc_id("v2")], None)
        },
    );

    // TTL is zero → expired → must re-fetch.
    let got = resolver.resolve(&key, None).unwrap();
    assert_eq!(got, svc_id("v2"), "expired TTL should trigger cache refresh");
}

// ── resolve_all ──────────────────────────────────────────

#[test]
fn test_resolve_all_returns_epoch_snapshot() {
    let members = vec![svc_id("m0"), svc_id("m1")];
    let reg = registry_with(vec![(
        local_key("app-1", "store"),
        TopologyEntry {
            epoch: TopologyEpoch(7),
            ..make_entry(
                TopologyMode::Sharded,
                members.clone(),
                Some(ShardingStrategy::HashSharding),
            )
        },
    )]);
    let resolver = LogicalResolver::new(reg);
    let key = local_key("app-1", "store");

    let all = resolver.resolve_all(&key).unwrap();
    assert_eq!(all.topology_epoch, TopologyEpoch(7));
    assert_eq!(all.members, members);
}

#[test]
fn test_resolve_all_unregistered_returns_error() {
    let reg = Arc::new(StaticInventory::new());
    let resolver = LogicalResolver::new(reg);
    let err = resolver.resolve_all(&local_key("ghost", "svc"));
    assert!(err.is_err());
}

// ── TopologyEntry serialization ──────────────────────────

#[test]
fn test_topology_entry_serialization_roundtrip() {
    let entry = TopologyEntry {
        mode: TopologyMode::Sharded,
        members: vec![svc_id("a"), svc_id("b")],
        sharding_strategy: Some(ShardingStrategy::EntityTagSharding),
        epoch: TopologyEpoch(42),
        cache_ttl: Duration::from_secs(120),
        not_after: Some(1_800_000_000),
    };

    let json = serde_json::to_string(&entry).unwrap();
    let decoded: TopologyEntry = serde_json::from_str(&json).unwrap();
    assert_eq!(entry, decoded);
}

// ── AppScope / TopologyKey / not_after ──────────────

/// Two unrelated apps both called `chat` must not collide -- each is keyed
/// by its own app DID, a disjoint namespace from `AppScope::Local`.
#[test]
fn two_foreign_apps_with_the_same_instance_id_do_not_collide() {
    let reg = registry_with(vec![
        (
            foreign_key("appA", "chat"),
            make_entry(TopologyMode::Singleton, vec![svc_id("member-a")], None),
        ),
        (
            foreign_key("appB", "chat"),
            make_entry(TopologyMode::Singleton, vec![svc_id("member-b")], None),
        ),
    ]);
    let resolver = LogicalResolver::new(reg);

    assert_eq!(resolver.resolve(&foreign_key("appA", "chat"), None).unwrap(), svc_id("member-a"));
    assert_eq!(resolver.resolve(&foreign_key("appB", "chat"), None).unwrap(), svc_id("member-b"));
}

#[test]
fn a_local_entry_and_a_foreign_entry_with_the_same_service_name_are_distinct() {
    let reg = registry_with(vec![
        (
            local_key("app-1", "auth"),
            make_entry(TopologyMode::Singleton, vec![svc_id("local-member")], None),
        ),
        (
            foreign_key("app-1", "auth"),
            make_entry(TopologyMode::Singleton, vec![svc_id("foreign-member")], None),
        ),
    ]);
    let resolver = LogicalResolver::new(reg);

    assert_eq!(
        resolver.resolve(&local_key("app-1", "auth"), None).unwrap(),
        svc_id("local-member"),
        "expected the local entry to resolve independently of the foreign one"
    );
}

/// Checked on both the registry-read path and the cache-hit path -- an
/// entry past `not_after` must fail, not keep answering from a warm cache.
#[test]
fn an_entry_past_its_not_after_stops_resolving() {
    // Registry path: an already-expired entry is never even cached.
    let past = unix_now().saturating_sub(3600);
    let inv = Arc::new(StaticInventory::new());
    let key = foreign_key("app-1", "svc");
    inv.register(
        key.clone(),
        TopologyEntry {
            not_after: Some(past),
            ..make_entry(TopologyMode::Singleton, vec![svc_id("m1")], None)
        },
    );
    let resolver = LogicalResolver::new(inv.clone());
    let err = resolver.resolve(&key, None).unwrap_err();
    assert!(err.to_string().contains("expired"), "{err}");

    // Cache-hit path: an entry valid when cached (a long `cache_ttl`,
    // a `not_after` a moment away) must stop answering once real time
    // carries it past `not_after`, with no registry re-read involved.
    // A 1s margin here raced `unix_now()`'s own second boundary under
    // load (the immediate "still valid" resolve could land exactly on
    // it); 3s leaves two full seconds of slack regardless of where in
    // its current second `register` happens to land.
    let key2 = foreign_key("app-1", "svc2");
    inv.register(
        key2.clone(),
        TopologyEntry {
            not_after: Some(unix_now() + 3),
            ..make_entry(TopologyMode::Singleton, vec![svc_id("m2")], None)
        },
    );
    assert!(resolver.resolve(&key2, None).is_ok(), "warms the cache while still valid");
    std::thread::sleep(Duration::from_millis(3200));
    let err2 = resolver.resolve(&key2, None).unwrap_err();
    assert!(err2.to_string().contains("expired"), "{err2}");
}

/// The absent-means-current-behavior property: every intra-app binding
/// entry has `not_after: None` and must resolve exactly as it does
/// today, with no expiry check ever tripping.
#[test]
fn an_entry_with_no_not_after_resolves_as_it_does_today() {
    let reg = registry_with(vec![(
        local_key("app-1", "auth"),
        make_entry(TopologyMode::Singleton, vec![svc_id("v1")], None),
    )]);
    let resolver = LogicalResolver::new(reg);
    assert_eq!(resolver.resolve(&local_key("app-1", "auth"), None).unwrap(), svc_id("v1"));
}

/// `not_after` is excluded from `classify_binding_write`'s content
/// comparison, the same way `cache_ttl` already is.
#[test]
fn a_not_after_difference_at_one_epoch_is_not_a_binding_conflict() {
    let held = TopologyEntry {
        not_after: Some(1_000),
        ..make_entry(TopologyMode::Singleton, vec![svc_id("v1")], None)
    };
    let incoming = TopologyEntry {
        not_after: Some(2_000),
        ..make_entry(TopologyMode::Singleton, vec![svc_id("v1")], None)
    };
    assert_eq!(classify_binding_write(Some(&held), &incoming), BindingWriteOutcome::NoOp);
}

// ── Performance: cache-hit latency budget ────────────────

#[test]
fn test_cache_hit_latency_under_100ns() {
    let members = vec![svc_id("only")];
    let reg = registry_with(vec![(
        local_key("app-perf", "svc"),
        make_entry(TopologyMode::Singleton, members, None),
    )]);
    let resolver = LogicalResolver::new(reg);
    let key_ref = local_key("app-perf", "svc");
    let key = b"hot-routing-key";

    // Warm the cache.
    resolver.resolve(&key_ref, Some(key)).unwrap();

    // Measure 1000 cache-hit resolutions.
    let start = Instant::now();
    for _ in 0..1000 {
        resolver.resolve(&key_ref, Some(key)).unwrap();
    }
    let elapsed = start.elapsed();
    let per_call_ns = elapsed.as_nanos() / 1000;

    // The architecture budget of <100ns per cache-hit is a release-build
    // target.  Debug builds run unoptimized and cannot reliably meet it.
    // We enforce the strict budget only in release mode.
    #[cfg(not(debug_assertions))]
    assert!(per_call_ns < 100, "cache-hit resolution averaged {per_call_ns}ns, expected <100ns");
    // In debug mode, assert a much more generous bound (10 µs) to at least
    // confirm the code path is exercised without excessive overhead.
    #[cfg(debug_assertions)]
    assert!(
        per_call_ns < 10_000,
        "cache-hit resolution averaged {per_call_ns}ns, expected <10µs in debug mode"
    );
}

#[test]
fn test_independent_round_robin() {
    let members_a = vec![svc_id("a1"), svc_id("a2")];
    let members_b = vec![svc_id("b1"), svc_id("b2")];
    let reg = registry_with(vec![
        (local_key("app", "svc_a"), make_entry(TopologyMode::Redundant, members_a.clone(), None)),
        (local_key("app", "svc_b"), make_entry(TopologyMode::Redundant, members_b.clone(), None)),
    ]);
    let resolver = LogicalResolver::new(reg);

    let ref_a = local_key("app", "svc_a");
    let ref_b = local_key("app", "svc_b");

    // Resolving A should not affect B's counter
    assert_eq!(resolver.resolve(&ref_a, None).unwrap(), members_a[0]);
    assert_eq!(resolver.resolve(&ref_b, None).unwrap(), members_b[0]);
    assert_eq!(resolver.resolve(&ref_a, None).unwrap(), members_a[1]);
    assert_eq!(resolver.resolve(&ref_b, None).unwrap(), members_b[1]);
    assert_eq!(resolver.resolve(&ref_a, None).unwrap(), members_a[0]);
}

#[test]
fn test_range_sharding_validation() {
    // Valid table
    let valid_table = RangeRoutingTable {
        chunks: vec![
            RangeChunk {
                start_key: None,
                end_key: Some(b"bar".to_vec()),
                target: svc_id("shard-1"),
            },
            RangeChunk {
                start_key: Some(b"bar".to_vec()),
                end_key: None,
                target: svc_id("shard-2"),
            },
        ],
    };
    assert!(valid_table.validate().is_ok());

    // Empty table is invalid
    let empty_table = RangeRoutingTable { chunks: vec![] };
    assert!(empty_table.validate().is_err());

    // First chunk doesn't start at -infinity
    let bad_first = RangeRoutingTable {
        chunks: vec![RangeChunk {
            start_key: Some(b"bar".to_vec()),
            end_key: None,
            target: svc_id("shard-1"),
        }],
    };
    assert!(bad_first.validate().is_err());

    // Last chunk doesn't end at +infinity
    let bad_last = RangeRoutingTable {
        chunks: vec![
            RangeChunk {
                start_key: None,
                end_key: Some(b"bar".to_vec()),
                target: svc_id("shard-1"),
            },
            RangeChunk {
                start_key: Some(b"bar".to_vec()),
                end_key: Some(b"foo".to_vec()),
                target: svc_id("shard-2"),
            },
        ],
    };
    assert!(bad_last.validate().is_err());

    // Gap / overlap in between chunks
    let gap_table = RangeRoutingTable {
        chunks: vec![
            RangeChunk {
                start_key: None,
                end_key: Some(b"bar".to_vec()),
                target: svc_id("shard-1"),
            },
            RangeChunk {
                start_key: Some(b"baz".to_vec()),
                end_key: None,
                target: svc_id("shard-2"),
            },
        ],
    };
    assert!(gap_table.validate().is_err());

    // Chunk with start_key >= end_key is invalid
    let bad_order = RangeRoutingTable {
        chunks: vec![
            RangeChunk {
                start_key: None,
                end_key: Some(b"foo".to_vec()),
                target: svc_id("shard-1"),
            },
            RangeChunk {
                start_key: Some(b"foo".to_vec()),
                end_key: Some(b"bar".to_vec()), // "foo" > "bar"
                target: svc_id("shard-2"),
            },
            RangeChunk {
                start_key: Some(b"bar".to_vec()),
                end_key: None,
                target: svc_id("shard-3"),
            },
        ],
    };
    assert!(bad_order.validate().is_err());
}

// ── classify_binding_write ────────────────────

#[test]
fn a_higher_epoch_applies() {
    let held = make_entry(TopologyMode::Singleton, vec![svc_id("v1")], None);
    let incoming = TopologyEntry {
        epoch: TopologyEpoch(1),
        ..make_entry(TopologyMode::Singleton, vec![svc_id("v2")], None)
    };
    assert_eq!(classify_binding_write(Some(&held), &incoming), BindingWriteOutcome::Applied);
}

#[test]
fn an_equal_epoch_with_identical_members_is_a_no_op() {
    let held = make_entry(TopologyMode::Singleton, vec![svc_id("v1")], None);
    let incoming = make_entry(TopologyMode::Singleton, vec![svc_id("v1")], None);
    assert_eq!(classify_binding_write(Some(&held), &incoming), BindingWriteOutcome::NoOp);
}

#[test]
fn an_equal_epoch_with_different_members_is_a_conflict() {
    let held = make_entry(TopologyMode::Singleton, vec![svc_id("v1")], None);
    let incoming = make_entry(TopologyMode::Singleton, vec![svc_id("v2")], None);
    assert_eq!(
        classify_binding_write(Some(&held), &incoming),
        BindingWriteOutcome::Conflict(TopologyEpoch::default())
    );
}

#[test]
fn a_lower_epoch_is_stale() {
    let held = TopologyEntry {
        epoch: TopologyEpoch(2),
        ..make_entry(TopologyMode::Singleton, vec![svc_id("v2")], None)
    };
    let incoming = TopologyEntry {
        epoch: TopologyEpoch(1),
        ..make_entry(TopologyMode::Singleton, vec![svc_id("v1")], None)
    };
    assert_eq!(
        classify_binding_write(Some(&held), &incoming),
        BindingWriteOutcome::Stale(TopologyEpoch(2))
    );
}

#[test]
fn an_absent_entry_applies() {
    let incoming = make_entry(TopologyMode::Singleton, vec![svc_id("v1")], None);
    assert_eq!(classify_binding_write(None, &incoming), BindingWriteOutcome::Applied);
}

#[test]
fn a_cache_ttl_difference_at_one_epoch_is_not_a_conflict() {
    let held = TopologyEntry {
        cache_ttl: Duration::from_secs(60),
        ..make_entry(TopologyMode::Singleton, vec![svc_id("v1")], None)
    };
    let incoming = TopologyEntry {
        cache_ttl: Duration::from_secs(120),
        ..make_entry(TopologyMode::Singleton, vec![svc_id("v1")], None)
    };
    assert_eq!(classify_binding_write(Some(&held), &incoming), BindingWriteOutcome::NoOp);
}

#[test]
fn test_range_sharding_routing() {
    let table = RangeRoutingTable {
        chunks: vec![
            RangeChunk {
                start_key: None,
                end_key: Some(b"bar".to_vec()),
                target: svc_id("shard-1"),
            },
            RangeChunk {
                start_key: Some(b"bar".to_vec()),
                end_key: Some(b"foo".to_vec()),
                target: svc_id("shard-2"),
            },
            RangeChunk {
                start_key: Some(b"foo".to_vec()),
                end_key: None,
                target: svc_id("shard-3"),
            },
        ],
    };

    let members = vec![svc_id("shard-1"), svc_id("shard-2"), svc_id("shard-3")];
    let reg = registry_with(vec![(
        local_key("app-1", "range-service"),
        make_entry(TopologyMode::Sharded, members, Some(ShardingStrategy::RangeSharding(table))),
    )]);
    let resolver = LogicalResolver::new(reg);
    let key = local_key("app-1", "range-service");

    // "a" < "bar" -> shard-1
    assert_eq!(resolver.resolve(&key, Some(b"a")).unwrap(), svc_id("shard-1"));
    // "bar" -> shard-2 (start_key inclusive)
    assert_eq!(resolver.resolve(&key, Some(b"bar")).unwrap(), svc_id("shard-2"));
    // "baz" -> shard-2 ("bar" <= "baz" < "foo")
    assert_eq!(resolver.resolve(&key, Some(b"baz")).unwrap(), svc_id("shard-2"));
    // "foo" -> shard-3 (start_key inclusive)
    assert_eq!(resolver.resolve(&key, Some(b"foo")).unwrap(), svc_id("shard-3"));
    // "z" -> shard-3
    assert_eq!(resolver.resolve(&key, Some(b"z")).unwrap(), svc_id("shard-3"));
}
