use std::fs;

use syneroym_core::config::DEFAULT_SUBSTRATE_KEY_FILE;
use syneroym_identity::{Identity, substrate};
use syneroym_passport::{LeaseStore, Proof, Registrar, SignerBinding};
use syneroym_ucan::CapabilityToken;

use super::*;
use crate::commands::passport_issue::SECS_PER_DAY;

fn did(identity: &Identity) -> String {
    substrate::derive_did_key(&identity.public_key())
}

fn args(from: Option<&str>, node_key: bool, to: &str, out: &Path) -> IssueArgs {
    IssueArgs {
        signer: IssueSigner {
            from: from.map(str::to_string),
            node_key,
            to: to.to_string(),
            expires_days: 30,
            out: Some(out.to_path_buf()),
        },
        max_nodes: Some(2),
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

/// The owner delegates the passport to a node and the node registers.
fn register(passport: &CapabilityToken, owner: &Identity, host: &str, root: &str) {
    let node = Identity::generate().unwrap();
    let now = syneroym_passport::now_secs();
    let token = syneroym_passport::child_token(owner, passport, &did(&node), 3_600, now).unwrap();
    let registrar = Registrar::new(
        RELAY_HOME,
        host.to_string(),
        vec![root.to_string()],
        LeaseStore::open_in_memory(&RELAY_HOME).unwrap(),
    )
    .unwrap();
    let challenge = registrar.issue_challenge(now).unwrap();
    let signature =
        syneroym_passport::sign_challenge(&node, host, &RELAY_HOME, &challenge, &token).unwrap();
    let proof = Proof::Challenge { challenge, signer_did: did(&node), signature };
    let checked = registrar.check(&token, &proof, &SignerBinding::SameKey, now).unwrap();
    let lease = registrar.grant(checked, now).unwrap();
    assert_eq!(lease.limits, Map::from_iter([(facts::MAX_NODES.to_string(), json!(2))]));
}

#[test]
fn issue_from_a_controller_writes_a_passport_that_registers() {
    let dir = tempfile::tempdir().unwrap();
    let controller = save_identity(dir.path(), "controller");
    let owner = Identity::generate().unwrap();
    let host = did(&Identity::generate().unwrap());
    let out = dir.path().join("passport.json");

    let id = handle_issue(
        &args(Some("controller"), false, &did(&owner), &out),
        Some(host.clone()),
        dir.path(),
    )
    .unwrap();

    let passport = read_passport(&out);
    assert_eq!(PassportId::of(&passport).unwrap(), id);
    assert_eq!(passport.issuer_did, did(&controller));
    assert_eq!(passport.audience_did, did(&owner));
    assert_eq!(passport.expires_at_secs - passport.not_before_secs, 30 * SECS_PER_DAY);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&out).unwrap().permissions().mode() & 0o777, 0o600);
    }
    register(&passport, &owner, &host, &did(&controller));
}

#[test]
fn issue_with_the_node_key() {
    let dir = tempfile::tempdir().unwrap();
    let node = Identity::generate().unwrap();
    node.save_to_path(dir.path().join(DEFAULT_SUBSTRATE_KEY_FILE)).unwrap();
    let owner = Identity::generate().unwrap();
    let out = dir.path().join("passport.json");

    handle_issue(&args(None, true, &did(&owner), &out), None, dir.path()).unwrap();

    let passport = read_passport(&out);
    assert_eq!(passport.issuer_did, did(&node));
    register(&passport, &owner, &did(&node), &did(&node));
}

#[test]
fn issue_refuses_a_node_key_that_is_not_the_substrate() {
    let dir = tempfile::tempdir().unwrap();
    Identity::generate()
        .unwrap()
        .save_to_path(dir.path().join(DEFAULT_SUBSTRATE_KEY_FILE))
        .unwrap();
    let other_host = did(&Identity::generate().unwrap());
    let out = dir.path().join("passport.json");

    let err =
        handle_issue(&args(None, true, &other_host, &out), Some(other_host.clone()), dir.path())
            .unwrap_err();
    assert!(err.to_string().contains("not of --substrate"), "{err}");
    assert!(!out.exists());
}

#[test]
fn issue_refuses_zero_expiry_days() {
    let dir = tempfile::tempdir().unwrap();
    save_identity(dir.path(), "controller");
    let host = did(&Identity::generate().unwrap());
    let out = dir.path().join("passport.json");
    let mut zero = args(Some("controller"), false, &host, &out);
    zero.signer.expires_days = 0;

    let err = handle_issue(&zero, Some(host), dir.path()).unwrap_err();
    assert!(err.to_string().contains("--expires-days"), "{err}");
}

#[tokio::test]
async fn deny_refuses_malformed_passport_id_offline() {
    let dir = tempfile::tempdir().unwrap();
    let command = CoordinatorCommands::Passport {
        command: PassportCommands::Deny { passport_id: "not-a-valid-id".to_string() },
    };
    let err =
        handle(&command, "http://127.0.0.1:1", None, dir.path(), None, None).await.unwrap_err();
    assert!(err.to_string().contains("64 lowercase hex"), "{err}");
}
