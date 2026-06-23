use clap::Subcommand;

#[derive(Subcommand, Debug, Clone)]
pub enum AppCommands {
    /// Deploy a `SynApp` manifest (Dual Versioning)
    Deploy,
}

pub async fn handle(_command: &AppCommands) -> anyhow::Result<()> {
    println!("SynApp dual-versioning manifest deployment coming soon.");
    Ok(())
}
