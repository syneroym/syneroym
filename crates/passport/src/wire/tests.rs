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
