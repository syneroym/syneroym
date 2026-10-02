//! The group half of Roym's conversation vocabulary: who may name a group,
//! how a membership change is written into Roym's own copy, how a first
//! sight of a group is admitted, and the one transcript check.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use syneroym_signed_record::{EnvelopeError, content_digest};

use super::{MessageRow, sort_key};

pub const GROUP_PROFILE_CONTENT_TYPE: &str = "application/vnd.roym.group-profile+json";
pub const MEMBERSHIP_EVENT_CONTENT_TYPE: &str = "application/vnd.roym.membership-event+json";
pub const GROUP_PROFILE_VERSION: u32 = 1;
pub const MAX_GROUP_NAME_CHARS: usize = 80;
pub const TRANSCRIPT_DIGEST_PREFIX: &str = "roym-transcript:";

/// Verbatim copies live in `crates/roym_web/ui/src/groups/words.ts`; a test
/// in `group/tests.rs` compares them character for character.
pub const OWNER_CAN_READ_NOTICE: &str = "The owner of this group makes and shares the group's \
                                         key, so the owner can read every message sent while they \
                                         own it. Adding or removing a member is shown to everyone \
                                         in the group.";
pub const GROUP_KEY_TRUST_NOTICE: &str =
    "Each member's messages are signed with a key this installation first saw when they joined. \
     That is a weaker check than a signed record, so messages here are never marked as verified.";
pub const GROUP_DELIVERY_NOTICE: &str =
    "\"Not yet delivered to every member\" means at least one member has not received it \
     directly. Members also pass messages to each other, so a member may still receive it later.";
pub const GROUP_JOIN_BOUNDARY_NOTICE: &str = "You can read messages sent after you joined. \
                                              Messages from before you joined are not shared with \
                                              you.";
pub const GROUP_REMOVED_NOTICE: &str = "You were removed from this group. You can still read what \
                                        you received before. You cannot read or send new messages.";
pub const GROUP_RESTORED_NOTICE: &str =
    "This group's history was restored from a backup. This installation is not a member, so it \
     cannot send or receive new messages here. Ask the owner to add your new address.";
pub const GROUP_ADD_UNREACHABLE_MESSAGE: &str = "Could not reach this person to add them. Someone \
                                                 you have not talked to before must be online \
                                                 when you add them.";
pub const GROUP_HIDDEN_NOTICE: &str =
    "A hidden group is not shown, and its new messages are not kept here. This installation still \
     receives them underneath, and you stay a member until the owner removes you.";
pub const TRANSCRIPT_CHECK_NOTICE: &str =
    "Members who see the same code hold the same messages in the same order.";
pub const CARDS_NOT_IN_GROUPS_MESSAGE: &str = "Cards are sent only in a 1:1 conversation.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum GroupAdmission {
    Shown,
    Hidden,
    Refused { reason: String },
}

/// Which message set the current name. Compared by `sort_key` order, so
/// every member that read the same messages picks the same name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NameSource {
    pub sender_timestamp_ms: i64,
    pub author: String,
    pub message_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupMeta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name_source: Option<NameSource>,
    pub admission: GroupAdmission,
    /// How many host membership events are already copied into Roym's
    /// own store. Lets `sync_membership_rows` skip work when nothing new
    /// arrived.
    pub membership_events_copied: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GroupNameError {
    #[error("a group name must not be empty")]
    Empty,
    #[error("a group name must be at most {MAX_GROUP_NAME_CHARS} characters")]
    TooLong,
    #[error("a group name must not contain control characters")]
    ControlCharacter,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GroupProfileError {
    #[error("group profile body is not valid JSON: {0}")]
    Json(String),
    #[error("group profile must be exactly {{\"name\": <string>, \"version\": 1}}")]
    Shape,
    #[error(transparent)]
    Name(#[from] GroupNameError),
}

/// Trims, then checks length in chars and refuses control characters.
pub fn validate_group_name(raw: &str) -> Result<String, GroupNameError> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(GroupNameError::Empty);
    }
    if name.chars().count() > MAX_GROUP_NAME_CHARS {
        return Err(GroupNameError::TooLong);
    }
    if name.chars().any(char::is_control) {
        return Err(GroupNameError::ControlCharacter);
    }
    Ok(name.to_string())
}

/// `{"name": <validated>, "version": 1}` as bytes. Callers validate first.
#[must_use]
pub fn group_profile_body(name: &str) -> Vec<u8> {
    serde_json::json!({
        "name": name,
        "version": GROUP_PROFILE_VERSION,
    })
    .to_string()
    .into_bytes()
}

/// Strict: exactly two keys, `version == 1`, and a name that passes
/// `validate_group_name`.
pub fn parse_group_profile(body: &[u8]) -> Result<String, GroupProfileError> {
    let v: Value =
        serde_json::from_slice(body).map_err(|e| GroupProfileError::Json(e.to_string()))?;
    let obj = v.as_object().ok_or(GroupProfileError::Shape)?;
    if obj.len() != 2 {
        return Err(GroupProfileError::Shape);
    }
    let version = obj.get("version").and_then(Value::as_u64).ok_or(GroupProfileError::Shape)?;
    if version != u64::from(GROUP_PROFILE_VERSION) {
        return Err(GroupProfileError::Shape);
    }
    let name_str = obj.get("name").and_then(Value::as_str).ok_or(GroupProfileError::Shape)?;
    let validated = validate_group_name(name_str)?;
    Ok(validated)
}

/// Applies a profile message to `meta` if it sorts after the current
/// source. Returns true when the name changed.
pub fn apply_group_profile(meta: &mut GroupMeta, name: String, src: NameSource) -> bool {
    let newer = match &meta.name_source {
        None => true,
        Some(cur) => {
            (src.sender_timestamp_ms, &src.author, &src.message_id)
                > (cur.sender_timestamp_ms, &cur.author, &cur.message_id)
        }
    };
    if !newer {
        return false;
    }
    let changed = meta.name.as_deref() != Some(name.as_str());
    meta.name = Some(name);
    meta.name_source = Some(src);
    changed
}

/// Canonical body for a membership row: `{"action","epoch","subject"}`.
#[must_use]
pub fn membership_event_body(action: &str, subject: &str, epoch: u64) -> String {
    serde_json::json!({
        "action": action,
        "epoch": epoch,
        "subject": subject,
    })
    .to_string()
}

/// Pure admission decision for a newly observed group.
#[must_use]
pub fn admission_for_new_group(is_owner: bool, admit_answer: Option<&str>) -> GroupAdmission {
    if is_owner {
        return GroupAdmission::Shown;
    }
    match admit_answer {
        Some("allow") => GroupAdmission::Shown,
        Some("blocked") => GroupAdmission::Refused { reason: "blocked".to_string() },
        _ => GroupAdmission::Refused { reason: "rate-limited".to_string() },
    }
}

/// Content digest of a group's message transcript. Sorts a copy of `rows` by
/// `sort_key`.
pub fn transcript_digest(rows: &[MessageRow]) -> Result<String, EnvelopeError> {
    let mut sorted: Vec<&MessageRow> = rows.iter().collect();
    sorted.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)));
    let lines: Vec<Value> = sorted
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.id,
                "author": r.author,
                "sender_timestamp_ms": r.sender_timestamp_ms,
                "content_type": r.content_type,
            })
        })
        .collect();
    content_digest(TRANSCRIPT_DIGEST_PREFIX, &Value::Array(lines))
}

/// True for the two reserved types a person never authors through
/// `conversation.send`, and which `delete-message`/`search` skip.
#[must_use]
pub fn is_group_system_type(content_type: &str) -> bool {
    content_type == GROUP_PROFILE_CONTENT_TYPE || content_type == MEMBERSHIP_EVENT_CONTENT_TYPE
}

#[cfg(test)]
mod tests;
