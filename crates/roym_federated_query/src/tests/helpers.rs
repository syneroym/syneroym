//! Shared test helpers and fake AppProxy implementation.

use std::sync::Mutex;

use serde_json::json;
use syneroym_app_host::{
    AppProxy,
    types::proxy::{CallOptions, CallRequest, CallTarget, ProxyError},
};
use syneroym_identity::{Identity, substrate};
use syneroym_roym_core::{
    directory::{AreaMatch, SearchHit},
    lang::LangTag,
    listing::{
        LISTING_VERSION, ListingPayload, ListingStatus, ListingTranslation, ListingVerdict,
        PaymentModel, PaymentTerms, derive_listing_id, verify_envelope,
    },
    membership::{
        MAX_REASON_LEN, MAX_RULE_LEN, MEMBERSHIP_CREDENTIAL_VERSION, MODERATION_DECISION_VERSION,
        MembershipCredentialPayload, MembershipEvidence, MembershipScope, ModerationAction,
        ModerationDecisionPayload, ModerationScope, REVOCATION_VERSION, RevocationPayload,
    },
    record::{
        RECORD_LISTING, RECORD_MEMBERSHIP_CREDENTIAL, RECORD_MODERATION_DECISION, RECORD_REVOCATION,
    },
};
use syneroym_signed_record::{Envelope, RecordDraft};

pub type FakeCallAllResponse = Result<Vec<Result<String, ProxyError>>, ProxyError>;

pub struct FakeProxy {
    pub recorded_calls: Mutex<Vec<CallRequest>>,
    pub response: Mutex<Option<FakeCallAllResponse>>,
}

impl FakeProxy {
    pub fn new(response: FakeCallAllResponse) -> Self {
        Self { recorded_calls: Mutex::new(Vec::new()), response: Mutex::new(Some(response)) }
    }

    pub fn calls(&self) -> Vec<CallRequest> {
        self.recorded_calls.lock().unwrap().clone()
    }
}

impl AppProxy for FakeProxy {
    async fn call(
        &self,
        _target: CallTarget,
        _interface: String,
        _method: String,
        _params: String,
        _options: Option<CallOptions>,
    ) -> Result<String, ProxyError> {
        unimplemented!()
    }

    async fn call_all(
        &self,
        calls: Vec<CallRequest>,
    ) -> Result<Vec<Result<String, ProxyError>>, ProxyError> {
        self.recorded_calls.lock().unwrap().extend(calls);
        self.response.lock().unwrap().take().expect("FakeProxy response called once")
    }

    async fn enqueue(
        &self,
        _target: CallTarget,
        _interface: String,
        _method: String,
        _params: String,
        _options: Option<CallOptions>,
    ) -> Result<(), ProxyError> {
        unimplemented!()
    }
}

pub fn sign_test_listing(slug: &str, title: &str, now: u64) -> (String, String, String) {
    let key = Identity::generate().unwrap();
    let issuer = substrate::derive_did_key(&key.public_key());
    sign_test_listing_with_issuer(&key, &issuer, slug, title, now)
}

pub fn sign_test_listing_with_translation(
    slug: &str,
    title: &str,
    tr_lang: &str,
    tr_title: &str,
    now: u64,
) -> (String, String, String) {
    let key = Identity::generate().unwrap();
    let issuer = substrate::derive_did_key(&key.public_key());
    let listing_id = derive_listing_id(&issuer, slug).unwrap();
    let payload = ListingPayload {
        listing_id: listing_id.clone(),
        slug: slug.to_string(),
        title: title.to_string(),
        summary: format!("Summary {slug}"),
        lang: LangTag::parse_canonical("en").expect("valid tag"),
        translations: vec![ListingTranslation {
            lang: LangTag::parse_canonical(tr_lang).expect("valid tr tag"),
            title: tr_title.to_string(),
            summary: format!("Summary {tr_title}"),
        }],
        categories: vec!["services".to_string()],
        conversation_address: format!("{issuer}#conv"),
        status: ListingStatus::Active,
        booking: None,
        payment: Some(PaymentTerms {
            currency: "EUR".to_string(),
            model: PaymentModel::PerHour,
            amount_minor: Some(3500),
            tax_included: true,
            fees_minor: None,
            methods: vec!["cash".to_string()],
            payee: "A. Provider".to_string(),
        }),
        product: None,
        service: None,
        location: None,
        relationship: None,
        service_record: None,
    };
    let draft = RecordDraft {
        version: LISTING_VERSION,
        record_type: RECORD_LISTING.to_string(),
        subject: listing_id.clone(),
        payload: serde_json::to_value(&payload).unwrap(),
        expires_at_secs: None,
        supersedes: None,
    };
    let (mut env, bytes) = Envelope::unsigned(draft, issuer, None, now).unwrap();
    let sig = z32::encode(&key.sign(&bytes).to_bytes());
    env.attach_signature(sig).unwrap();
    let envelope_json = env.to_json().unwrap();
    let verdict: ListingVerdict = verify_envelope(&envelope_json, now);
    assert!(verdict.verified, "{:?}", verdict.reason);
    let record_id = verdict.record_id.unwrap();
    (listing_id, record_id, envelope_json)
}

pub fn max_escaping_summary() -> String {
    let pattern = "text \"quoted\" \\\n ";
    pattern.repeat(2000 / pattern.len())
}

pub fn sign_max_payload_listing_with_issuer(
    key: &Identity,
    issuer: &str,
    slug: &str,
    title: &str,
    now: u64,
) -> (String, String, String) {
    let listing_id = derive_listing_id(issuer, slug).unwrap();
    let payload = ListingPayload {
        listing_id: listing_id.clone(),
        slug: slug.to_string(),
        title: title.to_string(),
        summary: max_escaping_summary(),
        lang: LangTag::parse_canonical("en").expect("valid tag"),
        translations: Vec::new(),
        categories: vec!["services".to_string()],
        conversation_address: format!("{issuer}#conv"),
        status: ListingStatus::Active,
        booking: None,
        payment: Some(PaymentTerms {
            currency: "EUR".to_string(),
            model: PaymentModel::PerHour,
            amount_minor: Some(3500),
            tax_included: true,
            fees_minor: None,
            methods: vec!["cash".to_string()],
            payee: "A. Provider".to_string(),
        }),
        product: None,
        service: None,
        location: None,
        relationship: None,
        service_record: None,
    };
    let mut payload_val = serde_json::to_value(&payload).unwrap();
    payload_val["description"] = serde_json::Value::String(String::new());
    let base_bytes = substrate::canonicalize_json_value(&payload_val).to_string().len();
    // In canonical JSON, each `"` character inside a JSON string escapes to `\"` (2
    // bytes).
    let target_canonical_bytes = 65_500usize;
    let quotes_count = target_canonical_bytes.saturating_sub(base_bytes) / 2;
    payload_val["description"] = serde_json::Value::String("\"".repeat(quotes_count));
    let canonical_len = substrate::canonicalize_json_value(&payload_val).to_string().len();
    assert!(
        canonical_len > 64_000 && canonical_len <= 65_536,
        "canonical payload length must be between 64,000 and 65,536 bytes, got {canonical_len}"
    );

    let draft = RecordDraft {
        version: LISTING_VERSION,
        record_type: RECORD_LISTING.to_string(),
        subject: listing_id.clone(),
        payload: payload_val,
        expires_at_secs: None,
        supersedes: None,
    };
    let (mut env, bytes) = Envelope::unsigned(draft, issuer.to_string(), None, now).unwrap();
    let sig = z32::encode(&key.sign(&bytes).to_bytes());
    env.attach_signature(sig).unwrap();
    let envelope_json = env.to_json().unwrap();
    let verdict: ListingVerdict = verify_envelope(&envelope_json, now);
    let record_id = verdict.record_id.unwrap();
    (listing_id, record_id, envelope_json)
}

pub fn sign_test_listing_with_issuer(
    key: &Identity,
    issuer: &str,
    slug: &str,
    title: &str,
    now: u64,
) -> (String, String, String) {
    let listing_id = derive_listing_id(issuer, slug).unwrap();
    let payload = ListingPayload {
        listing_id: listing_id.clone(),
        slug: slug.to_string(),
        title: title.to_string(),
        summary: format!("Summary {slug}"),
        lang: LangTag::parse_canonical("en").expect("valid tag"),
        translations: Vec::new(),
        categories: vec!["services".to_string()],
        conversation_address: format!("{issuer}#conv"),
        status: ListingStatus::Active,
        booking: None,
        payment: Some(PaymentTerms {
            currency: "EUR".to_string(),
            model: PaymentModel::PerHour,
            amount_minor: Some(3500),
            tax_included: true,
            fees_minor: None,
            methods: vec!["cash".to_string()],
            payee: "A. Provider".to_string(),
        }),
        product: None,
        service: None,
        location: None,
        relationship: None,
        service_record: None,
    };
    let draft = RecordDraft {
        version: LISTING_VERSION,
        record_type: RECORD_LISTING.to_string(),
        subject: listing_id.clone(),
        payload: serde_json::to_value(&payload).unwrap(),
        expires_at_secs: None,
        supersedes: None,
    };
    let (mut env, bytes) = Envelope::unsigned(draft, issuer.to_string(), None, now).unwrap();
    let sig = z32::encode(&key.sign(&bytes).to_bytes());
    env.attach_signature(sig).unwrap();
    let envelope_json = env.to_json().unwrap();
    let verdict: ListingVerdict = verify_envelope(&envelope_json, now);
    let record_id = verdict.record_id.unwrap();
    (listing_id, record_id, envelope_json)
}

pub fn canned_search_hit(
    listing_id: &str,
    record_id: &str,
    envelope: &str,
    evidence: MembershipEvidence,
    now: u64,
) -> SearchHit {
    SearchHit {
        listing_id: listing_id.to_string(),
        record_id: record_id.to_string(),
        envelope: envelope.to_string(),
        issued_at_secs: now,
        received_at_secs: now,
        area_match: AreaMatch::NotQueried,
        membership: evidence,
    }
}

pub fn canned_search_response(hits: Vec<SearchHit>, truncated: bool) -> String {
    json!({
        "jsonrpc": "2.0",
        "result": {
            "hits": hits,
            "truncated": truncated,
            "directory": "did:key:canned_dir",
            "answered_at_secs": 1000
        },
        "id": null
    })
    .to_string()
}

pub fn sign_record(key: &Identity, issuer: &str, draft: RecordDraft, now: u64) -> String {
    let (mut env, bytes) = Envelope::unsigned(draft, issuer.to_string(), None, now).unwrap();
    let sig = z32::encode(&key.sign(&bytes).to_bytes());
    env.attach_signature(sig).unwrap();
    env.to_json().unwrap()
}

pub fn build_max_evidence(
    issuer_key: &Identity,
    issuer_did: &str,
    member_did: &str,
    listing_id: &str,
    now: u64,
) -> MembershipEvidence {
    let mut credentials = Vec::new();
    for i in 0..4 {
        let payload = MembershipCredentialPayload {
            synorg_name: "Guild".to_string(),
            member_did: member_did.to_string(),
            scope: MembershipScope {
                categories: vec!["services".to_string(), format!("cat_{i}")],
                area: vec![],
            },
        };
        let draft = RecordDraft {
            version: MEMBERSHIP_CREDENTIAL_VERSION,
            record_type: RECORD_MEMBERSHIP_CREDENTIAL.to_string(),
            subject: member_did.to_string(),
            payload: serde_json::to_value(&payload).unwrap(),
            expires_at_secs: Some(now + 3600),
            supersedes: None,
        };
        credentials.push(sign_record(issuer_key, issuer_did, draft, now));
    }

    let mut revocations = Vec::new();
    for i in 0..4 {
        let payload = RevocationPayload {
            credential_record_id: format!("did:key:other_cred_{i}"),
            member_did: member_did.to_string(),
            reason: "x".repeat(MAX_REASON_LEN),
        };
        let draft = RecordDraft {
            version: REVOCATION_VERSION,
            record_type: RECORD_REVOCATION.to_string(),
            subject: format!("did:key:other_cred_{i}"),
            payload: serde_json::to_value(&payload).unwrap(),
            expires_at_secs: None,
            supersedes: None,
        };
        revocations.push(sign_record(issuer_key, issuer_did, draft, now));
    }

    let mut decisions = Vec::new();
    for _i in 0..8 {
        let payload = ModerationDecisionPayload {
            action: ModerationAction::Lift,
            member_did: member_did.to_string(),
            scope: ModerationScope::Listing { listing_id: listing_id.to_string() },
            rule: "r".repeat(MAX_RULE_LEN),
            reason: "m".repeat(MAX_REASON_LEN),
            until_secs: None,
        };
        let draft = RecordDraft {
            version: MODERATION_DECISION_VERSION,
            record_type: RECORD_MODERATION_DECISION.to_string(),
            subject: member_did.to_string(),
            payload: serde_json::to_value(&payload).unwrap(),
            expires_at_secs: None,
            supersedes: None,
        };
        decisions.push(sign_record(issuer_key, issuer_did, draft, now));
    }

    MembershipEvidence { credentials, revocations, decisions }
}
