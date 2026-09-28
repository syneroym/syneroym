//! Server half: one member's derived standing -- the evidence bytes a
//! consumer would receive for them, rebuilt whenever one of their signed
//! records changes -- and the two ways the directory judges it itself
//! (`directory.standing`'s answer, and its own membership verdict on a
//! publish or search).

use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::app) struct IssuedRecordRow {
    pub(in crate::app) record_id: String,
    pub(in crate::app) member_did: String,
    /// `credential_record_id` for a revocation; empty otherwise.
    #[serde(default)]
    pub(in crate::app) about: String,
    pub(in crate::app) issued_at_secs: u64,
    pub(in crate::app) envelope: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::app) struct StandingRow {
    pub(in crate::app) member_did: String,
    pub(in crate::app) evidence: membership::MembershipEvidence,
    pub(in crate::app) updated_at_secs: u64,
}

pub(in crate::app) fn issued_record_indexes() -> [IndexDefinition; 2] {
    [idx("member_did", IndexType::String), idx("issued_at_secs", IndexType::Numeric)]
}

/// Rebuilds `STANDING[member_did]` from the three issued-record
/// collections, then rewrites that member's `search_index` listed
/// windows so a standing change and the index it drives land in
/// the same call.
pub(in crate::app) async fn rebuild_for<H: AppHost>(
    host: &H,
    member_did: &str,
) -> Result<(), String> {
    for c in [CREDENTIALS, REVOCATIONS, DECISIONS] {
        ensure_coll(host, c, &issued_record_indexes()).await?;
    }
    ensure_coll(host, STANDING, &[]).await?;

    let mut creds: Vec<IssuedRecordRow> =
        collect_raw_where(host, CREDENTIALS, &json!({ "member_did": member_did }))
            .await?
            .into_iter()
            .filter_map(|(_, v)| serde_json::from_value(v).ok())
            .collect();
    creds.sort_by(|a, b| {
        b.issued_at_secs.cmp(&a.issued_at_secs).then(a.record_id.cmp(&b.record_id))
    });
    creds.truncate(membership::MAX_EVIDENCE_CREDENTIALS);
    let cred_ids: std::collections::BTreeSet<String> =
        creds.iter().map(|c| c.record_id.clone()).collect();

    let revs: Vec<IssuedRecordRow> =
        collect_raw_where(host, REVOCATIONS, &json!({ "member_did": member_did }))
            .await?
            .into_iter()
            .filter_map(|(_, v)| serde_json::from_value(v).ok())
            .filter(|r: &IssuedRecordRow| cred_ids.contains(&r.about))
            .collect();

    let mut decs: Vec<IssuedRecordRow> =
        collect_raw_where(host, DECISIONS, &json!({ "member_did": member_did }))
            .await?
            .into_iter()
            .filter_map(|(_, v)| serde_json::from_value(v).ok())
            .collect();
    decs.sort_by(|a, b| {
        b.issued_at_secs.cmp(&a.issued_at_secs).then(a.record_id.cmp(&b.record_id))
    });
    decs.truncate(membership::MAX_EVIDENCE_DECISIONS);

    let now = clock::now_secs();
    if creds.is_empty() && revs.is_empty() && decs.is_empty() {
        AppDataLayer::delete(host, STANDING.to_string(), member_did.to_string())
            .await
            .map_err(|e| e.to_string())?;
    } else {
        let evidence = membership::MembershipEvidence {
            credentials: creds.into_iter().map(|c| c.envelope).collect(),
            revocations: revs.into_iter().map(|r| r.envelope).collect(),
            decisions: decs.into_iter().map(|d| d.envelope).collect(),
        };
        let row = StandingRow {
            member_did: member_did.to_string(),
            evidence: evidence.clone(),
            updated_at_secs: now,
        };
        put_json(host, STANDING, member_did, &row).await?;
    }

    let evidence = load(host, member_did).await?;
    search_ops::rewrite_listed_windows(host, member_did, &evidence, now).await
}

/// Rebuilds every member's standing -- called from `import` and
/// `directory.reindex`, always **before** `search_ops::rebuild_search_index`
/// (the index rows read the standing to compute their window).
pub(in crate::app) async fn rebuild_all<H: AppHost>(host: &H) -> Result<u64, String> {
    for c in [CREDENTIALS, REVOCATIONS, DECISIONS] {
        ensure_coll(host, c, &issued_record_indexes()).await?;
    }
    AppDataLayer::delete_many(host, STANDING.to_string(), json!({}).to_string())
        .await
        .map_err(|e| e.to_string())?;

    let mut members: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for c in [CREDENTIALS, REVOCATIONS, DECISIONS] {
        for (_, v) in collect_raw(host, c).await? {
            if let Some(m) = v.get("member_did").and_then(Value::as_str) {
                members.insert(m.to_string());
            }
        }
    }
    let count = members.len() as u64;
    for m in members {
        rebuild_for(host, &m).await?;
    }
    Ok(count)
}

pub(in crate::app) async fn load<H: AppHost>(
    host: &H,
    member_did: &str,
) -> Result<membership::MembershipEvidence, String> {
    ensure_coll(host, STANDING, &[]).await?;
    Ok(get_json::<H, StandingRow>(host, STANDING, member_did)
        .await?
        .map(|r| r.evidence)
        .unwrap_or_default())
}

pub(in crate::app) async fn standing_verb<H: AppHost>(host: &H, req: &Request) -> Response {
    let member_did = match req.params.get("member_did").and_then(Value::as_str) {
        Some(m) if !m.is_empty() => m.to_string(),
        _ => return Response::invalid_params("member_did is required"),
    };
    let issuer = issuer_did(host).await;
    let evidence = match load(host, &member_did).await {
        Ok(e) => e,
        Err(e) => return Response::internal_error(e),
    };
    Response::ok(json!({
        "issuer_did": issuer,
        "member_did": member_did,
        "evidence": evidence,
        "answered_at_secs": clock::now_secs(),
    }))
}

/// The directory judging its own member: the owner is the
/// issuer.
pub(in crate::app) async fn own_verdict<H: AppHost>(
    host: &H,
    member_did: &str,
    listing: Option<ListingRef<'_>>,
    now: u64,
) -> Result<MembershipVerdict, String> {
    let Some(issuer) = issuer_did(host).await else {
        return Ok(MembershipVerdict::Unknown { reason: "issuer-not-pinned".to_string() });
    };
    let evidence = load(host, member_did).await?;
    Ok(membership::evaluate(
        &evidence,
        &CheckInput {
            pinned_issuer: Some(&issuer),
            member_did,
            listing,
            now_secs: now,
            evidence_as_of_secs: now,
        },
    ))
}

/// `load` + `membership::listed_window` with the own issuer; `(0, 0)`
/// when this node has no owner.
pub(in crate::app) async fn listed_window_for<H: AppHost>(
    host: &H,
    member_did: &str,
    listing_id: &str,
    now: u64,
) -> Result<(u64, u64), String> {
    let Some(issuer) = issuer_did(host).await else {
        return Ok((0, 0));
    };
    let evidence = load(host, member_did).await?;
    Ok(membership::listed_window(&evidence, &issuer, member_did, listing_id, now))
}
