use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

#[test]
fn test_node_initialization_and_status() -> Result<(), Box<dyn std::error::Error>> {
    // Create a temporary directory that will be cleaned up automatically
    let temp_dir = TempDir::new()?;
    let dir_path = temp_dir.path().to_str().unwrap();

    // 1. Initialize the node
    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.arg("node")
        .arg("init")
        .arg("--dir")
        .arg(dir_path)
        .assert()
        .success()
        .stdout(predicate::str::contains("Initialized node successfully"));

    // 2. Start the node in background
    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.arg("node")
        .arg("start")
        .arg("--dir")
        .arg(dir_path)
        .arg("--detach")
        .assert()
        .success()
        .stdout(predicate::str::contains("Node started"));

    // 3. Check status
    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.arg("status")
        .arg("--dir")
        .arg(dir_path)
        .assert()
        .success()
        .stdout(predicate::str::contains("is_online: true"));

    // 4. Stop the node
    let mut cmd = Command::cargo_bin("roymctl")?;
    cmd.arg("node")
        .arg("stop")
        .arg("--dir")
        .arg(dir_path)
        .assert()
        .success()
        .stdout(predicate::str::contains("Node stopped cleanly"));

    Ok(())
}
