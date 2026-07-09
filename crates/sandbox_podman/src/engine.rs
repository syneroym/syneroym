#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Podman container execution engine
//!
//! Handles lifecycle of Podman containers using std::process::Command.

use std::{
    fs,
    path::{Component, Path, PathBuf},
    process::Command,
    sync::Arc,
};

use anyhow::{Context, Result, anyhow};
use syneroym_data_db::traits::StorageProvider;
use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
    DeployManifest, ServiceType,
};
use tracing::{info, warn};

#[derive(Clone)]
pub struct ContainerEngine {
    podman_path: String,
    containers_dir: PathBuf,
    storage_provider: Option<Arc<dyn StorageProvider>>,
}

impl std::fmt::Debug for ContainerEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContainerEngine")
            .field("podman_path", &self.podman_path)
            .field("containers_dir", &self.containers_dir)
            .finish_non_exhaustive()
    }
}

fn sanitize_id(id: &str) -> String {
    id.replace(':', "_")
}

impl ContainerEngine {
    pub fn new(
        podman_path: String,
        app_local_data_dir: &Path,
        storage_provider: Option<Arc<dyn StorageProvider>>,
    ) -> Self {
        let containers_dir = app_local_data_dir.join("containers");
        Self { podman_path, containers_dir, storage_provider }
    }

    /// Safely resolve host path relative to the container's isolated local
    /// directory.
    fn resolve_host_path(&self, service_id: &str, host_path: &str) -> PathBuf {
        let container_base = self.containers_dir.join(sanitize_id(service_id));

        // Clean and prevent directory traversal
        let mut path = container_base.clone();
        for component in Path::new(host_path).components() {
            match component {
                Component::Normal(c) => path.push(c),
                Component::ParentDir if path != container_base => {
                    path.pop();
                }
                _ => {}
            }
        }
        path
    }

    /// Deploy a container based on the manifest
    pub async fn deploy(
        &self,
        service_id: &str,
        manifest: &DeployManifest,
    ) -> Result<Vec<(String, u16)>> {
        info!(service_id = %service_id, "ContainerEngine: Deploying Podman container");

        let ServiceType::Container(ref container_manifest) = manifest.service_type else {
            return Err(anyhow!("Expected container manifest"));
        };

        let sanitized_id = sanitize_id(service_id);

        // 1. Volumes setup
        let mut volume_args = Vec::new();
        for vol in &container_manifest.volumes {
            let host_path = self.resolve_host_path(service_id, &vol.host_path);
            fs::create_dir_all(&host_path)
                .with_context(|| format!("Failed to create host path directory {:?}", host_path))?;

            volume_args.push("-v".to_string());
            volume_args.push(format!("{}:{}", host_path.display(), vol.container_path));
        }

        // 2. Ports mapping
        let mut port_args = Vec::new();
        for port_map in &container_manifest.ports {
            let protocol = if port_map.protocol.is_empty() { "tcp" } else { &port_map.protocol };
            if let Some(host_port) = port_map.host_port {
                port_args.push("-p".to_string());
                port_args.push(format!("{}:{}/{}", host_port, port_map.container_port, protocol));
            } else {
                port_args.push("-p".to_string());
                port_args.push(format!("{}/{}", port_map.container_port, protocol));
            }
        }

        // 3. Environment variables
        let mut env_args = Vec::new();

        let mut config_map = std::collections::BTreeMap::new();
        #[allow(clippy::collapsible_if)]
        if let Some(sp) = &self.storage_provider {
            match sp.get_latest_config_generation(service_id).await {
                Ok(Some((_, blob))) => {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&blob) {
                        if let Some(map) = json.as_object() {
                            for (k, v) in map {
                                if let Some(s) = v.as_str() {
                                    config_map.insert(k.clone(), s.to_string());
                                }
                            }
                        }
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::error!("Failed to fetch config generation for {}: {}", service_id, e);
                }
            }
        }

        // Merge manifest env with config map
        for (k, v) in &manifest.config.env {
            config_map.insert(k.clone(), v.clone());
        }

        for (key, val) in &config_map {
            env_args.push("-e".to_string());
            env_args.push(format!("{}={}", key, val));
        }

        // 4. Command arguments
        let mut run_args = vec![
            "run".to_string(),
            "-d".to_string(),
            "--name".to_string(),
            sanitized_id.clone(),
            "--network".to_string(),
            "bridge".to_string(),
        ];

        run_args.extend(volume_args);
        run_args.extend(port_args);
        run_args.extend(env_args);
        run_args.push(container_manifest.image.clone());

        // Append args if provided
        for arg in &manifest.config.args {
            run_args.push(arg.clone());
        }

        info!(service_id = %service_id, args = ?run_args, "Running podman command");

        let output = Command::new(&self.podman_path)
            .args(&run_args)
            .output()
            .context("Failed to execute podman command")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("podman run failed: {}", stderr));
        }

        // 5. Query and map dynamic ports or verify running ports
        let mut actual_mappings = Vec::new();
        for port_map in &container_manifest.ports {
            let resolved_port = if let Some(host_port) = port_map.host_port {
                host_port
            } else {
                self.query_port(service_id, port_map.container_port, &port_map.protocol)
                    .await
                    .context("Failed to query dynamically allocated port from podman")?
            };
            actual_mappings.push((port_map.interface_name.clone(), resolved_port));
        }

        Ok(actual_mappings)
    }

    /// Query the assigned port via `podman port` after creation
    pub async fn query_port(
        &self,
        service_id: &str,
        container_port: u16,
        protocol: &str,
    ) -> Result<u16> {
        let protocol = if protocol.is_empty() { "tcp" } else { protocol };
        let sanitized_id = sanitize_id(service_id);
        let output = Command::new(&self.podman_path)
            .args(["port", &sanitized_id, &format!("{}/{}", container_port, protocol)])
            .output()
            .context("Failed to execute podman port")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("podman port failed: {}", stderr));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let line = stdout.trim();
        // The output is typically something like "0.0.0.0:49153" or "[::]:49153" or
        // "49153"
        let port_part = line
            .split(':')
            .next_back()
            .ok_or_else(|| anyhow!("Invalid output from podman port: {}", line))?;

        let parsed_port = port_part.trim().parse::<u16>().with_context(|| {
            format!("Failed to parse port '{}' from podman port output", port_part)
        })?;

        Ok(parsed_port)
    }

    /// Stop the container
    pub async fn stop(&self, service_id: &str) -> Result<()> {
        info!(service_id = %service_id, "ContainerEngine: Stopping Podman container");

        let sanitized_id = sanitize_id(service_id);
        let output = Command::new(&self.podman_path)
            .args(["stop", &sanitized_id])
            .output()
            .context("Failed to execute podman stop")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // If the container doesn't exist or is already stopped, we might want to log it
            warn!("podman stop failed (might already be stopped): {}", stderr);
        }

        Ok(())
    }

    /// Completely remove a stopped container
    pub async fn remove(&self, service_id: &str) -> Result<()> {
        info!(service_id = %service_id, "ContainerEngine: Removing Podman container");

        let sanitized_id = sanitize_id(service_id);
        let output = Command::new(&self.podman_path)
            .args(["rm", &sanitized_id])
            .output()
            .context("Failed to execute podman rm")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("podman rm failed (might already be removed): {}", stderr);
        }

        // Clean up the sandboxed volumes directory
        let container_dir = self.containers_dir.join(&sanitized_id);
        if container_dir.exists() {
            let _ = fs::remove_dir_all(&container_dir);
        }

        Ok(())
    }

    /// Check the readiness by running `podman inspect` to verify container is
    /// running
    pub async fn readyz(&self, service_id: &str) -> Result<()> {
        let sanitized_id = sanitize_id(service_id);
        let output = Command::new(&self.podman_path)
            .args(["inspect", "--format", "{{.State.Running}}", &sanitized_id])
            .output()
            .context("Failed to execute podman inspect")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("podman inspect failed: {}", stderr));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.trim() == "true" {
            Ok(())
        } else {
            Err(anyhow!(
                "Container {} is not running. State.Running = {}",
                service_id,
                stdout.trim()
            ))
        }
    }
}
