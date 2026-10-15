use serde_json::{Map, json};

use super::*;
use crate::{
    SignerBinding,
    test_support::{Fixture, did},
};

fn request_json(f: &Fixture) -> Vec<u8> {
    let token = f.token(Map::new());
    let Proof::Challenge { challenge, signer_did, signature } =
        f.challenge_proof(&f.subject, &token, f.now)
    else {
        unreachable!()
    };
    serde_json::to_vec(&RegisterRequest {
        token,
        challenge: challenge.to_hex(),
        signer_did,
        signature,
    })
    .unwrap()
}

/// Spaces are valid JSON padding, so only the size check can refuse it.
#[test]
fn a_body_over_the_limit_is_refused_before_the_parse() {
    let f = Fixture::relay();
    let mut body = request_json(&f);
    body.resize(MAX_REQUEST_BYTES + 1, b' ');
    assert_eq!(
        RegisterRequest::from_slice(&body).unwrap_err(),
        RegisterError::RequestTooLarge { max: MAX_REQUEST_BYTES }
    );
    body.truncate(MAX_REQUEST_BYTES);
    RegisterRequest::from_slice(&body).unwrap();
}

#[test]
fn invalid_json_is_malformed() {
    assert!(matches!(RegisterRequest::from_slice(b"{"), Err(RegisterError::Malformed(_))));
}

#[test]
fn an_unknown_field_is_malformed() {
    let f = Fixture::relay();
    let mut value: serde_json::Value = serde_json::from_slice(&request_json(&f)).unwrap();
    value["extra"] = json!(1);
    let body = serde_json::to_vec(&value).unwrap();
    assert!(matches!(RegisterRequest::from_slice(&body), Err(RegisterError::Malformed(_))));
}

#[test]
fn a_bad_challenge_hex_is_malformed() {
    let f = Fixture::relay();
    let mut request = RegisterRequest::from_slice(&request_json(&f)).unwrap();
    for bad in ["abc", &"g".repeat(64), &"a".repeat(66)] {
        request.challenge = bad.to_string();
        assert!(matches!(request.clone().into_parts(), Err(RegisterError::Malformed(_))), "{bad}");
    }
}

#[test]
fn a_request_round_trip_registers() {
    let f = Fixture::relay();
    let request = RegisterRequest::from_slice(&request_json(&f)).unwrap();
    let (token, proof) = request.into_parts().unwrap();
    let checked = f.registrar.check(&token, &proof, &SignerBinding::SameKey, f.now).unwrap();
    let lease = f.registrar.grant(checked, f.now).unwrap();
    assert_eq!(lease.subject, did(&f.subject));

    let response =
        RegisterResponse { passport_id: lease.passport_id, lease_ends_at: lease.lease_ends_at };
    let back: RegisterResponse =
        serde_json::from_str(&serde_json::to_string(&response).unwrap()).unwrap();
    assert_eq!(back, response);
}

#[test]
fn an_error_body_carries_the_code() {
    let body = ErrorBody::from(&RegisterError::Denied);
    assert_eq!(body.code, "denied");
    assert_eq!(body.message, "the passport is denied");
}

#[test]
fn a_store_error_does_not_reach_the_error_body() {
    let body = ErrorBody::from(&RegisterError::Store("disk I/O error at /var/db".into()));
    assert_eq!(body.code, "internal");
    assert_eq!(body.message, "internal error");
}

/// A newer coordinator may add a field to a response. An older substrate must
/// still read it.
#[test]
fn a_response_with_an_unknown_field_still_parses() {
    let id = "a".repeat(64);
    let ok: RegisterResponse = serde_json::from_value(
        json!({ "passport_id": id, "lease_ends_at": 5, "renew_after_secs": 3 }),
    )
    .unwrap();
    assert_eq!(ok.lease_ends_at, 5);
    let challenge: ChallengeResponse =
        serde_json::from_value(json!({ "challenge": id, "expires_in_secs": 60, "x": 1 })).unwrap();
    assert_eq!(challenge.expires_in_secs, 60);
    let error: ErrorBody =
        serde_json::from_value(json!({ "code": "denied", "message": "m", "retry": false }))
            .unwrap();
    assert_eq!(error.code, "denied");
}

#[test]
fn lease_info_round_trips_and_accepts_extra_field() {
    let lease = Lease {
        passport_id: "a".repeat(64).parse().unwrap(),
        owner_did: "did:key:z6Mku".to_string(),
        subject: "did:key:z6Mks".to_string(),
        lease_ends_at: 1000,
        registered_at: 500,
        limits: Map::new(),
    };
    let info = LeaseInfo::from(&lease);
    let serialized = serde_json::to_string(&info).unwrap();
    let deserialized: LeaseInfo = serde_json::from_str(&serialized).unwrap();
    assert_eq!(deserialized, info);

    let with_extra = json!({
        "passport_id": info.passport_id.to_string(),
        "owner_did": info.owner_did,
        "subject": info.subject,
        "lease_ends_at": info.lease_ends_at,
        "registered_at": info.registered_at,
        "extra_field": "ignored",
    });
    let parsed: LeaseInfo = serde_json::from_value(with_extra).unwrap();
    assert_eq!(parsed, info);
}
