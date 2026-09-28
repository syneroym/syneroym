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
    /// `credential_record_id` for a revocation, the superseded decision's
    /// `record_id` for a lift; empty otherwise (a credential, or a suspend
    /// decision).
    #[serde(default)]
    pub(in crate::app) about: String,
    pub(in crate::app) issued_at_secs: u64,
    /// A suspend decision's `until_secs` (`None` = until lifted). Absent
    /// for every other record type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(in crate::app) until_secs: Option<u64>,
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

/// Picks which decisions ride in a member's standing bytes, bounded by
/// `MAX_EVIDENCE_DECISIONS`. Every unlifted suspension that is still
/// active (no `until_secs`, or `until_secs` still ahead of `now`) is
/// always kept, however old -- age-based truncation alone could drop it
/// from the cap while it still governs the member, so a withdrawn member
/// would look valid again with nobody having signed a lift. The
/// remaining slots go to the newest history; a suspend and its lift are
/// always kept or dropped together, so a lift never rides without the
/// decision it lifts.
fn select_evidence_decisions(all: &[IssuedRecordRow], now: u64) -> Vec<IssuedRecordRow> {
    let lifts_by_target: std::collections::BTreeMap<&str, &IssuedRecordRow> =
        all.iter().filter(|d| !d.about.is_empty()).map(|d| (d.about.as_str(), d)).collect();

    struct Group<'a> {
        suspend: &'a IssuedRecordRow,
        lift: Option<&'a IssuedRecordRow>,
        active: bool,
        recency: u64,
    }
    let mut groups: Vec<Group<'_>> = all
        .iter()
        .filter(|d| d.about.is_empty())
        .map(|s| {
            let lift = lifts_by_target.get(s.record_id.as_str()).copied();
            let active = lift.is_none() && s.until_secs.is_none_or(|u| now < u);
            let recency = lift.map_or(s.issued_at_secs, |l| l.issued_at_secs.max(s.issued_at_secs));
            Group { suspend: s, lift, active, recency }
        })
        .collect();
    groups.sort_by(|a, b| {
        b.recency.cmp(&a.recency).then(a.suspend.record_id.cmp(&b.suspend.record_id))
    });

    let mut selected: Vec<IssuedRecordRow> = Vec::new();
    for g in groups.iter().filter(|g| g.active) {
        selected.push(g.suspend.clone());
    }
    let mut remaining = membership::MAX_EVIDENCE_DECISIONS.saturating_sub(selected.len());
    for g in groups.iter().filter(|g| !g.active) {
        if remaining == 0 {
            break;
        }
        let cost = usize::from(g.lift.is_some()) + 1;
        if cost > remaining {
            continue;
        }
        selected.push(g.suspend.clone());
        if let Some(l) = g.lift {
            selected.push(l.clone());
        }
        remaining -= cost;
    }
    selected.sort_by(|a, b| {
        b.issued_at_secs.cmp(&a.issued_at_secs).then(a.record_id.cmp(&b.record_id))
    });
    selected
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
    let now = clock::now_secs();

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

    let all_decs: Vec<IssuedRecordRow> =
        collect_raw_where(host, DECISIONS, &json!({ "member_did": member_did }))
            .await?
            .into_iter()
            .filter_map(|(_, v)| serde_json::from_value(v).ok())
            .collect();
    let decs = select_evidence_decisions(&all_decs, now);

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
    ensure_coll(host, STANDING, &[]).await?;
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
        Some(m) if person::is_did_key(m) => m.to_string(),
        Some(_) => return Response::invalid_params("member_did must be a did:key"),
        None => return Response::invalid_params("member_did is required"),
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
