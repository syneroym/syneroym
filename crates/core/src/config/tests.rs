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
