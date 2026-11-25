//! Shared fixtures for the "agree as listed" service tests: the small listing
//! shapes, their signing, and the consent a tap carries.

use syneroym_identity::{Identity, substrate};
use syneroym_roym_core::{
    as_listed::AsListed,
    deal::LocalText,
    lang::LangTag,
    listing::{
        self, Acceptance, AddressDisclosure, DEFAULT_INSTANT_CAP, ListingDealTerms, ListingPayload,
        ListingStatus, LocationTerms, OpenTo, PaymentModel, PaymentTerms, RelationshipTerms,
        ServiceLocation,
    },
    record::{Envelope, RECORD_LISTING, RecordDraft},
};
use z32;

pub(crate) const CONVERSATION: &str = "conv_tap";
pub(crate) const CONVERSATION_ADDRESS: &str = "did:key:zOwnerConversation";
pub(crate) const SLUG: &str = "hedge-trimming";

pub(crate) fn lang(code: &str) -> LangTag {
    LangTag::parse_canonical(code).expect("valid tag")
}

pub(crate) fn text(body: &str) -> LocalText {
    LocalText::plain(lang("en"), body)
}

pub(crate) fn deal_terms() -> ListingDealTerms {
    ListingDealTerms {
        cancellation_terms: text("Free until 24 hours before"),
        refund_terms: text("Full refund"),
        dispute_path: text("Write to us first"),
        quote_valid_secs: 3_600,
        payment_timing: None,
    }
}

fn base_listing() -> ListingPayload {
    ListingPayload {
        listing_id: String::new(),
        slug: SLUG.to_string(),
        title: "Hedge trimming".to_string(),
        summary: "Neat hedges.".to_string(),
        lang: lang("en"),
        translations: Vec::new(),
        categories: vec!["gardening".to_string()],
        conversation_address: CONVERSATION_ADDRESS.to_string(),
        status: ListingStatus::Active,
        extension: None,
        booking: None,
        payment: Some(PaymentTerms {
            currency: "EUR".to_string(),
            model: PaymentModel::Fixed,
            amount_minor: Some(3_500),
            tax_included: true,
            fees_minor: None,
            methods: vec!["cash".to_string()],
            payee: "A. Gardener".to_string(),
        }),
        product: None,
        service: None,
        location: Some(LocationTerms {
            where_: ServiceLocation::AtGiver,
            service_area: Vec::new(),
            address_disclosure: AddressDisclosure::OnAgreement,
        }),
        relationship: Some(RelationshipTerms { open_to: OpenTo::Anyone, member_of: None }),
        acceptance: Acceptance::ConfirmFirst,
        not_limited: true,
        instant_cap: DEFAULT_INSTANT_CAP,
        deal_terms: Some(deal_terms()),
        service_record: None,
    }
}

/// The owner's listing with the requested acceptance and cap.
pub(crate) fn listing(acceptance: Acceptance, cap: u16) -> ListingPayload {
    ListingPayload { acceptance, instant_cap: cap, ..base_listing() }
}

/// The counterparty's listing with the requested payment model.
pub(crate) fn listing_payload(model: PaymentModel) -> ListingPayload {
    let mut payload = base_listing();
    payload.acceptance = Acceptance::Instant;
    if let Some(payment) = &mut payload.payment {
        payment.model = model;
        payment.fees_minor = Some(100);
    }
    payload.deal_terms = Some(ListingDealTerms {
        payment_timing: Some(listing::PaymentTiming::AfterWork),
        ..deal_terms()
    });
    payload
}

/// The enquiry test's listing: confirm first, and not open until the owner
/// says it is.
pub(crate) fn ready_listing(model: PaymentModel) -> ListingPayload {
    let mut payload = listing_payload(model);
    payload.acceptance = Acceptance::ConfirmFirst;
    payload.not_limited = false;
    payload
}

/// Signs the offering under `key`, which must be its own issuer so the id
/// derives from it.
pub(crate) fn sign_listing(
    key: &Identity,
    mut payload: ListingPayload,
    now: u64,
) -> (String, ListingPayload) {
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

pub(crate) fn consent(record_id: &str, valid_until_secs: u64) -> AsListed {
    AsListed { record_id: record_id.to_string(), slot: None, variant: None, valid_until_secs }
}
