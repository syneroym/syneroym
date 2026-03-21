use assert_cmd::assert::OutputAssertExt;
use assert_cmd::cargo::CommandCargoExt;
use std::process::Command;
use std::time::Duration;

#[test]
fn substrate_integration_stub() {
    let mut command = Command::cargo_bin("syneroym-substrate").unwrap();

    // The stub tests basic binary execution (version or help)
    command.arg("--help");
    command.assert().success();
}

#[tokio::test]
async fn test_run_finishes_on_ctrl_c() {
    let mut command = Command::cargo_bin("syneroym-substrate").unwrap();

    // Spawn the substrate process
    let mut child = command.arg("run").spawn().expect("Failed to spawn syneroym-substrate process");

    // Wait a bit for the process to initialize and start listening for signals
    tokio::time::sleep(Duration::from_millis(1000)).await;

    // Send SIGINT (Ctrl-C) to the child process
    let pid = child.id() as i32;
    unsafe {
        libc::kill(pid, libc::SIGINT);
    }

    // Wait for the process to exit
    let status = child.wait().expect("Failed to wait on child process");

    // Verify the process exited successfully (graceful shutdown)
    assert!(
        status.success(),
        "Process did not exit successfully after SIGINT, status: {:?}",
        status
    );
}
