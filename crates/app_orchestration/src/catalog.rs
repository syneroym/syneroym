use std::collections::BTreeMap;

use anyhow::{Result, anyhow};

use crate::models::{
    AppBlueprintId, LogicalServiceName, ServiceConfig, ServiceSpec, ServiceType, SynAppManifest,
};

/// Trait for resolving application blueprints to their parsed manifests.
///
/// Implementations handle I/O (filesystem, network, etc.) so the compiler
/// core remains a pure function from manifests to plans.
#[async_trait::async_trait]
pub trait ManifestCatalog: Send + Sync {
    /// Resolve a blueprint ID (with an optional path hint from the Spawn
    /// directive) to a parsed and validated `SynAppManifest`.
    async fn resolve(
        &self,
        blueprint: &AppBlueprintId,
        manifest_path: Option<&str>,
    ) -> Result<SynAppManifest>;
}

/// A local filesystem-based `ManifestCatalog` implementation.
#[derive(Debug, Clone)]
pub struct LocalFilesystemCatalog {
    base_dir: std::path::PathBuf,
    manifests: BTreeMap<AppBlueprintId, SynAppManifest>,
}

impl LocalFilesystemCatalog {
    pub fn new(base_dir: std::path::PathBuf) -> Self {
        Self { base_dir, manifests: BTreeMap::new() }
    }

    pub fn register(&mut self, id: AppBlueprintId, manifest: SynAppManifest) {
        self.manifests.insert(id, manifest);
    }

    /// Legacy shim to generate an in-memory single-WASM manifest.
    pub fn register_legacy_wasm_shim(&mut self, id: AppBlueprintId, wasm_path: &str) {
        let mut services = BTreeMap::new();
        let config = ServiceConfig {
            service_type: ServiceType::Wasm,
            source: wasm_path.to_string(),
            hash: None,
            interfaces: vec![],
            env: BTreeMap::new(),
            args: vec![],
            custom_config: None,
            quota: None,
            schema_path: None,
            rotation_policy: Default::default(),
        };
        services.insert(
            LogicalServiceName::new("legacy-main"),
            ServiceSpec { config, depends_on: vec![] },
        );

        let manifest = SynAppManifest {
            id: id.clone(),
            version: semver::Version::new(0, 0, 0),
            description: Some("Legacy Single-WASM Shim".to_string()),
            services,
            dependencies: BTreeMap::new(),
        };
        self.register(id, manifest);
    }
}

#[async_trait::async_trait]
impl ManifestCatalog for LocalFilesystemCatalog {
    async fn resolve(
        &self,
        blueprint: &AppBlueprintId,
        manifest_path: Option<&str>,
    ) -> Result<SynAppManifest> {
        // First check in-memory map (useful for tests)
        if let Some(manifest) = self.manifests.get(blueprint) {
            return Ok(manifest.clone());
        }

        // Otherwise, resolve via filesystem
        let path = if let Some(path_hint) = manifest_path {
            let hint_path = std::path::Path::new(path_hint);
            if hint_path.is_absolute() {
                return Err(anyhow!(
                    "Absolute manifest path hint is rejected for security: {}",
                    path_hint
                ));
            }
            for component in hint_path.components() {
                if matches!(component, std::path::Component::ParentDir) {
                    return Err(anyhow!(
                        "Directory traversal (../) in manifest path hint is rejected: {}",
                        path_hint
                    ));
                }
            }
            self.base_dir.join(hint_path)
        } else {
            // Default path lookup
            let default_name = format!("{}.toml", blueprint.as_str().replace(':', "_"));
            self.base_dir.join(default_name)
        };

        if !path.exists() {
            return Err(anyhow!("Manifest file not found at path: {:?}", path));
        }

        let content = tokio::fs::read_to_string(&path).await.map_err(|e| {
            anyhow!("Failed to read manifest for '{}' at {:?}: {}", blueprint, path, e)
        })?;

        if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            SynAppManifest::from_json(&content)
        } else {
            SynAppManifest::from_toml(&content)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_local_filesystem_catalog() {
        let temp_dir = tempfile::tempdir().unwrap();
        let manifest_path = temp_dir.path().join("syneroym_db-app.toml");

        let toml_content = r#"
            id = "syneroym:db-app"
            version = "1.0.0"
            [services.db]
            service_type = "wasm"
            source = "db.wasm"
        "#;
        std::fs::write(&manifest_path, toml_content).unwrap();

        let catalog = LocalFilesystemCatalog::new(temp_dir.path().to_path_buf());
        let resolved = catalog
            .resolve(&AppBlueprintId::new("syneroym:db-app"), Some("syneroym_db-app.toml"))
            .await
            .unwrap();

        assert_eq!(resolved.id.as_str(), "syneroym:db-app");
    }

    #[tokio::test]
    async fn test_manifest_default_path() {
        let temp_dir = tempfile::tempdir().unwrap();
        let catalog = LocalFilesystemCatalog::new(temp_dir.path().to_path_buf());

        let toml_content = r#"
            id = "syneroym:my-blueprint"
            version = "1.0.0"
            [services.my-svc]
            service_type = "wasm"
            source = "source.wasm"
        "#;

        // Default filename format is: syneroym_my-blueprint.toml
        let target_path = temp_dir.path().join("syneroym_my-blueprint.toml");
        std::fs::write(&target_path, toml_content).unwrap();

        let blueprint_id = AppBlueprintId::new("syneroym:my-blueprint");
        let resolved = catalog.resolve(&blueprint_id, None).await.unwrap();
        assert_eq!(resolved.id, blueprint_id);
    }

    #[tokio::test]
    async fn test_negative_file_system() {
        let temp_dir = tempfile::tempdir().unwrap();
        let catalog = LocalFilesystemCatalog::new(temp_dir.path().to_path_buf());

        let blueprint_id = AppBlueprintId::new("syneroym:missing-blueprint");

        // 1. File not found
        let res = catalog.resolve(&blueprint_id, None).await;
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("Manifest file not found"));

        // 2. Invalid TOML syntax
        let target_path = temp_dir.path().join("syneroym_invalid-blueprint.toml");
        std::fs::write(&target_path, "invalid = [toml = logic").unwrap();

        let blueprint_invalid = AppBlueprintId::new("syneroym:invalid-blueprint");
        let res_invalid = catalog.resolve(&blueprint_invalid, None).await;
        assert!(res_invalid.is_err());
    }

    #[tokio::test]
    async fn test_traversal_rejection() {
        let temp_dir = tempfile::tempdir().unwrap();
        let catalog = LocalFilesystemCatalog::new(temp_dir.path().to_path_buf());
        let blueprint_id = AppBlueprintId::new("syneroym:malicious");

        // Absolute path hint should be rejected
        let absolute_res = catalog.resolve(&blueprint_id, Some("/etc/passwd")).await;
        assert!(absolute_res.is_err());
        assert!(
            absolute_res
                .unwrap_err()
                .to_string()
                .contains("Absolute manifest path hint is rejected")
        );

        // Parent directory traversal hint should be rejected
        let traversal_res = catalog.resolve(&blueprint_id, Some("../passwd")).await;
        assert!(traversal_res.is_err());
        assert!(traversal_res.unwrap_err().to_string().contains("Directory traversal (../)"));
    }

    #[tokio::test]
    async fn test_legacy_wasm_shim() {
        let catalog_dir = tempfile::tempdir().unwrap();
        let mut catalog = LocalFilesystemCatalog::new(catalog_dir.path().to_path_buf());
        let blueprint_id = AppBlueprintId::new("syneroym:legacy-wasm");

        catalog.register_legacy_wasm_shim(blueprint_id.clone(), "path/to/legacy.wasm");

        let resolved = catalog.resolve(&blueprint_id, None).await.unwrap();
        assert_eq!(resolved.id, blueprint_id);
        assert_eq!(resolved.services.len(), 1);
        let svc = resolved.services.get(&LogicalServiceName::new("legacy-main")).unwrap();
        assert_eq!(svc.config.service_type, ServiceType::Wasm);
        assert_eq!(svc.config.source, "path/to/legacy.wasm");
        assert_eq!(resolved.version.to_string(), "0.0.0");
    }
}
