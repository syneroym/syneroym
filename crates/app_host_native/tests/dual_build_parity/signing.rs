use super::helpers::*;

async fn assert_signing_identity<D: Driver>(name: &str, driver: &D) -> Value {
    let result = driver.run(r#"{"op":"signing-identity"}"#).await.unwrap();
    let v: Value = serde_json::from_str(&result).unwrap();
    let ok = &v["ok"];
    assert!(ok["signing_did"].is_string(), "{name}: signing_did must be string, got {v}");
    assert!(ok["pubkey_hex"].is_string(), "{name}: pubkey_hex must be string, got {v}");
    ok.clone()
}

#[tokio::test]
async fn signing_identity_returns_valid_did_and_pubkey_on_both_builds() {
    let h = harness().await;
    let wasm_id = assert_signing_identity("wasm", &h.wasm).await;
    let native_id = assert_signing_identity("native", &h.native).await;
    assert_eq!(wasm_id, native_id, "signing_identity must be identical across builds");
}

async fn assert_sign_as_service_and_verify<D: Driver>(name: &str, driver: &D) -> String {
    let draft_req = json!({
        "op": "sign-as-service",
        "draft": {
            "version": 1,
            "record_type": "listing",
            "subject": "sub1",
            "payload": r#"{"price":100}"#,
            "expires_at_secs": 2_000_000_000,
        }
    });
    let result = driver.run(&draft_req.to_string()).await.unwrap();
    let v: Value = serde_json::from_str(&result).unwrap();
    let signed_json =
        v["ok"].as_str().unwrap_or_else(|| panic!("{name}: expected ok string, got {v}"));

    let verify_req = json!({
        "op": "verify-record",
        "signed_json": signed_json
    });
    let verify_res = driver.run(&verify_req.to_string()).await.unwrap();
    let verify_v: Value = serde_json::from_str(&verify_res).unwrap();
    assert_eq!(
        verify_v["ok"]["valid"], true,
        "{name}: verified record must be valid, got {verify_v}"
    );
    assert_eq!(verify_v["ok"]["subject"], "sub1", "{name}: subject mismatch");
    signed_json.to_string()
}

#[tokio::test]
async fn sign_as_service_and_verify_succeeds_and_is_byte_identical_on_both_builds() {
    let h = harness().await;
    let wasm_signed = assert_sign_as_service_and_verify("wasm", &h.wasm).await;
    let native_signed = assert_sign_as_service_and_verify("native", &h.native).await;
    assert_eq!(wasm_signed, native_signed, "signed envelopes must be byte-identical across builds");
}

async fn assert_sign_with_invalid_float_payload_fails<D: Driver>(name: &str, driver: &D) {
    let draft_req = json!({
        "op": "sign-as-service",
        "draft": {
            "version": 1,
            "record_type": "listing",
            "subject": "sub1",
            "payload": r#"{"price":100.5}"#,
        }
    });
    let result = driver.run(&draft_req.to_string()).await.unwrap();
    let v: Value = serde_json::from_str(&result).unwrap();
    assert!(v["err"].is_string(), "{name}: float in draft payload must be refused, got {v}");
}

#[tokio::test]
async fn sign_with_float_payload_is_refused_on_both_builds() {
    let h = harness().await;
    assert_sign_with_invalid_float_payload_fails("wasm", &h.wasm).await;
    assert_sign_with_invalid_float_payload_fails("native", &h.native).await;
}

// Note: Parity suite covers 11 primary dual-build execution scenarios across
// valid signing, key mismatch, scope mismatch, caller DID mismatch, and expiry.
// Additional refusal-shape permutations (version 0, array payload, 65 KiB
// payload, revoked keys, tampered payloads) are covered via unit tests in
// `syneroym-signed-record`.
#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn delegated_signing_scenarios_and_verify_failures_on_both_builds() {
    let h = harness().await;
    let wasm_id = assert_signing_identity("wasm", &h.wasm).await;
    let signing_did = wasm_id["signing_did"].as_str().unwrap();
    let temp_pubkey = syneroym_identity::substrate::resolve_did_key(signing_did).unwrap();

    // 1. Valid delegation certificate
    let cert = DelegationCertificate::issue(
        &h.master_identity,
        temp_pubkey,
        86400 * 365,
        syneroym_identity::delegation::SCOPE_RECORD_SIGNING.to_string(),
    )
    .unwrap();
    let cert_json = serde_json::to_string(&cert).unwrap();

    let draft = json!({
        "version": 1,
        "record_type": "listing",
        "subject": "sub_del",
        "payload": r#"{"item":"book"}"#,
        "expires_at_secs": 2_000_000_000,
    });

    let del_req = json!({
        "op": "sign-as-delegated",
        "draft": draft,
        "delegation_json": cert_json,
    });

    let master_did = syneroym_identity::substrate::derive_did_key(&h.master_identity.public_key());
    let master_caller = caller_with_did(&master_did);

    let wasm_res =
        h.wasm.run_with_caller(&del_req.to_string(), master_caller.clone()).await.unwrap();
    let wasm_v: Value = serde_json::from_str(&wasm_res).unwrap();
    let wasm_signed = wasm_v["ok"].as_str().expect("wasm delegated signed string");

    let native_res =
        h.native.run_with_caller(&del_req.to_string(), master_caller.clone()).await.unwrap();
    let native_v: Value = serde_json::from_str(&native_res).unwrap();
    let native_signed = native_v["ok"].as_str().expect("native delegated signed string");

    assert_eq!(wasm_signed, native_signed, "delegated signed envelopes must be byte-identical");

    // Verify on both builds
    let verify_req = json!({
        "op": "verify-record",
        "signed_json": wasm_signed,
        "now_secs": 1_800_000_000,
    });
    let wasm_ver = h.wasm.run(&verify_req.to_string()).await.unwrap();
    let native_ver = h.native.run(&verify_req.to_string()).await.unwrap();
    assert_eq!(wasm_ver, native_ver);
    let ver_val: Value = serde_json::from_str(&wasm_ver).unwrap();
    assert_eq!(ver_val["ok"]["valid"], true);
    assert_eq!(
        ver_val["ok"]["asserter_did"],
        syneroym_identity::substrate::derive_did_key(&h.master_identity.public_key())
    );

    // 2. Refuse cert over wrong key
    let wrong_identity = Identity::generate().unwrap();
    let wrong_cert = DelegationCertificate::issue(
        &h.master_identity,
        wrong_identity.public_key(),
        3600,
        syneroym_identity::delegation::SCOPE_RECORD_SIGNING.to_string(),
    )
    .unwrap();
    let wrong_del_req = json!({
        "op": "sign-as-delegated",
        "draft": draft,
        "delegation_json": wrong_cert.to_json().unwrap(),
    });
    let wasm_err =
        h.wasm.run_with_caller(&wrong_del_req.to_string(), master_caller.clone()).await.unwrap();
    let native_err =
        h.native.run_with_caller(&wrong_del_req.to_string(), master_caller.clone()).await.unwrap();
    assert_eq!(wasm_err, native_err);
    let err_val: Value = serde_json::from_str(&wasm_err).unwrap();
    assert!(err_val["err"].is_string());

    // 3. Refuse cert with wrong scope
    let scope_cert = DelegationCertificate::issue(
        &h.master_identity,
        temp_pubkey,
        3600,
        syneroym_identity::delegation::SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    let scope_del_req = json!({
        "op": "sign-as-delegated",
        "draft": draft,
        "delegation_json": scope_cert.to_json().unwrap(),
    });
    let wasm_scope_err =
        h.wasm.run_with_caller(&scope_del_req.to_string(), master_caller.clone()).await.unwrap();
    let native_scope_err =
        h.native.run_with_caller(&scope_del_req.to_string(), master_caller).await.unwrap();
    assert_eq!(wasm_scope_err, native_scope_err);
    assert!(serde_json::from_str::<Value>(&wasm_scope_err).unwrap()["err"].is_string());

    // 4. Refuse cert when caller DID does not match certificate master DID
    let wrong_did = syneroym_identity::substrate::derive_did_key(&wrong_identity.public_key());
    let wrong_caller = caller_with_did(&wrong_did);
    let valid_del_req = json!({
        "op": "sign-as-delegated",
        "draft": draft,
        "delegation_json": cert.to_json().unwrap(),
    });
    let wasm_caller_err =
        h.wasm.run_with_caller(&valid_del_req.to_string(), wrong_caller.clone()).await.unwrap();
    let native_caller_err =
        h.native.run_with_caller(&valid_del_req.to_string(), wrong_caller).await.unwrap();
    assert_eq!(wasm_caller_err, native_caller_err);
    assert!(serde_json::from_str::<Value>(&wasm_caller_err).unwrap()["err"].is_string());

    // 5. Verify failure: past record expiry
    let expired_verify_req = json!({
        "op": "verify-record",
        "signed_json": wasm_signed,
        "now_secs": 2_000_000_001,
    });
    let wasm_exp_ver = h.wasm.run(&expired_verify_req.to_string()).await.unwrap();
    let native_exp_ver = h.native.run(&expired_verify_req.to_string()).await.unwrap();
    assert_eq!(wasm_exp_ver, native_exp_ver);
    assert!(serde_json::from_str::<Value>(&wasm_exp_ver).unwrap()["err"].is_string());
}

/// `syneroym:invocation`: the same drive answers `internal` on a local
/// call and reads the caller's identity on a wire call -- identically on
/// both builds. The local drive here hands a verified delegated caller to
/// a purely local path, which is exactly the case an auth-reading native
/// mapping would have gotten wrong.
#[tokio::test]
async fn caller_origin_is_identical_on_both_builds() {
    let h = harness().await;
    let req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "run".to_string(),
        params: json!([r#"{"op":"caller-origin"}"#]),
        id: None,
        idempotency_key: None,
    };
    let delegated = CallerContext {
        caller_did: "did:key:zOwnerDelegated".to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: "did:key:zOwnerDelegated".to_string(),
            ..Default::default()
        },
        auth: AuthLevel::Delegated,
        proof: None,
    };

    // The guest's `run` returns a JSON document *as a string*; unwrap that
    // one level so the two builds compare as structured values.
    let unwrap_str = |v: Value| -> Value {
        match v {
            Value::String(s) => serde_json::from_str(&s).unwrap(),
            other => other,
        }
    };

    // Local call: both builds report `internal`, whatever the caller says.
    let wasm_local = unwrap_str(
        h.wasm_engine
            .execute_wasm_json(SERVICE_ID, FIXTURE_INTERFACE, &req, Some(delegated.clone()))
            .await
            .unwrap(),
    );
    let native_local =
        h.native.run_with_caller(r#"{"op":"caller-origin"}"#, delegated.clone()).await;
    let native_local: Value = serde_json::from_str(&native_local.unwrap()).unwrap();
    assert_eq!(wasm_local, json!({ "ok": { "arm": "internal" } }));
    assert_eq!(native_local, json!({ "ok": { "arm": "internal" } }));

    // Wire call: both builds read the caller. Native goes through a fixture
    // built with `host_for_wire`, the parity harness's only such caller.
    let wire_fixture = {
        let f = h.native_factory.clone();
        let f_http = h.native_factory.clone();
        Arc::new(NativeFixture::new(
            SERVICE_ID.to_string(),
            move |c| f.host_for_wire(c),
            move |c| f_http.host_for_wire(c),
        ))
    };
    let wasm_wire = unwrap_str(
        h.wasm_engine
            .execute_wasm_json_from_wire(
                SERVICE_ID,
                FIXTURE_INTERFACE,
                &req,
                Some(delegated.clone()),
            )
            .await
            .unwrap(),
    );
    let native_wire = {
        use syneroym_rpc::NativeService;
        let inv = NativeInvocation {
            interface: "test-driver".to_string(),
            method: "run".to_string(),
            params: json!([r#"{"op":"caller-origin"}"#]),
            caller: delegated.clone(),
        };
        let Value::String(s) = wire_fixture.dispatch(inv).await.unwrap().payload else {
            panic!("expected string payload")
        };
        serde_json::from_str::<Value>(&s).unwrap()
    };
    let expected = json!({ "ok": { "arm": "verified", "did": "did:key:zOwnerDelegated" } });
    assert_eq!(wasm_wire, expected);
    assert_eq!(native_wire, expected);

    // Anonymous wire call: no caller on the connection.
    let wasm_anon = unwrap_str(
        h.wasm_engine
            .execute_wasm_json_from_wire(SERVICE_ID, FIXTURE_INTERFACE, &req, None)
            .await
            .unwrap(),
    );
    assert_eq!(wasm_anon, json!({ "ok": { "arm": "anonymous" } }));
}
