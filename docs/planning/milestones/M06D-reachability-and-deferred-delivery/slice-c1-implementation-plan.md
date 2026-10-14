# M06D slice C1 — Parked queue state: implementation plan

*Reader: the developer (or AI) who implements slice C1. You do not have the
planning conversation. This plan is meant to stand alone.*

## 0. Read first, and the rules

Read these before you change anything:

- `AGENTS.md`. All rules apply. The ones this slice touches most:
  `mise run verify` before you finish, the import clean-up pass, the
  living-docs close-out, the deferred backlog, no planning references in code
  (no "M06D", "C1", "C5", "X6", "S-9" in code, comments or test names; ADR ids
  are fine), simple English in comments, the 800-line file cap, the 100-line
  function cap, and the module layout rule (`foo.rs` beside `foo/`, never
  `mod.rs` under `src/`).
  **This plan names its decisions `C1-1` to `C1-11`. Those ids are for this
  plan only. When a step says to write a comment "for the reason in C1-n",
  write the reason itself in the comment, never the id.**
- `task.md` in this folder: §1 row X6, §3 slice C1 and slice C5.
- `docs/planning/changes/service-mailbox/change.md`: criteria S-3, S-5, S-6,
  S-9; "Sender flow"; the "Attempt budget" paragraph; `## Tasks`;
  `## Deviations`.
- [ADR-0023](../../../decisions/0023-durable-async-primitives.md) §1, §4, §5
  (the queue is owner-local, at-least-once, and the dead-letter table is an
  operator surface).
- `crates/async_queue/src/lib.rs`, `src/queue.rs`, `src/queue/types.rs`,
  `src/queue/tests.rs`.

**Branch.** `feat/m06d-c1-implementation` does not exist yet (not local, not on
`origin`, checked 2026-10-13). Create it from `main`:

```bash
git fetch origin main
git switch -c feat/m06d-c1-implementation origin/main
```

C1 needs no other slice. A3 is merged into `main` (#303, commit `540341e9`).
A3 did not touch `crates/async_queue`, `crates/conversation`, `crates/router`
or `crates/app_supervisor` in any way that this plan depends on. The line
numbers in this plan were read before that merge. If one does not match, find
the code by the function name given next to it.

## 1. What C1 builds, in one paragraph

`syneroym-async-queue` gets a second state for an outbox row: **parked**. A
parked row is still in the `outbox` table, so it still holds its key, its
payload and its place under `max_pending_rows`. But it is outside the attempt
budget: `claim_due` never returns it, and claiming it never advances
`claim_count` or `attempts`. It has its own poll interval, its own deadline and
an opaque context blob (C5 stores the mailbox deposit receipt there). A new
`claim_parked_due` returns parked rows whose poll time (or deadline) has come.
The caller then re-parks, completes, fails or unparks the row. `park` and
`unpark` also exist on `TxQueue`, so an owner can park in the same
transaction as its own state change. No outbox uses this yet; C5 does. The
supervisor, the proxy outbox and the conversation outbox must behave exactly
as before.

Why not reuse `Queue::defer`: a deferred row comes back through `claim_due`, so
the worker would make a direct delivery attempt (S-3 forbids that after a
deposit). `defer` also has no deadline, no context and no state that
`Queue::all` can show (S-6).

## 2. Decisions this plan makes (check them before you start)

Each of these fills a gap in task.md or the change doc. They are meant to
let C5 use `async_queue` without changing its API. All of them are decided;
the implementer does not need to ask anyone.

| # | Decision | Why |
| --- | --- | --- |
| C1-1 | The generic name in the crate is **parked**, not "deposited". | `async_queue` never knows about mailboxes. C5 maps "parked" to the `deposited` outbox state. |
| C1-2 | A row is parked if and only if `park_deadline IS NOT NULL`. Four new nullable columns: `parked_at`, `park_deadline`, `park_poll_ms`, `park_context`. They are set together and cleared together. No separate `state` column. | One source of truth. Two partial indexes give each claim query its own index. A check with the system `sqlite3` CLI (3.51) planned both queries as `SEARCH ... USING INDEX`. The build uses bundled SQLCipher (`rusqlite` `bundled-sqlcipher`), so that check is only a hint. The proof is the two query-plan tests in §5 and §6 (test 17), which run on the real build. |
| C1-3 | The crate does **not** act at the deadline. `park` schedules the next poll no later than the deadline (`visible_at = min(now + poll, deadline)`), and `Parked::is_past_deadline(now)` reports it. The caller chooses `unpark` or `fail`. | S-5 has two different outcomes (an expired receipt goes back to direct attempts; a rejected one fails). Only the caller knows which applies. |
| C1-4 | The deposit receipt (or any caller data) is kept in `park_context`, an opaque blob. `payload` is never changed by parking. | S-5: the sender keeps the original payload until a signed `delivered` receipt, and may need to send it directly again. A caller-side table would need its own cleanup on `complete`/`fail`/dead-letter. |
| C1-5 | The first `park` of a row un-counts the claim that led to it (same as `defer`): `claim_count = MAX(claim_count - 1, 0)`. A re-park of an already parked row does not change `claim_count`. `attempts` is never changed by `park`, `unpark` or `claim_parked_due`. | S-9: outside the attempt budget **and** outside the claim-count limit. The claim-count limit is checked by each worker (`item.claim_count > max_attempts`), not by the crate, so the crate keeps `claim_count` from growing while parked. The parked deadline is the outer bound for a poll that crashes every time. |
| C1-6 | `unpark` keeps `attempts` and `claim_count` as they are. There is no way to reset them, and none is planned. | The proxy outbox has no age limit; its attempt budget is its only bound. With a reset, a row whose receipts keep coming back `expired` would cycle (direct attempts, deposit, expiry) forever. Keeping `attempts` makes every cycle use budget, so the row ends as a dead letter, which an operator can replay (ADR-0023 §5). The Conversation outbox is also bound by its 30-day age limit. Normally a row is parked after one direct attempt (S-2), so after an expiry it still has almost all of its budget. |
| C1-7 | A non-terminal `fail` on a parked row clears the park columns (the row is pending again, with one more attempt charged). A terminal `fail` dead-letters it as today; the context is dropped. | `fail` means "an attempt failed". A parked row with a backoff `visible_at` but still parked would be a mixed state. |
| C1-8 | `defer` has no effect on a parked row (its `WHERE` excludes parked rows). It still returns `Ok(())` when no row matches, as today. | Without this guard, `defer` would subtract from a `claim_count` that the parked claim never added. |
| C1-9 | Parked rows count in `pending_count` (so in `max_pending_rows`), `has_pending`, `enqueue_if_absent` and the `replay` refusal. | The one-row-per-key invariant must hold while a message is deposited. A parked row is still kept data; it must stay bounded. |
| C1-10 | Existing database files get the new columns with an idempotent `ALTER TABLE ... ADD COLUMN` that ignores "duplicate column name". **Decided: keep it.** | The conversation queue lives **inside** the conversation store's database (`crates/conversation/src/store/schema.rs:32`, `Queue::from_connection`). Without the `ALTER`, an existing node fails every claim, and deleting the file to recover deletes chat history. The same pattern, with the same "not a version ladder" reasoning, is at `crates/app_supervisor/src/store/schema.rs:63-80` and `crates/data_db/src/registry_store.rs:135-150`. It needs no schema version. |
| C1-11 | `TxQueue` gets `park` and `unpark`, which write through the owner's transaction. `Queue::park`/`unpark` and the `TxQueue` versions share one SQL function each. | The Conversation worker completes (deletes) any row whose message is not `Pending` (`crates/conversation/src/outbox.rs:190`). If C5 set the message to `deposited` and parked the row as two separate writes, a crash between them could leave a `deposited` message with no row, which nothing would ever finish. The two writes must commit together. `Queue::transaction` and `TxQueue` exist for exactly this case (`store/message.rs:73`). The owner's own write must also go through the `&Transaction`. A store method that locks the shared connection itself (for example `ConversationStore::set_state`) hangs the thread if it is called inside the closure. |

## 3. Production code changes

All changes are in `crates/async_queue`. No other crate changes.

### 3.1 `crates/async_queue/src/queue/types.rs`

Add a new public type after `QueueItem`:

```rust
/// How a parked item waits: outside the attempt budget, on its own poll
/// interval, until its own deadline. A parked item is never returned by
/// [`Queue::claim_due`]; only [`Queue::claim_parked_due`] returns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parked {
    /// When the item was first parked (Unix ms). A re-park keeps it.
    pub parked_at: i64,
    /// Unix ms. [`Queue::park`] schedules the next poll no later than
    /// this. A claimed item that the caller leaves unresolved comes back
    /// only after the visibility timeout, which can be after this time.
    pub deadline_at: i64,
    pub poll_interval_ms: u64,
    /// The caller's own data for this wait, for example a deposit
    /// receipt. Opaque to this crate, like the payload.
    pub context: Vec<u8>,
}

impl Parked {
    /// Whether the deadline has come. The queue does not act on it; the
    /// caller decides to unpark or to fail the item.
    #[must_use]
    pub fn is_past_deadline(&self, now: i64) -> bool {
        now >= self.deadline_at
    }
}
```

Add one field to `QueueItem` (last field):

```rust
    /// `Some` while the item is parked. Always `None` for an item from
    /// [`Queue::claim_due`]; always `Some` for one from
    /// [`Queue::claim_parked_due`]. [`Queue::all`] returns both kinds.
    pub parked: Option<Parked>,
```

Update the doc of `QueueItem::claim_count`: a parked claim does not advance it.

Only two places build a `QueueItem` literal, both in `queue.rs`
(`claim_due` and `all`). No other crate builds one (checked with
`grep -rn 'QueueItem {' crates apps`). Other crates only read `id`,
`queue_key`, `payload`, `attempts` and `claim_count`, so the new field breaks
no call site.

### 3.2 Re-exports: `crates/async_queue/src/queue.rs` and `src/lib.rs`

`Parked` must be added in **two** places, because `lib.rs` re-exports through
`queue`:

- `crates/async_queue/src/queue.rs:13`: add `Parked` to
  `pub use types::{DEFAULT_MAX_PENDING_ROWS, DeadLetter, FailOutcome, QueueConfig, QueueItem};`.
- `crates/async_queue/src/lib.rs`: add `Parked` to the `queue::{...}` list in
  `pub use crate::{...}`.

### 3.3 `crates/async_queue/src/queue.rs` — schema

Replace `init_schema` with this order. **The order matters:** the partial
indexes name `park_deadline`, so they must be created after the `ALTER`s, or
an old database fails at open.

```rust
fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS outbox (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            group_key     TEXT NOT NULL,
            queue_key     TEXT NOT NULL,
            payload       BLOB NOT NULL,
            attempts      INTEGER NOT NULL DEFAULT 0,
            claim_count   INTEGER NOT NULL DEFAULT 0,
            visible_at    INTEGER NOT NULL,
            created_at    INTEGER NOT NULL,
            parked_at     INTEGER,
            park_deadline INTEGER,
            park_poll_ms  INTEGER,
            park_context  BLOB
         );
         CREATE INDEX IF NOT EXISTS idx_outbox_queue_key ON outbox(queue_key);
         -- dead_letters table and its three indexes: unchanged
        ",
    )?;
    for column in PARK_COLUMNS {           // see below
        add_column_if_missing(conn, "outbox", column)?;
    }
    conn.execute_batch(
        "DROP INDEX IF EXISTS idx_outbox_visible_at;
         CREATE INDEX IF NOT EXISTS idx_outbox_due
             ON outbox(visible_at) WHERE park_deadline IS NULL;
         CREATE INDEX IF NOT EXISTS idx_outbox_parked_due
             ON outbox(visible_at) WHERE park_deadline IS NOT NULL;",
    )?;
    Ok(())
}

const PARK_COLUMNS: [&str; 4] = [
    "parked_at INTEGER",
    "park_deadline INTEGER",
    "park_poll_ms INTEGER",
    "park_context BLOB",
];

/// Adds a column to a table that an older build created without it.
/// "duplicate column name" is the normal result on every open after the
/// first.
fn add_column_if_missing(conn: &Connection, table: &str, column_def: &str) -> Result<()> {
    if let Err(err) = conn.execute(&format!("ALTER TABLE {table} ADD COLUMN {column_def}"), [])
        && !err.to_string().contains("duplicate column name")
    {
        return Err(err.into());
    }
    Ok(())
}
```

Keep the existing SQL comments: move the comment that explains `visible_at`
(scheduling and visibility in one column) onto `idx_outbox_due`, and add one
line that says the two partial indexes split pending and parked rows, so each
claim is one indexed range scan. Keep the "Unconditional, not gated on
`PRAGMA user_version`" comment. Add that the `ALTER` is for files that an
older build created, and that it is not a version ladder (write the reason
from C1-10, not the id).

Why drop `idx_outbox_visible_at`: no query uses it after this change, and an
unused index costs a write on every enqueue, claim and fail.

### 3.4 `crates/async_queue/src/queue.rs` — shared row reading and the claim

Add, private to the module (child module `parked` can use private items of its
parent):

```rust
/// The columns every item read selects, in the order `read_item` expects.
const ITEM_COLUMNS: &str =
    "id, queue_key, payload, attempts, claim_count, parked_at, park_deadline, park_poll_ms, \
     park_context";

fn read_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<QueueItem> {
    let parked = match row.get::<_, Option<i64>>(6)? {
        None => None,
        // The four park columns are written together, so a NULL here is a
        // broken row; `get` of a non-Option type returns that as an error.
        Some(deadline_at) => Some(Parked {
            parked_at: row.get(5)?,
            deadline_at,
            poll_interval_ms: row.get::<_, i64>(7)? as u64,
            context: row.get(8)?,
        }),
    };
    Ok(QueueItem {
        id: row.get(0)?,
        queue_key: row.get(1)?,
        payload: row.get(2)?,
        attempts: row.get::<_, i64>(3)? as u32,
        claim_count: row.get::<_, i64>(4)? as u32,
        parked,
    })
}

/// Which rows a claim takes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ClaimKind { Pending, Parked }

impl ClaimKind {
    fn select_sql(self) -> String {
        let filter = match self {
            Self::Pending => "park_deadline IS NULL",
            Self::Parked => "park_deadline IS NOT NULL",
        };
        format!(
            "SELECT {ITEM_COLUMNS} FROM outbox WHERE {filter} AND visible_at <= ?1 \
             ORDER BY visible_at ASC LIMIT ?2"
        )
    }

    /// A pending claim counts against `claim_count`; a parked claim does
    /// not, because a parked item is outside the attempt budget.
    fn update_sql(self) -> &'static str {
        match self {
            Self::Pending => "UPDATE outbox SET visible_at = ?1, claim_count = claim_count + 1 \
                              WHERE id = ?2 AND visible_at <= ?3 AND park_deadline IS NULL",
            Self::Parked => "UPDATE outbox SET visible_at = ?1 \
                             WHERE id = ?2 AND visible_at <= ?3 AND park_deadline IS NOT NULL",
        }
    }
}
```

Replace the body of `claim_due` with a call to a shared private method, and
delete the local `Candidate` struct:

```rust
pub fn claim_due(&self, now: i64, limit: u32) -> Result<Vec<QueueItem>> {
    self.claim(now, limit, ClaimKind::Pending)
}

fn claim(&self, now: i64, limit: u32, kind: ClaimKind) -> Result<Vec<QueueItem>> {
    let conn = self.conn.lock().expect("queue connection lock poisoned");
    let locked_until = now + self.config.visibility_timeout_ms as i64;
    let candidates: Vec<QueueItem> = {
        let mut stmt = conn.prepare(&kind.select_sql())?;
        stmt.query_map(params![now, limit], read_item)?.collect::<rusqlite::Result<_>>()?
    };
    let mut claimed = Vec::with_capacity(candidates.len());
    for mut item in candidates {
        let updated = conn.execute(kind.update_sql(), params![locked_until, item.id, now])?;
        if updated == 0 {
            // keep the existing "Lost the row between the read above and this
            // write" comment here, unchanged
            continue;
        }
        if kind == ClaimKind::Pending {
            item.claim_count += 1;
        }
        claimed.push(item);
    }
    Ok(claimed)
}
```

Keep the existing doc comment of `claim_due`, and add one sentence: it never
returns a parked item.

`all()`: use `format!("SELECT {ITEM_COLUMNS} FROM outbox ORDER BY id ASC")`
and `query_map(.., read_item)`. Update its doc: it returns pending and parked
items.

`explain_claim_plan()` (test only): build its SQL from
`ClaimKind::Pending.select_sql()` (prefix `EXPLAIN QUERY PLAN `), so the test
checks the real query and cannot drift from it. Add a second test-only
`explain_parked_claim_plan()` the same way, or give the function a
`kind: ClaimKind` parameter (preferred: one function).

### 3.5 `crates/async_queue/src/queue.rs` — changes to existing methods

- **`defer`**: add `AND park_deadline IS NULL` to the `WHERE`. Add to the
  doc: "Has no effect on a parked item; re-park it with
  [`Self::park`] instead." Behaviour on a pending row is unchanged.
- **`fail`**, non-terminal branch only: the `UPDATE` becomes
  `UPDATE outbox SET attempts = ?1, visible_at = ?2, parked_at = NULL,
  park_deadline = NULL, park_poll_ms = NULL, park_context = NULL WHERE id = ?3`.
  Add one sentence to the doc: a failed attempt on a parked item makes it
  pending again, or dead-letters it when the budget is used up, the same as
  for a pending item. The terminal branch and `dead_letter` do not change.
- **`pending_count`**, **`has_pending`**, **`enqueue_if_absent`**, **`replay`**:
  no SQL change. Add one sentence to the docs of `pending_count` and
  `has_pending`: parked items count, because a parked item still holds its
  key and its data.
- **`complete`**, **`record_dead_letter`**, **`transaction`**,
  **`TxQueue::enqueue`**, **`dead_letters`**, **`max_attempts`**: no change.
  New rows get `NULL` park columns by default, so every `INSERT` stays as it is.

Share the "clear the park columns" text between `fail` and `unpark` with one
`const CLEAR_PARK: &str = "parked_at = NULL, park_deadline = NULL,
park_poll_ms = NULL, park_context = NULL";` in `queue.rs`, used through
`format!`. This keeps the two writes from drifting apart.

Add `mod parked;` near `pub mod types;` (private: it only holds an
`impl Queue` block).

### 3.6 New file `crates/async_queue/src/queue/parked.rs`

`queue.rs` is 637 lines today. The new methods go in a sibling file so it stays
well under the cap. The SQL lives in two free functions that take a
`&Connection`. `Queue` calls them on its locked connection. `TxQueue` calls
them on the owner's `&Transaction`, which derefs to `&Connection`. So the two
paths cannot drift apart (C1-11). File content:

```rust
//! The parked state: an outbox item that waits outside the attempt budget,
//! on its own poll interval, until its own deadline -- for example a
//! message handed to a third party, where the sender now only asks for a
//! receipt and must not deliver directly again.

use anyhow::{Result, ensure};
use rusqlite::{Connection, Transaction, params};

use super::{CLEAR_PARK, ClaimKind, Queue, QueueItem, TxQueue};

#[allow(clippy::expect_used)]   // same lock-poisoning reasoning as queue.rs
impl Queue {
    /// Parks an item, or updates an item that is already parked.
    ///
    /// The item stays in the outbox (it keeps its key and its payload) but
    /// [`Self::claim_due`] no longer returns it. [`Self::claim_parked_due`]
    /// returns it after `poll_interval_ms`, and never later than
    /// `deadline_at`. Parking never changes `attempts`. The first park
    /// un-counts the claim that led to it, as [`Self::defer`] does; a
    /// re-park does not change `claim_count`, since a parked claim never
    /// added to it.
    pub fn park(
        &self,
        id: i64,
        now: i64,
        poll_interval_ms: u64,
        deadline_at: i64,
        context: &[u8],
    ) -> Result<()> {
        let conn = self.conn.lock().expect("queue connection lock poisoned");
        park_on(&conn, id, now, poll_interval_ms, deadline_at, context)
    }

    /// Makes a parked item pending again, due at `visible_at`. `attempts`
    /// and `claim_count` keep the values they had when it was parked, so
    /// an item that is parked and unparked again and again still uses up
    /// its attempt budget and ends. Errors when the item does not exist or
    /// is not parked.
    pub fn unpark(&self, id: i64, visible_at: i64) -> Result<()> {
        let conn = self.conn.lock().expect("queue connection lock poisoned");
        unpark_on(&conn, id, visible_at)
    }

    /// Claims up to `limit` parked items whose poll time or deadline has
    /// come, and hides them for `visibility_timeout_ms`, as
    /// [`Self::claim_due`] does for pending items. A parked claim does not
    /// advance `claim_count` or `attempts`: the deadline, not the attempt
    /// budget, bounds a parked item. The caller resolves each item with
    /// [`Self::park`] (poll again later), [`Self::complete`],
    /// [`Self::fail`] or [`Self::unpark`]. An item it leaves alone comes
    /// back after the visibility timeout.
    pub fn claim_parked_due(&self, now: i64, limit: u32) -> Result<Vec<QueueItem>> {
        self.claim(now, limit, ClaimKind::Parked)
    }
}

impl TxQueue<'_> {
    /// Same effect as [`Queue::park`], through `tx`, so an owner can park
    /// an item in the same commit as its own state change.
    pub fn park(
        &self,
        tx: &Transaction<'_>,
        id: i64,
        now: i64,
        poll_interval_ms: u64,
        deadline_at: i64,
        context: &[u8],
    ) -> Result<()> {
        park_on(tx, id, now, poll_interval_ms, deadline_at, context)
    }

    /// Same effect as [`Queue::unpark`], through `tx`.
    pub fn unpark(&self, tx: &Transaction<'_>, id: i64, visible_at: i64) -> Result<()> {
        unpark_on(tx, id, visible_at)
    }
}

fn park_on(
    conn: &Connection,
    id: i64,
    now: i64,
    poll_interval_ms: u64,
    deadline_at: i64,
    context: &[u8],
) -> Result<()> {
    ensure!(poll_interval_ms > 0, "park poll interval must be above zero");
    ensure!(deadline_at > now, "park deadline {deadline_at} is not after now {now}");
    let poll = i64::try_from(poll_interval_ms).unwrap_or(i64::MAX);
    let visible_at = now.saturating_add(poll).min(deadline_at);
    // SQLite evaluates every SET expression against the row as it was
    // before this UPDATE, so the CASE sees the old park_deadline.
    let updated = conn.execute(
        "UPDATE outbox SET
             claim_count = CASE WHEN park_deadline IS NULL
                                THEN MAX(claim_count - 1, 0) ELSE claim_count END,
             parked_at = COALESCE(parked_at, ?1),
             park_deadline = ?2, park_poll_ms = ?3, park_context = ?4, visible_at = ?5
         WHERE id = ?6",
        params![now, deadline_at, poll, context, visible_at, id],
    )?;
    ensure!(updated == 1, "no outbox item with id {id}");
    Ok(())
}

fn unpark_on(conn: &Connection, id: i64, visible_at: i64) -> Result<()> {
    let updated = conn.execute(
        &format!(
            "UPDATE outbox SET {CLEAR_PARK}, visible_at = ?1 \
             WHERE id = ?2 AND park_deadline IS NOT NULL"
        ),
        params![visible_at, id],
    )?;
    ensure!(updated == 1, "no parked outbox item with id {id}");
    Ok(())
}
```

`TxQueue::park` has seven parameters with `self`. That is the most that
`clippy::too_many_arguments` allows with its default threshold (the workspace
`clippy.toml` does not change it), so no `allow` is needed. `claim`,
`ClaimKind` and `CLEAR_PARK` are private items of `queue.rs`. The child module
`queue::parked` can use them without `pub`. A `TxQueue` method does not take
the queue lock: the caller already holds it through `Queue::transaction`, the
same as `TxQueue::enqueue`.

## 4. Call sites outside the crate

**No code outside `crates/async_queue` changes in C1.** One site,
`route_handler.rs`, does not change but its rule breaks once C5 parks rows (see
the last row and §9 item 1). List made with
`grep -rn --include='*.rs' 'syneroym_async_queue' crates apps` and a `grep` for
each `Queue` method:

| Caller | Queue API used | Effect of C1 |
| --- | --- | --- |
| `crates/app_supervisor/src/service/queue_worker.rs` (`queue_worker_tick`, `fail_queued_item`, `parse_or_dead_letter`) | `claim_due`, `max_attempts`, `fail`, `dead_letters` | None. It never parks, so no parked row exists; `claim_due` and `fail` behave the same on pending rows. |
| `crates/app_supervisor/src/outbox.rs:163`, `service/bindings.rs:460` | `has_pending` | None. |
| `crates/app_supervisor/src/service/status.rs:277,389,418,468` | `all`, `dead_letters`, `replay` | None (reads `attempts` only). |
| `crates/app_supervisor/src/store.rs` | `Queue::from_connection` on `supervisor.db` | Gets the `ALTER`s at open. |
| `crates/router/src/proxy/outbox_forwarding.rs:96-170` (`drain_one_outbox`) | `claim_due`, `max_attempts`, `fail`, `complete` | None. |
| `crates/router/src/proxy_outbox.rs:277,292,408,427,440` | `pending_count`, `enqueue_if_absent`, `all`, `dead_letters`, `replay`, `record_dead_letter`, `Queue::open_encrypted` | None. `async.db` gets the `ALTER`s at open. |
| `crates/conversation/src/outbox.rs:150-304` (`drain_one`, `drain_item`, `settle_failed`) | `claim_due`, `fail`, `defer`, `complete`, `max_attempts` | None. `defer` is only called on rows from `claim_due`, which are never parked. |
| `crates/conversation/src/store/schema.rs:32` | `Queue::from_connection` on the conversation store's own database | Gets the `ALTER`s at open. |
| `crates/conversation/src/store/message.rs:73` | `Queue::transaction` + `TxQueue::enqueue` | None. |
| `crates/conversation/src/group.rs:368`, `group.rs:587` | `TxQueue::enqueue`; `Queue::transaction` | None. |
| `crates/router/src/proxy/tests/helpers.rs:328`, `crates/conversation/src/store/tests.rs:65,76`, `crates/conversation/src/host_impl/tests.rs:66,76`, `crates/conversation/src/store/tests/search_and_delete.rs:63` | `all` | None. They read the length or old fields only. |
| `crates/substrate/src/runtime/router.rs:534`, `crates/app_host_native/src/factory.rs:489,571`, `crates/router/src/proxy/tests/helpers.rs:268`, `crates/router/src/proxy/tests/saga_walk.rs:513`, the `dual_build_parity` helpers (`crates/app_host_native/tests/dual_build_parity/{helpers,conversation}.rs`, `crates/roym_web/tests/dual_build_parity/helpers.rs`) | build a `QueueConfig` | None. `QueueConfig` does not change. |
| `crates/router/src/route_handler.rs:250-263` | `QueueConfig::total_retry_window_ms`, `DedupConfig::derive` (`crates/async_queue/src/dedup.rs:85-96`) | **No change in C1, but its rule breaks in C5.** The receiver keeps an idempotency key for the sender's retry window plus one visibility timeout. The test `the_ttl_is_at_least_the_senders_own_total_retry_window` (`dedup.rs:779`) checks this. A parked row can make a direct attempt (after `unpark`), or be delivered through the mailbox, days after that window. The receiver may then have forgotten the key and run the call a second time. C1 adds no caller that parks, so nothing breaks yet. §9 item 1 gives the order rule for C5. |

No SQL outside `crates/async_queue` touches the `outbox` table except
`crates/app_supervisor/src/store/tests.rs:40` (`SELECT COUNT(*) FROM outbox`),
which still works.

## 5. Tests that must stay green unchanged

Every test in `crates/async_queue/src/queue/tests.rs` (29 tests), and the
queue-worker tests in `crates/app_supervisor/src/service/tests/queue_worker.rs`,
`crates/router/src/proxy/tests/outbox.rs`,
`crates/conversation/src/outbox/tests.rs`, plus
`crates/substrate/tests/proxy_outbox_e2e.rs` and
`roym_group_offline_e2e.rs`. Only one existing test changes:
`claim_due`'s query-plan test. It must now assert the plan names
`idx_outbox_due` (the old message names `idx_outbox_visible_at`). This test,
and test 17 below, are the real check that the bundled SQLCipher build uses the
partial indexes.

## 6. New tests

`queue/tests.rs` is 576 lines, so put the new tests in a new file
`crates/async_queue/src/queue/tests/parked.rs`, and add `mod parked;` at the
end of `queue/tests.rs` (layout `foo/tests.rs` + `foo/tests/` is allowed).
The child module reaches the parent's private `config()` with
`use super::*;` (or `super::config`). Each test is short; use the in-memory
queue and fixed `now` values, as the existing tests do. If the file grows past
the 800-line cap, split it by topic (for example `parked.rs` and
`parked_schema.rs`).

Test names describe behaviour; no planning ids.

1. `a_parked_item_is_never_returned_by_claim_due` — enqueue, claim, park
   (poll 1 000, deadline 100 000); `claim_due` at `now = 200_000` is empty.
2. `a_parked_item_is_claimed_by_claim_parked_due_after_its_poll_interval` —
   empty at `parked_at + poll - 1`, one item at `parked_at + poll`, with
   `parked == Some(..)` and the context bytes given to `park`.
3. `a_parked_claim_is_hidden_for_the_visibility_timeout` — after
   `claim_parked_due` at `t`, a second call at `t + visibility - 1` is empty;
   at `t + visibility` the item comes back.
4. `parking_uncounts_the_claim_and_keeps_attempts` — claim, `fail`
   (non-terminal) once, claim again (`claim_count == 2`, `attempts == 1`),
   park: `all()` shows `claim_count == 1`, `attempts == 1`.
5. `parking_an_unclaimed_item_keeps_claim_count_at_zero` — enqueue, then park
   with no claim first: `claim_count == 0` (the `MAX(.., 0)` floor), not -1.
6. `parked_claims_never_advance_claim_count_or_attempts` — with
   `max_attempts = 3`, park and then run 10 cycles of `claim_parked_due` +
   re-park. `claim_count` and `attempts` are unchanged, and the item is never
   dead-lettered. This is the property that a parked item stays parked after
   the attempt budget would have run out.
7. `each_claim_returns_only_its_own_kind` — one due pending row and one due
   parked row: `claim_due(now, 10)` returns only the pending row and
   `claim_parked_due(now, 10)` only the parked row. Then, with two due rows of
   each kind, `claim_due(now, 1)` and `claim_parked_due(now, 1)` each return
   one row, so `LIMIT` applies to each kind separately.
8. `a_parked_item_is_claimable_at_its_deadline_even_when_the_poll_is_later` —
   poll 10 000, deadline 3 000 after now: claimable at the deadline, and
   `parked.is_past_deadline(now)` is true; false one ms before.
9. `a_re_park_keeps_parked_at_and_replaces_the_context` — second `park` with
   new context and deadline: `parked_at` unchanged, context and deadline
   replaced.
10. `unpark_returns_the_item_to_claim_due_with_its_counters` — after unpark at
    `v`, `claim_parked_due` is empty, `claim_due(v)` returns it with
    `parked == None`, the earlier `attempts`, and `claim_count` + 1.
11. `unpark_of_a_pending_item_is_an_error` and
    `park_of_a_missing_item_is_an_error`.
12. `park_refuses_a_zero_poll_interval_and_a_past_deadline`.
13. `a_non_terminal_fail_on_a_parked_item_makes_it_pending` — `fail(..,
    false)` returns `Retrying`, the item has `parked == None`, `attempts + 1`,
    and `claim_due` returns it after the backoff.
14. `a_non_terminal_fail_on_a_parked_item_at_budget_dead_letters_it` — with
    `max_attempts = 2`: claim, `fail` (non-terminal, `attempts == 1`), claim,
    park, claim parked, `fail` (non-terminal): returns `DeadLettered`, the
    outbox is empty, and the dead letter has `attempts == 2`.
15. `a_terminal_fail_on_a_parked_item_dead_letters_it` — dead letter has the
    original payload (not the context).
16. `complete_removes_a_parked_item`.
17. `the_parked_claim_query_uses_its_index` — same shape as the existing plan
    test, for `ClaimKind::Parked`, asserting `idx_outbox_parked_due` and no
    `SCAN`.
18. `defer_has_no_effect_on_a_parked_item` — `claim_count`, `visible_at`
    (seen through `claim_parked_due` timing) and park fields unchanged.
19. `a_parked_item_still_holds_its_key` — `has_pending` is true,
    `enqueue_if_absent` returns `false`, `pending_count` counts it, and
    `replay` of a dead letter with the same key is refused.
20. `a_parked_item_survives_reopening_the_database` — file-backed queue
    (`Queue::open` on a temp dir): park, drop, reopen, `all()` shows the same
    `Parked`.
21. `park_inside_a_transaction_commits_with_the_owners_write` — use
    `Queue::transaction` with an owner table in the same connection (the
    existing transaction tests in `queue/tests.rs` show how): write an owner
    row and `TxQueue::park` in one closure; both are visible after commit.
    Then the same with the closure returning `Err` after `TxQueue::park`: the
    owner row is absent and the item is **not** parked (`claim_due` still
    returns it once it is due). Add the same pair for `TxQueue::unpark`.
22. `an_outbox_created_without_park_columns_gains_them_on_open` — open a raw
    `rusqlite::Connection` on a temp file, create the **old** `outbox` table
    (the eight columns of today) and `idx_outbox_visible_at`, insert one row,
    close; then `Queue::open` the file: the old row is claimable through
    `claim_due` with `parked == None`, it can be parked, and
    `idx_outbox_visible_at` no longer exists (`SELECT name FROM sqlite_master
    WHERE type = 'index'`). Open a second time to prove the `ALTER` is
    idempotent.

## 7. Work order

1. Create the branch (§0).
2. `types.rs` and both re-exports (§3.1, §3.2). `cargo build -p
   syneroym-async-queue` fails until step 3 fills the new field; that is
   expected.
3. `queue.rs` schema, row reader, claim refactor, existing-method changes
   (§3.3–§3.5). Run `cargo nextest run -p syneroym-async-queue`: all 29 old
   tests must pass (fix the plan test's index name).
4. `queue/parked.rs` (§3.6).
5. New tests (§6). Run `cargo nextest run -p syneroym-async-queue`.
6. Run the users' tests: `cargo nextest run -p syneroym-app-supervisor -p
   syneroym-router -p syneroym-conversation`.
7. `cargo clippy -q -p syneroym-async-queue --all-targets --all-features`,
   `cargo +nightly-2026-04-06 fmt --all`, `cargo xtask check-file-lengths`,
   `cargo xtask check-module-layout`, and the duplication check on the changed
   files (recipe in AGENTS.md "Context Budget"). `add_column_if_missing` has the
   same shape as the two existing `ALTER` sites; if cargo-dupes flags it, that
   is acceptable, but do not move those other sites in this slice.
8. Close-out (§8), then the full `mise run verify` in the background, with no
   `--skip`.

## 8. Docs close-out

This plan file is deleted after the slice (the project keeps only the change
doc). So every decision and open point below must be copied into a document
that stays. Nothing in §2 or §9 may live only in this file.

- **Living docs.** No living doc describes the queue's internal states. The
  user-visible states (`pending`, `delivered`, `failed`) do not change until C5.
  So C1 changes no living doc. Check again at the end with
  `grep -n -iE 'claim_count|visible_at|attempt budget' docs/system-architecture.md docs/developer-guide.md docs/system-requirements-spec.md`.
- **Change doc** `docs/planning/changes/service-mailbox/change.md`:
  - Tick the task "`async_queue`: a `deposited` state outside the attempt
    budget. Keep the supervisor behaviour."
  - Under `## Deviations`, add these, in the `D-<slice>-n` form the other
    M06D change docs use (see `D-A2-1` in
    `docs/planning/changes/registry-listed-coordinators/change.md`):
    - `D-C1-1`: the crate calls the state "parked"; the outboxes show it as
      `deposited`.
    - `D-C1-2`: the queue does not act at the parked deadline. It schedules the
      next poll no later than the deadline; the caller chooses `unpark` or
      `fail` (S-5 has two outcomes).
    - `D-C1-3`: the deposit receipt is kept in the queue row as an opaque
      context blob, not in a separate table.
    - `D-C1-4`: `unpark` keeps `attempts`, and there is no reset. This is what
      makes a proxy-outbox row that keeps getting `expired` receipts end (that
      outbox has no age limit).
    - `D-C1-5`: `TxQueue::park` and `TxQueue::unpark` exist. C5 must set the
      Conversation `deposited` state and park the row in one
      `Queue::transaction`, because the Conversation worker deletes any row
      whose message is not `Pending` (`crates/conversation/src/outbox.rs:190`).
      Inside that closure, C5 must not call `ConversationStore::set_state`,
      `set_recipient_state` or `restart_pending`
      (`crates/conversation/src/store/message.rs:612-665`): each one locks the
      connection mutex that `Queue::transaction` already holds, and a
      `std::sync::Mutex` locked twice by one thread hangs it forever. C5 needs
      versions of them that write through the `&Transaction`, as
      `store/message.rs:73` already does with `tx.execute`. This holds for the
      `expired` → `unpark` path too.
    - `D-C1-6`: the "Attempt budget" paragraph is partly stale. The
      Conversation outbox already calls `Queue::defer` for an unreachable peer
      (`crates/conversation/src/outbox.rs:227`), which charges no attempt and
      un-counts the claim. So for Conversation only the `Retry` path is bound
      by the attempt budget; an unreachable peer is bound by
      `conversation_max_pending_age_secs` (30 days). The parked state is still
      needed for S-3, its own poll interval and deadline, and S-6.
  - Under `## Open questions`, add (each marked "Resolve before the sender
    task"):
    - Parked rows count toward `max_pending_rows` (10 000). With a 7-day
      mailbox time-to-live, a sender with many sleeping receivers can fill its
      proxy outbox with parked rows and refuse new calls. Keep this bound, or
      give parked rows their own limit?
  - Do **not** set `status: done`; C2 to C8 are still open.
- **`task.md`** (this folder), the order rule from §9 item 1:
  - §3, slice C5: add "Needs R-7 (C6) in `main` first, or carries R-7 itself.
    Without it, an unparked row, or a mailbox delivery of a message that
    already landed by a direct attempt, can run twice on the receiver."
  - §4 table, row C5: add `R-7 of C6` to "Needs". In the text under the table,
    change "Parallel with C6" (in §3 C5) so it says the R-7 part of C6 must
    merge first.
- **`status.md`** (this folder): set C1 to Done, with implementer, reviewer and
  evidence (the new test file and its pass count, `mise run verify` pass).
  Split the "A4, B1 to B3, C1 to C8" row. A4 splits the same row, so if A4
  merges first, rebase and resolve this small conflict by hand.
- **Deferred backlog** `docs/planning/deferred-backlog.md`: add nothing unless
  the implementation shortcuts something. Optional, if you touch the rows
  anyway: two existing rows (§ around lines 310 and 323) cite
  `crates/async_queue/src/queue/mod.rs`, which no longer exists (it is
  `queue.rs`). Fix the path only; do not change the rows otherwise.
- **This plan file**: delete it at the end, after the copies above are done.

## 9. Ambiguous or stale points found while planning

1. **Parking breaks the rule that the receiver's dedup memory is built on.**
   The receiver keeps an idempotency key for the sender's retry window plus one
   visibility timeout (`crates/router/src/route_handler.rs:250-263`,
   `DedupConfig::derive` in `crates/async_queue/src/dedup.rs:85-96`). A parked
   row can reach the receiver days after that window, in two ways: a direct
   attempt after `unpark` (an `expired` receipt, up to 7 days later), or a
   mailbox delivery of a message whose earlier direct attempt timed out but
   had already run. Either can run the call twice. R-7 (in C6) is the fix: a
   longer dedup memory for services with a mailbox. task.md lets C5 and C6 run
   in parallel, so C5 could merge first. **Order rule: C5 must not merge
   before R-7 is in `main`, or C5 must carry R-7 itself.** C1 is safe on its
   own, because nothing parks yet. Recorded in task.md at close-out (§8).
2. **The branch does not exist.** `feat/m06d-c1-implementation` is not local
   and not on `origin`. §0 creates it from `main`.
3. **Who acts at the parked deadline** is not stated in task.md or S-9.
   Decided in C1-3 (the caller). Recorded as `D-C1-2`.
4. **Where the deposit receipt lives while parked** is not stated. Decided in
   C1-4 (opaque `park_context`). The alternative, a C5 table keyed by outbox
   id, needs its own cleanup on every exit path. Recorded as `D-C1-3`.
5. **"The claim-count limit of its queue" (S-9)** is not in the crate. Each
   worker checks `claim_count > max_attempts` itself
   (`queue_worker.rs:34`, `outbox_forwarding.rs:117`, `conversation/src/outbox.rs:173`).
   C1 meets S-9 by never advancing `claim_count` for a parked claim (C1-5).
6. **The "Attempt budget" paragraph of the change doc is partly stale** for
   Conversation. Recorded as `D-C1-6`. The line reference
   `crates/conversation/src/outbox.rs:173` is still correct.
7. **"Until the outbox age limit" (S-5) has no meaning for the proxy outbox.**
   It has no age limit, only the attempt budget (54 attempts, 900 s backoff
   ceiling, `crates/core/src/config/sandbox.rs:282-287`). Decided in C1-6:
   `unpark` keeps `attempts`, with no reset, so the proxy row still ends.
   Recorded as `D-C1-4`.
8. **Parked rows count toward `max_pending_rows` (10 000)** (C1-9). A bound,
   not a bug, but C5 should look at it. Recorded as a change-doc open question.
9. **Schema change on existing files (C1-10).** Decided: keep the idempotent
   `ALTER`, as two other stores in the repo already do.
10. **Two writes in C5 must be one transaction.** Decided in C1-11
    (`TxQueue::park`/`unpark`). Recorded as `D-C1-5`.
11. **The task.md serial order** lists C1 third (after A1), but A2 and A3 were
    done first. This changes nothing for C1: it has no dependency.

## 10. Commit

One PR. Suggested commits (Conventional Commits, 50/72 rule, short bodies):

- `feat(async-queue): add parked state outside budget`
- `test(async-queue): cover the parked state`
- `docs(planning): close out slice c1`
