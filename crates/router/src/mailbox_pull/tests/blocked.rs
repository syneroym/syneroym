use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use syneroym_control_plane::MailboxPullControl;
use syneroym_core::mailbox::codes;
use syneroym_identity::{DelegationCertificate, delegation::SCOPE_SERVICE_INSTANCE};
use syneroym_mailbox_format::Receipt;
use tokio::{sync::Notify, time};

use super::helpers::*;
use crate::mailbox_pull::notes::{Note, Notes};

/// A new certificate for the service's instance key.
fn renewed_cert(node: &PullNode) -> DelegationCertificate {
    let instance = node.node_identity.derive_service_identity(OWNER, &node.service_id);
    DelegationCertificate::issue(
        &node.svc_master,
        instance.public_key(),
        7200,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap()
}

#[tokio::test]
async fn an_expired_certificate_waits_and_then_pulls() {
    let node = pull_node(&["greeter#greet"]).await;
    node.pass().await;
    node.registry.remove_instance_cert(&node.service_id).await.unwrap();
    let sender = SenderMember::new();
    let sent = node.deposit(&sender, &greet()).await;

    let summary = node.pass().await;
    assert_eq!(summary.blocked, 1);
    assert_eq!(node.dispatch.call_count(), 0);
    assert!(matches!(node.receipt(&sender, &sent).await, Receipt::Stored { .. }));

    // The supervisor renews the certificate.
    node.registry.set_instance_cert(node.service_id.clone(), renewed_cert(&node)).await.unwrap();
    let summary = node.pass().await;
    assert_eq!(summary.delivered, 1);
}

#[tokio::test]
async fn a_refused_instance_waits_for_a_new_certificate() {
    let node = pull_node(&["greeter#greet"]).await;
    node.mailbox.fail_with("register", codes::NOT_AN_INSTANCE);
    let summary = node.pass().await;
    assert_eq!((summary.blocked, summary.registered), (1, 0), "{summary:?}");
    node.mailbox.clear_failures();

    // The mailbox would accept now, but the certificate is the same.
    node.pass().await;
    assert_eq!(node.mailbox.count("register"), 1);

    node.registry.set_instance_cert(node.service_id.clone(), renewed_cert(&node)).await.unwrap();
    node.pass().await;
    assert_eq!(node.mailbox.count("register"), 2);
}

#[tokio::test]
async fn a_stale_instance_stops_until_its_certificate_changes() {
    let node = pull_node(&["greeter#greet"]).await;
    node.pass().await;
    node.mailbox.fail_with("claim", codes::STALE_INSTANCE);
    node.pass().await;
    node.mailbox.clear_failures();
    let claims = node.mailbox.count("claim");

    node.pass().await;
    assert_eq!(node.mailbox.count("claim"), claims, "no claim with the same certificate");

    node.registry.set_instance_cert(node.service_id.clone(), renewed_cert(&node)).await.unwrap();
    node.pass().await;
    assert_eq!(node.mailbox.count("claim"), claims + 1);
}

/// The installed certificate and a renewal of it with a longer life, both
/// issued in the same second.
fn renewal_in_the_same_second(node: &PullNode) -> (DelegationCertificate, DelegationCertificate) {
    let instance = node.node_identity.derive_service_identity(OWNER, &node.service_id);
    let issue = |secs| {
        DelegationCertificate::issue(
            &node.svc_master,
            instance.public_key(),
            secs,
            SCOPE_SERVICE_INSTANCE.to_string(),
        )
        .unwrap()
    };
    loop {
        let (first, renewed) = (issue(3600), issue(7200));
        if first.issued_at_secs == renewed.issued_at_secs {
            return (first, renewed);
        }
    }
}

#[tokio::test]
async fn a_renewal_in_the_same_second_unblocks_the_service() {
    let node = pull_node(&["greeter#greet"]).await;
    let (first, renewed) = renewal_in_the_same_second(&node);
    node.registry.set_instance_cert(node.service_id.clone(), first).await.unwrap();
    node.mailbox.fail_with("register", codes::STALE_INSTANCE);
    assert_eq!(node.pass().await.blocked, 1);
    node.mailbox.clear_failures();

    node.registry.set_instance_cert(node.service_id.clone(), renewed).await.unwrap();

    assert_eq!(node.pass().await.registered, 1, "the renewed certificate is a new one");
}

#[tokio::test]
async fn a_locked_vault_skips_the_service() {
    let node = pull_node(&["greeter#greet"]).await;
    *node.vault.mode.lock().unwrap() = VaultMode::Locked;
    assert_eq!(node.pass().await.blocked, 1);
    assert_eq!(node.mailbox.count("register"), 0);

    *node.vault.mode.lock().unwrap() = VaultMode::NoKey;
    assert_eq!(node.pass().await.blocked, 1);

    *node.vault.mode.lock().unwrap() = VaultMode::Open;
    assert_eq!(node.pass().await.blocked, 0);
    assert_eq!(node.mailbox.count("register"), 1);
}

#[tokio::test]
async fn no_stored_record_skips_the_service() {
    let node = pull_node(&["greeter#greet"]).await;
    *node.vault.record.lock().unwrap() = None;

    assert_eq!(node.pass().await.blocked, 1);
    assert_eq!(node.mailbox.count("register"), 0);
}

#[tokio::test(start_paused = true)]
async fn a_note_is_written_once_an_hour() {
    let mut notes = Notes::default();

    assert!(notes.note("svc", Note::NoKey, ""));
    assert!(!notes.note("svc", Note::NoKey, ""));
    assert!(notes.note("svc", Note::NoRecord, ""), "another note is its own");
    assert!(notes.note("other", Note::NoKey, ""), "another service is its own");

    time::advance(Duration::from_secs(3599)).await;
    assert!(!notes.note("svc", Note::NoKey, ""));
    time::advance(Duration::from_secs(2)).await;
    assert!(notes.note("svc", Note::NoKey, ""));
}

#[tokio::test]
async fn pull_now_runs_after_a_running_pass() {
    let node = Arc::new(pull_node(&["greeter#greet"]).await);
    node.pass().await;
    let sender = SenderMember::new();
    node.deposit(&sender, &greet()).await;
    let (started, proceed) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    {
        let (started, proceed) = (started.clone(), proceed.clone());
        *node.dispatch.on_first.lock().unwrap() = Some(Box::new(move || {
            Box::pin(async move {
                started.notify_one();
                proceed.notified().await;
            })
        }));
    }
    let running = {
        let node = node.clone();
        tokio::spawn(async move { node.pass().await })
    };
    started.notified().await;

    let waiting = {
        let node = node.clone();
        tokio::spawn(async move { node.puller.pull_now().await })
    };
    time::sleep(Duration::from_millis(200)).await;
    assert!(!waiting.is_finished(), "pull-now waits for the pass that is running");

    proceed.notify_one();
    assert_eq!(running.await.unwrap().delivered, 1);
    assert_eq!(waiting.await.unwrap().delivered, 0, "its own pass finds nothing left");
}

#[tokio::test]
async fn a_full_mailbox_or_a_newer_instance_counts_as_blocked() {
    let node = pull_node(&["greeter#greet"]).await;
    node.mailbox.fail_with("register", codes::MAILBOX_FULL);
    let summary = node.pass().await;
    assert_eq!((summary.blocked, summary.registered), (1, 0), "{summary:?}");
    node.mailbox.clear_failures();
    assert_eq!(node.pass().await.registered, 1);

    node.mailbox.fail_with("claim", codes::STALE_INSTANCE);
    let summary = node.pass().await;
    assert_eq!((summary.blocked, summary.registered), (1, 0), "{summary:?}");
}

#[tokio::test]
async fn a_service_without_routes_after_a_restart_is_not_claimed() {
    let node = pull_node(&["greeter#greet"]).await;
    node.dispatch.not_started.store(true, Ordering::SeqCst);
    let sender = SenderMember::new();
    node.pass().await;
    let sent = node.deposit(&sender, &greet()).await;

    let summary = node.pass().await;

    assert_eq!((summary.blocked, summary.registered), (1, 1), "{summary:?}");
    assert_eq!(node.mailbox.count("claim"), 0, "no claim is used up");
    assert!(matches!(node.receipt(&sender, &sent).await, Receipt::Stored { .. }));

    // The deploy after the restart registers the routes again.
    node.dispatch.not_started.store(false, Ordering::SeqCst);
    assert_eq!(node.pass().await.delivered, 1);
}
