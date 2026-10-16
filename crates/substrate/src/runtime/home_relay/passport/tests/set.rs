use std::{fs, time::Duration};

use syneroym_control_plane::HomeRelayControl;
use syneroym_passport::{
    RegisterError,
    wire::{ErrorBody, HomeRelaySetRequest},
};
use syneroym_rpc::RpcError;
use tokio::time;

use super::{
    FIXED_RELAY, FakeIo, INFO_URL, START_SECS, StoredPassport, TestAgent, client::Failure,
    test_fixture,
};

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
    // Boot: fail try 1
    io.push_challenge_ok(SAMPLE_CHALLENGE);
    io.push_register_err(Failure::Refused(ErrorBody::from(&RegisterError::RateLimited)));

    let stored = StoredPassport {
        relay_url: FIXED_RELAY.to_owned(),
        info_url: INFO_URL.to_owned(),
        token: fixture1.child.clone(),
        ca_cert_pem: None,
    };
    let agent = TestAgent::start(io.clone(), &fixture1, Some(stored), false);
    time::sleep(Duration::from_millis(10)).await;

    // Fail try 2
    let status1 = agent.status().unwrap();
    let delay1 = status1.next_attempt_at.unwrap().saturating_sub(START_SECS);
    assert!((4..=6).contains(&delay1), "delay1 was {delay1}");

    io.push_challenge_ok(SAMPLE_CHALLENGE);
    io.push_register_err(Failure::Refused(ErrorBody::from(&RegisterError::RateLimited)));
    time::advance(Duration::from_secs(delay1)).await;
    time::sleep(Duration::from_millis(10)).await;

    let status2 = agent.status().unwrap();
    let delay2 = status2.next_attempt_at.unwrap().saturating_sub(START_SECS + delay1);
    assert!((8..=12).contains(&delay2), "delay2 was {delay2}");

    // Set passport 2; registration fails once
    let (_passport2, _passport_id2, child2) = fixture1.new_passport_for_node(30 * 24 * 3600);
    io.push_challenge_ok(SAMPLE_CHALLENGE);
    io.push_register_err(Failure::Refused(ErrorBody::from(&RegisterError::RateLimited)));

    let req = HomeRelaySetRequest {
        token: child2,
        info_url: INFO_URL.to_owned(),
        relay_url: None,
        ca_cert_pem: None,
    };
    let status_after_set = agent.control.set_passport(req).await.unwrap();
    let now_at_set = io.now_secs();
    let delay_after_set = status_after_set.next_attempt_at.unwrap().saturating_sub(now_at_set);
    // Reset backoff: delay is back to initial ~5s (not ~20s)
    assert!(
        (4..=6).contains(&delay_after_set),
        "backoff was not reset; delay was {delay_after_set}"
    );
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
    let dir = tempfile::tempdir().unwrap();

    // Put a regular file where the directory should be
    let obstacle = dir.path().join("obstacle");
    fs::write(&obstacle, b"file").unwrap();
    let invalid_path = obstacle.join("child").join("passport.json");

    let agent = TestAgent::start_at_path(io.clone(), &fixture, invalid_path, false);
    time::sleep(Duration::from_millis(10)).await;

    let req = HomeRelaySetRequest {
        token: fixture.child,
        info_url: INFO_URL.to_owned(),
        relay_url: None,
        ca_cert_pem: None,
    };
    let err = agent.control.set_passport(req).await.unwrap_err();
    match err {
        RpcError::InternalError(msg) => {
            assert!(
                msg.contains("cannot store"),
                "expected 'cannot store' in error message: {msg}"
            );
        }
        other => panic!("expected InternalError, got {other:?}"),
    }
}
