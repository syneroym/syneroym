use anyhow::{Context, Result};
use assert_cmd::cargo::cargo_bin;
use reqwest::Client;
use std::process::Stdio;
use std::time::Duration;
use syneroym_identity::Identity;
use tokio::process::{Child, Command};
use tracing::info;

pub struct TestEnvironment {
    substrate: Option<Child>,
    miniapp: Option<Child>,
    http_client: Client,
    pub substrate_identity: Identity,
    pub substrate_did: String,
    _key_file: tempfile::NamedTempFile,
}

impl TestEnvironment {
    pub async fn new() -> Result<Self> {
        let substrate_identity = Identity::generate()?;
        let substrate_did =
            syneroym_identity::substrate::derive_did_key(&substrate_identity.public_key());
        let key_file = tempfile::NamedTempFile::new()?;
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
        let bin = cargo_bin("miniapp-demo1-web");

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
        let bin = cargo_bin("syneroym-substrate"); // Might be `syneroym` depending on cargo setup. Let's assume syneroym-substrate based on Cargo.toml `name`.

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
        let start = std::time::Instant::now();
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
                    tokio::time::sleep(Duration::from_millis(200)).await;
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
