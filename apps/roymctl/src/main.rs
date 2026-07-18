#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Command-line control interface for operations on, and through, a Syneroym
//! substrate.

use std::{path::PathBuf, process};

use clap::Parser;
use commands::Commands;
use rustls::crypto::ring;

mod commands;

/// Default API endpoint for the Community Registry
const DEFAULT_API_URL: &str = "http://localhost:7961";

#[derive(Parser)]
#[command(name = "roymctl")]
#[command(about = "Syneroym Control CLI (API Client)", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// The base URL of the Syneroym Community Registry
    #[arg(global = true, long, default_value = DEFAULT_API_URL)]
    api_url: String,

    /// The DID of the target substrate to control
    #[arg(global = true, long)]
    substrate: Option<String>,

    /// Local directory for configuration and offline identities (defaults to
    /// current dir)
    #[arg(global = true, long, default_value = ".")]
    dir: PathBuf,

    /// Act as this locally-stored identity (see `roymctl identity create
    /// --name`). Defaults to an ephemeral per-invocation key, which owns
    /// nothing and can see nothing on an owned substrate (M04A Slice B7a).
    ///
    /// Distinct from `svc deploy --identity`, which names the *app's*
    /// signing key for its registry certificate; this names the *operator*.
    #[arg(global = true, long = "as")]
    run_as: Option<String>,

    /// Path to a signed UCAN `CapabilityToken` JSON file (see `roymctl
    /// identity issue-grant`) to present on connect (M04A Slice B7b) --
    /// proves whatever capability the grant names, on top of `--as`'s
    /// transport identity. Requires `--as <name>`, where `<name>` is the
    /// identity the grant's `--to` names -- the token's audience must match
    /// the connecting identity, which a bare `--ucan` (no `--as`) can never
    /// satisfy (a fresh ephemeral key every invocation).
    #[arg(global = true, long = "ucan", requires = "run_as")]
    ucan_path: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if ring::default_provider().install_default().is_err() {
        eprintln!("Failed to install rustls default crypto provider");
        process::exit(1);
    }

    let cli = Cli::parse();

    commands::run(cli.command, cli.api_url, cli.substrate, cli.dir, cli.run_as, cli.ucan_path)
        .await?;

    Ok(())
}
