//! Service tests for the counterparty's automatic acceptance.

use serde_json::json;
use syneroym_identity::{Identity, substrate};
use syneroym_roym_core::{
    as_listed::{self, AsListed},
    backup::{Bundle, ImportInput, SECTION_LEDGER},
    deal::{DealTerms, LocalText},
    lang::LangTag,
    listing::{
        self, Acceptance, AddressDisclosure, DEFAULT_INSTANT_CAP, ListingDealTerms, ListingPayload,
        ListingStatus, OpenTo, PaymentModel, PaymentTerms, RelationshipTerms, ServiceLocation,
    },
    method::NoParams,
    record::{Envelope, RECORD_LISTING, RECORD_QUOTE, RecordDraft},
    signing,
    testing::{
        TestHost,
        sd::{PlainQuote, conceal_quote},
    },
    transaction::{
        self, Answers, DEFAULT_DATA_USE_NOTICE, ENQUIRY_VERSION, EnquiryPayload, QUOTE_VERSION,
        RECORD_ENQUIRY, derive_quote_id, verify_quote,
    },
};
use z32;

use super::fixture;
use crate::app::{
    AGREEMENTS, AS_LISTED, AgreementRow, LEDGER,
    as_listed_ops::{
        AutoAcceptRow,
        accept::{AutoAccept, auto_accept, auto_accept_id, same_terms},
    },
    backup, disclosures,
    enquiry_ops::{
        self,
        as_listed::{AsListedRow, AsListedState},
    },
    get_row, put_row,
    quote_ops::store_received_quote,
};

const CONVERSATION: &str = "conv_tap";

fn lang(code: &str) -> LangTag {
    LangTag::parse_canonical(code).expect("valid tag")
}

fn text(body: &str) -> LocalText {
    LocalText::plain(lang("en"), body)
}

fn listing_payload(model: PaymentModel) -> ListingPayload {
    ListingPayload {
        listing_id: String::new(),
        slug: "hedge-trimming".to_string(),
        title: "Hedge trimming".to_string(),
        summary: "Neat hedges.".to_string(),
        lang: lang("en"),
        translations: Vec::new(),
        categories: vec!["gardening".to_string()],
        conversation_address: "did:key:zOwnerConversation".to_string(),
        status: ListingStatus::Active,
        extension: None,
        booking: None,
        payment: Some(PaymentTerms {
            currency: "EUR".to_string(),
            model,
            amount_minor: Some(3_500),
            tax_included: true,
            fees_minor: Some(100),
            methods: vec!["cash".to_string()],
            payee: "A. Gardener".to_string(),
        }),
        product: None,
        service: None,
        location: Some(listing::LocationTerms {
            where_: ServiceLocation::AtGiver,
            service_area: Vec::new(),
            address_disclosure: AddressDisclosure::OnAgreement,
        }),
        relationship: Some(RelationshipTerms { open_to: OpenTo::Anyone, member_of: None }),
        acceptance: Acceptance::Instant,
        not_limited: true,
        instant_cap: DEFAULT_INSTANT_CAP,
        deal_terms: Some(ListingDealTerms {
            cancellation_terms: text("Free until 24 hours before"),
            refund_terms: text("Full refund"),
            dispute_path: text("Write to us first"),
            quote_valid_secs: 3_600,
            payment_timing: Some(listing::PaymentTiming::AfterWork),
        }),
        service_record: None,
    }
}

/// Signs the offering under `key` and returns its envelope and payload.
fn sign_listing(key: &Identity, mut payload: ListingPayload, now: u64) -> (String, ListingPayload) {
    let issuer = substrate::derive_did_key(&key.public_key());
    payload.listing_id = listing::derive_listing_id(&issuer, &payload.slug).expect("listing id");
    let draft = RecordDraft {
        version: listing::LISTING_VERSION,
        record_type: RECORD_LISTING.to_string(),
        subject: payload.listing_id.clone(),
        payload: serde_json::to_value(&payload).expect("payload"),
        expires_at_secs: None,
        supersedes: None,
    };
    let (mut env, bytes) = Envelope::unsigned(draft, issuer, None, now).expect("envelope");
    let sig = z32::encode(&key.sign(&bytes).to_bytes());
    env.attach_signature(sig).expect("signature");
    (env.to_json().expect("json"), payload)
}

/// Signs and holds an as-listed enquiry from a peer, and returns its record id.
async fn hold_enquiry(
    host: &TestHost,
    peer: &Identity,
    tap: &Tap,
    valid_until_secs: u64,
    now: u64,
    owner_did: &str,
) -> String {
    let peer_did = substrate::derive_did_key(&peer.public_key());
    let enquiry_id =
        transaction::derive_enquiry_id(CONVERSATION, &peer_did, 1).expect("enquiry id");
    let payload = EnquiryPayload {
        enquiry_id: enquiry_id.clone(),
        conversation: CONVERSATION.to_string(),
        sequence: 1,
        listing_id: Some("lst_tap".to_string()),
        categories: Vec::new(),
        description: None,
        as_listed: Some(AsListed {
            record_id: tap.record_id.clone(),
            slot: None,
            variant: None,
            valid_until_secs,
        }),
        area: None,
        window: None,
        data_use_notice: DEFAULT_DATA_USE_NOTICE.to_string(),
    };
    let draft = RecordDraft {
        version: ENQUIRY_VERSION,
        record_type: RECORD_ENQUIRY.to_string(),
        subject: enquiry_id,
        payload: serde_json::to_value(&payload).expect("payload"),
        expires_at_secs: None,
        supersedes: None,
    };
    let (mut env, bytes) = Envelope::unsigned(draft, peer_did, None, now).expect("envelope");
    let sig = z32::encode(&peer.sign(&bytes).to_bytes());
    env.attach_signature(sig).expect("signature");
    let envelope = env.to_json().expect("json");
    let verdict = transaction::verify_enquiry(&envelope, now);
    enquiry_ops::store_received_enquiry(host, &envelope, &verdict, &payload, now, owner_did)
        .await
        .expect("store enquiry");
    verdict.record_id.expect("enquiry record id")
}

/// Stores a quote from the owner that answers the tap, and returns its record
/// id.
async fn store_quote(
    host: &TestHost,
    counterparty_did: &str,
    owner: &Identity,
    enquiry_record: &str,
    terms: DealTerms,
    expires_at_secs: u64,
    now: u64,
) -> String {
    let issuer = substrate::derive_did_key(&owner.public_key());
    let quote_id = derive_quote_id(CONVERSATION, &issuer, 1).expect("quote id");
    let plain = PlainQuote {
        quote_id,
        conversation: CONVERSATION.to_string(),
        sequence: 1,
        answers: Answers::Enquiry { record_id: enquiry_record.to_string() },
        counterparty_did: counterparty_did.to_string(),
        terms,
    };
    let (payload, disclosures) = conceal_quote(plain, 7);
    let draft = RecordDraft {
        version: QUOTE_VERSION,
        record_type: RECORD_QUOTE.to_string(),
        subject: CONVERSATION.to_string(),
        payload: serde_json::to_value(&payload).expect("payload"),
        expires_at_secs: Some(expires_at_secs),
        supersedes: None,
    };
    let (mut env, bytes) = Envelope::unsigned(draft, issuer, None, now).expect("envelope");
    let sig = z32::encode(&owner.sign(&bytes).to_bytes());
    env.attach_signature(sig).expect("signature");
    let envelope = env.to_json().expect("json");
    let verdict = verify_quote(&envelope, now);
    let record_id = verdict.record_id.clone().expect("record id");
    store_received_quote(host, &envelope, &verdict, &payload, now, counterparty_did)
        .await
        .expect("store quote");
    disclosures::store(host, &record_id, RECORD_QUOTE, &disclosures, now)
        .await
        .expect("store disclosures");
    record_id
}

struct Tap {
    envelope: String,
    record_id: String,
    terms: DealTerms,
}

/// The offering a person tapped, with the terms it builds.
fn tap(owner: &Identity, now: u64) -> Tap {
    let (envelope, payload) = sign_listing(owner, listing_payload(PaymentModel::Fixed), now);
    let verdict = listing::verify_envelope(&envelope, now);
    let record_id = verdict.record_id.expect("listing record");
    let consent = AsListed {
        record_id: record_id.clone(),
        slot: None,
        variant: None,
        valid_until_secs: now + 3_600,
    };
    let issuer = substrate::derive_did_key(&owner.public_key());
    let terms = as_listed::terms_from_listing(&payload, &issuer, &consent, now).expect("terms");
    Tap { envelope, record_id, terms }
}

async fn hold_tap(
    host: &TestHost,
    tap: &Tap,
    enquiry_record: &str,
    valid_until_secs: u64,
    now: u64,
) {
    let row = AsListedRow {
        enquiry_record_id: enquiry_record.to_string(),
        listing_id: "lst_tap".to_string(),
        conversation: CONVERSATION.to_string(),
        envelope: tap.envelope.clone(),
        record_id: tap.record_id.clone(),
        slot: None,
        valid_until_secs,
        viewed_lang: "en".to_string(),
        viewed_via_mt: false,
        state: AsListedState::Waiting,
        reason: None,
        quote_record_id: None,
        created_at_secs: now,
    };
    put_row(host, AS_LISTED, enquiry_record, &row).await.expect("store tap");
}

async fn held_tap(host: &TestHost, enquiry_record: &str) -> AsListedRow {
    get_row(host, AS_LISTED, enquiry_record).await.expect("read").expect("tap row")
}

#[tokio::test]
async fn a_quote_with_the_listed_terms_is_accepted_with_no_person() {
    let f = fixture();
    let now = f.host.now();
    let owner = Identity::generate().expect("owner");
    let peer = Identity::generate().expect("peer");
    let tap = tap(&owner, now);
    let enquiry_record = hold_enquiry(&f.host, &peer, &tap, now + 3_600, now, &f.master_did).await;
    hold_tap(&f.host, &tap, &enquiry_record, now + 3_600, now).await;
    let quote_record = store_quote(
        &f.host,
        &f.master_did,
        &owner,
        &enquiry_record,
        tap.terms.clone(),
        now + 3_600,
        now,
    )
    .await;

    let outcome =
        auto_accept(&f.host, &enquiry_record, &quote_record, now).await.expect("auto accept");
    assert_eq!(outcome, AutoAccept::Accepted);
    assert_eq!(held_tap(&f.host, &enquiry_record).await.state, AsListedState::Accepted);

    let agreement: AgreementRow =
        get_row(&f.host, AGREEMENTS, &quote_record).await.expect("read").expect("agreement");
    assert!(agreement.counterparty.is_some(), "the counterparty half is signed");
}

#[tokio::test]
async fn a_quote_that_differs_in_any_field_is_shown() {
    let f = fixture();
    let now = f.host.now();
    let owner = Identity::generate().expect("owner");
    let peer = Identity::generate().expect("peer");
    let tap = tap(&owner, now);
    let enquiry_record = hold_enquiry(&f.host, &peer, &tap, now + 3_600, now, &f.master_did).await;
    hold_tap(&f.host, &tap, &enquiry_record, now + 3_600, now).await;

    let mut different = tap.terms.clone();
    different.commitments[1].payee = Some("Someone Else".to_string());
    assert!(!same_terms(&tap.terms, &different, now, now));

    let quote_record =
        store_quote(&f.host, &f.master_did, &owner, &enquiry_record, different, now + 3_600, now)
            .await;
    let outcome =
        auto_accept(&f.host, &enquiry_record, &quote_record, now).await.expect("auto accept");
    assert_eq!(outcome, AutoAccept::Shown("terms-differ".to_string()));
    let row = held_tap(&f.host, &enquiry_record).await;
    assert_eq!(row.state, AsListedState::Shown);
    assert_eq!(row.reason.as_deref(), Some("terms-differ"));
}

#[test]
fn a_changed_field_anywhere_in_the_terms_fails_the_check() {
    let now = 1_700_000_000;
    let owner = Identity::generate().expect("owner");
    let tap = tap(&owner, now);
    let base = tap.terms.clone();
    assert!(same_terms(&base, &base, now, now));

    let mut cases: Vec<(&str, DealTerms)> = Vec::new();
    let mut changed = base.clone();
    changed.currency = "INR".to_string();
    cases.push(("currency", changed));
    let mut changed = base.clone();
    changed.listing_id = Some("lst_other".to_string());
    cases.push(("listing_id", changed));
    let mut changed = base.clone();
    changed.scope.text = "Other".to_string();
    cases.push(("scope", changed));
    let mut changed = base.clone();
    changed.cancellation_terms.text = "Other".to_string();
    cases.push(("cancellation_terms", changed));
    let mut changed = base.clone();
    changed.refund_terms.text = "Other".to_string();
    cases.push(("refund_terms", changed));
    let mut changed = base.clone();
    changed.dispute_path.text = "Other".to_string();
    cases.push(("dispute_path", changed));
    let mut changed = base.clone();
    changed.quote_expires_at_secs = now; // not after now
    cases.push(("expiry", changed));
    let mut changed = base.clone();
    changed.commitments[0].declared_value_minor += 1;
    cases.push(("declared_value_minor", changed));
    let mut changed = base.clone();
    changed.commitments[0].description.text = "Other".to_string();
    cases.push(("commitment description", changed));
    let mut changed = base.clone();
    changed.commitments[1].payee = Some("Other".to_string());
    cases.push(("payee", changed));
    let mut changed = base.clone();
    changed.commitments[1].fees_minor = None;
    cases.push(("fees_minor", changed));
    let mut changed = base.clone();
    changed.commitments[1].after = Vec::new();
    cases.push(("after", changed));

    for (field, quote) in cases {
        assert!(!same_terms(&base, &quote, now, now), "{field} must fail the check");
    }
}

#[tokio::test]
async fn a_quote_issued_after_the_consent_time_is_shown() {
    let f = fixture();
    let now = f.host.now();
    let owner = Identity::generate().expect("owner");
    let peer = Identity::generate().expect("peer");
    let tap = tap(&owner, now);
    let enquiry_record = hold_enquiry(&f.host, &peer, &tap, now + 60, now, &f.master_did).await;
    hold_tap(&f.host, &tap, &enquiry_record, now + 60, now).await;
    let quote_record = store_quote(
        &f.host,
        &f.master_did,
        &owner,
        &enquiry_record,
        tap.terms.clone(),
        now + 3_600,
        now + 61,
    )
    .await;

    let outcome =
        auto_accept(&f.host, &enquiry_record, &quote_record, now + 61).await.expect("auto accept");
    assert_eq!(outcome, AutoAccept::Shown("consent-expired".to_string()));
}

#[tokio::test]
async fn the_acceptance_carries_the_stored_viewed_fields() {
    let f = fixture();
    let now = f.host.now();
    let owner = Identity::generate().expect("owner");
    let peer = Identity::generate().expect("peer");
    let tap = tap(&owner, now);
    let enquiry_record = hold_enquiry(&f.host, &peer, &tap, now + 3_600, now, &f.master_did).await;
    hold_tap(&f.host, &tap, &enquiry_record, now + 3_600, now).await;
    let mut row = held_tap(&f.host, &enquiry_record).await;
    row.viewed_lang = "hi".to_string();
    row.viewed_via_mt = true;
    put_row(&f.host, AS_LISTED, &enquiry_record, &row).await.expect("update tap");
    let quote_record = store_quote(
        &f.host,
        &f.master_did,
        &owner,
        &enquiry_record,
        tap.terms.clone(),
        now + 3_600,
        now,
    )
    .await;

    auto_accept(&f.host, &enquiry_record, &quote_record, now).await.expect("auto accept");
    let agreement: AgreementRow =
        get_row(&f.host, AGREEMENTS, &quote_record).await.expect("read").expect("agreement");
    let half = agreement.counterparty.expect("half");
    let verdict = transaction::verify_agreement_receipt(&half.envelope, now);
    let payload = verdict.payload.expect("payload");
    assert_eq!(payload.viewed_lang, lang("hi"));
    assert!(payload.viewed_via_mt);
}

#[tokio::test]
async fn one_tap_accepts_at_most_one_quote() {
    let f = fixture();
    let now = f.host.now();
    let owner = Identity::generate().expect("owner");
    let peer = Identity::generate().expect("peer");
    let tap = tap(&owner, now);
    let enquiry_record = hold_enquiry(&f.host, &peer, &tap, now + 3_600, now, &f.master_did).await;
    hold_tap(&f.host, &tap, &enquiry_record, now + 3_600, now).await;
    let quote_record = store_quote(
        &f.host,
        &f.master_did,
        &owner,
        &enquiry_record,
        tap.terms.clone(),
        now + 3_600,
        now,
    )
    .await;

    put_row(
        &f.host,
        LEDGER,
        &auto_accept_id(&enquiry_record),
        &AutoAcceptRow {
            kind: crate::app::ledger::LedgerKind::AutoAccept,
            enquiry_record_id: enquiry_record.clone(),
            quote_record_id: "rec_other_quote".to_string(),
            created_at_secs: now,
        },
    )
    .await
    .expect("fence");

    let outcome =
        auto_accept(&f.host, &enquiry_record, &quote_record, now).await.expect("auto accept");
    assert_eq!(outcome, AutoAccept::Shown("already-accepted".to_string()));
}

#[tokio::test]
async fn the_import_checks_autoaccept_rows() {
    let f = fixture();
    let now = f.host.now();
    let owner = Identity::generate().expect("owner");
    let peer = Identity::generate().expect("peer");
    let tap = tap(&owner, now);
    let enquiry_record = hold_enquiry(&f.host, &peer, &tap, now + 3_600, now, &f.master_did).await;
    hold_tap(&f.host, &tap, &enquiry_record, now + 3_600, now).await;
    let quote_record = store_quote(
        &f.host,
        &f.master_did,
        &owner,
        &enquiry_record,
        tap.terms.clone(),
        now + 3_600,
        now,
    )
    .await;
    auto_accept(&f.host, &enquiry_record, &quote_record, now).await.expect("accept");

    let bundle = backup::export(&f.host, NoParams {}).await.expect("export");
    let mut sections = bundle.sections.clone();
    let rows = sections.get_mut(SECTION_LEDGER).expect("ledger");
    let row = rows
        .iter_mut()
        .find(|row| row["id"].as_str().is_some_and(|id| id.starts_with("autoaccept:")))
        .expect("autoaccept row");
    row["payload"]["quote_record_id"] = json!("rec_unknown");
    let mut tampered =
        Bundle::new(f.master_did.clone(), crate::app::SCHEMA_VERSION, sections).expect("bundle");
    signing::sign_bundle(&f.host, &mut tampered, now).await.expect("sign");

    let err = backup::import(&f.host, ImportInput { bundle: tampered }).await.expect_err("refused");
    let message = err.error.expect("error").message;
    assert!(message.contains("autoaccept:"), "{message}");
    assert!(message.contains("unheld quote"), "{message}");
}
