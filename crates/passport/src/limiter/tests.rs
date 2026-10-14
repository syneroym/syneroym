use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use super::*;

const NOW: u64 = 1_000_000;
const A: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));
const B: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 2));
const C: IpAddr = IpAddr::V6(Ipv6Addr::LOCALHOST);

#[test]
fn ten_requests_pass_and_the_eleventh_is_refused() {
    let limiter = RequestLimiter::default();
    for _ in 0..REQUESTS_PER_MINUTE {
        assert!(limiter.allow(A, NOW));
    }
    assert!(!limiter.allow(A, NOW + WINDOW_SECS - 1));
}

#[test]
fn a_new_minute_resets_the_count() {
    let limiter = RequestLimiter::default();
    for _ in 0..REQUESTS_PER_MINUTE {
        assert!(limiter.allow(A, NOW));
    }
    assert!(limiter.allow(A, NOW + WINDOW_SECS));
}

#[test]
fn two_addresses_are_independent() {
    let limiter = RequestLimiter::default();
    for _ in 0..REQUESTS_PER_MINUTE {
        assert!(limiter.allow(A, NOW));
    }
    assert!(!limiter.allow(A, NOW));
    assert!(limiter.allow(B, NOW));
}

#[test]
fn a_full_limiter_refuses_a_new_address_until_a_window_ends() {
    let limiter = RequestLimiter::new(REQUESTS_PER_MINUTE, 2);
    assert!(limiter.allow(A, NOW));
    assert!(limiter.allow(B, NOW + 1));
    assert!(!limiter.allow(C, NOW + 2));
    assert!(limiter.allow(A, NOW + 3), "a tracked address still works");
    assert!(limiter.allow(C, NOW + WINDOW_SECS), "A's window ended, so C gets its place");
}
