//! What the resolver does first: the order of the saved choice, the saved
//! list and the registry, and which entries it skips.

use std::slice;

use syneroym_core::coordinator_info::CoordinatorStatus;
use tokio::time::Instant;

use super::{FakeIo, at, cache, entry, info, ranked, secs, start};
use crate::runtime::home_relay::cache as cache_file;

#[tokio::test(start_paused = true)]
async fn saved_choice_that_answers_is_used_without_a_registry_request() {
    let (a, b, c) = (entry(1), entry(2), entry(3));
    let io = FakeIo::new();
    for e in [&a, &b, &c] {
        io.up(e);
        io.confirm(e);
    }
    let run = start(&io, Some(cache(&[a, b.clone(), c], 3600, 0, Some(&b))));

    secs(1).await;

    assert_eq!(io.fetches(), 0);
    assert_eq!(io.sets(), slice::from_ref(&b.url));
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn saved_choice_at_capacity_is_kept() {
    let (a, b) = (entry(1), entry(2));
    let io = FakeIo::new();
    io.up(&a);
    io.confirm(&a);
    io.answer(&b, Some(info(CoordinatorStatus::AtCapacity, true)));
    io.confirm(&b);
    let run = start(&io, Some(cache(&[a, b.clone()], 3600, 0, Some(&b))));

    secs(1).await;

    assert_eq!(io.sets(), slice::from_ref(&b.url));
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn fresh_saved_list_is_used_without_a_registry_request() {
    let entries = [entry(1), entry(2), entry(3)];
    let io = FakeIo::new();
    for e in &entries {
        io.up(e);
        io.confirm(e);
    }
    let run = start(&io, Some(cache(&entries, 3600, 0, None)));

    secs(1).await;

    assert_eq!(io.fetches(), 0);
    assert_eq!(io.sets(), [ranked(&entries)[0].url.clone()]);
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn expired_list_is_refreshed_and_the_choice_kept_when_still_listed() {
    let entries = [entry(1), entry(2)];
    let order = ranked(&entries);
    let (first, chosen) = (&order[0], &order[1]);
    let io = FakeIo::new();
    for e in &entries {
        io.up(e);
        io.confirm(e);
    }
    // The new list ranks `first` before `chosen`, but `chosen` still works.
    io.serve(&entries, 3600);
    let run = start(&io, Some(cache(&entries, 10, 100, Some(chosen))));

    secs(1).await;
    assert_eq!(io.sets(), slice::from_ref(&chosen.url));

    secs(30).await;
    assert_eq!(io.fetches(), 1, "the expired list is refreshed once");
    assert_eq!(io.sets(), slice::from_ref(&chosen.url), "{first:?} did not replace the choice");
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn expired_list_is_used_when_the_registry_fails() {
    let entries = [entry(1), entry(2)];
    let io = FakeIo::new();
    for e in &entries {
        io.up(e);
        io.confirm(e);
    }
    let run = start(&io, Some(cache(&entries, 10, 100, None)));

    secs(1).await;

    assert_eq!(io.fetches(), 1);
    assert_eq!(io.sets(), [ranked(&entries)[0].url.clone()]);
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn empty_fresh_list_asks_the_registry() {
    let a = entry(1);
    let io = FakeIo::new();
    io.up(&a);
    io.confirm(&a);
    io.serve(slice::from_ref(&a), 3600);
    let run = start(&io, Some(cache(&[], 3600, 0, None)));

    secs(1).await;

    assert_eq!(io.fetches(), 1);
    assert_eq!(io.sets(), slice::from_ref(&a.url));
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn fresh_list_with_no_working_entry_asks_the_registry() {
    let (stale, good) = (entry(1), entry(2));
    let io = FakeIo::new();
    io.up(&good);
    io.confirm(&good);
    io.serve(slice::from_ref(&good), 3600);
    let run = start(&io, Some(cache(slice::from_ref(&stale), 3600, 0, None)));

    secs(1).await;

    assert_eq!(io.fetches(), 1);
    assert_eq!(io.sets(), slice::from_ref(&good.url));
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn skips_full_down_unknown_and_relay_off_entries() {
    let (full, down, unknown, relay_off, good) = (entry(1), entry(2), entry(3), entry(4), entry(5));
    let io = FakeIo::new();
    io.answer(&full, Some(info(CoordinatorStatus::AtCapacity, true)));
    io.answer(&unknown, Some(info(CoordinatorStatus::Unknown, true)));
    io.answer(&relay_off, Some(info(CoordinatorStatus::Healthy, false)));
    io.up(&good);
    for e in [&full, &down, &unknown, &relay_off, &good] {
        io.confirm(e);
    }
    io.serve(&[full, down, unknown, relay_off, good.clone()], 3600);
    let run = start(&io, None);

    secs(1).await;

    assert_eq!(io.sets(), slice::from_ref(&good.url));
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn unconfirmed_relay_moves_on_to_the_next_entry() {
    let entries = [entry(1), entry(2)];
    let order = ranked(&entries);
    let io = FakeIo::new();
    for e in &entries {
        io.up(e);
    }
    io.confirm(&order[1]);
    io.serve(&entries, 3600);
    let run = start(&io, None);

    secs(1).await;

    assert_eq!(io.sets(), [order[0].url.clone(), order[1].url.clone()]);
    assert_eq!(io.current(), Some(order[1].url.clone()));
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn cache_from_another_registry_is_ignored() {
    let (old, new) = (entry(1), entry(2));
    let io = FakeIo::new();
    for e in [&old, &new] {
        io.up(e);
        io.confirm(e);
    }
    io.serve(slice::from_ref(&new), 3600);
    let mut saved = cache(slice::from_ref(&old), 3600, 0, Some(&old));
    saved.registry_url = "http://other-registry.test".into();
    let run = start(&io, Some(saved));

    secs(1).await;

    assert_eq!(io.fetches(), 1);
    assert_eq!(io.sets(), slice::from_ref(&new.url));
    assert_eq!(io.probes(&old), 0);
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn choice_is_written_to_the_cache_file() {
    let entries = [entry(1), entry(2)];
    let io = FakeIo::new();
    for e in &entries {
        io.up(e);
        io.confirm(e);
    }
    io.serve(&entries, 3600);
    let run = start(&io, None);

    secs(1).await;

    let saved = cache_file::load(&run.path).expect("the cache file exists");
    assert_eq!(saved.registry_url, super::REGISTRY);
    assert_eq!(saved.list.entries, entries);
    assert_eq!(saved.chosen.map(|e| e.url), Some(ranked(&entries)[0].url.clone()));
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn saved_choice_off_the_saved_list_is_not_used_at_boot() {
    let (dropped, listed) = (entry(1), entry(2));
    let io = FakeIo::new();
    for e in [&dropped, &listed] {
        io.up(e);
        io.confirm(e);
    }
    let run = start(&io, Some(cache(slice::from_ref(&listed), 3600, 0, Some(&dropped))));

    secs(1).await;

    assert_eq!(io.sets(), slice::from_ref(&listed.url));
    assert_eq!(io.probes(&dropped), 0);
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn max_age_zero_does_not_loop() {
    let a = entry(1);
    let io = FakeIo::new();
    io.up(&a);
    io.confirm(&a);
    io.serve(slice::from_ref(&a), 0);
    let t0 = Instant::now();
    let run = start(&io, None);

    at(t0, 30).await;

    assert!(io.fetches() <= 4, "fetched {} times in 30 s", io.fetches());
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn closed_endpoint_ends_the_task() {
    // Once while a relay is kept, once with no relay at all.
    let a = entry(1);
    let io = FakeIo::new();
    io.up(&a);
    io.confirm(&a);
    io.serve(slice::from_ref(&a), 3600);
    let run = start(&io, None);
    secs(1).await;
    assert_eq!(io.sets().len(), 1);
    run.stop().await;

    let io = FakeIo::new();
    let run = start(&io, None);
    secs(1).await;
    assert_eq!(io.sets().len(), 0);
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn saved_choice_follows_a_changed_info_url_in_the_list() {
    let old = entry(1);
    let mut moved = old.clone();
    moved.info_url = "http://info-moved.test".into();
    let io = FakeIo::new();
    io.up(&moved);
    io.confirm(&old);
    // Only the new `info_url` answers. The saved choice still has the old one.
    let run = start(&io, Some(cache(slice::from_ref(&moved), 3600, 0, Some(&old))));

    secs(1).await;

    assert_eq!(io.fetches(), 0);
    assert_eq!(io.sets(), slice::from_ref(&old.url));
    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn closed_endpoint_ends_the_task_during_a_slow_registry_request() {
    let io = FakeIo::new();
    io.hang_fetch();
    let run = start(&io, None);

    secs(1).await;
    assert_eq!(io.fetches(), 1, "the request is under way");

    run.stop().await;
}

#[tokio::test(start_paused = true)]
async fn saved_choice_is_probed_again_when_the_fetched_list_moves_its_info_url() {
    let (a, b) = (entry(1), entry(2));
    let mut moved = a.clone();
    moved.info_url = "http://info-moved.test".into();
    let io = FakeIo::new();
    io.up(&moved);
    io.confirm(&a);
    io.up(&b);
    io.confirm(&b);
    // The saved list is old and still has the old `info_url`. The registry
    // has the new one, and only the new one answers.
    io.serve(&[moved, b.clone()], 3600);
    let saved = cache(&[a.clone(), b.clone()], 10, 100, Some(&a));
    let run = start(&io, Some(saved));

    secs(1).await;

    assert_eq!(io.fetches(), 1);
    assert_eq!(io.sets(), slice::from_ref(&a.url), "the node did not move to another relay");
    run.stop().await;
}
