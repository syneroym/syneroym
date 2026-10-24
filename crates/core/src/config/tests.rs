#![allow(clippy::cognitive_complexity)]

use std::path::Path;

use super::*;

#[test]
fn test_resolve_paths() {
    let mut config = SubstrateConfig {
        app_data_dir: PathBuf::from("/tmp/app_data"),
        app_local_data_dir: PathBuf::from("/tmp/local_data"),
        app_config_dir: PathBuf::from("/tmp/config"),
        ..Default::default()
    };

    config.identity.key = Some(PathBuf::from("substrate.key"));
    config.identity.agreement = Some(PathBuf::from("agreement.json"));
    config.storage.db_dir = PathBuf::from("db");

    config.roles.coordinator = Some(CoordinatorRole {
        tls: Some(TlsConfig {
            cert_path: PathBuf::from("cert.pem"),
            key_path: PathBuf::from("key.pem"),
        }),
        ..Default::default()
    });

    config.resolve_paths();

    assert_eq!(config.identity.key.unwrap(), Path::new("/tmp/app_data/substrate.key"));
    assert_eq!(config.identity.agreement.unwrap(), Path::new("/tmp/app_data/agreement.json"));
    assert_eq!(config.storage.db_dir, Path::new("/tmp/local_data/db"));
    assert_eq!(config.storage.blob_store.local_root, Path::new("/tmp/local_data/blob_objects"));

    let tls = config.roles.coordinator.unwrap().tls.unwrap();
    assert_eq!(tls.cert_path, Path::new("/tmp/config/cert.pem"));
    assert_eq!(tls.key_path, Path::new("/tmp/config/key.pem"));
}

#[test]
fn test_blob_store_config_defaults() {
    let config = BlobStoreConfig::default();
    assert_eq!(config.backend, BlobBackend::Local);
    assert_eq!(config.local_root, Path::new("blob_objects"));
    assert_eq!(config.max_blob_bytes, 100 * 1024 * 1024);
    assert_eq!(config.max_service_total_bytes, None);
    assert!(config.s3.is_none());
}

#[test]
fn test_messaging_config_defaults() {
    let config = MessagingConfig::default();
    assert_eq!(config.channel_capacity, 1024);
}

#[test]
fn test_streaming_config_defaults() {
    let config = StreamingConfig::default();
    assert_eq!(config.max_concurrent_streams_per_service, 8);
}

/// The anchor-refresh cadence has to sit comfortably inside
/// the 24-hour window after which an anchor stops verifying at every
/// consumer -- a refresh interval at or above that window would let
/// anchors lapse between passes no matter how reliably the loop runs.
#[test]
fn supervisor_role_master_anchor_refresh_interval_secs_has_a_day_scale_default() {
    const ANCHOR_VALIDITY_SECS: u64 = 24 * 3600;
    let role = SupervisorRole::default();
    assert!(
        role.master_anchor_refresh_interval_secs * 2 <= ANCHOR_VALIDITY_SECS,
        "the refresh interval ({}s) must leave at least 2x margin inside the anchor's \
         {ANCHOR_VALIDITY_SECS}s validity window",
        role.master_anchor_refresh_interval_secs
    );
    assert!(
        role.master_anchor_refresh_interval_secs >= 3600,
        "and must not be so short that a fact needing to move once a day is republished every few \
         minutes"
    );
}

/// ADR-0022 §2: an app instance's Tier-1 record reuses this same
/// interval (no second config field), against `EndpointInfo`'s own
/// 30-day `not_after`, not the anchor's 24-hour one -- the number that
/// matters there is how many *consecutive failed* refreshes a
/// previously published record survives before it lapses, not the
/// interval alone. At the default 12h cadence against 30 days, sixty.
#[test]
fn tier1_refresh_survives_sixty_consecutive_failures_against_the_default_interval() {
    let role = SupervisorRole::default();
    let interval = role.master_anchor_refresh_interval_secs;
    let survivable = crate::dht_registry::DEFAULT_ENDPOINT_NOT_AFTER_SECS / interval;
    assert_eq!(
        survivable, 60,
        "sixty consecutive failed refreshes at the default cadence is the number an operator is \
         told they have before a locked vault costs discoverability"
    );
}

/// The supervisor's own certificate lifetime is short by design and is
/// deliberately *not* `roymctl`'s attended-posture default (24h), which
/// serves an operator with no renewal loop behind them.
#[test]
fn supervisor_role_renewed_cert_expires_hours_is_short_and_capped() {
    let role = SupervisorRole::default();
    assert!(role.renewed_cert_expires_hours < 24, "renewal makes short-lived the default");
    assert!(role.renewed_cert_expires_hours >= 1, "and it must still be a usable window");
    assert!(role.max_renewals_per_pass >= 1, "a cap of 0 would renew nothing, ever");
}

#[test]
fn test_resolve_paths_absolute_untouched() {
    let mut config =
        SubstrateConfig { app_data_dir: PathBuf::from("/tmp/app_data"), ..Default::default() };
    let abs_path = if cfg!(windows) { "C:\\abs\\key" } else { "/abs/key" };
    config.identity.key = Some(PathBuf::from(abs_path));

    config.resolve_paths();

    assert_eq!(config.identity.key.unwrap(), Path::new(abs_path));
}

#[test]
fn iroh_relay_url_is_none_without_the_section() {
    let config = toml::from_str::<SubstrateConfig>("").unwrap();
    assert_eq!(config.parent_coordinator.iroh.url, None);
}

#[test]
fn iroh_relay_url_is_none_in_an_empty_section() {
    let config = toml::from_str::<SubstrateConfig>("[parent_coordinator.iroh]\n").unwrap();
    assert_eq!(config.parent_coordinator.iroh.url, None);
}

#[test]
fn iroh_relay_url_is_read_from_the_section() {
    let toml = r#"
[parent_coordinator.iroh]
url = "http://relay.example:7964"
"#;
    let config = toml::from_str::<SubstrateConfig>(toml).unwrap();
    assert_eq!(config.parent_coordinator.iroh.url.as_deref(), Some("http://relay.example:7964"));
}

#[test]
fn old_coordinator_discovery_url_key_is_ignored() {
    let toml = r#"
[substrate]
coordinator_discovery_url = "http://x"
"#;
    let config = toml::from_str::<SubstrateConfig>(toml).unwrap();
    assert!(config.substrate.communication_interfaces.contains(&"iroh".to_string()));
}

#[test]
fn community_registry_coordinators_list_parses_from_toml() {
    let toml = r#"
[roles.community_registry]
list_max_age_secs = 3600

[[roles.community_registry.coordinators]]
transport = "iroh-relay"
url = "https://relay-1.example.org"
info_url = "https://relay-1.example.org:7974"
operator = "First Operator"

[[roles.community_registry.coordinators]]
transport = "iroh-relay"
url = "https://relay-2.example.org"
info_url = "https://relay-2.example.org:7974"
operator = "Second Operator"
"#;
    let config = toml::from_str::<SubstrateConfig>(toml).unwrap();
    let reg = config.roles.community_registry.unwrap();
    assert_eq!(reg.list_max_age_secs, 3600);
    assert_eq!(reg.coordinators.len(), 2);
    assert_eq!(reg.coordinators[0].url, "https://relay-1.example.org");
    assert_eq!(reg.coordinators[1].operator, "Second Operator");
}

#[test]
fn community_registry_coordinators_defaults_when_absent() {
    let toml = r#"
[roles.community_registry]
"#;
    let config = toml::from_str::<SubstrateConfig>(toml).unwrap();
    let reg = config.roles.community_registry.unwrap();
    assert_eq!(reg.list_max_age_secs, 86_400);
    assert!(reg.coordinators.is_empty());
}

#[test]
fn community_registry_coordinator_unknown_transport_fails() {
    let toml = r#"
[[roles.community_registry.coordinators]]
transport = "webrtc"
url = "https://relay.example.org"
info_url = "https://relay.example.org:7974"
operator = "Operator"
"#;
    assert!(toml::from_str::<SubstrateConfig>(toml).is_err());
}

#[test]
fn a_mailbox_role_parses_with_its_defaults() {
    let toml = r#"
[roles.mailbox]
"#;
    let config = toml::from_str::<SubstrateConfig>(toml).unwrap();
    assert_eq!(config.roles.mailbox, Some(MailboxRole::default()));

    let override_toml = r#"
[roles.mailbox]
max_queue_messages = 42
max_settled_receipts = 7
"#;
    let override_config = toml::from_str::<SubstrateConfig>(override_toml).unwrap();
    let expected =
        MailboxRole { max_queue_messages: 42, max_settled_receipts: 7, ..Default::default() };
    assert_eq!(override_config.roles.mailbox, Some(expected));
}

#[test]
fn no_mailbox_section_means_no_mailbox() {
    let config = toml::from_str::<SubstrateConfig>("").unwrap();
    assert_eq!(config.roles.mailbox, None);
}
