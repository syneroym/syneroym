//! The two state changes of a deposit: the message state and the outbox
//! row change in one commit.

use anyhow::Result;
use rusqlite::{OptionalExtension, Transaction, params};
use syneroym_rpc::ConversationDeliveryState;

use super::{ConversationStore, state_str};

/// Through `tx`. Never call `set_state` or `set_recipient_state` inside a
/// queue transaction: both lock the connection the transaction holds.
fn set_state_in(
    tx: &Transaction<'_>,
    message_id: &str,
    member: Option<&str>,
    state: ConversationDeliveryState,
) -> rusqlite::Result<()> {
    let state = state_str(state);
    match member {
        None => tx.execute(
            "UPDATE messages SET state = ?1, last_error = NULL WHERE id = ?2",
            params![state, message_id],
        ),
        Some(member) => tx.execute(
            "UPDATE message_recipients SET state = ?1, last_error = NULL WHERE message_id = ?2 \
             AND member_address = ?3",
            params![state, message_id, member],
        ),
    }
    .map(|_| ())
}

#[allow(clippy::expect_used)]
impl ConversationStore {
    /// Marks the message (1:1) or one member's copy (group) `deposited` and
    /// parks its outbox row, in one commit. The worker deletes any row whose
    /// message state it does not expect, so the two must never be seen apart.
    pub(crate) fn park_deposited(
        &self,
        item_id: i64,
        message_id: &str,
        member: Option<&str>,
        park: &Park<'_>,
    ) -> Result<()> {
        self.queue.transaction(|tx, txq| {
            set_state_in(tx, message_id, member, ConversationDeliveryState::Deposited)?;
            txq.park(tx, item_id, park.now, park.poll_interval_ms, park.deadline_at, park.context)
        })
    }

    /// The other way: back to `pending`, and the row back to direct attempts.
    pub(crate) fn unpark_to_pending(
        &self,
        item_id: i64,
        message_id: &str,
        member: Option<&str>,
        now: i64,
    ) -> Result<()> {
        self.queue.transaction(|tx, txq| {
            set_state_in(tx, message_id, member, ConversationDeliveryState::Pending)?;
            txq.unpark(tx, item_id, now)
        })
    }

    /// One member's state in a group message.
    pub(crate) fn recipient_state(
        &self,
        message_id: &str,
        member: &str,
    ) -> Result<Option<ConversationDeliveryState>> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        let state: Option<String> = conn
            .query_row(
                "SELECT state FROM message_recipients WHERE message_id = ?1 AND member_address = \
                 ?2",
                params![message_id, member],
                |row| row.get(0),
            )
            .optional()?;
        Ok(state.as_deref().map(super::state_from_str))
    }
}

/// How a deposited row waits.
pub(crate) struct Park<'a> {
    pub now: i64,
    pub poll_interval_ms: u64,
    pub deadline_at: i64,
    pub context: &'a [u8],
}
