//! How often the resolver warns.

use std::{fs, slice};

use tokio::time::Instant;

use super::{FakeIo, at, entry, secs, start, start_with};
use crate::runtime::home_relay::test_logs::{capture, lines_with, warnings_about};

#[tokio::test(start_paused = true)]
async fn degraded_mode_warns_once_per_five_minutes() {
    let (logs, _guard) = capture();
    let io = FakeIo::new();
    let t0 = Instant::now();
    let run = start(&io, None);

    // About seven tries by now, and one warning.
    at(t0, 280).await;
    assert!(io.fetches() >= 6, "{} tries", io.fetches());
    assert_eq!(lines_with(&logs, "WARN"), 1);

    // Retries wait 5, 10, ... 600 s, each with 10 % jitter. So the second
    // warning comes at 300 s or later, and a third only after 600 s.
    at(t0, 700).await;
    assert!((2..=3).contains(&lines_with(&logs, "WARN")), "{}", lines_with(&logs, "WARN"));
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn relay_set_outside_the_resolver_is_not_called_degraded() {
    let (logs, _guard) = capture();
    let io = FakeIo::new();
    io.set_current(&entry(1));
    let run = start(&io, None);

    secs(100).await;

    assert!(io.fetches() > 1, "the resolver keeps trying");
    assert_eq!(io.clears(), 0);
    assert_eq!(lines_with(&logs, "no home relay"), 0);
    assert!(lines_with(&logs, "set outside") >= 1);
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn relay_is_chosen_and_warnings_are_limited_when_the_cache_cannot_be_saved() {
    let (logs, _guard) = capture();
    let a = entry(1);
    let io = FakeIo::new();
    io.up(&a);
    io.confirm(&a);
    // A refresh every 10 s, and every save fails: the directory of the cache
    // file is a regular file.
    io.serve(slice::from_ref(&a), 0);
    let t0 = Instant::now();
    let run = start_with(&io, None, |dir| {
        fs::write(dir.join("blocker"), "").unwrap();
        dir.join("blocker").join("coordinators.json")
    });

    at(t0, 200).await;

    assert_eq!(io.sets(), slice::from_ref(&a.url));
    assert!(io.fetches() >= 10, "{} refreshes", io.fetches());
    assert_eq!(warnings_about(&logs, "cannot save"), 1);
    run.stop().await;
}
