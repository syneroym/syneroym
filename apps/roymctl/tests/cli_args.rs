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

/// Post-commit review (F2): `--ucan` without `--as` is a silent no-op at the
/// protocol level (the presented token's audience can never match a fresh
/// ephemeral per-invocation identity), so clap rejects the combination
/// up front with a clear message instead of letting the caller hit a
/// confusing downstream "holds no grant" failure.
#[test]
fn test_ucan_without_as_is_rejected() -> Result<(), Box<dyn Error>> {
    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.arg("--ucan")
        .arg("some-token.json")
        .arg("svc")
        .arg("list")
        .assert()
        .failure()
        .stderr(contains("--as"));
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

/// `identity certify-instance --help` parses (the subcommand and all its
/// flags are wired into clap).
#[test]
fn test_identity_certify_instance_help() -> Result<(), Box<dyn Error>> {
    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.arg("identity")
        .arg("certify-instance")
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("--master"))
        .stdout(contains("--substrate"))
        .stdout(contains("--expires-hours"));
    Ok(())
}

/// `svc deploy --master` parses alongside the existing deploy flags.
#[test]
fn test_svc_deploy_master_flag_parses() -> Result<(), Box<dyn Error>> {
    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.arg("svc").arg("deploy").arg("--help").assert().success().stdout(contains("--master"));
    Ok(())
}

/// `app deploy --mint-masters` parses alongside the existing deploy flags.
#[test]
fn test_app_deploy_mint_masters_flag_parses() -> Result<(), Box<dyn Error>> {
    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.arg("app")
        .arg("deploy")
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("--mint-masters"));
    Ok(())
}

/// M05A Slice A3: `app deploy --help` lists the new `--inventory` flag.
#[test]
fn app_deploy_help_lists_inventory() -> Result<(), Box<dyn Error>> {
    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.arg("app").arg("deploy").arg("--help").assert().success().stdout(contains("--inventory"));
    Ok(())
}

/// The supervisor is the scheduler, and `app deploy` has no supervisor
/// behind it -- so a schedule deployed this way validates, deploys, and
/// then never runs. The warning is the only thing standing between an
/// operator and a silently dead schedule, which is why it is pinned here
/// and not just its predicate: the run must keep going past it (the error
/// below is the *next* step failing for want of a substrate, not this
/// check refusing), so a warning quietly turned into a refusal fails too.
#[test]
fn app_deploy_warns_when_the_plan_carries_a_schedule_and_still_deploys()
-> Result<(), Box<dyn Error>> {
    let temp_dir = tempfile::tempdir()?;
    let manifest_path = temp_dir.path().join("app.toml");
    std::fs::write(
        &manifest_path,
        r#"
id = "syneroym:guild-app"
version = "0.1.0"

[services.worker]
service_type = "wasm"
source = "unused"
interfaces = ["scheduled-driver"]

[services.worker.schedule]
cron = "* * * * *"
interface = "scheduled-driver"
method = "tick"
"#,
    )?;

    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.arg("--dir")
        .arg(temp_dir.path())
        .arg("app")
        .arg("deploy")
        .arg("inst-1")
        .arg(&manifest_path)
        .arg("--journal-path")
        .arg(temp_dir.path().join("deployments.db"))
        .assert()
        .stderr(contains("declares a schedule"))
        .stderr(contains("supervisor submit"))
        .stderr(contains("Substrate DID not provided"));
    Ok(())
}

/// A manifest with `[placement]` naming an alias the inventory does not
/// define fails naming both the inventory file and the missing alias --
/// before any deploy call, since `check_placement` runs ahead of the
/// per-substrate preflight. An empty-but-present inventory file exercises
/// this with no live substrate needed.
#[test]
fn app_deploy_with_placement_and_no_matching_inventory_entry_names_the_path_and_alias()
-> Result<(), Box<dyn Error>> {
    let temp_dir = tempfile::tempdir()?;
    let manifest_path = temp_dir.path().join("app.toml");
    std::fs::write(
        &manifest_path,
        r#"
            id = "syneroym:a3-test-app"
            version = "0.1.0"

            [services.backend]
            service_type = "tcp"
            source = "127.0.0.1:9000"

            [services.backend.placement]
            substrate = "edge-1"
        "#,
    )?;
    // Present, but defines no substrates -- `check_placement` fails with no
    // network access at all.
    let inventory_path = temp_dir.path().join("substrates.toml");
    std::fs::write(&inventory_path, "")?;

    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.arg("--dir")
        .arg(temp_dir.path())
        .arg("app")
        .arg("deploy")
        .arg("a3-test-inst")
        .arg(&manifest_path)
        .arg("--journal-path")
        .arg(temp_dir.path().join("deployments.db"))
        .assert()
        .failure()
        .stderr(contains("edge-1"))
        .stderr(contains(inventory_path.to_string_lossy().to_string()));
    Ok(())
}

/// D-A3-20: a fully-placed app (every service named by alias) must not
/// require `--substrate`/`substrate.key`, since it never touches the
/// fallback target. The inventory entry names an address nothing listens
/// on, so the run still fails -- but on unreachability, never on the
/// `substrate.key not found` message `get_substrate_did` would raise if it
/// were called eagerly.
#[test]
fn app_deploy_fully_placed_does_not_require_a_substrate_key() -> Result<(), Box<dyn Error>> {
    let temp_dir = tempfile::tempdir()?;
    let manifest_path = temp_dir.path().join("app.toml");
    std::fs::write(
        &manifest_path,
        r#"
            id = "syneroym:a3-test-app"
            version = "0.1.0"

            [services.backend]
            service_type = "tcp"
            source = "127.0.0.1:9000"

            [services.backend.placement]
            substrate = "edge-1"
        "#,
    )?;
    let inventory_path = temp_dir.path().join("substrates.toml");
    std::fs::write(
        &inventory_path,
        r#"
            [substrates.edge-1]
            did = "did:key:z6MkExampleNodeA"
            api_url = "http://127.0.0.1:1"
        "#,
    )?;

    let mut cmd = Command::cargo_bin("roymctl")?;
    let output = cmd
        .arg("--dir")
        .arg(temp_dir.path())
        .arg("app")
        .arg("deploy")
        .arg("a3-test-inst")
        .arg(&manifest_path)
        .arg("--journal-path")
        .arg(temp_dir.path().join("deployments.db"))
        .assert()
        .failure()
        .get_output()
        .stderr
        .clone();
    let stderr = String::from_utf8_lossy(&output);
    assert!(!stderr.contains("substrate.key not found"), "{stderr}");
    Ok(())
}

/// `svc sagas --help` parses (the subcommand and its flag are wired into
/// clap).
#[test]
fn test_svc_sagas_help() -> Result<(), Box<dyn Error>> {
    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.arg("svc").arg("sagas").arg("--help").assert().success().stdout(contains("--svc-id"));
    Ok(())
}

/// `svc saga-compensate --help` parses.
#[test]
fn test_svc_saga_compensate_help() -> Result<(), Box<dyn Error>> {
    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.arg("svc")
        .arg("saga-compensate")
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("--svc-id"))
        .stdout(contains("--saga-id"));
    Ok(())
}

/// Test 97 (M05C S3): `roymctl alias --service` prints the app-scoped
/// host form, and without `--interface` prints the same host with no
/// `-i` segment.
#[test]
fn roymctl_alias_with_a_service_prints_the_app_scoped_form() -> Result<(), Box<dyn Error>> {
    let app_did = "did:key:h7wyixfzo3km6k8uq98mcini8q67pxs1jkf1ymnrmrogesimteapsufe";

    let mut cmd = Command::cargo_bin("roymctl")?;
    let output = cmd
        .arg("alias")
        .arg(app_did)
        .arg("--nickname")
        .arg("my-chat-app")
        .arg("--service")
        .arg("backend")
        .arg("--interface")
        .arg("default")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let host = String::from_utf8_lossy(&output);
    assert!(host.contains("my-chat-app-a"), "{host}");
    assert!(host.contains("-s"), "{host}");
    assert!(host.contains("-i"), "{host}");
    assert!(host.trim_end().ends_with(".localhost"), "{host}");

    let mut cmd_no_iface = Command::cargo_bin("roymctl")?;
    let output_no_iface = cmd_no_iface
        .arg("alias")
        .arg(app_did)
        .arg("--nickname")
        .arg("my-chat-app")
        .arg("--service")
        .arg("backend")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let host_no_iface = String::from_utf8_lossy(&output_no_iface);
    assert!(!host_no_iface.contains("-i"), "{host_no_iface}");
    assert!(host_no_iface.trim_end().ends_with(".localhost"), "{host_no_iface}");

    Ok(())
}

/// Test 98: the `AppInstanceId` requirement D-S3-2 rests on -- `--service`
/// without `--nickname` is refused at the clap level.
#[test]
fn roymctl_alias_with_a_service_and_no_nickname_is_refused() -> Result<(), Box<dyn Error>> {
    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.arg("alias")
        .arg("did:key:h7wyixfzo3km6k8uq98mcini8q67pxs1jkf1ymnrmrogesimteapsufe")
        .arg("--service")
        .arg("backend")
        .assert()
        .failure()
        .stderr(contains("--nickname"));
    Ok(())
}

/// `identity certify-signing --help` parses flags (--master,
/// --service, --hours).
#[test]
fn identity_certify_signing_help() -> Result<(), Box<dyn Error>> {
    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.arg("identity")
        .arg("certify-signing")
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("--master"))
        .stdout(contains("--service"))
        .stdout(contains("--expires-hours"));
    Ok(())
}
