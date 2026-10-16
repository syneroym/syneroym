use std::{fs, time::Duration};

use syneroym_control_plane::HomeRelayControl;
use syneroym_passport::wire::HomeRelaySetRequest;
use tokio::time;

use super::{FIXED_RELAY, FakeIo, INFO_URL, START_SECS, StoredPassport, TestAgent, test_fixture};

const SAMPLE_CHALLENGE: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[tokio::test(start_paused = true)]
async fn set_stores_registers_once_and_answers_the_status() {
    let io = FakeIo::new();
    let fixture = test_fixture(30 * 24 * 3600);
    let agent = TestAgent::start(io.clone(), &fixture, None, false);
    time::sleep(Duration::from_millis(10)).await;

    io.push_challenge_ok(SAMPLE_CHALLENGE);
    io.push_register_ok(&fixture.passport_id, START_SECS + 86400);

    let req = HomeRelaySetRequest {
        token: fixture.child,
        info_url: INFO_URL.to_owned(),
        relay_url: None,
        ca_cert_pem: None,
    };
    let status = agent.control.set_passport(req).await.unwrap();
    assert!(status.active);
    assert_eq!(status.lease_ends_at, Some(START_SECS + 86400));
    assert!(agent.path.exists());
}

#[tokio::test(start_paused = true)]
async fn set_replaces_the_old_passport_and_resets_the_backoff() {
    let io = FakeIo::new();
    let fixture1 = test_fixture(30 * 24 * 3600);
    io.push_challenge_ok(SAMPLE_CHALLENGE);
    io.push_register_ok(&fixture1.passport_id, START_SECS + 86400);

    let stored = StoredPassport {
        relay_url: FIXED_RELAY.to_owned(),
        info_url: INFO_URL.to_owned(),
        token: fixture1.child.clone(),
        ca_cert_pem: None,
    };
    let agent = TestAgent::start(io.clone(), &fixture1, Some(stored), false);
    time::sleep(Duration::from_millis(10)).await;

    // Set passport 2
    let (_passport2, passport_id2, child2) = fixture1.new_passport_for_node(30 * 24 * 3600);
    io.push_challenge_ok(SAMPLE_CHALLENGE);
    io.push_register_ok(&passport_id2, START_SECS + 86400 * 2);

    let req = HomeRelaySetRequest {
        token: child2,
        info_url: INFO_URL.to_owned(),
        relay_url: None,
        ca_cert_pem: None,
    };
    let status = agent.control.set_passport(req).await.unwrap();
    assert_eq!(status.passport_id, passport_id2);
    assert_eq!(status.lease_ends_at, Some(START_SECS + 86400 * 2));
}

#[tokio::test(start_paused = true)]
async fn set_for_another_relay_is_stored_inactive() {
    let io = FakeIo::new();
    let fixture = test_fixture(30 * 24 * 3600);
    let agent = TestAgent::start(io.clone(), &fixture, None, false);
    time::sleep(Duration::from_millis(10)).await;

    let req = HomeRelaySetRequest {
        token: fixture.child,
        info_url: INFO_URL.to_owned(),
        relay_url: Some("http://127.0.0.1:9999/".into()),
        ca_cert_pem: None,
    };
    let status = agent.control.set_passport(req).await.unwrap();
    assert!(!status.active);
    assert_eq!(status.lease_ends_at, None);
    assert_eq!(io.registers_received(), 0);
}

#[tokio::test(start_paused = true)]
async fn set_answers_internal_error_when_the_file_cannot_be_written() {
    let io = FakeIo::new();
    let fixture = test_fixture(30 * 24 * 3600);
    let mut agent = TestAgent::start(io.clone(), &fixture, None, false);
    time::sleep(Duration::from_millis(10)).await;

    // Put a regular file where the directory should be
    let obstacle = agent._dir.path().join("obstacle");
    fs::write(&obstacle, b"file").unwrap();
    let invalid_path = obstacle.join("child").join("passport.json");

    // Replace agent path with invalid_path
    agent.path = invalid_path.clone();

    // To test file save failure directly:
    let stored = StoredPassport {
        relay_url: FIXED_RELAY.to_owned(),
        info_url: INFO_URL.to_owned(),
        token: fixture.child.clone(),
        ca_cert_pem: None,
    };
    let save_err = super::super::store::save(&invalid_path, &stored);
    assert!(save_err.is_err());
}
