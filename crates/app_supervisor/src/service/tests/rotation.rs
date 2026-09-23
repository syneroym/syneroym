use std::collections::BTreeMap;

use syneroym_identity::substrate;

use super::{super::*, helpers::*};
use crate::tier1::RegistryTier1Writer;

/// A `0` here would make every signed topology document born expired.
#[test]
fn a_configured_zero_topology_document_not_after_secs_is_clamped() {
    let s = Fixture { topology_document_not_after_secs: Some(0), ..Fixture::default() }.build();
    assert_eq!(s.topology_document_not_after_secs, 3_600);
}

/// A `cache_ttl` at or above half of `not_after` breaks the property
/// that a served copy always outlives the caller's own cache TTL.
#[test]
fn a_cache_ttl_at_least_half_of_not_after_is_clamped() {
    let s = Fixture {
        topology_document_not_after_secs: Some(100),
        topology_document_cache_ttl_secs: Some(50),
        ..Fixture::default()
    }
    .build();
    assert_eq!(s.topology_document_cache_ttl_secs, 25);
}

/// `not_after_secs / 4` alone would clamp to `0` for any `not_after`
/// under 4 seconds, and a reader that takes `0` as its cache TTL gets
/// `Duration::ZERO`, which never registers a cache hit at all.
#[test]
fn the_cache_ttl_clamp_never_produces_zero() {
    let s = Fixture {
        topology_document_not_after_secs: Some(2),
        topology_document_cache_ttl_secs: Some(2),
        ..Fixture::default()
    }
    .build();
    assert_eq!(s.topology_document_cache_ttl_secs, 1);
}

#[tokio::test]
async fn master_anchor_refresh_is_skipped_when_not_yet_overdue() {
    let writer = Arc::new(RecordingAnchorWriter::default());
    let s = Fixture {
        anchor_writer: Some(writer.clone()),
        master_anchor_refresh_interval_secs: Some(43_200),
        ..Fixture::default()
    }
    .build();
    let master_did = seeded_member(&s, "backend").await;
    let plan =
        DeploymentPlan::from_json(&plan_json_with_master("backend", &master_did, "none")).unwrap();
    s.store.record_master_anchor_refresh(&master_did, NOW as i64 - 100).unwrap();

    s.refresh_due_master_anchors(&plan, NOW).await;

    assert!(writer.refreshed.lock().unwrap().is_empty());
    assert_eq!(
        s.store.last_master_anchor_refresh(&master_did).unwrap(),
        Some(NOW as i64 - 100),
        "a skipped refresh must not move the stamp"
    );
}

#[tokio::test]
async fn master_anchor_refresh_fires_once_the_interval_elapses() {
    let writer = Arc::new(RecordingAnchorWriter::default());
    let s = Fixture {
        anchor_writer: Some(writer.clone()),
        master_anchor_refresh_interval_secs: Some(43_200),
        ..Fixture::default()
    }
    .build();
    let master_did = seeded_member(&s, "backend").await;
    let plan =
        DeploymentPlan::from_json(&plan_json_with_master("backend", &master_did, "none")).unwrap();
    s.store.record_master_anchor_refresh(&master_did, NOW as i64 - 43_201).unwrap();

    s.refresh_due_master_anchors(&plan, NOW).await;

    assert_eq!(*writer.refreshed.lock().unwrap(), vec![master_did]);
}

/// `refresh_due_master_anchors` reads `svc.member_index` to resolve
/// which master to sign with (`keys::master_for_member`) -- a
/// regression to a hardcoded `0` would silently sign member 1's anchor
/// with member 0's key instead of failing loudly. Asserts the *key
/// the writer actually received*, not merely that a call happened.
#[tokio::test]
async fn master_anchor_refresh_republishes_each_members_own_anchor_and_stamps_its_own_row() {
    let writer = Arc::new(RecordingAnchorWriter::default());
    let s = Fixture { anchor_writer: Some(writer.clone()), ..Fixture::default() }.build();
    let master0 =
        s.vault.get_or_mint("member-inst-1#backend-0", keys::MasterKind::Member).await.unwrap();
    let master0_did = substrate::derive_did_key(&master0.public_key());
    let master1 =
        s.vault.get_or_mint("member-inst-1#backend-1", keys::MasterKind::Member).await.unwrap();
    let master1_did = substrate::derive_did_key(&master1.public_key());
    assert_ne!(master0_did, master1_did);

    let plan_json = serde_json::json!({
        "app_instance_id": "inst-1",
        "blueprint_id": "syneroym:test",
        "version": "1.0.0",
        "services": [
            {
                "service_id": master0_did,
                "logical_ref": "inst-1/backend",
                "substrate": "edge-1",
                "service_type": "tcp", "source": "127.0.0.1:9000",
                "rotation_policy": "none",
                "resolved_dependencies": {},
                "topology_mode": "redundant",
                "member_index": 0
            },
            {
                "service_id": master1_did,
                "logical_ref": "inst-1/backend",
                "substrate": "edge-1",
                "service_type": "tcp", "source": "127.0.0.1:9000",
                "rotation_policy": "none",
                "resolved_dependencies": {},
                "topology_mode": "redundant",
                "member_index": 1
            }
        ]
    })
    .to_string();
    let plan = DeploymentPlan::from_json(&plan_json).unwrap();

    s.refresh_due_master_anchors(&plan, NOW).await;

    let refreshed = writer.refreshed.lock().unwrap();
    assert_eq!(
        BTreeSet::from_iter(refreshed.iter().cloned()),
        BTreeSet::from([master0_did.clone(), master1_did.clone()]),
        "each member's own anchor must be republished, signed with its own key: {refreshed:?}"
    );
    drop(refreshed);
    assert_eq!(s.store.last_master_anchor_refresh(&master0_did).unwrap(), Some(NOW as i64));
    assert_eq!(
        s.store.last_master_anchor_refresh(&master1_did).unwrap(),
        Some(NOW as i64),
        "member 1's own row must be stamped, not silently folded into member 0's"
    );
}

/// The stamp moves only on success: a failed publish must leave the
/// previous one alone so the next pass retries rather than waiting out
/// another whole interval.
#[tokio::test]
async fn master_anchor_refresh_updates_last_refreshed_at_on_success() {
    let s = Fixture {
        anchor_writer: Some(Arc::new(RecordingAnchorWriter::default())),
        ..Fixture::default()
    }
    .build();
    let master_did = seeded_member(&s, "backend").await;
    let plan =
        DeploymentPlan::from_json(&plan_json_with_master("backend", &master_did, "none")).unwrap();

    // Never published before, so overdue on the first pass.
    assert_eq!(s.store.last_master_anchor_refresh(&master_did).unwrap(), None);
    s.refresh_due_master_anchors(&plan, NOW).await;
    assert_eq!(s.store.last_master_anchor_refresh(&master_did).unwrap(), Some(NOW as i64));

    let failing = Fixture {
        anchor_writer: Some(Arc::new(RecordingAnchorWriter {
            fail: true,
            ..RecordingAnchorWriter::default()
        })),
        ..Fixture::default()
    }
    .build();
    let failing_master = seeded_member(&failing, "backend").await;
    let failing_plan =
        DeploymentPlan::from_json(&plan_json_with_master("backend", &failing_master, "none"))
            .unwrap();
    failing.refresh_due_master_anchors(&failing_plan, NOW).await;
    assert_eq!(
        failing.store.last_master_anchor_refresh(&failing_master).unwrap(),
        None,
        "a failed publish must not be stamped as a success"
    );
}

/// A row with no app master DID yet (before this instance's first
/// `adopt`) is skipped without even opening the vault, and nothing
/// gets minted to fill the gap.
#[tokio::test]
async fn an_instance_with_no_app_master_did_is_skipped_and_nothing_is_minted() {
    let writer = Arc::new(RecordingTier1Writer::default());
    let s = Fixture { tier1_writer: Some(writer.clone()), ..Fixture::default() }.build();
    let state = desired_state_with_app_master("inst-1", "");
    let mut opened = Vec::new();

    s.refresh_due_app_tier1_record(&AppInstanceId::new("inst-1"), &state, NOW, &mut opened).await;

    assert!(
        writer.published.lock().unwrap().is_empty(),
        "no app master DID recorded must never reach the writer"
    );
    assert!(
        s.vault.get("app-inst-1").await.unwrap().is_none(),
        "the skip must never mint an app master"
    );
    assert!(opened.is_empty());
}

/// A supervisor with no registry configured holds no writer at all
/// (mirroring `RegistryAnchorWriter::from_registry_client`), and a
/// pass with none configured is a quiet no-op, never a panic. The
/// `RegistryTier1Writer::from_registry_client` half is asserted
/// directly here; the warning log line itself is written once at
/// supervisor init (`runtime.rs`), outside what a `SupervisorService`
/// unit test can reach.
#[tokio::test]
async fn no_configured_registry_holds_no_writer_and_the_supervisor_keeps_running() {
    assert!(
        RegistryTier1Writer::from_registry_client(None).is_none(),
        "no substrate.registry_url must mean no writer, not one that quietly does nothing"
    );

    let s = Fixture::default().build();
    let (app_did, _) = keys::app_master(&s.vault, "inst-1").await.unwrap();
    let state = desired_state_with_app_master("inst-1", &app_did);
    let mut opened = Vec::new();

    s.refresh_due_app_tier1_record(&AppInstanceId::new("inst-1"), &state, NOW, &mut opened).await;

    assert_eq!(
        s.store.last_tier1_refresh(&app_did).unwrap(),
        None,
        "with no writer configured there is nothing to stamp"
    );
}

/// The real property is not the interval, it is that a failure never
/// gives up: every pass this many seconds after the last success
/// retries again, with nothing here counting attempts toward a cap.
/// `tier1_refresh_survives_sixty_consecutive_failures_against_the_default_interval`
/// (`syneroym-core`) pins the number this buys against `EndpointInfo`'s
/// 30-day `not_after`.
#[tokio::test]
async fn a_failed_tier1_publish_is_retried_every_interval_with_no_internal_cap() {
    let failing = Arc::new(RecordingTier1Writer { fail: true, ..RecordingTier1Writer::default() });
    let s = Fixture {
        tier1_writer: Some(failing.clone()),
        master_anchor_refresh_interval_secs: Some(43_200),
        ..Fixture::default()
    }
    .build();
    let (app_did, _) = keys::app_master(&s.vault, "inst-1").await.unwrap();
    let state = desired_state_with_app_master("inst-1", &app_did);
    let instance_id = AppInstanceId::new("inst-1");

    for tick in 0..3u64 {
        let mut opened = Vec::new();
        s.refresh_due_app_tier1_record(&instance_id, &state, NOW + tick * 43_200, &mut opened)
            .await;
    }

    assert_eq!(
        *failing.calls.lock().unwrap(),
        3,
        "every interval-elapsed pass must retry -- nothing here counts attempts and stops"
    );
    assert_eq!(
        s.store.last_tier1_refresh(&app_did).unwrap(),
        None,
        "a failed publish is never stamped as a success"
    );
}

/// The gate itself, exercised across a real success: the failure-only
/// test above cannot regress this property, since a stamp that never
/// advances reads as "due" on every single tick regardless of what
/// the interval comparison does.
#[tokio::test]
async fn the_interval_gate_holds_after_a_successful_publish() {
    let writer = Arc::new(RecordingTier1Writer::default());
    let s = Fixture {
        tier1_writer: Some(writer.clone()),
        master_anchor_refresh_interval_secs: Some(43_200),
        ..Fixture::default()
    }
    .build();
    let (app_did, _) = keys::app_master(&s.vault, "inst-1").await.unwrap();
    let state = desired_state_with_app_master("inst-1", &app_did);
    let instance_id = AppInstanceId::new("inst-1");

    let mut opened = Vec::new();
    s.refresh_due_app_tier1_record(&instance_id, &state, NOW, &mut opened).await;
    assert_eq!(writer.published.lock().unwrap().len(), 1);

    s.refresh_due_app_tier1_record(&instance_id, &state, NOW + 100, &mut opened).await;
    assert_eq!(
        writer.published.lock().unwrap().len(),
        1,
        "a tick inside the interval must not re-publish"
    );

    s.refresh_due_app_tier1_record(&instance_id, &state, NOW + 43_200, &mut opened).await;
    assert_eq!(
        writer.published.lock().unwrap().len(),
        2,
        "a tick past the interval must publish again"
    );
}

/// The evaluation happens on the ordinary per-instance pass, against a
/// persisted fact -- not on a timer of its own -- so a pass with
/// otherwise nothing to do still reaches it purely because a
/// `tier1_writer` is configured (the same gate `anchor_writer` uses).
/// The published record also carries this pass's real generation, not
/// a hardcoded one -- `generation` is the entire mechanism that keeps
/// two supervisors publishing one app DID from colliding.
#[tokio::test]
async fn a_refresh_runs_on_the_ordinary_pass_tick_and_carries_the_instances_generation() {
    let writer = Arc::new(RecordingTier1Writer::default());
    let s = Fixture { tier1_writer: Some(writer.clone()), ..Fixture::default() }.build();
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:zOwner", 0).unwrap();
    let (app_did, _) = keys::app_master(&s.vault, "inst-1").await.unwrap();
    s.store.record_adopt("inst-1", 7, &app_did).unwrap();

    s.reconcile_instance_pass("inst-1").await;

    let published = writer.published.lock().unwrap();
    assert_eq!(
        published.len(),
        1,
        "a configured writer must be exercised even when every other work list is empty"
    );
    assert_eq!(
        published[0].info.generation, 7,
        "the published record must carry this instance's real generation"
    );
    drop(published);
    assert!(
        s.store.last_tier1_refresh(&app_did).unwrap().is_some(),
        "a successful publish through the ordinary pass must stamp the fact"
    );
}

/// S1-8: the property that matters is one level up from the signer
/// (`tier1::tests::a_locked_vault_fails_the_refresh_without_touching_
/// the_registry`, which takes no writer at all and so is true by
/// construction) -- with a writer actually configured, a locked vault
/// must stop before that writer is ever reached, and must raise
/// `VaultLocked` rather than only logging.
#[tokio::test]
async fn a_locked_vault_never_reaches_a_configured_tier1_writer() {
    let writer = Arc::new(RecordingTier1Writer::default());
    let s =
        Fixture { locked_vault: true, tier1_writer: Some(writer.clone()), ..Fixture::default() }
            .build();
    let state = desired_state_with_app_master("inst-1", "did:key:zPlaceholderApp");
    let instance_id = AppInstanceId::new("inst-1");
    let mut opened = Vec::new();

    s.refresh_due_app_tier1_record(&instance_id, &state, NOW, &mut opened).await;

    assert_eq!(*writer.calls.lock().unwrap(), 0, "a locked vault must never reach the writer");
    assert_eq!(opened, vec![(AlertKind::VaultLocked, "inst-1".to_string())]);
}

/// The regression `kek_is_loaded()` cannot describe: on a node with
/// `storage.encryption = false`, every vault read succeeds, but
/// `kek_is_loaded()` -- a `KeyStore`-only check -- still answers
/// `false`, since no KEK is ever injected on such a node. A pre-check
/// on that answer (which this refresh briefly copied) would skip this
/// instance's Tier-1 publish forever and raise a `VaultLocked` alert
/// that is never true. Reading the real attempt's own
/// `VaultError::Locked` instead must reach the writer here, where the
/// vault is merely unencrypted, not locked.
#[tokio::test]
async fn an_unencrypted_vault_with_no_kek_still_reaches_the_tier1_writer() {
    let writer = Arc::new(RecordingTier1Writer::default());
    let s = Fixture {
        skip_kek_injection: true,
        tier1_writer: Some(writer.clone()),
        ..Fixture::default()
    }
    .build();
    let (app_did, _) = keys::app_master(&s.vault, "inst-1").await.unwrap();
    let state = desired_state_with_app_master("inst-1", &app_did);
    let instance_id = AppInstanceId::new("inst-1");
    let mut opened = Vec::new();

    s.refresh_due_app_tier1_record(&instance_id, &state, NOW, &mut opened).await;

    assert_eq!(
        writer.published.lock().unwrap().len(),
        1,
        "an unencrypted vault with no KEK must not be treated as locked"
    );
    assert!(opened.is_empty(), "a genuinely reachable vault must raise no VaultLocked alert");
}

/// The vault's own key must match the DID the row recorded at the last
/// `adopt` -- a mismatch (an `import-master` not yet followed by an
/// `adopt`) must refuse and raise, never publish under whichever key
/// the vault happens to hold.
#[tokio::test]
async fn a_mismatched_vault_key_raises_an_alert_and_never_publishes() {
    let writer = Arc::new(RecordingTier1Writer::default());
    let s = Fixture { tier1_writer: Some(writer.clone()), ..Fixture::default() }.build();
    // The vault genuinely holds an app master for "inst-1"...
    keys::app_master(&s.vault, "inst-1").await.unwrap();
    // ...but the row claims a different DID, the stale-handover case.
    let state = desired_state_with_app_master("inst-1", "did:key:zStaleRowClaim");
    let instance_id = AppInstanceId::new("inst-1");
    let mut opened = Vec::new();

    s.refresh_due_app_tier1_record(&instance_id, &state, NOW, &mut opened).await;

    assert!(
        writer.published.lock().unwrap().is_empty(),
        "a mismatched identity must never be published"
    );
    assert_eq!(opened, vec![(AlertKind::AppIdentityMismatch, "inst-1".to_string())]);
}

/// Companion to the two loop paused-instance tests: `pause` excludes
/// an instance from the write phase entirely, and the Tier-1 refresh
/// is no exception.
#[tokio::test]
async fn a_paused_instance_gets_no_tier1_refresh() {
    let writer = Arc::new(RecordingTier1Writer::default());
    let s = Fixture { tier1_writer: Some(writer.clone()), ..Fixture::default() }.build();
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:zOwner", 0).unwrap();
    let (app_did, _) = keys::app_master(&s.vault, "inst-1").await.unwrap();
    s.store.record_adopt("inst-1", 0, &app_did).unwrap();
    s.store.pause("inst-1").unwrap();

    s.reconcile_instance_pass("inst-1").await;

    assert!(writer.published.lock().unwrap().is_empty(), "a paused instance gets zero work");
}

/// D-A5d-15: a revoked placement is skipped by `apply_with_clients`
/// itself, which is the one path every certificate-minting caller
/// passes through -- the loop, `submit`, and `force-reconcile` alike.
/// Without that, an ordinary resubmit silently re-mints the very key
/// the operator revoked.
#[tokio::test]
async fn a_submit_of_the_same_plan_does_not_recertify_a_revoked_placement() {
    let s = service();
    let plan = DeploymentPlan::from_json(&plan_json_two_services("inst-1", "backend", "frontend"))
        .unwrap();
    s.store.revoke_placement("inst-1", "inst-1/backend#0", 1_000).unwrap();

    // No clients are built for either service, so `certify_placed_
    // members` fails on whichever service actually reaches it -- and
    // the error names it. A revoked service that reached it would show
    // up here by name.
    let err = s
        .apply_with_clients(&plan, &plan, &BTreeMap::new(), &BTreeMap::new(), 0, Vec::new())
        .await
        .unwrap_err();
    assert!(
        !err.contains("hFabricatedA"),
        "the revoked member must never reach the certify step: {err}"
    );
    assert!(err.contains("hFabricatedB"), "the rest of the plan must still be attempted: {err}");

    let alerts = s.store.alerts.active(&AppInstanceId::new("inst-1")).unwrap();
    let revoked: Vec<_> = alerts.iter().filter(|a| a.kind == AlertKind::InstanceRevoked).collect();
    assert_eq!(revoked.len(), 1, "{alerts:?}");
    assert_eq!(revoked[0].logical_ref.as_deref(), Some("inst-1/backend#0"));
}

/// `force-reconcile` reaches the same gate by the same route: its own
/// doc already notes it bypasses several checks `submit` applies, so
/// putting the exclusion anywhere upstream of `apply_with_clients`
/// would have left this path open.
#[tokio::test]
async fn a_force_reconcile_does_not_recertify_a_revoked_placement_and_raises_instance_revoked_for_the_rest_of_the_plan()
 {
    let s = service();
    let plan_json = plan_json_two_services("inst-1", "backend", "frontend");
    let plan = DeploymentPlan::from_json(&plan_json).unwrap();
    s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();
    s.store.revoke_placement("inst-1", "inst-1/backend#0", 1_000).unwrap();

    let err = s
        .apply_with_clients(&plan, &plan, &BTreeMap::new(), &BTreeMap::new(), 0, Vec::new())
        .await
        .unwrap_err();
    assert!(!err.contains("hFabricatedA"), "{err}");

    let alerts = s.store.alerts.active(&AppInstanceId::new("inst-1")).unwrap();
    let revoked = alerts
        .iter()
        .find(|a| a.kind == AlertKind::InstanceRevoked)
        .unwrap_or_else(|| panic!("no InstanceRevoked alert among {alerts:?}"));
    assert!(
        revoked.detail.contains("Undeploy it separately"),
        "the alert must say revocation is not a teardown: {}",
        revoked.detail
    );
}

/// The raise used to pass the *alias* for both the
/// alias and DID arguments, so `substrate_did` on the stored row held
/// e.g. `edge-1` instead of a real DID -- inconsistent with every
/// other alert kind's rows. Resolved through this pass's own
/// connected clients, so the column holds what it is supposed to.
#[tokio::test]
async fn instance_revoked_records_a_real_substrate_did_not_the_alias() {
    let s = service();
    let plan_json = plan_json_one_service("inst-1", "backend", Some("edge-1"));
    let plan = DeploymentPlan::from_json(&plan_json).unwrap();
    s.store.revoke_placement("inst-1", "inst-1/backend#0", 1_000).unwrap();

    let identity = Identity::generate().unwrap();
    let client = Arc::new(SyneroymClient::new_with_identity(
        "did:key:zEdge1".to_string(),
        String::new(),
        identity,
    ));
    let clients: BTreeMap<SubstrateAlias, Arc<SyneroymClient>> =
        BTreeMap::from([(SubstrateAlias::new("edge-1"), client)]);

    // The plan's one service is entirely revoked, so nothing remains
    // to certify once it is filtered out -- no live connection is
    // needed for the call to succeed.
    s.apply_with_clients(&plan, &plan, &BTreeMap::new(), &clients, 0, Vec::new()).await.unwrap();

    let alerts = s.store.alerts.active(&AppInstanceId::new("inst-1")).unwrap();
    let revoked = alerts
        .iter()
        .find(|a| a.kind == AlertKind::InstanceRevoked)
        .unwrap_or_else(|| panic!("no InstanceRevoked alert among {alerts:?}"));
    assert_eq!(revoked.substrate_did, "did:key:zEdge1");
    assert_ne!(revoked.substrate_did, "edge-1", "must be the DID, not the alias");
}

/// Every existing revocation test drove one `apply_with_clients` call
/// in isolation and stopped, so nothing proved the *next* pass stays
/// quiet. The trigger: an ordinary `submit`
/// or `force-reconcile` after a revocation, which reaches
/// `apply_with_clients` with `record_plan == plan` -- followed by a
/// real resident-loop pass reading back what that call journaled.
#[tokio::test]
async fn a_revoked_placement_does_not_reappear_as_an_add_on_the_next_pass() {
    let s = service();
    let plan_json = plan_json_one_service("inst-1", "backend", Some("edge-1"));
    s.store.submit("inst-1", &plan_json, "{}", "did:key:owner", 0).unwrap();
    let plan = DeploymentPlan::from_json(&plan_json).unwrap();
    s.store.revoke_placement("inst-1", "inst-1/backend#0", 1_000).unwrap();

    let identity = Identity::generate().unwrap();
    let client = Arc::new(SyneroymClient::new_with_identity(
        "did:key:zEdge1".to_string(),
        String::new(),
        identity,
    ));
    let clients: BTreeMap<SubstrateAlias, Arc<SyneroymClient>> =
        BTreeMap::from([(SubstrateAlias::new("edge-1"), client)]);

    // The ordinary `submit`/`force-reconcile` route: `record_plan ==
    // plan`, the shape that used to drop the revoked member from the
    // journaled baseline.
    s.apply_with_clients(&plan, &plan, &BTreeMap::new(), &clients, 0, Vec::new()).await.unwrap();
    let instance_id = AppInstanceId::new("inst-1");
    let after_first_apply = s.store.journal.get_latest(&instance_id).unwrap().unwrap();

    // Checked directly against what the
    // next pass's own diff would read: the revoked member must not
    // show up as a fresh `Add` against the baseline the call above
    // just journaled.
    let diff = Reconciler::new(&s.store.journal).compute_diff(&plan).unwrap();
    assert!(
        diff.actions.is_empty(),
        "a revoked member must not read back as a change against its own just-journaled baseline: \
         {:?}",
        diff.actions
    );

    // A real resident-loop pass, reading exactly that baseline back.
    // Unbounded regrowth would show up here as a second journal
    // entry -- the ~2,880-rows-a-day shape the review measured.
    s.reconcile_instance_pass("inst-1").await;
    let after_second_pass = s.store.journal.get_latest(&instance_id).unwrap().unwrap();
    assert_eq!(
        after_second_pass.id, after_first_apply.id,
        "a quiet pass must not append a new journal record"
    );
}

/// The renewal work-list's own half of the same exclusion.
#[test]
fn a_renewal_pass_skips_a_revoked_placement_even_when_near_expiry() {
    let report = report_of(vec![
        near_expiry_health("backend", "did:key:hBackend"),
        near_expiry_health("frontend", "did:key:hFrontend"),
    ]);
    let revoked = BTreeSet::from(["inst-1/backend#0".to_string()]);
    let candidates =
        SupervisorService::renewal_candidates(&report, &BTreeSet::new(), &revoked, NOW, 5);
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].member_ref, "inst-1/frontend#0");
}

/// D-A5d-14: without the lock, an operator's `revoke-instance` and a
/// resident pass's renewal of the same member race -- the pass could
/// mint and install a fresh certificate in the gap between the anchor
/// write and the exclusion write landing, which is the window this
/// verb exists to close.
#[tokio::test]
async fn revoke_instance_takes_the_instance_lock_for_the_whole_call() {
    let s = Arc::new(
        Fixture {
            anchor_writer: Some(Arc::new(RecordingAnchorWriter::default())),
            ..Fixture::default()
        }
        .build(),
    );
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0).unwrap();

    let held = s.instance_lock("inst-1");
    let guard = held.lock().await;

    let s2 = s.clone();
    let call = tokio::spawn(async move {
        dispatch(
            &s2,
            admin_caller("did:key:zSupervisorNode"),
            "revoke-instance",
            serde_json::json!(["inst-1", "inst-1/backend"]),
        )
        .await
    });

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!call.is_finished(), "revoke-instance must block on the instance lock");
    drop(guard);

    // Now it proceeds -- and refuses, because the stored plan names no
    // such member. What matters here is that it got that far only
    // after the lock was released.
    let err = call.await.unwrap().unwrap_err();
    assert!(err.to_string().contains("inst-1/backend"), "{err}");
}

/// The line that actually decides which DID
/// gets revoked had no direct test, since `handle_revoke_instance`
/// needs a live client. Hoisted into a pure function so both branches
/// are assertable with no substrate at all.
#[test]
fn select_revocation_did_prefers_the_installed_did_over_the_derived_one() {
    let installed = syneroym_sdk::InstanceIdentity {
        instance_did: "did:key:zDerivedForThisCaller".to_string(),
        pubkey_hex: "aa".to_string(),
        installed_temporary_did: Some("did:key:zActuallyInstalled".to_string()),
    };
    assert_eq!(SupervisorService::select_revocation_did(installed), "did:key:zActuallyInstalled");

    let nothing_installed = syneroym_sdk::InstanceIdentity {
        instance_did: "did:key:zDerivedForThisCaller".to_string(),
        pubkey_hex: "aa".to_string(),
        installed_temporary_did: None,
    };
    assert_eq!(
        SupervisorService::select_revocation_did(nothing_installed),
        "did:key:zDerivedForThisCaller"
    );
}

/// The anchor half: the *derived instance* DID goes into the master's
/// revoked list, never the master's own -- revoking the master would
/// repudiate every instance it has ever certified.
#[tokio::test]
async fn revoke_instance_appends_the_derived_instance_did_to_revoked_keys() {
    let writer = Arc::new(RecordingAnchorWriter::default());
    let s = Fixture { anchor_writer: Some(writer.clone()), ..Fixture::default() }.build();
    let master_did = seeded_member(&s, "backend").await;

    s.record_revocation("inst-1", "inst-1/backend", "backend", 0, "did:key:zInstanceKey")
        .await
        .unwrap();

    assert_eq!(
        *writer.revoked.lock().unwrap(),
        vec![(master_did.clone(), "did:key:zInstanceKey".to_string())]
    );
    assert_ne!(
        writer.revoked.lock().unwrap()[0].1,
        master_did,
        "the revoked entry must be the instance key, not the member master"
    );
}

/// The local half, and the ordering between the two: the anchor
/// publish comes first, and the exclusion is written only after it
/// succeeds. A failed publish must leave the placement under ordinary
/// management rather than half-revoked -- excluded from renewal here
/// while still fully trusted by every consumer, which would let it age
/// out quietly instead of failing closed.
#[tokio::test]
async fn revoke_instance_writes_a_revoked_placements_row() {
    let s = Fixture {
        anchor_writer: Some(Arc::new(RecordingAnchorWriter::default())),
        ..Fixture::default()
    }
    .build();
    seeded_member(&s, "backend").await;

    s.record_revocation("inst-1", "inst-1/backend", "backend", 0, "did:key:zInstanceKey")
        .await
        .unwrap();
    assert_eq!(
        s.store.revoked_placements("inst-1").unwrap(),
        BTreeSet::from(["inst-1/backend".to_string()])
    );

    let failing_writer =
        Arc::new(RecordingAnchorWriter { fail: true, ..RecordingAnchorWriter::default() });
    let failing = Fixture { anchor_writer: Some(failing_writer), ..Fixture::default() }.build();
    seeded_member(&failing, "backend").await;

    let err = failing
        .record_revocation("inst-1", "inst-1/backend", "backend", 0, "did:key:zInstanceKey")
        .await
        .unwrap_err();
    assert!(err.contains("failed to publish"), "{err}");
    assert!(
        failing.store.revoked_placements("inst-1").unwrap().is_empty(),
        "a revocation that did not publish must not have written a local exclusion"
    );
}

/// A node with no registry configured cannot publish a revocation at
/// all, and must say so rather than writing a local exclusion that no
/// consumer can see.
#[tokio::test]
async fn revoke_instance_is_refused_when_the_node_has_no_registry_configured() {
    let s = service();
    s.store.submit("inst-1", &plan_json_no_services("inst-1"), "{}", "did:key:owner", 0).unwrap();

    let err = s
        .handle_revoke_instance(
            &admin_caller("did:key:zSupervisorNode"),
            serde_json::json!(["inst-1", "inst-1/backend"]),
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("registry"), "{err}");
    assert!(s.store.revoked_placements("inst-1").unwrap().is_empty());
}

/// The ordinary path, over a services-less plan so no substrate is
/// involved -- `adopt` mints an app master and both the vault and the
/// instance row carry it afterwards.
#[tokio::test]
async fn adopt_mints_an_app_master_and_records_it_on_the_instance_row() {
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
    let did = adopt_field(&res, "app_master_did").and_then(Value::as_str).unwrap();
    let vault_name = adopt_field(&res, "vault_name").and_then(Value::as_str).unwrap();
    assert!(did.starts_with("did:key:"), "{did}");
    assert_eq!(vault_name, "app-inst-1");
    assert_eq!(adopt_field(&res, "generation").and_then(Value::as_u64), Some(1));

    let row_did = s.store.get("inst-1").unwrap().unwrap().app_master_did;
    assert_eq!(row_did, did);
    let vault_entry = s.vault.get("app-inst-1").await.unwrap().unwrap();
    assert_eq!(substrate::derive_did_key(&vault_entry.public_key()), did);
}
