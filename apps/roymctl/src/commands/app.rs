use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use clap::Subcommand;
use semver::Version;
use syneroym_app_orchestration::{
    AppInstanceId, DeploymentJournal, DeploymentState, LocalFilesystemCatalog, Reconciler,
    SynAppManifest, compile,
    models::{AppBlueprintId, LogicalServiceName, ServiceConfig, ServiceSpec, ServiceType},
};
use syneroym_sdk::{SyneroymClient, mapper};

#[derive(Subcommand, Debug, Clone)]
pub enum AppCommands {
    /// Deploy a `SynApp` manifest (Dual Versioning)
    Deploy {
        /// The AppInstanceId to deploy
        instance_id: String,
        /// Path to the SynApp manifest TOML file or legacy .wasm file
        manifest_path: PathBuf,
        /// Path to the SQLite deployment journal
        #[arg(long, default_value = "deployments.db")]
        journal_path: PathBuf,
    },
    /// Reconcile a deployment to recover or compute updates
    Reconcile {
        /// The AppInstanceId to reconcile
        instance_id: String,
        /// Optional path to a new SynApp manifest to diff against
        #[arg(long)]
        manifest_path: Option<PathBuf>,
        /// Path to the SQLite deployment journal
        #[arg(long, default_value = "deployments.db")]
        journal_path: PathBuf,
    },
}

pub async fn handle(
    command: &AppCommands,
    api_url: &str,
    substrate_did: String,
) -> anyhow::Result<()> {
    match command {
        AppCommands::Deploy { instance_id, manifest_path, journal_path } => {
            let instance_id = AppInstanceId::try_new(instance_id.clone())?;

            let manifest = if manifest_path.extension().and_then(|s| s.to_str()) == Some("wasm") {
                let mut services = BTreeMap::new();
                services.insert(
                    LogicalServiceName::new("main"),
                    ServiceSpec {
                        config: ServiceConfig {
                            service_type: ServiceType::Wasm,
                            source: manifest_path.to_string_lossy().to_string(),
                            hash: None,
                            interfaces: vec![],
                            env: BTreeMap::new(),
                            args: vec![],
                            custom_config: None,
                            quota: None,
                            schema_path: None,
                            rotation_policy: Default::default(),
                        },
                        depends_on: vec![],
                    },
                );
                SynAppManifest {
                    id: AppBlueprintId::new("legacy-wasm-app"),
                    version: Version::new(0, 1, 0),
                    description: Some("Auto-generated legacy wrapper".to_string()),
                    services,
                    dependencies: BTreeMap::new(),
                }
            } else {
                let toml_str = fs::read_to_string(manifest_path)?;
                SynAppManifest::from_toml(&toml_str)?
            };

            let catalog = LocalFilesystemCatalog::new(
                manifest_path.parent().unwrap_or(Path::new(".")).to_path_buf(),
            );

            let compiled = compile(instance_id.clone(), &manifest, &catalog).await?;

            if let Some(target_plan) = compiled.plans.last() {
                let parent_dir = journal_path.parent().unwrap_or(Path::new("."));
                let db_name = journal_path
                    .file_name()
                    .ok_or_else(|| anyhow::anyhow!("Invalid journal path"))?
                    .to_str()
                    .ok_or_else(|| anyhow::anyhow!("Invalid journal path characters"))?;

                let journal = DeploymentJournal::open(parent_dir, db_name)?;
                let reconciler = Reconciler::new(&journal);

                let record_id = journal.append(target_plan, DeploymentState::Planned)?;
                let _diff = reconciler.compute_diff(target_plan)?;
                journal.update_state(record_id, DeploymentState::Applying)?;

                let wit_plan = mapper::map_deployment_plan_to_wit(target_plan.clone())?;

                let mut client = SyneroymClient::new(substrate_did.clone(), api_url.to_string());
                client.connect().await?;
                client.deploy_plan(wit_plan).await?;

                journal.update_state(record_id, DeploymentState::Active)?;

                println!("Successfully deployed application plan for {}", instance_id);
            } else {
                return Err(anyhow::anyhow!("Compiled deployment contains no plans"));
            }
        }
        AppCommands::Reconcile { instance_id, manifest_path, journal_path } => {
            let instance_id = AppInstanceId::try_new(instance_id.clone())?;

            let parent_dir = journal_path.parent().unwrap_or(Path::new("."));
            let db_name = journal_path
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("Invalid journal path"))?
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("Invalid journal path characters"))?;

            let journal = DeploymentJournal::open(parent_dir, db_name)?;
            let reconciler = Reconciler::new(&journal);

            if let Some(recovery_plan) = reconciler.recover_applying(&instance_id)? {
                println!("Found APPLYING state for {}. Computed recovery plan:", instance_id);
                for action in recovery_plan.actions {
                    println!(" - {:?}", action);
                }
            } else {
                let active = journal.get_last_state(&instance_id, DeploymentState::Active)?;
                if active.is_some() {
                    if let Some(manifest_path) = manifest_path {
                        println!(
                            "App {} is ACTIVE. Diffing active deployment against manifest at {:?}",
                            instance_id, manifest_path
                        );

                        let toml_str = fs::read_to_string(manifest_path)?;
                        let manifest = SynAppManifest::from_toml(&toml_str)?;
                        let catalog = LocalFilesystemCatalog::new(
                            manifest_path.parent().unwrap_or(Path::new(".")).to_path_buf(),
                        );

                        let compiled = compile(instance_id.clone(), &manifest, &catalog).await?;

                        if let Some(target_plan) = compiled.plans.last() {
                            let diff = reconciler.compute_diff(target_plan)?;
                            println!("Computed diff:");
                            if diff.actions.is_empty() {
                                println!(" (No changes)");
                            } else {
                                for action in diff.actions {
                                    println!(" - {:?}", action);
                                }
                            }
                        } else {
                            println!("Compiled deployment contains no plans.");
                        }
                    } else {
                        println!(
                            "App {} is ACTIVE. Provide a --manifest-path to compute a diff.",
                            instance_id
                        );
                    }
                } else {
                    println!("No ACTIVE or APPLYING state found for {}", instance_id);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct DummyCli {
        #[command(subcommand)]
        command: AppCommands,
    }

    #[test]
    fn test_app_reconcile_command_parsing() {
        let cli = DummyCli::try_parse_from([
            "dummy",
            "reconcile",
            "inst-1",
            "--manifest-path",
            "test.toml",
            "--journal-path",
            "test.db",
        ])
        .unwrap();

        match cli.command {
            AppCommands::Reconcile { instance_id, manifest_path, journal_path } => {
                assert_eq!(instance_id, "inst-1");
                assert_eq!(manifest_path, Some(PathBuf::from("test.toml")));
                assert_eq!(journal_path, PathBuf::from("test.db"));
            }
            _ => panic!("Expected Reconcile command"),
        }
    }
}
