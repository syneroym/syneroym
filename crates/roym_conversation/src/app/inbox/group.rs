//! Inbound group message handling: admission, blocking, system types, and
//! storage.

use syneroym_app_host::{AppConversation, AppHost, types::conversation::Message};
use syneroym_roym_core::conversation::group::GroupAdmission;

use super::record_refused;
use crate::app::{
    create_conversation,
    group::{Stored, new_group_row, store_group_message, sync_membership_rows},
    load_conversation, put_conversation,
};

pub(super) async fn on_group_message<H: AppHost>(
    host: &H,
    msg: &Message,
    now: u64,
) -> Result<(), String> {
    let info = AppConversation::group_info(host, msg.conversation.clone())
        .await
        .map_err(|e| format!("group-info: {e:?}"))?;

    let mut row = match load_conversation(host, &msg.conversation).await? {
        Some(r) => r,
        None => {
            let mut r = new_group_row(host, &msg.conversation, &info, now).await?;
            sync_membership_rows(host, &mut r, &info).await?;
            if !create_conversation(host, &r).await? {
                load_conversation(host, &msg.conversation)
                    .await?
                    .ok_or_else(|| "group row missing after concurrent create".to_string())?
            } else {
                r
            }
        }
    };

    let admission = row.group.as_ref().map(|g| g.admission.clone());
    match admission {
        Some(GroupAdmission::Shown) => {}
        Some(GroupAdmission::Hidden) => {
            return record_refused(host, msg, "group-hidden", now).await;
        }
        Some(GroupAdmission::Refused { reason }) => {
            return record_refused(host, msg, &reason, now).await;
        }
        None => return Err("group row without group meta".to_string()),
    }

    let stored = store_group_message(host, &mut row, &info, msg, now).await?;
    if stored == Stored::Kept {
        put_conversation(host, &row).await?;
    }
    Ok(())
}
