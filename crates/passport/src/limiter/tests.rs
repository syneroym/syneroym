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
fn a_full_limiter_drops_the_earliest_window_for_a_new_address() {
    let limiter = RequestLimiter::new(REQUESTS_PER_MINUTE, 2);
    for _ in 0..REQUESTS_PER_MINUTE {
        assert!(limiter.allow(A, NOW));
    }
    assert!(limiter.allow(B, NOW + 1));
    assert!(limiter.allow(C, NOW + 2), "a new address is never refused for lack of room");
    assert!(limiter.allow(A, NOW + 3), "A had the earliest window, so it was dropped");
    assert!(limiter.allow(C, NOW + 3), "C is still tracked");
}

#[test]
fn a_flood_of_new_addresses_does_not_lock_out_a_real_user() {
    let limiter = RequestLimiter::new(REQUESTS_PER_MINUTE, 100);
    assert!(limiter.allow(A, NOW));
    for n in 0..1_000u32 {
        let flood = IpAddr::V4(Ipv4Addr::from(0x0a00_0000 + n));
        assert!(limiter.allow(flood, NOW + 1 + u64::from(n % 50)));
    }
    assert!(limiter.allow(B, NOW + 55), "a new real user still gets in");
    assert!(limiter.allow(A, NOW + 55));
    let state = limiter.state.lock().unwrap();
    assert!(state.windows.len() <= 100);
    assert!(state.order.len() <= 101, "stale entries do not pile up");
}

#[test]
fn ipv6_addresses_of_one_64_share_a_limit() {
    let limiter = RequestLimiter::default();
    let in_64 = |last: u16| IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 1, 2, 0, 0, 0, last));
    for n in 0..REQUESTS_PER_MINUTE {
        assert!(limiter.allow(in_64(n as u16), NOW));
    }
    assert!(!limiter.allow(in_64(999), NOW));
    let other_64 = IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 1, 3, 0, 0, 0, 1));
    assert!(limiter.allow(other_64, NOW));
}

#[test]
fn an_ipv4_mapped_address_counts_as_the_ipv4_address() {
    let limiter = RequestLimiter::default();
    let mapped = IpAddr::V6(Ipv4Addr::new(192, 0, 2, 1).to_ipv6_mapped());
    for _ in 0..REQUESTS_PER_MINUTE {
        assert!(limiter.allow(mapped, NOW));
    }
    assert!(!limiter.allow(A, NOW));
}
