//! Integration tests for the mailbox passport commands and deploy flags of
//! Roymctl: the argument parser and its refusals, with no node.

use std::error::Error;

use assert_cmd::Command;
use predicates::str::contains;

const MAILBOX_HOST: &str = "did:key:h7wy641jeaawpdosg7mwogqromgksnkk3rh7faa93ga5cntowi1duxwe";

/// `mailbox passport issue --help` lists every flag, and the limit flags
/// say what an omitted or low value means.
#[test]
fn test_mailbox_passport_issue_help() -> Result<(), Box<dyn Error>> {
    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.args(["mailbox", "passport", "issue", "--help"])
        .assert()
        .success()
        .stdout(contains("--from"))
        .stdout(contains("--node-key"))
        .stdout(contains("--to"))
        .stdout(contains("--expires-days"))
        .stdout(contains("--max-services"))
        .stdout(contains("--max-bytes"))
        .stdout(contains("--max-messages"))
        .stdout(contains("--max-ttl-days"))
        .stdout(contains("--out"))
        .stdout(contains("mailbox's own limit"));
    Ok(())
}

/// `mailbox passport deny` and `leases` parse, name the `--as` hint and the
/// unowned-host refusal; `deny` without an id is a usage error.
#[test]
fn test_mailbox_passport_deny_and_leases_help() -> Result<(), Box<dyn Error>> {
    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.args(["mailbox", "passport", "deny", "--help"])
        .assert()
        .success()
        .stdout(contains("--as"))
        .stdout(contains("admin_ucan_root"));

    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.args(["mailbox", "passport", "leases", "--help"])
        .assert()
        .success()
        .stdout(contains("--as"))
        .stdout(contains("admin_ucan_root"));

    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.args(["mailbox", "passport", "deny"]).assert().failure().stderr(contains("<PASSPORT_ID>"));
    Ok(())
}

/// The signer is always named for `mailbox passport issue` too.
#[test]
fn test_mailbox_passport_issue_needs_a_signer() -> Result<(), Box<dyn Error>> {
    let base = ["mailbox", "passport", "issue", "--to", "did:key:hx", "--expires-days", "1"];
    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.args(base).assert().failure().stderr(contains("--from"));
    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.args(base)
        .args(["--from", "controller", "--node-key"])
        .assert()
        .failure()
        .stderr(contains("cannot be used with"));
    Ok(())
}

/// `svc deploy --mailbox-passport` is listed and needs `--mailbox`.
#[test]
fn test_svc_deploy_mailbox_passport_flag_needs_mailbox() -> Result<(), Box<dyn Error>> {
    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.args(["svc", "deploy", "--help"]).assert().success().stdout(contains("--mailbox-passport"));

    let base = ["svc", "deploy", "--svc-id", "did:key:zTest", "--interfaces", "default", "--tcp"];
    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.args(base)
        .args(["localhost:1", "--mailbox-passport", "p.json"])
        .assert()
        .failure()
        .stderr(contains("--mailbox"));

    // With `--mailbox` the flag parses, so the deploy fails later, for the
    // missing substrate.
    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.args(base)
        .args(["localhost:1", "--mailbox", MAILBOX_HOST, "--mailbox-passport", "p.json"])
        .assert()
        .failure()
        .stderr(contains("Substrate DID not provided"));
    Ok(())
}

/// `app deploy --mailbox-passport` is listed, repeats, and needs
/// `--mint-masters`.
#[test]
fn test_app_deploy_mailbox_passport_flag_needs_mint_masters() -> Result<(), Box<dyn Error>> {
    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.args(["app", "deploy", "--help"]).assert().success().stdout(contains("--mailbox-passport"));

    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.args(["app", "deploy", "inst-1", "app.toml", "--mailbox-passport", "p.json"])
        .assert()
        .failure()
        .stderr(contains("--mint-masters"));

    // Parses and repeats with `--mint-masters`: the deploy then fails for
    // the missing `--as`, not for the flags.
    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.args([
        "app",
        "deploy",
        "inst-1",
        "app.toml",
        "--mint-masters",
        "--mailbox-passport",
        "a.json",
        "--mailbox-passport",
        "b.json",
    ])
    .assert()
    .failure()
    .stderr(contains("--mailbox-passport needs --as"));
    Ok(())
}
