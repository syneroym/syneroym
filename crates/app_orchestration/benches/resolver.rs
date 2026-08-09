#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! `LogicalResolver::resolve`'s "no network hop" budget (M05A A2/A5e,
//! D-A5e-13): the name -> master-DID step must stay an in-process cache
//! lookup. The backlog row this resolves named `crates/router/benches/
//! proxy.rs` as the bench's home, which does not fit -- `ProxyRouter` has
//! no dependency target at all; `CallTarget::Dependency` is resolved in the
//! WASM host capability, before a `ProxyRequest` ever exists (`empty_
//! resolver()` is what that bench's router uses). This is `resolve`'s own
//! home instead, over the three cases the budget's wording and A5e's
//! `replicas` between them actually name: a cache hit, a cache miss that
//! goes through the registry, and a two-member `Redundant` round-robin
//! (the first slice to make that mode real).

use std::sync::Arc;

use criterion::{Criterion, criterion_group, criterion_main};
use syneroym_app_orchestration::{
    LogicalResolver, StaticInventory, TopologyEntry, TopologyEpoch, TopologyKey,
    models::{AppInstanceId, LogicalServiceName, ServiceId, TopologyMode},
};

fn topology_key(instance: &str, service: &str) -> TopologyKey {
    TopologyKey::local(AppInstanceId::new(instance), LogicalServiceName::new(service))
}

fn entry(mode: TopologyMode, members: Vec<ServiceId>) -> TopologyEntry {
    TopologyEntry {
        mode,
        members,
        sharding_strategy: None,
        epoch: TopologyEpoch(1),
        cache_ttl: std::time::Duration::from_secs(60),
        not_after: None,
    }
}

/// The cache-hit path: `resolve` after the topology is already warm.
fn bench_cache_hit(c: &mut Criterion) {
    let key = topology_key("bench-inst", "backend");
    let registry = Arc::new(StaticInventory::new());
    let resolver = LogicalResolver::new(registry);
    resolver.register(
        key.clone(),
        entry(TopologyMode::Singleton, vec![ServiceId::new("did:key:hMember")]),
    );
    // Warm the cache once before timing.
    resolver.resolve(&key, None).unwrap();

    c.bench_function("logical_resolver_resolve_cache_hit", |b| {
        b.iter(|| resolver.resolve(&key, None).unwrap());
    });
}

/// The cache-miss path: `invalidate` before every call forces a re-read
/// through the registry (`AppRegistry::get`), which is the "network hop"
/// this budget exists to bound in-process against.
fn bench_cache_miss_through_registry(c: &mut Criterion) {
    let key = topology_key("bench-inst", "backend");
    let registry = Arc::new(StaticInventory::new());
    let resolver = LogicalResolver::new(registry);
    resolver.register(
        key.clone(),
        entry(TopologyMode::Singleton, vec![ServiceId::new("did:key:hMember")]),
    );

    c.bench_function("logical_resolver_resolve_cache_miss_through_registry", |b| {
        b.iter(|| {
            resolver.invalidate(&key);
            resolver.resolve(&key, None).unwrap()
        });
    });
}

/// M05A A5e: the first slice where a `Redundant` topology is real
/// (`replicas > 1`). Unkeyed resolution round-robins in-process
/// (`ResolvedTopology::rr_counter`) -- still no network hop, just an
/// atomic increment on top of the cache-hit path above.
fn bench_two_member_redundant_round_robin(c: &mut Criterion) {
    let key = topology_key("bench-inst", "backend");
    let registry = Arc::new(StaticInventory::new());
    let resolver = LogicalResolver::new(registry);
    resolver.register(
        key.clone(),
        entry(
            TopologyMode::Redundant,
            vec![ServiceId::new("did:key:hMember0"), ServiceId::new("did:key:hMember1")],
        ),
    );
    resolver.resolve(&key, None).unwrap();

    c.bench_function("logical_resolver_resolve_redundant_round_robin", |b| {
        b.iter(|| resolver.resolve(&key, None).unwrap());
    });
}

criterion_group!(
    benches,
    bench_cache_hit,
    bench_cache_miss_through_registry,
    bench_two_member_redundant_round_robin
);
criterion_main!(benches);
