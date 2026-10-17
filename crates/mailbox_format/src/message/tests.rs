use syneroym_identity::delegation::SCOPE_ROUTING;

use super::*;
use crate::test_support::{CERT_LIFE_SECS, Member, TTL_SECS, common_now, content, seal_between};

const DAY: u64 = 24 * 3600;

fn open_default(receiver: &Member, sealed: &Sealed, now: u64) -> Result<OpenedMessage, OpenError> {
    open(
        &receiver.mailbox,
        &receiver.service_id(),
        &sealed.deposit.deposit_id,
        &sealed.deposit.sealed,
        OpenPolicy::default(),
        now,
    )
}

/// Seals `content()` by hand with the checks of `seal` skipped.
fn seal_at(sender: Sender<'_>, receiver: &Member, deposit_id: DepositId, sent_at: u64) -> Sealed {
    let id = receiver.service_id();
    seal_unchecked(&content(), receiver.recipient(&id), sender, deposit_id, TTL_SECS, sent_at)
        .unwrap()
}

fn refused(result: Result<OpenedMessage, OpenError>) -> RefusedMessage {
    match result {
        Err(OpenError::Refused(refused)) => *refused,
        other => panic!("expected a refused message, got {other:?}"),
    }
}

#[test]
fn a_sealed_message_opens_with_its_content_sender_and_hash() {
    let (sender, receiver) = (Member::new(CERT_LIFE_SECS), Member::new(CERT_LIFE_SECS));
    let (sealed, now) = seal_between(&sender, &receiver);
    let opened = open_default(&receiver, &sealed, now).unwrap();
    assert_eq!(opened.content, content());
    assert_eq!(opened.sender_master_did(), sender.service_id());
    assert_eq!(opened.sender_instance_did(), sender.cert.temporary_did);
    assert_eq!(opened.sent_at_secs, now);
    assert_eq!(opened.deposit_id, sealed.sent.deposit_id);
    assert_eq!(opened.message_hash, sealed.sent.message_hash);
    assert_eq!(sealed.sent.receiver_service_id, receiver.service_id());
    assert_eq!(sealed.sent.sender_master_did, sender.service_id());
}

#[test]
fn the_mailbox_sees_no_interface_method_or_params() {
    let (sender, receiver) = (Member::new(CERT_LIFE_SECS), Member::new(CERT_LIFE_SECS));
    let (sealed, _) = seal_between(&sender, &receiver);
    let wire = serde_json::to_string(&sealed.deposit).unwrap();
    for secret in ["conversation", "deliver", "msg-1", "hello world", "room", "ünï"] {
        assert!(!wire.contains(secret), "the deposit shows {secret}");
    }
    assert!(wire.contains(&receiver.service_id()));
}

#[test]
fn opening_with_another_services_key_fails() {
    let (sender, receiver, other) =
        (Member::new(CERT_LIFE_SECS), Member::new(CERT_LIFE_SECS), Member::new(CERT_LIFE_SECS));
    let (sealed, now) = seal_between(&sender, &receiver);
    assert_eq!(open_default(&other, &sealed, now), Err(OpenError::Undecryptable));
}

#[test]
fn a_tampered_sealed_body_fails() {
    let (sender, receiver) = (Member::new(CERT_LIFE_SECS), Member::new(CERT_LIFE_SECS));
    let (sealed, now) = seal_between(&sender, &receiver);
    let mut bytes = sealed.deposit.sealed.as_slice().to_vec();
    let middle = bytes.len() / 2;
    bytes[middle] ^= 0x01;
    let tampered = Sealed {
        deposit: Deposit { sealed: SealedBytes::new(bytes), ..sealed.deposit.clone() },
        sent: sealed.sent.clone(),
    };
    assert_eq!(open_default(&receiver, &tampered, now), Err(OpenError::Undecryptable));
}

#[test]
fn a_message_opened_under_another_deposit_id_fails() {
    let (sender, receiver) = (Member::new(CERT_LIFE_SECS), Member::new(CERT_LIFE_SECS));
    let (sealed, now) = seal_between(&sender, &receiver);
    let other_id = DepositId::random().unwrap();
    let result = open(
        &receiver.mailbox,
        &receiver.service_id(),
        &other_id,
        &sealed.deposit.sealed,
        OpenPolicy::default(),
        now,
    );
    assert_eq!(result, Err(OpenError::BadSignature));
}

#[test]
fn a_message_opened_by_another_destination_fails() {
    let (sender, receiver, other) =
        (Member::new(CERT_LIFE_SECS), Member::new(CERT_LIFE_SECS), Member::new(CERT_LIFE_SECS));
    let (sealed, now) = seal_between(&sender, &receiver);
    // Same mailbox key, other service id: a message replayed to another
    // service that shares a key.
    let result = open(
        &receiver.mailbox,
        &other.service_id(),
        &sealed.deposit.deposit_id,
        &sealed.deposit.sealed,
        OpenPolicy::default(),
        now,
    );
    assert_eq!(result, Err(OpenError::BadSignature));
}

#[test]
fn the_same_message_opened_twice_gives_the_same_idempotency_key() {
    let (sender, receiver) = (Member::new(CERT_LIFE_SECS), Member::new(CERT_LIFE_SECS));
    let (sealed, now) = seal_between(&sender, &receiver);
    let first = open_default(&receiver, &sealed, now).unwrap();
    let second = open_default(&receiver, &sealed, now).unwrap();
    assert_eq!(first, second);
    assert_eq!(first.content.idempotency_key, "msg-1");
}

#[test]
fn a_signature_by_a_key_other_than_the_certificates_fails() {
    let (sender, receiver) = (Member::new(CERT_LIFE_SECS), Member::new(CERT_LIFE_SECS));
    let other = Identity::generate().unwrap();
    let now = common_now(&[&sender, &receiver]);
    let wrong = Sender { instance: &other, certificate: &sender.cert };
    let sealed = seal_at(wrong, &receiver, DepositId::random().unwrap(), now);
    assert_eq!(open_default(&receiver, &sealed, now), Err(OpenError::BadSignature));
}

#[test]
fn a_certificate_with_another_scope_is_refused() {
    let (sender, receiver) = (Member::new(CERT_LIFE_SECS), Member::new(CERT_LIFE_SECS));
    let routing = DelegationCertificate::issue(
        &sender.master,
        sender.instance.public_key(),
        CERT_LIFE_SECS,
        SCOPE_ROUTING.to_string(),
    )
    .unwrap();
    let now = common_now(&[&sender, &receiver]);
    let by_routing = Sender { instance: &sender.instance, certificate: &routing };
    let sealed = seal_at(by_routing, &receiver, DepositId::random().unwrap(), now);
    let refused = refused(open_default(&receiver, &sealed, now));
    assert_eq!(refused.code, RejectCode::BadCertificate);
}

#[test]
fn a_certificate_expired_at_send_time_is_refused() {
    let (sender, receiver) = (Member::new(CERT_LIFE_SECS), Member::new(CERT_LIFE_SECS));
    let sent_at = sender.cert.expires_at_secs;
    let sealed = seal_at(sender.sender(), &receiver, DepositId::random().unwrap(), sent_at);
    let refused = refused(open_default(&receiver, &sealed, sent_at));
    assert_eq!(refused.code, RejectCode::BadCertificate);
}

#[test]
fn a_certificate_expired_now_but_valid_at_send_time_is_accepted() {
    let (sender, receiver) = (Member::new(CERT_LIFE_SECS), Member::new(CERT_LIFE_SECS));
    let (sealed, _) = seal_between(&sender, &receiver);
    let later = sender.cert.expires_at_secs + 3_600;
    assert!(open_default(&receiver, &sealed, later).is_ok());
}

#[test]
fn a_send_time_older_than_ttl_plus_skew_is_refused() {
    let (sender, receiver) = (Member::new(CERT_LIFE_SECS), Member::new(CERT_LIFE_SECS));
    let (sealed, sent_at) = seal_between(&sender, &receiver);
    let limit = 7 * DAY + DAY;
    assert!(open_default(&receiver, &sealed, sent_at + limit).is_ok());
    let refused = refused(open_default(&receiver, &sealed, sent_at + limit + 1));
    assert_eq!(refused.code, RejectCode::TooOld);
}

#[test]
fn a_larger_max_ttl_accepts_an_older_message() {
    let (sender, receiver) = (Member::new(CERT_LIFE_SECS), Member::new(CERT_LIFE_SECS));
    let (sealed, sent_at) = seal_between(&sender, &receiver);
    let now = sent_at + 10 * DAY;
    let policy = OpenPolicy { max_ttl_secs: 30 * DAY, ..OpenPolicy::default() };
    let open_with_policy = |policy| {
        open(
            &receiver.mailbox,
            &receiver.service_id(),
            &sealed.deposit.deposit_id,
            &sealed.deposit.sealed,
            policy,
            now,
        )
    };
    assert!(open_with_policy(policy).is_ok());
    assert!(matches!(open_with_policy(OpenPolicy::default()), Err(OpenError::Refused(_))));
}

#[test]
fn a_send_time_too_far_in_the_future_is_refused() {
    let (sender, receiver) = (Member::new(CERT_LIFE_SECS), Member::new(CERT_LIFE_SECS));
    let (sealed, sent_at) = seal_between(&sender, &receiver);
    assert!(open_default(&receiver, &sealed, sent_at - DAY).is_ok());
    let refused = refused(open_default(&receiver, &sealed, sent_at - DAY - 1));
    assert_eq!(refused.code, RejectCode::FromFuture);
}

#[test]
fn seal_refuses_content_over_the_size_limit() {
    let (sender, receiver) = (Member::new(CERT_LIFE_SECS), Member::new(CERT_LIFE_SECS));
    let mut big = content();
    big.params = json!({ "text": "x".repeat(70_000) });
    let id = receiver.service_id();
    let result = seal(&big, receiver.recipient(&id), sender.sender(), TTL_SECS, sender.at());
    assert!(matches!(result, Err(SealError::TooLarge { max: MAX_SEALED_BYTES, .. })));
}

#[test]
fn seal_refuses_an_instance_key_that_does_not_match_the_certificate() {
    let (sender, receiver) = (Member::new(CERT_LIFE_SECS), Member::new(CERT_LIFE_SECS));
    let other = Identity::generate().unwrap();
    let wrong = Sender { instance: &other, certificate: &sender.cert };
    let id = receiver.service_id();
    let result = seal(&content(), receiver.recipient(&id), wrong, TTL_SECS, sender.at());
    assert_eq!(result.unwrap_err(), SealError::SignerMismatch);
}

#[test]
fn seal_refuses_an_expired_certificate() {
    let (sender, receiver) = (Member::new(CERT_LIFE_SECS), Member::new(CERT_LIFE_SECS));
    let id = receiver.service_id();
    let now = sender.cert.expires_at_secs;
    let result = seal(&content(), receiver.recipient(&id), sender.sender(), TTL_SECS, now);
    assert!(matches!(result, Err(SealError::BadCertificate(_))));
}

#[test]
fn seal_refuses_a_zero_ttl_and_a_destination_that_is_not_a_did_key() {
    let (sender, receiver) = (Member::new(CERT_LIFE_SECS), Member::new(CERT_LIFE_SECS));
    let id = receiver.service_id();
    let zero = seal(&content(), receiver.recipient(&id), sender.sender(), 0, sender.at());
    assert_eq!(zero.unwrap_err(), SealError::ZeroTtl);
    let web = receiver.recipient("did:web:example.com");
    let result = seal(&content(), web, sender.sender(), TTL_SECS, sender.at());
    assert!(matches!(result, Err(SealError::NotADidKey(_))));
}

#[test]
fn inner_content_with_an_unknown_field_still_opens() {
    let (sender, receiver) = (Member::new(CERT_LIFE_SECS), Member::new(CERT_LIFE_SECS));
    let now = common_now(&[&sender, &receiver]);
    let deposit_id = DepositId::random().unwrap();
    let c = content();
    let p_hash = payload_hash(&c.interface, &c.method, &c.params).unwrap();
    let statement = message_statement(
        &receiver.service_id(),
        &sender.service_id(),
        &deposit_id,
        &c.idempotency_key,
        &p_hash,
        now,
    );
    let inner = json!({
        "interface": c.interface,
        "method": c.method,
        "params": c.params,
        "idempotency_key": c.idempotency_key,
        "sent_at_secs": now,
        "sender_certificate": sender.cert,
        "signature": sender.instance.sign_json(&statement).unwrap(),
        "added_by_a_newer_version": [1, 2],
    });
    let bytes =
        crypto::seal_to(receiver.mailbox_public.as_bytes(), &serde_json::to_vec(&inner).unwrap())
            .unwrap();
    let opened = open(
        &receiver.mailbox,
        &receiver.service_id(),
        &deposit_id,
        &SealedBytes::new(bytes),
        OpenPolicy::default(),
        now,
    )
    .unwrap();
    assert_eq!(opened.content, c);
}

#[test]
fn payload_hash_is_stable_across_a_json_round_trip() {
    let params = json!({
        "z": {"b": 1, "a": [0.1, 1e-7, 123456789.125]},
        "ünï": "ok",
        "a": null,
    });
    let first = payload_hash("conversation", "deliver", &params).unwrap();
    let parsed: Value = serde_json::from_str(&serde_json::to_string(&params).unwrap()).unwrap();
    assert_eq!(payload_hash("conversation", "deliver", &parsed).unwrap(), first);
    // The method is part of the hash.
    assert_ne!(payload_hash("conversation", "other", &params).unwrap(), first);
}

#[test]
fn sealing_the_same_call_twice_gives_new_ids_and_hashes() {
    let (sender, receiver) = (Member::new(CERT_LIFE_SECS), Member::new(CERT_LIFE_SECS));
    let (first, now) = seal_between(&sender, &receiver);
    let id = receiver.service_id();
    let second =
        seal(&content(), receiver.recipient(&id), sender.sender(), TTL_SECS, now + 1).unwrap();
    assert_ne!(first.sent.deposit_id, second.sent.deposit_id);
    assert_ne!(first.sent.message_hash, second.sent.message_hash);
    assert!(open_default(&receiver, &first, now + 1).is_ok());
    assert!(open_default(&receiver, &second, now + 1).is_ok());
}

#[test]
fn a_message_hash_has_one_lowercase_text_form() {
    let (sender, receiver) = (Member::new(CERT_LIFE_SECS), Member::new(CERT_LIFE_SECS));
    let (sealed, _) = seal_between(&sender, &receiver);
    let text = sealed.sent.message_hash.to_string();
    assert_eq!(text.len(), 64);
    assert_eq!(text.parse::<MessageHash>().unwrap(), sealed.sent.message_hash);
    assert!(text.to_uppercase().parse::<MessageHash>().is_err());
    assert!("ab".parse::<MessageHash>().is_err());
}

/// Floats from a fixed generator: latitudes, fractions and price times rate.
/// Many of these change by one bit through a parser that is not exact.
fn sample_floats() -> Vec<f64> {
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 11) as f64 / (1u64 << 53) as f64
    };
    let mut floats = vec![0.01 * 1.07];
    for _ in 0..200 {
        floats.push(next());
        floats.push(next() * 180.0 - 90.0);
        floats.push(next() * 100.0 * 1.0725);
    }
    floats
}

#[test]
fn payload_hash_is_stable_for_floats_that_change_in_an_inexact_parser() {
    for value in sample_floats() {
        let params = json!({ "value": value });
        let parsed: Value = serde_json::from_str(&serde_json::to_string(&params).unwrap()).unwrap();
        assert_eq!(
            payload_hash("conversation", "deliver", &params).unwrap(),
            payload_hash("conversation", "deliver", &parsed).unwrap(),
            "{value:?}"
        );
    }
}

#[test]
fn a_message_with_computed_floats_in_its_params_opens() {
    let (sender, receiver) = (Member::new(CERT_LIFE_SECS), Member::new(CERT_LIFE_SECS));
    let mut computed = content();
    computed.params = json!({ "values": sample_floats() });
    let now = common_now(&[&sender, &receiver]);
    let id = receiver.service_id();
    let sealed = seal(&computed, receiver.recipient(&id), sender.sender(), TTL_SECS, now).unwrap();
    let opened = open_default(&receiver, &sealed, now).unwrap();
    assert_eq!(opened.content, computed);
}

#[test]
fn a_certificate_from_another_master_for_the_same_key_fails() {
    // Any master can issue a valid certificate for any instance key. The
    // mailbox swaps the certificate and keeps the signature.
    let (sender, receiver) = (Member::new(CERT_LIFE_SECS), Member::new(CERT_LIFE_SECS));
    let other_master = Identity::generate().unwrap();
    let swapped = DelegationCertificate::issue(
        &other_master,
        sender.instance.public_key(),
        CERT_LIFE_SECS,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    let now = common_now(&[&sender, &receiver]);
    let deposit_id = DepositId::random().unwrap();
    let honest = seal_at(sender.sender(), &receiver, deposit_id, now);
    let by_other = Sender { instance: &sender.instance, certificate: &swapped };
    let relabelled = seal_at(by_other, &receiver, deposit_id, now);
    assert_ne!(relabelled.sent.message_hash, honest.sent.message_hash);

    // The mailbox keeps the honest signature and puts the other certificate
    // in the inner part. `seal_unchecked` signs again, so build the inner
    // part by hand from the honest one.
    let plaintext =
        crypto::open_with(receiver.mailbox.seed(), honest.deposit.sealed.as_slice()).unwrap();
    let mut inner: Value = serde_json::from_slice(&plaintext).unwrap();
    inner["sender_certificate"] = serde_json::to_value(&swapped).unwrap();
    let bytes =
        crypto::seal_to(receiver.mailbox_public.as_bytes(), &serde_json::to_vec(&inner).unwrap())
            .unwrap();
    let swapped_sealed = Sealed {
        deposit: Deposit { sealed: SealedBytes::new(bytes), ..honest.deposit.clone() },
        sent: honest.sent.clone(),
    };
    assert_eq!(open_default(&receiver, &swapped_sealed, now), Err(OpenError::BadSignature));
}
