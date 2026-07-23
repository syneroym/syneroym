#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Podman container execution engine
//!
//! Handles lifecycle of Podman containers using std::process::Command.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    io::Write,
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
use tokio::task;
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

    /// Resolves a volume file's content.
    ///
    /// Only `Inline` is accepted here, deliberately. For `schema` and
    /// `fdae-policy` a host-side path is safe because the bytes stay
    /// server-side; a volume file is copied into a directory bind-mounted
    /// into a container whose image the deploy caller chose, so reading one
    /// off the substrate's disk would hand that caller any file the substrate
    /// can reach. Bounding the read to the working directory does not fix
    /// that -- the working directory is where the substrate's own data lives.
    /// Reintroducing the path arm needs an operator-configured document root.
    fn resolve_volume_file(source: &DocumentSource, field_name: &str) -> Result<String> {
        match source {
            DocumentSource::Inline(content) => {
                deploy_docs::check_inline_size(content, field_name).map_err(|e| anyhow!(e))?;
                Ok(content.clone())
            }
            DocumentSource::Path(_) => Err(anyhow!(
                "{field_name} must carry inline content: a substrate-host path is not accepted \
                 for container volume files, because the volume is readable by the deployed \
                 container"
            )),
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

    /// Materializes a volume's declared files, returning the deploy-wide byte
    /// budget left over.
    ///
    /// Every file is resolved and checked before a single byte is written, and
    /// the set is then built in a sibling staging directory that replaces the
    /// live one only once it is complete. Three properties fall out of that
    /// shape rather than needing their own guards:
    ///
    /// - A failure anywhere leaves the live directory untouched, so a redeploy
    ///   whose `podman run` fails does not strand a half-rewritten config
    ///   directory under a container that is still serving from it.
    /// - Nothing stale can survive, because the directory is replaced whole
    ///   rather than pruned file by file.
    /// - A symlink an earlier deploy or the container itself planted cannot
    ///   redirect a write, because the staging directory is new and every file
    ///   is created exclusively.
    fn materialize_volume_files(
        volume_root: &Path,
        files: &[ContainerVolumeFile],
        budget: u64,
    ) -> Result<u64> {
        let mut remaining = budget;
        let mut resolved: Vec<(PathBuf, String)> = Vec::with_capacity(files.len());
        let mut seen = BTreeSet::new();

        for file in files {
            deploy_docs::reject_relative_escape(&file.relative_path, "volume file relative-path")
                .map_err(|e| anyhow!(e))?;
            let content = Self::resolve_volume_file(&file.content, "volume file")?;

            remaining = remaining.checked_sub(content.len() as u64).ok_or_else(|| {
                anyhow!(
                    "container volume files exceed the {} byte budget for one deploy",
                    deploy_docs::MAX_DEPLOY_VOLUME_BYTES
                )
            })?;

            let relative = PathBuf::from(&file.relative_path);
            if !seen.insert(relative.clone()) {
                return Err(anyhow!("duplicate container volume file {:?}", file.relative_path));
            }
            resolved.push((relative, content));
        }

        let staging = Self::sibling_dir(volume_root, ".staging")?;
        let _ = fs::remove_dir_all(&staging);
        fs::create_dir_all(&staging)
            .with_context(|| format!("Failed to create staging directory {:?}", staging))?;

        if let Err(e) = Self::write_staged_files(&staging, &resolved) {
            let _ = fs::remove_dir_all(&staging);
            return Err(e);
        }

        Self::swap_into_place(&staging, volume_root)?;
        Ok(remaining)
    }

    fn write_staged_files(staging: &Path, resolved: &[(PathBuf, String)]) -> Result<()> {
        for (relative, content) in resolved {
            let target = staging.join(relative);
            let parent = target
                .parent()
                .ok_or_else(|| anyhow!("volume file {:?} has no parent", relative))?;
            fs::create_dir_all(parent)?;

            // `create_new` is `O_CREAT|O_EXCL`, which fails on an existing
            // final component *including a symlink* -- `fs::write` would
            // follow one. The staging directory is fresh, so this can only
            // trip on a duplicate, but it is the guard that makes "no write
            // escapes the volume" true rather than merely likely.
            let mut handle = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
                .with_context(|| format!("Failed to create volume file {:?}", target))?;
            handle
                .write_all(content.as_bytes())
                .with_context(|| format!("Failed to write volume file {:?}", target))?;
        }
        Ok(())
    }

    /// Replaces `volume_root` with `staging`. The old directory is moved aside
    /// first and only removed once the new one is in place, so a failure
    /// mid-swap restores what was there rather than leaving the mount point
    /// missing.
    fn swap_into_place(staging: &Path, volume_root: &Path) -> Result<()> {
        let retired = Self::sibling_dir(volume_root, ".retired")?;
        let _ = fs::remove_dir_all(&retired);

        let had_previous = volume_root.exists();
        if had_previous {
            fs::rename(volume_root, &retired).with_context(|| {
                format!("Failed to move aside container volume {:?}", volume_root)
            })?;
        }

        match fs::rename(staging, volume_root) {
            Ok(()) => {
                let _ = fs::remove_dir_all(&retired);
                Ok(())
            }
            Err(e) => {
                if had_previous {
                    let _ = fs::rename(&retired, volume_root);
                }
                let _ = fs::remove_dir_all(staging);
                Err(anyhow!("Failed to install container volume files: {e}"))
            }
        }
    }

    fn sibling_dir(volume_root: &Path, suffix: &str) -> Result<PathBuf> {
        let parent = volume_root
            .parent()
            .ok_or_else(|| anyhow!("container volume {:?} has no parent directory", volume_root))?;
        let name = volume_root
            .file_name()
            .ok_or_else(|| anyhow!("container volume {:?} has no directory name", volume_root))?;
        let mut staged = name.to_os_string();
        staged.push(suffix);
        Ok(parent.join(staged))
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
        let mut volume_budget = deploy_docs::MAX_DEPLOY_VOLUME_BYTES;
        for vol in &container_manifest.volumes {
            let host_path = self.resolve_host_path(service_id, &vol.host_path);
            fs::create_dir_all(&host_path)
                .with_context(|| format!("Failed to create host path directory {:?}", host_path))?;

            // Only a volume that declares files is managed. An empty list is
            // left strictly alone rather than replaced, because that is the
            // scratch/data case and wiping it on every redeploy would destroy
            // whatever the container had written. The cost is that a volume
            // converted from config back to scratch keeps its old files; a
            // redeploy cannot tell that transition apart from a plain data
            // volume without state we deliberately don't keep.
            //
            // The work is several MiB of synchronous file I/O, so it goes to a
            // blocking thread rather than stalling every other future on this
            // worker -- the router, health, and metrics share it.
            if !vol.files.is_empty() {
                let files = vol.files.clone();
                let root = host_path.clone();
                volume_budget = task::spawn_blocking(move || {
                    Self::materialize_volume_files(&root, &files, volume_budget)
                })
                .await
                .map_err(|e| anyhow!("container volume materialization panicked: {e}"))??;
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

    const BUDGET: u64 = deploy_docs::MAX_DEPLOY_VOLUME_BYTES;

    fn inline_file(relative_path: &str, content: &str) -> ContainerVolumeFile {
        ContainerVolumeFile {
            relative_path: relative_path.to_string(),
            content: DocumentSource::Inline(content.to_string()),
        }
    }

    /// A volume root nested one level down, since materialization stages into
    /// a sibling directory and so needs a parent it may write to.
    fn volume_root(dir: &tempfile::TempDir) -> PathBuf {
        let root = dir.path().join("vol");
        fs::create_dir_all(&root).unwrap();
        root
    }

    /// The gap this closes: an image that reads a mounted config file could
    /// not be deployed through the API at all, because the volume was created
    /// empty and nothing ever wrote into it.
    #[test]
    fn inline_files_are_written_into_the_volume() {
        let dir = tempfile::tempdir().unwrap();
        let root = volume_root(&dir);
        let files = vec![
            inline_file("default.conf", "server { listen 80; }"),
            inline_file("certs/ca.pem", "----BEGIN----"),
        ];

        ContainerEngine::materialize_volume_files(&root, &files, BUDGET).unwrap();

        assert_eq!(fs::read_to_string(root.join("default.conf")).unwrap(), "server { listen 80; }");
        assert_eq!(fs::read_to_string(root.join("certs/ca.pem")).unwrap(), "----BEGIN----");
    }

    /// The mount must reflect the current manifest, not the union of every
    /// deploy that came before it.
    #[test]
    fn redeploy_removes_files_the_manifest_no_longer_declares() {
        let dir = tempfile::tempdir().unwrap();
        let root = volume_root(&dir);

        ContainerEngine::materialize_volume_files(
            &root,
            &[inline_file("keep.conf", "a"), inline_file("nested/drop.conf", "b")],
            BUDGET,
        )
        .unwrap();
        assert!(root.join("nested/drop.conf").exists());

        ContainerEngine::materialize_volume_files(&root, &[inline_file("keep.conf", "a2")], BUDGET)
            .unwrap();

        assert_eq!(fs::read_to_string(root.join("keep.conf")).unwrap(), "a2");
        assert!(!root.join("nested/drop.conf").exists(), "stale file survived redeploy");
    }

    #[test]
    fn escaping_relative_path_is_rejected_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let root = volume_root(&dir);
        let outside = dir.path().join("escaped.conf");

        for bad in ["../escaped.conf", "/etc/escaped.conf", "a/../../escaped.conf"] {
            let err = ContainerEngine::materialize_volume_files(
                &root,
                &[inline_file(bad, "pwned")],
                BUDGET,
            )
            .unwrap_err();
            assert!(
                err.to_string().contains("must be a relative path inside the volume"),
                "{bad}: {err}"
            );
        }

        assert!(!outside.exists(), "a rejected path still wrote outside the volume");
    }

    /// A writable volume lets the container plant a symlink; the next deploy
    /// declaring that same name must not write through it. Staging into a
    /// fresh directory and creating each file exclusively is what prevents it
    /// -- `fs::write` onto the live directory would have followed the link.
    #[test]
    fn a_symlink_left_in_the_volume_cannot_redirect_a_write() {
        let dir = tempfile::tempdir().unwrap();
        let root = volume_root(&dir);
        let secret = dir.path().join("substrate-secret.toml");
        fs::write(&secret, "original").unwrap();

        std::os::unix::fs::symlink(&secret, root.join("app.conf")).unwrap();

        ContainerEngine::materialize_volume_files(
            &root,
            &[inline_file("app.conf", "attacker-controlled")],
            BUDGET,
        )
        .unwrap();

        assert_eq!(
            fs::read_to_string(&secret).unwrap(),
            "original",
            "write followed a symlink out of the volume"
        );
        assert_eq!(fs::read_to_string(root.join("app.conf")).unwrap(), "attacker-controlled");
    }

    /// A rejected file set must not leave a partially written volume behind,
    /// which a per-file write-then-check loop could not guarantee.
    #[test]
    fn an_over_budget_file_set_writes_nothing_at_all() {
        let dir = tempfile::tempdir().unwrap();
        let root = volume_root(&dir);
        let huge = "x".repeat(deploy_docs::MAX_DEPLOY_DOCUMENT_BYTES as usize);

        let err = ContainerEngine::materialize_volume_files(
            &root,
            &[
                inline_file("ok.conf", "fine"),
                inline_file("a.conf", &huge),
                inline_file("b.conf", &huge),
                inline_file("c.conf", &huge),
                inline_file("d.conf", &huge),
                inline_file("e.conf", &huge),
            ],
            BUDGET,
        )
        .unwrap_err();

        assert!(err.to_string().contains("budget"), "{err}");
        assert!(!root.join("ok.conf").exists(), "partial write survived a rejected file set");
    }

    /// The budget is deploy-wide, so it must carry across volumes rather than
    /// resetting for each one.
    #[test]
    fn the_byte_budget_is_shared_across_volumes() {
        let dir = tempfile::tempdir().unwrap();
        let root = volume_root(&dir);
        // At the per-file cap, so the budget is what runs out first.
        let one_mib = "x".repeat(deploy_docs::MAX_DEPLOY_DOCUMENT_BYTES as usize);

        let remaining = ContainerEngine::materialize_volume_files(
            &root,
            &[
                inline_file("a.conf", &one_mib),
                inline_file("b.conf", &one_mib),
                inline_file("c.conf", &one_mib),
            ],
            BUDGET,
        )
        .unwrap();
        assert_eq!(remaining, BUDGET - 3 * deploy_docs::MAX_DEPLOY_DOCUMENT_BYTES);

        // A second volume inherits what is left, rather than a fresh budget.
        let second = dir.path().join("vol2");
        fs::create_dir_all(&second).unwrap();
        let err = ContainerEngine::materialize_volume_files(
            &second,
            &[inline_file("d.conf", &one_mib), inline_file("e.conf", &one_mib)],
            remaining,
        )
        .unwrap_err();
        assert!(err.to_string().contains("budget"), "{err}");
        assert!(!second.join("d.conf").exists());
    }

    /// A host-side path is accepted for `schema` and `fdae-policy`, whose
    /// bytes stay server-side, but not here: the volume is readable by a
    /// container the deploy caller chose.
    #[test]
    fn a_host_path_is_refused_for_volume_content() {
        let dir = tempfile::tempdir().unwrap();
        let root = volume_root(&dir);

        let err = ContainerEngine::materialize_volume_files(
            &root,
            &[ContainerVolumeFile {
                relative_path: "leak.txt".to_string(),
                content: DocumentSource::Path("data/syneroym.db".to_string()),
            }],
            BUDGET,
        )
        .unwrap_err();

        assert!(err.to_string().contains("must carry inline content"), "{err}");
        assert!(!root.join("leak.txt").exists());
    }

    /// A failed materialization must leave the directory a running container
    /// is still mounting exactly as it was.
    #[test]
    fn a_failed_materialization_leaves_the_live_volume_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let root = volume_root(&dir);

        ContainerEngine::materialize_volume_files(
            &root,
            &[inline_file("live.conf", "serving")],
            BUDGET,
        )
        .unwrap();

        let err = ContainerEngine::materialize_volume_files(
            &root,
            &[inline_file("new.conf", "replacement"), inline_file("../escape.conf", "bad")],
            BUDGET,
        )
        .unwrap_err();
        assert!(err.to_string().contains("must be a relative path inside the volume"), "{err}");

        assert_eq!(fs::read_to_string(root.join("live.conf")).unwrap(), "serving");
        assert!(!root.join("new.conf").exists(), "a failed deploy mutated the live volume");
    }

    #[test]
    fn duplicate_relative_paths_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let root = volume_root(&dir);

        let err = ContainerEngine::materialize_volume_files(
            &root,
            &[inline_file("a.conf", "one"), inline_file("a.conf", "two")],
            BUDGET,
        )
        .unwrap_err();
        assert!(err.to_string().contains("duplicate"), "{err}");
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
