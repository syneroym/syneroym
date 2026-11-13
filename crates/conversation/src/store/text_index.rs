//! Full-text search index over messages using trigram and word tables.

use std::str;

use rusqlite::{Connection, Result, params};
use syneroym_text_lang::{
    SEARCH_KEY_VERSION,
    fts::{self, TRI_TABLE_OPTIONS, WORD_TABLE_OPTIONS},
};

use super::message::is_searchable_content_type;

/// Replace the index rows of `rowid` with the keys of `body`.
pub(crate) fn sync(conn: &Connection, rowid: i64, body: &str) -> Result<()> {
    remove(conn, rowid)?;
    let ik = fts::index_keys(body);
    conn.execute(
        "INSERT INTO messages_tri (rowid, body) VALUES (?1, ?2)",
        params![rowid, ik.joined],
    )?;
    conn.execute(
        "INSERT INTO messages_word (rowid, body) VALUES (?1, ?2)",
        params![rowid, ik.words],
    )?;
    Ok(())
}

/// Remove the index rows of `rowid` (no error when there are none).
pub(crate) fn remove(conn: &Connection, rowid: i64) -> Result<()> {
    conn.execute("DELETE FROM messages_tri WHERE rowid = ?1", params![rowid])?;
    conn.execute("DELETE FROM messages_word WHERE rowid = ?1", params![rowid])?;
    Ok(())
}

/// Create the tables; drop `messages_fts`; rebuild when the stored key
/// version differs from `SEARCH_KEY_VERSION`.
pub(crate) fn ensure(conn: &Connection) -> Result<()> {
    let ddl = format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS messages_tri USING fts5(body, {TRI_TABLE_OPTIONS});
         CREATE VIRTUAL TABLE IF NOT EXISTS messages_word USING fts5(body, {WORD_TABLE_OPTIONS});
         CREATE TABLE IF NOT EXISTS text_index_version (
             id INTEGER PRIMARY KEY CHECK (id = 1),
             key_version INTEGER NOT NULL
         );"
    );
    conn.execute_batch(&ddl)?;

    let have: Option<u32> = conn
        .query_row("SELECT key_version FROM text_index_version WHERE id = 1", [], |r| r.get(0))
        .ok();

    let old_fts: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = \
             'messages_fts')",
            [],
            |r| r.get(0),
        )
        .unwrap_or(false);

    if have == Some(SEARCH_KEY_VERSION) && !old_fts {
        return Ok(());
    }

    let tx = conn.unchecked_transaction()?;
    tx.execute_batch("DROP TABLE IF EXISTS messages_fts;")?;
    tx.execute_batch("DELETE FROM messages_tri; DELETE FROM messages_word;")?;

    let to_sync: Vec<(i64, String)> = {
        let mut stmt = tx.prepare(
            "SELECT rowid, content_type, body FROM messages WHERE system = 0 AND deleted_at IS \
             NULL AND admission = 'accepted'",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, Vec<u8>>(2)?))
        })?;
        let mut list = Vec::new();
        for r in rows {
            let (rowid, ct, body_bytes) = r?;
            if is_searchable_content_type(&ct)
                && let Ok(body_str) = str::from_utf8(&body_bytes)
            {
                list.push((rowid, body_str.to_string()));
            }
        }
        list
    };

    for (rowid, body) in to_sync {
        sync(&tx, rowid, &body)?;
    }

    tx.execute(
        "INSERT INTO text_index_version (id, key_version) VALUES (1, ?1) ON CONFLICT(id) DO \
         UPDATE SET key_version = excluded.key_version",
        params![SEARCH_KEY_VERSION],
    )?;

    tx.commit()?;
    Ok(())
}
