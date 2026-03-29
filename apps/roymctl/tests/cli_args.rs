use assert_cmd::Command;

#[test]
fn test_cli_parsing() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Check status
    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.arg("node").arg("status").assert().success();

    // 2. Deploy an App
    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.arg("app")
        .arg("deploy")
        .arg("--app-id")
        .arg("test-app")
        .arg("--manifest")
        .arg("App.toml")
        .assert()
        .success();

    // 3. Connect to a Peer
    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.arg("peer").arg("connect").arg("--peer-id").arg("dummy-peer").assert().success();

    Ok(())
}
