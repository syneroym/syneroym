//! Group conversation operations: create, rename, membership, info, sync, and
//! visibility.

use serde_json::{Value, json};
use syneroym_app_host::{
    AppConversation, AppDataLayer, AppHost,
    types::{
        conversation::{ConversationError, ConversationKind, GroupInfo, Message},
        data_layer::QueryOptions,
    },
};
use syneroym_roym_core::{
    clock,
    conversation::{
        BodyEncoding, ConversationRow, ConversationRowKind, DELETION_REQUEST_CONTENT_TYPE,
        Direction, MessageRow, StoredState,
        group::{
            GROUP_ADD_UNREACHABLE_MESSAGE, GROUP_DELIVERY_NOTICE, GROUP_JOIN_BOUNDARY_NOTICE,
            GROUP_KEY_TRUST_NOTICE, GROUP_PROFILE_CONTENT_TYPE, GROUP_REMOVED_NOTICE,
            GROUP_RESTORED_NOTICE, GroupAdmission, GroupMeta, MEMBERSHIP_EVENT_CONTENT_TYPE,
            NameSource, OWNER_CAN_READ_NOTICE, admission_for_new_group, apply_group_profile,
            group_profile_body, membership_event_body, parse_group_profile, validate_group_name,
        },
    },
    envelope::{Request, Response},
};

use super::{
    REFUSED_MESSAGES, contacts_map, create_conversation,
    inbox::{honour_deletion_request, incoming_row, is_blocked, record_refused},
    load_conversation, load_message,
    messages::{resolve_open_address, send_and_record},
    person_did_for_address, profile_call, put_conversation, put_message,
};

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Stored {
    Kept,
    Refused(String),
    Ignored,
}

pub(crate) async fn new_group_row<H: AppHost>(
    host: &H,
    id: &str,
    info: &GroupInfo,
    now: u64,
) -> Result<ConversationRow, String> {
    let owner_did = person_did_for_address(host, &info.owner).await;
    let answer = if info.is_owner {
        None
    } else {
        let resp = profile_call(
            host,
            "contacts.admit-first-contact",
            json!({ "sender_address": info.owner, "sender_person_did": owner_did }),
        )
        .await?;
        resp.result.and_then(|v| v.get("admission").and_then(Value::as_str).map(str::to_string))
    };
    Ok(ConversationRow {
        id: id.to_string(),
        kind: ConversationRowKind::Group,
        peer_address: info.owner.clone(),
        peer_person_did: owner_did,
        opened_at_secs: now,
        last_activity_ms: 0,
        message_count: 0,
        group: Some(GroupMeta {
            name: None,
            name_source: None,
            admission: admission_for_new_group(info.is_owner, answer.as_deref()),
            membership_events_copied: 0,
        }),
    })
}

pub(crate) async fn sync_membership_rows<H: AppHost>(
    host: &H,
    row: &mut ConversationRow,
    info: &GroupInfo,
) -> Result<u32, String> {
    let events = AppConversation::membership_history(host, row.id.clone())
        .await
        .map_err(|e| format!("{e:?}"))?;
    let seen = row.group.as_ref().ok_or("not a group row")?.membership_events_copied;
    if events.len() as u32 == seen {
        return Ok(0);
    }
    let direction = if info.is_owner { Direction::Outgoing } else { Direction::Incoming };
    let mut copied = 0;
    for ev in &events {
        if load_message(host, &ev.entry).await?.is_some() {
            continue;
        }
        put_message(
            host,
            &MessageRow {
                id: ev.entry.clone(),
                conversation: row.id.clone(),
                author: info.owner.clone(),
                direction,
                sender_timestamp_ms: ev.sender_timestamp,
                content_type: MEMBERSHIP_EVENT_CONTENT_TYPE.to_string(),
                body_encoding: BodyEncoding::Utf8,
                body: Some(membership_event_body(&ev.action, &ev.subject, ev.epoch)),
                state: StoredState::Delivered,
                last_error: None,
                deleted_at_secs: None,
                stored_at_secs: clock::now_secs(),
            },
        )
        .await?;
        row.last_activity_ms = row.last_activity_ms.max(ev.sender_timestamp);
        copied += 1;
    }
    if let Some(meta) = row.group.as_mut() {
        meta.membership_events_copied = events.len() as u32;
    }
    Ok(copied)
}

pub(crate) async fn adopt_new_groups<H: AppHost>(host: &H) {
    let Ok(all) = AppConversation::conversations(host).await else {
        eprintln!("roym conversation: host conversation list unavailable; adoption skipped");
        return;
    };
    for c in all.into_iter().filter(|c| c.kind == ConversationKind::Group) {
        if let Err(e) = adopt_one(host, &c.id).await {
            eprintln!("roym conversation: group {} not adopted yet: {e}", c.id);
        }
    }
}

async fn adopt_one<H: AppHost>(host: &H, id: &str) -> Result<(), String> {
    if load_conversation(host, id).await?.is_some() {
        return Ok(());
    }
    let info =
        AppConversation::group_info(host, id.to_string()).await.map_err(|e| format!("{e:?}"))?;
    let mut row = new_group_row(host, id, &info, clock::now_secs()).await?;
    sync_membership_rows(host, &mut row, &info).await?;
    // Losing the create race to a concurrent inbox or list call is fine;
    // the stored decision wins.
    create_conversation(host, &row).await.map(|_| ())
}

pub(crate) async fn store_group_message<H: AppHost>(
    host: &H,
    row: &mut ConversationRow,
    info: &GroupInfo,
    msg: &Message,
    now: u64,
) -> Result<Stored, String> {
    let person_did = person_did_for_address(host, &msg.author).await;
    if is_blocked(host, &msg.author, person_did.as_deref()).await? {
        record_refused(host, msg, "blocked", now).await?;
        return Ok(Stored::Refused("blocked".to_string()));
    }
    if msg.content_type == DELETION_REQUEST_CONTENT_TYPE {
        honour_deletion_request(host, msg, now).await?;
        return Ok(Stored::Ignored);
    }
    if msg.content_type == MEMBERSHIP_EVENT_CONTENT_TYPE {
        record_refused(host, msg, "reserved-type", now).await?;
        return Ok(Stored::Refused("reserved-type".to_string()));
    }
    if load_message(host, &msg.id).await?.is_some() {
        return Ok(Stored::Ignored);
    }
    if msg.content_type == GROUP_PROFILE_CONTENT_TYPE {
        if msg.author != info.owner {
            record_refused(host, msg, "not-owner", now).await?;
            return Ok(Stored::Refused("not-owner".to_string()));
        }
        let Ok(name) = parse_group_profile(&msg.body) else {
            record_refused(host, msg, "bad-group-profile", now).await?;
            return Ok(Stored::Refused("bad-group-profile".to_string()));
        };
        let meta = row.group.as_mut().ok_or("no meta")?;
        apply_group_profile(
            meta,
            name,
            NameSource {
                sender_timestamp_ms: msg.sender_timestamp,
                author: msg.author.clone(),
                message_id: msg.id.clone(),
            },
        );
    }

    put_message(host, &incoming_row(msg, now)).await?;
    row.message_count += 1;
    row.last_activity_ms = row.last_activity_ms.max(msg.sender_timestamp);
    sync_membership_rows(host, row, info).await?;
    Ok(Stored::Kept)
}

pub(crate) async fn create<H: AppHost>(host: &H, req: &Request) -> Response {
    let validated_name = match req.params.get("name").and_then(Value::as_str) {
        Some(raw) => match validate_group_name(raw) {
            Ok(n) => Some(n),
            Err(e) => return Response::invalid_params(e.to_string()),
        },
        None => None,
    };
    let conversation_id = match AppConversation::create_group(host).await {
        Ok(id) => id,
        Err(e) => return Response::internal_error(format!("{e:?}")),
    };
    let info = match AppConversation::group_info(host, conversation_id.clone()).await {
        Ok(i) => i,
        Err(e) => return Response::internal_error(format!("{e:?}")),
    };
    let now = clock::now_secs();
    let mut row = match new_group_row(host, &conversation_id, &info, now).await {
        Ok(r) => r,
        Err(e) => return Response::internal_error(e),
    };
    if let Some(name) = validated_name
        && let Some(meta) = row.group.as_mut()
    {
        meta.name = Some(name);
        meta.name_source = Some(NameSource {
            sender_timestamp_ms: clock::now_ms(),
            author: info.owner.clone(),
            message_id: "local".to_string(),
        });
    }
    if let Err(e) = sync_membership_rows(host, &mut row, &info).await {
        return Response::internal_error(e);
    }
    if let Err(e) = put_conversation(host, &row).await {
        return Response::internal_error(e);
    }
    Response::ok(json!({
        "conversation_id": conversation_id,
        "owner_address": info.owner,
    }))
}

fn extract_conversation_param(req: &Request) -> Option<String> {
    req.params
        .get("conversation")
        .or_else(|| req.params.get("group"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

pub(crate) async fn rename<H: AppHost>(host: &H, req: &Request) -> Response {
    let conversation = match extract_conversation_param(req) {
        Some(c) => c,
        None => return Response::invalid_params("conversation is required"),
    };
    let raw_name = match req.params.get("name").and_then(Value::as_str) {
        Some(n) => n,
        None => return Response::invalid_params("name is required"),
    };
    let name = match validate_group_name(raw_name) {
        Ok(n) => n,
        Err(e) => return Response::invalid_params(e.to_string()),
    };
    let mut row = match load_conversation(host, &conversation).await {
        Ok(Some(r)) => r,
        Ok(None) => return Response::invalid_params("conversation not found"),
        Err(e) => return Response::internal_error(e),
    };
    if row.kind != ConversationRowKind::Group {
        return Response::invalid_params("not a group conversation");
    }
    let info = match AppConversation::group_info(host, conversation.clone()).await {
        Ok(i) => i,
        Err(e) => return Response::internal_error(format!("{e:?}")),
    };
    if !info.is_owner {
        return Response::invalid_params("only the group's owner can rename it");
    }

    let mut sent = false;
    let mut send_error = None;
    let src = if info.members.len() > 1 {
        match send_and_record(
            host,
            &conversation,
            GROUP_PROFILE_CONTENT_TYPE,
            &group_profile_body(&name),
        )
        .await
        {
            Ok(msg) => {
                sent = true;
                NameSource {
                    sender_timestamp_ms: msg.sender_timestamp_ms,
                    author: msg.author,
                    message_id: msg.id,
                }
            }
            Err(e) => {
                send_error = Some(
                    e.error.map(|err| err.message).unwrap_or_else(|| "send failed".to_string()),
                );
                NameSource {
                    sender_timestamp_ms: clock::now_ms(),
                    author: info.owner.clone(),
                    message_id: "local".to_string(),
                }
            }
        }
    } else {
        NameSource {
            sender_timestamp_ms: clock::now_ms(),
            author: info.owner.clone(),
            message_id: "local".to_string(),
        }
    };

    if sent {
        row = load_conversation(host, &conversation).await.ok().flatten().unwrap_or(row);
    }

    if let Some(meta) = row.group.as_mut() {
        apply_group_profile(meta, name.clone(), src);
    }
    if let Err(e) = put_conversation(host, &row).await {
        return Response::internal_error(e);
    }

    let mut res = json!({ "name": name, "sent": sent });
    if let Some(err) = send_error {
        res["send_error"] = json!(err);
    }
    Response::ok(res)
}

pub(crate) async fn add_member<H: AppHost>(host: &H, req: &Request) -> Response {
    let conversation = match extract_conversation_param(req) {
        Some(c) => c,
        None => return Response::invalid_params("conversation is required"),
    };
    let mut row = match load_conversation(host, &conversation).await {
        Ok(Some(r)) => r,
        Ok(None) => return Response::invalid_params("conversation not found"),
        Err(e) => return Response::internal_error(e),
    };
    if row.kind != ConversationRowKind::Group {
        return Response::invalid_params("not a group conversation");
    }
    let info = match AppConversation::group_info(host, conversation.clone()).await {
        Ok(i) => i,
        Err(e) => return Response::internal_error(format!("{e:?}")),
    };
    if !info.is_owner {
        return Response::invalid_params("only the group's owner can add members");
    }
    let member_address = match resolve_open_address(host, req).await {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    if let Err(e) =
        AppConversation::add_member(host, conversation.clone(), member_address.clone()).await
    {
        return match e {
            ConversationError::Unreachable(_) => {
                Response::invalid_params(GROUP_ADD_UNREACHABLE_MESSAGE)
            }
            ConversationError::QuotaExceeded => Response::invalid_params("this group is full"),
            other => Response::internal_error(format!("{other:?}")),
        };
    }

    let updated_info = match AppConversation::group_info(host, conversation.clone()).await {
        Ok(i) => i,
        Err(e) => return Response::internal_error(format!("{e:?}")),
    };
    let epoch = updated_info.epoch;

    let mut name_sent = false;
    let mut send_error = None;
    if let Some(name) = row.group.as_ref().and_then(|g| g.name.clone()) {
        match send_and_record(
            host,
            &conversation,
            GROUP_PROFILE_CONTENT_TYPE,
            &group_profile_body(&name),
        )
        .await
        {
            Ok(_) => name_sent = true,
            Err(e) => {
                send_error = Some(
                    e.error.map(|err| err.message).unwrap_or_else(|| "send failed".to_string()),
                );
            }
        }
    }

    if name_sent {
        row = load_conversation(host, &conversation).await.ok().flatten().unwrap_or(row);
    }

    if let Err(e) = sync_membership_rows(host, &mut row, &updated_info).await {
        return Response::internal_error(e);
    }
    if let Err(e) = put_conversation(host, &row).await {
        return Response::internal_error(e);
    }

    let mut res = json!({
        "added": member_address,
        "epoch": epoch,
        "name_sent": name_sent,
    });
    if let Some(err) = send_error {
        res["send_error"] = json!(err);
    }
    Response::ok(res)
}

pub(crate) async fn remove_member<H: AppHost>(host: &H, req: &Request) -> Response {
    let conversation = match extract_conversation_param(req) {
        Some(c) => c,
        None => return Response::invalid_params("conversation is required"),
    };
    let mut row = match load_conversation(host, &conversation).await {
        Ok(Some(r)) => r,
        Ok(None) => return Response::invalid_params("conversation not found"),
        Err(e) => return Response::internal_error(e),
    };
    if row.kind != ConversationRowKind::Group {
        return Response::invalid_params("not a group conversation");
    }
    let info = match AppConversation::group_info(host, conversation.clone()).await {
        Ok(i) => i,
        Err(e) => return Response::internal_error(format!("{e:?}")),
    };
    if !info.is_owner {
        return Response::invalid_params("only the group's owner can remove members");
    }
    let member_address = match resolve_open_address(host, req).await {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    if let Err(e) =
        AppConversation::remove_member(host, conversation.clone(), member_address.clone()).await
    {
        return match e {
            ConversationError::InvalidArgument(msg) => Response::invalid_params(msg),
            other => Response::internal_error(format!("{other:?}")),
        };
    }

    let updated_info = match AppConversation::group_info(host, conversation.clone()).await {
        Ok(i) => i,
        Err(e) => return Response::internal_error(format!("{e:?}")),
    };
    let epoch = updated_info.epoch;

    if let Err(e) = sync_membership_rows(host, &mut row, &updated_info).await {
        return Response::internal_error(e);
    }
    if let Err(e) = put_conversation(host, &row).await {
        return Response::internal_error(e);
    }
    Response::ok(json!({ "removed": member_address, "epoch": epoch }))
}

pub(crate) async fn info<H: AppHost>(host: &H, req: &Request) -> Response {
    let conversation = match extract_conversation_param(req) {
        Some(c) => c,
        None => return Response::invalid_params("conversation is required"),
    };
    if load_conversation(host, &conversation).await.ok().flatten().is_none() {
        adopt_new_groups(host).await;
    }
    let mut row = match load_conversation(host, &conversation).await {
        Ok(Some(r)) => r,
        Ok(None) => return Response::invalid_params("conversation not found"),
        Err(e) => return Response::internal_error(e),
    };
    if row.kind != ConversationRowKind::Group {
        return Response::invalid_params("not a group conversation");
    }
    let host_info = AppConversation::group_info(host, conversation.clone()).await;
    match host_info {
        Err(ConversationError::NotFound) => {
            let name = row.group.as_ref().and_then(|g| g.name.clone());
            let admission = row.group.as_ref().map(|g| &g.admission);
            Response::ok(json!({
                "conversation_id": row.id,
                "name": name,
                "owner_address": row.peer_address,
                "owner_person_did": row.peer_person_did,
                "is_owner": false,
                "is_member": false,
                "restored_only": true,
                "admission": admission,
                "epoch": 0,
                "key_epoch": 0,
                "key_stored_at_ms": 0,
                "members": [],
                "can_read_new_messages": false,
                "notices": {
                    "owner_can_read": OWNER_CAN_READ_NOTICE,
                    "key_trust": GROUP_KEY_TRUST_NOTICE,
                    "delivery": GROUP_DELIVERY_NOTICE,
                    "join_boundary": GROUP_JOIN_BOUNDARY_NOTICE,
                    "removed": null,
                    "restored": GROUP_RESTORED_NOTICE,
                }
            }))
        }
        Ok(info) => {
            if let Ok(copied) = sync_membership_rows(host, &mut row, &info).await
                && copied > 0
            {
                let _ = put_conversation(host, &row).await;
            }
            let contacts = contacts_map(host).await;
            let members: Vec<Value> = info
                .members
                .iter()
                .map(|m| {
                    json!({
                        "address": m,
                        "person_did": contacts.get(m),
                        "is_owner": *m == info.owner,
                    })
                })
                .collect();
            let can_read = info.key_epoch >= info.epoch && info.is_member;
            let name = row.group.as_ref().and_then(|g| g.name.clone());
            let admission = row.group.as_ref().map(|g| &g.admission);
            Response::ok(json!({
                "conversation_id": row.id,
                "name": name,
                "owner_address": row.peer_address,
                "owner_person_did": row.peer_person_did,
                "is_owner": info.is_owner,
                "is_member": info.is_member,
                "restored_only": false,
                "admission": admission,
                "epoch": info.epoch,
                "key_epoch": info.key_epoch,
                "key_stored_at_ms": info.key_stored_at,
                "members": members,
                "can_read_new_messages": can_read,
                "notices": {
                    "owner_can_read": OWNER_CAN_READ_NOTICE,
                    "key_trust": GROUP_KEY_TRUST_NOTICE,
                    "delivery": GROUP_DELIVERY_NOTICE,
                    "join_boundary": GROUP_JOIN_BOUNDARY_NOTICE,
                    "removed": if !info.is_member { Some(GROUP_REMOVED_NOTICE) } else { None },
                    "restored": null,
                }
            }))
        }
        Err(e) => Response::internal_error(format!("{e:?}")),
    }
}

pub(crate) async fn sync<H: AppHost>(host: &H, req: &Request) -> Response {
    let conversation = match extract_conversation_param(req) {
        Some(c) => c,
        None => return Response::invalid_params("conversation is required"),
    };
    let mut row = match load_conversation(host, &conversation).await {
        Ok(Some(r)) => r,
        Ok(None) => return Response::invalid_params("conversation not found"),
        Err(e) => return Response::internal_error(e),
    };
    if row.kind != ConversationRowKind::Group {
        return Response::invalid_params("not a group conversation");
    }
    if let Err(e) = AppConversation::sync_now(host, conversation.clone()).await {
        return Response::internal_error(format!("{e:?}"));
    }
    let info = match AppConversation::group_info(host, conversation.clone()).await {
        Ok(i) => i,
        Err(e) => return Response::internal_error(format!("{e:?}")),
    };
    let events_copied = match sync_membership_rows(host, &mut row, &info).await {
        Ok(n) => n,
        Err(e) => return Response::internal_error(e),
    };
    if let Err(e) = put_conversation(host, &row).await {
        return Response::internal_error(e);
    }
    Response::ok(json!({ "synced": true, "events_copied": events_copied }))
}

pub(crate) async fn hide<H: AppHost>(host: &H, req: &Request) -> Response {
    let conversation = match extract_conversation_param(req) {
        Some(c) => c,
        None => return Response::invalid_params("conversation is required"),
    };
    let mut row = match load_conversation(host, &conversation).await {
        Ok(Some(r)) => r,
        Ok(None) => return Response::invalid_params("conversation not found"),
        Err(e) => return Response::internal_error(e),
    };
    if row.kind != ConversationRowKind::Group {
        return Response::invalid_params("not a group conversation");
    }
    if let Some(meta) = row.group.as_mut() {
        meta.admission = GroupAdmission::Hidden;
    }
    if let Err(e) = put_conversation(host, &row).await {
        return Response::internal_error(e);
    }
    Response::ok(json!({ "admission": { "state": "hidden" } }))
}

pub(crate) async fn unhide<H: AppHost>(host: &H, req: &Request) -> Response {
    let conversation = match extract_conversation_param(req) {
        Some(c) => c,
        None => return Response::invalid_params("conversation is required"),
    };
    let mut row = match load_conversation(host, &conversation).await {
        Ok(Some(r)) => r,
        Ok(None) => return Response::invalid_params("conversation not found"),
        Err(e) => return Response::internal_error(e),
    };
    if row.kind != ConversationRowKind::Group {
        return Response::invalid_params("not a group conversation");
    }
    let info = match AppConversation::group_info(host, conversation.clone()).await {
        Ok(i) => i,
        Err(e) => return Response::internal_error(format!("{e:?}")),
    };

    if let Some(meta) = row.group.as_mut() {
        meta.admission = GroupAdmission::Shown;
    }
    if let Err(e) = put_conversation(host, &row).await {
        return Response::internal_error(e);
    }

    let filter = json!({
        "conversation": conversation,
        "reason": { "$in": ["group-hidden", "rate-limited"] }
    })
    .to_string();

    let mut filled_in = 0u32;
    let now = clock::now_secs();
    for _pass in 0..2 {
        let mut cursor = None;
        let mut pass_filled = 0;
        loop {
            let page = match AppDataLayer::query(
                host,
                REFUSED_MESSAGES.to_string(),
                QueryOptions {
                    filter: Some(filter.clone()),
                    limit: Some(500),
                    cursor: cursor.clone(),
                },
            )
            .await
            {
                Ok(p) => p,
                Err(e) => return Response::internal_error(e.to_string()),
            };
            for rec in page.records {
                let msg_id = rec.id;
                if let Ok(msg) = AppConversation::get_message(host, msg_id.clone()).await
                    && let Ok(Stored::Kept) =
                        store_group_message(host, &mut row, &info, &msg, now).await
                {
                    let _ = AppDataLayer::delete(host, REFUSED_MESSAGES.to_string(), msg_id).await;
                    filled_in += 1;
                    pass_filled += 1;
                }
            }
            if page.next_cursor.is_none() || page.next_cursor == cursor {
                break;
            }
            cursor = page.next_cursor;
        }
        if pass_filled == 0 {
            break;
        }
    }

    if let Ok(Some(mut reloaded)) = load_conversation(host, &conversation).await {
        reloaded.message_count += u64::from(filled_in);
        reloaded.last_activity_ms = reloaded.last_activity_ms.max(row.last_activity_ms);
        if let Some(meta) = reloaded.group.as_mut() {
            meta.admission = GroupAdmission::Shown;
        }
        if let (Some(meta), Some(cur_meta)) = (row.group.as_ref(), reloaded.group.as_mut()) {
            cur_meta.membership_events_copied = meta.membership_events_copied;
            if let (Some(name), Some(src)) = (&meta.name, &meta.name_source) {
                apply_group_profile(cur_meta, name.clone(), src.clone());
            }
        }
        let _ = put_conversation(host, &reloaded).await;
    }

    Response::ok(json!({
        "admission": { "state": "shown" },
        "filled_in": filled_in,
    }))
}
