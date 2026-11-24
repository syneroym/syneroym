//! Service tests for an enquiry that agrees to a listing as listed.

use serde_json::json;
use syneroym_identity::Identity;
use syneroym_roym_core::{
    backup::{ImportInput, SECTION_AS_LISTED},
    deal::LocalText,
    listing::PaymentModel,
    method::NoParams,
    testing::TestHost,
    transaction::{self, DEFAULT_DATA_USE_NOTICE},
};

use super::{
    as_listed_fixture::{CONVERSATION, CONVERSATION_ADDRESS, ready_listing, sign_listing},
    fixture,
};
use crate::app::{
    AS_LISTED, ENQUIRY_HISTORY, backup,
    enquiry_ops::{
        self, EnquirySetInput,
        as_listed::{AsListedInput, AsListedRow},
    },
    get_bytes, get_row,
};

fn stub_conversation(host: &TestHost, conversation: &str, peer: &str) {
    let conversation = conversation.to_string();
    let peer = peer.to_string();
    host.on_call("conversation", "conversation.list", move |_params| {
        Ok(json!({
            "conversations": [
                { "id": conversation, "peer_address": peer }
            ]
        })
        .to_string())
    });
}

fn error_data(err: syneroym_roym_core::envelope::Response) -> serde_json::Value {
    err.error.expect("error").data.expect("data")
}

fn input(as_listed: Option<AsListedInput>, description: Option<LocalText>) -> EnquirySetInput {
    EnquirySetInput {
        conversation: CONVERSATION.to_string(),
        enquiry_id: None,
        listing_id: None,
        categories: Vec::new(),
        description,
        as_listed,
        area: None,
        window: None,
        data_use_notice: DEFAULT_DATA_USE_NOTICE.to_string(),
    }
}

fn tap(envelope: &str) -> AsListedInput {
    AsListedInput {
        envelope: envelope.to_string(),
        slot: None,
        valid_for_secs: 3_600,
        viewed_lang: "en".to_string(),
        viewed_via_mt: false,
    }
}

#[tokio::test]
async fn an_as_listed_enquiry_stores_its_offering_and_consent() {
    let f = fixture();
    let owner = Identity::generate().expect("owner key");
    stub_conversation(&f.host, CONVERSATION, CONVERSATION_ADDRESS);
    let now = f.host.now();
    let (envelope, payload) = sign_listing(&owner, ready_listing(PaymentModel::Fixed), now);
    let listing_id = payload.listing_id;

    let out =
        enquiry_ops::set(&f.host, input(Some(tap(&envelope)), None)).await.expect("enquiry set");

    let row: AsListedRow =
        get_row(&f.host, AS_LISTED, &out.record_id).await.expect("read").expect("row");
    assert_eq!(row.listing_id, listing_id);
    assert_eq!(row.conversation, CONVERSATION);
    assert_eq!(row.envelope, envelope);
    assert_eq!(row.viewed_lang, "en");
    assert!(!row.viewed_via_mt);
    assert_eq!(row.valid_until_secs, now + 3_600);

    let bytes = get_bytes(&f.host, ENQUIRY_HISTORY, &out.record_id)
        .await
        .expect("read")
        .expect("enquiry bytes");
    let verdict = transaction::verify_enquiry(&String::from_utf8(bytes).expect("utf8"), now);
    let payload = verdict.payload.expect("payload");
    assert!(payload.description.is_none());
    assert_eq!(payload.listing_id.as_deref(), Some(listing_id.as_str()));
    let consent = payload.as_listed.expect("consent");
    assert_eq!(consent.record_id, row.record_id);
    assert_eq!(consent.valid_until_secs, now + 3_600);
}

#[tokio::test]
async fn a_listing_of_this_nodes_owner_is_refused() {
    let f = fixture();
    stub_conversation(&f.host, CONVERSATION, CONVERSATION_ADDRESS);
    let now = f.host.now();
    let (envelope, _) = sign_listing(&f.master, ready_listing(PaymentModel::Fixed), now);

    let err =
        enquiry_ops::set(&f.host, input(Some(tap(&envelope)), None)).await.expect_err("refused");
    assert_eq!(error_data(err)["code"], "as-listed-own-listing");
}

#[tokio::test]
async fn a_listing_for_another_conversation_is_refused() {
    let f = fixture();
    stub_conversation(&f.host, CONVERSATION, "did:key:zSomebodyElse");
    let now = f.host.now();
    let owner = Identity::generate().expect("owner key");
    let (envelope, _) = sign_listing(&owner, ready_listing(PaymentModel::Fixed), now);

    let err =
        enquiry_ops::set(&f.host, input(Some(tap(&envelope)), None)).await.expect_err("refused");
    assert_eq!(error_data(err)["code"], "as-listed-wrong-conversation");
}

#[tokio::test]
async fn a_listing_that_cannot_be_agreed_as_listed_carries_its_reason() {
    let f = fixture();
    stub_conversation(&f.host, CONVERSATION, CONVERSATION_ADDRESS);
    let now = f.host.now();
    let owner = Identity::generate().expect("owner key");
    let (envelope, _) = sign_listing(&owner, ready_listing(PaymentModel::PerHour), now);

    let err =
        enquiry_ops::set(&f.host, input(Some(tap(&envelope)), None)).await.expect_err("refused");
    let data = error_data(err);
    assert_eq!(data["code"], "as-listed-not-complete");
    assert_eq!(data["reason"], "price-not-fixed");
}

#[tokio::test]
async fn an_enquiry_needs_a_description_or_a_consent() {
    let f = fixture();
    let err = enquiry_ops::set(&f.host, input(None, None)).await.expect_err("refused");
    assert_eq!(error_data(err)["code"], "description-required");
}

#[tokio::test]
async fn a_consent_window_outside_the_bound_is_refused() {
    for valid_for_secs in [59, 7 * 86_400 + 1] {
        let f = fixture();
        stub_conversation(&f.host, CONVERSATION, CONVERSATION_ADDRESS);
        let now = f.host.now();
        let owner = Identity::generate().expect("owner key");
        let (envelope, _) = sign_listing(&owner, ready_listing(PaymentModel::Fixed), now);
        let mut spec = tap(&envelope);
        spec.valid_for_secs = valid_for_secs;
        let err = enquiry_ops::set(&f.host, input(Some(spec), None)).await.expect_err("refused");
        assert_eq!(error_data(err)["code"], "consent-window-invalid");
    }
}

#[tokio::test]
async fn an_offering_that_does_not_verify_is_refused() {
    let f = fixture();
    stub_conversation(&f.host, CONVERSATION, CONVERSATION_ADDRESS);
    let now = f.host.now();
    let owner = Identity::generate().expect("owner key");
    let (envelope, _) = sign_listing(&owner, ready_listing(PaymentModel::Fixed), now);
    let mut value: serde_json::Value = serde_json::from_str(&envelope).expect("json");
    value["payload"]["title"] = json!("Tampered");
    let err = enquiry_ops::set(&f.host, input(Some(tap(&value.to_string())), None))
        .await
        .expect_err("refused");
    assert_eq!(error_data(err)["code"], "as-listed-not-complete");
}

#[tokio::test]
async fn a_missing_conversation_refuses_the_tap() {
    let f = fixture();
    let now = f.host.now();
    let owner = Identity::generate().expect("owner key");
    let (envelope, _) = sign_listing(&owner, ready_listing(PaymentModel::Fixed), now);
    f.host.on_call("conversation", "conversation.list", |_params| {
        Ok(json!({ "conversations": [] }).to_string())
    });
    let err =
        enquiry_ops::set(&f.host, input(Some(tap(&envelope)), None)).await.expect_err("refused");
    assert_eq!(error_data(err)["code"], "as-listed-wrong-conversation");
}

#[tokio::test]
async fn the_import_checks_as_listed_rows() {
    let f = fixture();
    let owner = Identity::generate().expect("owner key");
    stub_conversation(&f.host, CONVERSATION, CONVERSATION_ADDRESS);
    let now = f.host.now();
    let (envelope, _) = sign_listing(&owner, ready_listing(PaymentModel::Fixed), now);
    enquiry_ops::set(&f.host, input(Some(tap(&envelope)), None)).await.expect("enquiry set");

    let bundle = backup::export(&f.host, NoParams {}).await.expect("export");
    assert!(bundle.sections.contains_key(SECTION_AS_LISTED));

    let mut tampered = bundle.clone();
    let rows = tampered.sections.get_mut(SECTION_AS_LISTED).expect("section");
    rows[0]["payload"]["envelope"] = json!("{}");
    let err = backup::import(&f.host, ImportInput { bundle: tampered }).await.expect_err("refused");
    let message = err.error.expect("error").message;
    assert!(message.contains("as_listed"), "the refusal must name the section: {message}");

    backup::import(&f.host, ImportInput { bundle }).await.expect("import");
}
