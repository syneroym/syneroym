//! Agreement, payment, and fulfilment subcommands for Roym.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use clap::Subcommand;
use serde_json::{Value, json};
use syneroym_roym_core::progress;

use super::directory::{RpcCtx, call_and_print};
use crate::{DEFAULT_GATEWAY_URL, commands::session};

#[derive(Subcommand, Debug, Clone)]
pub enum AgreementCommands {
    /// Get agreement progress and next expected step.
    Get {
        #[arg(long)]
        agreement: Option<String>,
        #[arg(long)]
        quote: Option<String>,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// List agreements for this installation.
    List {
        #[arg(long)]
        conversation: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long, default_value_t = 0)]
        offset: usize,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Cancel an agreement.
    Cancel {
        #[arg(long)]
        agreement: String,
        #[arg(long)]
        reason: String,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// View agreement progress snapshot history.
    Progress {
        #[arg(long)]
        agreement: String,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum PaymentCommands {
    /// Request payment from the counterparty.
    Request {
        #[arg(long)]
        agreement: String,
        #[arg(long)]
        commitment: Option<String>,
        #[arg(long)]
        note: Option<String>,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Acknowledge payment sent or received.
    #[command(alias = "acknowledge")]
    Ack {
        #[arg(long)]
        agreement: String,
        #[arg(long)]
        commitment: Option<String>,
        #[arg(long)]
        observed_at: Option<u64>,
        #[arg(long)]
        method: Option<String>,
        #[arg(long)]
        reference: Option<String>,
        #[arg(long)]
        supersedes: Option<String>,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Get payment status and receipts for an agreement.
    Get {
        #[arg(long)]
        agreement: String,
        #[arg(long)]
        commitment: Option<String>,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Verify a payment record envelope (output is the signed form, with
    /// digests in place of hidden fields).
    Verify {
        #[arg(long)]
        envelope: Option<String>,
        #[arg(long)]
        envelope_file: Option<PathBuf>,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum FulfilmentCommands {
    /// Sign a fulfilment receipt.
    Sign {
        #[arg(long)]
        agreement: String,
        #[arg(long)]
        commitment: Option<String>,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Get fulfilment receipts for an agreement.
    Get {
        #[arg(long)]
        agreement: String,
        #[arg(long)]
        commitment: Option<String>,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
}

async fn resolve_commitment_id(
    ctx: RpcCtx<'_>,
    gateway_url: &str,
    host: Option<&str>,
    agreement: &str,
    target_kind: &str,
) -> Result<String> {
    let v = session::rpc_call(
        gateway_url,
        host,
        ctx.run_as,
        ctx.ucan_path,
        ctx.dir,
        "agreement.get",
        json!({ "agreement": agreement }),
    )
    .await?;
    let empty = vec![];
    let commitments = v
        .get("terms")
        .and_then(|t| t.get("commitments"))
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    let matching: Vec<&str> = commitments
        .iter()
        .filter(|c| c.get("kind").and_then(Value::as_str).is_some_and(|k| k == target_kind))
        .filter_map(|c| c.get("commitment_id").and_then(Value::as_str))
        .collect();
    match matching.as_slice() {
        [single] => Ok((*single).to_string()),
        [] => anyhow::bail!("agreement has no {target_kind} commitment"),
        _ => anyhow::bail!(
            "agreement has multiple {target_kind} commitments, specify --commitment <id>"
        ),
    }
}

pub(super) async fn handle_agreement(
    cmd: &AgreementCommands,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> Result<()> {
    let ctx = RpcCtx { run_as, ucan_path, dir };
    match cmd {
        AgreementCommands::Get { agreement, quote, gateway_url, host } => {
            println!("{}", progress::PROGRESS_NOTICE);
            let mut params = json!({});
            if let Some(a) = agreement {
                params["agreement"] = json!(a);
            } else if let Some(q) = quote {
                params["quote_record_id"] = json!(q);
            } else {
                anyhow::bail!("either --agreement or --quote is required");
            }
            call_and_print(ctx, gateway_url, host.as_deref(), "agreement.get", params).await
        }
        AgreementCommands::List { conversation, limit, offset, gateway_url, host } => {
            let mut params = json!({ "limit": limit, "offset": offset });
            if let Some(c) = conversation {
                params["conversation"] = json!(c);
            }
            call_and_print(ctx, gateway_url, host.as_deref(), "agreement.list", params).await
        }
        AgreementCommands::Cancel { agreement, reason, gateway_url, host } => {
            let params = json!({ "agreement": agreement, "reason": reason });
            call_and_print(ctx, gateway_url, host.as_deref(), "agreement.cancel", params).await
        }
        AgreementCommands::Progress { agreement, gateway_url, host } => {
            let params = json!({ "agreement": agreement });
            call_and_print(ctx, gateway_url, host.as_deref(), "agreement.progress", params).await
        }
    }
}

pub(super) async fn handle_payment(
    cmd: &PaymentCommands,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> Result<()> {
    let ctx = RpcCtx { run_as, ucan_path, dir };
    match cmd {
        PaymentCommands::Request { agreement, commitment, note, gateway_url, host } => {
            let cid = match commitment {
                Some(c) => c.clone(),
                None => {
                    resolve_commitment_id(ctx, gateway_url, host.as_deref(), agreement, "currency")
                        .await?
                }
            };
            let mut params = json!({ "agreement": agreement, "commitment_id": cid });
            if let Some(n) = note {
                params["note"] = json!({ "lang": "en", "text": n });
            }
            call_and_print(ctx, gateway_url, host.as_deref(), "payment.request", params).await
        }
        PaymentCommands::Ack {
            agreement,
            commitment,
            observed_at,
            method,
            reference,
            supersedes,
            gateway_url,
            host,
        } => {
            let cid = match commitment {
                Some(c) => c.clone(),
                None => {
                    resolve_commitment_id(ctx, gateway_url, host.as_deref(), agreement, "currency")
                        .await?
                }
            };
            let mut params = json!({ "agreement": agreement, "commitment_id": cid });
            if let Some(oa) = observed_at {
                params["observed_at_secs"] = json!(oa);
            }
            if let Some(m) = method {
                params["method"] = json!(m);
            }
            if let Some(r) = reference {
                params["reference"] = json!(r);
            }
            if let Some(s) = supersedes {
                params["supersedes"] = json!(s);
            }
            call_and_print(ctx, gateway_url, host.as_deref(), "payment.acknowledge", params).await
        }
        PaymentCommands::Get { agreement, commitment, gateway_url, host } => {
            println!("{}", progress::PAYMENT_NOTICE);
            let cid = match commitment {
                Some(c) => c.clone(),
                None => {
                    resolve_commitment_id(ctx, gateway_url, host.as_deref(), agreement, "currency")
                        .await?
                }
            };
            let params = json!({ "agreement": agreement, "commitment_id": cid });
            call_and_print(ctx, gateway_url, host.as_deref(), "payment.get", params).await
        }
        PaymentCommands::Verify { envelope, envelope_file, gateway_url, host } => {
            let env_str = match (envelope, envelope_file) {
                (Some(e), _) => e.clone(),
                (None, Some(f)) => fs::read_to_string(f)
                    .with_context(|| format!("reading envelope file {}", f.display()))?,
                (None, None) => anyhow::bail!("either --envelope or --envelope-file is required"),
            };
            let params = json!({ "envelope": env_str });
            call_and_print(ctx, gateway_url, host.as_deref(), "payment.verify", params).await
        }
    }
}

pub(super) async fn handle_fulfilment(
    cmd: &FulfilmentCommands,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> Result<()> {
    let ctx = RpcCtx { run_as, ucan_path, dir };
    match cmd {
        FulfilmentCommands::Sign { agreement, commitment, gateway_url, host } => {
            let cid = match commitment {
                Some(c) => c.clone(),
                None => {
                    resolve_commitment_id(
                        ctx,
                        gateway_url,
                        host.as_deref(),
                        agreement,
                        "fulfilment",
                    )
                    .await?
                }
            };
            let params = json!({ "agreement": agreement, "commitment_id": cid });
            call_and_print(ctx, gateway_url, host.as_deref(), "fulfilment.sign", params).await
        }
        FulfilmentCommands::Get { agreement, commitment, gateway_url, host } => {
            let cid = match commitment {
                Some(c) => c.clone(),
                None => {
                    resolve_commitment_id(
                        ctx,
                        gateway_url,
                        host.as_deref(),
                        agreement,
                        "fulfilment",
                    )
                    .await?
                }
            };
            let params = json!({ "agreement": agreement, "commitment_id": cid });
            call_and_print(ctx, gateway_url, host.as_deref(), "fulfilment.get", params).await
        }
    }
}
