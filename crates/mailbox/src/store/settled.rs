//! The cap on settled receipts: receipts of messages the mailbox no longer
//! holds. When there are too many, the senders that hold the most lose their
//! oldest receipts first, so one sender cannot push the receipts of others
//! out.

use std::collections::BTreeMap;

use rusqlite::{Connection, Transaction};

use crate::{
    error::MailboxError,
    store::{from_sql, to_sql},
};

/// The most receipts one pass drops. It keeps a single transaction short
/// when the cap was lowered a lot; later settles and sweeps drop the rest.
const MAX_DROP_BATCH: u64 = 1_000;

/// Drops settled receipts when there are more than `max`, and returns how
/// many it dropped. A pass drops a little more than the excess (1% of the
/// cap, at most [`MAX_DROP_BATCH`] in all), so a node at its cap does not
/// scan the receipts again on every settle. A sender that asks about a
/// dropped receipt reads `unknown` and goes back to direct attempts.
pub(super) fn drop_settled_over_cap(tx: &Transaction, max: u64) -> Result<u64, MailboxError> {
    let count: i64 =
        tx.query_row("SELECT settled_receipts FROM mailbox_stats WHERE id = 1", [], |r| r.get(0))?;
    let count = from_sql(count);
    if count <= max {
        return Ok(0);
    }
    let want = (count - max).saturating_add(max / 100).min(MAX_DROP_BATCH);
    let mut dropped = 0u64;
    for (sender, n) in fair_cuts(settled_per_sender(tx)?, want) {
        dropped += tx.execute(
            "DELETE FROM receipts WHERE rowid IN (SELECT rowid FROM receipts WHERE settled = 1 \
             AND sender = ?1 ORDER BY expires_at ASC LIMIT ?2)",
            rusqlite::params![sender, to_sql(n)],
        )? as u64;
    }
    tx.execute(
        "UPDATE mailbox_stats SET settled_receipts = MAX(0, settled_receipts - ?1) WHERE id = 1",
        [to_sql(dropped)],
    )?;
    Ok(dropped)
}

/// Brings the settled receipts down to `max` before the mailbox serves
/// anything, in bounded passes. It matters when the cap was lowered since
/// the last start.
pub(super) fn trim_at_open(conn: &mut Connection, max: u64) -> Result<(), MailboxError> {
    loop {
        let tx = conn.transaction()?;
        let dropped = drop_settled_over_cap(&tx, max)?;
        tx.commit()?;
        if dropped == 0 {
            return Ok(());
        }
    }
}

fn settled_per_sender(tx: &Transaction) -> Result<Vec<(String, u64)>, MailboxError> {
    let mut stmt = tx.prepare(
        "SELECT sender, COUNT(*) FROM receipts WHERE settled = 1 GROUP BY sender ORDER BY sender",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, from_sql(r.get(1)?))))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// How many receipts to drop from each sender to drop `want` in all, taking
/// from the senders that hold the most. It lowers one common level over the
/// senders: those above it are cut down to it, and when a few more are
/// needed, one more is taken from senders that sit at the level, in the order
/// of their names.
pub(super) fn fair_cuts(counts: Vec<(String, u64)>, want: u64) -> Vec<(String, u64)> {
    let over = |level: u64| -> u64 { counts.iter().map(|(_, c)| c.saturating_sub(level)).sum() };
    let (mut low, mut high) = (0, counts.iter().map(|(_, c)| *c).max().unwrap_or(0));
    while low < high {
        let mid = low + (high - low) / 2;
        if over(mid) <= want {
            high = mid;
        } else {
            low = mid + 1;
        }
    }
    let level = low;
    let mut spare = want.saturating_sub(over(level));
    let mut cuts = BTreeMap::new();
    for (sender, count) in &counts {
        let mut cut = count.saturating_sub(level);
        if spare > 0 && level > 0 && *count >= level {
            cut += 1;
            spare -= 1;
        }
        if cut > 0 {
            cuts.insert(sender.clone(), cut);
        }
    }
    cuts.into_iter().collect()
}
