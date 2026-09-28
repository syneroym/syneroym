//! A SynOrg's signed statements about one member -- credential,
//! revocation, moderation decision -- and the one function that turns a
//! bundle of them into a verdict. The directory uses it on its own
//! records; a consumer uses it on records it received. Neither ever takes
//! the other's verdict.

use serde::{Deserialize, Serialize};

use crate::{
    area::{Area, AreaError, areas_intersect, labels_match},
    directory, person, record,
};

pub const MEMBERSHIP_CREDENTIAL_VERSION: u32 = 1;
pub const REVOCATION_VERSION: u32 = 1;
pub const MODERATION_DECISION_VERSION: u32 = 1;

pub const MAX_CREDENTIAL_LIFETIME_SECS: u64 = 2 * 365 * 24 * 3600;
pub const MAX_REASON_LEN: usize = 2048;
pub const MAX_RULE_LEN: usize = 256;
/// Evidence one member's standing carries. Bounded because
/// `directory.standing` and every search hit are anonymous-reachable.
pub const MAX_EVIDENCE_CREDENTIALS: usize = 4;
pub const MAX_EVIDENCE_DECISIONS: usize = 8;

/// Shown verbatim by the Hub (and mirrored in
/// `crates/roym_web/ui/src/directory/membership.ts`; a test in this
/// module reads that file and compares).
pub const NO_INSTANT_REMOVAL_NOTICE: &str = "A group's decision reaches copies other people \
                                             already hold only when they next check. Nobody can \
                                             promise it is removed everywhere at once.";
pub const WITHHELD_REVOCATION_NOTICE: &str = "This shows every withdrawal the group has \
                                              published. It cannot show one the group chose not \
                                              to publish.";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MembershipScope {
    /// 1..=`directory::MAX_CATEGORIES`, each normalized with
    /// `directory::normalize_category`. Never empty: a credential always
    /// names what it covers.
    pub categories: Vec<String>,
    /// Empty = no area restriction. At most `directory::MAX_AREAS`.
    #[serde(default)]
    pub area: Vec<Area>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MembershipCredentialPayload {
    pub synorg_name: String,
    pub member_did: String,
    pub scope: MembershipScope,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RevocationPayload {
    pub credential_record_id: String,
    pub member_did: String,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModerationAction {
    Suspend,
    Lift,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ModerationScope {
    Membership,
    Listing { listing_id: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModerationDecisionPayload {
    pub action: ModerationAction,
    pub member_did: String,
    pub scope: ModerationScope,
    /// Which of the SynOrg's own rules. Required for `suspend`, empty for
    /// `lift`.
    pub rule: String,
    pub reason: String,
    /// `suspend` only. Absent = until lifted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until_secs: Option<u64>,
}

/// The signed bytes, exactly as the issuer produced them. Every field is
/// envelope JSON strings; nothing here is trusted until `evaluate` says so.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MembershipEvidence {
    #[serde(default)]
    pub credentials: Vec<String>,
    #[serde(default)]
    pub revocations: Vec<String>,
    #[serde(default)]
    pub decisions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum MembershipVerdict {
    /// The directory presented no credential for this member.
    None,
    Valid {
        credential_record_id: String,
        issuer: String,
        synorg_name: String,
        scope: MembershipScope,
        expires_at_secs: u64,
        /// When this node last fetched the issuer's withdrawals. Not a
        /// claim that none exist.
        revocations_checked_as_of_secs: u64,
    },
    Expired {
        credential_record_id: String,
        expires_at_secs: u64,
    },
    OutOfScope {
        credential_record_id: String,
        outside: Vec<String>,
    },
    Revoked {
        credential_record_id: String,
        revocation_record_id: String,
        revoked_at_secs: u64,
        reason: String,
    },
    Suspended {
        decision_record_id: String,
        scope: ModerationScope,
        rule: String,
        reason: String,
        since_secs: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        until_secs: Option<u64>,
    },
    /// Evidence was presented and did not check out on this node.
    Refused {
        reason: String,
    },
    /// This node could not check at all.
    Unknown {
        reason: String,
    },
}

/// What is being judged. `categories`/`areas` are `None` when the caller
/// does not judge scope (the directory's search filter, and a
/// listing-free membership check).
#[derive(Debug, Clone, Copy)]
pub struct ListingRef<'a> {
    pub listing_id: &'a str,
    pub categories: Option<&'a [String]>,
    pub areas: Option<&'a [Area]>,
}

#[derive(Debug, Clone, Copy)]
pub struct CheckInput<'a> {
    pub pinned_issuer: Option<&'a str>,
    pub member_did: &'a str,
    pub listing: Option<ListingRef<'a>>,
    pub now_secs: u64,
    pub evidence_as_of_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MembershipError {
    #[error("member_did '{0}' is not a did:key")]
    NotDidKey(String),
    #[error("scope carries no categories")]
    NoCategories,
    #[error("more than {max} categories", max = directory::MAX_CATEGORIES)]
    TooManyCategories,
    #[error("category '{0}' is not 1..={max} bytes", max = directory::MAX_CATEGORY_LEN)]
    CategoryShape(String),
    #[error("more than {max} areas", max = crate::area::MAX_AREAS)]
    TooManyAreas,
    #[error("area: {0}")]
    Area(#[from] AreaError),
    #[error("scope names a category '{0}' this SynOrg does not itself list")]
    CategoryOutsideSynOrg(String),
    #[error("expires_at_secs is not in (now, now + {MAX_CREDENTIAL_LIFETIME_SECS}]")]
    ExpiryOutOfBounds,
    #[error("reason is longer than {MAX_REASON_LEN} bytes")]
    ReasonTooLong,
    #[error("rule is not 1..={MAX_RULE_LEN} bytes for suspend, or non-empty for lift")]
    RuleShape,
    #[error("until_secs is not after now_secs")]
    UntilInPast,
}

impl MembershipScope {
    pub fn validate(&self) -> Result<(), MembershipError> {
        if self.categories.is_empty() {
            return Err(MembershipError::NoCategories);
        }
        if self.categories.len() > directory::MAX_CATEGORIES {
            return Err(MembershipError::TooManyCategories);
        }
        for c in &self.categories {
            let normalized = directory::normalize_category(c);
            if &normalized != c || c.is_empty() || c.len() > directory::MAX_CATEGORY_LEN {
                return Err(MembershipError::CategoryShape(c.clone()));
            }
        }
        if self.area.len() > crate::area::MAX_AREAS {
            return Err(MembershipError::TooManyAreas);
        }
        for a in &self.area {
            a.validate()?;
        }
        Ok(())
    }
}

impl MembershipCredentialPayload {
    pub fn validate(&self) -> Result<(), MembershipError> {
        if !person::is_did_key(&self.member_did) {
            return Err(MembershipError::NotDidKey(self.member_did.clone()));
        }
        self.scope.validate()
    }
}

impl RevocationPayload {
    pub fn validate(&self) -> Result<(), MembershipError> {
        if !person::is_did_key(&self.member_did) {
            return Err(MembershipError::NotDidKey(self.member_did.clone()));
        }
        if self.reason.len() > MAX_REASON_LEN {
            return Err(MembershipError::ReasonTooLong);
        }
        Ok(())
    }
}

impl ModerationDecisionPayload {
    pub fn validate(&self, now_secs: u64) -> Result<(), MembershipError> {
        if !person::is_did_key(&self.member_did) {
            return Err(MembershipError::NotDidKey(self.member_did.clone()));
        }
        if self.reason.len() > MAX_REASON_LEN {
            return Err(MembershipError::ReasonTooLong);
        }
        match self.action {
            ModerationAction::Suspend => {
                if self.rule.is_empty() || self.rule.len() > MAX_RULE_LEN {
                    return Err(MembershipError::RuleShape);
                }
                if let Some(u) = self.until_secs
                    && u <= now_secs
                {
                    return Err(MembershipError::UntilInPast);
                }
            }
            ModerationAction::Lift => {
                if !self.rule.is_empty() {
                    return Err(MembershipError::RuleShape);
                }
            }
        }
        Ok(())
    }
}

/// A credential whose signature, delegation, issuer and shape all
/// checked out. Not yet known to be current (see `pick_current`), not yet
/// known to be unrevoked or unexpired.
struct VerifiedCredential {
    record_id: String,
    supersedes: Option<String>,
    issued_at_secs: u64,
    expires_at_secs: u64,
    payload: MembershipCredentialPayload,
    issuer: String,
}

struct VerifiedRevocation {
    record_id: String,
    issued_at_secs: u64,
    payload: RevocationPayload,
}

struct VerifiedDecision {
    record_id: String,
    supersedes: Option<String>,
    issued_at_secs: u64,
    payload: ModerationDecisionPayload,
}

fn verify_credential(
    env: &str,
    issuer: &str,
    member_did: &str,
    now_secs: u64,
) -> Result<VerifiedCredential, String> {
    let opts = record::VerifyOptions::new(now_secs).expecting(issuer).allowing_expired();
    let v = record::verify_json(env, &opts).map_err(|e| e.to_string())?;
    if v.record_type != record::RECORD_MEMBERSHIP_CREDENTIAL
        || v.version != MEMBERSHIP_CREDENTIAL_VERSION
    {
        return Err("not a membership-credential record".to_string());
    }
    if v.subject != member_did {
        return Err("credential subject does not match".to_string());
    }
    let payload: MembershipCredentialPayload =
        serde_json::from_value(v.payload).map_err(|e| e.to_string())?;
    payload.validate().map_err(|e| e.to_string())?;
    if payload.member_did != v.subject {
        return Err("credential payload member_did does not match subject".to_string());
    }
    let expires_at_secs =
        v.expires_at_secs.ok_or_else(|| "credential carries no expiry".to_string())?;
    Ok(VerifiedCredential {
        record_id: v.record_id,
        supersedes: v.supersedes,
        issued_at_secs: v.issued_at_secs,
        expires_at_secs,
        payload,
        issuer: v.issuer,
    })
}

fn verify_revocation(env: &str, issuer: &str, now_secs: u64) -> Result<VerifiedRevocation, String> {
    let opts = record::VerifyOptions::new(now_secs).expecting(issuer);
    let v = record::verify_json(env, &opts).map_err(|e| e.to_string())?;
    if v.record_type != record::RECORD_REVOCATION || v.version != REVOCATION_VERSION {
        return Err("not a revocation record".to_string());
    }
    let payload: RevocationPayload =
        serde_json::from_value(v.payload).map_err(|e| e.to_string())?;
    if v.subject != payload.credential_record_id {
        return Err("revocation subject does not match its payload".to_string());
    }
    Ok(VerifiedRevocation { record_id: v.record_id, issued_at_secs: v.issued_at_secs, payload })
}

fn verify_decision(
    env: &str,
    issuer: &str,
    member_did: &str,
    now_secs: u64,
) -> Result<VerifiedDecision, String> {
    let opts = record::VerifyOptions::new(now_secs).expecting(issuer);
    let v = record::verify_json(env, &opts).map_err(|e| e.to_string())?;
    if v.record_type != record::RECORD_MODERATION_DECISION
        || v.version != MODERATION_DECISION_VERSION
    {
        return Err("not a moderation-decision record".to_string());
    }
    if v.subject != member_did {
        return Err("decision subject does not match".to_string());
    }
    let payload: ModerationDecisionPayload =
        serde_json::from_value(v.payload).map_err(|e| e.to_string())?;
    if payload.member_did != member_did {
        return Err("decision payload member_did does not match subject".to_string());
    }
    if payload.action == ModerationAction::Lift && v.supersedes.is_none() {
        return Err("a lift must supersede the suspension it lifts".to_string());
    }
    Ok(VerifiedDecision {
        record_id: v.record_id,
        supersedes: v.supersedes,
        issued_at_secs: v.issued_at_secs,
        payload,
    })
}

/// Drops any credential another good credential's `supersedes` names,
/// then picks the newest by `issued_at_secs`, tie-broken by the smaller
/// record id. `good` must be non-empty.
fn pick_current(good: Vec<VerifiedCredential>) -> VerifiedCredential {
    let superseded: std::collections::BTreeSet<String> =
        good.iter().filter_map(|c| c.supersedes.clone()).collect();
    let (mut current, mut dropped): (Vec<_>, Vec<_>) =
        good.into_iter().partition(|c| !superseded.contains(&c.record_id));
    if current.is_empty() {
        current.append(&mut dropped);
    }
    current.sort_by(|a, b| {
        b.issued_at_secs.cmp(&a.issued_at_secs).then_with(|| a.record_id.cmp(&b.record_id))
    });
    current.remove(0)
}

fn find_revocation(
    evidence: &MembershipEvidence,
    issuer: &str,
    cred: &VerifiedCredential,
    member_did: &str,
    now_secs: u64,
) -> Option<VerifiedRevocation> {
    evidence.revocations.iter().find_map(|env| {
        let r = verify_revocation(env, issuer, now_secs).ok()?;
        (r.payload.credential_record_id == cred.record_id && r.payload.member_did == member_did)
            .then_some(r)
    })
}

fn applies(scope: &ModerationScope, listing: Option<ListingRef<'_>>) -> bool {
    match scope {
        ModerationScope::Membership => true,
        ModerationScope::Listing { listing_id } => {
            listing.is_some_and(|l| l.listing_id == listing_id)
        }
    }
}

/// Every suspend decision that is not superseded by a lift in this same
/// evidence bundle, newest first. Whether it is still *active* (its
/// `until_secs`, if any, has not passed) is for the caller to judge.
fn suspend_decisions(
    evidence: &MembershipEvidence,
    issuer: &str,
    member_did: &str,
    now_secs: u64,
) -> Vec<VerifiedDecision> {
    let mut decisions: Vec<VerifiedDecision> = evidence
        .decisions
        .iter()
        .take(MAX_EVIDENCE_DECISIONS)
        .filter_map(|e| verify_decision(e, issuer, member_did, now_secs).ok())
        .collect();
    decisions.sort_by(|a, b| {
        b.issued_at_secs.cmp(&a.issued_at_secs).then_with(|| a.record_id.cmp(&b.record_id))
    });
    let lifted: std::collections::BTreeSet<String> = decisions
        .iter()
        .filter(|d| d.payload.action == ModerationAction::Lift)
        .filter_map(|d| d.supersedes.clone())
        .collect();
    decisions.retain(|d| {
        d.payload.action == ModerationAction::Suspend && !lifted.contains(&d.record_id)
    });
    decisions
}

fn active_suspension(
    evidence: &MembershipEvidence,
    issuer: &str,
    input: &CheckInput<'_>,
) -> Option<VerifiedDecision> {
    suspend_decisions(evidence, issuer, input.member_did, input.now_secs).into_iter().find(|d| {
        d.payload.until_secs.is_none_or(|u| input.now_secs < u)
            && applies(&d.payload.scope, input.listing)
    })
}

/// `(Named, Named)` compares labels; a `Named` paired with a geometric
/// area cannot show containment either way, so it counts as outside; two
/// geometric areas fall back to *intersects* (a listing area that pokes
/// outside the SynOrg's area still counts as in scope -- a backlog row,
/// not a bug, see the plan's "Flag for review").
fn area_within(listing_area: &Area, scope_area: &Area) -> bool {
    match (listing_area, scope_area) {
        (Area::Named { .. }, Area::Named { .. }) => labels_match(listing_area, scope_area),
        (Area::Named { .. }, _) | (_, Area::Named { .. }) => false,
        _ => areas_intersect(listing_area, scope_area) == Some(true),
    }
}

fn scope_violations(cred_scope: &MembershipScope, listing: ListingRef<'_>) -> Vec<String> {
    let mut outside = Vec::new();
    if let Some(cats) = listing.categories {
        for c in cats {
            let normalized = directory::normalize_category(c);
            if !cred_scope.categories.iter().any(|sc| sc == &normalized) {
                outside.push(c.clone());
            }
        }
    }
    if let Some(areas) = listing.areas
        && !cred_scope.area.is_empty()
        && areas.iter().any(|a| !cred_scope.area.iter().any(|sa| area_within(a, sa)))
    {
        outside.push("area".to_string());
    }
    outside
}

pub fn evaluate(evidence: &MembershipEvidence, input: &CheckInput<'_>) -> MembershipVerdict {
    let Some(issuer) = input.pinned_issuer else {
        return MembershipVerdict::Unknown { reason: "issuer-not-pinned".to_string() };
    };

    let mut good = Vec::new();
    let mut first_refusal: Option<String> = None;
    for env in evidence.credentials.iter().take(MAX_EVIDENCE_CREDENTIALS) {
        match verify_credential(env, issuer, input.member_did, input.now_secs) {
            Ok(c) => good.push(c),
            Err(r) => {
                if first_refusal.is_none() {
                    first_refusal = Some(r);
                }
            }
        }
    }
    if good.is_empty() {
        return first_refusal
            .map_or(MembershipVerdict::None, |reason| MembershipVerdict::Refused { reason });
    }
    let cred = pick_current(good);

    if let Some(r) = find_revocation(evidence, issuer, &cred, input.member_did, input.now_secs) {
        return MembershipVerdict::Revoked {
            credential_record_id: cred.record_id.clone(),
            revocation_record_id: r.record_id,
            revoked_at_secs: r.issued_at_secs,
            reason: r.payload.reason,
        };
    }

    if let Some(d) = active_suspension(evidence, issuer, input) {
        return MembershipVerdict::Suspended {
            decision_record_id: d.record_id,
            scope: d.payload.scope,
            rule: d.payload.rule,
            reason: d.payload.reason,
            since_secs: d.issued_at_secs,
            until_secs: d.payload.until_secs,
        };
    }

    if input.now_secs >= cred.expires_at_secs {
        return MembershipVerdict::Expired {
            credential_record_id: cred.record_id.clone(),
            expires_at_secs: cred.expires_at_secs,
        };
    }

    if let Some(listing) = input.listing {
        let outside = scope_violations(&cred.payload.scope, listing);
        if !outside.is_empty() {
            return MembershipVerdict::OutOfScope {
                credential_record_id: cred.record_id.clone(),
                outside,
            };
        }
    }

    MembershipVerdict::Valid {
        credential_record_id: cred.record_id.clone(),
        issuer: cred.issuer.clone(),
        synorg_name: cred.payload.synorg_name.clone(),
        scope: cred.payload.scope.clone(),
        expires_at_secs: cred.expires_at_secs,
        revocations_checked_as_of_secs: input.evidence_as_of_secs,
    }
}

/// The time window in which one listing of this member may appear in
/// this directory's search, as `(listed_from_secs, listed_until_secs)`.
/// `(0, 0)` means "not listed". Used only by the directory on its own
/// evidence, so the search filter can run at the host.
pub fn listed_window(
    evidence: &MembershipEvidence,
    issuer: &str,
    member_did: &str,
    listing_id: &str,
    now_secs: u64,
) -> (u64, u64) {
    let mut good = Vec::new();
    for env in evidence.credentials.iter().take(MAX_EVIDENCE_CREDENTIALS) {
        if let Ok(c) = verify_credential(env, issuer, member_did, now_secs) {
            good.push(c);
        }
    }
    if good.is_empty() {
        return (0, 0);
    }
    let cred = pick_current(good);
    if find_revocation(evidence, issuer, &cred, member_did, now_secs).is_some() {
        return (0, 0);
    }

    let until = cred.expires_at_secs;
    let listing = ListingRef { listing_id, categories: None, areas: None };
    let mut from = 0u64;
    for d in suspend_decisions(evidence, issuer, member_did, now_secs) {
        if !applies(&d.payload.scope, Some(listing)) {
            continue;
        }
        match d.payload.until_secs {
            None => return (0, 0),
            Some(u) => from = from.max(u),
        }
    }
    if from >= until {
        return (0, 0);
    }
    (from, until)
}

/// Words for `roymctl` and a Rust-side assertion; the Hub has its own
/// copy (`crates/roym_web/ui/src/directory/membership.ts`).
pub fn verdict_word(v: &MembershipVerdict) -> &'static str {
    match v {
        MembershipVerdict::None => "none",
        MembershipVerdict::Valid { .. } => "valid",
        MembershipVerdict::Expired { .. } => "expired",
        MembershipVerdict::OutOfScope { .. } => "out-of-scope",
        MembershipVerdict::Revoked { .. } => "revoked",
        MembershipVerdict::Suspended { .. } => "suspended",
        MembershipVerdict::Refused { .. } => "refused",
        MembershipVerdict::Unknown { .. } => "unknown",
    }
}

#[cfg(test)]
mod tests;
