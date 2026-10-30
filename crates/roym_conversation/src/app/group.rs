//! Group conversation operations: create, rename, membership, info, sync, and
//! visibility.

use serde::{Deserialize, Serialize};
use syneroym_app_host::{AppConversation, AppHost, types::conversation::ConversationError};
use syneroym_roym_core::{
    conversation::{
        AdmissionState,
        group::{
            GROUP_ADD_UNREACHABLE_MESSAGE, GROUP_DELIVERY_NOTICE, GROUP_JOIN_BOUNDARY_NOTICE,
            GROUP_KEY_TRUST_NOTICE, GROUP_REMOVED_NOTICE, GROUP_RESTORED_NOTICE,
            OWNER_CAN_READ_NOTICE, validate_group_name,
        },
    },
    envelope::Response,
    method::Reply,
};

use super::{
    contacts_map, from_host, load_admission,
    messages::{ConversationRef, resolve_open_address},
    set_admission,
};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupCreateInput {
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GroupCreateOutput {
    pub conversation_id: String,
    pub owner_address: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupRenameInput {
    #[serde(alias = "group")]
    pub conversation: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GroupRenameOutput {
    pub renamed: bool,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupMemberInput {
    #[serde(alias = "group")]
    pub conversation: String,
    pub address: Option<String>,
    pub person_did: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GroupAddOutput {
    pub added: String,
    pub epoch: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct GroupRemoveOutput {
    pub removed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct GroupSyncOutput {
    pub synced: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdmissionOutput {
    pub admission: AdmissionState,
}

#[derive(Debug, Clone, Serialize)]
pub struct GroupUnhideOutput {
    pub admission: AdmissionState,
    pub filled_in: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct GroupNotices {
    pub owner_can_read: &'static str,
    pub key_trust: &'static str,
    pub delivery: &'static str,
    pub join_boundary: &'static str,
    pub removed: Option<&'static str>,
    pub restored: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GroupMemberItem {
    pub address: String,
    pub person_did: Option<String>,
    pub is_owner: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct GroupInfoOutput {
    pub conversation_id: String,
    pub name: Option<String>,
    pub owner_address: String,
    pub owner_person_did: Option<String>,
    pub is_owner: bool,
    pub is_member: bool,
    pub restored_only: bool,
    pub admission: AdmissionState,
    pub epoch: u64,
    pub key_epoch: u64,
    pub key_stored_at_ms: i64,
    pub members: Vec<GroupMemberItem>,
    pub can_read_new_messages: bool,
    pub notices: GroupNotices,
}

pub(crate) async fn create<H: AppHost>(
    host: &H,
    input: GroupCreateInput,
) -> Reply<GroupCreateOutput> {
    let name = match input.name {
        Some(raw) => match validate_group_name(&raw) {
            Ok(n) => Some(n),
            Err(e) => return Err(Response::invalid_params(e.to_string())),
        },
        None => None,
    };
    let group_id = AppConversation::create_group(host).await.map_err(from_host)?;
    if let Some(n) = name
        && let Err(e) = AppConversation::set_group_name(host, group_id.clone(), n).await
    {
        return Err(from_host(e));
    }
    set_admission(host, &group_id, "shown").await.map_err(Response::internal_error)?;
    let info = AppConversation::group_info(host, group_id.clone()).await.map_err(from_host)?;
    Ok(GroupCreateOutput { conversation_id: group_id, owner_address: info.owner })
}

pub(crate) async fn rename<H: AppHost>(
    host: &H,
    input: GroupRenameInput,
) -> Reply<GroupRenameOutput> {
    let valid_name = match validate_group_name(&input.name) {
        Ok(n) => n,
        Err(e) => return Err(Response::invalid_params(e.to_string())),
    };
    AppConversation::set_group_name(host, input.conversation, valid_name.clone())
        .await
        .map_err(from_host)?;
    Ok(GroupRenameOutput { renamed: true, name: valid_name })
}

pub(crate) async fn add_member<H: AppHost>(
    host: &H,
    input: GroupMemberInput,
) -> Reply<GroupAddOutput> {
    let address =
        resolve_open_address(host, input.address.as_deref(), input.person_did.as_deref()).await?;
    if let Err(e) =
        AppConversation::add_member(host, input.conversation.clone(), address.clone()).await
    {
        return Err(match e {
            ConversationError::Unreachable(_) => {
                Response::invalid_params(GROUP_ADD_UNREACHABLE_MESSAGE)
            }
            ConversationError::QuotaExceeded => Response::invalid_params("this group is full"),
            err => from_host(err),
        });
    }

    let updated_info =
        AppConversation::group_info(host, input.conversation).await.map_err(from_host)?;
    Ok(GroupAddOutput { added: address, epoch: updated_info.epoch })
}

pub(crate) async fn remove_member<H: AppHost>(
    host: &H,
    input: GroupMemberInput,
) -> Reply<GroupRemoveOutput> {
    let address =
        resolve_open_address(host, input.address.as_deref(), input.person_did.as_deref()).await?;
    AppConversation::remove_member(host, input.conversation, address).await.map_err(from_host)?;
    Ok(GroupRemoveOutput { removed: true })
}

pub(crate) async fn info<H: AppHost>(host: &H, input: ConversationRef) -> Reply<GroupInfoOutput> {
    let group = input.conversation;
    let info = AppConversation::group_info(host, group.clone()).await.map_err(from_host)?;
    let admission = load_admission(host, &group)
        .await
        .unwrap_or(None)
        .unwrap_or_else(|| if info.restored { "hidden".to_string() } else { "shown".to_string() });

    let contacts = contacts_map(host).await;
    let members: Vec<GroupMemberItem> = info
        .members
        .iter()
        .map(|m| GroupMemberItem {
            address: m.clone(),
            person_did: contacts.get(m).cloned(),
            is_owner: *m == info.owner,
        })
        .collect();
    let can_read = info.key_epoch >= info.epoch && info.is_member && !info.restored;
    let owner_did = contacts.get(&info.owner).cloned();
    Ok(GroupInfoOutput {
        conversation_id: group,
        name: info.name,
        owner_address: info.owner,
        owner_person_did: owner_did,
        is_owner: info.is_owner,
        is_member: info.is_member,
        restored_only: info.restored,
        admission: AdmissionState { state: admission },
        epoch: info.epoch,
        key_epoch: info.key_epoch,
        key_stored_at_ms: info.key_stored_at,
        members,
        can_read_new_messages: can_read,
        notices: GroupNotices {
            owner_can_read: OWNER_CAN_READ_NOTICE,
            key_trust: GROUP_KEY_TRUST_NOTICE,
            delivery: GROUP_DELIVERY_NOTICE,
            join_boundary: GROUP_JOIN_BOUNDARY_NOTICE,
            removed: if !info.is_member { Some(GROUP_REMOVED_NOTICE) } else { None },
            restored: if info.restored { Some(GROUP_RESTORED_NOTICE) } else { None },
        },
    })
}

pub(crate) async fn sync<H: AppHost>(host: &H, input: ConversationRef) -> Reply<GroupSyncOutput> {
    AppConversation::group_info(host, input.conversation.clone()).await.map_err(from_host)?;
    AppConversation::sync_now(host, input.conversation).await.map_err(from_host)?;
    Ok(GroupSyncOutput { synced: true })
}

pub(crate) async fn hide<H: AppHost>(host: &H, input: ConversationRef) -> Reply<AdmissionOutput> {
    AppConversation::group_info(host, input.conversation.clone()).await.map_err(from_host)?;
    set_admission(host, &input.conversation, "hidden").await.map_err(Response::internal_error)?;
    Ok(AdmissionOutput { admission: AdmissionState { state: "hidden".to_string() } })
}

pub(crate) async fn unhide<H: AppHost>(
    host: &H,
    input: ConversationRef,
) -> Reply<GroupUnhideOutput> {
    AppConversation::group_info(host, input.conversation.clone()).await.map_err(from_host)?;
    set_admission(host, &input.conversation, "shown").await.map_err(Response::internal_error)?;
    let filled_in =
        AppConversation::readmit(host, input.conversation, vec!["group-hidden".to_string()])
            .await
            .map_err(from_host)?;
    Ok(GroupUnhideOutput {
        admission: AdmissionState { state: "shown".to_string() },
        filled_in: filled_in as u64,
    })
}
