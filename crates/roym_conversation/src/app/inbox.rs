//! Inbound message admission and first-contact rate limiting.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use syneroym_app_host::{
    AppConversation, AppDataLayer, AppHost,
    types::{
        conversation::{Admission, ConversationError, DropAnswer, GroupInfo, Message},
        data_layer::RecordWriteValue,
    },
};
use syneroym_roym_core::{
    clock,
    conversation::group::{GROUP_PROFILE_CONTENT_TYPE, MEMBERSHIP_EVENT_CONTENT_TYPE},
    paging,
};

use super::{
    FIRST_CONTACT_CHARGES, ensure_charges, load_admission, person_did_for_address, profile_call,
    set_admission, set_admission_peer,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct FirstContactCharge {
    pub at_secs: u64,
    pub admission: String,
    pub blocked: bool,
    pub retry_after_secs: Option<u64>,
}

pub(crate) async fn load_charge<H: AppHost>(
    host: &H,
    message_id: &str,
) -> Result<Option<FirstContactCharge>, String> {
    ensure_charges(host).await?;
    let row = AppDataLayer::get(host, FIRST_CONTACT_CHARGES.to_string(), message_id.to_string())
        .await
        .map_err(|e| e.to_string())?;
    match row {
        Some(r) => serde_json::from_slice(&r.payload).map(Some).map_err(|e| e.to_string()),
        None => Ok(None),
    }
}

pub(crate) async fn save_charge<H: AppHost>(
    host: &H,
    message_id: &str,
    charge: &FirstContactCharge,
) -> Result<(), String> {
    ensure_charges(host).await?;
    let payload = serde_json::to_vec(charge).map_err(|e| e.to_string())?;
    AppDataLayer::put(
        host,
        FIRST_CONTACT_CHARGES.to_string(),
        RecordWriteValue { id: message_id.to_string(), payload },
    )
    .await
    .map_err(|e| e.to_string())
}

static LAST_PRUNE_SECS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
const PRUNE_INTERVAL_SECS: u64 = 3600;

pub(crate) async fn prune_old_charges<H: AppHost>(host: &H, now_secs: u64) {
    let last = LAST_PRUNE_SECS.load(std::sync::atomic::Ordering::Relaxed);
    if now_secs.saturating_sub(last) < PRUNE_INTERVAL_SECS {
        return;
    }
    LAST_PRUNE_SECS.store(now_secs, std::sync::atomic::Ordering::Relaxed);
    let floor = now_secs.saturating_sub(30 * 24 * 3600);
    let filter = json!({ "at_secs": { "$lt": floor } }).to_string();
    if let Ok(old) =
        paging::filter_map(host, FIRST_CONTACT_CHARGES, Some(filter), |r| Some(r.id)).await
    {
        for id in old {
            let _ = AppDataLayer::delete(host, FIRST_CONTACT_CHARGES.to_string(), id).await;
        }
    }
}

pub(crate) async fn is_blocked<H: AppHost>(
    host: &H,
    address: &str,
    person_did: Option<&str>,
) -> Result<bool, String> {
    let resp =
        profile_call(host, "block.check", json!({ "address": address, "person_did": person_did }))
            .await?;
    if resp.error.is_some() {
        return Err(format!("block.check returned an error: {:?}", resp.error));
    }
    let blocked = resp.result.as_ref().and_then(|v| v.get("blocked")).and_then(Value::as_bool);
    match blocked {
        Some(v) => Ok(v),
        None => Err("block.check: missing 'blocked' field in result".to_string()),
    }
}

pub async fn on_message<H: AppHost>(host: &H, msg: Message) -> Result<Admission, String> {
    let now = clock::now_secs();
    prune_old_charges(host, now).await;

    if msg.content_type == MEMBERSHIP_EVENT_CONTENT_TYPE {
        return Ok(Admission::Drop(DropAnswer {
            reason: "reserved-content-type".to_string(),
            report: false,
        }));
    }

    let is_group = match AppConversation::group_info(host, msg.conversation.clone()).await {
        Ok(info) => Some(info),
        Err(ConversationError::InvalidArgument(_) | ConversationError::NotFound) => None,
        Err(e) => return Err(format!("group-info lookup failed: {e:?}")),
    };

    if msg.content_type == GROUP_PROFILE_CONTENT_TYPE {
        return Ok(Admission::Drop(DropAnswer {
            reason: "reserved-content-type".to_string(),
            report: false,
        }));
    }

    if let Some(info) = is_group {
        return on_group_message(host, &msg, &info).await;
    }

    on_direct_message(host, &msg, now).await
}

async fn on_direct_message<H: AppHost>(
    host: &H,
    msg: &Message,
    now: u64,
) -> Result<Admission, String> {
    let person_did = person_did_for_address(host, &msg.author).await;
    let blocked = is_blocked(host, &msg.author, person_did.as_deref()).await?;

    let is_accepted =
        load_admission(host, &msg.conversation).await?.is_some_and(|s| s == "accepted");

    if is_accepted {
        if blocked {
            return Ok(Admission::Drop(DropAnswer {
                reason: "blocked".to_string(),
                report: false,
            }));
        }
        return Ok(Admission::Accept);
    }

    let charge = match load_charge(host, &msg.id).await? {
        Some(c) => c,
        None => {
            let admit = profile_call(
                host,
                "contacts.admit-first-contact",
                json!({ "sender_address": msg.author, "sender_person_did": person_did }),
            )
            .await?;
            if admit.error.is_some() {
                return Err(format!(
                    "contacts.admit-first-contact returned an error: {:?}",
                    admit.error
                ));
            }
            let res = admit
                .result
                .ok_or_else(|| "contacts.admit-first-contact: missing result".to_string())?;
            let adm_str = res.get("admission").and_then(Value::as_str).ok_or_else(|| {
                "contacts.admit-first-contact: missing 'admission' field".to_string()
            })?;
            let blk = res.get("blocked").and_then(Value::as_bool).unwrap_or(blocked);
            let retry = res.get("retry_after_secs").and_then(Value::as_u64);
            let c = FirstContactCharge {
                at_secs: now,
                admission: adm_str.to_string(),
                blocked: blk,
                retry_after_secs: retry,
            };
            save_charge(host, &msg.id, &c).await?;
            c
        }
    };

    if charge.admission == "rate-limited" {
        return Ok(Admission::Drop(DropAnswer {
            reason: "rate-limited".to_string(),
            report: true,
        }));
    }

    set_admission_peer(host, &msg.conversation, "accepted", Some(&msg.author)).await?;

    if charge.blocked || blocked {
        return Ok(Admission::Drop(DropAnswer { reason: "blocked".to_string(), report: false }));
    }

    Ok(Admission::Accept)
}

async fn on_group_message<H: AppHost>(
    host: &H,
    msg: &Message,
    info: &GroupInfo,
) -> Result<Admission, String> {
    let author_did = person_did_for_address(host, &msg.author).await;
    let author_blocked = is_blocked(host, &msg.author, author_did.as_deref()).await?;

    let visibility = match load_admission(host, &msg.conversation).await? {
        Some(vis) => vis,
        None => {
            let vis = decide_group_visibility(host, info).await?;
            // A concurrent first message may have decided while this one
            // waited on the profile service. The earlier decision stands, so
            // the group never flips between two answers.
            match load_admission(host, &msg.conversation).await? {
                Some(existing) => existing,
                None => {
                    set_admission(host, &msg.conversation, &vis).await?;
                    vis
                }
            }
        }
    };

    // A hidden group is not shown; hold the message so it can be admitted
    // later when the user shows the group.
    if visibility == "hidden" {
        return Ok(Admission::Hold("group-hidden".to_string()));
    }

    if author_blocked {
        return Ok(Admission::Drop(DropAnswer { reason: "blocked".to_string(), report: false }));
    }

    Ok(Admission::Accept)
}

async fn decide_group_visibility<H: AppHost>(host: &H, info: &GroupInfo) -> Result<String, String> {
    if info.is_owner {
        return Ok("shown".to_string());
    }
    let owner_did = person_did_for_address(host, &info.owner).await;
    let owner_blocked = is_blocked(host, &info.owner, owner_did.as_deref()).await?;
    // A blocked owner's group starts hidden, so the person can show it
    // later after unblocking.
    if owner_blocked {
        return Ok("hidden".to_string());
    }
    let admit = profile_call(
        host,
        "contacts.admit-first-contact",
        json!({ "sender_address": info.owner, "sender_person_did": owner_did }),
    )
    .await?;
    if admit.error.is_some() {
        return Err(format!("contacts.admit-first-contact returned an error: {:?}", admit.error));
    }
    let res =
        admit.result.ok_or_else(|| "contacts.admit-first-contact: missing result".to_string())?;
    let adm = res
        .get("admission")
        .and_then(Value::as_str)
        .ok_or_else(|| "contacts.admit-first-contact: missing 'admission' field".to_string())?;
    if adm == "allow" { Ok("shown".to_string()) } else { Ok("hidden".to_string()) }
}
