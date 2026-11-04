#![allow(clippy::unwrap_used)]

use std::{fs, path::Path, sync::Arc};

use syneroym_core::dht_registry::client::RegistryClient;
use syneroym_identity::{Identity, substrate};
use tempfile::tempdir;

use super::AuthService;

/// A service that reads keys from `dir`. `login_local` never resolves a
/// master anchor, so the production resolver (no DHT, no registry URL) is
/// never called.
fn service() -> AuthService {
    let node_did = substrate::derive_did_key(&Identity::generate().unwrap().public_key());
    let resolver = Arc::new(RegistryClient::new(false, None));
    AuthService::new(Identity::generate().unwrap(), node_did, 3600, 60, None, resolver)
}

fn write_alice(dir: &Path) -> String {
    let alice = Identity::generate().unwrap();
    fs::write(dir.join("alice.key"), alice.to_bytes()).unwrap();
    substrate::derive_did_key(&alice.public_key())
}

#[test]
fn a_known_identity_gets_a_session_for_its_own_did() {
    let dir = tempdir().unwrap();
    let alice_did = write_alice(dir.path());

    let grant = service().login_local(dir.path(), "alice").unwrap();

    assert_eq!(grant.person_did, alice_did);
}

#[test]
fn an_unknown_identity_is_refused_with_401() {
    let dir = tempdir().unwrap();
    write_alice(dir.path());

    assert_eq!(service().login_local(dir.path(), "bob").unwrap_err().0, 401);
}

#[test]
fn a_name_that_could_leave_the_directory_is_refused_with_400() {
    let dir = tempdir().unwrap();
    write_alice(dir.path());
    let service = service();

    for name in ["../alice", "subdir/alice", "sub\\alice", "", "  "] {
        assert_eq!(service.login_local(dir.path(), name).unwrap_err().0, 400, "name {name:?}");
    }
}
