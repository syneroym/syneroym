use std::sync::Arc;

use crate::{
    event_bus::{
        Cursor, EventBus, EventError, MAX_WINDOW_EVENTS, READ_LIMIT_CAP, ReadOutcome, WINDOW_AGE_MS,
    },
    local_registry::EndpointRegistry,
    storage::MockStorage,
};

async fn make_test_bus() -> (Arc<EventBus>, EndpointRegistry) {
    let storage = Arc::new(MockStorage::new());
    let registry = EndpointRegistry::new(storage).await.unwrap();
    let bus = EventBus::new(registry.clone());
    (bus, registry)
}

#[tokio::test]
async fn a_recorded_event_is_tagged_with_the_writers_app_instance_and_service() {
    let (bus, registry) = make_test_bus().await;
    registry
        .set_app_context("svc1".to_string(), "app1".to_string(), "conversation".to_string())
        .await
        .unwrap();

    let cursor = bus.record("svc1", "conversation.message", "conv1", Some("msg1"), 1000).unwrap();
    assert_eq!(cursor.seq, 1);

    let outcome =
        bus.read("app1", Some(&Cursor { run_id: bus.run_id().to_string(), seq: 0 }), 10, 1000);
    match outcome {
        ReadOutcome::Events { events, last } => {
            assert_eq!(events.len(), 1);
            let event = &events[0];
            assert_eq!(event.cursor, cursor);
            assert_eq!(event.kind, "conversation.message");
            assert_eq!(event.service, "conversation");
            assert_eq!(event.subject, "conv1");
            assert_eq!(event.item.as_deref(), Some("msg1"));
            assert_eq!(event.at_ms, 1000);
            assert_eq!(last, cursor);
        }
        ReadOutcome::Reload { .. } => panic!("expected events, got reload"),
    }
}

#[tokio::test]
async fn a_service_outside_any_app_instance_cannot_record() {
    let (bus, _) = make_test_bus().await;
    let err = bus.record("unregistered_svc", "some.kind", "sub", None, 1000).unwrap_err();
    assert_eq!(err, EventError::NoAppInstance);
}

#[tokio::test]
async fn one_app_instance_never_reads_another_apps_events() {
    let (bus, registry) = make_test_bus().await;
    registry
        .set_app_context("svc1".to_string(), "app1".to_string(), "s1".to_string())
        .await
        .unwrap();
    registry
        .set_app_context("svc2".to_string(), "app2".to_string(), "s2".to_string())
        .await
        .unwrap();

    let cur1 = bus.record("svc1", "test.kind", "sub1", None, 1000).unwrap();
    let cur2 = bus.record("svc2", "test.kind", "sub2", None, 1000).unwrap();
    assert_eq!(cur1.seq, 1);
    assert_eq!(cur2.seq, 1);

    let start = Cursor { run_id: bus.run_id().to_string(), seq: 0 };

    match bus.read("app1", Some(&start), 10, 1000) {
        ReadOutcome::Events { events, .. } => {
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].service, "s1");
            assert_eq!(events[0].subject, "sub1");
        }
        ReadOutcome::Reload { .. } => panic!("expected app1 events"),
    }

    match bus.read("app2", Some(&start), 10, 1000) {
        ReadOutcome::Events { events, .. } => {
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].service, "s2");
            assert_eq!(events[0].subject, "sub2");
        }
        ReadOutcome::Reload { .. } => panic!("expected app2 events"),
    }
}

#[tokio::test]
async fn read_without_a_cursor_returns_nothing_and_the_head() {
    let (bus, registry) = make_test_bus().await;
    registry
        .set_app_context("svc1".to_string(), "app1".to_string(), "s1".to_string())
        .await
        .unwrap();

    bus.record("svc1", "test.kind", "sub", None, 1000).unwrap();
    bus.record("svc1", "test.kind", "sub", None, 1001).unwrap();

    match bus.read("app1", None, 10, 1002) {
        ReadOutcome::Events { events, last } => {
            assert!(events.is_empty());
            assert_eq!(last.seq, 2);
            assert_eq!(last.run_id, bus.run_id());
        }
        ReadOutcome::Reload { .. } => panic!("expected events"),
    }
}

#[tokio::test]
async fn read_after_a_cursor_returns_the_later_events_oldest_first() {
    let (bus, registry) = make_test_bus().await;
    registry
        .set_app_context("svc1".to_string(), "app1".to_string(), "s1".to_string())
        .await
        .unwrap();

    bus.record("svc1", "test.kind", "sub1", None, 1000).unwrap();
    bus.record("svc1", "test.kind", "sub2", None, 1001).unwrap();
    bus.record("svc1", "test.kind", "sub3", None, 1002).unwrap();

    let cur1 = Cursor { run_id: bus.run_id().to_string(), seq: 1 };

    match bus.read("app1", Some(&cur1), 10, 1003) {
        ReadOutcome::Events { events, last } => {
            assert_eq!(events.len(), 2);
            assert_eq!(events[0].cursor.seq, 2);
            assert_eq!(events[0].subject, "sub2");
            assert_eq!(events[1].cursor.seq, 3);
            assert_eq!(events[1].subject, "sub3");
            assert_eq!(last.seq, 3);
        }
        ReadOutcome::Reload { .. } => panic!("expected events"),
    }
}

#[tokio::test]
async fn read_at_the_head_returns_nothing_and_is_not_a_reload() {
    let (bus, registry) = make_test_bus().await;
    registry
        .set_app_context("svc1".to_string(), "app1".to_string(), "s1".to_string())
        .await
        .unwrap();

    bus.record("svc1", "test.kind", "sub", None, 1000).unwrap();
    let head = bus.head("app1");
    assert_eq!(head.seq, 1);

    match bus.read("app1", Some(&head), 10, 1001) {
        ReadOutcome::Events { events, last } => {
            assert!(events.is_empty());
            assert_eq!(last, head);
        }
        ReadOutcome::Reload { .. } => panic!("expected empty events, not reload"),
    }
}

#[tokio::test]
async fn the_limit_caps_one_read_and_the_next_read_continues() {
    let (bus, registry) = make_test_bus().await;
    registry
        .set_app_context("svc1".to_string(), "app1".to_string(), "s1".to_string())
        .await
        .unwrap();

    for i in 1..=5 {
        bus.record("svc1", "test.kind", &format!("sub{i}"), None, 1000 + i).unwrap();
    }

    let start = Cursor { run_id: bus.run_id().to_string(), seq: 0 };

    let (batch1, cur_batch1) = match bus.read("app1", Some(&start), 2, 2000) {
        ReadOutcome::Events { events, last } => {
            assert_eq!(events.len(), 2);
            assert_eq!(events[0].cursor.seq, 1);
            assert_eq!(events[1].cursor.seq, 2);
            assert_eq!(last.seq, 2);
            (events, last)
        }
        ReadOutcome::Reload { .. } => panic!("expected events"),
    };
    assert_eq!(batch1.len(), 2);

    let (batch2, cur_batch2) = match bus.read("app1", Some(&cur_batch1), 2, 2000) {
        ReadOutcome::Events { events, last } => {
            assert_eq!(events.len(), 2);
            assert_eq!(events[0].cursor.seq, 3);
            assert_eq!(events[1].cursor.seq, 4);
            assert_eq!(last.seq, 4);
            (events, last)
        }
        ReadOutcome::Reload { .. } => panic!("expected events"),
    };
    assert_eq!(batch2.len(), 2);

    match bus.read("app1", Some(&cur_batch2), 2, 2000) {
        ReadOutcome::Events { events, last } => {
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].cursor.seq, 5);
            assert_eq!(last.seq, 5);
        }
        ReadOutcome::Reload { .. } => panic!("expected events"),
    }
}

#[tokio::test]
async fn the_window_keeps_the_newest_ten_thousand() {
    let (bus, registry) = make_test_bus().await;
    registry
        .set_app_context("svc1".to_string(), "app1".to_string(), "s1".to_string())
        .await
        .unwrap();

    let total = MAX_WINDOW_EVENTS + 5;
    for _ in 1..=total {
        bus.record("svc1", "test.kind", "sub", None, 1000).unwrap();
    }

    // Sequence 0 is before the window (oldest kept is 6)
    let start = Cursor { run_id: bus.run_id().to_string(), seq: 0 };
    assert!(matches!(bus.read("app1", Some(&start), 10, 1000), ReadOutcome::Reload { .. }));

    // Sequence 5 is immediately before the oldest kept event (6), so it is valid
    let cur5 = Cursor { run_id: bus.run_id().to_string(), seq: 5 };
    let mut read_cursor = cur5;
    let mut collected = 0;
    loop {
        match bus.read("app1", Some(&read_cursor), READ_LIMIT_CAP, 1000) {
            ReadOutcome::Events { events, last } => {
                if events.is_empty() {
                    break;
                }
                collected += events.len();
                read_cursor = last;
            }
            ReadOutcome::Reload { .. } => panic!("unexpected reload"),
        }
    }
    assert_eq!(collected, MAX_WINDOW_EVENTS);
}

#[tokio::test]
async fn events_older_than_a_day_are_dropped_on_read_and_on_record() {
    let (bus, registry) = make_test_bus().await;
    registry
        .set_app_context("svc1".to_string(), "app1".to_string(), "s1".to_string())
        .await
        .unwrap();

    bus.record("svc1", "test.kind", "sub1", None, 1000).unwrap();

    let t2 = 1000 + WINDOW_AGE_MS + 10;
    bus.record("svc1", "test.kind", "sub2", None, t2).unwrap();

    // Event 1 aged out on record
    let cur0 = Cursor { run_id: bus.run_id().to_string(), seq: 0 };
    assert!(matches!(bus.read("app1", Some(&cur0), 10, t2), ReadOutcome::Reload { .. }));

    // Event 2 is still kept
    let cur1 = Cursor { run_id: bus.run_id().to_string(), seq: 1 };
    match bus.read("app1", Some(&cur1), 10, t2) {
        ReadOutcome::Events { events, .. } => {
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].cursor.seq, 2);
        }
        ReadOutcome::Reload { .. } => panic!("expected event 2"),
    }

    // Now advance time on read so event 2 ages out too
    let t3 = t2 + WINDOW_AGE_MS + 1;
    assert!(matches!(bus.read("app1", Some(&cur1), 10, t3), ReadOutcome::Reload { .. }));
}

#[tokio::test]
async fn a_cursor_older_than_the_window_is_a_reload_with_the_head() {
    let (bus, registry) = make_test_bus().await;
    registry
        .set_app_context("svc1".to_string(), "app1".to_string(), "s1".to_string())
        .await
        .unwrap();

    for _ in 1..=(MAX_WINDOW_EVENTS + 1) {
        bus.record("svc1", "test.kind", "sub", None, 1000).unwrap();
    }

    let head = bus.head("app1");
    let old_cursor = Cursor { run_id: bus.run_id().to_string(), seq: 0 };

    match bus.read("app1", Some(&old_cursor), 10, 1000) {
        ReadOutcome::Reload { now } => assert_eq!(now, head),
        ReadOutcome::Events { .. } => panic!("expected reload"),
    }
}

#[tokio::test]
async fn a_cursor_from_another_run_is_a_reload() {
    let (bus, registry) = make_test_bus().await;
    registry
        .set_app_context("svc1".to_string(), "app1".to_string(), "s1".to_string())
        .await
        .unwrap();

    bus.record("svc1", "test.kind", "sub", None, 1000).unwrap();
    let foreign = Cursor { run_id: "0011223344556677".to_string(), seq: 1 };

    match bus.read("app1", Some(&foreign), 10, 1000) {
        ReadOutcome::Reload { now } => assert_eq!(now.run_id, bus.run_id()),
        ReadOutcome::Events { .. } => panic!("expected reload"),
    }
}

#[tokio::test]
async fn a_cursor_ahead_of_the_head_is_a_reload() {
    let (bus, registry) = make_test_bus().await;
    registry
        .set_app_context("svc1".to_string(), "app1".to_string(), "s1".to_string())
        .await
        .unwrap();

    bus.record("svc1", "test.kind", "sub", None, 1000).unwrap();
    let future = Cursor { run_id: bus.run_id().to_string(), seq: 999 };

    match bus.read("app1", Some(&future), 10, 1000) {
        ReadOutcome::Reload { now } => assert_eq!(now.seq, 1),
        ReadOutcome::Events { .. } => panic!("expected reload"),
    }
}

#[test]
fn malformed_cursor_text_is_invalid_argument() {
    assert_eq!(Cursor::parse(""), Err(EventError::InvalidArgument("cursor".to_string())));
    assert_eq!(Cursor::parse("nohyphen"), Err(EventError::InvalidArgument("cursor".to_string())));
    assert_eq!(Cursor::parse("short-1"), Err(EventError::InvalidArgument("cursor".to_string())));
    assert_eq!(
        Cursor::parse("3F9A0C1D22E47B65-1"),
        Err(EventError::InvalidArgument("cursor".to_string()))
    );
    assert_eq!(
        Cursor::parse("3f9a0c1d22e47b65-abc"),
        Err(EventError::InvalidArgument("cursor".to_string()))
    );

    let parsed = Cursor::parse("3f9a0c1d22e47b65-42").unwrap();
    assert_eq!(parsed.run_id, "3f9a0c1d22e47b65");
    assert_eq!(parsed.seq, 42);
    assert_eq!(parsed.to_string(), "3f9a0c1d22e47b65-42");
}

#[tokio::test]
async fn kind_subject_and_item_limits_name_the_field() {
    let (bus, registry) = make_test_bus().await;
    registry
        .set_app_context("svc1".to_string(), "app1".to_string(), "s1".to_string())
        .await
        .unwrap();

    // Kind tests
    assert_eq!(
        bus.record("svc1", "", "sub", None, 1000).unwrap_err(),
        EventError::InvalidArgument("kind".to_string())
    );
    assert_eq!(
        bus.record("svc1", &"a".repeat(65), "sub", None, 1000).unwrap_err(),
        EventError::InvalidArgument("kind".to_string())
    );
    assert_eq!(
        bus.record("svc1", "1kind", "sub", None, 1000).unwrap_err(),
        EventError::InvalidArgument("kind".to_string())
    );
    assert_eq!(
        bus.record("svc1", "-kind", "sub", None, 1000).unwrap_err(),
        EventError::InvalidArgument("kind".to_string())
    );
    assert_eq!(
        bus.record("svc1", "Upper", "sub", None, 1000).unwrap_err(),
        EventError::InvalidArgument("kind".to_string())
    );
    assert_eq!(
        bus.record("svc1", "has space", "sub", None, 1000).unwrap_err(),
        EventError::InvalidArgument("kind".to_string())
    );

    // Subject tests
    assert_eq!(
        bus.record("svc1", "valid.kind", "", None, 1000).unwrap_err(),
        EventError::InvalidArgument("subject".to_string())
    );
    assert_eq!(
        bus.record("svc1", "valid.kind", &"s".repeat(129), None, 1000).unwrap_err(),
        EventError::InvalidArgument("subject".to_string())
    );
    assert_eq!(
        bus.record("svc1", "valid.kind", "sub\nwith\nctrl", None, 1000).unwrap_err(),
        EventError::InvalidArgument("subject".to_string())
    );

    // Item tests
    assert_eq!(
        bus.record("svc1", "valid.kind", "sub", Some(""), 1000).unwrap_err(),
        EventError::InvalidArgument("item".to_string())
    );
    assert_eq!(
        bus.record("svc1", "valid.kind", "sub", Some(&"i".repeat(129)), 1000).unwrap_err(),
        EventError::InvalidArgument("item".to_string())
    );
    assert_eq!(
        bus.record("svc1", "valid.kind", "sub", Some("item\0null"), 1000).unwrap_err(),
        EventError::InvalidArgument("item".to_string())
    );
}

#[tokio::test]
async fn changed_wakes_after_a_record() {
    let (bus, registry) = make_test_bus().await;
    registry
        .set_app_context("svc1".to_string(), "app1".to_string(), "s1".to_string())
        .await
        .unwrap();

    let mut changed = bus.changed("app1");
    assert_eq!(*changed.borrow_and_update(), 0);

    bus.record("svc1", "test.kind", "sub", None, 1000).unwrap();
    changed.changed().await.unwrap();
    assert_eq!(*changed.borrow_and_update(), 1);
}

#[tokio::test]
async fn after_all_events_age_out_a_new_reader_does_not_get_a_false_reload() {
    let (bus, registry) = make_test_bus().await;
    registry
        .set_app_context("svc1".to_string(), "app1".to_string(), "s1".to_string())
        .await
        .unwrap();

    for _ in 1..=5 {
        bus.record("svc1", "test.kind", "sub", None, 1000).unwrap();
    }

    let t2 = 1000 + WINDOW_AGE_MS + 5000;
    // New reader starts at head
    let read_head = match bus.read("app1", None, 10, t2) {
        ReadOutcome::Events { events, last } => {
            assert!(events.is_empty());
            assert_eq!(last.seq, 5);
            last
        }
        ReadOutcome::Reload { .. } => panic!("new reader should get head"),
    };

    // A new event arrives
    bus.record("svc1", "test.kind", "sub6", None, t2).unwrap();

    // The reader resumes from read_head (seq 5). It must get event 6, not reload.
    match bus.read("app1", Some(&read_head), 10, t2) {
        ReadOutcome::Events { events, last } => {
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].cursor.seq, 6);
            assert_eq!(events[0].subject, "sub6");
            assert_eq!(last.seq, 6);
        }
        ReadOutcome::Reload { .. } => panic!("expected event 6, got false reload"),
    }
}

#[tokio::test]
async fn after_all_events_age_out_the_old_head_cursor_is_not_a_reload() {
    let (bus, registry) = make_test_bus().await;
    registry
        .set_app_context("svc1".to_string(), "app1".to_string(), "s1".to_string())
        .await
        .unwrap();

    for _ in 1..=5 {
        bus.record("svc1", "test.kind", "sub", None, 1000).unwrap();
    }
    let old_head = bus.head("app1");
    assert_eq!(old_head.seq, 5);

    let t2 = 1000 + WINDOW_AGE_MS + 5000;
    // All events are aged out. Reading at the old head cursor should return empty
    // events, not reload.
    match bus.read("app1", Some(&old_head), 10, t2) {
        ReadOutcome::Events { events, last } => {
            assert!(events.is_empty());
            assert_eq!(last, old_head);
        }
        ReadOutcome::Reload { .. } => panic!("old head cursor should not get reload"),
    }
}

#[tokio::test]
async fn read_with_limit_zero_returns_no_events_and_the_cursor() {
    let (bus, registry) = make_test_bus().await;
    registry
        .set_app_context("svc1".to_string(), "app1".to_string(), "s1".to_string())
        .await
        .unwrap();

    bus.record("svc1", "test.kind", "sub1", None, 1000).unwrap();
    bus.record("svc1", "test.kind", "sub2", None, 1001).unwrap();

    let cur1 = Cursor { run_id: bus.run_id().to_string(), seq: 1 };

    match bus.read("app1", Some(&cur1), 0, 1002) {
        ReadOutcome::Events { events, last } => {
            assert!(events.is_empty());
            assert_eq!(last, cur1);
        }
        ReadOutcome::Reload { .. } => panic!("expected events"),
    }

    match bus.read("app1", None, 0, 1002) {
        ReadOutcome::Events { events, last } => {
            assert!(events.is_empty());
            assert_eq!(last.seq, 2);
        }
        ReadOutcome::Reload { .. } => panic!("expected head"),
    }

    let future = Cursor { run_id: bus.run_id().to_string(), seq: 999 };
    assert!(matches!(bus.read("app1", Some(&future), 0, 1002), ReadOutcome::Reload { .. }));
}
