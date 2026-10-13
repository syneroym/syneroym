//! What the resolver does while it holds a relay: probes, refreshes, and
//! picking again.

use std::slice;

use syneroym_core::coordinator_info::CoordinatorStatus;
use tokio::time::Instant;

use super::{FakeIo, at, cache, entry, info, secs, start};

/// A relay `a` that works and is listed, picked from the saved list.
fn io_with_relay(a: &super::CoordinatorEntry) -> FakeIo {
    let io = FakeIo::new();
    io.up(a);
    io.confirm(a);
    io
}

#[tokio::test(start_paused = true)]
async fn two_failures_in_five_minutes_keep_the_relay() {
    let a = entry(1);
    let io = io_with_relay(&a);
    let t0 = Instant::now();
    let run = start(&io, Some(cache(slice::from_ref(&a), 3600, 0, None)));

    secs(1).await;
    io.down(&a);
    // The probes at 60 s and 120 s fail; the one at 180 s passes.
    at(t0, 125).await;
    io.up(&a);
    at(t0, 600).await;

    assert_eq!(io.sets(), slice::from_ref(&a.url));
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn three_failures_in_five_minutes_pick_another_relay() {
    let (a, b) = (entry(1), entry(2));
    let io = io_with_relay(&a);
    io.up(&b);
    io.confirm(&b);
    let t0 = Instant::now();
    let run = start(&io, Some(cache(&[a.clone(), b.clone()], 3600, 0, Some(&a))));

    secs(1).await;
    assert_eq!(io.sets(), slice::from_ref(&a.url));
    io.down(&a);
    at(t0, 170).await;
    assert_eq!(io.sets(), slice::from_ref(&a.url), "two failures are not enough");

    at(t0, 200).await;
    assert_eq!(io.sets(), [a.url.clone(), b.url.clone()]);
    assert_eq!(io.current(), Some(b.url.clone()));
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn failures_spread_over_more_than_five_minutes_keep_the_relay() {
    let a = entry(1);
    let io = io_with_relay(&a);
    let t0 = Instant::now();
    let run = start(&io, Some(cache(slice::from_ref(&a), 3600, 0, None)));

    // One failed probe at 60 s, 240 s and 420 s: never 3 within 300 s.
    for probe_at in [60, 240, 420] {
        at(t0, probe_at - 5).await;
        io.down(&a);
        at(t0, probe_at + 5).await;
        io.up(&a);
    }
    at(t0, 700).await;

    assert_eq!(io.sets(), slice::from_ref(&a.url));
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn relay_removed_from_the_list_is_replaced() {
    let (a, b) = (entry(1), entry(2));
    let io = io_with_relay(&a);
    io.up(&b);
    io.confirm(&b);
    io.serve(slice::from_ref(&a), 60);
    let t0 = Instant::now();
    let run = start(&io, None);

    secs(1).await;
    assert_eq!(io.sets(), slice::from_ref(&a.url));

    io.serve(slice::from_ref(&b), 60);
    at(t0, 70).await;

    assert_eq!(io.sets(), [a.url.clone(), b.url.clone()]);
    assert_eq!(io.current(), Some(b.url.clone()));
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn refresh_failure_keeps_the_relay_and_retries() {
    let a = entry(1);
    let io = io_with_relay(&a);
    io.serve(slice::from_ref(&a), 60);
    let t0 = Instant::now();
    let run = start(&io, None);

    secs(1).await;
    assert_eq!(io.fetches(), 1);
    io.registry_down();

    // Refreshes fail at 60 s, then about 60 s, 120 s and 240 s later.
    at(t0, 400).await;

    assert_eq!(io.fetches(), 4);
    assert_eq!(io.sets(), slice::from_ref(&a.url));
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn nothing_works_clears_the_map_and_retries_with_backoff() {
    let a = entry(1);
    let io = FakeIo::new();
    let t0 = Instant::now();
    let run = start(&io, None);

    secs(1).await;
    assert_eq!(io.fetches(), 1);
    assert!(io.clears() >= 1, "the unconfirmed relay is removed");

    // Retries come about 5 s, then 10 s later (each with 10 % jitter).
    at(t0, 8).await;
    assert_eq!(io.fetches(), 2);
    at(t0, 12).await;
    assert_eq!(io.fetches(), 2);
    at(t0, 18).await;
    assert_eq!(io.fetches(), 3);

    io.up(&a);
    io.confirm(&a);
    io.serve(slice::from_ref(&a), 3600);
    at(t0, 60).await;
    assert_eq!(io.sets(), slice::from_ref(&a.url));
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn failed_repick_keeps_the_current_relay() {
    let (a, b) = (entry(1), entry(2));
    let io = io_with_relay(&a);
    io.up(&b);
    io.serve(slice::from_ref(&a), 60);
    let t0 = Instant::now();
    let run = start(&io, None);

    secs(1).await;
    // `b` answers but its relay never confirms.
    io.serve(slice::from_ref(&b), 60);
    at(t0, 70).await;

    assert_eq!(io.sets(), [a.url.clone(), b.url.clone()]);
    assert_eq!(io.current(), Some(a.url.clone()));
    run.stop().await;
}

/// `a` is working and listed. A refresh drops it for `[b, c]`: `b` answers
/// but its relay never confirms, and `c` does not answer.
async fn kept_relay_off_the_list_retries_on_the_backoff(max_age_secs: u64) {
    let (a, b, c) = (entry(1), entry(2), entry(3));
    let io = io_with_relay(&a);
    io.up(&b);
    io.serve(slice::from_ref(&a), max_age_secs);
    let t0 = Instant::now();
    let run = start(&io, None);

    secs(1).await;
    assert_eq!(io.sets(), slice::from_ref(&a.url));
    io.serve(&[b.clone(), c.clone()], max_age_secs);

    // Only a replacement try probes `c`, and a refresh never probes. So the
    // count of `c` shows the tries and nothing else.
    let drop_at = if max_age_secs == 0 { 10 } else { 3600 };
    // Tries come about 0, 5, 15 and 35 s after the drop, each wait with 10 %
    // jitter, so the checks keep clear of the earliest and latest times.
    for (after, tries) in [(3, 1), (8, 2), (12, 2), (18, 3), (30, 3), (42, 4)] {
        at(t0, drop_at + after).await;
        assert_eq!(io.probes(&c), tries, "probes of c {after} s after the drop");
        assert_eq!(io.current(), Some(a.url.clone()));
    }

    // `b` failed once. The cool-down keeps it from being tried again soon,
    // because `a` still works.
    at(t0, drop_at + 3000).await;
    assert_eq!(io.set_count(&b), 1);
    assert_eq!(io.current(), Some(a.url.clone()));

    // `a` is listed again: the tries stop.
    io.serve(&[a, b, c.clone()], max_age_secs);
    at(t0, drop_at + 3000 + 4800).await;
    let probes = io.probes(&c);
    at(t0, drop_at + 3000 + 4800 + 3600).await;
    assert_eq!(io.probes(&c), probes);
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn kept_relay_off_the_list_retries_on_the_backoff_with_a_long_max_age() {
    kept_relay_off_the_list_retries_on_the_backoff(3600).await;
}

#[tokio::test(start_paused = true)]
async fn kept_relay_off_the_list_retries_on_the_backoff_with_max_age_zero() {
    kept_relay_off_the_list_retries_on_the_backoff(0).await;
}

#[tokio::test(start_paused = true)]
async fn dead_kept_relay_gets_no_cool_down() {
    let (a, b) = (entry(1), entry(2));
    let io = io_with_relay(&a);
    io.up(&b);
    let t0 = Instant::now();
    let run = start(&io, Some(cache(&[a.clone(), b.clone()], 3600, 0, Some(&a))));

    secs(1).await;
    assert_eq!(io.sets(), slice::from_ref(&a.url));

    // `a` fails the probes at 60 s, 120 s and 180 s, so a new pick starts.
    // `b` does not confirm at first.
    io.down(&a);
    at(t0, 181).await;
    assert_eq!(io.sets(), [a.url.clone(), b.url.clone()]);
    assert_eq!(io.current(), Some(a.url.clone()));

    // The next try comes about 5 s later, not an hour later.
    io.confirm(&b);
    at(t0, 190).await;
    assert_eq!(io.sets(), [a.url.clone(), b.url.clone(), b.url.clone()]);
    assert_eq!(io.current(), Some(b.url.clone()));
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn relay_at_capacity_is_not_left_by_the_health_probe() {
    let a = entry(1);
    let io = io_with_relay(&a);
    let t0 = Instant::now();
    let run = start(&io, Some(cache(slice::from_ref(&a), 3600, 0, None)));

    secs(1).await;
    io.answer(&a, Some(info(CoordinatorStatus::AtCapacity, true)));
    at(t0, 600).await;

    assert_eq!(io.sets(), slice::from_ref(&a.url));
    run.stop().await;
}
