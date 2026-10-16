use std::{fs, sync::Arc, time::Duration};

use syneroym_control_plane::HomeRelayControl;
use syneroym_passport::{
    RegisterError,
    wire::{ErrorBody, HomeRelaySetRequest, UNREACHABLE_CODE},
};
use tokio::time;

use super::{
    FIXED_RELAY, FakeIo, INFO_URL, START_SECS, Scripted, StoredPassport, TestAgent,
    client::Failure, test_fixture,
};
use crate::runtime::home_relay::test_logs;

const SAMPLE_CHALLENGE: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[tokio::test(start_paused = true)]
async fn no_stored_passport_releases_the_relay_at_once() {
    let io = FakeIo::new();
    let fixture = test_fixture(30 * 24 * 3600);
    let agent = TestAgent::start(io.clone(), &fixture, None, false);
    time::sleep(Duration::from_millis(50)).await;
    assert!(agent.boot_done());
    assert_eq!(io.challenges_received(), 0);
    assert_eq!(io.registers_received(), 0);
    assert_eq!(agent.status(), None);
}

#[tokio::test(start_paused = true)]
async fn boot_registers_before_the_relay_is_inserted() {
    let io = FakeIo::new();
    io.push_challenge_ok(SAMPLE_CHALLENGE);
    let fixture = test_fixture(30 * 24 * 3600);
    let notify = Arc::new(tokio::sync::Notify::new());
    io.push_register_hanging(
        notify.clone(),
        Ok(syneroym_passport::wire::RegisterResponse {
            passport_id: fixture.passport_id.clone(),
            lease_ends_at: START_SECS + 86400,
        }),
    );

    let stored = StoredPassport {
        relay_url: FIXED_RELAY.to_owned(),
        info_url: INFO_URL.to_owned(),
        token: fixture.child.clone(),
        ca_cert_pem: None,
    };
    let agent = TestAgent::start(io.clone(), &fixture, Some(stored), false);

    time::sleep(Duration::from_millis(10)).await;
    assert_eq!(io.challenges_received(), 1);
    assert_eq!(io.registers_received(), 1);
    // The register call was issued, but boot_done has not fired yet while waiting
    // for the response.
    assert!(!agent.boot_done());

    // Release the register response
    notify.notify_one();
    time::sleep(Duration::from_millis(10)).await;
    assert!(agent.boot_done());
    let status = agent.status().expect("status available");
    assert!(status.active);
    assert_eq!(status.lease_ends_at, Some(START_SECS + 86400));
}

#[tokio::test(start_paused = true)]
async fn boot_inserts_the_relay_after_three_failed_tries() {
    let io = FakeIo::new();
    for _ in 0..3 {
        io.push_challenge_err(Failure::Unreachable("network down".into()));
    }
    let fixture = test_fixture(30 * 24 * 3600);
    let stored = StoredPassport {
        relay_url: FIXED_RELAY.to_owned(),
        info_url: INFO_URL.to_owned(),
        token: fixture.child.clone(),
        ca_cert_pem: None,
    };
    let agent = TestAgent::start(io.clone(), &fixture, Some(stored), false);

    // Try 1 fails immediately. Backoff 5 s.
    time::sleep(Duration::from_millis(10)).await;
    assert!(!agent.boot_done());
    assert_eq!(io.challenges_received(), 1);

    // Advance 5.5 s -> Try 2 fails. Backoff 10 s.
    time::advance(Duration::from_millis(5500)).await;
    time::sleep(Duration::from_millis(10)).await;
    assert!(!agent.boot_done());
    assert_eq!(io.challenges_received(), 2);

    // Advance 11 s -> Try 3 fails.
    time::advance(Duration::from_millis(11000)).await;
    time::sleep(Duration::from_millis(10)).await;
    assert!(agent.boot_done());
    assert_eq!(io.challenges_received(), 3);

    // Try 4 runs later in normal agent loop.
    io.push_challenge_err(Failure::Unreachable("still down".into()));
    time::advance(Duration::from_secs(30)).await;
    time::sleep(Duration::from_millis(10)).await;
    assert_eq!(io.challenges_received(), 4);
}

#[tokio::test(start_paused = true)]
async fn boot_inserts_the_relay_after_a_settled_refusal() {
    let io = FakeIo::new();
    io.push_challenge_ok(SAMPLE_CHALLENGE);
    io.push_register_err(Failure::Refused(ErrorBody::from(&RegisterError::Denied)));

    let fixture = test_fixture(30 * 24 * 3600);
    let stored = StoredPassport {
        relay_url: FIXED_RELAY.to_owned(),
        info_url: INFO_URL.to_owned(),
        token: fixture.child.clone(),
        ca_cert_pem: None,
    };
    let agent = TestAgent::start(io.clone(), &fixture, Some(stored), false);

    time::sleep(Duration::from_millis(10)).await;
    assert!(agent.boot_done());
    assert_eq!(io.registers_received(), 1);
    let status = agent.status().expect("status");
    assert_eq!(status.last_error.unwrap().code, "denied");
}

#[tokio::test(start_paused = true)]
async fn inactive_passport_is_kept_and_not_used() {
    let io = FakeIo::new();
    let fixture = test_fixture(30 * 24 * 3600);
    let stored = StoredPassport {
        relay_url: "http://127.0.0.1:9999/".to_owned(),
        info_url: INFO_URL.to_owned(),
        token: fixture.child.clone(),
        ca_cert_pem: None,
    };
    let agent = TestAgent::start(io.clone(), &fixture, Some(stored), false);

    time::sleep(Duration::from_millis(10)).await;
    assert!(agent.boot_done());
    assert_eq!(io.challenges_received(), 0);
    let status = agent.status().expect("status");
    assert!(!status.active);
    assert_eq!(status.next_attempt_at, None);
}

#[tokio::test(start_paused = true)]
async fn expired_stored_passport_makes_no_attempt() {
    let io = FakeIo::new();
    let fixture = test_fixture(10);
    let stored = StoredPassport {
        relay_url: FIXED_RELAY.to_owned(),
        info_url: INFO_URL.to_owned(),
        token: fixture.child.clone(),
        ca_cert_pem: None,
    };
    time::advance(Duration::from_secs(20)).await;
    let agent = TestAgent::start(io.clone(), &fixture, Some(stored), false);

    time::sleep(Duration::from_millis(10)).await;
    assert!(agent.boot_done());
    assert_eq!(io.challenges_received(), 0);
    let status = agent.status().expect("status");
    assert_eq!(status.last_error.unwrap().code, "expired");
}

#[tokio::test(start_paused = true)]
async fn unreadable_file_is_ignored() {
    let io = FakeIo::new();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("home_relay_passport.json");
    fs::write(&path, b"not valid json").unwrap();
    let fixture = test_fixture(30 * 24 * 3600);
    let agent = TestAgent::start_at_path(io.clone(), &fixture, path, false);

    time::sleep(Duration::from_millis(10)).await;
    assert!(agent.boot_done());
    assert_eq!(agent.status(), None);
}

#[tokio::test(start_paused = true)]
async fn boot_round_ends_after_its_budget() {
    let io = FakeIo::new();
    io.push_challenge(Scripted::Hang);
    let fixture = test_fixture(30 * 24 * 3600);
    let stored = StoredPassport {
        relay_url: FIXED_RELAY.to_owned(),
        info_url: INFO_URL.to_owned(),
        token: fixture.child.clone(),
        ca_cert_pem: None,
    };
    let agent = TestAgent::start(io.clone(), &fixture, Some(stored), false);
    time::sleep(Duration::from_millis(10)).await;

    time::advance(Duration::from_secs(29)).await;
    time::sleep(Duration::from_millis(10)).await;
    assert!(!agent.boot_done());

    time::advance(Duration::from_secs(2)).await;
    time::sleep(Duration::from_millis(10)).await;
    assert!(agent.boot_done());
    let status = agent.status().expect("status");
    assert_eq!(status.last_error.unwrap().code, UNREACHABLE_CODE);
}

#[tokio::test(start_paused = true)]
async fn closing_the_endpoint_stops_a_running_attempt() {
    let io = FakeIo::new();
    io.push_challenge_ok(SAMPLE_CHALLENGE);
    io.push_register(Scripted::Hang);

    let fixture = test_fixture(30 * 24 * 3600);
    let stored = StoredPassport {
        relay_url: FIXED_RELAY.to_owned(),
        info_url: INFO_URL.to_owned(),
        token: fixture.child.clone(),
        ca_cert_pem: None,
    };
    let agent = TestAgent::start(io.clone(), &fixture, Some(stored), false);

    time::sleep(Duration::from_millis(10)).await;
    assert_eq!(io.registers_received(), 1);

    io.close();
    let join_res = time::timeout(Duration::from_millis(50), agent.handle).await;
    assert!(join_res.is_ok(), "agent task did not terminate on close");
    assert_eq!(io.registers_received(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_set_during_boot_runs_after_the_round() {
    let io = FakeIo::new();
    io.push_challenge(Scripted::Hang);
    let fixture = test_fixture(30 * 24 * 3600);
    let stored = StoredPassport {
        relay_url: FIXED_RELAY.to_owned(),
        info_url: INFO_URL.to_owned(),
        token: fixture.child.clone(),
        ca_cert_pem: None,
    };
    let agent = TestAgent::start(io.clone(), &fixture, Some(stored), false);
    time::sleep(Duration::from_millis(10)).await;

    let control = agent.control.clone();
    let (_passport2, passport_id2, child2) = fixture.new_passport_for_node(30 * 24 * 3600);
    let set_handle = tokio::spawn(async move {
        control
            .set_passport(HomeRelaySetRequest {
                token: child2,
                info_url: INFO_URL.to_owned(),
                relay_url: None,
                ca_cert_pem: None,
            })
            .await
    });

    io.push_challenge_ok(SAMPLE_CHALLENGE);
    io.push_register_ok(&passport_id2, START_SECS + 86400);

    time::advance(Duration::from_secs(31)).await;
    time::sleep(Duration::from_millis(10)).await;
    assert!(agent.boot_done());

    let res = set_handle.await.unwrap().expect("set succeeds");
    assert!(res.active);
}

#[tokio::test(start_paused = true)]
async fn an_active_passport_with_a_coordinator_role_warns() {
    let (logs, _guard) = test_logs::capture();
    let io = FakeIo::new();
    io.push_challenge_ok(SAMPLE_CHALLENGE);
    let fixture = test_fixture(30 * 24 * 3600);
    io.push_register_ok(&fixture.passport_id, START_SECS + 86400);

    let stored = StoredPassport {
        relay_url: FIXED_RELAY.to_owned(),
        info_url: INFO_URL.to_owned(),
        token: fixture.child.clone(),
        ca_cert_pem: None,
    };
    let agent = TestAgent::start(io.clone(), &fixture, Some(stored), true);

    time::sleep(Duration::from_millis(10)).await;
    assert!(agent.boot_done());
    assert!(test_logs::warnings_about(&logs, "coordinator role also uses") > 0);
}
