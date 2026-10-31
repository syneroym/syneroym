//! Server half: `credential.*` and `revocation.*` -- a SynOrg issuing and
//! withdrawing signed membership statements about its own members
//! its own members.

use std::collections::BTreeSet;

use syneroym_roym_core::method::Reply;

use super::*;
use crate::app::{self, standing::IssuedRecordRow};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CredentialIssueInput {
    pub member_did: String,
    pub categories: Vec<String>,
    pub area: Option<Vec<Area>>,
    pub expires_at_secs: u64,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RevokeInput {
    pub credential_record_id: String,
    pub reason: Option<String>,
}

/// Signs `payload` as this SynOrg (the installation's owner).
/// Returns the envelope JSON, its derived record id, and the
/// `issued_at_secs` the envelope itself carries -- a stored row uses
/// this, never a separate `clock::now_secs()` call, so the row and the
/// signed bytes can never disagree.
pub(in crate::app) async fn sign_as_synorg<H: AppHost>(
    host: &H,
    record_type: &str,
    version: u32,
    subject: &str,
    payload: &impl Serialize,
    expires_at_secs: Option<u64>,
    supersedes: Option<String>,
) -> Result<(String, String, u64), Response> {
    let now = clock::now_secs();
    let (principal, _owner) = match signing::person_principal(host, now).await {
        Ok(v) => v,
        Err(CertificateError::NotEnrolled) => {
            return Err(Response::invalid_params("signing-not-enrolled"));
        }
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    let payload_json = match serde_json::to_string(payload) {
        Ok(s) => s,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    let draft = RecordDraft {
        version,
        record_type: record_type.to_string(),
        subject: subject.to_string(),
        payload: payload_json,
        expires_at_secs,
        supersedes,
    };
    let envelope = match AppSigning::sign_record(host, draft, principal).await {
        Ok(e) => e,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    let parsed = match Envelope::from_json(&envelope) {
        Ok(e) => e,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    let record_id = match parsed.record_id() {
        Ok(id) => id,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    Ok((envelope, record_id, parsed.issued_at_secs))
}

fn validate_categories_within_synorg(
    categories: &[String],
    settings: &directory::SynOrgSettings,
) -> Result<(), Response> {
    let own: BTreeSet<String> =
        settings.categories.iter().map(|c| directory::normalize_category(c)).collect();
    for c in categories {
        let normalized = directory::normalize_category(c);
        if !own.contains(&normalized) {
            return Err(Response::invalid_params(format!(
                "category '{c}' is not one of this SynOrg's own categories"
            )));
        }
    }
    Ok(())
}

/// Upserts `MEMBERS[member_did]`, keeping its existing note and
/// `added_at_secs` when the caller supplies neither.
async fn upsert_member<H: AppHost>(
    host: &H,
    member_did: &str,
    note: &str,
    now: u64,
) -> Result<(), String> {
    ensure_coll(host, MEMBERS, &[]).await?;
    let existing: Option<directory::Member> = get_json(host, MEMBERS, member_did).await?;
    let member = directory::Member {
        did: member_did.to_string(),
        note: if note.is_empty() {
            existing.as_ref().map(|m| m.note.clone()).unwrap_or_default()
        } else {
            note.to_string()
        },
        added_at_secs: existing.map_or(now, |m| m.added_at_secs),
    };
    put_json(host, MEMBERS, member_did, &member).await
}

pub(crate) async fn issue<H: AppHost>(
    host: &H,
    input: CredentialIssueInput,
) -> Reply<app::SignedRecordOutput> {
    if input.member_did.is_empty() {
        return Err(Response::invalid_params("member_did is required"));
    }
    let member_did = input.member_did;
    let categories = input.categories;
    let area = input.area.unwrap_or_default();
    let expires_at_secs = input.expires_at_secs;

    let settings = match synorg::load_settings(host).await {
        Ok(Some(s)) => s,
        Ok(None) => return Err(Response::invalid_params("this installation runs no SynOrg yet")),
        Err(e) => return Err(Response::internal_error(e)),
    };

    let normalized_categories: Vec<String> =
        categories.iter().map(|c| directory::normalize_category(c)).collect();
    let scope = membership::MembershipScope { categories: normalized_categories, area };
    let payload = membership::MembershipCredentialPayload {
        synorg_name: settings.name.clone(),
        member_did: member_did.clone(),
        scope,
    };
    if let Err(e) = payload.validate() {
        return Err(Response::invalid_params(e.to_string()));
    }
    validate_categories_within_synorg(&payload.scope.categories, &settings)?;

    let now = clock::now_secs();
    if !(now < expires_at_secs && expires_at_secs <= now + membership::MAX_CREDENTIAL_LIFETIME_SECS)
    {
        return Err(Response::invalid_params("expires_at_secs is not a valid future expiry"));
    }

    for c in [CREDENTIALS, REVOCATIONS, DECISIONS] {
        if let Err(e) = ensure_coll(host, c, &standing::issued_record_indexes()).await {
            return Err(Response::internal_error(e));
        }
    }

    let supersedes = match current_credential_id(host, &member_did).await {
        Ok(id) => id,
        Err(e) => return Err(Response::internal_error(e)),
    };
    let (envelope, record_id, issued_at_secs) = match sign_as_synorg(
        host,
        record::RECORD_MEMBERSHIP_CREDENTIAL,
        membership::MEMBERSHIP_CREDENTIAL_VERSION,
        &member_did,
        &payload,
        Some(expires_at_secs),
        supersedes,
    )
    .await
    {
        Ok(v) => v,
        Err(resp) => return Err(resp),
    };

    let row = IssuedRecordRow {
        record_id: record_id.clone(),
        member_did: member_did.clone(),
        about: String::new(),
        issued_at_secs,
        until_secs: None,
        is_membership_scope: false,
        envelope: envelope.clone(),
    };
    if let Err(e) = put_json(host, CREDENTIALS, &record_id, &row).await {
        return Err(Response::internal_error(e));
    }
    let note = input.note.as_deref().unwrap_or_default();
    if let Err(e) = upsert_member(host, &member_did, note, now).await {
        return Err(Response::internal_error(e));
    }
    if let Err(e) = standing::rebuild_for(host, &member_did).await {
        return Err(Response::internal_error(e));
    }
    Ok(app::SignedRecordOutput { record_id, envelope })
}

async fn current_credential_id<H: AppHost>(
    host: &H,
    member_did: &str,
) -> Result<Option<String>, String> {
    let filter = json!({ "member_did": member_did }).to_string();
    let mut rows: Vec<IssuedRecordRow> = paging::query_all(host, CREDENTIALS, Some(filter)).await?;
    rows.sort_by(|a, b| {
        b.issued_at_secs.cmp(&a.issued_at_secs).then(a.record_id.cmp(&b.record_id))
    });
    Ok(rows.into_iter().next().map(|r| r.record_id))
}

pub(crate) async fn revoke<H: AppHost>(
    host: &H,
    input: RevokeInput,
) -> Reply<app::SignedRecordOutput> {
    if input.credential_record_id.is_empty() {
        return Err(Response::invalid_params("credential_record_id is required"));
    }
    let credential_record_id = input.credential_record_id;
    let reason = input.reason.unwrap_or_default();

    for c in [CREDENTIALS, REVOCATIONS] {
        if let Err(e) = ensure_coll(host, c, &standing::issued_record_indexes()).await {
            return Err(Response::internal_error(e));
        }
    }
    let row: Option<IssuedRecordRow> =
        match get_json(host, CREDENTIALS, &credential_record_id).await {
            Ok(v) => v,
            Err(e) => return Err(Response::internal_error(e)),
        };
    let Some(row) = row else {
        return Err(Response::invalid_params("no credential with that id was issued here"));
    };

    let about = json!({ "about": credential_record_id }).to_string();
    let existing: Vec<IssuedRecordRow> =
        match paging::query_all(host, REVOCATIONS, Some(about)).await {
            Ok(v) => v,
            Err(e) => return Err(Response::internal_error(e)),
        };
    if let Some(existing) = existing.into_iter().next() {
        // Idempotent retry after a partial failure: the record already
        // exists, but the standing rebuild that should have followed it
        // may not have run. Always re-run it before returning early, or a
        // retry after `rewrite_listed_windows` failed leaves the member
        // looking unrevoked until someone runs `directory.reindex` by hand.
        if let Err(e) = standing::rebuild_for(host, &row.member_did).await {
            return Err(Response::internal_error(e));
        }
        return Ok(app::SignedRecordOutput {
            record_id: existing.record_id,
            envelope: existing.envelope,
        });
    }

    // A credential another one of the member's credentials `supersedes`
    // is already ignored by `pick_current`, so revoking it would sign a
    // real record that changes no verdict anywhere. Refuse and name the
    // credential that actually needs revoking.
    let member_filter = json!({ "member_did": row.member_did }).to_string();
    let siblings: Vec<IssuedRecordRow> =
        match paging::query_all(host, CREDENTIALS, Some(member_filter)).await {
            Ok(v) => v,
            Err(e) => return Err(Response::internal_error(e)),
        };
    if let Some(newer) = siblings.iter().find(|sib| {
        Envelope::from_json(&sib.envelope)
            .is_ok_and(|env| env.supersedes.as_deref() == Some(credential_record_id.as_str()))
    }) {
        return Err(Response::invalid_params(format!(
            "credential '{credential_record_id}' has been replaced by '{}'; revoke the current \
             credential instead",
            newer.record_id
        )));
    }

    let payload = membership::RevocationPayload {
        credential_record_id: credential_record_id.clone(),
        member_did: row.member_did.clone(),
        reason,
    };
    if let Err(e) = payload.validate() {
        return Err(Response::invalid_params(e.to_string()));
    }
    let (envelope, record_id, issued_at_secs) = match sign_as_synorg(
        host,
        record::RECORD_REVOCATION,
        membership::REVOCATION_VERSION,
        &credential_record_id,
        &payload,
        None,
        None,
    )
    .await
    {
        Ok(v) => v,
        Err(resp) => return Err(resp),
    };
    let out_row = IssuedRecordRow {
        record_id: record_id.clone(),
        member_did: row.member_did.clone(),
        about: credential_record_id,
        issued_at_secs,
        until_secs: None,
        is_membership_scope: false,
        envelope: envelope.clone(),
    };
    if let Err(e) = put_json(host, REVOCATIONS, &record_id, &out_row).await {
        return Err(Response::internal_error(e));
    }
    if let Err(e) = standing::rebuild_for(host, &row.member_did).await {
        return Err(Response::internal_error(e));
    }
    Ok(app::SignedRecordOutput { record_id, envelope })
}

/// Every record `collection` holds, or only one member's when the request
/// names `member_did`.
pub(in crate::app) async fn list_issued<H: AppHost>(
    host: &H,
    collection: &str,
    input: app::MemberFilterInput,
) -> Reply<app::IssuedRecordsOutput> {
    if let Err(e) = ensure_coll(host, collection, &standing::issued_record_indexes()).await {
        return Err(Response::internal_error(e));
    }
    let filter = input.member_did.as_deref().map(|m| json!({ "member_did": m }).to_string());
    let rows: Vec<Value> = match paging::query_all(host, collection, filter).await {
        Ok(r) => r,
        Err(e) => return Err(Response::internal_error(e)),
    };
    let records: Vec<IssuedRecordRow> =
        rows.into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect();
    Ok(app::IssuedRecordsOutput { records })
}

pub(crate) async fn list<H: AppHost>(
    host: &H,
    input: app::MemberFilterInput,
) -> Reply<app::IssuedRecordsOutput> {
    list_issued(host, CREDENTIALS, input).await
}

pub(crate) async fn list_revocations<H: AppHost>(
    host: &H,
    input: app::MemberFilterInput,
) -> Reply<app::IssuedRecordsOutput> {
    list_issued(host, REVOCATIONS, input).await
}
