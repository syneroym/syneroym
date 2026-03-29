use assert_cmd::assert::OutputAssertExt;
use assert_cmd::cargo::CommandCargoExt;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::Duration;
use syneroym_core::config::{ClientGatewayRole, SubstrateConfig};
use tempfile::NamedTempFile;
use tokio::time::sleep;

fn send_ctrl_c(#[allow(unused_variables)] pid: u32) {
    #[cfg(unix)]
    {
        unsafe {
            libc::kill(pid as i32, libc::SIGINT);
        }
    }
    #[cfg(windows)]
    {
        // On Windows, there is no direct equivalent to sending SIGINT to a specific PID.
        #[link(name = "kernel32")]
        extern "system" {
            fn GenerateConsoleCtrlEvent(dwCtrlEvent: u32, dwProcessGroupId: u32) -> i32;
        }
        unsafe {
            // 0 = CTRL_C_EVENT
            GenerateConsoleCtrlEvent(0, 0);
        }
    }
}

#[test]
// The stub tests basic binary execution (version or help)
fn substrate_integration_stub() {
    let mut command = Command::cargo_bin("syneroym-substrate").unwrap();

    // The stub tests basic binary execution (version or help)
    command.arg("--help");
    command.assert().success();
}

#[tokio::test]
// Test basic binary execution and graceful shutdown on Ctrl-C.
// This is a black-box test that runs the binary as a subprocess.
async fn test_run_finishes_on_ctrl_c() {
    // Create a temporary config file to explicitly enable the client_gateway role
    let mut config_file = NamedTempFile::new().expect("Failed to create temp config file");
    let config_toml = r#"
    [roles.client_gateway]
    "#;
    write!(config_file, "{}", config_toml).expect("Failed to write to temp config file");

    let mut command = Command::cargo_bin("syneroym-substrate").unwrap();

    // Spawn the substrate process with piped stdout
    let mut child = command
        .arg("run")
        .arg("--config")
        .arg(config_file.path())
        .stdout(Stdio::piped())
        .spawn()
        .expect("Failed to spawn syneroym-substrate process");

    // Wait for the process to initialize and start running by reading its stdout
    let stdout = child.stdout.take().expect("Failed to capture stdout");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    loop {
        line.clear();
        let bytes_read = reader.read_line(&mut line).expect("Failed to read from child stdout");
        if bytes_read == 0 {
            panic!("Process closed stdout before reaching running state");
        }
        // Look for a reliable log line indicating the the process is running with reasonable confidence. Enough for sanity purpose.
        if line.contains("running client gateway") {
            break;
        }
    }

    // Give the process a brief moment to ensure signal handler registration completes
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Send SIGINT (Ctrl-C) to the child process
    send_ctrl_c(child.id());

    // Wait for the process to exit
    let status = child.wait().expect("Failed to wait on child process");

    // Verify the process exited successfully (graceful shutdown)
    assert!(
        status.success(),
        "Process did not exit successfully after SIGINT, status: {:?}",
        status
    );
}

/// This in-process integration test starts a substrate,
/// runs operations done over a typical substrate lifetime, finally shutting it down
#[tokio::test]
async fn test_in_process_lifecycle_shutdown_on_ctrl_c() {
    // Construct the configuration programmatically.
    let mut config = SubstrateConfig::default();
    config.roles.client_gateway = Some(ClientGatewayRole::default());
    config.profile = "test-lifecycle".to_string();

    // Spawn the entire substrate in a background task.
    let mut substrate_handle = tokio::spawn(async move {
        syneroym_substrate::run(config).await.expect("Substrate failed to run");
    });

    // Give the substrate a moment to start up its components before we check for availability.
    sleep(Duration::from_millis(500)).await;
    abort_if_failed(
        wait_for_substrate(),
        &mut substrate_handle,
        "Substrate did not become available within 1 second",
    )
    .await;

    // Simulate a Ctrl-C (SIGINT) to trigger graceful shutdown.
    send_ctrl_c(std::process::id());

    // Await the substrate handle to ensure it shuts down cleanly.
    let result = substrate_handle.await;
    assert!(result.is_ok(), "Substrate task should shut down cleanly without panicking.");
}

async fn check_substrate_available() -> bool {
    // TODO: Implement actual availability check (e.g., check an HTTP endpoint or native service)
    true
}

async fn wait_for_substrate() -> bool {
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if check_substrate_available().await {
                return;
            }
            sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .is_ok()
}

async fn abort_if_failed<F>(step: F, handle: &mut tokio::task::JoinHandle<()>, msg: &str)
where
    F: std::future::Future<Output = bool>,
{
    if !step.await {
        abort_substrate(handle, msg).await;
    }
}

async fn abort_substrate(handle: &mut tokio::task::JoinHandle<()>, msg: &str) {
    send_ctrl_c(std::process::id());
    let _ = handle.await;
    panic!("{}", msg);
}
