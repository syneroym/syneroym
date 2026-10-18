use super::*;

#[test]
fn the_default_role_is_valid() {
    let role = MailboxRole::default();
    let limits = Limits::from_role(&role).expect("default role must be valid");
    assert_eq!(limits.queue.max_bytes, role.max_queue_bytes);
    assert_eq!(limits.queue.max_messages, role.max_queue_messages);
    assert_eq!(limits.queue.max_ttl_secs, role.max_ttl_secs);
    assert_eq!(limits.receipt_keep_secs, role.receipt_keep_secs);
    assert_eq!(limits.visibility_secs, role.visibility_secs);
    assert_eq!(limits.queue_lifetime_secs, role.queue_lifetime_secs);
    assert_eq!(limits.max_total_bytes, role.max_total_bytes);
    assert_eq!(limits.max_queues, role.max_queues);
}

#[test]
fn zero_values_are_refused() {
    let role = MailboxRole { max_queue_bytes: 0, ..Default::default() };
    let err = Limits::from_role(&role).unwrap_err().to_string();
    assert!(err.contains("max_queue_bytes"));

    let role = MailboxRole { max_queue_messages: 0, ..Default::default() };
    let err = Limits::from_role(&role).unwrap_err().to_string();
    assert!(err.contains("max_queue_messages"));

    let role = MailboxRole { max_ttl_secs: 0, ..Default::default() };
    let err = Limits::from_role(&role).unwrap_err().to_string();
    assert!(err.contains("max_ttl_secs"));

    let role = MailboxRole { receipt_keep_secs: 0, ..Default::default() };
    let err = Limits::from_role(&role).unwrap_err().to_string();
    assert!(err.contains("receipt_keep_secs"));

    let role = MailboxRole { visibility_secs: 0, ..Default::default() };
    let err = Limits::from_role(&role).unwrap_err().to_string();
    assert!(err.contains("visibility_secs"));

    let role = MailboxRole { queue_lifetime_secs: 0, ..Default::default() };
    let err = Limits::from_role(&role).unwrap_err().to_string();
    assert!(err.contains("queue_lifetime_secs"));

    let role = MailboxRole { max_total_bytes: 0, ..Default::default() };
    let err = Limits::from_role(&role).unwrap_err().to_string();
    assert!(err.contains("max_total_bytes"));

    let role = MailboxRole { max_queues: 0, ..Default::default() };
    let err = Limits::from_role(&role).unwrap_err().to_string();
    assert!(err.contains("max_queues"));

    let role = MailboxRole { deposits_per_minute_per_sender: 0, ..Default::default() };
    let err = Limits::from_role(&role).unwrap_err().to_string();
    assert!(err.contains("deposits_per_minute_per_sender"));

    let role = MailboxRole { deposits_per_minute_total: 0, ..Default::default() };
    let err = Limits::from_role(&role).unwrap_err().to_string();
    assert!(err.contains("deposits_per_minute_total"));

    let role = MailboxRole { sweep_interval_secs: 0, ..Default::default() };
    let err = Limits::from_role(&role).unwrap_err().to_string();
    assert!(err.contains("sweep_interval_secs"));

    let role = MailboxRole { republish_interval_secs: 0, ..Default::default() };
    let err = Limits::from_role(&role).unwrap_err().to_string();
    assert!(err.contains("republish_interval_secs"));
}

#[test]
fn ttl_above_open_max_is_refused() {
    let role = MailboxRole { max_ttl_secs: OPEN_MAX_TTL_SECS + 1, ..Default::default() };
    let err = Limits::from_role(&role).unwrap_err().to_string();
    assert!(err.contains("max_ttl_secs"));
}

#[test]
fn queue_bytes_under_sealed_max_is_refused() {
    let role = MailboxRole { max_queue_bytes: (MAX_SEALED_BYTES as u64) - 1, ..Default::default() };
    let err = Limits::from_role(&role).unwrap_err().to_string();
    assert!(err.contains("max_queue_bytes"));
}

#[test]
fn total_bytes_under_queue_bytes_is_refused() {
    let mut role = MailboxRole::default();
    role.max_total_bytes = role.max_queue_bytes - 1;
    let err = Limits::from_role(&role).unwrap_err().to_string();
    assert!(err.contains("max_total_bytes"));
}

#[test]
fn invalid_db_names_are_refused() {
    for bad in ["", "sub/dir.db", "sub\\dir.db", "../escape.db"] {
        let role = MailboxRole { db_name: bad.to_string(), ..Default::default() };
        let err = Limits::from_role(&role).unwrap_err().to_string();
        assert!(err.contains("db_name"), "expected refusal for '{bad}'");
    }
}
