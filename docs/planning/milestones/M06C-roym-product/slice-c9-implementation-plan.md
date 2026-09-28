# Slice C9 — Cross-installation trust (R3) and the inherited cross-node cases

**Milestone:** [task.md](task.md) (row C9, `D-06C-6`, `D-06C-7`, `D-06C-12`)
**Spec:** [roym-integrated-experience-spec.md](../../../roym-integrated-experience-spec.md) — R3 (all three rows), Phase 2 S4–S13, Phase 3 C4–C9, Records table, Search.
**Status:** Plan only. Written 2026-09-28 against `main` at `3452c630`.
**Depends on:** C8 (complete).

This plan is written so that a different session can execute it without
this session's reasoning. Every file path is repo-relative. Line numbers
are from `3452c630` and must be re-checked before editing.

---

## §0 Read this first — decisions this plan needs you to confirm

The plan below assumes the **recommended** answer to each question. If you
choose differently, the section named in the right column changes.

| # | Question | Recommendation | Changes |
|---|---|---|---|
| Q1 | Who is the issuer of `membership-credential`, `revocation`, `moderation-decision`? | **The DID that deployed Z's Roym instance** (Z's recorded owner), signing through the directory service's own record-signing certificate. A SynOrg that wants an organisation identity separate from a person deploys Z under a dedicated org identity. No "acts-for" delegation is built. | §2 D-C9-1, §5 |
| Q2 | Does `directory.publish` require the listing's issuer to hold a valid credential from this SynOrg? | **Yes.** The spec says the directory "holds the listings its members published". A withdrawal is still accepted from a non-member. This changes fixtures in ~30 C6 parity scenarios, one e2e, and Hub cases 13–23b (§10.3). | §5.6, §10.3 |
| Q3 | How does the consumer know which issuer DID a directory should sign with? | **Pinned per source at `directory.add-source`** — from an explicit `issuer_did` parameter if the person gives one, otherwise from `directory.info` (trust on first use). Never re-pinned silently. | §2 D-C9-4, §6.1 |
| Q4 | R3 row 3 says "cached copies show the **revocation**" after a **suspension**. The Records table has both `revocation` and `moderation-decision`. | **Keep them separate records** (S11 suspend ≠ S13 revoke). The consumer's check shows *either* withdrawal: `revoked` or `suspended`. Test wording: "shows the withdrawal". | §2 D-C9-5 |
| Q5 | Four inherited cross-node rows (dropped ack, forged author, future/past timestamp) cannot be driven over a real connection through any public interface. | **Add a `test-support` cargo feature to `syneroym-conversation`** with two one-shot hooks, enabled only from `syneroym-substrate`'s `[dev-dependencies]`. Alternative: accept crate-level unit tests as the coverage and record the residual. | §9, §2 D-C9-10 |
| Q6 | Two backlog rows are targeted "C9 / follow-on" but are not R3: the Hub transaction action panel (Playwright 33–39) and `conversation.history` reconciliation. | **Do not build them in C9.** Retarget both rows to `TBD` with their existing triggers. | §12 |
| Q7 | `fct` claims portability row (deferred-backlog §7, "M06C C9 to decide"). | **Decide: not portable.** C9 makes no cross-installation use of `fct`; every cross-install trust statement is a signed record. Move the row to "Recently resolved". | §12 |

---

## §1 What C9 must deliver

From `task.md` row C9 and the spec's R3 table:

1. **R3 row 1** — the full R1+R2 flow with consumer X, provider Y, and SynOrg
   owner Z on three separate installations, resolving each other through
   the registry (ADR-0022), not through pre-seeded addresses.
2. **R3 row 2** — a signed `membership-credential` (issuer, scope, expiry)
   and signed `revocation`s. The consumer's **own** node verifies
   signature, issuer, scope, and expiry. The directory's word is never a
   verdict.
3. **R3 row 3** — a signed, scoped `moderation-decision` with source and
   timestamp. A suspended member vanishes from that directory's results.
   A copy the consumer already holds shows the withdrawal on the next
   check. The product says plainly that instant removal is not promised.
4. **`D-06C-7`** — M06B B4's uncovered cross-node test rows, except alias
   canonicalization.
5. Backlog rows targeted at C9 (§12).

For how the finished feature looks to a person, read §15 (the
from-scratch demo script) first.

**Not in C9:** group chat (C10), any new host interface, any new WIT,
issuer-key revocation through the registry (§2 D-C9-8), a signed
"complete revocation list" record (§2 D-C9-6).

---

## §2 Decisions (D-C9-n)

| # | Decision | Why |
|---|---|---|
| D-C9-1 | **Directory records are signed with `Principal::Delegated` under the directory service's installed record-signing certificate.** The issuer is Z's recorded owner. `directory` gains the two certificate verbs every other signing service has, and joins `SIGNING_SERVICES` in all four places (§7). | `signing::install` (`crates/roym_core/src/signing.rs:81`) refuses a certificate whose master is not the recorded owner. The spec's "one person per installation, and that person deploys it" rule says the same. `D-C3-3` hoped for a dedicated org master; that works only when the org identity *is* the deployer. No new mechanism. |
| D-C9-2 | **Three payload types in a new module `roym_core::membership`**, each at version 1, all integers, subject fixed per type: credential → member DID; revocation → credential `record_id`; moderation decision → member DID. | `D-06C-1` (explicit version), `D-C3-11` (integers only). A fixed subject makes "is this record about *this* member" a string comparison the consumer does itself. |
| D-C9-3 | **One pure evaluator, `membership::evaluate`, is the only place a membership verdict is computed.** The directory uses it on its own evidence (search filter, publish gate). The consumer uses it on evidence it received (search, `check-standing`, `memberships`). | One definition of "valid" or the two sides disagree. Pure and synchronous, so it has unit tests that do not need a host. |
| D-C9-4 | **The consumer pins an issuer DID per source.** `SourceRow` gains `issuer_did: Option<String>`. `add-source` sets it from an explicit parameter, or else from `directory.info`'s new `issuer_did` field. A later reply naming a different issuer yields `unknown` with reason `issuer-changed`, and the pin is not changed. Only the person re-adding the source with an explicit `issuer_did` changes it. | Without a pinned issuer, "checks issuer" means "checks the issuer the directory claims", which is the directory's word. Trust on first use matches how the product already treats conversation keys (`task.md`, carried-forward limits). The UI must use a different word for it than for a checked signature. |
| D-C9-5 | **Suspension and revocation are separate records and separate verdict states.** `member.suspend` signs a `moderation-decision` with `action: suspend`; `member.lift` signs one with `action: lift` and `supersedes` = the suspension's `record_id`. `revocation.issue` signs a `revocation`. Verdict precedence: `refused` > `revoked` > `suspended` > `expired` > `out-of-scope` > `valid`. | S11 (suspend) and S13 (revoke) are different actions in the spec. A suspension is reversible; a revocation is not. Treating both as "withdrawn" on the consumer side is what R3 row 3's test needs. |
| D-C9-6 | **"Signed revocation list" = the set of individually signed `revocation` records the issuer serves.** No signed snapshot record. The consumer's `valid` verdict carries `revocations_checked_as_of_secs` and the UI states that a withheld revocation cannot be detected. | The Records table has `revocation`, not `revocation-list`, and `RECORD_TYPES` is fixed. In R3 the issuer and the directory are the same party, so omission means the issuer hiding its own decision. A snapshot record is a backlog row with trigger "a directory serves credentials it did not issue". |
| D-C9-7 | **The directory keeps a derived `standing` row per member**: the evidence bytes (credentials, their revocations, the newest decisions) a consumer would receive. It is rebuilt whenever one of that member's records is written, and fully rebuilt on `import` and `reindex`. `directory.search` and `directory.standing` both serve these bytes. | One indexed `get` per distinct issuer at search time, instead of three queries. The same bytes feed the directory's filter and the consumer's check. Derived and rebuildable, like `search_index` (C6's rule). |
| D-C9-8 | **Issuer-*key* revocation is not checked.** `RevocationSource::check_did` stays `Unknown`. Record-level revocation is computed by the evaluator, not through `RevocationSource`. | Key revocation lives in the registry's master anchor (`crates/core/src/dht_registry.rs`, `revoked_keys`), which no guest can reach. C3's §18 note E said "C9 supplies the real source for both". That is stale (§14 item 2). Backlog row. |
| D-C9-9 | **The directory's server-side search filter judges membership, not scope.** A hit stays only when `evaluate(.., listing: listing_id only)` is `valid`. Scope is judged by the consumer and by the publish gate. | The index row does not carry the full payload, and parsing each envelope before truncation is expensive on an anonymous path. The publish gate already refuses an out-of-scope listing. |
| D-C9-10 | **(Q5) A `test-support` feature on `syneroym-conversation`** adds two one-shot hooks: `drop_next_acks(n)` on the receiving side (store, then fail instead of ack), and `override_next_outgoing(OutgoingOverride { author, sender_timestamp_ms })` on the sending side. All hook code is `#[cfg(feature = "test-support")]`. Only `crates/substrate/Cargo.toml` `[dev-dependencies]` enables it. There is no automatic guard against a normal dependency enabling it; the guard is review plus the feature's name. |  Rows 4, 7, 13 need a peer that misbehaves in a precise way. The receiving code is the real code over a real connection; only the misbehaving sender/ack is synthetic. |
| D-C9-11 | **Directory schema version 3 → 4.** New collections and new bundle sections. No migration (pre-release). | `DIRECTORY_SCHEMA_VERSION`'s own comment: a bundle from before a required field must fail at the version gate. |
| D-C9-12 | **`directory.standing` is the fourth wire-reachable directory method (`WireRule::Open`).** | A credential, a revocation, and a moderation decision are public statements on purpose. Reading them costs the SynOrg nothing to leave open. |

---

## §3 `roym_core` — vocabulary (WO1)

### 3.1 `crates/roym_core/src/record.rs`

Add after `RECORD_FULFILMENT_RECEIPT`:

```rust
pub const RECORD_MEMBERSHIP_CREDENTIAL: &str = "membership-credential";
pub const RECORD_REVOCATION: &str = "revocation";
pub const RECORD_MODERATION_DECISION: &str = "moderation-decision";
```

`RECORD_TYPES` already lists all three at version 1. No change there.

### 3.2 New module `crates/roym_core/src/membership.rs` (+ `membership/tests.rs`)

Add `pub mod membership;` to `crates/roym_core/src/lib.rs` (alphabetical,
after `listing`). Keep the production file under 800 lines; tests go in
`src/membership/tests.rs` (module-layout rule: `membership.rs` +
`membership/`).

```rust
//! A SynOrg's signed statements about one member -- credential,
//! revocation, moderation decision -- and the one function that turns a
//! bundle of them into a verdict. The directory uses it on its own
//! records; a consumer uses it on records it received. Neither ever takes
//! the other's verdict.

use serde::{Deserialize, Serialize};

use crate::{area::{self, Area}, directory, record};

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
pub const NO_INSTANT_REMOVAL_NOTICE: &str = "A group's decision reaches copies \
    other people already hold only when they next check. Nobody can promise it \
    is removed everywhere at once.";
pub const WITHHELD_REVOCATION_NOTICE: &str = "This shows every withdrawal the group \
    has published. It cannot show one the group chose not to publish.";

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
pub enum ModerationAction { Suspend, Lift }

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
    /// Which of the SynOrg's own rules. Required for `suspend`, empty for `lift`.
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
        /// claim that none exist (D-C9-6).
        revocations_checked_as_of_secs: u64,
    },
    Expired { credential_record_id: String, expires_at_secs: u64 },
    OutOfScope { credential_record_id: String, outside: Vec<String> },
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
    Refused { reason: String },
    /// This node could not check at all.
    Unknown { reason: String },
}

/// What is being judged. `categories`/`areas` are `None` when the caller
/// does not judge scope (the directory's search filter, D-C9-9, and a
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
pub enum MembershipError { /* NotDidKey, NoCategories, TooManyCategories,
    CategoryShape, TooManyAreas, Area(AreaError), CategoryOutsideSynOrg(String),
    ExpiryOutOfBounds, ReasonTooLong, RuleShape, UntilInPast */ }

impl MembershipScope { pub fn validate(&self) -> Result<(), MembershipError>; }
impl MembershipCredentialPayload { pub fn validate(&self) -> Result<(), MembershipError>; }
impl RevocationPayload { pub fn validate(&self) -> Result<(), MembershipError>; }
impl ModerationDecisionPayload { pub fn validate(&self, now_secs: u64) -> Result<(), MembershipError>; }

pub fn evaluate(evidence: &MembershipEvidence, input: &CheckInput<'_>) -> MembershipVerdict;

/// Words for `roymctl` and a Rust-side assertion; the Hub has its own copy.
pub fn verdict_word(v: &MembershipVerdict) -> &'static str; // "none" | "valid" | ...
```

Validation rules (all return `MembershipError`):

- `member_did`: `person::is_did_key`.
- `scope.categories`: 1..=`MAX_CATEGORIES`; each equals
  `normalize_category(c)` and is 1..=`MAX_CATEGORY_LEN` bytes.
- `scope.area`: ≤ `MAX_AREAS`, each `Area::validate()`.
- `reason` ≤ `MAX_REASON_LEN`; `rule` 1..=`MAX_RULE_LEN` for `suspend`,
  empty for `lift`; `until_secs`, when present, `> now_secs` and only on
  `suspend`; `lift` must carry `scope` equal to the suspension it lifts
  (checked in the directory, §5.4, not here).

#### Pseudo-code for `evaluate`

```text
fn evaluate(ev, input):
    let Some(issuer) = input.pinned_issuer
        else return Unknown { reason: "issuer-not-pinned" }

    // 1. Credentials. Verify each; keep the good ones; remember the first refusal.
    good = []; first_refusal = None
    for env in ev.credentials.iter().take(MAX_EVIDENCE_CREDENTIALS):
        match verify_credential(env, issuer, input.member_did, input.now_secs):
            Ok(c)  => good.push(c)
            Err(r) => first_refusal.get_or_insert(r)
    if good.is_empty():
        return first_refusal.map(|r| Refused { reason: r }).unwrap_or(None)
    cred = pick_current(good)   // drop any whose record_id another good one `supersedes`;
                                // then newest issued_at_secs; tie -> smaller record_id

    // 2. Revocation of *that* credential.
    for env in ev.revocations:
        if let Ok(r) = verify_revocation(env, issuer, input.now_secs)   // bad ones are ignored,
           && r.payload.credential_record_id == cred.record_id          // never promoted
           && r.payload.member_did == input.member_did:
            return Revoked { credential_record_id, revocation_record_id: r.record_id,
                             revoked_at_secs: r.issued_at_secs, reason }

    // 3. Active suspension.
    decisions = ev.decisions.iter().take(MAX_EVIDENCE_DECISIONS)
                  .filter_map(|e| verify_decision(e, issuer, input.member_did, now).ok())
    lifted = { d.supersedes | d in decisions, d.action == Lift }
    for d in decisions sorted newest first:
        if d.action == Suspend && !lifted.contains(d.record_id)
           && d.until_secs.map_or(true, |u| input.now_secs < u)
           && applies(d.scope, input.listing):          // Membership: always;
                                                        // Listing{id}: input.listing.listing_id == id
            return Suspended { .. }

    // 4. Expiry (verify_credential used allowing_expired()).
    if input.now_secs >= cred.expires_at_secs:
        return Expired { .. }

    // 5. Scope, only for the parts the caller asked to judge.
    if let Some(l) = input.listing:
        outside = []
        if let Some(cats) = l.categories:
            outside += cats.filter(|c| !cred.scope.categories.contains(normalize_category(c)))
        if let Some(areas) = l.areas && !cred.scope.area.is_empty():
            for a in areas:
                if !cred.scope.area.any(|s| area_within(a, s)): outside.push("area")   // once
        if !outside.is_empty(): return OutOfScope { .. }

    Valid { .., revocations_checked_as_of_secs: input.evidence_as_of_secs }

fn verify_credential(env, issuer, member, now) -> Result<Cred, String>:
    v = record::verify_json(env, VerifyOptions::new(now).expecting(issuer).allowing_expired())
          .map_err(to_string)?                 // signature, delegation, issuer, clock skew
    require v.record_type == RECORD_MEMBERSHIP_CREDENTIAL && v.version == 1
    require v.subject == member
    p: MembershipCredentialPayload = from_value(v.payload)?; p.validate()?
    require p.member_did == v.subject
    exp = v.expires_at_secs.ok_or("credential carries no expiry")?
    Ok(Cred { record_id: v.record_id, supersedes: v.supersedes, issued_at: v.issued_at_secs,
              expires_at_secs: exp, payload: p, issuer: v.issuer })

fn verify_revocation / verify_decision: same shape; type/version check;
    revocation: subject == payload.credential_record_id;
    decision:   subject == member && payload.member_did == member;
                lift must carry `supersedes`.

fn area_within(listing_area, scope_area) -> bool:
    match (listing_area, scope_area):
        (Named, Named) => area::labels_match(listing_area, scope_area)
        (Named, _) | (_, Named) => false        // cannot show it is inside
        _ => area::areas_intersect(listing_area, scope_area) == Some(true)
```

**Flag for review:** `area_within` uses *intersects*, not *contains*. A
listing area that pokes outside the SynOrg's area still counts as in
scope. Containment for circles/boxes is not in `roym_core::area` today.
Recorded as a backlog row (§12), not built.

#### Unit tests — `crates/roym_core/src/membership/tests.rs`

Use `syneroym_identity::Identity` + `DelegationCertificate::issue` + a
local `sign_record` helper that builds `Envelope::unsigned` and signs
(copy the pattern from `crates/signed_record/src/verify.rs` tests
`sign_env`). One test each:

1. valid credential, no revocations/decisions → `Valid`, carries `evidence_as_of_secs`.
2. no pinned issuer → `Unknown{issuer-not-pinned}`.
3. credential signed by a different master → `Refused` (issuer mismatch).
4. credential whose subject is another DID → `Refused`.
5. wrong record type / version 2 → `Refused`.
6. empty evidence → `None`.
7. revocation of this credential → `Revoked`; revocation of another credential → ignored (`Valid`).
8. revocation signed by a different issuer → ignored.
9. suspend (membership) → `Suspended`; plus a lift that supersedes it → `Valid`.
10. suspend with `until_secs` in the past (sign with `now` earlier) → `Valid`.
11. listing-scoped suspend → `Suspended` for that listing, `Valid` for another and for `listing: None`.
12. expired credential (sign at T, check at T + lifetime) → `Expired`; revoked + expired → `Revoked` (precedence).
13. category outside scope → `OutOfScope{outside:["x"]}`; area outside → `OutOfScope{outside:["area"]}`; scope not judged when `categories: None`.
14. two credentials, the newer supersedes the older → newer chosen; the older's revocation no longer matters.
15. `NO_INSTANT_REMOVAL_NOTICE` and `WITHHELD_REVOCATION_NOTICE` appear verbatim in `../roym_web/ui/src/directory/membership.ts` (same file-read pattern as `router.rs:240`).

### 3.3 `crates/roym_core/src/directory.rs`

- `DIRECTORY_SCHEMA_VERSION`: `3` → `4`. Update its doc comment to say why
  (new sections, `SourceRow.issuer_did`, `SearchHit.membership`).
- Remove the stale sentence in `SynOrgSettings`'s doc comment: "`directory`
  mounts no signing certificate in this slice." Replace with: settings stay
  unsigned app state; the SynOrg's signed statements are the three
  `membership` record types.
- `SearchHit` gains:

```rust
    /// The issuer's own signed statements about `issuer`'s membership,
    /// exactly as the directory stores them. Never a verdict.
    #[serde(default)]
    pub membership: MembershipEvidence,
```

(`use crate::membership::MembershipEvidence;`)

### 3.4 `crates/roym_core/src/backup.rs`

Add section constants:

```rust
pub const SECTION_CREDENTIALS: &str = "credentials";
pub const SECTION_REVOCATIONS: &str = "revocations";
pub const SECTION_DECISIONS: &str = "moderation_decisions";
pub const SECTION_HELD_MEMBERSHIPS: &str = "held_memberships";
```

### 3.5 `crates/roym_core/src/router.rs`

- `ROUTES`: add `("credential.", DIRECTORY, MethodAuth::Owner)` and
  `("revocation.", DIRECTORY, MethodAuth::Owner)` after `("member.", ...)`.
  (`member.suspend` / `member.lift` already route through `"member."`.)
- `every_certificate_mounted_service_routes_under_its_own_name`: `expected`
  becomes `["profile", "catalog", "conversation", "transaction", "directory"]`
  and its comment "mounted on these four" → "these five".

---

## §4 Collections and rows (directory)

In `crates/roym_directory/src/app.rs`, add constants:

```rust
pub const CREDENTIALS: &str = "credentials";          // server half, id = record_id
pub const REVOCATIONS: &str = "revocations";          // server half, id = record_id
pub const DECISIONS: &str = "moderation_decisions";   // server half, id = record_id
pub const STANDING: &str = "standing";                // server half, derived, id = member_did
pub const HELD_MEMBERSHIPS: &str = "held_memberships"; // client half, id = "{source}#{member_did}"
```

New file `crates/roym_directory/src/app/standing.rs` (server half) holds
the row types:

```rust
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
    pub(in crate::app) evidence: MembershipEvidence,
    pub(in crate::app) updated_at_secs: u64,
}
```

All three issued-record collections are created with indexes
`[idx("member_did", IndexType::String), idx("issued_at_secs", IndexType::Numeric)]`.

New file `crates/roym_directory/src/app/held.rs` (client half):

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::app) struct HeldMembershipRow {
    pub(in crate::app) source: String,
    pub(in crate::app) member_did: String,
    pub(in crate::app) issuer_did: String,
    pub(in crate::app) evidence: MembershipEvidence,
    /// This node's own clock when `evidence` was fetched.
    pub(in crate::app) as_of_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(in crate::app) last_error: Option<SourceError>,
}
```

Index: `idx("member_did", IndexType::String)`.

---

## §5 Directory — server half (WO2)

### 5.1 Enrolment

`crates/roym_directory/src/app.rs::invoke`: after `admit::admit(...)`,
before the `match`:

```rust
    if let Some(resp) = signing::handle_certificate_verb(host, "directory.", &req).await {
        return resp;
    }
```

`directory.signing-status` / `directory.install-signing-certificate` are
not in `WIRE_REACHABLE`, so `admit` already refuses them from the wire.
`use syneroym_roym_core::signing;`.

### 5.2 Issuer identity helper

In `app.rs`, next to `owner_did_or_node`:

```rust
/// The DID this SynOrg's signed statements are issued under: the
/// installation's recorded owner. `None` when there is no owner -- such a
/// node can run a directory but can issue nothing.
pub(in crate::app) async fn issuer_did<H: AppHost>(host: &H) -> Option<String> {
    signing::owner_did(host).await.ok()
}
```

### 5.3 `credential.*` and `revocation.*` — new file `app/credential_ops.rs`

Dispatch lines in `invoke`:

```rust
        "credential.issue" => credential_ops::issue(host, &req).await,
        "credential.list" => credential_ops::list(host, &req).await,
        "revocation.issue" => credential_ops::revoke(host, &req).await,
        "revocation.list" => credential_ops::list_revocations(host, &req).await,
        "member.suspend" => moderation_ops::suspend(host, &req).await,
        "member.lift" => moderation_ops::lift(host, &req).await,
        "member.decisions" => moderation_ops::list(host, &req).await,
        "directory.standing" => standing::standing_verb(host, &req).await,
```

Shared signing helper in `credential_ops.rs`:

```rust
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
) -> Result<(String, String), Response>
```

Pseudo-code:

```text
(principal, _owner) = signing::person_principal(host, now)
    .map_err(|e| NotEnrolled => invalid_params("signing-not-enrolled"),
                  other => internal_error)
draft = RecordDraft { version, record_type, subject, payload: to_string(payload),
                      expires_at_secs, supersedes }
env = AppSigning::sign_record(host, draft, principal)   // map SigningError -> internal/invalid
record_id = Envelope::from_json(&env)?.record_id()?
Ok((env, record_id))
```

`issue` (`credential.issue`), params
`{ member_did, categories: [..], area?: [..], expires_at_secs, note? }`:

```text
settings = synorg::load_settings -> None => invalid_params("this installation runs no SynOrg yet")
scope = MembershipScope { categories: normalized(params.categories), area }
payload = MembershipCredentialPayload { synorg_name: settings.name, member_did, scope }
payload.validate()                           -> invalid_params
every scope.category must be in settings.categories (normalized) -> invalid_params
    ("category '{c}' is not one of this SynOrg's own categories")
now < expires_at_secs <= now + MAX_CREDENTIAL_LIFETIME_SECS -> else invalid_params
(env, rid) = sign_as_synorg(RECORD_MEMBERSHIP_CREDENTIAL, 1, member_did, &payload,
                            Some(expires_at_secs), supersedes = current credential id if any)
put CREDENTIALS[rid] = IssuedRecordRow { record_id: rid, member_did, about: "", issued_at_secs, envelope }
upsert MEMBERS[member_did] = Member { did, note (param or existing), added_at_secs (keep existing) }
standing::rebuild_for(host, member_did)
ok { record_id, envelope }
```

`issued_at_secs` in the row comes from the parsed envelope, never from
`clock::now_secs()`, so the row and the signed bytes cannot disagree.

`revoke` (`revocation.issue`), params `{ credential_record_id, reason }`:

```text
row = get CREDENTIALS[credential_record_id] -> None => invalid_params("no credential with that id was issued here")
if a REVOCATIONS row with about == credential_record_id exists -> ok(existing) (idempotent)
payload = RevocationPayload { credential_record_id, member_did: row.member_did, reason }; validate
(env, rid) = sign_as_synorg(RECORD_REVOCATION, 1, credential_record_id, &payload, None, None)
put REVOCATIONS[rid] = IssuedRecordRow { about: credential_record_id, .. }
standing::rebuild_for(host, row.member_did)
ok { record_id, envelope }
```

`list` / `list_revocations`: optional `member_did` filter →
`collect_raw_where(.., {"member_did": m})`, else `collect_raw`. Rows are
returned as stored (envelopes included).

### 5.4 `member.suspend` / `member.lift` / `member.decisions` — new file `app/moderation_ops.rs`

`suspend`, params `{ member_did, rule, reason, scope?: {kind, listing_id?}, until_secs? }`
(default scope `membership`):

```text
require MEMBERS[member_did] exists -> else invalid_params("not a member of this SynOrg")
payload = ModerationDecisionPayload { action: Suspend, .. }; payload.validate(now)
(env, rid) = sign_as_synorg(RECORD_MODERATION_DECISION, 1, member_did, &payload, None, None)
put DECISIONS[rid]; standing::rebuild_for(member_did)
ok { record_id, envelope }
```

`lift`, params `{ decision_record_id, reason }`:

```text
d = get DECISIONS[decision_record_id] -> None => invalid_params
parsed = verify own envelope (verify_json expecting issuer_did) ; require action == Suspend
if a DECISIONS row with action Lift and supersedes == decision_record_id exists -> ok(existing)
payload = { action: Lift, member_did, scope: parsed.scope, rule: "", reason, until_secs: None }
(env, rid) = sign_as_synorg(.., supersedes = Some(decision_record_id))
put; rebuild_for; ok
```

Store `about` = the superseded record id for a lift so the "already
lifted" check is one filter: `{"$and":[{"member_did":m},{"about":id}]}`.

`member.decisions`: like `credential.list`.

`member.remove` (`synorg.rs:106`) — **behaviour change:** before deleting,
compute `standing::own_verdict(host, did, None, now)`. Refuse when the
verdict is `Valid` or `Suspended` (the credential would still verify once
a suspension ends) with
`Response::invalid_params("revoke this member's credential first")`.
Otherwise delete as today. Rationale: a roster that forgets a member
while their credential still verifies elsewhere is a lie.

### 5.5 `app/standing.rs`

```rust
pub(in crate::app) async fn rebuild_for<H: AppHost>(host: &H, member_did: &str) -> Result<(), String>;
pub(in crate::app) async fn rebuild_all<H: AppHost>(host: &H) -> Result<u64, String>;
pub(in crate::app) async fn load<H: AppHost>(host: &H, member_did: &str) -> Result<MembershipEvidence, String>;
pub(in crate::app) async fn standing_verb<H: AppHost>(host: &H, req: &Request) -> Response;
/// The directory judging its own member (D-C9-9): the owner is the issuer.
pub(in crate::app) async fn own_verdict<H: AppHost>(
    host: &H, member_did: &str, listing: Option<ListingRef<'_>>, now: u64,
) -> Result<MembershipVerdict, String>;
```

`rebuild_for` pseudo-code:

```text
creds = collect_raw_where(CREDENTIALS, {"member_did": m}) -> IssuedRecordRow
        sort by issued_at_secs desc, record_id asc; take MAX_EVIDENCE_CREDENTIALS
ids = creds.record_id set
revs = collect_raw_where(REVOCATIONS, {"member_did": m}) filter about in ids
decs = collect_raw_where(DECISIONS, {"member_did": m}) sort desc; take MAX_EVIDENCE_DECISIONS
if creds, revs, decs all empty: delete STANDING[m] (ignore NotFound); return
put STANDING[m] = StandingRow { member_did: m, evidence: {credentials, revocations, decisions}
                                 (envelopes only), updated_at_secs: now }
```

`rebuild_all`: `delete_many(STANDING, {})`, then distinct `member_did`
over the three collections → `rebuild_for` each. Called from `import`
and from `directory.reindex` (`search_ops::reindex` calls both
`rebuild_search_index` and `standing::rebuild_all`; return
`{ rebuilt, standing }`).

`standing_verb` (`directory.standing`, wire-open), params `{ member_did }`:

```text
require member_did is did:key -> invalid_params
issuer = issuer_did(host)          // may be None
evidence = load(member_did) (empty if no row)
ok { issuer_did: issuer, member_did, evidence, answered_at_secs: clock::now_secs() }
```

`own_verdict`: `evaluate(&load(m)?, &CheckInput { pinned_issuer: issuer_did(host).as_deref(), member_did: m, listing, now_secs: now, evidence_as_of_secs: now })`.

Add to `WIRE_REACHABLE` (`app.rs:52`):

```rust
    ("directory.standing", WireRule::Open),
```

and update its doc comment ("The four methods a foreign node may reach").

### 5.6 Publish gate (`app/publication_ops.rs::publish`)

Insert **after** the `Withdrawn` branch (`publication_ops.rs:181-183`) and
**before** `check_rate_limit`:

```rust
    if let Err(resp) = require_member(host, &issuer_of_listing, &payload, now).await {
        return resp;
    }
```

(`issuer` from the verdict is currently moved into `PublishedRecord`
later; bind `let issuer_of_listing = issuer.clone();` where the tuple is
destructured.)

```rust
/// A listing is admitted only from a member whose credential from this
/// SynOrg checks out on this node for exactly this listing. A withdrawal
/// never reaches here: a suspended member must still be able to take a
/// listing down.
async fn require_member<H: AppHost>(
    host: &H,
    listing_issuer: &str,
    payload: &listing::ListingPayload,
    now: u64,
) -> Result<(), Response> {
    let areas = payload.location.as_ref().map(|l| l.service_area.as_slice());
    let listing = ListingRef {
        listing_id: &payload.listing_id,
        categories: Some(&payload.categories),
        areas: Some(areas.unwrap_or(&[])),
    };
    let verdict = standing::own_verdict(host, listing_issuer, Some(listing), now)
        .await
        .map_err(Response::internal_error)?;
    match verdict {
        MembershipVerdict::Valid { .. } => Ok(()),
        other => Err(Response::invalid_params(format!(
            "this SynOrg does not admit this listing: {}",
            membership::verdict_word(&other)
        ))
        .with_data(json!({ "admission": "not-admitted", "membership": other }))),
    }
}
```

The listing issuer is the provider's **person** DID (the listing is signed
under the provider's delegation). The credential's subject must be that
DID. `published_by` (the connection identity) is still what the rate
limit keys on; do not change that.

### 5.7 Search filter + evidence (`app/search_ops.rs::search`)

After `let by_listing = refine_by_listing(..)` (line ~187), before sort:

```rust
    let now = clock::now_secs();
    let standing = match standing_by_issuer(host, by_listing.values()).await {
        Ok(s) => s,
        Err(e) => return Response::internal_error(e),
    };
    let mut hits: Vec<(SearchIndexRow, AreaMatch)> = by_listing
        .into_values()
        .filter(|(row, _)| listed(&standing, row, own_issuer.as_deref(), now))
        .collect();
```

```rust
/// One `get` per distinct issuer among the candidates, never per row.
async fn standing_by_issuer<'a, H: AppHost>(
    host: &H,
    rows: impl Iterator<Item = &'a (SearchIndexRow, AreaMatch)>,
) -> Result<BTreeMap<String, MembershipEvidence>, String>;

/// D-C9-9: membership only, listing-scoped suspension included, scope not judged.
fn listed(standing: &BTreeMap<String, MembershipEvidence>, row: &SearchIndexRow,
          issuer: Option<&str>, now: u64) -> bool {
    let Some(ev) = standing.get(&row.issuer) else { return false };
    matches!(membership::evaluate(ev, &CheckInput {
        pinned_issuer: issuer, member_did: &row.issuer,
        listing: Some(ListingRef { listing_id: &row.listing_id, categories: None, areas: None }),
        now_secs: now, evidence_as_of_secs: now,
    }), MembershipVerdict::Valid { .. })
}
```

`own_issuer = issuer_did(host).await` is read once at the top of
`search`. `SearchIndexRow.issuer` is private today — make it
`pub(in crate::app)`.

`hits_with_envelopes` gains a `&BTreeMap<String, MembershipEvidence>`
parameter and sets `membership: standing[&row.issuer].clone()` on each
`SearchHit`.

**Cost note (check it, do not assume it):** a full page is 50 hits × one
credential envelope (~2–3 KiB with its delegation). Measure one real
`directory.search` reply size in the parity suite and assert it stays
under the proxy's reply limit (find the limit in `crates/router/src/proxy.rs`
before writing the assertion).

### 5.8 `directory.info`

`synorg::info` adds two fields:

```rust
        "issuer_did": issuer_did(host).await,   // null when no owner
        "signing_did": /* AppSigning::signing_identity(host).signing_did, or null */,
```

### 5.9 Export / import (`app/backup.rs`)

- Export: add sections `SECTION_CREDENTIALS`, `SECTION_REVOCATIONS`,
  `SECTION_DECISIONS`, `SECTION_HELD_MEMBERSHIPS` (from `CREDENTIALS`,
  `REVOCATIONS`, `DECISIONS`, `HELD_MEMBERSHIPS`). `STANDING` is derived
  and is **not** exported.
- Export signs the manifest exactly like `crates/roym_catalog/src/app/backup.rs:71`:
  `signing::sign_bundle(host, &mut bundle, now)`, `NotEnrolled` →
  `invalid_params("signing-not-enrolled")`. `subject_did` stays
  `owner_did_or_node(host)`.
- Import: replace the unsigned `bundle.check_integrity()` path with
  `syneroym_roym_core::backup::check_signed_bundle(&bundle, &owner, now)`
  (same as catalog `backup.rs:97`). Map the four new section names to
  their collections in the `match name.as_str()` (line ~107). After the
  writes, call `search_ops::rebuild_search_index` (already done) **and**
  `standing::rebuild_all`.
- Create the new collections (with their indexes) before writing, the
  same way the existing loop does.

---

## §6 Directory — client half (WO3)

### 6.1 Pinned issuer (`app/client_sources.rs`)

`SourceRow` gains:

```rust
    /// D-C9-4. Absent until this node learns it; never changed by a reply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(in crate::app) issuer_did: Option<String>,
```

`load_source_row_or_default` (in `client_query.rs:385`) sets `issuer_did: None`.

`add_source` pseudo-code change:

```text
explicit = params.issuer_did (optional; if present must be did:key -> else invalid_params)
existing_row = get SOURCES[did]
probe as today; info_issuer = resp.result.issuer_did (string) when present
issuer_did = explicit
          .or(existing_row.issuer_did)      // re-adding never re-pins silently
          .or(info_issuer)
row.issuer_did = issuer_did
note: if explicit.is_some() && info_issuer.is_some() && explicit != info_issuer:
      probe_note = "this directory says it signs as {info_issuer}; you chose {explicit}"
```

`mark_source_ok` / `record_source_error` must keep `issuer_did` (they
load-then-modify, so they already do; add a test).

### 6.2 Verdict on each search hit (`app/client_query.rs`)

`SearchRunRow`: replace `pub(in crate::app) credential: String,` with

```rust
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(in crate::app) membership: Option<MembershipVerdict>,
```

`query_source` loads the `SourceRow` it already validates in
`validate_run_and_source` — change that function to return
`Result<SourceRow, Response>` and pass `row.issuer_did.as_deref()` down.

`store_verified_hit` computes:

```rust
    let areas = payload.location.as_ref().map(|l| l.service_area.clone()).unwrap_or_default();
    let verdict = membership::evaluate(&hit.membership, &CheckInput {
        pinned_issuer,
        member_did: &issuer,                 // verdict.issuer (the listing's issuer)
        listing: Some(ListingRef { listing_id: &listing_id,
                                   categories: Some(&payload.categories),
                                   areas: Some(&areas) }),
        now_secs: now,
        evidence_as_of_secs: now,
    });
```

and stores `membership: Some(verdict)`. It also calls
`held::remember(host, source, &issuer, pinned_issuer, &hit.membership, now)`
(best-effort; a failure there must not drop the hit).
`store_refused_hit` stores `membership: None`.

`pinned_issuer == None` gives `Unknown{issuer-not-pinned}`; that is the
honest answer for a source added before this slice.

### 6.3 Merge (`app/client_merge.rs::build_hits`)

- Each `sources[]` entry gains `"membership": r.membership`.
- Remove the top-level `"credential": winner.credential` field.
- Nothing else changes; the round-robin is unchanged.

### 6.4 `directory.check-standing` and `directory.memberships` — `app/held.rs`

Dispatch:

```rust
        "directory.check-standing" => held::check_standing(host, &req).await,
        "directory.memberships" => held::memberships(host, &req).await,
```

`check_standing`, params `{ source, member_did }`:

```text
row = get SOURCES[source] -> None => invalid_params("source is not in this person's own sources")
call CallTarget::Service(source), DIRECTORY interface, "invoke",
     {"method":"directory.standing","params":{"member_did": member_did}},
     CallOptions { idempotent: true, timeout_ms: Some(DEFAULT_SOURCE_TIMEOUT_MS), .. }
on transport/parse/JSON-RPC error:
    record last_error on the HELD row (keep its evidence and as_of); return
    ok { verdict: evaluate(stored evidence, stored as_of), as_of_secs, refreshed: false, error }
    (no stored row -> verdict Unknown{reason: "could not reach this directory"})
reply.issuer_did != row.issuer_did (when pinned) ->
    return ok { verdict: Unknown{reason:"issuer-changed"}, refreshed: false } ; do NOT store
now = clock::now_secs()
remember(host, source, member_did, row.issuer_did, &reply.evidence, now)
ok { verdict: evaluate(&reply.evidence, {pinned: row.issuer_did, member_did, listing: None,
                                          now, as_of: now}),
     as_of_secs: now, refreshed: true }
```

`memberships`, params `{ member_did? }`: list `HELD_MEMBERSHIPS` (filtered
when `member_did` is given); for each row return
`{ source, member_did, issuer_did, as_of_secs, last_error, verdict }` where
`verdict` is **recomputed now** from the stored evidence. Recomputing on
read is what makes "verification status reproduces after import" true by
construction (failure-matrix row 13): the import writes the evidence
bytes, and the verdict is derived, never stored.

`remember(...)`: upsert `HELD_MEMBERSHIPS["{source}#{member_did}"]`,
`last_error: None`. Skip writing when `evidence` is empty *and* no row
exists (a non-member produces no cached copy).

---

## §7 Enrolment surface — four lists (WO4)

`directory` joins every `SIGNING_SERVICES` list. All four must change in
the same commit (the router test compares the Rust expectation with the
UI list):

| File | Change |
|---|---|
| `apps/roymctl/src/commands/roym/signing.rs:12` | add `"directory"` |
| `crates/roym_web/ui/src/session/enrolment.ts:7` | add `"directory"` |
| `crates/substrate/tests/common/roym.rs:35` | add `"directory"` |
| `crates/roym_core/src/router.rs` test `expected` | add `"directory"` (§3.5) |

Consequence to state in `status.md`: every Hub now needs `directory`
enrolled before its setup gate opens, including a consumer who never
runs a SynOrg. That is the cost of `directory.export` being signed.
`roymctl roym enrol-signing` already loops the list, and the Playwright
global setup calls it (`global-setup.ts:405`), so no setup change there.

Also check `crates/substrate/tests/e2e/tests/roym-hub.spec.ts` case 32
(transaction unenrolled gate) still passes — it intercepts
`transaction.signing-status` only.

### 7.1 `roymctl`

New file `apps/roymctl/src/commands/roym/trust.rs` (handlers only).
`DirectoryCommands` in `apps/roymctl/src/commands/roym/directory.rs` gains
variants that call into it:

```rust
    /// Issue, list, or revoke membership credentials (SynOrg owner).
    Credential { #[command(subcommand)] command: CredentialCommands },
    /// Fetch a member's standing from a source and check it on this node.
    Standing { #[arg(long)] source: String, #[arg(long)] member: String },
    /// The membership checks this node holds, re-evaluated now.
    Memberships { #[arg(long)] member: Option<String> },
```

`CredentialCommands`: `Issue { member, category: Vec<String>, area_json: Option<String>, expires_days: u64 (default 365), note: Option<String> }`,
`List { member: Option<String> }`, `Revoke { credential: String, reason: String }`.

`MemberCommands` (existing, `directory.rs:117`) gains
`Suspend { member, rule, reason, listing: Option<String>, until_secs: Option<u64> }`,
`Lift { decision: String, reason: String }`, `Decisions { member: Option<String> }`.

`Add` gains `#[arg(long)] issuer: Option<String>` → `issuer_did` param.

`Find` output: print one line per source with `membership::verdict_word`
(roymctl may depend on `syneroym-roym-core`; check `apps/roymctl/Cargo.toml`
first — if not already a dependency, print the `state` string instead of
adding one).

Add CLI parse tests for every new subcommand in
`apps/roymctl/src/commands/roym/tests.rs` (this also closes part of the
C6 backlog row "`find`, `serve`, `member` have no automated test"; add
`find`/`serve`/`member add` parse tests while there and move that row to
"Recently resolved").

---

## §8 Tests — dual-build parity (WO5)

`crates/roym_web/tests/dual_build_parity/directory.rs` is at its capped
length (1137, `xtask/oversized-test-files.txt`). **Do not grow it.** New
scenarios go in a new file `crates/roym_web/tests/dual_build_parity/trust.rs`
(≤ 800 lines; split into `trust.rs` + `trust_client.rs` if needed) with
its own fixtures in `trust_fixtures.rs` (`fixtures.rs` is at 792 lines).
Register the modules in `crates/roym_web/tests/dual_build_parity.rs`.

Rebuild the WASM artifacts first (`mise run build:roym`) — the parity
suite loads pre-built components and otherwise runs stale code.

### 8.1 Fixture changes (publish gate, Q2)

In `fixtures.rs` (keep it ≤ 800 lines; move helpers to `trust_fixtures.rs`
if it would pass):

- `ensure_synorg(h)`: after setting settings, `enrol_signing(h, "directory")`
  and issue a credential to `owner_did()` and to `peer_did()` covering the
  categories the existing listing fixtures use. Find every category string
  used by `full_listing_params` and `publish_listing_to_*` and put all of
  them in the SynOrg's `categories` too.
- `ensure_dir2_synorg(h)`: same for the second directory (its own owner
  and its own enrolment through `h.dir2_local`).

Then run the whole directory module. Any scenario that still fails is one
whose publisher is neither the owner nor the peer — fix it by issuing a
credential in that scenario, not by weakening the gate.

### 8.2 Scenarios to rewrite

| Scenario | File | Change |
|---|---|---|
| 118 "exactly three directory verbs are wire-reachable" | `directory.rs:640` | Four, including `directory.standing`. Rename the fn to `scenario_118_exactly_four_directory_verbs_are_wire_reachable_parity` |
| 93 "a search response carries no verification verdict" | `directory.rs:392` | Still true: assert `membership` is evidence (arrays of strings) and no `state`/`verified` key appears anywhere in a hit |
| 97 / 98 / 102c / 102d / 119 | `directory.rs` | Any assertion on `hit["credential"]` becomes `hit["sources"][i]["membership"]["state"]` |
| 117 export/import round trip | `directory.rs:849` | Schema version 4; bundle is now signed |
| 170 "directory export unsigned and imports" | `bundles.rs:294` | Rename to `scenario_170_directory_export_is_signed_and_imports_parity`; assert `manifest_signature` present and verifies (`verify_and_strip_manifest_signature`), and an unsigned copy is refused |

### 8.3 New scenarios (173–191) in `trust.rs`

Each scenario runs on both builds and compares with `stripped(...)`.

| # | Name (fn `scenario_N_..._parity`) | Asserts |
|---|---|---|
| 173 | `credential_issue_signs_under_the_owner` | Envelope verifies, issuer = `owner_did()`, subject = member, expiry set; byte-identical across builds |
| 174 | `credential_issue_refuses_bad_input` | non-did member; expiry past / too far; category not in SynOrg; directory not enrolled → `signing-not-enrolled` |
| 175 | `standing_over_the_wire_is_open_and_bounded` | anonymous `directory.standing` works; unknown member → empty evidence; evidence arrays ≤ caps after issuing 6 credentials |
| 176 | `publish_by_a_non_member_is_refused_then_admitted_after_issue` | `data.admission == "not-admitted"`, `data.membership.state == "none"`; then ok |
| 177 | `publish_out_of_scope_is_refused` | `data.membership.state == "out-of-scope"` |
| 178 | `a_withdrawal_is_accepted_from_a_suspended_member` | withdraw ok while suspended |
| 179 | `revocation_removes_the_member_from_search` | hit present → `revocation.issue` → gone; revoke twice is idempotent |
| 180 | `suspend_hides_and_lift_restores` | membership scope |
| 181 | `a_listing_scoped_suspension_hides_only_that_listing` | two listings, one hidden |
| 182 | `a_consumer_sees_valid_membership_per_source` | two-directory harness: `sources[0].membership.state == "valid"`, issuer pinned from `info` |
| 183 | `a_forged_credential_is_refused_on_the_consumers_node` | `hostile_source_response` serves a credential signed by `peer_identity()` → `refused` (matrix row 1/2) |
| 184 | `a_directory_asserting_an_expired_credential_does_not_win` | hostile source serves an expired credential (sign via raw `Envelope::unsigned` with past `issued_at`/`expires_at`, pinned issuer = dir2 owner) → `expired` |
| 185 | `a_directory_asserting_an_out_of_scope_credential_does_not_win` | → `out-of-scope` naming the category |
| 186 | `a_held_copy_shows_the_withdrawal_on_next_check` | search (valid) → owner suspends → `memberships` still `valid` (stale copy, honest `as_of_secs`) → `check-standing` → `suspended`; after revoke → `revoked` |
| 187 | `a_changed_issuer_is_never_re_pinned` | source added with explicit `issuer_did` X; its info names Y → search membership `unknown` reason `issuer-changed`… (driven through the hostile harness) |
| 188 | `trust_state_round_trips_through_a_signed_export` | export → import on a wiped store → `credential.list`, `revocation.list`, `member.decisions`, `memberships` verdicts identical; search filter still hides the revoked member (standing rebuilt) |
| 189 | `member_remove_is_refused_while_a_credential_is_valid` | then ok after revoke |
| 190 | `lift_of_a_non_suspension_is_refused_and_lift_is_idempotent` | |
| 191 | `search_reply_with_full_page_of_evidence_fits_the_proxy_limit` | the §5.7 size check |

If the hostile harness (`helpers.rs:317 hostile_source_response`) cannot
serve an arbitrary `directory.standing`/search reply for 183–187, extend
it in a new helper file rather than growing `helpers.rs` (capped at 1494).

---

## §9 Tests — the inherited cross-node cases (WO7, `D-06C-7`)

Source table: `docs/planning/milestones/M06B-roym-substrate-foundations/slice-b4-implementation-plan.md` §10.2.

### 9.1 Row-by-row disposition

| B4 §10.2 row | Case | Covered today? | C9 action |
|---|---|---|---|
| 1, 2, 3, 5, 6 | pending/restart/delivered/never-delivered-while-down/no-broker-leak | Yes — `conversation_e2e.rs` | none |
| 4 | dropped ack → retry, one copy at B | No | new test, needs Q5 hook `drop_next_acks` |
| 7 | node C delivers an envelope whose `author` claims A | No (unit only: `transport.rs:229`) | new test, needs Q5 hook `override_next_outgoing(author)` |
| 8 | a peer re-presents a different signing key for a pinned address | No | new test, **no hook**: deploy the same fixture service (same member master) on a second node, so its instance key differs, and send to B → refused, B's pin unchanged |
| 9 | guest calls `conversation` on another service via proxy | No | new test: fixture op `ProxyCallCrossServiceNative` targeting B's service, interface `conversation`, method `deliver` → refused by the capability gate |
| 10 | same-service exemption (`D-B4-26`) | No (unit/parity only) | new test: fixture op `ProxyCallSelf` with `conversation/deliver` → reaches the arm, refused `PermissionDenied` (`transport.rs:223`) |
| 11 | `prekey-bundle` past the per-peer hourly limit | No | new test: B with `conversation_prekey_requests_per_peer_per_hour = 2`; a stranger `SyneroymClient` calls `conversation/prekey-bundle` 3 times → third refused; then A still establishes a session (pool not drained) |
| 12 | per-conversation quota isolation | No | new test: A with `conversation_max_pending_per_conversation = 2`, B down; conv1: 2 sends ok, 3rd `quota-exceeded`; conv2 (to a third address) still sends |
| 13 | `sender_timestamp` a year in the future refused; a year in the past accepted | No | new test, needs Q5 hook `override_next_outgoing(sender_timestamp_ms)` |
| 14 | peer offline past `max_pending_age_secs` → `failed`; `retry` re-arms | Half: `roym_conversation_e2e.rs:455` proves `failed` | extend that test with `conversation.retry` → back to `pending` |
| 15 | send with no installed instance certificate → terminal failure naming the certificate | No | new test: deploy the fixture without `certify_instance`; send → outbox `failed` with a reason containing `instance certificate` (`transport.rs:69`) |
| 16 | alias canonicalization | — | **excluded** (`D-B4-29`, `D-06C-7`) |
| 17 | `open-direct` on an address that resolves to nothing, refused at the call | — | **excluded — flag**: this is also `D-B4-29` and is not implemented (`crates/conversation/src/lib.rs:361` does not resolve). `D-06C-7` names only row 16; row 17 has the same cause and must share its backlog row |

### 9.2 Files

- Move `publish_endpoint`, `deploy_fixture`, `publish_master_anchor`,
  `fixture_run`, `wait_until`, `fixture_wasm`, `FIXTURE_INTERFACE`, and
  `fast_conversation_role` out of `crates/substrate/tests/conversation_e2e.rs`
  into `crates/substrate/tests/common/conversation_fixture.rs` (declare in
  `common/mod.rs`). `conversation_e2e.rs` then uses them. `deploy_fixture`
  gains a `certify: bool` parameter for row 15.
- New `crates/substrate/tests/conversation_cross_node_e2e.rs`: one
  `#[tokio::test]` per row (4, 7, 8, 9, 10, 11, 12, 13, 15), each starting
  with `let _serial_guard = common::serial_guard().await;`. Each test ≤ 100
  lines; file ≤ 800 lines (split into `conversation_abuse_e2e.rs` for rows
  7, 8, 13 if needed). Ports only from `SubstrateNode`/`alloc_ports`.
- Put the new binary in the nextest `substrate-e2e` group — check
  `.config/nextest.toml`'s filter; if it matches by name pattern, no edit.

### 9.3 The Q5 hooks (only if Q5 = hooks)

`crates/conversation/Cargo.toml`:

```toml
[features]
test-support = []
```

`crates/substrate/Cargo.toml` `[dev-dependencies]`:
`syneroym-conversation = { workspace = true, features = ["test-support"] }`.

New file `crates/conversation/src/test_support.rs`, declared in `lib.rs`
as `#[cfg(feature = "test-support")] pub mod test_support;`:

```rust
//! One-shot misbehaviour for cross-node tests. Compiled only with the
//! `test-support` feature, which only `syneroym-substrate`'s dev build
//! enables.

pub struct OutgoingOverride { pub author: Option<String>, pub sender_timestamp_ms: Option<i64> }

/// The next `n` deliveries this process *receives* are stored and then
/// answered with an error instead of an ack.
pub fn drop_next_acks(n: u32);
/// The next delivery this process *sends* uses these values in its
/// signed payload.
pub fn override_next_outgoing(o: OutgoingOverride);

pub(crate) fn take_drop_ack() -> bool;          // AtomicU32 decrement
pub(crate) fn take_outgoing_override() -> Option<OutgoingOverride>; // Mutex<Option<_>>
```

Call sites (each wrapped in `#[cfg(feature = "test-support")]`):

- `transport.rs::peer_deliver_impl` (`:263`): after the store commit and
  before building the ack → `if test_support::take_drop_ack() { return Err(ConversationError::Unreachable("test: ack dropped".into())) }`.
- `transport.rs::deliver_one` (`:112`): where the `DeliveryPayload` is
  built (`:160`), apply `take_outgoing_override()` to `author` and
  `sender_timestamp_ms` *before* signing, so the envelope is validly
  signed by the real sender key and only the claims are false.

**Process-global caveat:** the hook statics are process-wide, and nextest
runs one test per process, so this is safe under nextest. Under plain
`cargo test` two tests in one binary could race — the serial guard
already serializes the tests in this binary; keep every hook use inside
the guard.

---

## §10 Tests — three installations (WO6, R3 rows 1–3)

### 10.1 Shared flow helpers

`crates/substrate/tests/roym_booking_e2e.rs` holds private helpers the new
test needs (`open_request_conv`, `provider_conv_for`,
`exchange_quotes_and_accept`, `complete_winner_lifecycle`,
`assert_no_directory`). Move the generic parts to a new
`crates/substrate/tests/common/roym_flow.rs` (declare in
`common/mod.rs`), parameterised so both files call them. Do not copy
them — `cargo xtask check-duplication` will fail. From
`roym_directory_e2e.rs`, move `run_client_loop`, `listing_envelope`,
`deliver_one_message` the same way.

### 10.2 New `crates/substrate/tests/roym_trust_e2e.rs`

One `#[tokio::test]`,
`a_consumer_hires_a_member_found_through_a_synorg_on_a_third_installation`,
calling step helpers (each ≤ 100 lines). Three `RoymNode`s: Z hosts the
registry; X and Y share it (`roym_directory_e2e.rs` explains why only one
registry). Every node: `full_bring_up()` (which enrols all five services).

Steps and assertions:

1. **Z** `directory.set-settings` (name, rules, categories `["cycling"]`,
   area, support contact, dispute path, retention).
2. **Y** creates profile + a listing in `cycling` + availability.
3. **Y** `directory.publish-to-source` Z → refused, `admission: not-admitted`,
   `membership.state: none` (member-only publish, Q2).
4. **Z** `credential.issue { member_did: Y.owner_did, categories: ["cycling"], expires_at_secs: now+30d }`.
5. **Y** publish again → ok.
6. **X** `directory.add-source { did: Z_directory_service_did }` → returned
   `source.issuer_did == Z.owner_did`. X learns nothing else about Y or Z:
   Y's addresses come only from the search hit (resolution through the
   registry — assert X's node has no endpoint for Y before this step by
   checking the proxy fails, or simply that the test passes no Y address
   to X).
7. **X** client loop (search `cycling`) → exactly one hit; its
   `sources[0].membership.state == "valid"`, `issuer == Z.owner_did`,
   `scope.categories == ["cycling"]`, `expires_at_secs` as issued. This is
   R3 row 2: computed on X by `store_verified_hit`, not by Z.
8. **X ↔ Y** the R1+R2 flow through `common::roym_flow`: conversation from
   the hit's `conversation_address`, request → quote → accept → booking →
   `payment.request` → both `payment.acknowledge` → both `fulfilment.sign`
   → booking `completed`. Reuse the booking e2e's waits.
9. **Z** `member.suspend { member_did: Y, rule: "r1", reason: "test" }`.
10. **X** search again → zero hits (R3 row 3, first half).
11. **X** `directory.memberships` → the held copy still says `valid`, with
    its old `as_of_secs` (the product does not claim instant removal).
    Then `directory.check-standing { source: Z, member_did: Y }` →
    `suspended`, `refreshed: true` (second half).
12. **Z** `revocation.issue` on the credential; X `check-standing` → `revoked`.
13. **X** `directory.export` and `transaction.export`; boot a clean node
    X′ with X's owner identity (use the restore helper pattern from
    `roym_restore_e2e.rs`), import both → X′ `directory.memberships`
    verdict for Y is `revoked` (verification status reproduces), and X′'s
    agreement still verifies.

Budget: this is the heaviest e2e in the workspace. Measure its wall time
once and record it in `status.md`; if it passes ~4 minutes, split step 13
into its own test.

### 10.3 Existing tests the publish gate breaks (Q2)

| Test | Fix |
|---|---|
| `crates/substrate/tests/roym_directory_e2e.rs` | Z issues Y a credential before Y's first publish; the "publication past the SynOrg's limit" step needs the credential too. The stranger `VerifiedOnly` publish step now gets `not-admitted` instead of success — update the assertion and its doc comment (the stranger is admitted by the wire rule and then refused by membership) |
| `crates/substrate/tests/e2e/tests/roym-hub.spec.ts` cases 13–23b | In the directory `beforeAll`, call `credential.issue` for the node's own owner (loopback source) before any publish. Case 21 ("publishes a listing") needs it; check each case that publishes |
| parity scenarios | §8.1 fixtures |

---

## §11 Hub (WO8)

Files and changes:

| File | Change |
|---|---|
| **new** `crates/roym_web/ui/src/directory/membership.ts` (+ `membership.test.ts`) | `NO_INSTANT_REMOVAL_NOTICE`, `WITHHELD_REVOCATION_NOTICE` (verbatim copies of §3.2); `type MembershipVerdict` (mirror of the Rust enum); `membershipWords(v, sourceLabel): string`. Words: `valid` → "Member of {name}, checked on your node. Group withdrawals checked {age} ago."; `none` → "No membership shown by {source}"; `unknown` → "Membership: unknown ({reason})"; `refused` → "Membership evidence did NOT check out — treat as unknown"; `expired`, `out-of-scope` (names categories), `revoked`, `suspended` (names rule, since, until). **Never the word "verified"** (vitest asserts it for every variant). Trust-on-first-use issuer is worded "the group this directory said it is when you added it" |
| `crates/roym_web/ui/src/directory/search.ts` | `credential: string` → per-source `membership?: MembershipVerdict` |
| `crates/roym_web/ui/src/screens/directory.ts:352` | replace `"membership: not checked"` with one line per source from `membershipWords`; add a "Check membership again" button per source → `directory.check-standing` |
| **new** `crates/roym_web/ui/src/screens/memberships.ts` | the held copies (`directory.memberships`): source, member, verdict words, "checked {age} ago", "Check again", and `NO_INSTANT_REMOVAL_NOTICE` + `WITHHELD_REVOCATION_NOTICE` always visible. Add a tab in `main.ts` next to Directory |
| **new** `crates/roym_web/ui/src/screens/synorg_members.ts` | called from `synorg.ts`'s roster section: issue credential (categories chosen from the SynOrg's own, expiry days), list credentials, revoke with reason, suspend (membership or one listing, rule, reason, optional until), lift, decision history. Show `NO_INSTANT_REMOVAL_NOTICE` next to suspend and revoke |
| `crates/roym_web/ui/src/session/enrolment.ts` | §7 |

All values are rendered with `textContent` (the existing `text()`
helper). No `innerHTML` anywhere (a grep in the vitest suite already
exists for cards; add `membership.ts` and the two new screens to it if it
is a file list).

Playwright: new file `crates/substrate/tests/e2e/tests/roym-trust.spec.ts`
(do not grow `roym-hub.spec.ts`, 1182 lines). Single node, own directory
as a loopback source (the C6 cases' pattern). Test names are descriptive,
no planning ids:

1. `a listing from a non-member is refused with the reason in words`
2. `the SynOrg issues a credential and the search result shows the membership checked on this node`
3. `suspending the member removes the result and the held copy shows the suspension only after "check again"` (asserts the notice text verbatim)
4. `lifting the suspension brings the result back`
5. `revoking the credential shows "revoked" on the next check`
6. `no membership line ever uses the word "verified"`

---

## §12 Backlog and documents (WO9)

### `docs/planning/deferred-backlog.md`

Move to "Recently resolved":

- `directory.export does not sign its bundle manifest in C8` (§5.9).
- The 13-uncovered-cross-node-cases row (`deferred-backlog.md:213`), minus
  rows 16 and 17.
- `fct` claims row (`:163`) — resolved by decision Q7: "C9 uses no
  cross-installation `fct`; cross-install trust is signed records only."
- The C6 row about `roymctl roym directory find`/`serve`/`member` having
  no CLI test (§7.1).

Retarget (Q6):

- Hub transaction action panel + Playwright 33–39 (`:431`) → `TBD`, trigger
  "before the Hub is used by anyone but its developers".
- `conversation.history` reconciliation (`:222`) → `TBD`, trigger unchanged
  ("a missed inbound message is observed"); note that C9's harness did not
  inject inbox failures.

New rows:

| Row | Target | Link |
|---|---|---|
| Issuer-key revocation is not checked on any record (`check_did` stays `Unknown`; registry master anchor not guest-reachable) | TBD | `crates/signed_record/src/verify.rs:13`, `roym_core::membership` |
| A withheld revocation cannot be detected; no signed revocation-list snapshot | TBD, trigger "a directory serves credentials it did not issue" | §2 D-C9-6 |
| Credential area scope uses *intersects*, not *contains* | TBD | `roym_core::membership::area_within` |
| Issuer pin is trust-on-first-use; no out-of-band issuer verification flow in the Hub | TBD | §2 D-C9-4 |
| No "acts-for" delegation: a SynOrg's issuer is its deployer | TBD, trigger "a SynOrg with more than one administrator" | §2 D-C9-1 |
| `open-direct` does not refuse an unresolvable address (B4 row 17) — shares `D-B4-29` | same as the alias row | `crates/conversation/src/lib.rs:361` |
| (only if Q5 = unit-only) rows 4, 7, 13 covered at crate level only | TBD | §9 |
| Credential renewal is a manual re-issue; no expiry reminder | TBD | `credential_ops::issue` |

### Other documents

| Document | Edit |
|---|---|
| `CLAUDE.md` / `AGENTS.md` architecture paragraph | "a named three-method table" → four, adding `directory.standing`; name the `credential.*`/`revocation.*`/`member.suspend`/`member.lift` verbs as local-only |
| `docs/roym-integrated-experience-spec.md` | R3 marked **Passed** with slice owner (only after the acceptance tests pass); in the Records table note under `revocation`/`moderation-decision`: "a suspension is a `moderation-decision`; a revocation is permanent; the consumer's check shows either" (Q4); Search section: the issuer pin |
| `task.md` | C9 row → Complete with evidence pointer; D-06C-7's "13" corrected to the real row list (§14 item 1); "Documents this milestone edits" row for C9 |
| `status.md` | C9 section: what shipped, evidence table (same shape as C8's), the enrolment-gate consequence (§7), the e2e wall time |
| `slice-c3-implementation-plan.md` §18 note E | one dated line: key revocation was not supplied by C9 (D-C9-8) |

---

## §13 Work orders and order of execution

Work on a branch: `git checkout -b feat/m06c-slice-c9` (commits are
banned on `main`).

| WO | Content | Done when |
|---|---|---|
| WO1 | §3 (roym_core) | `cargo nextest run -p syneroym-roym-core` green, incl. the 15 evaluator tests |
| WO2 | §4, §5 (directory server half) | `cargo clippy -p syneroym-roym-directory` clean; WASM builds (`mise run build:roym`) |
| WO3 | §6 (client half) | same |
| WO4 | §7 (enrolment lists, roymctl) | `cargo nextest run -p syneroym-roym-core router` + roymctl tests green |
| WO5 | §8 parity | `cargo nextest run -p syneroym-roym-web --test dual_build_parity` all green on both builds (167 old + 19 new) |
| WO6 | §10 three-install e2e + §10.3 fixes | `roym_trust_e2e`, `roym_directory_e2e`, `roym_booking_e2e` green |
| WO7 | §9 inherited cases (after Q5 answer) | new binary green |
| WO8 | §11 Hub + Playwright | vitest, `npm run build`, `mise run test:e2e` green |
| WO9 | §12 docs/backlog | — |
| Final | `mise run verify` in the background, no `--skip`; import cleanup pass on every edited file; `cargo dupes report` filtered to changed files; planning-ref grep | all gates pass |

WO1 → WO2 → WO3 are strictly ordered. WO4 can follow WO2. WO5 needs
WO1–WO4. WO6 needs WO1–WO4. WO7 is independent of WO1–WO6 and can run in
parallel. WO8 needs WO3.

**Function-length watch list** (100-line limit): `publication_ops::publish`
is already long — put the gate in `require_member` (§5.6), not inline.
`search_ops::search` — put the filter in `standing_by_issuer`/`listed`.
`backup::import` — move the section→collection `match` into a
`fn collection_for(section: &str) -> Option<&'static str>` helper.

---

## §14 Things in the docs that look stale or ambiguous

1. **The "13 uncovered cases" count does not match its own list.**
   `task.md` D-06C-7 and M06B `status.md:815` name eight categories
   (including alias); counting B4 §10.2 rows that no test covers gives 12
   (rows 4, 7–17). The M06B ledger's "rows 7–10 are B5's" refers to the
   **failure matrix**, a different table from the §10.2 test list. This
   plan works from the §10.2 rows (§9.1).
2. **C3 plan §18 note E says "C9 supplies the real source for both"**
   (key and credential revocation). Key revocation is in the registry's
   master anchor and no guest can reach it. C9 supplies credential
   revocation only (D-C9-8).
3. **`D-C3-3` expects a SynOrg to sign under a dedicated org master** that
   is not the administrator. C4's `signing::install` refuses any master
   that is not the recorded owner, and the spec's "one person per
   installation" rule agrees. The dedicated-identity path works only when
   that identity is the deployer (Q1).
4. **The spec's R3 contract says "signed revocation list";** the Records
   table has a per-credential `revocation` and no list type (D-C9-6).
5. **R3 row 3 says a suspended member's cached copies "show the
   revocation".** A suspension is a `moderation-decision`, not a
   `revocation` (Q4).
6. **The spec's Directory API column lists `member.*`, `credential.*`,
   `revocation.*`** and no moderation prefix; this plan puts suspend/lift
   under `member.` to stay inside it.
7. **`task.md` exit criterion 6 / `D-06C-6a`: "no Directory deployed
   anywhere"** cannot be literally true — `roym.toml` deploys `directory`
   on every node. C8 already tests it as "no directory source configured"
   (`roym_booking_e2e.rs:394 assert_no_directory`). This plan keeps that
   reading; the task text should say so.
8. **B4 §10.2 row 17** is the same unimplemented `D-B4-29` as row 16, but
   `D-06C-7` excepts only "alias canonicalization".
9. **`CLAUDE.md`'s "named three-method table"** becomes wrong in this slice.
10. **`SynOrgSettings`'s doc comment** ("`directory` mounts no signing
    certificate in this slice") becomes wrong in this slice (§3.3).
11. **`task.md` exit criterion 14 names `cargo test --workspace`;** the
    repo now runs `cargo nextest run --workspace` + doctests through
    `mise run verify`.
12. **Backlog `:431`** targets the Hub transaction panel at "C9 /
    follow-on", but nothing in R3 needs it (Q6).
13. **The C6 SearchHit's `directory` field** is the directory's
    *signing* DID (`search_ops.rs:201`), not the service DID the consumer
    addressed, and not the issuer. The Hub must not present it as the
    SynOrg's identity; the issuer is the pinned `issuer_did`.

---

## §15 From-scratch demo script

This section describes the finished feature as a person sees it. It is
written so a reviewer can judge the feature without reading the code, and
so it can be run by hand as a manual acceptance check after WO8. It shows
the key use cases only. A refusal appears only where the refusal itself
is the feature: members-only publishing, a check done on the consumer's
own node, and withdrawals. Input validation and other edge cases are left
to the tests in §8–§10. Each heading names the automated test that proves
the same thing, so a manual run and the test suite can be compared.

Words in *italics* are the exact text the Hub shows (§3.2, §11).
Commands marked **(new)** are added by this slice (§7.1); the rest exist
today.

### 15.0 Setup — three people, three installations

- Three machines (or three substrates on one machine with separate data
  directories and ports): **X** is the consumer, **Y** the provider, **Z**
  the SynOrg owner. Each person creates their own identity with
  `roymctl identity` and starts their own substrate. Z's node hosts the
  registry; X and Y point at it. Nothing else is shared between them.
- Each person deploys the Roym app on their own node:
  `roymctl app deploy roym crates/roym_core/app/roym.toml --mint-masters --registry-url <url>`.
- Each person runs `roymctl roym enrol-signing` once. It now reports
  **five** services, including `directory:` (§7).
- Each person opens their Hub in a browser and logs in. The rest of the
  demo uses the Hub. A `roymctl` equivalent is given for each step,
  because the API is the product boundary (exit criterion 2).

### 15.1 Z starts a group and admits Y (S2–S6) — *`trust.rs` 173; `roym_trust_e2e` steps 1, 4*

- Z opens the **SynOrg** tab and fills in name "Bengaluru Cycle Guild",
  rules, area, category `cycling`, support contact, dispute path, and
  retention. Saving shows the settings back.
- Out of band (a chat, a phone call), Y gives Z their DID. Z opens
  **Members → Issue credential**, pastes Y's DID, picks the category
  `cycling` from the group's own list, and sets an expiry of 365 days.
- Z sees a new credential row: member, categories, expiry date, and its
  record id. Next to it: *"A group's decision reaches copies other
  people already hold only when they next check. Nobody can promise it
  is removed everywhere at once."*
- CLI: `roymctl roym directory credential issue --member <Y-did> --category cycling --expires-days 365` **(new)**.

### 15.2 Y publishes a listing to Z's directory — members only (S7) — *`trust.rs` 176; `roym_trust_e2e` steps 2–5*

- Y creates a profile and a listing "Bike repair at home" in category
  `cycling`, with availability. Y is already reachable by direct link;
  no directory is involved yet.
- If Y publishes to Z's directory **before** 15.1, it is refused with
  words: *this SynOrg does not admit this listing: none*. Y holds no
  credential from Z. The listing stays published on Y's own node.
- After Z issues the credential, Y presses **Publish** again → accepted.

### 15.3 X adds Z's directory and searches (C4–C9) — *`trust.rs` 182; `roym_trust_e2e` steps 6–7*

- X got Z's directory address from a friend. X opens **Directory → Add
  source** and pastes it. X may also paste the group's issuer DID if the
  friend gave it. If not, the Hub pins the one the directory states now,
  and says so: the group is shown as *"the group this directory said it
  is when you added it"*.
- X searches for `cycling`. One result: "Bike repair at home", with its
  source (Z's directory) and its age.
- Under the result, per source: *"Member of Bengaluru Cycle Guild,
  checked on your node. Group withdrawals checked just now."* It shows
  the scope (`cycling`) and the expiry date. X's own node did this check
  from Z's signed records. Z's directory sent evidence, not a verdict.

### 15.4 X hires Y across three installations (R3 row 1) — *`roym_trust_e2e` step 8*

- X presses **Message this provider** on the result. The conversation
  opens to Y's address taken from Y's signed listing. X never typed an
  address for Y, and X's node found Y's node through the registry.
- The full R1+R2 flow then runs as it did in C8, now across three
  installations: request card → quote card → accept (agreement receipt)
  → book a slot → Y requests payment → both acknowledge payment → both
  sign the fulfilment receipt → the booking shows completed.
- Z is not in this path at all. Z's node can be switched off after 15.3
  and the flow still completes.

### 15.5 Z suspends Y; the result vanishes; X's copy updates only on check (R3 row 3) — *`trust.rs` 180, 181, 186; `roym_trust_e2e` steps 9–11*

- Z opens **Members → Y → Suspend**, enters rule "r1 — no-shows" and a
  reason, and leaves "until" empty. Z sees a signed decision with a time,
  and the same no-instant-removal sentence.
- X searches `cycling` again → **no result** from Z's directory.
- X opens the **Memberships** tab. The copy X already holds still says
  *valid*, with *checked 5 minutes ago*. It does not change by itself.
  The page always shows both notices: no instant removal, and *"This
  shows every withdrawal the group has published. It cannot show one the
  group chose not to publish."*
- X presses **Check again** → *Suspended by Bengaluru Cycle Guild on
  <date> under rule "r1 — no-shows"*, with *checked just now*.
- CLI: `roymctl roym directory memberships` and
  `roymctl roym directory standing --source <Z> --member <Y>` **(new)**.
- Z presses **Lift** on the decision → X's next search shows Y's listing
  again, and **Check again** shows *valid*.
- Variant: Z suspends **one listing** only → that listing disappears
  from search, and Y's other `cycling` listings stay.

### 15.6 Z revokes Y's credential (S13) — *`trust.rs` 179; `roym_trust_e2e` step 12*

- Z presses **Revoke** with a reason → a signed revocation row appears.
- X presses **Check again** → *Revoked by Bengaluru Cycle Guild on
  <date>: <reason>*.
- Y publishing again → refused *… revoked*. Y must get a new credential
  from Z first.

### 15.7 A dishonest directory cannot make X trust it (matrix rows 1, 2) — *`trust.rs` 183–185*

This is the core promise of R3: finding is separate from trusting. It
cannot be clicked through with honest nodes, so it is shown with the
parity harness's hostile directory. What X sees in each case:

- A credential signed by someone other than the pinned group →
  *Membership evidence did NOT check out — treat as unknown*.
- A credential that expired last week, served as if it were current →
  *Membership expired on <date>*. The directory's claim does not win.
- A credential for `plumbing` attached to a `cycling` listing →
  *Membership does not cover: cycling*.

### 15.8 X leaves with their data (R2 export row, now with trust records) — *`trust.rs` 188; `roym_trust_e2e` step 13*

- X runs `roymctl roym backup create`. On a clean machine X restores
  identity and data (existing C8 flow).
- On the new machine, **Memberships** shows the same verdict for Y as
  before (*Revoked …*). It is recomputed from the stored signed records,
  not copied as text. X's agreement with Y still checks out.
