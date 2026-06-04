use std::{
    env,
    path::PathBuf,
    process::Stdio,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use reqwest::Client;
use syneroym_identity::{Identity, substrate};
use tempfile::NamedTempFile;
use tokio::{
    process::{Child, Command},
    time,
};
use tracing::info;

fn get_cargo_bin(name: &str) -> PathBuf {
    if let Some(path) = std::env::var_os(format!("CARGO_BIN_EXE_{}", name)) {
        PathBuf::from(path)
    } else {
        // Fall back to target/debug or target/release based on current executable parent
        if let Ok(current_exe) = env::current_exe()
            && let Some(parent) = current_exe.parent()
        {
            let bin_path = parent.join(name);
            if bin_path.exists() {
                return bin_path;
            }
        }
        // Fall back to workspace target/debug or target/release based on compilation profile
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
        let workspace_target =
            std::path::Path::new(&manifest_dir).parent().unwrap().parent().unwrap().join("target");

        let primary_profile = if cfg!(debug_assertions) { "debug" } else { "release" };
        let secondary_profile = if cfg!(debug_assertions) { "release" } else { "debug" };

        let primary_path = workspace_target.join(primary_profile).join(name);
        if primary_path.exists() {
            primary_path
        } else {
            let secondary_path = workspace_target.join(secondary_profile).join(name);
            if secondary_path.exists() { secondary_path } else { PathBuf::from(name) }
        }
    }
}

pub struct TestEnvironment {
    substrate: Option<Child>,
    miniapp: Option<Child>,
    http_client: Client,
    pub substrate_identity: Identity,
    pub substrate_did: String,
    _key_file: NamedTempFile,
}

impl TestEnvironment {
    pub async fn new() -> Result<Self> {
        let substrate_identity = Identity::generate()?;
        let substrate_did = substrate::derive_did_key(&substrate_identity.public_key());
        let key_file = NamedTempFile::new()?;
        substrate_identity.save_to_path(key_file.path())?;

        Ok(Self {
            substrate: None,
            miniapp: None,
            http_client: Client::builder().timeout(Duration::from_secs(5)).build()?,
            substrate_identity,
            substrate_did,
            _key_file: key_file,
        })
    }

    pub async fn start_miniapp(&mut self, port: u16) -> Result<()> {
        info!("Starting miniapp-demo1-web on port {port}");
        let bin = get_cargo_bin("miniapp-demo1-web");

        // Use a temporary data dir for miniapp to avoid pollution
        let data_dir = tempfile::tempdir()?.keep();

        let child = Command::new(bin)
            .arg("--port")
            .arg(port.to_string())
            .arg("--data-dir")
            .arg(data_dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .context("Failed to start miniapp")?;

        self.miniapp = Some(child);
        // Wait for readiness
        let url = format!("http://127.0.0.1:{port}/");
        self.wait_for_http(&url, Duration::from_secs(10)).await?;

        Ok(())
    }

    pub async fn start_substrate(&mut self) -> Result<()> {
        info!("Starting syneroym-substrate");
        let bin = get_cargo_bin("syneroym-substrate"); // Might be `syneroym` depending on cargo setup. Let's assume syneroym-substrate based on Cargo.toml `name`.

        let child = Command::new(bin)
            .arg("run")
            .arg("--key")
            .arg(self._key_file.path())
            // Observability is on by default in dev_mode_config.
            // Using default dev_mode config.
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .context("Failed to start substrate")?;

        self.substrate = Some(child);

        // Wait for readiness - use community registry on 7961
        let url = "http://127.0.0.1:7961/lookup/probe";
        self.wait_for_http(url, Duration::from_secs(15)).await?;

        Ok(())
    }

    async fn wait_for_http(&mut self, url: &str, timeout: Duration) -> Result<()> {
        let start = Instant::now();
        while start.elapsed() < timeout {
            // Check if process crashed
            if let Some(child) = &mut self.substrate
                && let Ok(Some(status)) = child.try_wait()
            {
                return Err(anyhow::anyhow!("Substrate process exited early with status {status}"));
            }
            if let Some(child) = &mut self.miniapp
                && let Ok(Some(status)) = child.try_wait()
            {
                return Err(anyhow::anyhow!("Miniapp process exited early with status {status}"));
            }

            match self.http_client.get(url).send().await {
                Ok(resp)
                    if resp.status().is_success()
                        || resp.status() == reqwest::StatusCode::NOT_FOUND =>
                {
                    return Ok(());
                }
                _ => {
                    time::sleep(Duration::from_millis(200)).await;
                }
            }
        }
        Err(anyhow::anyhow!("Timeout waiting for {url} to become ready"))
    }

    pub async fn teardown(&mut self) {
        info!("Tearing down test environment");
        if let Some(mut child) = self.substrate.take() {
            let _ = child.kill().await;
        }
        if let Some(mut child) = self.miniapp.take() {
            let _ = child.kill().await;
        }
    }
}

impl Drop for TestEnvironment {
    fn drop(&mut self) {
        // Fallback sync kill if teardown was not called
        if let Some(mut child) = self.substrate.take() {
            let _ = child.start_kill();
        }
        if let Some(mut child) = self.miniapp.take() {
            let _ = child.start_kill();
        }
    }
}
