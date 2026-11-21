use std::{collections::BTreeMap, sync::Arc};

use serde_json::{Value, json};
use syneroym_app_host::types::signing::SigningIdentity;
use syneroym_identity::{Identity, substrate};
use syneroym_roym_core::{
    backup::{Bundle, ImportInput, SECTION_AVAILABILITY, SECTION_RESERVATIONS},
    method::{NoParams, scheduling::ReservationState},
    record::{Envelope, RecordDraft as CoreRecordDraft},
    signing::{
        self, CERTIFICATE_ID, CERTIFICATES, DelegationCertificate, MAX_CERTIFICATE_LIFETIME_SECS,
        SCOPE_RECORD_SIGNING, StoredCertificate,
    },
    testing::TestHost,
};

use super::{NOW, code_of, key, reserve_input, row_ids, slot};
use crate::app::{SCHEMA_VERSION, backup, reservation, reservation::RESERVATION_TTL_SECS};

/// A host that holds a signing certificate of `master`, so it can sign a
/// bundle as the person.
fn signed_host(master: &Arc<Identity>) -> TestHost {
    let master_did = substrate::derive_did_key(&master.public_key());
    let service_key = Arc::new(Identity::generate().unwrap());
    let service_did = substrate::derive_did_key(&service_key.public_key());
    let cert = DelegationCertificate::issue(
        master,
        service_key.public_key(),
        MAX_CERTIFICATE_LIFETIME_SECS,
        SCOPE_RECORD_SIGNING.to_string(),
    )
    .unwrap();
    let host = TestHost::default();
    host.set_now(NOW.max(cert.issued_at_secs));
    host.set_signing_id(SigningIdentity {
        signing_did: service_did.clone(),
        pubkey_hex: String::new(),
        owner_did: Some(master_did.clone()),
    });
    let stored = StoredCertificate {
        signing_did: service_did.clone(),
        master_did: master_did.clone(),
        issued_at_secs: cert.issued_at_secs,
        expires_at_secs: cert.expires_at_secs,
        certificate: cert.to_json().unwrap(),
    };
    host.set_storage_row(CERTIFICATES, CERTIFICATE_ID, &stored);
    host.set_record_signer(move |draft, principal, now| {
        let (issuer, delegation) = match principal {
            syneroym_app_host::types::signing::Principal::Service => (service_did.clone(), None),
            syneroym_app_host::types::signing::Principal::Delegated(d) => {
                (master_did.clone(), Some(d))
            }
        };
        let payload = serde_json::from_str(&draft.payload).unwrap();
        let core = CoreRecordDraft {
            version: draft.version,
            record_type: draft.record_type,
            subject: draft.subject,
            payload,
            expires_at_secs: draft.expires_at_secs,
            supersedes: draft.supersedes,
        };
        let (mut env, bytes) = Envelope::unsigned(core, issuer, delegation, now).unwrap();
        env.attach_signature(z32::encode(&service_key.sign(&bytes).to_bytes())).unwrap();
        Ok(env.to_json().unwrap())
    });
    host
}

fn master() -> Arc<Identity> {
    Arc::new(Identity::generate().unwrap())
}

/// A bundle over `sections`, signed as the person by `host`.
async fn bundle_of(host: &TestHost, sections: BTreeMap<String, Vec<Value>>) -> Bundle {
    let owner = signing::owner_did(host).await.unwrap();
    let mut bundle = Bundle::new(owner, SCHEMA_VERSION, sections).unwrap();
    signing::sign_bundle(host, &mut bundle, host.now()).await.unwrap();
    bundle
}

/// The exported bundle's reservation rows, for a test to change.
fn reservation_records(bundle: &Bundle) -> Vec<Value> {
    bundle.sections[SECTION_RESERVATIONS].clone()
}

fn with_reservations(bundle: &Bundle, records: Vec<Value>) -> BTreeMap<String, Vec<Value>> {
    let mut sections = bundle.sections.clone();
    sections.insert(SECTION_RESERVATIONS.to_string(), records);
    sections
}

async fn two_bookings(host: &TestHost) -> Bundle {
    let slot_id = slot(host, 2).await;
    reservation::reserve_at(host, reserve_input("agr-a", "c1", &slot_id), NOW).await.unwrap();
    reservation::reserve_at(host, reserve_input("agr-b", "c1", &slot_id), NOW).await.unwrap();
    reservation::confirm_at(host, key("agr-b", "c1"), NOW).await.unwrap();
    backup::export(host, NoParams {}).await.unwrap()
}

async fn refusal_of(host: &TestHost, sections: BTreeMap<String, Vec<Value>>) -> String {
    let bundle = bundle_of(host, sections).await;
    let refused = backup::import(host, ImportInput { bundle }).await.unwrap_err();
    format!("{} {}", refused.error.as_ref().unwrap().message, code_of(&refused))
}

#[tokio::test]
async fn export_then_import_keeps_a_held_and_a_confirmed_reservation_and_its_seat() {
    let master = master();
    let origin = signed_host(&master);
    let bundle = two_bookings(&origin).await;
    let target = signed_host(&master);
    let out = backup::import(&target, ImportInput { bundle }).await.unwrap();
    assert_eq!(out.imported["availability"], 1);
    assert_eq!(out.imported["reservations"], row_ids(&origin, "").await.len());
    assert_eq!(row_ids(&target, "seat:").await, row_ids(&origin, "seat:").await);
    // The held seat is still held, and the confirmed one still confirmed.
    let repeat_a =
        reservation::reserve_at(&target, reserve_input("agr-a", "c1", "ignored"), NOW).await;
    assert_eq!(repeat_a.unwrap().state, ReservationState::Held);
    let confirm_b = reservation::confirm_at(&target, key("agr-b", "c1"), NOW).await.unwrap();
    assert_eq!(confirm_b.state, ReservationState::Confirmed);
}

#[tokio::test]
async fn importing_the_same_bundle_twice_is_a_repeat() {
    let master = master();
    let origin = signed_host(&master);
    let bundle = two_bookings(&origin).await;
    let target = signed_host(&master);
    backup::import(&target, ImportInput { bundle: bundle.clone() }).await.unwrap();
    backup::import(&target, ImportInput { bundle }).await.unwrap();
    assert_eq!(row_ids(&target, "seat:").await, row_ids(&origin, "seat:").await);
}

#[tokio::test]
async fn a_seat_row_without_its_reservation_is_refused() {
    let master = master();
    let host = signed_host(&master);
    let bundle = two_bookings(&host).await;
    let records: Vec<Value> = reservation_records(&bundle)
        .into_iter()
        .filter(|r| !r["id"].as_str().unwrap().starts_with("rsv:"))
        .collect();
    let said = refusal_of(&host, with_reservations(&bundle, records)).await;
    assert!(said.contains("names no reservation"), "{said}");
}

#[tokio::test]
async fn a_reservation_without_its_seat_row_is_refused() {
    let master = master();
    let host = signed_host(&master);
    let bundle = two_bookings(&host).await;
    let records: Vec<Value> = reservation_records(&bundle)
        .into_iter()
        .filter(|r| !r["id"].as_str().unwrap().starts_with("seat:"))
        .collect();
    let said = refusal_of(&host, with_reservations(&bundle, records)).await;
    assert!(said.contains("no seat row"), "{said}");
}

#[tokio::test]
async fn a_confirmation_that_names_no_reservation_is_refused() {
    let master = master();
    let host = signed_host(&master);
    let bundle = two_bookings(&host).await;
    let mut records = reservation_records(&bundle);
    records.push(json!({
        "id": "decided:rsv_nowhere",
        "payload": { "kind": "decided", "state": "confirmed", "at_secs": 1 }
    }));
    let said = refusal_of(&host, with_reservations(&bundle, records)).await;
    assert!(said.contains("decided:rsv_nowhere"), "{said}");
}

#[tokio::test]
async fn a_release_that_names_no_reservation_is_refused() {
    let master = master();
    let host = signed_host(&master);
    let bundle = two_bookings(&host).await;
    let mut records = reservation_records(&bundle);
    records.push(json!({
        "id": "free:rsv_nowhere",
        "payload": { "kind": "free", "why": "released", "at_secs": 1 }
    }));
    let said = refusal_of(&host, with_reservations(&bundle, records)).await;
    assert!(said.contains("free:rsv_nowhere"), "{said}");
}

#[tokio::test]
async fn a_reservation_with_an_id_that_is_not_its_digest_is_refused() {
    let master = master();
    let host = signed_host(&master);
    let bundle = two_bookings(&host).await;
    let mut records = reservation_records(&bundle);
    for record in &mut records {
        let payload = &mut record["payload"];
        if payload["kind"] == "rsv" {
            payload["generation"] = json!(7);
            break;
        }
    }
    let said = refusal_of(&host, with_reservations(&bundle, records)).await;
    assert!(said.contains("not the id of its key"), "{said}");
}

#[tokio::test]
async fn a_pointer_that_names_no_reservation_is_refused() {
    let master = master();
    let host = signed_host(&master);
    let bundle = two_bookings(&host).await;
    let mut records = reservation_records(&bundle);
    records.push(json!({
        "id": "key:agr-z:c1",
        "payload": { "kind": "key", "generation": 1 }
    }));
    let said = refusal_of(&host, with_reservations(&bundle, records)).await;
    assert!(said.contains("pointer 'key:agr-z:c1'"), "{said}");
}

#[tokio::test]
async fn a_row_with_an_unknown_kind_is_refused() {
    let master = master();
    let host = signed_host(&master);
    let bundle = two_bookings(&host).await;
    let mut records = reservation_records(&bundle);
    records.push(json!({ "id": "rsv:x", "payload": { "kind": "mystery" } }));
    let said = refusal_of(&host, with_reservations(&bundle, records)).await;
    assert!(said.contains("does not parse"), "{said}");
}

#[tokio::test]
async fn a_row_listed_twice_is_refused() {
    let master = master();
    let host = signed_host(&master);
    let bundle = two_bookings(&host).await;
    let mut records = reservation_records(&bundle);
    records.push(records[0].clone());
    let said = refusal_of(&host, with_reservations(&bundle, records)).await;
    assert!(said.contains("listed twice"), "{said}");
}

#[tokio::test]
async fn a_slot_row_with_the_wrong_id_is_refused() {
    let master = master();
    let host = signed_host(&master);
    let bundle = two_bookings(&host).await;
    let mut sections = bundle.sections.clone();
    sections.get_mut(SECTION_AVAILABILITY).unwrap()[0]["payload"]["capacity"] = json!(3);
    sections.get_mut(SECTION_AVAILABILITY).unwrap()[0]["payload"]["start_secs"] = json!(5);
    let said = refusal_of(&host, sections).await;
    assert!(said.contains("wrong slot id"), "{said}");
}

#[tokio::test]
async fn an_unknown_section_is_refused() {
    let master = master();
    let host = signed_host(&master);
    let said = refusal_of(&host, BTreeMap::from([("holds".to_string(), vec![])])).await;
    assert!(said.contains("unknown section"), "{said}");
}

#[tokio::test]
async fn a_reservation_that_exists_with_other_content_is_refused() {
    let master = master();
    let origin = signed_host(&master);
    let bundle = two_bookings(&origin).await;
    // The target holds the same reservation key, taken for another seat.
    let target = signed_host(&master);
    let slot_id = slot(&target, 2).await;
    reservation::reserve_at(&target, reserve_input("agr-x", "c1", &slot_id), NOW).await.unwrap();
    let said = refusal_of_bundle(&target, bundle).await;
    assert!(said.contains("reservation-differs"), "{said}");
}

async fn refusal_of_bundle(host: &TestHost, bundle: Bundle) -> String {
    let refused = backup::import(host, ImportInput { bundle }).await.unwrap_err();
    format!("{} {}", refused.error.as_ref().unwrap().message, code_of(&refused))
}

#[tokio::test]
async fn nothing_is_written_when_a_row_is_refused() {
    let master = master();
    let host = signed_host(&master);
    let bundle = two_bookings(&host).await;
    let target = signed_host(&master);
    let mut records = reservation_records(&bundle);
    records.push(json!({
        "id": "free:rsv_nowhere",
        "payload": { "kind": "free", "why": "released", "at_secs": 1 }
    }));
    let bad = bundle_of(&host, with_reservations(&bundle, records)).await;
    assert!(backup::import(&target, ImportInput { bundle: bad }).await.is_err());
    assert!(row_ids(&target, "").await.is_empty());
}

#[tokio::test]
async fn a_decision_that_exists_with_other_content_is_refused() {
    let master = master();
    let host = signed_host(&master);
    let bundle = two_bookings(&host).await;
    let mut records = reservation_records(&bundle);
    for record in &mut records {
        if record["id"].as_str().unwrap().starts_with("decided:") {
            record["payload"]["state"] = json!("lapsed");
        }
    }
    let said = refusal_of(&host, with_reservations(&bundle, records)).await;
    assert!(said.contains("reservation-differs"), "{said}");
}

#[tokio::test]
async fn a_stopped_import_never_shows_a_held_seat_as_free() {
    let master = master();
    let origin = signed_host(&master);
    let slot_id = slot(&origin, 60).await;
    for n in 0..60 {
        let agreement = format!("agr-{n}");
        reservation::reserve_at(&origin, reserve_input(&agreement, "c1", &slot_id), NOW)
            .await
            .unwrap();
    }
    let bundle = backup::export(&origin, NoParams {}).await.unwrap();

    // The second chunk of 100 rows fails: all 60 seats and 40 reservations are
    // written.
    let target = signed_host(&master);
    target.crash_on_write_skip("reservations", "rsv:", 40);
    let stopped = backup::import(&target, ImportInput { bundle: bundle.clone() }).await;
    assert!(stopped.is_err());
    assert!(target.was_crashed_write_hit());
    target.restart();

    let full =
        reservation::reserve_at(&target, reserve_input("agr-new", "c1", &slot_id), NOW).await;
    assert_eq!(code_of(&full.unwrap_err()), "slot-taken", "every seat is still held");

    // The same bundle imports again and repairs the rest.
    backup::import(&target, ImportInput { bundle }).await.unwrap();
    assert_eq!(row_ids(&target, "rsv:").await.len(), 60);
}

#[tokio::test]
async fn a_lapsed_reservation_stays_lost_and_its_taker_keeps_the_seat_after_a_restore() {
    let master = master();
    let origin = signed_host(&master);
    let slot_id = slot(&origin, 1).await;
    let due = NOW + RESERVATION_TTL_SECS;
    reservation::reserve_at(&origin, reserve_input("agr-a", "c1", &slot_id), NOW).await.unwrap();
    reservation::reserve_at(&origin, reserve_input("agr-b", "c1", &slot_id), due + 1)
        .await
        .unwrap();
    let bundle = backup::export(&origin, NoParams {}).await.unwrap();
    let target = signed_host(&master);
    backup::import(&target, ImportInput { bundle: bundle.clone() }).await.unwrap();
    assert_eq!(row_ids(&target, "decided:").await, row_ids(&origin, "decided:").await);
    assert_eq!(row_ids(&target, "free:").await, row_ids(&origin, "free:").await);
    let lost = reservation::confirm_at(&target, key("agr-a", "c1"), due + 2).await;
    assert_eq!(code_of(&lost.unwrap_err()), "reservation-lost");
    let taker = reservation::confirm_at(&target, key("agr-b", "c1"), due + 2).await.unwrap();
    assert_eq!(taker.state, ReservationState::Confirmed);
    // A repeat import of the same bundle changes nothing.
    backup::import(&target, ImportInput { bundle }).await.unwrap();
}
