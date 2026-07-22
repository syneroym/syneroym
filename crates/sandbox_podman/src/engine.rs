#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Podman container execution engine
//!
//! Handles lifecycle of Podman containers using std::process::Command.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    path::{Component, Path, PathBuf},
    process::Command,
    sync::Arc,
};

use anyhow::{Context, Result, anyhow};
use serde_json::Value;
use syneroym_core::deploy_docs;
use syneroym_data_db::traits::StorageProvider;
use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
    ContainerVolumeFile, DeployManifest, DocumentSource, ServiceType,
};
use tracing::{error, info, warn};

#[derive(Clone)]
pub struct ContainerEngine {
    podman_path: String,
    containers_dir: PathBuf,
    storage_provider: Option<Arc<dyn StorageProvider>>,
}

impl fmt::Debug for ContainerEngine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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

    /// Resolves a volume file's content. `Inline` arrives with the deploy call
    /// itself; `Path` is read from the substrate host's own filesystem, under
    /// the traversal and size guards shared with the control plane.
    fn resolve_document(source: &DocumentSource, field_name: &str) -> Result<String> {
        match source {
            DocumentSource::Inline(content) => {
                deploy_docs::check_inline_size(content, field_name).map_err(|e| anyhow!(e))?;
                Ok(content.clone())
            }
            DocumentSource::Path(path) => {
                deploy_docs::read_host_document(Path::new(path), field_name).map_err(|e| anyhow!(e))
            }
        }
    }

    /// A volume carrying manifest-supplied files is configuration the
    /// substrate owns, so it is mounted read-only. A volume without them keeps
    /// the original behavior: an empty writable directory the container may
    /// use as it likes.
    fn volume_mount_arg(host_path: &Path, container_path: &str, has_files: bool) -> String {
        let mode = if has_files { ":ro" } else { "" };
        format!("{}:{}{}", host_path.display(), container_path, mode)
    }

    /// Writes a volume's manifest-supplied files beneath `volume_root` and
    /// removes anything a previous deploy left there that this one no longer
    /// declares, so the mount is a function of the current manifest rather
    /// than of deploy history.
    fn materialize_volume_files(volume_root: &Path, files: &[ContainerVolumeFile]) -> Result<()> {
        let canonical_root = fs::canonicalize(volume_root).with_context(|| {
            format!("Failed to resolve container volume directory {:?}", volume_root)
        })?;
        let mut total: u64 = 0;
        let mut written = BTreeSet::new();

        for file in files {
            deploy_docs::reject_relative_escape(&file.relative_path, "volume file relative-path")
                .map_err(|e| anyhow!(e))?;
            let content = Self::resolve_document(&file.content, "volume file")?;

            total += content.len() as u64;
            if total > deploy_docs::MAX_VOLUME_TOTAL_BYTES {
                return Err(anyhow!(
                    "container volume files exceed the {} byte total limit",
                    deploy_docs::MAX_VOLUME_TOTAL_BYTES
                ));
            }

            let target = canonical_root.join(&file.relative_path);
            let parent = target
                .parent()
                .ok_or_else(|| anyhow!("volume file {:?} has no parent", file.relative_path))?;
            fs::create_dir_all(parent)?;

            // `reject_relative_escape` is purely lexical, so it cannot see a
            // symlink an earlier deploy left under the volume root that
            // redirects this write outside it. Canonicalizing the parent after
            // creating it closes exactly that gap.
            let canonical_parent = fs::canonicalize(parent)?;
            if !canonical_parent.starts_with(&canonical_root) {
                return Err(anyhow!(
                    "volume file {:?} resolves outside the volume directory",
                    file.relative_path
                ));
            }

            fs::write(&target, content)
                .with_context(|| format!("Failed to write volume file {:?}", target))?;
            written.insert(target);
        }

        Self::remove_stale_volume_files(&canonical_root, &written, 0)
    }

    /// Depth is bounded because `relative-path` permits arbitrary nesting and
    /// this walk runs on a directory tree the manifest shaped.
    fn remove_stale_volume_files(dir: &Path, keep: &BTreeSet<PathBuf>, depth: usize) -> Result<()> {
        const MAX_VOLUME_DEPTH: usize = 32;
        if depth > MAX_VOLUME_DEPTH {
            return Err(anyhow!("container volume nesting exceeds {MAX_VOLUME_DEPTH} levels"));
        }

        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                Self::remove_stale_volume_files(&path, keep, depth + 1)?;
            } else if !keep.contains(&path) {
                fs::remove_file(&path)
                    .with_context(|| format!("Failed to remove stale volume file {:?}", path))?;
            }
        }
        Ok(())
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

            // Only a volume that declares files is managed. An empty list is
            // left strictly alone rather than pruned, because that is the
            // scratch/data case and wiping it on every redeploy would destroy
            // whatever the container had written. The cost is that a volume
            // converted from config back to scratch keeps its old files; a
            // redeploy cannot tell that transition apart from a plain data
            // volume without state we deliberately don't keep.
            if !vol.files.is_empty() {
                Self::materialize_volume_files(&host_path, &vol.files)?;
            }

            volume_args.push("-v".to_string());
            volume_args.push(Self::volume_mount_arg(
                &host_path,
                &vol.container_path,
                !vol.files.is_empty(),
            ));
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

        let mut config_map = BTreeMap::new();
        #[allow(clippy::collapsible_if)]
        if let Some(sp) = &self.storage_provider {
            match sp.get_latest_config_generation(service_id).await {
                Ok(Some((_, blob))) => {
                    if let Ok(json) = serde_json::from_str::<Value>(&blob) {
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
                    error!("Failed to fetch config generation for {}: {}", service_id, e);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn inline_file(relative_path: &str, content: &str) -> ContainerVolumeFile {
        ContainerVolumeFile {
            relative_path: relative_path.to_string(),
            content: DocumentSource::Inline(content.to_string()),
        }
    }

    /// The gap this closes: an image that reads a mounted config file could
    /// not be deployed through the API at all, because the volume was created
    /// empty and nothing ever wrote into it.
    #[test]
    fn inline_files_are_written_into_the_volume() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![
            inline_file("default.conf", "server { listen 80; }"),
            inline_file("certs/ca.pem", "----BEGIN----"),
        ];

        ContainerEngine::materialize_volume_files(dir.path(), &files).unwrap();

        assert_eq!(
            fs::read_to_string(dir.path().join("default.conf")).unwrap(),
            "server { listen 80; }"
        );
        assert_eq!(fs::read_to_string(dir.path().join("certs/ca.pem")).unwrap(), "----BEGIN----");
    }

    /// The mount must reflect the current manifest, not the union of every
    /// deploy that came before it.
    #[test]
    fn redeploy_removes_files_the_manifest_no_longer_declares() {
        let dir = tempfile::tempdir().unwrap();

        ContainerEngine::materialize_volume_files(
            dir.path(),
            &[inline_file("keep.conf", "a"), inline_file("nested/drop.conf", "b")],
        )
        .unwrap();
        assert!(dir.path().join("nested/drop.conf").exists());

        ContainerEngine::materialize_volume_files(dir.path(), &[inline_file("keep.conf", "a2")])
            .unwrap();

        assert_eq!(fs::read_to_string(dir.path().join("keep.conf")).unwrap(), "a2");
        assert!(!dir.path().join("nested/drop.conf").exists(), "stale file survived redeploy");
    }

    #[test]
    fn escaping_relative_path_is_rejected_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().parent().unwrap().join("escaped.conf");

        for bad in ["../escaped.conf", "/etc/escaped.conf", "a/../../escaped.conf"] {
            let err =
                ContainerEngine::materialize_volume_files(dir.path(), &[inline_file(bad, "pwned")])
                    .unwrap_err();
            assert!(
                err.to_string().contains("must be a relative path inside the volume"),
                "{bad}: {err}"
            );
        }

        assert!(!outside.exists(), "a rejected path still wrote outside the volume");
    }

    #[test]
    fn volume_files_are_capped_in_aggregate() {
        let dir = tempfile::tempdir().unwrap();
        let half = "x".repeat(deploy_docs::MAX_DEPLOY_DOCUMENT_BYTES as usize);
        let files: Vec<_> = (0..5).map(|i| inline_file(&format!("f{i}.conf"), &half)).collect();

        let err = ContainerEngine::materialize_volume_files(dir.path(), &files).unwrap_err();
        assert!(err.to_string().contains("total limit"), "{err}");
    }

    #[test]
    fn mount_is_read_only_only_when_the_manifest_supplied_files() {
        let path = Path::new("/srv/vol");
        assert_eq!(
            ContainerEngine::volume_mount_arg(path, "/etc/nginx/conf.d", true),
            "/srv/vol:/etc/nginx/conf.d:ro"
        );
        assert_eq!(ContainerEngine::volume_mount_arg(path, "/data", false), "/srv/vol:/data");
    }
}
