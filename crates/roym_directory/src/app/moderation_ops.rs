//! Server half: `member.suspend` / `member.lift` / `member.decisions` --
//! a SynOrg's signed moderation decisions about its own members (D-C9-5).

use super::*;
use crate::app::{credential_ops::sign_as_synorg, standing::IssuedRecordRow};

fn parse_scope(req: &Request) -> Result<membership::ModerationScope, Response> {
    match req.params.get("scope") {
        None | Some(Value::Null) => Ok(membership::ModerationScope::Membership),
        Some(v) => serde_json::from_value(v.clone())
            .map_err(|e| Response::invalid_params(format!("invalid scope: {e}"))),
    }
}

pub(in crate::app) async fn suspend<H: AppHost>(host: &H, req: &Request) -> Response {
    let member_did = match req.params.get("member_did").and_then(Value::as_str) {
        Some(m) if !m.is_empty() => m.to_string(),
        _ => return Response::invalid_params("member_did is required"),
    };
    let rule = req.params.get("rule").and_then(Value::as_str).unwrap_or_default().to_string();
    let reason = req.params.get("reason").and_then(Value::as_str).unwrap_or_default().to_string();
    let until_secs = req.params.get("until_secs").and_then(Value::as_u64);
    let scope = match parse_scope(req) {
        Ok(s) => s,
        Err(resp) => return resp,
    };

    if let Err(e) = ensure_coll(host, MEMBERS, &[]).await {
        return Response::internal_error(e);
    }
    let is_member: Option<directory::Member> = match get_json(host, MEMBERS, &member_did).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    if is_member.is_none() {
        return Response::invalid_params("not a member of this SynOrg");
    }

    let now = clock::now_secs();
    let payload = membership::ModerationDecisionPayload {
        action: membership::ModerationAction::Suspend,
        member_did: member_did.clone(),
        scope,
        rule,
        reason,
        until_secs,
    };
    if let Err(e) = payload.validate(now) {
        return Response::invalid_params(e.to_string());
    }

    for c in [CREDENTIALS, DECISIONS] {
        if let Err(e) = ensure_coll(host, c, &standing::issued_record_indexes()).await {
            return Response::internal_error(e);
        }
    }
    let (envelope, record_id) = match sign_as_synorg(
        host,
        record::RECORD_MODERATION_DECISION,
        membership::MODERATION_DECISION_VERSION,
        &member_did,
        &payload,
        None,
        None,
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
    if let Err(e) = put_json(host, DECISIONS, &record_id, &row).await {
        return Response::internal_error(e);
    }
    if let Err(e) = standing::rebuild_for(host, &member_did).await {
        return Response::internal_error(e);
    }
    Response::ok(json!({ "record_id": record_id, "envelope": envelope }))
}

pub(in crate::app) async fn lift<H: AppHost>(host: &H, req: &Request) -> Response {
    let decision_record_id = match req.params.get("decision_record_id").and_then(Value::as_str) {
        Some(d) if !d.is_empty() => d.to_string(),
        _ => return Response::invalid_params("decision_record_id is required"),
    };
    let reason = req.params.get("reason").and_then(Value::as_str).unwrap_or_default().to_string();

    if let Err(e) = ensure_coll(host, DECISIONS, &standing::issued_record_indexes()).await {
        return Response::internal_error(e);
    }
    let row: Option<IssuedRecordRow> = match get_json(host, DECISIONS, &decision_record_id).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let Some(row) = row else {
        return Response::invalid_params("no decision with that id was issued here");
    };

    let now = clock::now_secs();
    let owner = match signing::owner_did(host).await {
        Ok(o) => o,
        Err(e) => return Response::internal_error(e.to_string()),
    };
    // Scoped so the non-`Send` `VerifyOptions` (it carries `&dyn
    // RevocationSource`) does not live across a later `.await`.
    let payload_value = {
        let opts = record::VerifyOptions::new(now).expecting(&owner);
        match record::verify_json(&row.envelope, &opts) {
            Ok(v) => v.payload,
            Err(e) => return Response::internal_error(e.to_string()),
        }
    };
    let payload: membership::ModerationDecisionPayload = match serde_json::from_value(payload_value)
    {
        Ok(p) => p,
        Err(e) => return Response::internal_error(e.to_string()),
    };
    if payload.action != membership::ModerationAction::Suspend {
        return Response::invalid_params("that decision is not a suspension");
    }

    let existing: Vec<IssuedRecordRow> =
        match collect_raw_where(host, DECISIONS, &json!({ "about": decision_record_id })).await {
            Ok(v) => v.into_iter().filter_map(|(_, v)| serde_json::from_value(v).ok()).collect(),
            Err(e) => return Response::internal_error(e),
        };
    if let Some(existing) = existing.into_iter().next() {
        return Response::ok(
            json!({ "record_id": existing.record_id, "envelope": existing.envelope }),
        );
    }

    let lift_payload = membership::ModerationDecisionPayload {
        action: membership::ModerationAction::Lift,
        member_did: row.member_did.clone(),
        scope: payload.scope,
        rule: String::new(),
        reason,
        until_secs: None,
    };
    let (envelope, record_id) = match sign_as_synorg(
        host,
        record::RECORD_MODERATION_DECISION,
        membership::MODERATION_DECISION_VERSION,
        &row.member_did,
        &lift_payload,
        None,
        Some(decision_record_id.clone()),
    )
    .await
    {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let out_row = IssuedRecordRow {
        record_id: record_id.clone(),
        member_did: row.member_did.clone(),
        about: decision_record_id,
        issued_at_secs: now,
        envelope: envelope.clone(),
    };
    if let Err(e) = put_json(host, DECISIONS, &record_id, &out_row).await {
        return Response::internal_error(e);
    }
    if let Err(e) = standing::rebuild_for(host, &row.member_did).await {
        return Response::internal_error(e);
    }
    Response::ok(json!({ "record_id": record_id, "envelope": envelope }))
}

pub(in crate::app) async fn list<H: AppHost>(host: &H, req: &Request) -> Response {
    if let Err(e) = ensure_coll(host, DECISIONS, &standing::issued_record_indexes()).await {
        return Response::internal_error(e);
    }
    let member_did = req.params.get("member_did").and_then(Value::as_str);
    let rows = match member_did {
        Some(m) => collect_raw_where(host, DECISIONS, &json!({ "member_did": m })).await,
        None => collect_raw(host, DECISIONS).await,
    };
    match rows {
        Ok(v) => {
            Response::ok(json!({ "records": v.into_iter().map(|(_, v)| v).collect::<Vec<_>>() }))
        }
        Err(e) => Response::internal_error(e),
    }
}
