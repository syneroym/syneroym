use syneroym_identity::substrate;

use super::{super::*, helpers::*};

/// A locked vault refuses the whole call, before a generation is
/// claimed and before any key is minted -- not through a
/// `kek_is_loaded` pre-check. The locked fixture (encryption on, no
/// KEK) is the only shape that proves anything about locking.
#[tokio::test]
async fn adopt_on_a_locked_vault_refuses_before_it_claims_a_generation() {
    let s = Fixture { locked_vault: true, ..Fixture::default() }.build();
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0).unwrap();

    let err = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "adopt",
        serde_json::json!(["inst-1"]),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("inject-kek"), "{err}");

    let row = s.store.get("inst-1").unwrap().unwrap();
    assert_eq!(row.generation, 0, "a refused adopt must not claim a generation");
    assert_eq!(row.app_master_did, "", "a refused adopt must not record a DID");
}

/// `adopt` is the only mint point, stated as a decision rather than
/// merely true of the paths tested so far. Another test shows the
/// field absent right after `submit`; this covers the two paths most
/// likely to grow a mint by accident later, since both re-run the
/// same apply pipeline `adopt` does over the identical plan --
/// `force-reconcile` and one resident-loop pass.
#[tokio::test]
async fn app_master_did_stays_empty_through_force_reconcile_and_a_loop_pass_without_adopt() {
    let s = service();
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0).unwrap();

    dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "force-reconcile",
        serde_json::json!(["inst-1"]),
    )
    .await
    .unwrap();
    assert_eq!(s.store.get("inst-1").unwrap().unwrap().app_master_did, "");

    s.run_pass().await;
    assert_eq!(s.store.get("inst-1").unwrap().unwrap().app_master_did, "");
}

/// The DID stays stable across two `adopt`s -- resolving, not minting,
/// on the second call. Over a services-less plan, the generation
/// itself stays `1` on both calls too: `claim_next_generation` reads
/// the held maximum only from the substrates the plan places services
/// on, and an empty plan has none to remember a prior claim, so this
/// in-process shape cannot demonstrate the generation actually
/// advancing -- that needs a real substrate, which the e2e proves
/// alongside DID stability. Named for what it actually asserts, not
/// for an increment this test cannot produce.
#[tokio::test]
async fn a_second_adopt_reports_the_same_app_master_did() {
    let s = service();
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0).unwrap();

    let first = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "adopt",
        serde_json::json!(["inst-1"]),
    )
    .await
    .unwrap();
    let second = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "adopt",
        serde_json::json!(["inst-1"]),
    )
    .await
    .unwrap();

    assert_eq!(
        adopt_field(&first, "app_master_did").and_then(Value::as_str),
        adopt_field(&second, "app_master_did").and_then(Value::as_str)
    );
    assert_eq!(adopt_field(&first, "generation").and_then(Value::as_u64), Some(1));
    assert_eq!(adopt_field(&second, "generation").and_then(Value::as_u64), Some(1));
}

/// `status` reports the same DID `adopt` minted.
#[tokio::test]
async fn status_reports_the_app_master_did_of_an_adopted_instance() {
    let s = service();
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0).unwrap();
    let adopted = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "adopt",
        serde_json::json!(["inst-1"]),
    )
    .await
    .unwrap();
    let minted_did = adopt_field(&adopted, "app_master_did").and_then(Value::as_str).unwrap();

    let status = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "status",
        serde_json::json!(["inst-1"]),
    )
    .await
    .unwrap();
    assert_eq!(status.payload.get("app_master_did").and_then(Value::as_str), Some(minted_did));
}

/// Absent, not an empty string -- an instance that has never been
/// adopted must not read as though it holds a DID of `""`.
#[tokio::test]
async fn status_reports_no_app_master_for_an_instance_that_was_never_adopted() {
    let s = service();
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0).unwrap();

    let status = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "status",
        serde_json::json!(["inst-1"]),
    )
    .await
    .unwrap();
    assert!(
        status.payload.get("app_master_did").is_none_or(Value::is_null),
        "absent means null once serialized, not an empty string: {:?}",
        status.payload.get("app_master_did")
    );
}

/// `status` reports the currently-published Tier-1 record's expiry,
/// derived from the last successful refresh this supervisor
/// stamped -- not from a fresh registry lookup.
#[tokio::test]
async fn status_reports_the_tier_one_record_expiry() {
    let s = service();
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0).unwrap();
    s.store.record_adopt("inst-1", 0, "did:key:zAppMaster").unwrap();
    s.store.record_tier1_refresh("did:key:zAppMaster", 1_000).unwrap();

    let status = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "status",
        serde_json::json!(["inst-1"]),
    )
    .await
    .unwrap();

    assert_eq!(
        status.payload.get("app_record_expires_at").and_then(Value::as_u64),
        Some(1_000u64 + DEFAULT_ENDPOINT_NOT_AFTER_SECS)
    );
}

/// The absent case: no successful publish is on record (never
/// adopted, no registry configured, or a vault locked since before
/// the first refresh), so there is no expiry to report.
#[tokio::test]
async fn status_reports_no_tier_one_expiry_when_never_published() {
    let s = service();
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0).unwrap();

    let status = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "status",
        serde_json::json!(["inst-1"]),
    )
    .await
    .unwrap();

    assert!(status.payload.get("app_record_expires_at").is_none_or(Value::is_null));
}

/// `pause`'s response reports the date its own write-phase skip
/// (`a_paused_instance_gets_no_tier1_refresh`, above) will let the
/// currently-published record decay to. Named for the response field
/// this actually asserts, not the `tracing::warn!` alongside it --
/// that log line is real (`handle_pause`) but outside what a
/// dispatch-level RPC test can capture without the same
/// `run_capturing_logs`-shaped machinery `keys.rs` uses.
#[tokio::test]
async fn pause_reports_the_records_expiry_date() {
    let s = service();
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0).unwrap();
    s.store.record_adopt("inst-1", 0, "did:key:zAppMaster").unwrap();
    s.store.record_tier1_refresh("did:key:zAppMaster", 1_000).unwrap();

    let res = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "pause",
        serde_json::json!(["inst-1"]),
    )
    .await
    .unwrap();

    assert_eq!(
        res.payload.get("app_record_expires_at").and_then(Value::as_u64),
        Some(1_000u64 + DEFAULT_ENDPOINT_NOT_AFTER_SECS)
    );
    assert!(s.store.get("inst-1").unwrap().unwrap().paused, "pause must still take effect");
}

/// Nothing published yet means nothing to warn about -- `pause` must
/// not invent an expiry for a record that was never signed.
#[tokio::test]
async fn pause_has_nothing_to_warn_about_when_never_published() {
    let s = service();
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0).unwrap();

    let res = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "pause",
        serde_json::json!(["inst-1"]),
    )
    .await
    .unwrap();

    assert!(res.payload.get("app_record_expires_at").is_none());
}

/// The handover-order repair inside one vault -- mint by adopting,
/// import a different key under the same name (simulating an
/// operator-carried backup replacing this vault's own key), adopt
/// again, and the row follows the vault rather than keeping the
/// replaced DID.
#[tokio::test]
async fn adopt_after_an_import_records_the_imported_did_not_the_one_it_replaced() {
    let s = service();
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0).unwrap();
    let first = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "adopt",
        serde_json::json!(["inst-1"]),
    )
    .await
    .unwrap();
    let original_did =
        adopt_field(&first, "app_master_did").and_then(Value::as_str).unwrap().to_string();

    let replacement = Identity::generate().unwrap();
    s.vault.import("app-inst-1", &replacement.to_bytes()).await.unwrap();
    let replacement_did = substrate::derive_did_key(&replacement.public_key());
    assert_ne!(original_did, replacement_did);

    let second = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "adopt",
        serde_json::json!(["inst-1"]),
    )
    .await
    .unwrap();
    assert_eq!(
        adopt_field(&second, "app_master_did").and_then(Value::as_str),
        Some(replacement_did.as_str())
    );
    assert_eq!(s.store.get("inst-1").unwrap().unwrap().app_master_did, replacement_did);
}

/// An instance whose row predates the app-master column -- generation
/// already claimed, `app_master_did` empty -- gains one at its *next*
/// `adopt`, never anywhere else. Simulated by writing that older state
/// directly rather than going through `adopt` to reach it.
#[tokio::test]
async fn an_instance_row_with_no_app_master_gains_one_on_its_next_adopt() {
    let s = service();
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0).unwrap();
    s.store.set_generation("inst-1", 2).unwrap();
    assert_eq!(s.store.get("inst-1").unwrap().unwrap().app_master_did, "");

    let res = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "adopt",
        serde_json::json!(["inst-1"]),
    )
    .await
    .unwrap();
    // The generation bump itself is the named cost -- not asserted at
    // a specific number here, since a services-less plan has no
    // substrate to remember `2` was already claimed and
    // `claim_next_generation` always computes fresh from what the plan
    // places, which is nothing.
    let did = adopt_field(&res, "app_master_did").and_then(Value::as_str).unwrap();
    assert!(did.starts_with("did:key:"));
    assert_eq!(s.store.get("inst-1").unwrap().unwrap().app_master_did, did);
}

/// An earlier failure, asserted directly -- the returned name must be
/// one `export-master` actually accepts, not the bare logical name.
#[tokio::test]
async fn adopt_returns_the_vault_name_export_master_accepts() {
    let s = service();
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0).unwrap();
    let res = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "adopt",
        serde_json::json!(["inst-1"]),
    )
    .await
    .unwrap();
    let vault_name = adopt_field(&res, "vault_name").and_then(Value::as_str).unwrap().to_string();

    let export = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "export-master",
        serde_json::json!([vault_name]),
    )
    .await
    .unwrap();
    assert!(export.payload.as_str().unwrap().contains("app-inst-1"));
}

/// `status` must stay readable through a genuinely locked vault -- the
/// column exists precisely so the app's identity is visible while the
/// vault is shut, and this is the only test in this file that reaches
/// that state over one held service rather than a fresh, empty
/// rebuild. Locked *in place*: unlocked at construction so `adopt` can
/// mint, then the KEK is cleared afterward.
#[tokio::test]
async fn status_reports_the_app_master_did_while_the_vault_is_locked() {
    let (s, key_store) =
        Fixture { locked_vault: true, inject_kek_anyway: true, ..Fixture::default() }
            .build_with_key_store();
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0).unwrap();
    let adopted = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "adopt",
        serde_json::json!(["inst-1"]),
    )
    .await
    .unwrap();
    let minted_did =
        adopt_field(&adopted, "app_master_did").and_then(Value::as_str).unwrap().to_string();

    key_store.clear_kek();
    assert!(
        s.vault.get("app-inst-1").await.is_err(),
        "the vault must genuinely be locked for this test to prove anything"
    );

    let status = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "status",
        serde_json::json!(["inst-1"]),
    )
    .await
    .unwrap();
    assert_eq!(
        status.payload.get("app_master_did").and_then(Value::as_str),
        Some(minted_did.as_str())
    );
}

/// A *second* supervisor, which has never adopted this instance,
/// imports the app master another supervisor already exported, and
/// its first `adopt` reports the imported DID rather than minting a
/// fresh one. Two independent fixture-built services sharing one
/// test-owned backup directory -- the stand-in for the file an
/// operator carries between two real supervisors during a handover.
#[tokio::test]
async fn a_second_supervisor_that_imports_the_app_master_adopts_without_minting_a_new_one() {
    let backup_dir = tempfile::tempdir().unwrap();
    let supervisor_a =
        Fixture { backup_dir: Some(backup_dir.path().to_path_buf()), ..Fixture::default() }.build();
    let supervisor_b =
        Fixture { backup_dir: Some(backup_dir.path().to_path_buf()), ..Fixture::default() }.build();

    supervisor_a
        .store
        .submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0)
        .unwrap();
    let adopted_a = dispatch(
        &supervisor_a,
        admin_caller("did:key:zSupervisorNode"),
        "adopt",
        serde_json::json!(["inst-1"]),
    )
    .await
    .unwrap();
    let a_did =
        adopt_field(&adopted_a, "app_master_did").and_then(Value::as_str).unwrap().to_string();
    let vault_name =
        adopt_field(&adopted_a, "vault_name").and_then(Value::as_str).unwrap().to_string();

    dispatch(
        &supervisor_a,
        admin_caller("did:key:zSupervisorNode"),
        "export-master",
        serde_json::json!([vault_name.clone()]),
    )
    .await
    .unwrap();

    // Supervisor B has never adopted this instance -- it must still
    // hold its own desired-state row before `adopt` can act on it.
    supervisor_b
        .store
        .submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0)
        .unwrap();
    dispatch(
        &supervisor_b,
        admin_caller("did:key:zSupervisorNode"),
        "import-master",
        serde_json::json!([vault_name]),
    )
    .await
    .unwrap();

    let adopted_b = dispatch(
        &supervisor_b,
        admin_caller("did:key:zSupervisorNode"),
        "adopt",
        serde_json::json!(["inst-1"]),
    )
    .await
    .unwrap();
    assert_eq!(
        adopt_field(&adopted_b, "app_master_did").and_then(Value::as_str),
        Some(a_did.as_str()),
        "B's first adopt must resolve A's imported DID, not mint a second identity"
    );
    assert_eq!(supervisor_b.store.get("inst-1").unwrap().unwrap().app_master_did, a_did);
}

/// The mint-before-claim, record-after-claim asymmetry's failing
/// direction -- a claim that
/// fails after the mint already landed must not mint a *second* key on
/// the retry, since a vault key with no row is meant to be
/// recoverable. Needs a placed service on an unreachable substrate, so
/// the services-less shortcut every other test in this section uses
/// does not apply.
#[tokio::test]
async fn a_failed_claim_after_a_successful_mint_reuses_the_same_app_master() {
    let s = service();
    let plan_json = plan_json_one_service("inst-1", "backend", Some("edge-1"));
    let inventory_json =
        serde_json::json!({"edge-1": {"did": "did:key:zEdge1", "api_url": "http://127.0.0.1:1"}})
            .to_string();
    s.store.submit("inst-1", &plan_json, &inventory_json, "did:key:owner", 0).unwrap();

    let err = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "adopt",
        serde_json::json!(["inst-1"]),
    )
    .await
    .unwrap_err();
    // The claim failed against the unreachable substrate, not the
    // mint -- the row must show no generation was claimed, but the
    // vault must already hold a key, since the mint runs first.
    // `.expect` here, not `.map`: a plain `Option` comparison at the
    // bottom of this test would pass on `None == None` if the mint
    // ever stopped running before the claim -- the exact regression
    // this test exists to catch -- since both reads would then find
    // nothing rather than the same key.
    assert_eq!(s.store.get("inst-1").unwrap().unwrap().generation, 0, "{err}");
    let minted = s
        .vault
        .get("app-inst-1")
        .await
        .unwrap()
        .expect("the mint runs before the claim, so the vault must already hold a key");
    let minted_did = substrate::derive_did_key(&minted.public_key());

    // A real deployment would fix the substrate before retrying;
    // here the same unreachable alias still fails the claim, so the
    // only thing left to prove is that the vault key from the first
    // attempt was reused, not replaced.
    let _ = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "adopt",
        serde_json::json!(["inst-1"]),
    )
    .await
    .unwrap_err();
    let second_minted = s
        .vault
        .get("app-inst-1")
        .await
        .unwrap()
        .expect("the retried mint must also resolve a key, not find nothing");
    let second_did = substrate::derive_did_key(&second_minted.public_key());
    assert_eq!(minted_did, second_did, "a retried mint must resolve the same key, not a new one");
}

/// `adopt`'s un-retire and its app-master write now land in the same
/// `record_adopt` call, but nothing at the service level had exercised
/// them running back to back on a genuinely retired instance --
/// store-level coverage
/// (`store.rs`'s
/// `record_adopt_writes_generation_retired_and_app_master_did_together`)
/// proves the store method alone, not that `handle_adopt` actually
/// reaches it starting from `retired`.
#[tokio::test]
async fn adopt_on_a_retired_instance_un_retires_and_records_the_app_master_together() {
    let s = service();
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0).unwrap();
    s.store.retire("inst-1").unwrap();
    assert!(s.store.get("inst-1").unwrap().unwrap().retired);

    let res = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "adopt",
        serde_json::json!(["inst-1"]),
    )
    .await
    .unwrap();

    let row = s.store.get("inst-1").unwrap().unwrap();
    assert!(!row.retired, "adopt must un-retire the instance");
    let did = adopt_field(&res, "app_master_did").and_then(Value::as_str).unwrap();
    assert_eq!(row.app_master_did, did);
}
