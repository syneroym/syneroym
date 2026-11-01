//! Server half: settings, roster.

#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};
use syneroym_app_host::{AppDataLayer, AppHost, AppSigning};
use syneroym_roym_core::{
    area::Area,
    clock,
    directory::{Member, SynOrgSettings},
    envelope::Response,
    membership::MembershipVerdict,
    method::{NoParams, Reply, WireNoParams, directory::DirectoryInfoOutput},
    paging,
    safety::PublicationLimits,
};

use super::{
    MEMBERS, SETTINGS, SETTINGS_KEY, ensure_coll, get_json, issuer_did, publication_ops, put_json,
    standing,
};
use crate::app;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SynOrgSettingsInput {
    pub name: String,
    pub rules: String,
    #[serde(default)]
    pub area: Vec<Area>,
    #[serde(default)]
    pub categories: Vec<String>,
    pub support_contact: String,
    pub dispute_path: String,
    pub retention_secs: u64,
    #[serde(default)]
    pub publication_limits: PublicationLimits,
}

impl SynOrgSettingsInput {
    pub(crate) fn into_settings(self) -> SynOrgSettings {
        let Self {
            name,
            rules,
            area,
            categories,
            support_contact,
            dispute_path,
            retention_secs,
            publication_limits,
        } = self;
        SynOrgSettings {
            name,
            rules,
            area,
            categories,
            support_contact,
            dispute_path,
            retention_secs,
            publication_limits,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MemberAddInput {
    pub did: String,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MemberListOutput {
    pub members: Vec<Member>,
}

pub(in crate::app) async fn load_settings<H: AppHost>(
    host: &H,
) -> Result<Option<SynOrgSettings>, String> {
    ensure_coll(host, SETTINGS, &[]).await?;
    get_json(host, SETTINGS, SETTINGS_KEY).await
}

pub(crate) async fn get_settings<H: AppHost>(
    host: &H,
    _input: NoParams,
) -> Reply<Option<SynOrgSettings>> {
    load_settings(host).await.map_err(Response::internal_error)
}

pub(crate) async fn set_settings<H: AppHost>(
    host: &H,
    input: SynOrgSettingsInput,
) -> Reply<SynOrgSettings> {
    let settings = input.into_settings();
    if let Err(e) = settings.validate() {
        return Err(Response::invalid_params(e.to_string()));
    }
    if let Err(e) = ensure_coll(host, SETTINGS, &[]).await {
        return Err(Response::internal_error(e));
    }
    if let Err(e) = put_json(host, SETTINGS, SETTINGS_KEY, &settings).await {
        return Err(Response::internal_error(e));
    }
    Ok(settings)
}

async fn member_count<H: AppHost>(host: &H) -> Result<u64, String> {
    ensure_coll(host, MEMBERS, &[]).await?;
    Ok(paging::count(host, MEMBERS, None).await? as u64)
}

pub(crate) async fn info<H: AppHost>(
    host: &H,
    _input: WireNoParams,
) -> Reply<Option<DirectoryInfoOutput>> {
    let settings = match load_settings(host).await {
        Ok(s) => s,
        Err(e) => return Err(Response::internal_error(e)),
    };
    let Some(settings) = settings else {
        // Refusing would be indistinguishable from a network fault, which
        // is worse for the case this is actually about: someone adding an
        // address a friend gave them.
        return Ok(None);
    };
    // Best-effort: a retention prune that fails must not fail the probe a
    // stranger makes while deciding whether to trust this group.
    let _ = publication_ops::prune_expired_publications(host, settings.retention_secs).await;
    let count = match member_count(host).await {
        Ok(c) => c,
        Err(e) => return Err(Response::internal_error(e)),
    };
    let issuer = issuer_did(host).await;
    let signing_did = AppSigning::signing_identity(host).await.ok().map(|id| id.signing_did);
    Ok(Some(DirectoryInfoOutput {
        name: settings.name,
        rules: settings.rules,
        area: settings.area,
        categories: settings.categories,
        support_contact: settings.support_contact,
        dispute_path: settings.dispute_path,
        retention_secs: settings.retention_secs,
        member_count: count,
        issuer_did: issuer,
        signing_did,
    }))
}

pub(crate) async fn member_add<H: AppHost>(host: &H, input: MemberAddInput) -> Reply<Member> {
    if input.did.is_empty() {
        return Err(Response::invalid_params("did is required"));
    }
    let did = input.did;
    let note = input.note.unwrap_or_default();
    if let Err(e) = ensure_coll(host, MEMBERS, &[]).await {
        return Err(Response::internal_error(e));
    }
    let member = Member { did: did.clone(), note, added_at_secs: clock::now_secs() };
    if let Err(e) = put_json(host, MEMBERS, &did, &member).await {
        return Err(Response::internal_error(e));
    }
    Ok(member)
}

pub(crate) async fn member_remove<H: AppHost>(
    host: &H,
    input: app::DidInput,
) -> Reply<app::RemovedOutput> {
    let did = input.did;
    if let Err(e) = ensure_coll(host, MEMBERS, &[]).await {
        return Err(Response::internal_error(e));
    }
    let now = clock::now_secs();
    let verdict = match standing::own_verdict(host, &did, None, now).await {
        Ok(v) => v,
        Err(e) => return Err(Response::internal_error(e)),
    };
    if matches!(verdict, MembershipVerdict::Valid { .. } | MembershipVerdict::Suspended { .. }) {
        return Err(Response::invalid_params("revoke this member's credential first"));
    }
    let existed = AppDataLayer::get(host, MEMBERS.to_string(), did.clone())
        .await
        .map(|o| o.is_some())
        .unwrap_or(false);
    if existed && let Err(e) = AppDataLayer::delete(host, MEMBERS.to_string(), did).await {
        return Err(Response::internal_error(e.to_string()));
    }
    Ok(app::RemovedOutput { removed: existed })
}

pub(crate) async fn member_list<H: AppHost>(host: &H, _input: NoParams) -> Reply<MemberListOutput> {
    if let Err(e) = ensure_coll(host, MEMBERS, &[]).await {
        return Err(Response::internal_error(e));
    }
    let members: Vec<Member> = match paging::query_all(host, MEMBERS, None).await {
        Ok(v) => v,
        Err(e) => return Err(Response::internal_error(e)),
    };
    Ok(MemberListOutput { members })
}
