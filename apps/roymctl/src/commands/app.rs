use std::path::PathBuf;

use clap::Subcommand;
use syneroym_app_orchestration::{AppInstanceId, DeploymentJournal, Reconciler};

#[derive(Subcommand, Debug, Clone)]
pub enum AppCommands {
    /// Deploy a `SynApp` manifest (Dual Versioning)
    Deploy,
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

pub async fn handle(command: &AppCommands) -> anyhow::Result<()> {
    match command {
        AppCommands::Deploy => {
            println!("SynApp dual-versioning manifest deployment coming soon.");
        }
        AppCommands::Reconcile { instance_id, manifest_path, journal_path } => {
            let instance_id = AppInstanceId::try_new(instance_id.clone())?;

            let parent_dir = journal_path.parent().unwrap_or(std::path::Path::new("."));
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
                let active = journal.get_last_state(
                    &instance_id,
                    syneroym_app_orchestration::DeploymentState::Active,
                )?;
                if active.is_some() {
                    if let Some(manifest_path) = manifest_path {
                        println!(
                            "App {} is ACTIVE. Diffing active deployment against manifest at {:?}",
                            instance_id, manifest_path
                        );

                        let toml_str = std::fs::read_to_string(manifest_path)?;
                        let manifest =
                            syneroym_app_orchestration::SynAppManifest::from_toml(&toml_str)?;
                        let catalog = syneroym_app_orchestration::LocalFilesystemCatalog::new(
                            manifest_path
                                .parent()
                                .unwrap_or(std::path::Path::new("."))
                                .to_path_buf(),
                        );

                        let compiled = syneroym_app_orchestration::compile(
                            instance_id.clone(),
                            &manifest,
                            &catalog,
                        )
                        .await?;

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
