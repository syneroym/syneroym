//! Client half: the membership checks this person has made against their
//! sources, cached and re-evaluated on read.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use syneroym_app_host::{
    AppHost,
    types::proxy::{CallOptions, CallTarget},
};
use syneroym_roym_core::{
    clock,
    directory::DEFAULT_SOURCE_TIMEOUT_MS,
    envelope::{Request, Response},
    membership::{self, CheckInput, MembershipEvidence, MembershipVerdict},
    services,
};

use super::{
    HELD_MEMBERSHIPS, SOURCES, client_sources::SourceRow, collect_raw_where, ensure_coll, get_json,
    put_json,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::app) struct HeldMembershipRow {
    pub(in crate::app) source: String,
    pub(in crate::app) member_did: String,
    pub(in crate::app) issuer_did: Option<String>,
    pub(in crate::app) evidence: MembershipEvidence,
    /// This node's own clock when `evidence` was fetched.
    pub(in crate::app) as_of_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(in crate::app) last_error: Option<String>,
}

fn held_key(source: &str, member_did: &str) -> String {
    format!("{source}#{member_did}")
}

/// Upserts the held copy for `(source, member_did)`. Best-effort: a
/// caller that cannot afford to lose the search hit it came from ignores
/// this call's own error. Skipped when `evidence` is empty and no row
/// exists yet -- a non-member produces no cached copy.
pub(in crate::app) async fn remember<H: AppHost>(
    host: &H,
    source: &str,
    member_did: &str,
    issuer_did: Option<&str>,
    evidence: &MembershipEvidence,
    now: u64,
) -> Result<(), String> {
    ensure_coll(host, HELD_MEMBERSHIPS, &[]).await?;
    let key = held_key(source, member_did);
    let is_empty = evidence.credentials.is_empty()
        && evidence.revocations.is_empty()
        && evidence.decisions.is_empty();
    if is_empty && get_json::<H, HeldMembershipRow>(host, HELD_MEMBERSHIPS, &key).await?.is_none() {
        return Ok(());
    }
    let row = HeldMembershipRow {
        source: source.to_string(),
        member_did: member_did.to_string(),
        issuer_did: issuer_did.map(str::to_string),
        evidence: evidence.clone(),
        as_of_secs: now,
        last_error: None,
    };
    put_json(host, HELD_MEMBERSHIPS, &key, &row).await
}

pub(in crate::app) async fn check_standing<H: AppHost>(host: &H, req: &Request) -> Response {
    let source = match req.params.get("source").and_then(Value::as_str) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return Response::invalid_params("source is required"),
    };
    let member_did = match req.params.get("member_did").and_then(Value::as_str) {
        Some(m) if !m.is_empty() => m.to_string(),
        _ => return Response::invalid_params("member_did is required"),
    };
    if let Err(e) = ensure_coll(host, SOURCES, &[]).await {
        return Response::internal_error(e);
    }
    let source_row: Option<SourceRow> = match get_json(host, SOURCES, &source).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let Some(source_row) = source_row else {
        return Response::invalid_params("source is not in this person's own sources");
    };

    let params = json!({ "method": "directory.standing", "params": { "member_did": member_did } })
        .to_string();
    let call_result = host
        .call(
            CallTarget::Service(source.clone()),
            services::DIRECTORY.interface.to_string(),
            "invoke".to_string(),
            json!([params]).to_string(),
            Some(CallOptions {
                protocol: None,
                idempotent: true,
                timeout_ms: Some(DEFAULT_SOURCE_TIMEOUT_MS),
                routing_key: None,
                idempotency_key: None,
            }),
        )
        .await;

    handle_standing_reply(host, &source, &member_did, source_row.issuer_did.as_deref(), call_result)
        .await
}

async fn handle_standing_reply<H: AppHost>(
    host: &H,
    source: &str,
    member_did: &str,
    pinned_issuer: Option<&str>,
    call_result: Result<String, syneroym_app_host::types::proxy::ProxyError>,
) -> Response {
    let raw = match call_result {
        Ok(r) => r,
        Err(e) => {
            return stale_or_unknown(host, source, member_did, pinned_issuer, format!("{e:?}"))
                .await;
        }
    };
    let resp: Response = match serde_json::from_str(&raw) {
        Ok(r) => r,
        Err(e) => {
            return stale_or_unknown(host, source, member_did, pinned_issuer, e.to_string()).await;
        }
    };
    let Some(result) = resp.result else {
        let msg = resp.error.map(|e| e.message).unwrap_or_default();
        return stale_or_unknown(host, source, member_did, pinned_issuer, msg).await;
    };
    let reply_issuer = result.get("issuer_did").and_then(Value::as_str).map(str::to_string);
    if let (Some(pinned), Some(claimed)) = (pinned_issuer, reply_issuer.as_deref())
        && pinned != claimed
    {
        return Response::ok(json!({
            "verdict": MembershipVerdict::Unknown { reason: "issuer-changed".to_string() },
            "refreshed": false,
        }));
    }
    let evidence: MembershipEvidence =
        match serde_json::from_value(result.get("evidence").cloned().unwrap_or(json!({}))) {
            Ok(e) => e,
            Err(e) => {
                return stale_or_unknown(host, source, member_did, pinned_issuer, e.to_string())
                    .await;
            }
        };
    let now = clock::now_secs();
    let _ = remember(host, source, member_did, pinned_issuer, &evidence, now).await;
    let verdict = membership::evaluate(
        &evidence,
        &CheckInput {
            pinned_issuer,
            member_did,
            listing: None,
            now_secs: now,
            evidence_as_of_secs: now,
        },
    );
    Response::ok(json!({ "verdict": verdict, "as_of_secs": now, "refreshed": true }))
}

/// The far end could not be reached or answered something this node
/// cannot use. Falls back to the stored evidence (if any) so the caller
/// still gets an honest, dated verdict instead of a bare error.
async fn stale_or_unknown<H: AppHost>(
    host: &H,
    source: &str,
    member_did: &str,
    pinned_issuer: Option<&str>,
    error: String,
) -> Response {
    let key = held_key(source, member_did);
    let _ = ensure_coll(host, HELD_MEMBERSHIPS, &[]).await;
    let stored: Option<HeldMembershipRow> =
        get_json(host, HELD_MEMBERSHIPS, &key).await.ok().flatten();
    let mut row = stored.unwrap_or(HeldMembershipRow {
        source: source.to_string(),
        member_did: member_did.to_string(),
        issuer_did: pinned_issuer.map(str::to_string),
        evidence: MembershipEvidence::default(),
        as_of_secs: 0,
        last_error: None,
    });
    row.last_error = Some(error.clone());
    let _ = put_json(host, HELD_MEMBERSHIPS, &key, &row).await;
    let verdict = if row.as_of_secs == 0 {
        MembershipVerdict::Unknown { reason: "could not reach this directory".to_string() }
    } else {
        membership::evaluate(
            &row.evidence,
            &CheckInput {
                pinned_issuer,
                member_did,
                listing: None,
                now_secs: row.as_of_secs,
                evidence_as_of_secs: row.as_of_secs,
            },
        )
    };
    Response::ok(json!({
        "verdict": verdict,
        "as_of_secs": row.as_of_secs,
        "refreshed": false,
        "error": error,
    }))
}

/// `directory.memberships`, params `{ member_did? }`: the held copies,
/// each re-evaluated now from its stored evidence -- never a stored
/// verdict, so a restore reproduces the same answer by construction.
pub(in crate::app) async fn memberships<H: AppHost>(host: &H, req: &Request) -> Response {
    if let Err(e) = ensure_coll(host, HELD_MEMBERSHIPS, &[]).await {
        return Response::internal_error(e);
    }
    let filter_member = req.params.get("member_did").and_then(Value::as_str);
    let rows: Vec<HeldMembershipRow> = match filter_member {
        Some(m) => match collect_raw_where(host, HELD_MEMBERSHIPS, &json!({ "member_did": m }))
            .await
        {
            Ok(v) => v.into_iter().filter_map(|(_, v)| serde_json::from_value(v).ok()).collect(),
            Err(e) => return Response::internal_error(e),
        },
        None => match collect_raw_where(host, HELD_MEMBERSHIPS, &json!({})).await {
            Ok(v) => v.into_iter().filter_map(|(_, v)| serde_json::from_value(v).ok()).collect(),
            Err(e) => return Response::internal_error(e),
        },
    };
    let now = clock::now_secs();
    let out: Vec<Value> = rows
        .into_iter()
        .map(|row| {
            let verdict = membership::evaluate(
                &row.evidence,
                &CheckInput {
                    pinned_issuer: row.issuer_did.as_deref(),
                    member_did: &row.member_did,
                    listing: None,
                    now_secs: now,
                    evidence_as_of_secs: row.as_of_secs,
                },
            );
            json!({
                "source": row.source,
                "member_did": row.member_did,
                "issuer_did": row.issuer_did,
                "as_of_secs": row.as_of_secs,
                "last_error": row.last_error,
                "verdict": verdict,
            })
        })
        .collect();
    Response::ok(json!({ "memberships": out }))
}
