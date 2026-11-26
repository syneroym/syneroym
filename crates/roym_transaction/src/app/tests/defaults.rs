use std::{collections::BTreeMap, sync::Arc};

use serde_json::{Value, json};
use syneroym_identity::Identity;
use syneroym_roym_core::{
    backup::{Bundle, ImportInput, SECTION_DEAL_DEFAULTS},
    clock,
    deal::{LocalText, MAX_TERMS_TEXT_LEN},
    envelope::Response,
    lang::LangTag,
    method::NoParams,
    signing,
};

use super::{fixture, fixture_with_master};
use crate::app::{
    SCHEMA_VERSION, backup,
    defaults_ops::{self, DealDefaults},
};

fn code_of(err: &Response) -> Value {
    err.error.as_ref().and_then(|e| e.data.clone()).unwrap_or(Value::Null)
}

fn message_of(err: &Response) -> String {
    err.error.as_ref().map(|e| e.message.clone()).unwrap_or_default()
}

fn sample_defaults() -> DealDefaults {
    let en = LangTag::parse_canonical("en").expect("lang");
    DealDefaults {
        cancellation_terms: LocalText::plain(en.clone(), "Standard cancellation terms"),
        refund_terms: LocalText::plain(en.clone(), "Standard refund terms"),
        dispute_path: LocalText::plain(en, "Standard dispute path"),
        quote_valid_secs: 86400,
    }
}

#[tokio::test]
async fn set_and_get_deal_defaults() {
    let fix = fixture();

    let initial = defaults_ops::get(&fix.host, NoParams {}).await.expect("initial get succeeds");
    assert!(initial.is_none());

    let def = sample_defaults();
    let set_out = defaults_ops::set(&fix.host, def.clone()).await.expect("set succeeds");
    assert_eq!(set_out, def);

    let retrieved =
        defaults_ops::get(&fix.host, NoParams {}).await.expect("retrieved get succeeds");
    assert_eq!(retrieved, Some(def));
}

#[tokio::test]
async fn deal_defaults_validation_refusals() {
    let fix = fixture();
    let en = LangTag::parse_canonical("en").expect("lang");
    let fr = LangTag::parse_canonical("fr").expect("lang");

    // 1. quote_valid_secs too short (< 300)
    let mut d_short = sample_defaults();
    d_short.quote_valid_secs = 299;
    let err_short = defaults_ops::set(&fix.host, d_short).await.unwrap_err();
    assert_eq!(message_of(&err_short), "bad-deal-defaults");
    assert_eq!(code_of(&err_short)["code"], "bad-deal-defaults");

    // 2. quote_valid_secs too long (> 90 days = 7_776_000)
    let mut d_long = sample_defaults();
    d_long.quote_valid_secs = 7_776_001;
    let err_long = defaults_ops::set(&fix.host, d_long).await.unwrap_err();
    assert_eq!(message_of(&err_long), "bad-deal-defaults");
    assert_eq!(code_of(&err_long)["code"], "bad-deal-defaults");

    // 3. Empty text
    let mut d_empty = sample_defaults();
    d_empty.cancellation_terms = LocalText::plain(en.clone(), "   ");
    let err_empty = defaults_ops::set(&fix.host, d_empty).await.unwrap_err();
    assert_eq!(message_of(&err_empty), "bad-deal-defaults");
    assert_eq!(code_of(&err_empty)["code"], "bad-deal-defaults");

    // 4. Overlength text (> 2048 bytes)
    let mut d_over = sample_defaults();
    d_over.refund_terms = LocalText::plain(en.clone(), "x".repeat(MAX_TERMS_TEXT_LEN + 1));
    let err_over = defaults_ops::set(&fix.host, d_over).await.unwrap_err();
    assert_eq!(message_of(&err_over), "bad-deal-defaults");
    assert_eq!(code_of(&err_over)["code"], "bad-deal-defaults");

    // 5. Language mismatch
    let mut d_mismatch = sample_defaults();
    d_mismatch.dispute_path = LocalText::plain(fr, "Voie de recours");
    let err_mismatch = defaults_ops::set(&fix.host, d_mismatch).await.unwrap_err();
    assert_eq!(message_of(&err_mismatch), "bad-deal-defaults");
    assert_eq!(code_of(&err_mismatch)["code"], "bad-deal-defaults");
}

#[tokio::test]
async fn deal_defaults_survive_backup_round_trip() {
    let master = Arc::new(Identity::generate().expect("master"));
    let fix1 = fixture_with_master(master.clone());
    let fix2 = fixture_with_master(master);

    let def = sample_defaults();
    defaults_ops::set(&fix1.host, def.clone()).await.expect("set on host 1");

    let bundle = backup::export(&fix1.host, NoParams {}).await.expect("export bundle");
    assert_eq!(bundle.manifest.sections[SECTION_DEAL_DEFAULTS].schema_version, SCHEMA_VERSION);
    assert_eq!(bundle.manifest.sections[SECTION_DEAL_DEFAULTS].record_count, 1);

    let import_out =
        backup::import(&fix2.host, ImportInput { bundle }).await.expect("import succeeds");
    assert!(import_out.imported);

    let restored = defaults_ops::get(&fix2.host, NoParams {}).await.expect("get on host 2");
    assert_eq!(restored, Some(def));
}

#[tokio::test]
async fn backup_import_refuses_corrupted_deal_defaults() {
    let master = Arc::new(Identity::generate().expect("master"));
    let fix1 = fixture_with_master(master.clone());
    let fix2 = fixture_with_master(master);

    let mut sections = BTreeMap::new();
    let corrupted_rows = vec![json!({
        "id": "defaults",
        "payload": {
            "cancellation_terms": { "lang": "en", "text": "Terms", "translations": [] },
            "refund_terms": { "lang": "en", "text": "Terms", "translations": [] },
            "dispute_path": { "lang": "en", "text": "Terms", "translations": [] },
            "quote_valid_secs": 10 // Invalid (< 300)
        }
    })];
    sections.insert(SECTION_DEAL_DEFAULTS.to_string(), corrupted_rows);

    let mut bundle = Bundle::new(fix1.master_did.clone(), SCHEMA_VERSION, sections).unwrap();
    signing::sign_bundle(&fix1.host, &mut bundle, clock::now_secs()).await.unwrap();

    let err = backup::import(&fix2.host, ImportInput { bundle }).await.unwrap_err();
    assert_eq!(message_of(&err), "bad-deal-defaults");
    assert_eq!(code_of(&err)["code"], "bad-deal-defaults");
}
