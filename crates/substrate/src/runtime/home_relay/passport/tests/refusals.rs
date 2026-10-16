use std::time::Duration;

use syneroym_control_plane::HomeRelayControl;
use syneroym_passport::{
    RegisterError,
    wire::{ErrorBody, HomeRelaySetRequest},
};
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

    // Try 3
    io.push_challenge_ok(SAMPLE_CHALLENGE);
    io.push_register_err(Failure::Unreachable("network down".into()));
    time::advance(Duration::from_secs(delay2)).await;
    time::sleep(Duration::from_millis(10)).await;
    assert_eq!(io.registers_received(), 3);
    let status3 = agent.status().unwrap();
    let delay3 = status3.next_attempt_at.unwrap().saturating_sub(START_SECS + delay1 + delay2);
    assert!((18..=22).contains(&delay3), "delay3 was {delay3}");
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

#[tokio::test(start_paused = true)]
async fn host_sleep_clock_jump_triggers_renewal_at_next_cap_tick() {
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
    assert_eq!(io.registers_received(), 1);

    // Renewal is scheduled at 16 h (57600 s)
    let status = agent.status().unwrap();
    assert_eq!(status.next_attempt_at, Some(START_SECS + 57600));

    // Simulate host sleep/suspend for 16 hours: wall clock advances by 16h without
    // tokio monotonic advancing
    io.advance_clock(57600);

    // Prepare response for the renewal attempt
    io.push_challenge_ok(SAMPLE_CHALLENGE);
    io.push_register_ok(&fixture.passport_id, START_SECS + 57600 + 86400);

    // Advance tokio monotonic clock by one SLEEP_CAP tick (300 s)
    time::advance(Duration::from_secs(300)).await;
    time::sleep(Duration::from_millis(10)).await;

    // The attempt ran at the cap tick!
    assert_eq!(io.registers_received(), 2);
    let status2 = agent.status().unwrap();
    assert_eq!(status2.lease_ends_at, Some(START_SECS + 57600 + 86400));
}

#[tokio::test(start_paused = true)]
async fn warning_logged_and_rate_limited_when_access_ends_soon() {
    let (logs, _guard) = crate::runtime::home_relay::test_logs::capture();
    let io = FakeIo::new();
    // Passport expiring in 5 days (<= 7 days)
    let five_days = 5 * 86400;
    let fixture = test_fixture(five_days);
    io.push_challenge_ok(SAMPLE_CHALLENGE);
    io.push_register_ok(&fixture.passport_id, START_SECS + 86400);

    let stored = StoredPassport {
        relay_url: FIXED_RELAY.to_owned(),
        info_url: INFO_URL.to_owned(),
        token: fixture.child.clone(),
        ca_cert_pem: None,
    };
    let _agent = TestAgent::start(io.clone(), &fixture, Some(stored), false);
    time::sleep(Duration::from_millis(10)).await;
    assert_eq!(io.registers_received(), 1);
    assert_eq!(
        crate::runtime::home_relay::test_logs::warnings_about(&logs, "passport ends soon"),
        1
    );

    // Next retry after backoff doesn't re-log warning within 24h
    io.push_challenge_ok(SAMPLE_CHALLENGE);
    io.push_register_err(Failure::Refused(ErrorBody::from(&RegisterError::RateLimited)));
    time::advance(Duration::from_secs(57600)).await;
    time::sleep(Duration::from_millis(10)).await;
    assert_eq!(io.registers_received(), 2);
    assert_eq!(
        crate::runtime::home_relay::test_logs::warnings_about(&logs, "passport ends soon"),
        1
    );

    // After 24h have passed since the previous warning, another attempt logs again
    io.push_challenge_ok(SAMPLE_CHALLENGE);
    io.push_register_err(Failure::Refused(ErrorBody::from(&RegisterError::RateLimited)));
    time::advance(Duration::from_secs(30000)).await;
    time::sleep(Duration::from_millis(10)).await;
    assert_eq!(io.registers_received(), 3);
    assert_eq!(
        crate::runtime::home_relay::test_logs::warnings_about(&logs, "passport ends soon"),
        2
    );
}

#[tokio::test(start_paused = true)]
async fn expired_refusal_before_access_end_does_not_spin_at_access_end() {
    let io = FakeIo::new();
    let access_end = START_SECS + 3600;
    let fixture = test_fixture(3600);
    // Initial registration grants a lease that ends at the access end
    io.push_challenge_ok(SAMPLE_CHALLENGE);
    io.push_register_ok(&fixture.passport_id, access_end);

    let stored = StoredPassport {
        relay_url: FIXED_RELAY.to_owned(),
        info_url: INFO_URL.to_owned(),
        token: fixture.child.clone(),
        ca_cert_pem: None,
    };
    let agent = TestAgent::start(io.clone(), &fixture, Some(stored), false);
    time::sleep(Duration::from_millis(10)).await;
    assert_eq!(io.registers_received(), 1);

    // Advance to 60s before access end and trigger an attempt
    time::advance(Duration::from_secs(3540)).await;

    // Coordinator clock is slightly ahead, so it answers expired
    io.push_challenge_ok(SAMPLE_CHALLENGE);
    io.push_register_err(Failure::Refused(ErrorBody::from(&RegisterError::Expired)));

    let req = HomeRelaySetRequest {
        token: fixture.child.clone(),
        info_url: INFO_URL.to_owned(),
        relay_url: None,
        ca_cert_pem: None,
    };
    let res = agent.control.set_passport(req).await.unwrap();
    assert_eq!(res.last_error.as_ref().map(|e| e.code.as_str()), Some("expired"));
    // Since now < access_end, next attempt was planned for access_end
    assert_eq!(res.next_attempt_at, Some(access_end));

    // Advance past access end. If the agent spins at 100% CPU, paused time never
    // completes and the test hangs.
    time::advance(Duration::from_secs(70)).await;
    time::sleep(Duration::from_millis(10)).await;

    let status = agent.status().unwrap();
    assert_eq!(status.next_attempt_at, None);
    assert_eq!(status.last_error.as_ref().map(|e| e.code.as_str()), Some("expired"));
}
