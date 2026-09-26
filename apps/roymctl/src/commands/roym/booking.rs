//! Booking, payment, and fulfilment subcommands for Roym.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use clap::Subcommand;
use serde_json::json;
use syneroym_roym_core::booking;

use super::directory::{RpcCtx, call_and_print};
use crate::DEFAULT_GATEWAY_URL;

#[derive(Subcommand, Debug, Clone)]
pub enum BookingCommands {
    /// Get booking progress and next expected step.
    Get {
        #[arg(long)]
        agreement: String,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// List bookings for this installation.
    List {
        #[arg(long)]
        agreement: Option<String>,
        #[arg(long)]
        conversation: Option<String>,
        #[arg(long)]
        state: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long, default_value_t = 0)]
        offset: usize,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Provider starts work on a scheduled booking.
    Start {
        #[arg(long)]
        agreement: String,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Provider cancels a scheduled booking.
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
    /// View booking progress history.
    History {
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
    /// Request payment from consumer.
    Request {
        #[arg(long)]
        agreement: String,
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
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Verify a payment record envelope.
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
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Get fulfilment receipts for an agreement.
    Get {
        #[arg(long)]
        agreement: String,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
}

pub(super) async fn handle_booking(
    cmd: &BookingCommands,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> Result<()> {
    let ctx = RpcCtx { run_as, ucan_path, dir };
    match cmd {
        BookingCommands::Get { agreement, gateway_url, host } => {
            println!("{}", booking::PROGRESS_NOTICE);
            let params = json!({ "agreement": agreement });
            call_and_print(ctx, gateway_url, host.as_deref(), "booking.get", params).await
        }
        BookingCommands::List {
            agreement,
            conversation,
            state,
            limit,
            offset,
            gateway_url,
            host,
        } => {
            let mut params = json!({ "limit": limit, "offset": offset });
            if let Some(a) = agreement {
                params["agreement"] = json!(a);
            }
            if let Some(c) = conversation {
                params["conversation"] = json!(c);
            }
            if let Some(s) = state {
                params["state"] = json!(s);
            }
            call_and_print(ctx, gateway_url, host.as_deref(), "booking.list", params).await
        }
        BookingCommands::Start { agreement, gateway_url, host } => {
            let params = json!({ "agreement": agreement });
            call_and_print(ctx, gateway_url, host.as_deref(), "booking.start", params).await
        }
        BookingCommands::Cancel { agreement, reason, gateway_url, host } => {
            let params = json!({ "agreement": agreement, "reason": reason });
            call_and_print(ctx, gateway_url, host.as_deref(), "booking.cancel", params).await
        }
        BookingCommands::History { agreement, gateway_url, host } => {
            let params = json!({ "agreement": agreement });
            call_and_print(ctx, gateway_url, host.as_deref(), "booking.history", params).await
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
        PaymentCommands::Request { agreement, note, gateway_url, host } => {
            let mut params = json!({ "agreement": agreement });
            if let Some(n) = note {
                params["note"] = json!(n);
            }
            call_and_print(ctx, gateway_url, host.as_deref(), "payment.request", params).await
        }
        PaymentCommands::Ack {
            agreement,
            observed_at,
            method,
            reference,
            supersedes,
            gateway_url,
            host,
        } => {
            let mut params = json!({ "agreement": agreement });
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
        PaymentCommands::Get { agreement, gateway_url, host } => {
            println!("{}", booking::PAYMENT_NOTICE);
            let params = json!({ "agreement": agreement });
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
        FulfilmentCommands::Sign { agreement, gateway_url, host } => {
            let params = json!({ "agreement": agreement });
            call_and_print(ctx, gateway_url, host.as_deref(), "fulfilment.sign", params).await
        }
        FulfilmentCommands::Get { agreement, gateway_url, host } => {
            let params = json!({ "agreement": agreement });
            call_and_print(ctx, gateway_url, host.as_deref(), "fulfilment.get", params).await
        }
    }
}
