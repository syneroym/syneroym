//! Integration tests for Roymctl CLI argument parser
//!
//! Verifies correct routing and option validations for roymctl subcommands.

use std::error::Error;

use assert_cmd::Command;
use predicates::str::contains;

// TODO: Expand CLI argument parsing tests.
// Consider adding unit tests for argument permutations (e.g. mutually exclusive
// args like --wasm/--tcp), testing invalid arguments, and ensuring proper error
// messages are propagated.
#[test]
fn test_cli_parsing() -> Result<(), Box<dyn Error>> {
    // 1. Check node status help
    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.arg("node").arg("status").arg("--help").assert().success();

    // 1b. Check substrate status help
    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.arg("substrate").arg("status").arg("--help").assert().success();

    // 2. Check app deploy help
    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.arg("app").arg("deploy").arg("--help").assert().success();

    Ok(())
}

/// M04A Slice B7b: `identity issue-grant --help` parses (the subcommand and
/// all its flags are wired into clap).
#[test]
fn test_identity_issue_grant_help() -> Result<(), Box<dyn Error>> {
    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.arg("identity")
        .arg("issue-grant")
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("--from"))
        .stdout(contains("--to"))
        .stdout(contains("--can"))
        .stdout(contains("--with"))
        .stdout(contains("--expires-days"))
        .stdout(contains("--no-delegate"));
    Ok(())
}

/// M04A Slice B7b: the global `--ucan <path>` flag parses alongside an
/// existing subcommand.
#[test]
fn test_global_ucan_flag_parses() -> Result<(), Box<dyn Error>> {
    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.arg("--ucan")
        .arg("some-token.json")
        .arg("svc")
        .arg("list")
        .arg("--help")
        .assert()
        .success();
    Ok(())
}

/// M04A Slice B7b end to end: `identity create` then `identity issue-grant`
/// produces a signed `CapabilityToken` JSON naming exactly the requested
/// `with`/`can`/`to`/`can_delegate`.
#[test]
fn test_identity_issue_grant_produces_a_signed_token() -> Result<(), Box<dyn Error>> {
    let temp_dir = tempfile::tempdir()?;

    let mut create_cmd = Command::cargo_bin("roymctl")?;
    create_cmd
        .arg("--dir")
        .arg(temp_dir.path())
        .arg("identity")
        .arg("create")
        .arg("--name")
        .arg("owner")
        .assert()
        .success();

    let mut grant_cmd = Command::cargo_bin("roymctl")?;
    let output = grant_cmd
        .arg("--dir")
        .arg(temp_dir.path())
        .arg("identity")
        .arg("issue-grant")
        .arg("--from")
        .arg("owner")
        .arg("--to")
        .arg("did:key:zGrantee")
        .arg("--can")
        .arg("orchestrator/deploy")
        .arg("--with")
        .arg("substrate:did:key:zNode/app/*")
        .arg("--expires-days")
        .arg("30")
        .arg("--no-delegate")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let token: serde_json::Value = serde_json::from_slice(&output)?;
    assert_eq!(token["audience_did"], "did:key:zGrantee");
    assert_eq!(token["capabilities"][0]["with"], "substrate:did:key:zNode/app/*");
    assert_eq!(token["capabilities"][0]["can"], "orchestrator/deploy");
    assert_eq!(token["capabilities"][0]["caveats"]["can_delegate"], false);
    assert!(token["signature"].as_str().is_some_and(|s| !s.is_empty()));
    Ok(())
}
