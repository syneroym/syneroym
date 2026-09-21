//! Validation and application of incoming DAG entries: signature/timestamp
//! checks on a wire entry before it is trusted, and folding an already-
//! trusted entry (membership or message) into local storage.

use anyhow::Result;
use ed25519_dalek::VerifyingKey;
use rusqlite::Transaction;
use syneroym_rpc::{ConversationDeliveryState, ConversationError};

use super::internal;
use crate::{
    dag::{
        EntryKind, MAX_PARENTS, WireEntry, canonical_entry_bytes, canonical_entry_prefix,
        decode_body, open, verify_entry,
    },
    ids::derive_entry_id,
    store::{
        ConversationConfig, ConversationRow, ConversationStore, StoredDagEntry, StoredMessage,
        now_ms,
    },
};

pub fn apply_entry(
    tx: &Transaction<'_>,
    _svc: &str,
    conv_id: &str,
    entry: &StoredDagEntry,
    config: &ConversationConfig,
    now: i64,
) -> Result<(bool, Option<StoredMessage>)> {
    match entry.kind {
        EntryKind::Membership => apply_membership_entry(tx, conv_id, entry, now),
        EntryKind::Message => apply_message_entry(tx, conv_id, entry, config, now),
    }
}

fn apply_membership_entry(
    tx: &Transaction<'_>,
    conv_id: &str,
    entry: &StoredDagEntry,
    now: i64,
) -> Result<(bool, Option<StoredMessage>)> {
    if let Some(payload) = &entry.payload {
        ConversationStore::apply_membership(tx, conv_id, payload)?;
        // Verify member_list_hash matches local calculation of members after
        // application
        let mut stmt = tx.prepare(
            "SELECT member_address FROM group_members WHERE conversation_id = ?1 AND joined_epoch \
             <= ?2 AND (removed_epoch IS NULL OR removed_epoch > ?2) ORDER BY member_address ASC",
        )?;
        let mut rows = stmt.query(rusqlite::params![conv_id, payload.new_epoch as i64])?;
        let mut current_members = Vec::new();
        while let Some(r) = rows.next()? {
            current_members.push(r.get::<_, String>(0)?);
        }
        let calculated_hash = super::hash_members(&current_members);
        if calculated_hash != payload.member_list_hash {
            return Err(anyhow::anyhow!(
                "member_list_hash mismatch: expected {}, calculated {}",
                payload.member_list_hash,
                calculated_hash
            ));
        }

        tx.execute(
            "UPDATE conversations SET current_epoch = MAX(current_epoch, ?1), last_activity = ?2 \
             WHERE id = ?3",
            rusqlite::params![payload.new_epoch as i64, now, conv_id],
        )?;
        ConversationStore::mark_dag_applied(tx, &entry.entry_id)?;
    }
    Ok((false, None))
}

fn apply_message_entry(
    tx: &Transaction<'_>,
    conv_id: &str,
    entry: &StoredDagEntry,
    config: &ConversationConfig,
    now: i64,
) -> Result<(bool, Option<StoredMessage>)> {
    let msg_count: i64 = tx.query_row(
        "SELECT COUNT(*) FROM messages WHERE conversation_id = ?1",
        rusqlite::params![conv_id],
        |r| r.get(0),
    )?;
    if msg_count as u32 >= config.max_messages_per_conversation {
        return Ok((false, None));
    }

    let key = ConversationStore::epoch_key_in(tx, conv_id, entry.epoch)?;
    let Some(key) = key else {
        return Ok((false, None));
    };
    let prefix = canonical_entry_prefix(
        conv_id,
        &entry.author,
        entry.sender_timestamp_ms,
        entry.epoch,
        entry.kind,
        &entry.parents,
    );
    let ct = entry.ciphertext.as_deref().unwrap_or(&[]);
    let Some(nonce) = &entry.nonce else {
        return Ok((false, None));
    };
    let plaintext = match open(&key, &prefix, nonce, ct) {
        Ok(p) => p,
        Err(_) => return Ok((false, None)),
    };
    let (content_type, body) = match decode_body(&plaintext) {
        Ok(b) => b,
        Err(_) => return Ok((false, None)),
    };
    if body.len() as u32 > config.max_body_bytes {
        return Ok((false, None));
    }
    let inserted = tx.execute(
        "INSERT OR IGNORE INTO messages (id, conversation_id, author, sender_timestamp, \
         received_at, content_type, body, signature, outgoing, verified, state, last_error, \
         system, entry_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, 1, 'delivered', NULL, 0, ?1)",
        rusqlite::params![
            entry.entry_id,
            conv_id,
            entry.author,
            entry.sender_timestamp_ms,
            now,
            content_type,
            body,
            entry.signature.as_slice()
        ],
    )?;
    ConversationStore::mark_dag_applied(tx, &entry.entry_id)?;
    ConversationStore::touch_conversation(tx, conv_id, now)?;
    if inserted > 0 {
        let msg = StoredMessage {
            id: entry.entry_id.clone(),
            conversation_id: conv_id.to_string(),
            author: entry.author.clone(),
            sender_timestamp_ms: entry.sender_timestamp_ms,
            received_at_ms: now,
            content_type,
            body,
            signature: entry.signature,
            outgoing: false,
            verified: true,
            state: ConversationDeliveryState::Delivered,
            last_error: None,
            system: false,
            entry_id: Some(entry.entry_id.clone()),
        };
        Ok((true, Some(msg)))
    } else {
        Ok((false, None))
    }
}

/// Resolves the signing key that should have produced `entry`'s signature.
/// A membership entry must be authored by the conversation owner, and may
/// additionally trust the subject's own claimed key or a pinned session
/// key (the owner does not yet have a `group_members` row for a brand new
/// subject). Any other entry kind trusts only an existing member's key or
/// a pinned session key — never the entry's own claimed key, which an
/// outsider could set to anything.
fn resolve_entry_sig_key(
    store: &ConversationStore,
    conv: &ConversationRow,
    entry: &WireEntry,
) -> Result<[u8; 32], ConversationError> {
    let sig_key = if entry.kind == EntryKind::Membership {
        if conv.owner_address.as_deref() != Some(&entry.author) {
            return Err(ConversationError::PermissionDenied);
        }
        if let Some(key) =
            store.member_sig_key_at(&conv.id, &entry.author, entry.epoch).map_err(internal)?
        {
            if key != [0u8; 32] {
                key
            } else if let Some(p) = &entry.payload
                && p.subject_address == entry.author
            {
                p.subject_sig_key
            } else if let Ok(Some(sess)) = store.session(&entry.author) {
                sess.pinned_sig_key
            } else {
                return Err(ConversationError::PermissionDenied);
            }
        } else if let Some(key) = store.member_sig_key(&conv.id, &entry.author).map_err(internal)? {
            if key != [0u8; 32] {
                key
            } else if let Some(p) = &entry.payload
                && p.subject_address == entry.author
            {
                p.subject_sig_key
            } else if let Ok(Some(sess)) = store.session(&entry.author) {
                sess.pinned_sig_key
            } else {
                return Err(ConversationError::PermissionDenied);
            }
        } else if let Some(p) = &entry.payload
            && p.subject_address == entry.author
        {
            p.subject_sig_key
        } else if let Ok(Some(sess)) = store.session(&entry.author) {
            sess.pinned_sig_key
        } else {
            return Err(ConversationError::PermissionDenied);
        }
    } else if let Some(key) =
        store.member_sig_key_at(&conv.id, &entry.author, entry.epoch).map_err(internal)?
    {
        if key != [0u8; 32] {
            key
        } else if let Ok(Some(sess)) = store.session(&entry.author) {
            sess.pinned_sig_key
        } else {
            return Err(ConversationError::PermissionDenied);
        }
    } else if let Some(key) = store.member_sig_key(&conv.id, &entry.author).map_err(internal)? {
        if key != [0u8; 32] {
            key
        } else if let Ok(Some(sess)) = store.session(&entry.author) {
            sess.pinned_sig_key
        } else {
            return Err(ConversationError::PermissionDenied);
        }
    } else {
        return Err(ConversationError::PermissionDenied);
    };
    Ok(sig_key)
}

pub fn validate_and_insert(
    store: &ConversationStore,
    svc: &str,
    conv: &ConversationRow,
    entry: &WireEntry,
) -> Result<(bool, Option<StoredMessage>), ConversationError> {
    let header = canonical_entry_bytes(entry);
    if derive_entry_id(&header) != entry.entry_id {
        return Err(ConversationError::InvalidArgument("mismatched entry id".to_string()));
    }

    let sig_key = resolve_entry_sig_key(store, conv, entry)?;
    let vk = VerifyingKey::from_bytes(&sig_key).map_err(internal)?;
    if !verify_entry(&vk, &header, &entry.signature) {
        return Err(ConversationError::PermissionDenied);
    }

    let now = now_ms();
    let max_skew_ms = (store.config().max_clock_skew_secs as i64) * 1000;
    if entry.sender_timestamp_ms > now + max_skew_ms {
        return Err(ConversationError::InvalidArgument(
            "sender timestamp implausibly far in the future".to_string(),
        ));
    }

    // Check that entry's epoch is not older than current conversation epoch by more
    // than reasonable window
    if conv.current_epoch > 0 && entry.epoch + 10 < conv.current_epoch {
        return Err(ConversationError::PermissionDenied);
    }

    // A removed member still holds the key for the epoch just before its
    // removal, and `member_sig_key_at` correctly authorises it to author
    // there — it really was a member then. But nothing so far stops it
    // from backdating that entry's `sender_timestamp_ms` to sort anywhere
    // in the history, including after messages sent post-removal. Make
    // removal a hard cutoff in time as well as in epoch: an entry cannot
    // claim a timestamp past the moment the removal took effect.
    if let Some(removed_at) =
        store.removed_epoch_created_at(&conv.id, &entry.author).map_err(internal)?
        && entry.sender_timestamp_ms > removed_at + max_skew_ms
    {
        return Err(ConversationError::PermissionDenied);
    }

    if store.dag_entry_count(&conv.id).map_err(internal)?
        >= store.config().conversation_max_dag_entries_per_conversation
    {
        return Err(ConversationError::QuotaExceeded);
    }
    if entry.parents.len() > MAX_PARENTS {
        return Err(ConversationError::InvalidArgument("too many parents".to_string()));
    }

    let mut newly_stored_msg = None;
    let inserted = store
        .queue()
        .transaction(|tx, _| {
            let ins = ConversationStore::insert_entry_if_absent(tx, &conv.id, entry, false, true)?;
            if !ins {
                return Ok(false);
            }
            let stored_dag = StoredDagEntry {
                seq: 0,
                entry_id: entry.entry_id.clone(),
                conversation_id: conv.id.clone(),
                author: entry.author.clone(),
                sender_timestamp_ms: entry.sender_timestamp_ms,
                epoch: entry.epoch,
                kind: entry.kind,
                header: header.clone(),
                ciphertext: entry.ciphertext.clone(),
                nonce: entry.nonce,
                payload: entry.payload.clone(),
                signature: entry.signature,
                applied: false,
                relay_pending: true,
                parents: entry.parents.clone(),
            };
            let (_, msg_opt) = apply_entry(tx, svc, &conv.id, &stored_dag, store.config(), now)?;
            newly_stored_msg = msg_opt;
            Ok(true)
        })
        .map_err(internal)?;

    Ok((inserted, newly_stored_msg))
}
