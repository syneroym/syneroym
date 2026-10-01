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
    pub conversation_id: Option<String>,
}

static DROP_ACKS: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Mutex::default);
static SEND_OVERRIDES: LazyLock<Mutex<HashMap<String, SendOverride>>> =
    LazyLock::new(Mutex::default);
static CLOCK_OFFSETS: LazyLock<Mutex<HashMap<String, i64>>> = LazyLock::new(Mutex::default);

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

/// Whether a `drop_next_ack` for `service_id` has not fired yet. A test
/// reads it after the delivery to prove the hook really ran.
pub fn drop_ack_pending(service_id: &str) -> bool {
    locked(&DROP_ACKS).contains(service_id)
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

/// From now on, `service_id` signs group entries as if its clock were
/// `offset_ms` ahead (negative: behind). The receiver's own checks use
/// the real clock, as they would against a real skewed peer.
pub fn set_clock_offset_ms(service_id: &str, offset_ms: i64) {
    locked(&CLOCK_OFFSETS).insert(service_id.to_string(), offset_ms);
}

pub fn clear_clock_offsets() {
    locked(&CLOCK_OFFSETS).clear();
}

pub(crate) fn clock_offset_ms(service_id: &str) -> i64 {
    locked(&CLOCK_OFFSETS).get(service_id).copied().unwrap_or(0)
}
