//! The terms struct test: every field of `DealTerms` and `Commitment` must
//! change `same_terms`.

use std::collections::BTreeSet;

use serde_json::{Value, json};
use syneroym_identity::Identity;
use syneroym_roym_core::{
    area::Area,
    deal::{
        CommitmentKind, DealTerms, Details, DetailsExtra, ExtensionRef, LocalText, ResourceKind,
        TextTranslation,
    },
    listing::ServiceLocation,
    transaction::{DealRole, QuoteLocation, TimeWindow},
};

use super::{as_listed_accept::tap, as_listed_fixture::lang};
use crate::app::as_listed_ops::accept::same_terms;

/// Every field of the terms and of a commitment, changed one at a time. A
/// field added to either type and not named here fails the coverage check.
const TERM_FIELDS: &[&str] = &[
    "address",
    "after",
    "area",
    "cancellation_terms",
    "code",
    "commitment_id",
    "commitments",
    "currency",
    "data",
    "declared_value_minor",
    "description",
    "details",
    "dispute_path",
    "earliest_secs",
    "eep",
    "extension",
    "extra",
    "fees_minor",
    "giver",
    "home_exchange",
    "id",
    "kind",
    "label",
    "lang",
    "latest_secs",
    "listing_id",
    "location",
    "payment_methods",
    "payee",
    "quote_expires_at_secs",
    "refund_terms",
    "resource_kind",
    "schema",
    "scope",
    "tax_minor",
    "text",
    "translations",
    "value",
    "version",
    "where",
    "window",
];

/// The keys of a serialised value, not descending into the two free-form
/// blocks whose contents are the extension's, not the terms'.
fn keys(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (key, inner) in map {
                out.insert(key.clone());
                if key != "data" && key != "eep" {
                    keys(inner, out);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                keys(item, out);
            }
        }
        _ => {}
    }
}

fn each_text(terms: &mut DealTerms) -> Vec<&mut LocalText> {
    let mut texts = vec![
        &mut terms.scope,
        &mut terms.cancellation_terms,
        &mut terms.refund_terms,
        &mut terms.dispute_path,
    ];
    for commitment in &mut terms.commitments {
        texts.push(&mut commitment.description);
    }
    texts
}

/// Changes the named field of every commitment, and says whether the name is
/// a field this test knows.
fn change_field(terms: &mut DealTerms, field: &str) -> bool {
    match field {
        "currency" => terms.currency.push('X'),
        "quote_expires_at_secs" => terms.quote_expires_at_secs = 0,
        "home_exchange" => terms.home_exchange = Some("other".to_string()),
        "commitments" => {
            let first = terms.commitments[0].clone();
            terms.commitments.push(first);
        }
        "listing_id" => terms.listing_id = Some("lst_other".to_string()),
        _ => {
            return change_text_field(terms, field)
                || change_commitment_field(terms, field)
                || change_details_field(terms, field)
                || change_place_field(terms, field);
        }
    }
    true
}

fn change_text_field(terms: &mut DealTerms, field: &str) -> bool {
    match field {
        "lang" => {
            for text in each_text(terms) {
                text.lang = lang("de");
                for tr in &mut text.translations {
                    tr.lang = lang("de");
                }
            }
        }
        "text" => {
            for text in each_text(terms) {
                text.text.push('X');
                for tr in &mut text.translations {
                    tr.text.push('X');
                }
            }
        }
        "translations" => {
            for text in each_text(terms) {
                text.translations
                    .push(TextTranslation { lang: lang("fr"), text: "Autre".to_string() });
            }
        }
        "scope" => terms.scope.text.push('X'),
        "cancellation_terms" => terms.cancellation_terms.text.push('X'),
        "refund_terms" => terms.refund_terms.text.push('X'),
        "dispute_path" => terms.dispute_path.text.push('X'),
        "description" => terms.commitments[0].description.text.push('X'),
        _ => return false,
    }
    true
}

fn change_commitment_field(terms: &mut DealTerms, field: &str) -> bool {
    match field {
        "commitment_id" => terms.commitments[0].commitment_id.push('X'),
        "giver" => terms.commitments[0].giver = DealRole::Counterparty,
        "resource_kind" => terms.commitments[0].resource_kind = ResourceKind::Currency,
        "kind" => terms.commitments[0].kind = CommitmentKind::Currency,
        "extension" => {
            for commitment in &mut terms.commitments {
                commitment.extension = Some(ExtensionRef { id: "other".to_string(), version: 2 });
            }
        }
        "after" => terms.commitments[0].after.push("x".to_string()),
        "declared_value_minor" => terms.commitments[0].declared_value_minor += 1,
        "payee" => terms.commitments[0].payee = Some("other".to_string()),
        "payment_methods" => {
            for commitment in &mut terms.commitments {
                if let Some(methods) = &mut commitment.payment_methods {
                    methods.push("other".to_string());
                }
            }
        }
        "tax_minor" => terms.commitments[0].tax_minor = Some(2),
        "fees_minor" => terms.commitments[0].fees_minor = Some(2),
        "eep" => terms.commitments[0].eep = Some(json!({"k": 2})),
        _ => return false,
    }
    true
}

fn change_details_field(terms: &mut DealTerms, field: &str) -> bool {
    match field {
        "details" => {
            for commitment in &mut terms.commitments {
                if let Some(details) = &mut commitment.details {
                    details.schema.push('X');
                }
            }
        }
        "id" => {
            for commitment in &mut terms.commitments {
                if let Some(ext) = &mut commitment.extension {
                    ext.id.push('X');
                }
            }
        }
        "version" => {
            for commitment in &mut terms.commitments {
                if let Some(ext) = &mut commitment.extension {
                    ext.version += 1;
                }
            }
        }
        "schema" => {
            for commitment in &mut terms.commitments {
                if let Some(details) = &mut commitment.details {
                    details.schema.push('X');
                }
            }
        }
        "data" => {
            for commitment in &mut terms.commitments {
                if let Some(details) = &mut commitment.details {
                    details.data = json!({"x": 2});
                }
            }
        }
        "extra" => {
            for commitment in &mut terms.commitments {
                if let Some(details) = &mut commitment.details {
                    details
                        .extra
                        .push(DetailsExtra { label: "x".to_string(), value: "y".to_string() });
                }
            }
        }
        "value" => {
            for commitment in &mut terms.commitments {
                if let Some(details) = &mut commitment.details {
                    for extra in &mut details.extra {
                        extra.value.push('X');
                    }
                }
            }
        }
        _ => return false,
    }
    true
}

fn change_place_field(terms: &mut DealTerms, field: &str) -> bool {
    match field {
        "window" => {
            for commitment in &mut terms.commitments {
                if let Some(window) = &mut commitment.window {
                    window.earliest_secs += 1;
                }
            }
        }
        "location" => {
            for commitment in &mut terms.commitments {
                if let Some(location) = &mut commitment.location {
                    location.address = Some("other".to_string());
                }
            }
        }
        "earliest_secs" => {
            for commitment in &mut terms.commitments {
                if let Some(window) = &mut commitment.window {
                    window.earliest_secs += 1;
                }
            }
        }
        "latest_secs" => {
            for commitment in &mut terms.commitments {
                if let Some(window) = &mut commitment.window {
                    window.latest_secs += 1;
                }
            }
        }
        "where" => {
            for commitment in &mut terms.commitments {
                if let Some(location) = &mut commitment.location {
                    location.where_ = ServiceLocation::AtReceiver;
                }
            }
        }
        "area" => {
            for commitment in &mut terms.commitments {
                if let Some(location) = &mut commitment.location {
                    location.area = Some(Area::Named {
                        label: "other".to_string(),
                        code: Some("x".to_string()),
                    });
                }
            }
        }
        "address" => {
            for commitment in &mut terms.commitments {
                if let Some(location) = &mut commitment.location {
                    location.address = Some("other".to_string());
                }
            }
        }
        "label" => {
            for commitment in &mut terms.commitments {
                if let Some(location) = &mut commitment.location
                    && let Some(Area::Named { label, .. }) = &mut location.area
                {
                    label.push('X');
                }
            }
        }
        "code" => {
            for commitment in &mut terms.commitments {
                if let Some(location) = &mut commitment.location
                    && let Some(Area::Named { code, .. }) = &mut location.area
                {
                    *code = Some("other".to_string());
                }
            }
        }
        _ => return false,
    }
    true
}

/// A value with every field set, so the coverage walk sees every key.
fn full_terms(base: &DealTerms, now: u64) -> DealTerms {
    let mut terms = base.clone();
    terms.quote_expires_at_secs = now + 3_600;
    terms.home_exchange = Some("RE".to_string());
    terms.listing_id = Some("lst_full".to_string());
    for text in each_text(&mut terms) {
        text.translations =
            vec![TextTranslation { lang: lang("fr"), text: "Traduction".to_string() }];
    }
    for commitment in &mut terms.commitments {
        commitment.extension = Some(ExtensionRef { id: "scheduling".to_string(), version: 1 });
        commitment.after = vec!["c1".to_string()];
        commitment.details = Some(Details {
            extension: "scheduling".to_string(),
            schema: "scheduling/slot@1".to_string(),
            data: json!({"slot_id": "slot_1"}),
            extra: vec![DetailsExtra { label: "label".to_string(), value: "value".to_string() }],
        });
        commitment.window = Some(TimeWindow { earliest_secs: now, latest_secs: now + 3_600 });
        commitment.location = Some(QuoteLocation {
            where_: ServiceLocation::AtGiver,
            area: Some(Area::Named { label: "area".to_string(), code: Some("code".to_string()) }),
            address: Some("address".to_string()),
        });
        commitment.payee = Some("payee".to_string());
        commitment.payment_methods = Some(vec!["cash".to_string()]);
        commitment.tax_minor = Some(1);
        commitment.fees_minor = Some(1);
        commitment.eep = Some(json!({"k": 1}));
    }
    terms
}

#[test]
fn a_changed_field_anywhere_in_the_terms_fails_the_check() {
    let now = 1_700_000_000;
    let owner = Identity::generate().expect("owner");
    let tap = tap(&owner, now);
    let base = full_terms(&tap.terms, now);
    assert!(same_terms(&base, &base, now, now));

    let mut seen = BTreeSet::new();
    keys(&serde_json::to_value(&base).expect("terms json"), &mut seen);
    let expected: BTreeSet<&str> = TERM_FIELDS.iter().copied().collect();
    assert_eq!(seen, expected.into_iter().map(str::to_string).collect());

    for field in TERM_FIELDS {
        let mut changed = base.clone();
        assert!(change_field(&mut changed, field), "no change for field '{field}'");
        assert!(!same_terms(&base, &changed, now, now), "field '{field}' must fail the check");
    }
}
