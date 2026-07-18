#![allow(clippy::unwrap_used, clippy::panic)]
use std::time::{SystemTime, UNIX_EPOCH};

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use serde_json::Map;
use syneroym_identity::{Identity, substrate::derive_did_key};
use syneroym_ucan::{
    Ability, Capability, CapabilityToken, ChainVerifyOpts, ResourceUri, verify_chain,
};

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

/// Verification of a 2-link chain (`owner` -> `alice` -> `bob`, one
/// attenuation hop), cache-cold -- the M04A performance budget's "UCAN
/// chain verification (cache-cold) < 5 ms p99" (task.md).
fn bench_chain_verify(c: &mut Criterion) {
    let owner = Identity::generate().unwrap();
    let alice = Identity::generate().unwrap();
    let bob = Identity::generate().unwrap();
    let admin_root = derive_did_key(&owner.public_key());
    let bob_did = derive_did_key(&bob.public_key());
    let resource = ResourceUri::service("app1", "svc1");

    let owner_to_alice = CapabilityToken::issue(
        &owner,
        &derive_did_key(&alice.public_key()),
        vec![Capability {
            with: resource.clone(),
            can: Ability(Ability::DATA_LAYER_ADMIN.to_string()),
            caveats: None,
        }],
        Map::new(),
        3600,
        vec![],
    )
    .unwrap();
    let alice_to_bob = CapabilityToken::issue(
        &alice,
        &bob_did,
        vec![Capability {
            with: resource,
            can: Ability(Ability::DATA_LAYER_WRITE.to_string()),
            caveats: None,
        }],
        Map::new(),
        3600,
        vec![owner_to_alice],
    )
    .unwrap();

    let is_root = |iss: &str, _cap: &Capability| iss == admin_root;

    c.bench_function("verify_chain_two_link", |b| {
        b.iter(|| {
            let opts = ChainVerifyOpts {
                expected_audience_did: &bob_did,
                is_trusted_root: &is_root,
                now_secs: now_secs(),
            };
            black_box(verify_chain(black_box(&alice_to_bob), &opts).unwrap());
        });
    });
}

criterion_group!(benches, bench_chain_verify);
criterion_main!(benches);
