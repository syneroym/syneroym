use anyhow::{Context, Result};
use std::path::PathBuf;
use syneroym_bindings::exports::syneroym::control_plane::orchestrator::{
    ArtifactSource, DeployManifest, ServiceType,
};
use syneroym_core::{config::SubstrateConfig, registry::SubstrateEndpoint};

/// Engine: Passive code module that wraps low-level OS operations
/// to spin up Wasmtime or Podman instances.
pub struct AppSandboxEngine {
    blobs_dir: PathBuf,
}

impl AppSandboxEngine {
    /// Initializes the App Sandbox and warms up any existing WASM endpoints
    pub async fn init(
        config: &SubstrateConfig,
        endpoints: Vec<(String, String, SubstrateEndpoint)>,
    ) -> anyhow::Result<Self> {
        let blobs_dir = config.storage.blobs_dir.clone();

        // Ensure blobs directory exists
        if !blobs_dir.exists() {
            tokio::fs::create_dir_all(&blobs_dir).await?;
        }

        let engine = Self { blobs_dir };

        for (service_id, _interface_name, endpoint) in endpoints {
            if let SubstrateEndpoint::WasmChannel { channel_details: channel_id } = endpoint {
                tracing::info!(
                    service_id = %service_id,
                    channel_id = %channel_id,
                    "Warming up WASM component"
                );

                // Perform your engine's warmup routine here
                // engine.load_and_warmup(&service_id, &channel_id).await?;
            }
        }

        Ok(engine)
    }

    /// Spin up a new Wasmtime instance
    pub async fn deploy_wasm(&self, service_id: &str, manifest: &DeployManifest) -> Result<()> {
        tracing::info!("AppSandboxEngine: Deploying Wasm component for {}", service_id);

        let wasm_manifest = match &manifest.service_type {
            ServiceType::Wasm(w) => w,
            _ => return Err(anyhow::anyhow!("Expected Wasm manifest")),
        };

        // 1. Fetch bytes
        let bytes = match &wasm_manifest.source {
            ArtifactSource::Url(url) => {
                tracing::info!("Fetching WASM from URL: {}", url);
                reqwest::get(url)
                    .await
                    .context("Failed to fetch WASM from URL")?
                    .bytes()
                    .await
                    .context("Failed to read WASM bytes")?
                    .to_vec()
            }
            ArtifactSource::Binary(b) => b.clone(),
        };

        // 2. Verify hash
        if let Some(expected_hash) = &wasm_manifest.hash {
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            let computed_hash = hex::encode(hasher.finalize());

            // Allow checking with or without standard 'sha256:' prefix from OCI registries
            let expected_hash_clean =
                expected_hash.strip_prefix("sha256:").unwrap_or(expected_hash);

            if computed_hash != *expected_hash_clean {
                return Err(anyhow::anyhow!(
                    "Hash mismatch: expected {}, got {}",
                    expected_hash_clean,
                    computed_hash
                ));
            }
            tracing::info!("WASM hash verified successfully");
        }

        // 3. Store locally in blobs_dir
        let file_path = self.blobs_dir.join(format!("{}.wasm", service_id));
        tokio::fs::write(&file_path, &bytes).await.context("Failed to save WASM binary locally")?;

        tracing::info!("WASM binary stored at {:?}", file_path);

        // 4. Register in Wasmtime (stubbed for now since no Wasmtime engine in AppSandboxEngine)
        // ...

        Ok(())
    }

    /// Stop a running Wasm component
    pub async fn stop_wasm(&self, _service_id: &str) -> Result<()> {
        tracing::info!("AppSandboxEngine: Stopping Wasm component for {}", _service_id);
        Ok(())
    }

    /// Remove a stopped Wasm component
    pub async fn remove_wasm(&self, _service_id: &str) -> Result<()> {
        tracing::info!("AppSandboxEngine: Removing Wasm component for {}", _service_id);
        Ok(())
    }

    /// Spin up a new Podman instance
    pub async fn deploy_podman(&self, _service_id: &str, _manifest: &[u8]) -> Result<()> {
        tracing::info!("AppSandboxEngine: Deploying Podman container for {}", _service_id);
        Ok(())
    }
}
