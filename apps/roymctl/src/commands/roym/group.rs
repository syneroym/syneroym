//! Group conversation commands: create, rename, membership, messages, and sync.

use std::path::Path;

use anyhow::{Result, bail};
use clap::Subcommand;
use serde_json::json;

use super::directory::{RpcCtx, call_and_print};
use crate::DEFAULT_GATEWAY_URL;

#[derive(Subcommand, Debug, Clone)]
pub enum GroupCommands {
    /// Create a new group conversation.
    Create {
        #[arg(long)]
        name: Option<String>,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Rename an existing group conversation.
    Rename {
        #[arg(long)]
        group: String,
        #[arg(long)]
        name: String,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Add a member to a group conversation.
    Add {
        #[arg(long)]
        group: String,
        #[arg(long, group = "who")]
        address: Option<String>,
        #[arg(long, group = "who")]
        person_did: Option<String>,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Remove a member from a group conversation.
    Remove {
        #[arg(long)]
        group: String,
        #[arg(long, group = "who")]
        address: Option<String>,
        #[arg(long, group = "who")]
        person_did: Option<String>,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// List group conversations.
    List {
        #[arg(long)]
        include_hidden: bool,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Show information and notices for a group conversation.
    Info {
        #[arg(long)]
        group: String,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Send a message into a group conversation.
    Send {
        #[arg(long)]
        group: String,
        #[arg(long)]
        body: String,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Show message history of a group conversation.
    History {
        #[arg(long)]
        group: String,
        #[arg(long)]
        limit: Option<u32>,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Synchronize group membership and messages with peers.
    Sync {
        #[arg(long)]
        group: String,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Verify transcript consistency and compute the transcript digest.
    Check {
        #[arg(long)]
        group: String,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Hide a group conversation and refuse further messages into local
    /// storage.
    Hide {
        #[arg(long)]
        group: String,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Unhide a group conversation and recover missed messages.
    Unhide {
        #[arg(long)]
        group: String,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
}

pub(super) async fn handle_group(
    command: &GroupCommands,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> Result<()> {
    let ctx = RpcCtx { run_as, ucan_path, dir };
    match command {
        GroupCommands::Create { name, gateway_url, host } => {
            let mut params = json!({});
            if let Some(n) = name {
                params["name"] = json!(n);
            }
            call_and_print(ctx, gateway_url, host.as_deref(), "group.create", params).await
        }
        GroupCommands::Rename { group, name, gateway_url, host } => {
            let params = json!({ "conversation": group, "name": name });
            call_and_print(ctx, gateway_url, host.as_deref(), "group.rename", params).await
        }
        GroupCommands::Add { group, address, person_did, gateway_url, host } => {
            if address.is_none() && person_did.is_none() {
                bail!("either --address or --person-did is required");
            }
            let mut params = json!({ "conversation": group });
            if let Some(a) = address {
                params["address"] = json!(a);
            }
            if let Some(d) = person_did {
                params["person_did"] = json!(d);
            }
            call_and_print(ctx, gateway_url, host.as_deref(), "group.add-member", params).await
        }
        GroupCommands::Remove { group, address, person_did, gateway_url, host } => {
            if address.is_none() && person_did.is_none() {
                bail!("either --address or --person-did is required");
            }
            let mut params = json!({ "conversation": group });
            if let Some(a) = address {
                params["address"] = json!(a);
            }
            if let Some(d) = person_did {
                params["person_did"] = json!(d);
            }
            call_and_print(ctx, gateway_url, host.as_deref(), "group.remove-member", params).await
        }
        GroupCommands::List { include_hidden, gateway_url, host } => {
            let params = json!({ "kind": "group", "include_hidden": include_hidden });
            call_and_print(ctx, gateway_url, host.as_deref(), "conversation.list", params).await
        }
        GroupCommands::Info { group, gateway_url, host } => {
            let params = json!({ "conversation": group });
            call_and_print(ctx, gateway_url, host.as_deref(), "group.info", params).await
        }
        GroupCommands::Send { group, body, gateway_url, host } => {
            let params = json!({ "conversation": group, "body": body });
            call_and_print(ctx, gateway_url, host.as_deref(), "conversation.send", params).await
        }
        GroupCommands::History { group, limit, gateway_url, host } => {
            let mut params = json!({ "conversation": group });
            if let Some(l) = limit {
                params["limit"] = json!(l);
            }
            call_and_print(ctx, gateway_url, host.as_deref(), "conversation.history", params).await
        }
        GroupCommands::Sync { group, gateway_url, host } => {
            let params = json!({ "conversation": group });
            call_and_print(ctx, gateway_url, host.as_deref(), "group.sync", params).await
        }
        GroupCommands::Check { group, gateway_url, host } => {
            let params = json!({ "conversation": group });
            call_and_print(
                ctx,
                gateway_url,
                host.as_deref(),
                "conversation.transcript-digest",
                params,
            )
            .await
        }
        GroupCommands::Hide { group, gateway_url, host } => {
            let params = json!({ "conversation": group });
            call_and_print(ctx, gateway_url, host.as_deref(), "group.hide", params).await
        }
        GroupCommands::Unhide { group, gateway_url, host } => {
            let params = json!({ "conversation": group });
            call_and_print(ctx, gateway_url, host.as_deref(), "group.unhide", params).await
        }
    }
}
