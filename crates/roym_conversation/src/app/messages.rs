//! Conversation messaging operations: open, send, history, changes, search, and
//! deletion.

use std::{cmp::Reverse, collections::HashMap};

use serde::{Deserialize, Serialize};
use syneroym_app_host::{
    AppConversation, AppHost,
    types::conversation::{ConversationError, ConversationKind, HistoryItem},
};
use syneroym_roym_core::{
    conversation::{
        BodyEncoding, DELETION_REQUEST_CONTENT_TYPE, Direction, StoredState,
        group::{
            CARDS_NOT_IN_GROUPS_MESSAGE, GROUP_PROFILE_CONTENT_TYPE, GROUP_REMOVED_NOTICE,
            GROUP_RESTORED_NOTICE, MEMBERSHIP_EVENT_CONTENT_TYPE, group_profile_body,
            is_group_system_type, membership_event_body,
        },
    },
    envelope::Response,
    method::{
        NoParams, Reply, SiblingError, call_sibling,
        conversation::{
            AdmissionState, ChangesInput, ChangesOutput, ConversationGroupItem,
            ConversationListItem, ConversationRowKind, MessageView, SendInput, SendOutput,
        },
        profile::{ContactKeyInput, ResolveAddressOutput},
    },
    services,
};

use super::{contacts_map, from_host, load_admission_info, set_admission_peer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum KindFilter {
    Direct,
    Group,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenInput {
    pub address: Option<String>,
    pub person_did: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenOutput {
    pub conversation_id: String,
    pub peer_address: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationListInput {
    pub kind: Option<KindFilter>,
    pub include_hidden: Option<bool>,
    pub offset: Option<u64>,
    pub limit: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConversationListOutput {
    pub conversations: Vec<ConversationListItem>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryInput {
    pub conversation: String,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoryOutput {
    pub messages: Vec<MessageView>,
    pub next_cursor: Option<String>,
    pub kind: ConversationRowKind,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageIdInput {
    pub message_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeliveryStatusOutput {
    pub state: StoredState,
}

#[derive(Debug, Clone, Serialize)]
pub struct OutboxItem {
    pub id: String,
    pub conversation: String,
    pub state: StoredState,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OutboxOutput {
    pub outbox: Vec<OutboxItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RetryOutput {
    pub retried: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteMessageInput {
    pub message_id: String,
    pub ask_peer: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeleteMessageOutput {
    pub deleted: String,
    pub asked_peer: bool,
    pub note: &'static str,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchInput {
    pub query: String,
    pub conversation: Option<String>,
    pub limit: Option<u32>,
    pub kind: Option<KindFilter>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchOutput {
    pub matches: Vec<MessageView>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationRef {
    #[serde(alias = "group")]
    pub conversation: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TranscriptDigestOutput {
    pub digest: String,
    pub rows: u64,
}

pub(crate) async fn resolve_open_address<H: AppHost>(
    host: &H,
    address: Option<&str>,
    person_did: Option<&str>,
) -> Result<String, Response> {
    if let Some(addr) = address {
        return Ok(addr.to_string());
    }
    let Some(did) = person_did else {
        return Err(Response::invalid_params("person_did or address is required"));
    };
    let out = call_sibling::<_, ContactKeyInput, ResolveAddressOutput>(
        host,
        services::PROFILE,
        "contacts.resolve-address",
        &ContactKeyInput { person_did: did.to_string() },
    )
    .await
    .map_err(|e| match e {
        SiblingError::Refused(r) => Response::err(r.code, r.message),
        other => Response::internal_error(other.to_string()),
    })?;
    Ok(out.conversation_address)
}

pub(crate) async fn open<H: AppHost>(host: &H, input: OpenInput) -> Reply<OpenOutput> {
    let address =
        resolve_open_address(host, input.address.as_deref(), input.person_did.as_deref()).await?;
    let conversation_id =
        AppConversation::open_direct(host, address.clone()).await.map_err(from_host)?;
    set_admission_peer(host, &conversation_id, "accepted", Some(&address))
        .await
        .map_err(Response::internal_error)?;
    Ok(OpenOutput { conversation_id, peer_address: address })
}

pub(crate) async fn list<H: AppHost>(
    host: &H,
    input: ConversationListInput,
) -> Reply<ConversationListOutput> {
    let offset = input.offset.unwrap_or(0) as usize;
    let limit = input.limit.unwrap_or(100) as usize;
    let kind_filter = input.kind;
    let include_hidden = input.include_hidden.unwrap_or(false);

    let summaries = AppConversation::conversations(host).await.map_err(from_host)?;
    let contacts = contacts_map(host).await;
    let mut admissions: HashMap<String, (Option<String>, Option<String>)> = HashMap::new();
    for s in &summaries {
        if let Ok(info) = load_admission_info(host, &s.id).await {
            admissions.insert(s.id.clone(), info);
        }
    }
    let mut rows = Vec::new();

    for s in summaries {
        let is_group = s.kind == ConversationKind::Group;
        if let Some(KindFilter::Direct) = kind_filter
            && is_group
        {
            continue;
        }
        if let Some(KindFilter::Group) = kind_filter
            && !is_group
        {
            continue;
        }

        let admission_info = admissions.get(&s.id);
        let admission_str = admission_info
            .and_then(|(st, _)| st.as_deref())
            .unwrap_or(if is_group { "shown" } else { "accepted" });

        if is_group && !include_hidden && admission_str == "hidden" {
            continue;
        }

        let peer_addr =
            if is_group { String::new() } else { s.peer_address.clone().unwrap_or_default() };
        let person_did = contacts.get(&peer_addr).cloned();

        let item = ConversationListItem {
            id: s.id,
            kind: if is_group { ConversationRowKind::Group } else { ConversationRowKind::Direct },
            peer_address: peer_addr,
            peer_person_did: person_did,
            opened_at_secs: s.created_at,
            last_activity_ms: s.last_activity_at,
            message_count: u64::from(s.message_count),
            name: s.name.clone(),
            restored: s.restored,
            group: if is_group {
                Some(ConversationGroupItem {
                    name: s.name,
                    admission: AdmissionState { state: admission_str.to_string() },
                })
            } else {
                None
            },
        };
        rows.push((s.last_activity_at, item));
    }

    rows.sort_by_key(|(act, _)| Reverse(*act));
    let out: Vec<ConversationListItem> =
        rows.into_iter().skip(offset).take(limit).map(|(_, r)| r).collect();
    Ok(ConversationListOutput { conversations: out })
}

pub(crate) async fn send<H: AppHost>(host: &H, input: SendInput) -> Reply<SendOutput> {
    let conversation = input.conversation;
    let body = input.body;
    let content_type = input.content_type.as_deref().unwrap_or("text/plain");

    let is_group = match AppConversation::group_info(host, conversation.clone()).await {
        Ok(info) => {
            if info.restored {
                return Err(Response::invalid_params(GROUP_RESTORED_NOTICE));
            }
            if !info.is_member {
                return Err(Response::invalid_params(GROUP_REMOVED_NOTICE));
            }
            true
        }
        Err(ConversationError::InvalidArgument(_) | ConversationError::NotFound) => false,
        Err(e) => return Err(from_host(e)),
    };

    if content_type == MEMBERSHIP_EVENT_CONTENT_TYPE
        || content_type == DELETION_REQUEST_CONTENT_TYPE
        || content_type == GROUP_PROFILE_CONTENT_TYPE
        || (is_group && content_type == "application/vnd.roym.card+json")
    {
        return Err(Response::invalid_params(
            if content_type == "application/vnd.roym.card+json" {
                CARDS_NOT_IN_GROUPS_MESSAGE
            } else {
                "this content type is reserved"
            },
        ));
    }

    let message_id = AppConversation::send(
        host,
        conversation.clone(),
        content_type.to_string(),
        body.into_bytes(),
    )
    .await
    .map_err(from_host)?;

    let host_msg = AppConversation::get_message(host, message_id.clone()).await.ok();
    Ok(SendOutput {
        message_id,
        state: host_msg
            .as_ref()
            .map(|m| StoredState::from(m.state))
            .unwrap_or(StoredState::Pending),
        sender_timestamp_ms: host_msg.map(|m| m.sender_timestamp).unwrap_or(0),
    })
}

fn history_item_to_view(conversation: &str, item: HistoryItem) -> MessageView {
    match item {
        HistoryItem::Message(m) => MessageView::from(&m),
        HistoryItem::Membership(ev) => {
            let body = membership_event_body(&ev.action, &ev.subject, ev.epoch);
            MessageView {
                id: ev.entry,
                conversation: conversation.to_string(),
                author: ev.subject,
                direction: Direction::Incoming,
                sender_timestamp_ms: ev.sender_timestamp,
                content_type: MEMBERSHIP_EVENT_CONTENT_TYPE.to_string(),
                body_encoding: BodyEncoding::Utf8,
                body: Some(body),
                state: StoredState::Delivered,
                last_error: None,
                stored_at_secs: ev.sender_timestamp / 1000,
                refused: None,
                restored: false,
                visible_seq: 0,
                verified: true,
                deleted_at_secs: None,
            }
        }
        HistoryItem::GroupName(ev) => {
            let body = String::from_utf8(group_profile_body(&ev.name))
                .unwrap_or_else(|_| format!("{{\"name\":\"{}\"}}", ev.name));
            MessageView {
                id: ev.entry,
                conversation: conversation.to_string(),
                author: String::new(),
                direction: Direction::Incoming,
                sender_timestamp_ms: ev.sender_timestamp,
                content_type: GROUP_PROFILE_CONTENT_TYPE.to_string(),
                body_encoding: BodyEncoding::Utf8,
                body: Some(body),
                state: StoredState::Delivered,
                last_error: None,
                stored_at_secs: ev.sender_timestamp / 1000,
                refused: None,
                restored: false,
                visible_seq: 0,
                verified: true,
                deleted_at_secs: None,
            }
        }
    }
}

pub(crate) async fn history<H: AppHost>(host: &H, input: HistoryInput) -> Reply<HistoryOutput> {
    let conversation = input.conversation;
    let limit = input.limit.unwrap_or(200);
    let cursor = input.cursor.filter(|s| !s.is_empty());

    let is_group = match AppConversation::group_info(host, conversation.clone()).await {
        Ok(_) => true,
        Err(ConversationError::InvalidArgument(_) | ConversationError::NotFound) => false,
        Err(e) => return Err(from_host(e)),
    };

    let page = AppConversation::history(host, conversation.clone(), limit, cursor)
        .await
        .map_err(from_host)?;

    let items: Vec<MessageView> =
        page.items.into_iter().map(|item| history_item_to_view(&conversation, item)).collect();

    Ok(HistoryOutput {
        messages: items,
        next_cursor: page.next_cursor,
        kind: if is_group { ConversationRowKind::Group } else { ConversationRowKind::Direct },
    })
}

pub(crate) async fn changes<H: AppHost>(host: &H, input: ChangesInput) -> Reply<ChangesOutput> {
    let after_seq = input.after_seq.unwrap_or(0);
    let limit = input.limit.unwrap_or(200);

    let is_group = AppConversation::group_info(host, input.conversation.clone()).await.is_ok();

    let page = AppConversation::changes(host, input.conversation, after_seq, limit)
        .await
        .map_err(from_host)?;

    let msgs: Vec<MessageView> = page.messages.iter().map(MessageView::from).collect();
    Ok(ChangesOutput {
        messages: msgs,
        last_seq: page.last_seq,
        kind: if is_group { ConversationRowKind::Group } else { ConversationRowKind::Direct },
    })
}

pub(crate) async fn delivery_status<H: AppHost>(
    host: &H,
    input: MessageIdInput,
) -> Reply<DeliveryStatusOutput> {
    let s = AppConversation::delivery_status(host, input.message_id).await.map_err(from_host)?;
    Ok(DeliveryStatusOutput { state: StoredState::from(s) })
}

pub(crate) async fn outbox<H: AppHost>(host: &H, _: NoParams) -> Reply<OutboxOutput> {
    let msgs = AppConversation::outbox(host).await.map_err(from_host)?;
    let out = msgs
        .into_iter()
        .map(|m| OutboxItem {
            id: m.id,
            conversation: m.conversation,
            state: StoredState::from(m.state),
            last_error: m.last_error,
        })
        .collect();
    Ok(OutboxOutput { outbox: out })
}

pub(crate) async fn retry<H: AppHost>(host: &H, input: MessageIdInput) -> Reply<RetryOutput> {
    AppConversation::retry(host, input.message_id).await.map_err(from_host)?;
    Ok(RetryOutput { retried: true })
}

const DELETE_NOTE: &str = "The local copy is removed and a deletion record kept. A request to \
                           delete it was sent to the other side; whether their client honours it \
                           is theirs to decide, and this cannot check. This installation's own \
                           message store still holds what it received.";
const DELETE_NOTE_NO_PEER: &str = "The local copy is removed and a deletion record kept. This is \
                                   a message you received; the other side's copy is theirs.";
const DELETE_NOTE_GROUP_ALONE: &str = "The local copy is removed and a deletion record kept. No \
                                       request was sent: nobody else in this group can receive \
                                       one from you now.";
const DELETE_NOTE_GROUP: &str = "The local copy is removed and a deletion record kept. A request \
                                 to delete it was sent to the other members; whether their \
                                 clients honour it is theirs to decide, and this cannot check. \
                                 Every member already holds the key this message was sent under.";

pub(crate) async fn delete_message<H: AppHost>(
    host: &H,
    input: DeleteMessageInput,
) -> Reply<DeleteMessageOutput> {
    let message_id = input.message_id;
    let ask_peer_req = input.ask_peer.unwrap_or(true);

    let msg = match AppConversation::get_message(host, message_id.clone()).await {
        Ok(m) => Some(m),
        Err(ConversationError::NotFound) => None,
        Err(e) => return Err(from_host(e)),
    };

    let (ask_peer, note) = if let Some(m) = &msg {
        let group_info = AppConversation::group_info(host, m.conversation.clone()).await.ok();
        if !m.outgoing {
            (false, DELETE_NOTE_NO_PEER)
        } else if let Some(g) = group_info {
            if !g.is_member || g.restored || g.members.len() <= 1 {
                (false, DELETE_NOTE_GROUP_ALONE)
            } else {
                (ask_peer_req, DELETE_NOTE_GROUP)
            }
        } else {
            (ask_peer_req, DELETE_NOTE)
        }
    } else {
        (ask_peer_req, DELETE_NOTE)
    };

    AppConversation::delete_message(host, message_id.clone(), ask_peer).await.map_err(from_host)?;
    Ok(DeleteMessageOutput { deleted: message_id, asked_peer: ask_peer, note })
}

pub(crate) async fn search<H: AppHost>(host: &H, input: SearchInput) -> Reply<SearchOutput> {
    let query = input.query;
    if query.is_empty() {
        return Err(Response::invalid_params("query is required"));
    }
    let limit = input.limit.unwrap_or(100);
    let expected_kind = match input.kind {
        Some(KindFilter::Direct) => Some(ConversationKind::Direct),
        Some(KindFilter::Group) => Some(ConversationKind::Group),
        None => None,
    };

    let conv_kinds = if expected_kind.is_some() {
        let summaries = AppConversation::conversations(host).await.map_err(from_host)?;
        let map: HashMap<String, ConversationKind> =
            summaries.into_iter().map(|s| (s.id, s.kind)).collect();
        Some(map)
    } else {
        None
    };

    let msgs =
        AppConversation::search(host, query, input.conversation, limit).await.map_err(from_host)?;

    let out: Vec<MessageView> = msgs
        .into_iter()
        .filter(|m| !is_group_system_type(&m.content_type))
        .filter(|m| {
            if let (Some(k), Some(map)) = (expected_kind, &conv_kinds) {
                map.get(&m.conversation).copied() == Some(k)
            } else {
                true
            }
        })
        .map(|m| MessageView::from(&m))
        .collect();
    Ok(SearchOutput { matches: out })
}

pub(crate) async fn transcript_digest<H: AppHost>(
    host: &H,
    input: ConversationRef,
) -> Reply<TranscriptDigestOutput> {
    let transcript =
        AppConversation::transcript_digest(host, input.conversation).await.map_err(from_host)?;
    Ok(TranscriptDigestOutput { digest: transcript.digest, rows: u64::from(transcript.rows) })
}
