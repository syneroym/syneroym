use std::error::Error;

use assert_cmd::Command;
use predicates::str::contains;

#[test]
fn roym_transaction_statement_and_close_subcommands_help() -> Result<(), Box<dyn Error>> {
    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.arg("roym")
        .arg("transaction")
        .arg("agreement")
        .arg("close")
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("--agreement"))
        .stdout(contains("--close-as"))
        .stdout(contains("--commitment"));

    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.arg("roym")
        .arg("transaction")
        .arg("eventlog")
        .arg("list")
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("--after-secs"))
        .stdout(contains("--after-id"))
        .stdout(contains("--limit"));

    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.arg("roym")
        .arg("transaction")
        .arg("eventlog")
        .arg("rebuild")
        .arg("--help")
        .assert()
        .success();

    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.arg("roym")
        .arg("transaction")
        .arg("statement")
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("--from"))
        .stdout(contains("--to"));

    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.arg("roym").arg("transaction").arg("defaults").arg("get").arg("--help").assert().success();

    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.arg("roym")
        .arg("transaction")
        .arg("defaults")
        .arg("set")
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("--cancellation"))
        .stdout(contains("--refund"))
        .stdout(contains("--dispute"))
        .stdout(contains("--lang"))
        .stdout(contains("--quote-valid-hours"));

    Ok(())
}
