//! The two state changes of a deposit: the message state and the outbox
//! row change in one commit.

use anyhow::Result;
use rusqlite::{OptionalExtension, Transaction, params};
use syneroym_rpc::ConversationDeliveryState;

use super::{ConversationStore, state_str};

/// Moves the message (1:1) or one member's copy (group) from `from` to `to`,
/// through `tx`, and says whether a row changed. The change is conditional
/// because the worker read the message before it waited on the network: a
/// delete or a restore made meanwhile must stay, so a message that is no
/// longer in `from`, or was deleted or restored, is left as it is.
///
/// Never call `set_state` or `set_recipient_state` inside a queue
/// transaction: both lock the connection the transaction holds.
fn move_state_in(
    tx: &Transaction<'_>,
    message_id: &str,
    member: Option<&str>,
    from: ConversationDeliveryState,
    to: ConversationDeliveryState,
) -> rusqlite::Result<bool> {
    let (from, to) = (state_str(from), state_str(to));
    let changed = match member {
        None => tx.execute(
            "UPDATE messages SET state = ?1, last_error = NULL WHERE id = ?2 AND state = ?3 AND \
             deleted_at IS NULL AND restored = 0",
            params![to, message_id, from],
        )?,
        Some(member) => tx.execute(
            "UPDATE message_recipients SET state = ?1, last_error = NULL WHERE message_id = ?2 \
             AND member_address = ?3 AND state = ?4 AND EXISTS (SELECT 1 FROM messages WHERE id = \
             ?2 AND deleted_at IS NULL AND restored = 0)",
            params![to, message_id, member, from],
        )?,
    };
    Ok(changed > 0)
}

#[allow(clippy::expect_used)]
impl ConversationStore {
    /// Marks the message (1:1) or one member's copy (group) `deposited` and
    /// parks its outbox row, in one commit. The worker deletes any row whose
    /// message state it does not expect, so the two must never be seen apart.
    ///
    /// Returns `false`, and parks nothing, when the message is no longer
    /// `pending` (it was deleted or restored while the deposit was made).
    pub(crate) fn park_deposited(
        &self,
        item_id: i64,
        message_id: &str,
        member: Option<&str>,
        park: &Park<'_>,
    ) -> Result<bool> {
        use ConversationDeliveryState::{Deposited, Pending};
        self.queue.transaction(|tx, txq| {
            if !move_state_in(tx, message_id, member, Pending, Deposited)? {
                return Ok(false);
            }
            txq.park(tx, item_id, park.now, park.poll_interval_ms, park.deadline_at, park.context)?;
            Ok(true)
        })
    }

    /// The other way: back to `pending`, and the row back to direct attempts.
    /// Returns `false`, and leaves the row parked, when the message is no
    /// longer `deposited` (it was deleted or restored while the poll ran).
    pub(crate) fn unpark_to_pending(
        &self,
        item_id: i64,
        message_id: &str,
        member: Option<&str>,
        now: i64,
    ) -> Result<bool> {
        use ConversationDeliveryState::{Deposited, Pending};
        self.queue.transaction(|tx, txq| {
            if !move_state_in(tx, message_id, member, Deposited, Pending)? {
                return Ok(false);
            }
            txq.unpark(tx, item_id, now)?;
            Ok(true)
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
