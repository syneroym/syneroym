use std::time::Duration;

use syneroym_control_plane::HomeRelayControl;
use syneroym_observability::recorder::MemoryRecorder;
use syneroym_passport::wire::HomeRelaySetRequest;
use tokio::time;

use super::{
    FIXED_RELAY, FakeIo, INFO_URL, START_SECS, StoredPassport, TestAgent, client::Failure,
    test_fixture,
};
use crate::runtime::home_relay::passport::{ACCESS_END_GAUGE, ending_soon, renew_at};

const SAMPLE_CHALLENGE: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[tokio::test(start_paused = true)]
async fn renews_when_a_third_of_the_lease_is_left() {
    let io = FakeIo::new();
    let fixture = test_fixture(30 * 24 * 3600);
    // 1st register at boot
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

    // 2nd register at 16 hours (57600 s)
    io.push_challenge_ok(SAMPLE_CHALLENGE);
    io.push_register_ok(&fixture.passport_id, START_SECS + 57600 + 86400);

    time::advance(Duration::from_secs(57600)).await;
    time::sleep(Duration::from_millis(10)).await;
    assert_eq!(io.registers_received(), 2);
    let status = agent.status().unwrap();
    assert_eq!(status.lease_ends_at, Some(START_SECS + 57600 + 86400));
}

#[tokio::test(start_paused = true)]
async fn an_ended_lease_is_renewed_while_the_token_runs() {
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

    // Renewal at 16 h fails with Unreachable
    io.push_challenge_ok(SAMPLE_CHALLENGE);
    io.push_register_err(Failure::Unreachable("temporary drop".into()));

    time::advance(Duration::from_secs(57600)).await;
    time::sleep(Duration::from_millis(10)).await;
    assert_eq!(io.registers_received(), 2);

    // Lease runs out at 24 h (86400 s). Advance past it to 25 h (90000 s)
    io.push_challenge_ok(SAMPLE_CHALLENGE);
    io.push_register_ok(&fixture.passport_id, START_SECS + 90000 + 86400);

    time::advance(Duration::from_secs(32400)).await;
    time::sleep(Duration::from_millis(10)).await;
    assert!(io.registers_received() >= 3);
    let status = agent.status().unwrap();
    assert_eq!(status.lease_ends_at, Some(START_SECS + 90000 + 86400));
}

#[tokio::test(start_paused = true)]
async fn no_renewal_when_the_lease_reaches_the_access_end() {
    let io = FakeIo::new();
    let access_end = START_SECS + 3600;
    let fixture = test_fixture(3600);
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
    let status = agent.status().unwrap();
    assert_eq!(status.next_attempt_at, None);
}

#[test]
fn renew_at_never_sooner_than_a_minute() {
    let now = 1000;
    // lease_ends_at >= access_ends_at -> None
    assert_eq!(renew_at(now, 2000, 2000), None);
    assert_eq!(renew_at(now, 2500, 2000), None);

    // Normal case: 24h lease, access end far away
    let lease_ends_at = now + 86400;
    let access_ends_at = now + 86400 * 10;
    let scheduled = renew_at(now, lease_ends_at, access_ends_at).unwrap();
    assert_eq!(scheduled, now + 57600);

    // Very short lease: min 60 s delay
    let short_lease = now + 30;
    let scheduled_short = renew_at(now, short_lease, access_ends_at).unwrap();
    assert_eq!(scheduled_short, now + 60);
}

#[test]
fn ending_soon_is_true_at_seven_days() {
    let now = 1000;
    let seven_days = 7 * 24 * 3600;
    assert!(ending_soon(now, now + seven_days));
    assert!(ending_soon(now, now + seven_days - 1));
    assert!(!ending_soon(now, now + seven_days + 1));
}

#[tokio::test(start_paused = true)]
async fn access_end_gauge_names_only_the_active_passport() {
    let rec = MemoryRecorder::new();
    let _ = rec.clone().install();

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

    let snapshot = rec.snapshot();
    let gauge_val = snapshot.gauges.get(ACCESS_END_GAUGE).copied().unwrap_or(0.0);
    assert_eq!(gauge_val, (START_SECS + 30 * 24 * 3600) as f64);

    // Replace with inactive passport -> gauge set to 0.0
    let (_passport2, _id2, child2) = fixture.new_passport_for_node(30 * 24 * 3600);
    let req = HomeRelaySetRequest {
        token: child2,
        info_url: INFO_URL.to_owned(),
        relay_url: Some("http://127.0.0.1:9999/".into()),
        ca_cert_pem: None,
    };
    let res = agent.control.set_passport(req).await.unwrap();
    assert!(!res.active);

    let snapshot2 = rec.snapshot();
    let gauge_val2 = snapshot2.gauges.get(ACCESS_END_GAUGE).copied().unwrap_or(-1.0);
    assert_eq!(gauge_val2, 0.0);
}
