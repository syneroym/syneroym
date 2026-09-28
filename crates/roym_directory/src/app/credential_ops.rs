//! Server half: `credential.*` and `revocation.*` -- a SynOrg issuing and
//! withdrawing signed membership statements about its own members
//! (D-C9-1).

use super::*;
use crate::app::standing::IssuedRecordRow;

/// Signs `payload` as this SynOrg (the installation's owner, D-C9-1).
/// Returns the envelope JSON and its derived record id.
pub(in crate::app) async fn sign_as_synorg<H: AppHost>(
    host: &H,
    record_type: &str,
    version: u32,
    subject: &str,
    payload: &impl Serialize,
    expires_at_secs: Option<u64>,
    supersedes: Option<String>,
) -> Result<(String, String), Response> {
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
    let record_id = match Envelope::from_json(&envelope).and_then(|e| e.record_id()) {
        Ok(id) => id,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    Ok((envelope, record_id))
}

fn validate_categories_within_synorg(
    categories: &[String],
    settings: &directory::SynOrgSettings,
) -> Result<(), Response> {
    let own: std::collections::BTreeSet<String> =
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

struct IssueParams {
    member_did: String,
    categories: Vec<String>,
    area: Vec<Area>,
    expires_at_secs: u64,
}

fn parse_issue_params(req: &Request) -> Result<IssueParams, Response> {
    let member_did = match req.params.get("member_did").and_then(Value::as_str) {
        Some(m) if !m.is_empty() => m.to_string(),
        _ => return Err(Response::invalid_params("member_did is required")),
    };
    let categories: Vec<String> = match req.params.get("categories") {
        Some(v) => serde_json::from_value(v.clone())
            .map_err(|e| Response::invalid_params(format!("invalid categories: {e}")))?,
        None => return Err(Response::invalid_params("categories is required")),
    };
    let area: Vec<Area> = match req.params.get("area") {
        Some(v) if !v.is_null() => serde_json::from_value(v.clone())
            .map_err(|e| Response::invalid_params(format!("invalid area: {e}")))?,
        _ => vec![],
    };
    let expires_at_secs = match req.params.get("expires_at_secs").and_then(Value::as_u64) {
        Some(e) => e,
        None => return Err(Response::invalid_params("expires_at_secs is required")),
    };
    Ok(IssueParams { member_did, categories, area, expires_at_secs })
}

/// Upserts `MEMBERS[member_did]`, keeping its existing note and
/// `added_at_secs` when the caller supplies neither.
async fn upsert_member<H: AppHost>(
    host: &H,
    member_did: &str,
    note: &str,
    now: u64,
) -> Result<(), String> {
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
    ensure_coll(host, MEMBERS, &[]).await?;
    put_json(host, MEMBERS, member_did, &member).await
}

pub(in crate::app) async fn issue<H: AppHost>(host: &H, req: &Request) -> Response {
    let IssueParams { member_did, categories, area, expires_at_secs } =
        match parse_issue_params(req) {
            Ok(p) => p,
            Err(resp) => return resp,
        };

    let settings = match synorg::load_settings(host).await {
        Ok(Some(s)) => s,
        Ok(None) => return Response::invalid_params("this installation runs no SynOrg yet"),
        Err(e) => return Response::internal_error(e),
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
        return Response::invalid_params(e.to_string());
    }
    if let Err(resp) = validate_categories_within_synorg(&payload.scope.categories, &settings) {
        return resp;
    }

    let now = clock::now_secs();
    if !(now < expires_at_secs && expires_at_secs <= now + membership::MAX_CREDENTIAL_LIFETIME_SECS)
    {
        return Response::invalid_params("expires_at_secs is not a valid future expiry");
    }

    for c in [CREDENTIALS, REVOCATIONS, DECISIONS] {
        if let Err(e) = ensure_coll(host, c, &standing::issued_record_indexes()).await {
            return Response::internal_error(e);
        }
    }

    let supersedes = match current_credential_id(host, &member_did).await {
        Ok(id) => id,
        Err(e) => return Response::internal_error(e),
    };
    let (envelope, record_id) = match sign_as_synorg(
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
        Err(resp) => return resp,
    };

    let row = IssuedRecordRow {
        record_id: record_id.clone(),
        member_did: member_did.clone(),
        about: String::new(),
        issued_at_secs: now,
        envelope: envelope.clone(),
    };
    if let Err(e) = put_json(host, CREDENTIALS, &record_id, &row).await {
        return Response::internal_error(e);
    }
    let note = req.params.get("note").and_then(Value::as_str).unwrap_or_default();
    if let Err(e) = upsert_member(host, &member_did, note, now).await {
        return Response::internal_error(e);
    }
    if let Err(e) = standing::rebuild_for(host, &member_did).await {
        return Response::internal_error(e);
    }
    Response::ok(json!({ "record_id": record_id, "envelope": envelope }))
}

async fn current_credential_id<H: AppHost>(
    host: &H,
    member_did: &str,
) -> Result<Option<String>, String> {
    let mut rows: Vec<IssuedRecordRow> =
        collect_raw_where(host, CREDENTIALS, &json!({ "member_did": member_did }))
            .await?
            .into_iter()
            .filter_map(|(_, v)| serde_json::from_value(v).ok())
            .collect();
    rows.sort_by(|a, b| {
        b.issued_at_secs.cmp(&a.issued_at_secs).then(a.record_id.cmp(&b.record_id))
    });
    Ok(rows.into_iter().next().map(|r| r.record_id))
}

pub(in crate::app) async fn revoke<H: AppHost>(host: &H, req: &Request) -> Response {
    let credential_record_id = match req.params.get("credential_record_id").and_then(Value::as_str)
    {
        Some(c) if !c.is_empty() => c.to_string(),
        _ => return Response::invalid_params("credential_record_id is required"),
    };
    let reason = req.params.get("reason").and_then(Value::as_str).unwrap_or_default().to_string();

    for c in [CREDENTIALS, REVOCATIONS] {
        if let Err(e) = ensure_coll(host, c, &standing::issued_record_indexes()).await {
            return Response::internal_error(e);
        }
    }
    let row: Option<IssuedRecordRow> =
        match get_json(host, CREDENTIALS, &credential_record_id).await {
            Ok(v) => v,
            Err(e) => return Response::internal_error(e),
        };
    let Some(row) = row else {
        return Response::invalid_params("no credential with that id was issued here");
    };

    let existing: Vec<IssuedRecordRow> =
        match collect_raw_where(host, REVOCATIONS, &json!({ "about": credential_record_id })).await
        {
            Ok(v) => v.into_iter().filter_map(|(_, v)| serde_json::from_value(v).ok()).collect(),
            Err(e) => return Response::internal_error(e),
        };
    if let Some(existing) = existing.into_iter().next() {
        return Response::ok(
            json!({ "record_id": existing.record_id, "envelope": existing.envelope }),
        );
    }

    let payload = membership::RevocationPayload {
        credential_record_id: credential_record_id.clone(),
        member_did: row.member_did.clone(),
        reason,
    };
    if let Err(e) = payload.validate() {
        return Response::invalid_params(e.to_string());
    }
    let (envelope, record_id) = match sign_as_synorg(
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
        Err(resp) => return resp,
    };
    let now = clock::now_secs();
    let out_row = IssuedRecordRow {
        record_id: record_id.clone(),
        member_did: row.member_did.clone(),
        about: credential_record_id,
        issued_at_secs: now,
        envelope: envelope.clone(),
    };
    if let Err(e) = put_json(host, REVOCATIONS, &record_id, &out_row).await {
        return Response::internal_error(e);
    }
    if let Err(e) = standing::rebuild_for(host, &row.member_did).await {
        return Response::internal_error(e);
    }
    Response::ok(json!({ "record_id": record_id, "envelope": envelope }))
}

async fn list_issued<H: AppHost>(host: &H, collection: &str, req: &Request) -> Response {
    if let Err(e) = ensure_coll(host, collection, &standing::issued_record_indexes()).await {
        return Response::internal_error(e);
    }
    let member_did = req.params.get("member_did").and_then(Value::as_str);
    let rows = match member_did {
        Some(m) => collect_raw_where(host, collection, &json!({ "member_did": m })).await,
        None => collect_raw(host, collection).await,
    };
    match rows {
        Ok(v) => {
            Response::ok(json!({ "records": v.into_iter().map(|(_, v)| v).collect::<Vec<_>>() }))
        }
        Err(e) => Response::internal_error(e),
    }
}

pub(in crate::app) async fn list<H: AppHost>(host: &H, req: &Request) -> Response {
    list_issued(host, CREDENTIALS, req).await
}

pub(in crate::app) async fn list_revocations<H: AppHost>(host: &H, req: &Request) -> Response {
    list_issued(host, REVOCATIONS, req).await
}
