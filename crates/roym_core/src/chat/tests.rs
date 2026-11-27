use serde_json::json;
use syneroym_app_host::types::signing::RecordDraft as AppRecordDraft;
use syneroym_identity::{
    Identity,
    delegation::{DelegationCertificate, SCOPE_RECORD_SIGNING, SCOPE_SERVICE_INSTANCE},
    substrate::derive_did_key,
};

use super::*;
use crate::record::RecordDraft as EnvelopeDraft;

fn to_envelope_draft(draft: AppRecordDraft) -> EnvelopeDraft {
    EnvelopeDraft {
        version: draft.version,
        record_type: draft.record_type,
        subject: draft.subject,
        payload: serde_json::from_str(&draft.payload).unwrap(),
        expires_at_secs: draft.expires_at_secs,
        supersedes: draft.supersedes,
    }
}

struct Signer {
    service: Identity,
    service_did: String,
    master_did: String,
    cert_json: String,
    cert_issued_at: u64,
    cert_expires_at: u64,
}

fn signer_with(scope: &str, lifetime: u64) -> Signer {
    let master = Identity::generate().unwrap();
    let service = Identity::generate().unwrap();
    let cert =
        DelegationCertificate::issue(&master, service.public_key(), lifetime, scope.to_string())
            .unwrap();
    Signer {
        service_did: derive_did_key(&service.public_key()),
        service,
        master_did: derive_did_key(&master.public_key()),
        cert_json: cert.to_json().unwrap(),
        cert_issued_at: cert.issued_at_secs,
        cert_expires_at: cert.expires_at_secs,
    }
}

fn signer() -> Signer {
    signer_with(SCOPE_RECORD_SIGNING, 3_600)
}

impl Signer {
    /// Signed as the person: the envelope names the master DID and carries
    /// the service's delegation certificate.
    fn sign_as_person(&self, draft: AppRecordDraft, at: u64) -> String {
        self.sign(
            to_envelope_draft(draft),
            self.master_did.clone(),
            Some(self.cert_json.clone()),
            at,
        )
    }

    /// Signed by the service key alone, with no delegation.
    fn sign_alone(&self, draft: AppRecordDraft, at: u64) -> String {
        self.sign(to_envelope_draft(draft), self.service_did.clone(), None, at)
    }

    fn sign(
        &self,
        draft: EnvelopeDraft,
        issuer: String,
        delegation: Option<String>,
        at: u64,
    ) -> String {
        let (mut env, bytes) = Envelope::unsigned(draft, issuer, delegation, at).unwrap();
        let sig = z32::encode(&self.service.sign(&bytes).to_bytes());
        env.attach_signature(sig).unwrap();
        env.to_json().unwrap()
    }
}

fn payload_for(text: &str, languages: &[&str]) -> MessageOriginPayload {
    let languages: Vec<String> = languages.iter().map(|s| (*s).to_string()).collect();
    MessageOriginPayload {
        content_type: INNER_TEXT_CONTENT_TYPE.to_string(),
        content_hash: content_hash(INNER_TEXT_CONTENT_TYPE, &languages, text.as_bytes(), &[]),
        languages,
    }
}

fn langs(values: &[&str]) -> Vec<String> {
    values.iter().map(|s| (*s).to_string()).collect()
}

fn signed_body(s: &Signer, text: &str, languages: &[&str], at: u64) -> ChatBody {
    let payload = payload_for(text, languages);
    ChatBody {
        chat_version: CHAT_BODY_VERSION,
        content_type: INNER_TEXT_CONTENT_TYPE.to_string(),
        text: text.to_string(),
        forward_mode: ForwardMode::Signed,
        origin: Some(s.sign_as_person(origin_draft(&payload), at)),
        forward_count: None,
    }
}

fn body_json(value: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&value).unwrap()
}

fn base_body() -> serde_json::Value {
    json!({
        "chat_version": 1,
        "content_type": "text/plain",
        "text": "hello",
        "forward_mode": "signed",
    })
}

#[test]
fn a_chat_body_round_trips_through_bytes() {
    let body = ChatBody {
        chat_version: CHAT_BODY_VERSION,
        content_type: INNER_TEXT_CONTENT_TYPE.to_string(),
        text: "Market opens at 9".to_string(),
        forward_mode: ForwardMode::Signed,
        origin: Some("{\"envelope\":1}".to_string()),
        forward_count: Some(2),
    };
    assert_eq!(ChatBody::parse(&body.to_bytes()).unwrap(), body);
}

#[test]
fn a_body_of_another_version_is_unsupported() {
    let mut value = base_body();
    value["chat_version"] = json!(2);
    assert!(matches!(ChatBody::parse(&body_json(value)), Err(ChatBodyError::Unsupported(_))));
}

#[test]
fn a_body_with_another_inner_content_type_is_unsupported() {
    let mut value = base_body();
    value["content_type"] = json!("text/html");
    assert!(matches!(ChatBody::parse(&body_json(value)), Err(ChatBodyError::Unsupported(_))));
}

#[test]
fn an_unknown_forward_mode_reads_as_not_forwardable() {
    let mut value = base_body();
    value["forward_mode"] = json!("a-mode-from-a-later-version");
    let body = ChatBody::parse(&body_json(value)).unwrap();
    assert_eq!(body.forward_mode, ForwardMode::NotForwardable);
}

#[test]
fn a_body_that_misses_a_field_is_malformed() {
    let mut value = base_body();
    value.as_object_mut().unwrap().remove("text");
    assert!(matches!(ChatBody::parse(&body_json(value)), Err(ChatBodyError::Malformed(_))));
    assert!(matches!(ChatBody::parse(b"not json"), Err(ChatBodyError::Malformed(_))));
}

#[test]
fn an_origin_on_a_not_forwardable_body_is_ignored() {
    let s = signer();
    let at = s.cert_issued_at + 10;
    let mut body = signed_body(&s, "hello", &[], at);
    body.forward_mode = ForwardMode::NotForwardable;
    assert_eq!(check_origin(&body, &[], at), OriginCheck::Absent);
    let view = forward_view(&body, &[], None, None, at);
    assert!(!view.forwardable);
    assert_eq!(view.origin, None);
    assert_eq!(view.origin_error, None);
}

#[test]
fn an_origin_over_the_size_limit_fails_as_too_large() {
    let mut body = ChatBody {
        chat_version: CHAT_BODY_VERSION,
        content_type: INNER_TEXT_CONTENT_TYPE.to_string(),
        text: "hello".to_string(),
        forward_mode: ForwardMode::Signed,
        origin: Some("x".repeat(MAX_ORIGIN_BYTES + 1)),
        forward_count: None,
    };
    assert_eq!(
        check_origin(&body, &[], 0),
        OriginCheck::Failed { reason: "origin too large".to_string() }
    );
    body.origin = Some("x".repeat(MAX_ORIGIN_BYTES));
    assert!(
        matches!(check_origin(&body, &[], 0), OriginCheck::Failed { reason } if reason != "origin too large")
    );
}

#[test]
fn a_forward_count_of_zero_or_a_negative_count_reads_as_none() {
    for raw in [json!(0), json!(-3), json!(-1.5)] {
        let mut value = base_body();
        value["forward_count"] = raw.clone();
        let body = ChatBody::parse(&body_json(value)).unwrap_or_else(|e| panic!("{raw}: {e}"));
        assert_eq!(body.forward_count, None, "{raw}");
    }
}

#[test]
fn a_huge_forward_count_is_clamped_and_the_body_stays_readable() {
    for raw in [json!(2_000), json!(99_999_999_999u64), json!(1e30)] {
        let mut value = base_body();
        value["forward_count"] = raw.clone();
        let body = ChatBody::parse(&body_json(value)).unwrap_or_else(|e| panic!("{raw}: {e}"));
        assert_eq!(body.forward_count, Some(MAX_FORWARD_COUNT), "{raw}");
    }
}

#[test]
fn a_count_that_is_not_a_number_is_malformed() {
    let mut value = base_body();
    value["forward_count"] = json!("many");
    assert!(matches!(ChatBody::parse(&body_json(value)), Err(ChatBodyError::Malformed(_))));
}

#[test]
fn a_strict_forward_mode_refuses_an_unknown_string() {
    assert!(serde_json::from_str::<ForwardMode>("\"signd\"").is_err());
    assert_eq!(serde_json::from_str::<ForwardMode>("\"signed\"").unwrap(), ForwardMode::Signed);
    assert_eq!(
        serde_json::from_str::<ForwardMode>("\"not-forwardable\"").unwrap(),
        ForwardMode::NotForwardable
    );
}

#[test]
fn unknown_fields_in_a_chat_body_are_ignored() {
    let mut value = base_body();
    value["from_a_later_version"] = json!({"n": 1});
    assert!(ChatBody::parse(&body_json(value)).is_ok());
}

#[test]
fn the_content_hash_is_stable() {
    assert_eq!(
        content_hash(INNER_TEXT_CONTENT_TYPE, &[], b"hello", &[]),
        "sha256:c4c9260ddc0d2312aa46c765dcf81c258de1ac3558367067feb206a53327acbf"
    );
    assert_eq!(
        content_hash(INNER_TEXT_CONTENT_TYPE, &langs(&["en"]), b"Market opens at 9", &[]),
        "sha256:c6cb988786ec95c032c2bcbc2bd1e12627566753603574a6984cc757523f0f97"
    );
}

#[test]
fn the_hash_changes_with_the_text() {
    assert_ne!(
        content_hash("text/plain", &[], b"hello", &[]),
        content_hash("text/plain", &[], b"hello!", &[])
    );
}

#[test]
fn the_hash_changes_with_one_language_tag() {
    assert_ne!(
        content_hash("text/plain", &langs(&["en"]), b"hello", &[]),
        content_hash("text/plain", &langs(&["en", "hi"]), b"hello", &[])
    );
}

#[test]
fn the_hash_changes_with_the_order_of_tags() {
    assert_ne!(
        content_hash("text/plain", &langs(&["en", "hi"]), b"hello", &[]),
        content_hash("text/plain", &langs(&["hi", "en"]), b"hello", &[])
    );
}

#[test]
fn the_hash_changes_with_the_content_type() {
    assert_ne!(
        content_hash("text/plain", &[], b"hello", &[]),
        content_hash("text/html", &[], b"hello", &[])
    );
}

#[test]
fn the_hash_changes_with_the_blob_list() {
    assert_ne!(
        content_hash("text/plain", &[], b"hello", &[]),
        content_hash("text/plain", &[], b"hello", &[[7u8; 32]])
    );
    assert_ne!(
        content_hash("text/plain", &[], b"hello", &[[7u8; 32]]),
        content_hash("text/plain", &[], b"hello", &[[8u8; 32]])
    );
}

#[test]
fn length_prefixes_keep_two_splits_apart() {
    assert_ne!(
        content_hash("text/plain", &langs(&["ab", "c"]), b"", &[]),
        content_hash("text/plain", &langs(&["a", "bc"]), b"", &[])
    );
}

#[test]
fn a_signed_origin_checks_and_names_the_master_did() {
    let s = signer();
    let at = s.cert_issued_at + 10;
    let body = signed_body(&s, "hello", &["en"], at);
    match check_origin(&body, &langs(&["en"]), at) {
        OriginCheck::Checked { author_did, written_at_secs, record_id } => {
            assert_eq!(author_did, s.master_did);
            assert_eq!(written_at_secs, at);
            assert!(record_id.starts_with("rec_"));
        }
        other => panic!("expected checked, got {other:?}"),
    }
}

#[test]
fn an_origin_still_checks_after_its_certificate_expired() {
    let s = signer();
    let at = s.cert_issued_at + 10;
    let body = signed_body(&s, "hello", &[], at);
    let later = s.cert_expires_at + 100_000;
    assert!(later > s.cert_expires_at);
    assert!(matches!(check_origin(&body, &[], later), OriginCheck::Checked { .. }));
}

#[test]
fn an_origin_signed_by_the_service_key_alone_is_refused() {
    let s = signer();
    let at = s.cert_issued_at + 10;
    let payload = payload_for("hello", &[]);
    let body = ChatBody {
        chat_version: CHAT_BODY_VERSION,
        content_type: INNER_TEXT_CONTENT_TYPE.to_string(),
        text: "hello".to_string(),
        forward_mode: ForwardMode::Signed,
        origin: Some(s.sign_alone(origin_draft(&payload), at)),
        forward_count: None,
    };
    assert!(matches!(
        check_origin(&body, &[], at),
        OriginCheck::Failed { reason } if reason.contains("signed as a person")
    ));
}

fn with_draft(s: &Signer, at: u64, mutate: impl FnOnce(&mut AppRecordDraft)) -> ChatBody {
    let payload = payload_for("hello", &[]);
    let mut draft = origin_draft(&payload);
    mutate(&mut draft);
    ChatBody {
        chat_version: CHAT_BODY_VERSION,
        content_type: INNER_TEXT_CONTENT_TYPE.to_string(),
        text: "hello".to_string(),
        forward_mode: ForwardMode::Signed,
        origin: Some(s.sign_as_person(draft, at)),
        forward_count: None,
    }
}

#[test]
fn an_origin_with_a_subject_is_refused() {
    let s = signer();
    let at = s.cert_issued_at + 10;
    let body = with_draft(&s, at, |d| d.subject = "conv".to_string());
    assert!(matches!(
        check_origin(&body, &[], at),
        OriginCheck::Failed { reason } if reason.contains("subject")
    ));
}

#[test]
fn an_origin_with_an_expiry_is_refused() {
    let s = signer();
    let at = s.cert_issued_at + 10;
    let body = with_draft(&s, at, |d| d.expires_at_secs = Some(at + 1_000));
    assert!(matches!(
        check_origin(&body, &[], at),
        OriginCheck::Failed { reason } if reason.contains("expiry")
    ));
}

#[test]
fn an_origin_of_another_record_type_is_refused() {
    let s = signer();
    let at = s.cert_issued_at + 10;
    let body = with_draft(&s, at, |d| d.record_type = "profile".to_string());
    assert!(matches!(
        check_origin(&body, &[], at),
        OriginCheck::Failed { reason } if reason.contains("message-origin")
    ));
}

#[test]
fn an_origin_that_is_another_record_of_the_same_person_is_refused() {
    let s = signer();
    let at = s.cert_issued_at + 10;
    let draft = EnvelopeDraft {
        version: 1,
        record_type: "profile".to_string(),
        subject: s.master_did.clone(),
        payload: json!({"name": "A"}),
        expires_at_secs: None,
        supersedes: None,
    };
    let body = ChatBody {
        chat_version: CHAT_BODY_VERSION,
        content_type: INNER_TEXT_CONTENT_TYPE.to_string(),
        text: "hello".to_string(),
        forward_mode: ForwardMode::Signed,
        origin: Some(s.sign(draft, s.master_did.clone(), Some(s.cert_json.clone()), at)),
        forward_count: None,
    };
    assert!(matches!(check_origin(&body, &[], at), OriginCheck::Failed { .. }));
}

#[test]
fn an_origin_for_other_text_fails_the_hash() {
    let s = signer();
    let at = s.cert_issued_at + 10;
    let mut body = signed_body(&s, "hello", &[], at);
    body.text = "hello there".to_string();
    assert_eq!(
        check_origin(&body, &[], at),
        OriginCheck::Failed { reason: "content hash does not match".to_string() }
    );
}

#[test]
fn an_origin_for_other_languages_fails() {
    let s = signer();
    let at = s.cert_issued_at + 10;
    let body = signed_body(&s, "hello", &["en"], at);
    assert_eq!(
        check_origin(&body, &langs(&["hi"]), at),
        OriginCheck::Failed { reason: "origin is for other languages".to_string() }
    );
}

#[test]
fn a_tampered_origin_signature_fails() {
    let s = signer();
    let at = s.cert_issued_at + 10;
    let mut body = signed_body(&s, "hello", &[], at);
    let mut env = Envelope::from_json(body.origin.as_deref().unwrap()).unwrap();
    env.payload["content_hash"] = json!(content_hash("text/plain", &[], b"other", &[]));
    body.origin = Some(env.to_json().unwrap());
    assert!(matches!(check_origin(&body, &[], at), OriginCheck::Failed { .. }));
}

#[test]
fn an_origin_dated_far_in_the_future_fails_and_checks_once_the_clock_catches_up() {
    let s = signer_with(SCOPE_RECORD_SIGNING, 100_000);
    let signed_at = s.cert_issued_at + 10_000;
    let body = signed_body(&s, "hello", &[], signed_at);
    assert!(matches!(
        check_origin(&body, &[], s.cert_issued_at + 10),
        OriginCheck::Failed { reason } if reason.contains("future")
    ));
    assert!(matches!(check_origin(&body, &[], signed_at), OriginCheck::Checked { .. }));
}

#[test]
fn an_origin_signed_for_one_master_but_naming_another_issuer_fails() {
    let s = signer();
    let at = s.cert_issued_at + 10;
    let other_master = Identity::generate().unwrap();
    let other_did = derive_did_key(&other_master.public_key());
    let payload = payload_for("hello", &[]);
    let body = ChatBody {
        chat_version: CHAT_BODY_VERSION,
        content_type: INNER_TEXT_CONTENT_TYPE.to_string(),
        text: "hello".to_string(),
        forward_mode: ForwardMode::Signed,
        origin: Some(s.sign(
            to_envelope_draft(origin_draft(&payload)),
            other_did,
            Some(s.cert_json.clone()),
            at,
        )),
        forward_count: None,
    };
    let OriginCheck::Failed { reason } = check_origin(&body, &[], at) else {
        panic!("expected the origin check to fail");
    };
    assert!(reason.starts_with("delegation is invalid"), "{reason}");
}

#[test]
fn an_origin_with_a_certificate_of_another_scope_is_refused() {
    let s = signer_with(SCOPE_SERVICE_INSTANCE, 3_600);
    let at = s.cert_issued_at + 10;
    let body = signed_body(&s, "hello", &[], at);
    let OriginCheck::Failed { reason } = check_origin(&body, &[], at) else {
        panic!("expected the origin check to fail");
    };
    assert!(reason.starts_with("delegation is invalid"), "{reason}");
}

#[test]
fn a_changed_text_with_the_old_origin_is_not_forwardable() {
    let s = signer();
    let at = s.cert_issued_at + 10;
    let mut body = signed_body(&s, "Market opens at 9", &["en"], at);
    body.text = "Market opens at 10".to_string();
    let view = forward_view(&body, &langs(&["en"]), None, None, at);
    assert!(!view.forwardable);
    assert_eq!(view.origin, None);
    assert!(view.origin_error.is_some());
}

#[test]
fn a_not_forwardable_body_has_no_origin_and_offers_no_forward() {
    let body = ChatBody {
        chat_version: CHAT_BODY_VERSION,
        content_type: INNER_TEXT_CONTENT_TYPE.to_string(),
        text: "hello".to_string(),
        forward_mode: ForwardMode::NotForwardable,
        origin: None,
        forward_count: None,
    };
    let view = forward_view(&body, &[], None, None, 0);
    assert_eq!(view.mode, ForwardMode::NotForwardable);
    assert!(!view.forwardable);
    assert_eq!(view.origin, None);
    assert!(view.origin_error.is_none());
    assert!(!view.forwarded);
}

#[test]
fn a_forwarded_checked_message_is_marked_forwarded() {
    let s = signer();
    let at = s.cert_issued_at + 10;
    let mut body = signed_body(&s, "hello", &[], at);
    body.forward_count = Some(1);
    let view = forward_view(&body, &[], None, None, at);
    assert!(view.forwardable);
    assert!(view.forwarded);
    assert_eq!(view.forward_count, 1);
    assert!(view.origin.is_some());
    assert!(view.origin_error.is_none());
}

#[test]
fn a_known_sender_who_is_not_the_author_marks_it_forwarded_even_without_a_count() {
    let s = signer();
    let at = s.cert_issued_at + 10;
    let body = signed_body(&s, "hello", &[], at);
    let view = forward_view(&body, &[], Some("did:key:zForwarder"), None, at);
    assert!(view.forwarded);
    assert_eq!(view.forward_count, 0);
}

#[test]
fn a_forward_view_row_for_an_unknown_sender_keeps_the_origin_but_not_the_forwarded_line() {
    let s = signer();
    let at = s.cert_issued_at + 10;
    let body = signed_body(&s, "hello", &[], at);
    let view = forward_view(&body, &[], None, None, at);
    assert!(view.origin.is_some());
    assert!(!view.forwarded);
}

#[test]
fn a_failed_origin_never_names_an_author() {
    let s = signer();
    let at = s.cert_issued_at + 10;
    let mut body = signed_body(&s, "hello", &[], at);
    body.text = "changed".to_string();
    let view = forward_view(&body, &[], None, None, at);
    assert!(!view.forwardable);
    assert_eq!(view.origin, None);
    assert!(view.origin_error.is_some());
    assert!(!view.forwarded);
}

#[test]
fn an_unreadable_body_is_not_forwardable() {
    let view = unreadable_view("message body could not be read");
    assert!(!view.forwardable);
    assert_eq!(view.origin, None);
    assert_eq!(view.origin_error.as_deref(), Some("message body could not be read"));
    assert!(!view.forwarded);
}

#[test]
fn the_author_reading_a_forward_of_their_own_message_sees_is_you() {
    let s = signer();
    let at = s.cert_issued_at + 10;
    let mut body = signed_body(&s, "hello", &[], at);
    body.forward_count = Some(1);
    let view = forward_view(&body, &[], None, Some(&s.master_did), at);
    assert!(view.origin.unwrap().is_you);
    let other = forward_view(&body, &[], None, Some("did:key:zSomeoneElse"), at);
    assert!(!other.origin.unwrap().is_you);
}

#[test]
fn a_forward_of_a_forward_counts_up() {
    assert_eq!(next_forward_count(1, false), 2);
    assert_eq!(next_forward_count(4, false), 5);
}

#[test]
fn many_forwards_starts_at_five() {
    let s = signer();
    let at = s.cert_issued_at + 10;
    for (count, many) in [(4, false), (5, true)] {
        let mut body = signed_body(&s, "hello", &[], at);
        body.forward_count = Some(count);
        assert_eq!(forward_view(&body, &[], None, None, at).many_forwards, many, "count {count}");
    }
}

#[test]
fn forwarding_a_received_message_adds_one() {
    assert_eq!(next_forward_count(0, false), 1);
    assert_eq!(next_forward_count(3, false), 4);
}

#[test]
fn forwarding_your_own_forward_keeps_the_count() {
    assert_eq!(next_forward_count(1, true), 1);
    assert_eq!(next_forward_count(7, true), 7);
    // A first forward of one's own message still counts as one.
    assert_eq!(next_forward_count(0, true), 1);
}

#[test]
fn the_count_never_passes_the_clamp() {
    assert_eq!(next_forward_count(MAX_FORWARD_COUNT, false), MAX_FORWARD_COUNT);
    assert_eq!(next_forward_count(MAX_FORWARD_COUNT, true), MAX_FORWARD_COUNT);
    assert_eq!(next_forward_count(u32::MAX, false), MAX_FORWARD_COUNT);
}

#[test]
fn the_origin_draft_payload_matches_the_struct() {
    let payload = payload_for("hello", &["en"]);
    let draft = origin_draft(&payload);
    assert_eq!(draft.version, 1);
    assert_eq!(draft.record_type, "message-origin");
    assert_eq!(draft.subject, "");
    assert_eq!(draft.expires_at_secs, None);
    assert_eq!(draft.supersedes, None);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&draft.payload).unwrap(),
        serde_json::to_value(&payload).unwrap()
    );
}
