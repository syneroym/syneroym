//! Group conversation management: creation, membership changes, epochs,
//! rekeying, entry validation, application, and history queries.

use anyhow::Result;
use ed25519_dalek::{SigningKey, VerifyingKey};
use rand::RngCore;
use rusqlite::Transaction;
use syneroym_rpc::{ConversationDeliveryState, ConversationError, ConversationKind};

use crate::{
    ConversationService, crypto,
    dag::{
        EntryKind, GROUP_KEY_CONTENT_TYPE, GroupKeyPayload, MAX_PARENTS, MembershipPayload,
        WireEntry, canonical_entry_bytes, canonical_entry_prefix, decode_body, encode_body, open,
        seal, sign_entry, verify_entry,
    },
    ids::{derive_entry_id, derive_group_id},
    store::{
        ConversationConfig, ConversationRow, ConversationStore, StoredDagEntry, StoredMessage,
        now_ms,
    },
};

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests;

fn internal(e: impl std::fmt::Display) -> ConversationError {
    ConversationError::Internal(e.to_string())
}

#[must_use]
pub fn hash_members(members: &[String]) -> String {
    let mut sorted = members.to_vec();
    sorted.sort();
    let mut h = blake3::Hasher::new();
    for m in &sorted {
        h.update(m.as_bytes());
        h.update(&[0u8]);
    }
    hex::encode(h.finalize().as_bytes())
}

#[must_use]
pub fn build_membership_entry(
    sk: &SigningKey,
    conversation_id: &str,
    author: &str,
    now_ms: i64,
    epoch: u64,
    parents: Vec<String>,
    payload: MembershipPayload,
) -> WireEntry {
    let mut entry = WireEntry {
        entry_id: String::new(),
        conversation_id: conversation_id.to_string(),
        author: author.to_string(),
        sender_timestamp_ms: now_ms,
        epoch,
        kind: EntryKind::Membership,
        parents,
        ciphertext: None,
        nonce: None,
        payload: Some(payload),
        signature: [0u8; 64],
    };
    let header = canonical_entry_bytes(&entry);
    entry.entry_id = derive_entry_id(&header);
    entry.signature = sign_entry(sk, &header);
    entry
}

#[allow(clippy::too_many_arguments)]
pub fn build_message_entry(
    sk: &SigningKey,
    conversation_id: &str,
    author: &str,
    now_ms: i64,
    epoch: u64,
    parents: Vec<String>,
    epoch_key: &[u8; 32],
    plaintext: &[u8],
) -> Result<WireEntry, String> {
    let prefix = canonical_entry_prefix(
        conversation_id,
        author,
        now_ms,
        epoch,
        EntryKind::Message,
        &parents,
    );
    let (ciphertext, nonce) = seal(epoch_key, &prefix, plaintext)?;
    let mut entry = WireEntry {
        entry_id: String::new(),
        conversation_id: conversation_id.to_string(),
        author: author.to_string(),
        sender_timestamp_ms: now_ms,
        epoch,
        kind: EntryKind::Message,
        parents,
        ciphertext: Some(ciphertext),
        nonce: Some(nonce),
        payload: None,
        signature: [0u8; 64],
    };
    let header = canonical_entry_bytes(&entry);
    entry.entry_id = derive_entry_id(&header);
    entry.signature = sign_entry(sk, &header);
    Ok(entry)
}

pub fn apply_entry(
    tx: &Transaction<'_>,
    _svc: &str,
    conv_id: &str,
    entry: &StoredDagEntry,
    config: &ConversationConfig,
    now: i64,
) -> Result<(bool, Option<StoredMessage>)> {
    match entry.kind {
        EntryKind::Membership => {
            if let Some(payload) = &entry.payload {
                ConversationStore::apply_membership(tx, conv_id, payload)?;
                // Verify member_list_hash matches local calculation of members after
                // application
                let mut stmt = tx.prepare(
                    "SELECT member_address FROM group_members WHERE conversation_id = ?1 AND \
                     joined_epoch <= ?2 AND (removed_epoch IS NULL OR removed_epoch > ?2) ORDER \
                     BY member_address ASC",
                )?;
                let mut rows = stmt.query(rusqlite::params![conv_id, payload.new_epoch as i64])?;
                let mut current_members = Vec::new();
                while let Some(r) = rows.next()? {
                    current_members.push(r.get::<_, String>(0)?);
                }
                let calculated_hash = hash_members(&current_members);
                if calculated_hash != payload.member_list_hash {
                    return Err(anyhow::anyhow!(
                        "member_list_hash mismatch: expected {}, calculated {}",
                        payload.member_list_hash,
                        calculated_hash
                    ));
                }

                tx.execute(
                    "UPDATE conversations SET current_epoch = MAX(current_epoch, ?1), \
                     last_activity = ?2 WHERE id = ?3",
                    rusqlite::params![payload.new_epoch as i64, now, conv_id],
                )?;
                ConversationStore::mark_dag_applied(tx, &entry.entry_id)?;
            }
            Ok((false, None))
        }
        EntryKind::Message => {
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
                 received_at, content_type, body, signature, outgoing, verified, state, \
                 last_error, system, entry_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, 1, \
                 'delivered', NULL, 0, ?1)",
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
    }
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

impl ConversationService {
    pub(crate) async fn create_group_impl(
        &self,
        service_id: &str,
    ) -> Result<String, ConversationError> {
        let store = self.store_for(service_id).await.map_err(internal)?;
        let ident =
            store.local_identity_or_generate(crypto::generate_identity_bytes).map_err(internal)?;
        let sig_bytes: [u8; 32] =
            ident.sig_secret.as_slice().try_into().map_err(|_| {
                ConversationError::Internal("corrupt local signing key".to_string())
            })?;
        let sk = SigningKey::from_bytes(&sig_bytes);
        let my_vk = sk.verifying_key().to_bytes();

        let now = now_ms();
        let mut nonce = [0u8; 16];
        rand::rng().fill_bytes(&mut nonce);
        let group_id = derive_group_id(service_id, now, &nonce);

        let initial_epoch = 1u64;
        let mut initial_key = [0u8; 32];
        rand::rng().fill_bytes(&mut initial_key);

        let initial_members = vec![service_id.to_string()];
        let payload = MembershipPayload {
            action: "add".to_string(),
            subject_address: service_id.to_string(),
            subject_sig_key: my_vk,
            new_epoch: initial_epoch,
            member_list_hash: hash_members(&initial_members),
        };

        let entry = build_membership_entry(
            &sk,
            &group_id,
            service_id,
            now,
            initial_epoch,
            vec![],
            payload.clone(),
        );

        store
            .queue()
            .transaction(|tx, _| {
                ConversationStore::get_or_create_group_shell(
                    tx,
                    &group_id,
                    service_id,
                    initial_epoch,
                    now,
                )?;
                ConversationStore::insert_entry_if_absent(tx, &group_id, &entry, true, false)?;
                ConversationStore::apply_membership(tx, &group_id, &payload)?;
                tx.execute(
                    "INSERT INTO group_epochs (conversation_id, epoch, key, created_at) VALUES \
                     (?1, ?2, ?3, ?4)",
                    rusqlite::params![group_id, initial_epoch as i64, initial_key.as_slice(), now],
                )?;
                Ok(())
            })
            .map_err(internal)?;

        Ok(group_id)
    }

    pub(crate) async fn change_membership_impl(
        &self,
        service_id: &str,
        conversation: &str,
        member_address: &str,
        action: &str,
    ) -> Result<(), ConversationError> {
        let store = self.store_for(service_id).await.map_err(internal)?;
        let conv = store
            .get_conversation(conversation)
            .map_err(internal)?
            .ok_or(ConversationError::NotFound)?;
        if conv.kind != ConversationKind::Group {
            return Err(ConversationError::InvalidArgument("not a group conversation".to_string()));
        }
        if conv.owner_address.as_deref() != Some(service_id) {
            return Err(ConversationError::PermissionDenied);
        }
        if member_address == service_id {
            return Err(ConversationError::InvalidArgument(
                "the owner is always a member".to_string(),
            ));
        }

        let members = store.current_members(conversation).map_err(internal)?;
        let (subject_sig_key, next_members) = if action == "add" {
            if members.contains(&member_address.to_string()) {
                return Ok(());
            }
            if members.len() as u32 >= store.config().conversation_max_group_members {
                return Err(ConversationError::QuotaExceeded);
            }
            let bundle = self.fetch_prekey_bundle(service_id, member_address).await?;
            let mut nm = members.clone();
            nm.push(member_address.to_string());
            nm.sort();
            (bundle.sig_key, nm)
        } else {
            if !members.contains(&member_address.to_string()) {
                return Ok(());
            }
            let sig_key = store
                .member_sig_key(conversation, member_address)
                .map_err(internal)?
                .ok_or_else(|| ConversationError::Internal("missing member sig key".to_string()))?;
            let nm: Vec<String> = members.into_iter().filter(|m| m != member_address).collect();
            (sig_key, nm)
        };

        let now = now_ms();
        let heads = store.heads(conversation).map_err(internal)?;
        let ident =
            store.local_identity_or_generate(crypto::generate_identity_bytes).map_err(internal)?;
        let sig_bytes: [u8; 32] =
            ident.sig_secret.as_slice().try_into().map_err(|_| {
                ConversationError::Internal("corrupt local signing key".to_string())
            })?;
        let sk = SigningKey::from_bytes(&sig_bytes);

        let mut new_key = [0u8; 32];
        rand::rng().fill_bytes(&mut new_key);

        let member_list_hash = hash_members(&next_members);

        let (_new_epoch, key_msg_bytes) = store
            .queue()
            .transaction(|tx, _| {
                let cur_epoch = ConversationStore::current_epoch_in(tx, conversation)?;
                let next_epoch = cur_epoch + 1;

                let payload = MembershipPayload {
                    action: action.to_string(),
                    subject_address: member_address.to_string(),
                    subject_sig_key,
                    new_epoch: next_epoch,
                    member_list_hash: member_list_hash.clone(),
                };

                let entry = build_membership_entry(
                    &sk,
                    conversation,
                    service_id,
                    now,
                    next_epoch,
                    heads.clone(),
                    payload.clone(),
                );

                ConversationStore::insert_entry_if_absent(tx, conversation, &entry, true, true)?;
                ConversationStore::apply_membership(tx, conversation, &payload)?;
                tx.execute(
                    "INSERT INTO group_epochs (conversation_id, epoch, key, created_at) VALUES \
                     (?1, ?2, ?3, ?4)",
                    rusqlite::params![conversation, next_epoch as i64, new_key.as_slice(), now],
                )?;
                tx.execute(
                    "UPDATE conversations SET current_epoch = ?1, last_activity = ?2 WHERE id = ?3",
                    rusqlite::params![next_epoch as i64, now, conversation],
                )?;

                let key_bytes = serde_json::to_vec(&GroupKeyPayload {
                    group_id: conversation.to_string(),
                    epoch: next_epoch,
                    key: new_key,
                    members: next_members.clone(),
                    owner: service_id.to_string(),
                })
                .map_err(|e| anyhow::anyhow!("serialize error: {e}"))?;

                Ok((next_epoch, key_bytes))
            })
            .map_err(internal)?;

        for m in &next_members {
            if m != service_id {
                let _ = self
                    .enqueue_direct(
                        &store,
                        service_id,
                        m,
                        GROUP_KEY_CONTENT_TYPE,
                        &key_msg_bytes,
                        true,
                    )
                    .await;
            }
        }

        Ok(())
    }

    pub(crate) async fn send_group(
        &self,
        service_id: &str,
        store: &ConversationStore,
        conv: &ConversationRow,
        content_type: &str,
        body: &[u8],
    ) -> Result<String, ConversationError> {
        let members = store.current_members(&conv.id).map_err(internal)?;
        if members.len() <= 1 {
            return Err(ConversationError::InvalidArgument(
                "a group with no other member has nowhere to deliver".to_string(),
            ));
        }
        let epoch = conv.current_epoch;
        let key = store.epoch_key(&conv.id, epoch).map_err(internal)?.ok_or_else(|| {
            ConversationError::Internal("no key for the current epoch".to_string())
        })?;

        let now = now_ms();
        let heads = store.heads(&conv.id).map_err(internal)?;
        let ident =
            store.local_identity_or_generate(crypto::generate_identity_bytes).map_err(internal)?;
        let sig_bytes: [u8; 32] =
            ident.sig_secret.as_slice().try_into().map_err(|_| {
                ConversationError::Internal("corrupt local signing key".to_string())
            })?;
        let sk = SigningKey::from_bytes(&sig_bytes);

        let plaintext = encode_body(content_type, body);
        let entry =
            build_message_entry(&sk, &conv.id, service_id, now, epoch, heads, &key, &plaintext)
                .map_err(internal)?;
        let entry_id = entry.entry_id.clone();

        let max_pending = store.config().max_pending_per_conversation;
        let max_messages = store.config().max_messages_per_conversation;

        store
            .queue()
            .transaction(|tx, txq| {
                let pending_count: u32 = tx.query_row(
                    "SELECT COUNT(*) FROM messages WHERE conversation_id = ?1 AND state = \
                     'pending'",
                    rusqlite::params![conv.id],
                    |r| r.get::<_, i64>(0),
                )? as u32;
                if pending_count >= max_pending {
                    return Err(crate::store::StoreError::PendingQuotaExceeded.into());
                }
                let message_count: u32 = tx.query_row(
                    "SELECT COUNT(*) FROM messages WHERE conversation_id = ?1",
                    rusqlite::params![conv.id],
                    |r| r.get::<_, i64>(0),
                )? as u32;
                if message_count >= max_messages {
                    return Err(crate::store::StoreError::MessageQuotaExceeded.into());
                }

                ConversationStore::insert_entry_if_absent(tx, &conv.id, &entry, true, false)?;
                tx.execute(
                    "INSERT INTO messages (id, conversation_id, author, sender_timestamp, \
                     received_at, content_type, body, signature, outgoing, verified, state, \
                     last_error, system, entry_id) VALUES (?1, ?2, ?3, ?4, ?4, ?5, ?6, ?7, 1, 1, \
                     'pending', NULL, 0, ?1)",
                    rusqlite::params![
                        entry_id,
                        conv.id,
                        service_id,
                        now,
                        content_type,
                        body,
                        entry.signature.as_slice(),
                    ],
                )?;

                for m in &members {
                    if m != service_id {
                        tx.execute(
                            "INSERT INTO message_recipients (message_id, member_address, state, \
                             last_error) VALUES (?1, ?2, 'pending', NULL)",
                            rusqlite::params![entry_id, m],
                        )?;
                        let payload = serde_json::to_vec(&crate::store::OutboxItem {
                            message_id: entry_id.clone(),
                            peer_address: m.clone(),
                            group: Some(conv.id.clone()),
                        })?;
                        txq.enqueue(tx, &conv.id, &format!("{entry_id}:{m}"), &payload, now)?;
                    }
                }
                ConversationStore::touch_conversation(tx, &conv.id, now)?;
                Ok(())
            })
            .map_err(|e| {
                if e.downcast_ref::<crate::store::StoreError>().is_some() {
                    ConversationError::QuotaExceeded
                } else {
                    internal(e)
                }
            })?;

        Ok(entry_id)
    }

    pub(crate) async fn apply_pending_entries(
        &self,
        store: &ConversationStore,
        svc: &str,
        group_id: &str,
    ) {
        let unapplied = match store.unapplied_dag_entries(group_id) {
            Ok(entries) => entries,
            Err(_) => return,
        };
        // One transaction per entry: a membership entry that fails its
        // `member_list_hash` check (or any other entry that turns out to
        // be unappliable) must not roll back every other entry replayed
        // alongside it in the same pass — that entry is retried next time
        // `apply_pending_entries` runs, and the rest are not held hostage
        // to it in the meantime.
        let now = now_ms();
        for entry in &unapplied {
            let result = store
                .queue()
                .transaction(|tx, _| apply_entry(tx, svc, group_id, entry, store.config(), now));
            match result {
                Ok((_, Some(msg))) => {
                    self.notify_message(svc, msg.into_wire()).await;
                }
                Ok((_, None)) => {}
                Err(e) => {
                    tracing::warn!(
                        svc,
                        group_id,
                        entry_id = entry.entry_id,
                        error = ?e,
                        "apply_pending_entries: entry did not apply"
                    );
                }
            }
        }
    }

    pub(crate) async fn scheduled_rekey_once(&self) {
        let now = now_ms();
        for svc in self.candidate_service_ids() {
            let Ok(store) = self.store_for(&svc).await else {
                continue;
            };
            let Ok(convs) = store.group_conversations() else {
                continue;
            };
            for conv in convs {
                if conv.owner_address.as_deref() != Some(&svc) {
                    continue;
                }
                let Ok(Some((_epoch, created_at))) = store.current_epoch_row(&conv.id) else {
                    continue;
                };
                let rekey_interval_ms =
                    (store.config().conversation_group_rekey_secs as i64) * 1000;
                if now - created_at < rekey_interval_ms {
                    continue;
                }
                let Ok(members) = store.current_members(&conv.id) else {
                    continue;
                };
                let mut new_key = [0u8; 32];
                rand::rng().fill_bytes(&mut new_key);

                let res = store.queue().transaction(|tx, _| {
                    let cur_epoch = ConversationStore::current_epoch_in(tx, &conv.id)?;
                    let next_epoch = cur_epoch + 1;
                    tx.execute(
                        "INSERT INTO group_epochs (conversation_id, epoch, key, created_at) \
                         VALUES (?1, ?2, ?3, ?4)",
                        rusqlite::params![conv.id, next_epoch as i64, new_key.as_slice(), now],
                    )?;
                    tx.execute(
                        "UPDATE conversations SET current_epoch = ?1, last_activity = ?2 WHERE id \
                         = ?3",
                        rusqlite::params![next_epoch as i64, now, conv.id],
                    )?;

                    let key_msg_bytes = serde_json::to_vec(&GroupKeyPayload {
                        group_id: conv.id.clone(),
                        epoch: next_epoch,
                        key: new_key,
                        members: members.clone(),
                        owner: svc.clone(),
                    })
                    .map_err(|e| anyhow::anyhow!("serialize error: {e}"))?;

                    Ok(key_msg_bytes)
                });
                if let Ok(key_msg_bytes) = res {
                    for m in &members {
                        if m != &svc {
                            let _ = self
                                .enqueue_direct(
                                    &store,
                                    &svc,
                                    m,
                                    GROUP_KEY_CONTENT_TYPE,
                                    &key_msg_bytes,
                                    true,
                                )
                                .await;
                        }
                    }
                }
            }
        }
    }
}
