use std::time::Duration;

use syneroym_passport::{RegisterError, wire::ErrorBody};
use tokio::time;

use super::{
    FIXED_RELAY, FakeIo, INFO_URL, START_SECS, StoredPassport, TestAgent,
    client::{Failure, Target, register_once},
    test_fixture,
};

const SAMPLE_CHALLENGE: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[tokio::test(start_paused = true)]
async fn a_stale_challenge_is_retried_once_at_once() {
    let io = FakeIo::new();
    let fixture = test_fixture(30 * 24 * 3600);
    // 1st challenge -> register fails with unknown_challenge -> immediately
    // requests 2nd challenge -> register ok
    io.push_challenge_ok(SAMPLE_CHALLENGE);
    io.push_register_err(Failure::Refused(ErrorBody::from(&RegisterError::UnknownChallenge)));
    io.push_challenge_ok(SAMPLE_CHALLENGE);
    io.push_register_ok(&fixture.passport_id, START_SECS + 86400);

    let stored = StoredPassport {
        relay_url: FIXED_RELAY.to_owned(),
        info_url: INFO_URL.to_owned(),
        token: fixture.child.clone(),
        ca_cert_pem: None,
    };
    let agent = TestAgent::start(io.clone(), &fixture, Some(stored), false);
    time::sleep(Duration::from_millis(10)).await;

    assert_eq!(io.challenges_received(), 2);
    assert_eq!(io.registers_received(), 2);
    let status = agent.status().unwrap();
    assert_eq!(status.lease_ends_at, Some(START_SECS + 86400));
}

#[tokio::test(start_paused = true)]
async fn challenge_expired_twice_fails_the_attempt() {
    let io = FakeIo::new();
    let fixture = test_fixture(30 * 24 * 3600);
    // 1st challenge -> register fails with challenge_expired
    io.push_challenge_ok(SAMPLE_CHALLENGE);
    io.push_register_err(Failure::Refused(ErrorBody::from(&RegisterError::ChallengeExpired)));
    // 2nd challenge -> register fails again with challenge_expired -> gives up for
    // this attempt
    io.push_challenge_ok(SAMPLE_CHALLENGE);
    io.push_register_err(Failure::Refused(ErrorBody::from(&RegisterError::ChallengeExpired)));

    let target = Target { info_url: INFO_URL.to_owned(), ca_cert_pem: None };
    let res = register_once(
        &io,
        &target,
        &fixture.node,
        &fixture.node_did,
        &fixture.host_did,
        &fixture.child,
    )
    .await;
    assert!(res.is_err());
    assert_eq!(io.challenges_received(), 2);
    assert_eq!(io.registers_received(), 2);
}

#[tokio::test(start_paused = true)]
async fn transient_failures_back_off() {
    let io = FakeIo::new();
    let fixture = test_fixture(30 * 24 * 3600);
    io.push_challenge_ok(SAMPLE_CHALLENGE);
    io.push_register_err(Failure::Refused(ErrorBody::from(&RegisterError::RateLimited)));

    let stored = StoredPassport {
        relay_url: FIXED_RELAY.to_owned(),
        info_url: INFO_URL.to_owned(),
        token: fixture.child.clone(),
        ca_cert_pem: None,
    };
    let agent = TestAgent::start(io.clone(), &fixture, Some(stored), false);
    time::sleep(Duration::from_millis(10)).await;

    // Try 1 at t=0
    assert_eq!(io.registers_received(), 1);
    let status = agent.status().unwrap();
    let next_at = status.next_attempt_at.unwrap();
    let delay1 = next_at.saturating_sub(START_SECS);
    assert!((4..=6).contains(&delay1), "delay1 was {delay1}");

    // Try 2
    io.push_challenge_ok(SAMPLE_CHALLENGE);
    io.push_register_err(Failure::Refused(ErrorBody {
        code: "some_unknown_new_code".into(),
        message: "something new".into(),
    }));
    time::advance(Duration::from_secs(delay1)).await;
    time::sleep(Duration::from_millis(10)).await;
    assert_eq!(io.registers_received(), 2);
    let status2 = agent.status().unwrap();
    let delay2 = status2.next_attempt_at.unwrap().saturating_sub(START_SECS + delay1);
    assert!((8..=12).contains(&delay2), "delay2 was {delay2}");
}

#[tokio::test(start_paused = true)]
async fn settled_refusal_waits_an_hour() {
    let io = FakeIo::new();
    let fixture = test_fixture(30 * 24 * 3600);
    io.push_challenge_ok(SAMPLE_CHALLENGE);
    io.push_register_err(Failure::Refused(ErrorBody::from(&RegisterError::UntrustedRoot)));

    let stored = StoredPassport {
        relay_url: FIXED_RELAY.to_owned(),
        info_url: INFO_URL.to_owned(),
        token: fixture.child.clone(),
        ca_cert_pem: None,
    };
    let agent = TestAgent::start(io.clone(), &fixture, Some(stored), false);
    time::sleep(Duration::from_millis(10)).await;

    assert_eq!(io.registers_received(), 1);
    let status = agent.status().unwrap();
    assert_eq!(status.next_attempt_at, Some(START_SECS + 3600));
}

#[tokio::test(start_paused = true)]
async fn no_attempt_after_the_access_end() {
    let io = FakeIo::new();
    let fixture = test_fixture(100);
    io.push_challenge_ok(SAMPLE_CHALLENGE);
    io.push_register_err(Failure::Refused(ErrorBody::from(&RegisterError::RateLimited)));

    let stored = StoredPassport {
        relay_url: FIXED_RELAY.to_owned(),
        info_url: INFO_URL.to_owned(),
        token: fixture.child.clone(),
        ca_cert_pem: None,
    };
    let agent = TestAgent::start(io.clone(), &fixture, Some(stored), false);
    time::sleep(Duration::from_millis(10)).await;

    time::advance(Duration::from_secs(120)).await;
    time::sleep(Duration::from_millis(10)).await;
    let status = agent.status().unwrap();
    assert_eq!(status.next_attempt_at, None);
}

#[tokio::test(start_paused = true)]
async fn failure_keeps_the_last_lease_in_the_status() {
    let io = FakeIo::new();
    let fixture = test_fixture(30 * 24 * 3600);
    io.push_challenge_ok(SAMPLE_CHALLENGE);
    io.push_register_ok(&fixture.passport_id, START_SECS + 86400);

    let stored = StoredPassport {
        relay_url: FIXED_RELAY.to_owned(),
        info_url: INFO_URL.to_owned(),
        token: fixture.child.clone(),
        ca_cert_pem: None,
    };
    let agent = TestAgent::start(io.clone(), &fixture, Some(stored), false);
    time::sleep(Duration::from_millis(10)).await;
    assert_eq!(agent.status().unwrap().lease_ends_at, Some(START_SECS + 86400));

    // Next renewal fails with Revoked
    io.push_challenge_ok(SAMPLE_CHALLENGE);
    io.push_register_err(Failure::Refused(ErrorBody::from(&RegisterError::Revoked)));
    time::advance(Duration::from_secs(57600)).await;
    time::sleep(Duration::from_millis(10)).await;

    let status = agent.status().unwrap();
    assert_eq!(status.lease_ends_at, Some(START_SECS + 86400));
    assert_eq!(status.last_error.unwrap().code, "revoked");
}

#[tokio::test(start_paused = true)]
async fn denied_clears_the_lease_in_the_status() {
    let io = FakeIo::new();
    let fixture = test_fixture(30 * 24 * 3600);
    io.push_challenge_ok(SAMPLE_CHALLENGE);
    io.push_register_ok(&fixture.passport_id, START_SECS + 86400);

    let stored = StoredPassport {
        relay_url: FIXED_RELAY.to_owned(),
        info_url: INFO_URL.to_owned(),
        token: fixture.child.clone(),
        ca_cert_pem: None,
    };
    let agent = TestAgent::start(io.clone(), &fixture, Some(stored), false);
    time::sleep(Duration::from_millis(10)).await;
    assert_eq!(agent.status().unwrap().lease_ends_at, Some(START_SECS + 86400));

    // Next renewal fails with Denied
    io.push_challenge_ok(SAMPLE_CHALLENGE);
    io.push_register_err(Failure::Refused(ErrorBody::from(&RegisterError::Denied)));
    time::advance(Duration::from_secs(57600)).await;
    time::sleep(Duration::from_millis(10)).await;

    let status = agent.status().unwrap();
    assert_eq!(status.lease_ends_at, None);
    assert_eq!(status.last_error.unwrap().code, "denied");
}
