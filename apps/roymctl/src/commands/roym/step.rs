//! Work steps of an extension, recorded inside a formed agreement.

use std::path::Path;

use anyhow::{Context, Result};
use clap::Subcommand;
use serde_json::{Value, json};

use super::directory::{RpcCtx, call_and_print};
use crate::DEFAULT_GATEWAY_URL;

#[derive(Subcommand, Debug, Clone)]
pub enum StepCommands {
    /// Record a work step of an extension for one commitment of an
    /// agreement, for example `scheduling/started`. The other party gets a
    /// signed card. The step does not change any deal state.
    Add {
        #[arg(long)]
        agreement: String,
        #[arg(long)]
        commitment: String,
        /// The step type, written `<extension>/<name>`.
        #[arg(long = "type")]
        step_type: String,
        /// The step's data as a JSON object. The default is `{}`.
        #[arg(long)]
        data: Option<String>,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// List the steps of an agreement, oldest first.
    List {
        #[arg(long)]
        agreement: String,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
}

/// Reads `--data` at this boundary. It must be a JSON object, as a step's
/// data always is.
pub(crate) fn parse_step_data(input: Option<&str>) -> Result<Value> {
    let Some(text) = input else {
        return Ok(json!({}));
    };
    let value: Value = serde_json::from_str(text).context("--data is not valid JSON")?;
    if !value.is_object() {
        anyhow::bail!("--data must be a JSON object");
    }
    Ok(value)
}

pub(super) async fn handle_step(
    cmd: &StepCommands,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> Result<()> {
    let ctx = RpcCtx { run_as, ucan_path, dir };
    match cmd {
        StepCommands::Add { agreement, commitment, step_type, data, gateway_url, host } => {
            let params = json!({
                "agreement": agreement,
                "commitment_id": commitment,
                "step_type": step_type,
                "data": parse_step_data(data.as_deref())?,
            });
            call_and_print(ctx, gateway_url, host.as_deref(), "step.add", params).await
        }
        StepCommands::List { agreement, gateway_url, host } => {
            let params = json!({ "agreement": agreement });
            call_and_print(ctx, gateway_url, host.as_deref(), "step.list", params).await
        }
    }
}
