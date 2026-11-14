use syneroym_wit_interfaces::translation_host::syneroym::translation::translation::{
    Host as TranslationHost, TranslationError,
};

use super::tests_events::test_host_state;
use crate::host_capabilities::HostState;

#[tokio::test]
async fn targets_are_empty() {
    let temp = tempfile::tempdir().unwrap();
    let mut host = test_host_state("svc_a", None, false, None, temp.path());

    assert!(TranslationHost::supported_targets(&mut host).await.is_empty());
}

#[tokio::test]
async fn translate_is_unavailable() {
    let temp = tempfile::tempdir().unwrap();
    let mut host = test_host_state("svc_a", None, false, None, temp.path());

    let res =
        TranslationHost::translate(&mut host, "नल वाला".into(), Some("hi".into()), "en".into())
            .await;
    assert!(matches!(res, Err(TranslationError::Unavailable)));
}

#[tokio::test]
async fn detect_is_unavailable() {
    let temp = tempfile::tempdir().unwrap();
    let mut host = test_host_state("svc_a", None, false, None, temp.path());

    let res = TranslationHost::detect_language(&mut host, "plumber".into()).await;
    assert!(matches!(res, Err(TranslationError::Unavailable)));
}

/// The three answers need no host state: no event bus, no app instance and
/// no network. The native build calls these functions with no host at all.
#[test]
fn no_op_host_makes_no_call() {
    assert!(HostState::translation_targets().is_empty());
    assert!(matches!(HostState::detect_language("x"), Err(TranslationError::Unavailable)));
    assert!(matches!(HostState::translate("x", None, "en"), Err(TranslationError::Unavailable)));
}
