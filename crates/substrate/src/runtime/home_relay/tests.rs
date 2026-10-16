use std::{path::PathBuf, time::Duration};

use syneroym_core::config::SubstrateConfig;
use tokio::time;

use super::{Backoff, HomeRelaySource, WARN_INTERVAL, WarnLimiter};

fn config(url: Option<&str>, registry_url: Option<&str>) -> SubstrateConfig {
    let mut config = SubstrateConfig::default();
    config.parent_coordinator.iroh.url = url.map(Into::into);
    config.substrate.registry_url = registry_url.map(Into::into);
    config
}

#[test]
fn no_url_and_no_registry_is_degraded_mode() {
    let source = HomeRelaySource::from_config(&config(None, None)).unwrap();
    assert_eq!(source, None);
}

#[test]
fn bad_url_stops_startup() {
    let err = HomeRelaySource::from_config(&config(Some("localhost:7964"), None))
        .unwrap_err()
        .to_string();
    assert!(err.contains("parent_coordinator.iroh.url"), "expected setting in error: {err}");
    assert!(err.contains("localhost:7964"), "expected bad value in error: {err}");
}

#[test]
fn url_is_a_fixed_source() {
    let mut config = config(Some("http://127.0.0.1:7964"), Some("http://registry.example"));
    config.storage.db_dir = PathBuf::from("/x/db");
    let source = HomeRelaySource::from_config(&config).unwrap();
    let want_url = "http://127.0.0.1:7964/".parse().unwrap();
    assert_eq!(
        source,
        Some(HomeRelaySource::Fixed {
            url: want_url,
            passport_path: PathBuf::from("/x/db/home_relay_passport.json"),
            coordinator_on_parent: false,
        })
    );
}

#[test]
fn fixed_source_notes_a_coordinator_role() {
    let mut config = config(Some("http://127.0.0.1:7964"), None);
    config.roles.coordinator = Some(syneroym_core::config::CoordinatorRole {
        iroh: Some(syneroym_core::config::CoordinatorIrohConfig::default()),
        ..Default::default()
    });
    let source = HomeRelaySource::from_config(&config).unwrap();
    let want_url = "http://127.0.0.1:7964/".parse().unwrap();
    assert_eq!(
        source,
        Some(HomeRelaySource::Fixed {
            url: want_url,
            passport_path: PathBuf::from("db/home_relay_passport.json"),
            coordinator_on_parent: true,
        })
    );
}

#[test]
fn no_url_with_registry_is_a_registry_source() {
    let mut config = config(None, Some("HTTP://Reg.Example:80/"));
    config.storage.db_dir = PathBuf::from("/x/db");
    let source = HomeRelaySource::from_config(&config).unwrap();
    assert_eq!(
        source,
        Some(HomeRelaySource::Registry {
            registry_url: "http://reg.example".into(),
            cache_path: PathBuf::from("/x/db/coordinators.json"),
        })
    );
}

#[test]
fn bad_registry_url_stops_startup() {
    let err =
        HomeRelaySource::from_config(&config(None, Some("reg.example"))).unwrap_err().to_string();
    assert!(err.contains("substrate.registry_url"), "expected setting in error: {err}");
    assert!(err.contains("reg.example"), "expected bad value in error: {err}");
}

#[test]
fn webrtc_only_node_ignores_a_bad_registry_url() {
    let mut config = config(None, Some("reg.example"));
    config.substrate.communication_interfaces = vec!["webrtc".into()];
    assert_eq!(HomeRelaySource::from_config(&config).unwrap(), None);
}

#[tokio::test(start_paused = true)]
async fn warn_limiter_allows_one_warning_every_interval() {
    let mut limiter = WarnLimiter::default();
    assert!(limiter.should_warn());

    time::sleep(WARN_INTERVAL - Duration::from_secs(1)).await;
    assert!(!limiter.should_warn());

    time::sleep(Duration::from_secs(1)).await;
    assert!(limiter.should_warn());
    assert!(!limiter.should_warn());
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

#[cfg(unix)]
#[test]
fn a_left_over_tmp_file_gets_mode_0600() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("secret.json");
    let tmp = target.with_extension("tmp");
    std::fs::write(&tmp, b"old").unwrap();
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644)).unwrap();
    super::write_file_atomic(&target, b"new").unwrap();
    let meta = std::fs::metadata(&target).unwrap();
    assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    assert_eq!(std::fs::read(&target).unwrap(), b"new");
}
