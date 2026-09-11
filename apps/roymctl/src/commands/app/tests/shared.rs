//! Tests for helpers defined in `app/mod.rs` (`resolve_under`,
//! `resolve_credentials`) — infrastructure shared by deploy and health.

use std::path::{Path, PathBuf};

use syneroym_app_orchestration::{models::SubstrateAlias, substrate_inventory::SubstrateEntry};

use super::*;

fn entry(identity: Option<&str>, ucan: Option<&str>) -> SubstrateEntry {
    SubstrateEntry {
        did: "did:key:z6MkExampleNodeA".to_string(),
        api_url: None,
        identity: identity.map(str::to_string),
        ucan: ucan.map(PathBuf::from),
        capabilities: None,
    }
}

#[test]
fn resolve_under_leaves_an_absolute_path_untouched() {
    let dir = Path::new("/roymctl/dir");
    let abs = Path::new("/etc/grants/edge-1.json");
    assert_eq!(resolve_under(dir, abs), abs);
}

#[test]
fn resolve_under_joins_a_relative_path_under_dir() {
    let dir = Path::new("/roymctl/dir");
    let rel = Path::new("grants/edge-1.json");
    assert_eq!(resolve_under(dir, rel), Path::new("/roymctl/dir/grants/edge-1.json"));
}

/// An entry overriding neither field inherits the global identity/ucan
/// pair as-is.
#[test]
fn resolve_credentials_falls_back_to_the_global_pair_when_the_entry_sets_neither() {
    let alias = SubstrateAlias::new("edge-1");
    let e = entry(None, None);
    let (id, ucan) = resolve_credentials(
        &alias,
        &e,
        Path::new("substrates.toml"),
        Path::new("/dir"),
        Some("global-op"),
        Some(Path::new("grants/global.json")),
    )
    .unwrap();
    assert_eq!(id, Some("global-op"));
    assert_eq!(ucan.as_deref(), Some(Path::new("grants/global.json")));
}

/// An entry overriding both fields together is always consistent,
/// regardless of what the globals are.
#[test]
fn resolve_credentials_uses_the_entrys_own_pair_when_it_sets_both() {
    let alias = SubstrateAlias::new("edge-1");
    let e = entry(Some("edge1-op"), Some("grants/edge-1.json"));
    let (id, ucan) = resolve_credentials(
        &alias,
        &e,
        Path::new("substrates.toml"),
        Path::new("/dir"),
        Some("global-op"),
        Some(Path::new("grants/global.json")),
    )
    .unwrap();
    assert_eq!(id, Some("edge1-op"));
    assert_eq!(ucan.as_deref(), Some(Path::new("/dir/grants/edge-1.json")));
}

/// The hazard: `identity` overridden, `ucan` left to fall back to a
/// *global* `--ucan` whose audience is the global identity, not this
/// entry's. Must be rejected, not silently paired.
#[test]
fn resolve_credentials_rejects_identity_override_with_a_global_ucan_present() {
    let alias = SubstrateAlias::new("edge-1");
    let e = entry(Some("edge1-op"), None);
    let err = resolve_credentials(
        &alias,
        &e,
        Path::new("substrates.toml"),
        Path::new("/dir"),
        Some("global-op"),
        Some(Path::new("grants/global.json")),
    )
    .unwrap_err();
    assert!(err.to_string().contains("edge-1"), "{err}");
}

/// The symmetric case: `ucan` overridden, `identity` left to fall back to
/// a global `--as` the entry's token was never minted for.
#[test]
fn resolve_credentials_rejects_ucan_override_with_a_global_identity_present() {
    let alias = SubstrateAlias::new("edge-1");
    let e = entry(None, Some("grants/edge-1.json"));
    let err = resolve_credentials(
        &alias,
        &e,
        Path::new("substrates.toml"),
        Path::new("/dir"),
        Some("global-op"),
        None,
    )
    .unwrap_err();
    assert!(err.to_string().contains("edge-1"), "{err}");
}
