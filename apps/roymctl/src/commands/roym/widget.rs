//! Interactive chat widgets (such as polls) and their answers.

use std::path::Path;

use anyhow::{Context, Result};
use clap::Subcommand;
use serde_json::{Value, json};
use syneroym_roym_core::widget::poll_definition;

use super::directory::{RpcCtx, call_and_print};
use crate::DEFAULT_GATEWAY_URL;

#[derive(Subcommand, Debug, Clone)]
pub enum WidgetCommands {
    /// Send a single-choice poll into a conversation.
    Poll {
        #[arg(long)]
        conversation: String,
        #[arg(long)]
        question: String,
        #[arg(long = "option")]
        options: Vec<String>,
        #[arg(long = "lang")]
        langs: Vec<String>,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Send an answer to a widget.
    Answer {
        #[arg(long)]
        conversation: String,
        #[arg(long = "widget")]
        widget_id: String,
        /// The answer data as a JSON object, e.g. '{"option":"o1"}'.
        #[arg(long)]
        data: String,
        #[arg(long = "lang")]
        langs: Vec<String>,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// List the widgets in a conversation with their option counts.
    List {
        #[arg(long)]
        conversation: String,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
}

/// Parses `--data` at this boundary. It must be a JSON object.
pub(crate) fn parse_widget_data(input: &str) -> Result<Value> {
    let value: Value = serde_json::from_str(input).context("--data is not valid JSON")?;
    if !value.is_object() {
        anyhow::bail!("--data must be a JSON object");
    }
    Ok(value)
}

pub(super) async fn handle_widget(
    cmd: &WidgetCommands,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> Result<()> {
    let ctx = RpcCtx { run_as, ucan_path, dir };
    match cmd {
        WidgetCommands::Poll { conversation, question, options, langs, gateway_url, host } => {
            let poll = poll_definition(question, options);
            let mut params = json!({
                "conversation": conversation,
                "widget": poll,
            });
            if !langs.is_empty() {
                params["languages"] = json!(langs);
            }
            call_and_print(ctx, gateway_url, host.as_deref(), "widget.send", params).await
        }
        WidgetCommands::Answer { conversation, widget_id, data, langs, gateway_url, host } => {
            let data_val = parse_widget_data(data)?;
            let mut params = json!({
                "conversation": conversation,
                "widget_id": widget_id,
                "data": data_val,
            });
            if !langs.is_empty() {
                params["languages"] = json!(langs);
            }
            call_and_print(ctx, gateway_url, host.as_deref(), "widget.answer", params).await
        }
        WidgetCommands::List { conversation, gateway_url, host } => {
            let params = json!({
                "conversation": conversation,
            });
            call_and_print(ctx, gateway_url, host.as_deref(), "widget.list", params).await
        }
    }
}
