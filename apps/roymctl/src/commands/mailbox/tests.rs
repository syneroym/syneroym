use std::fs;

use syneroym_identity::{Identity, substrate};
use syneroym_ucan::CapabilityToken;

use super::*;

fn did(identity: &Identity) -> String {
    substrate::derive_did_key(&identity.public_key())
}

fn args(to: &str, out: &Path) -> MailboxIssueArgs {
    MailboxIssueArgs {
        signer: IssueSigner {
            from: Some("controller".to_string()),
            node_key: false,
            to: to.to_string(),
            expires_days: 30,
            out: Some(out.to_path_buf()),
        },
        max_services: None,
        max_bytes: None,
        max_messages: None,
        max_ttl_days: None,
    }
}

fn save_identity(dir: &Path, name: &str) -> Identity {
    fs::create_dir_all(dir.join("identities")).unwrap();
    let identity = Identity::generate().unwrap();
    identity.save_to_path(dir.join("identities").join(format!("{name}.key"))).unwrap();
    identity
}

fn read_passport(path: &Path) -> CapabilityToken {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn issue_writes_exactly_the_given_facts_for_the_mailbox_selector() {
    let dir = tempfile::tempdir().unwrap();
    let controller = save_identity(dir.path(), "controller");
    let owner = Identity::generate().unwrap();
    let host = did(&Identity::generate().unwrap());
    let out = dir.path().join("passport.json");
    let mut a = args(&did(&owner), &out);
    a.max_services = Some(3);
    a.max_bytes = Some(5_000_000);
    a.max_messages = Some(500);
    a.max_ttl_days = Some(14);

    let id = handle_issue(&a, Some(host.clone()), dir.path()).unwrap();

    let passport = read_passport(&out);
    assert_eq!(PassportId::of(&passport).unwrap(), id);
    assert_eq!(passport.issuer_did, did(&controller));
    assert_eq!(passport.audience_did, did(&owner));
    assert_eq!(passport.capabilities.len(), 1);
    assert_eq!(passport.capabilities[0].with.0, format!("substrate:{host}/mailbox"));
    assert_eq!(passport.capabilities[0].can.0, MAILBOX_USE.ability);
    let expected = json!({
        "max_services": 3,
        "max_bytes": 5_000_000,
        "max_messages": 500,
        "max_ttl_secs": 14 * 24 * 3600,
    });
    assert_eq!(serde_json::Value::Object(passport.facts), expected);
}

#[test]
fn issue_with_no_limit_flags_writes_no_facts() {
    let dir = tempfile::tempdir().unwrap();
    save_identity(dir.path(), "controller");
    let owner = Identity::generate().unwrap();
    let host = did(&Identity::generate().unwrap());
    let out = dir.path().join("passport.json");

    handle_issue(&args(&did(&owner), &out), Some(host), dir.path()).unwrap();

    assert!(read_passport(&out).facts.is_empty());
}

#[test]
fn issue_refuses_a_time_to_live_over_the_cap_or_zero() {
    let dir = tempfile::tempdir().unwrap();
    save_identity(dir.path(), "controller");
    let owner = Identity::generate().unwrap();
    let host = did(&Identity::generate().unwrap());
    let out = dir.path().join("passport.json");

    for days in [0, 31] {
        let mut a = args(&did(&owner), &out);
        a.max_ttl_days = Some(days);
        let err = handle_issue(&a, Some(host.clone()), dir.path()).unwrap_err();
        assert!(err.to_string().contains("--max-ttl-days must be from 1 to 30"), "{err}");
        assert!(!out.exists());
    }

    let mut ok = args(&did(&owner), &out);
    ok.max_ttl_days = Some(30);
    handle_issue(&ok, Some(host), dir.path()).unwrap();
    assert_eq!(read_passport(&out).facts["max_ttl_secs"], json!(DEDICATED_MAX_TTL_SECS));
}

#[tokio::test]
async fn deny_refuses_a_malformed_passport_id_offline() {
    let dir = tempfile::tempdir().unwrap();
    let command = MailboxCommands::Passport {
        command: MailboxPassportCommands::Deny { passport_id: "not-a-valid-id".to_string() },
    };
    let err =
        handle(&command, "http://127.0.0.1:1", None, dir.path(), None, None).await.unwrap_err();
    assert!(err.to_string().contains("64 lowercase hex"), "{err}");
}

#[test]
fn issue_refuses_a_limit_of_zero() {
    let dir = tempfile::tempdir().unwrap();
    save_identity(dir.path(), "controller");
    let owner = Identity::generate().unwrap();
    let host = did(&Identity::generate().unwrap());
    let out = dir.path().join("passport.json");

    for set in [
        |a: &mut MailboxIssueArgs| a.max_services = Some(0),
        |a: &mut MailboxIssueArgs| a.max_bytes = Some(0),
        |a: &mut MailboxIssueArgs| a.max_messages = Some(0),
    ] {
        let mut a = args(&did(&owner), &out);
        set(&mut a);
        let err = handle_issue(&a, Some(host.clone()), dir.path()).unwrap_err();
        assert!(err.to_string().contains("at least 1"), "{err}");
        assert!(!out.exists(), "no useless passport is written");
    }
}
