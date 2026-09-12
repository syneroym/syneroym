use syneroym_identity::{Identity, substrate};

use super::*;

/// Signs with the issuer this payload's `listing_id` was actually
/// derived from -- the only shape `verify_envelope` accepts as
/// `verified: true`.
fn sign_listing_with_own_issuer(payload: &ListingPayload, now: u64) -> String {
    let issuer_key = Identity::generate().unwrap();
    let issuer = substrate::derive_did_key(&issuer_key.public_key());
    let mut p = payload.clone();
    p.listing_id = derive_listing_id(&issuer, &p.slug).unwrap();
    let draft = syneroym_signed_record::RecordDraft {
        version: LISTING_VERSION,
        record_type: crate::record::RECORD_LISTING.to_string(),
        subject: p.listing_id.clone(),
        payload: serde_json::to_value(&p).unwrap(),
        expires_at_secs: None,
        supersedes: None,
    };
    let (mut env, bytes) =
        syneroym_signed_record::Envelope::unsigned(draft, issuer.clone(), None, now).unwrap();
    let sig = z32::encode(&issuer_key.sign(&bytes).to_bytes());
    env.attach_signature(sig).unwrap();
    env.to_json().unwrap()
}

#[test]
fn verify_envelope_accepts_a_correctly_signed_listing() {
    let p = core();
    let env = sign_listing_with_own_issuer(&p, 1000);
    let verdict = verify_envelope(&env, 1000);
    assert!(verdict.verified, "{:?}", verdict.reason);
    assert_eq!(verdict.status, Some(ListingStatus::Active));
    assert!(verdict.conversation_address.is_some());
    assert_eq!(verdict.revocation_status.as_deref(), Some("unknown"));
}

#[test]
fn verify_envelope_refuses_a_tampered_envelope() {
    let p = core();
    let mut env: serde_json::Value =
        serde_json::from_str(&sign_listing_with_own_issuer(&p, 1000)).unwrap();
    env["payload"]["title"] = serde_json::json!("Tampered");
    let verdict = verify_envelope(&env.to_string(), 1000);
    assert!(!verdict.verified);
    assert!(verdict.reason.is_some());
}

#[test]
fn verify_envelope_refuses_a_listing_id_not_derivable_from_the_issuer() {
    let key = Identity::generate().unwrap();
    let issuer = substrate::derive_did_key(&key.public_key());
    // `core()`'s listing_id is derived from a different issuer entirely.
    let p = core();
    let draft = syneroym_signed_record::RecordDraft {
        version: LISTING_VERSION,
        record_type: crate::record::RECORD_LISTING.to_string(),
        subject: p.listing_id.clone(),
        payload: serde_json::to_value(&p).unwrap(),
        expires_at_secs: None,
        supersedes: None,
    };
    let (mut sealed, bytes) =
        syneroym_signed_record::Envelope::unsigned(draft, issuer, None, 1000).unwrap();
    let sig = z32::encode(&key.sign(&bytes).to_bytes());
    sealed.attach_signature(sig).unwrap();
    let env = sealed.to_json().unwrap();
    let verdict = verify_envelope(&env, 1000);
    assert!(!verdict.verified);
    assert_eq!(
        verdict.reason.as_deref(),
        Some("listing_id is not derivable from the signature's own issuer")
    );
}

fn core() -> ListingPayload {
    ListingPayload {
        listing_id: derive_listing_id("did:key:zIssuer", "hedge-trimming").unwrap(),
        slug: "hedge-trimming".to_string(),
        title: "Hedge trimming".to_string(),
        summary: "Neat hedges, fortnightly.".to_string(),
        categories: vec!["gardening".to_string(), "outdoor".to_string()],
        conversation_address: "did:key:zProviderConv".to_string(),
        status: ListingStatus::Active,
        booking: None,
        payment: Some(PaymentTerms {
            currency: "EUR".to_string(),
            model: PaymentModel::PerHour,
            amount_minor: Some(3500),
            tax_included: true,
            fees_minor: None,
            methods: vec!["cash".to_string()],
            payee: "A. Gardener".to_string(),
        }),
        product: None,
        service: None,
        location: None,
        relationship: None,
        service_record: None,
    }
}

#[test]
fn a_full_payload_validates_and_signs() {
    let mut p = core();
    p.booking = Some(BookingTerms {
        mode: BookingMode::Slots,
        lead_time_secs: 3600,
        cancellation_window_secs: 86_400,
        max_per_booking: 2,
    });
    p.product = Some(ProductDetail {
        unit: "hour".to_string(),
        pack_size: 1,
        condition: ProductCondition::New,
        sku: Some("HT-1".to_string()),
    });
    p.service = Some(ServiceDetail {
        duration_secs: 3600,
        includes: vec!["clippings removed".to_string()],
        excludes: vec![],
        prerequisites: vec![],
    });
    p.location = Some(LocationTerms {
        where_: ServiceLocation::AtCustomer,
        service_area: vec![Area::Circle {
            lat_e6: 48_856_600,
            lon_e6: 2_352_200,
            radius_m: 15_000,
        }],
        address_disclosure: AddressDisclosure::OnAgreement,
    });
    p.relationship = Some(RelationshipTerms { open_to: OpenTo::Anyone, member_of: None });
    p.service_record = Some(ServiceRecordTerms {
        issues_fulfilment_receipt: true,
        warranty_secs: 0,
        retention_secs: 31_536_000,
    });
    p.validate().unwrap();

    // The host would sign it: a `RecordDraft` over this payload passes
    // `RecordDraft::validate` (no non-integer number anywhere).
    let draft = syneroym_signed_record::RecordDraft {
        version: LISTING_VERSION,
        record_type: crate::record::RECORD_LISTING.to_string(),
        subject: p.listing_id.clone(),
        payload: serde_json::to_value(&p).unwrap(),
        expires_at_secs: None,
        supersedes: None,
    };
    draft.validate(0).unwrap();
}

#[test]
fn a_float_in_the_payload_is_refused_by_draft_validate() {
    let mut v = serde_json::to_value(core()).unwrap();
    v["payment"]["amount_minor"] = serde_json::json!(35.5);
    let draft = syneroym_signed_record::RecordDraft {
        version: LISTING_VERSION,
        record_type: crate::record::RECORD_LISTING.to_string(),
        subject: "sub".to_string(),
        payload: v,
        expires_at_secs: None,
        supersedes: None,
    };
    assert!(draft.validate(0).is_err(), "a decimal price must be refused before signing");
}

#[test]
fn derive_listing_id_is_stable_and_issuer_separated() {
    let a = derive_listing_id("did:key:zA", "x").unwrap();
    assert_eq!(a, derive_listing_id("did:key:zA", "x").unwrap());
    assert_ne!(a, derive_listing_id("did:key:zB", "x").unwrap());
    assert_ne!(a, derive_listing_id("did:key:zA", "y").unwrap());
    assert!(a.starts_with("lst_"));
}

#[test]
fn title_bounds() {
    let mut p = core();
    p.title = String::new();
    assert_eq!(p.validate(), Err(ListingError::TitleEmpty));
    p.title = "a".repeat(MAX_TITLE_LEN + 1);
    assert_eq!(p.validate(), Err(ListingError::TitleTooLong));
    p.title = "a".repeat(MAX_TITLE_LEN);
    assert!(p.validate().is_ok());
}

#[test]
fn summary_bound() {
    let mut p = core();
    p.summary = "a".repeat(MAX_SUMMARY_LEN + 1);
    assert_eq!(p.validate(), Err(ListingError::SummaryTooLong));
}

#[test]
fn slug_shape() {
    let mut p = core();
    p.slug = "Bad Slug".to_string();
    assert!(matches!(p.validate(), Err(ListingError::SlugShape(_))));
    p.slug = "a".repeat(MAX_SLUG_LEN + 1);
    assert!(matches!(p.validate(), Err(ListingError::SlugShape(_))));
    p.slug = String::new();
    assert!(matches!(p.validate(), Err(ListingError::SlugShape(_))));
}

#[test]
fn category_count_and_shape() {
    let mut p = core();
    p.categories = (0..MAX_CATEGORIES + 1).map(|i| format!("c{i}")).collect();
    assert_eq!(p.validate(), Err(ListingError::TooManyCategories));
    p.categories = vec!["Bad Cat".to_string()];
    assert!(matches!(p.validate(), Err(ListingError::CategoryShape(_))));
}

#[test]
fn conversation_address_and_listing_id_required() {
    let mut p = core();
    p.conversation_address = "  ".to_string();
    assert_eq!(p.validate(), Err(ListingError::ConversationAddressEmpty));
    let mut p = core();
    p.listing_id = String::new();
    assert_eq!(p.validate(), Err(ListingError::ListingIdEmpty));
}

#[test]
fn currency_unknown() {
    let mut p = core();
    p.payment.as_mut().unwrap().currency = "eur".to_string();
    assert!(matches!(p.validate(), Err(ListingError::CurrencyUnknown(_))));
    p.payment.as_mut().unwrap().currency = "EURO".to_string();
    assert!(matches!(p.validate(), Err(ListingError::CurrencyUnknown(_))));
    p.payment.as_mut().unwrap().currency = "XYZ".to_string();
    assert!(matches!(p.validate(), Err(ListingError::CurrencyUnknown(_))));
}

#[test]
fn payment_required_unless_enquiry() {
    let mut p = core();
    p.payment = None;
    assert_eq!(p.validate(), Err(ListingError::PaymentRequired));
    p.booking = Some(BookingTerms {
        mode: BookingMode::Enquiry,
        lead_time_secs: 0,
        cancellation_window_secs: 0,
        max_per_booking: 1,
    });
    assert!(p.validate().is_ok(), "enquiry-mode booking needs no payment block");
}

#[test]
fn amount_minor_rules_by_model() {
    let mut p = core();
    p.payment.as_mut().unwrap().model = PaymentModel::Fixed;
    p.payment.as_mut().unwrap().amount_minor = None;
    assert_eq!(p.validate(), Err(ListingError::AmountMinorRequired));
    p.payment.as_mut().unwrap().model = PaymentModel::QuoteOnly;
    p.payment.as_mut().unwrap().amount_minor = Some(1);
    assert_eq!(p.validate(), Err(ListingError::AmountMinorForbidden));
    p.payment.as_mut().unwrap().amount_minor = None;
    assert!(p.validate().is_ok());
}

#[test]
fn service_area_bounds() {
    let mut p = core();
    p.location = Some(LocationTerms {
        where_: ServiceLocation::Remote,
        service_area: (0..MAX_AREAS + 1)
            .map(|_| Area::Named { label: "x".to_string(), code: None })
            .collect(),
        address_disclosure: AddressDisclosure::Public,
    });
    assert_eq!(p.validate(), Err(ListingError::TooManyAreas));
    p.location.as_mut().unwrap().service_area =
        vec![Area::Circle { lat_e6: 0, lon_e6: 0, radius_m: area::MAX_RADIUS_M + 1 }];
    assert!(matches!(p.validate(), Err(ListingError::Area(_))));
}

#[test]
fn relationship_member_of_rules() {
    let mut p = core();
    p.relationship = Some(RelationshipTerms { open_to: OpenTo::Members, member_of: None });
    assert_eq!(p.validate(), Err(ListingError::MemberOfRequired));
    p.relationship =
        Some(RelationshipTerms { open_to: OpenTo::Members, member_of: Some("nope".to_string()) });
    assert!(matches!(p.validate(), Err(ListingError::MemberOfNotDid(_))));
    p.relationship = Some(RelationshipTerms {
        open_to: OpenTo::Members,
        member_of: Some("did:key:zGroup".to_string()),
    });
    assert!(p.validate().is_ok());
}

#[test]
fn slug_from_title_shapes() {
    assert_eq!(slug_from_title("Hedge Trimming!!"), Some("hedge-trimming".to_string()));
    assert_eq!(slug_from_title("  --  "), None);
    assert_eq!(slug_from_title(""), None);
    assert_eq!(slug_from_title("A").as_deref(), Some("a"));
    assert!(slug_from_title(&"x ".repeat(100)).unwrap().len() <= MAX_SLUG_LEN);
}

#[test]
fn absent_blocks_contribute_no_bytes() {
    // A payload with three blocks and one where four are explicitly
    // null must serialize identically to one with just the three.
    let mut with_nulls = serde_json::to_value(core()).unwrap();
    with_nulls["product"] = serde_json::Value::Null;
    with_nulls["service"] = serde_json::Value::Null;
    let reparsed: ListingPayload = serde_json::from_value(with_nulls).unwrap();
    assert_eq!(serde_json::to_value(&reparsed).unwrap(), serde_json::to_value(core()).unwrap());
}
