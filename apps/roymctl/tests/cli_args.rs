//! Integration tests for Roymctl CLI argument parser
//!
//! Verifies correct routing and option validations for roymctl subcommands.

use assert_cmd::Command;

// TODO: Expand CLI argument parsing tests.
// Consider adding unit tests for argument permutations (e.g. mutually exclusive
// args like --wasm/--tcp), testing invalid arguments, and ensuring proper error
// messages are propagated.
#[test]
fn test_cli_parsing() -> Result<(), Box<dyn std::error::Error>> {
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
