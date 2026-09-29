//! One-shot misbehaviour for cross-node tests. Compiled only with the
//! `test-support` feature, which only `syneroym-substrate`'s dev build
//! enables. Every hook is keyed by the *sending* service id, because all
//! test nodes run as tasks inside one process: an unkeyed hook could be
//! taken by another node's outbox worker.

use std::{
    collections::{HashMap, HashSet},
    sync::{LazyLock, Mutex, MutexGuard, PoisonError},
};

/// What the next message a service sends will falsely claim. The message
/// is still signed by the real sender key: only its claims are false.
#[derive(Debug, Default, Clone)]
pub struct SendOverride {
    pub author: Option<String>,
    pub sender_timestamp_ms: Option<i64>,
}

static DROP_ACKS: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Mutex::default);
static SEND_OVERRIDES: LazyLock<Mutex<HashMap<String, SendOverride>>> =
    LazyLock::new(Mutex::default);

/// A poisoned lock still holds a valid set: a panicking test must not turn
/// every later test's hook into a second panic.
fn locked<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The next delivery `service_id` makes: the peer stores it and answers,
/// and the sender then behaves as if the answer never arrived.
pub fn drop_next_ack(service_id: &str) {
    locked(&DROP_ACKS).insert(service_id.to_string());
}

/// The next `send` on `service_id` uses these values in the message it
/// signs and stores.
pub fn override_next_send(service_id: &str, o: SendOverride) {
    locked(&SEND_OVERRIDES).insert(service_id.to_string(), o);
}

pub(crate) fn take_drop_ack(service_id: &str) -> bool {
    locked(&DROP_ACKS).remove(service_id)
}

pub(crate) fn take_send_override(service_id: &str) -> Option<SendOverride> {
    locked(&SEND_OVERRIDES).remove(service_id)
}
