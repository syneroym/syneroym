//! Transaction subcommands for Roym.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use clap::Subcommand;
use serde_json::{Value, json};
use syneroym_roym_core::{money, transaction};

use super::{
    directory::{RpcCtx, call_and_print},
    parse_near,
};
use crate::{DEFAULT_GATEWAY_URL, commands::session};

#[derive(Subcommand, Debug, Clone)]
pub enum TransactionCommands {
    /// Send a signed request card in a conversation.
    Request {
        #[arg(long)]
        conversation: String,
        #[arg(long)]
        description: String,
        #[arg(long = "category")]
        categories: Vec<String>,
        #[arg(long)]
        listing: Option<String>,
        /// `lat,lon,radius_m` in decimal degrees and metres; converted to
        /// integer micro-degrees at this boundary, never signed as a
        /// decimal.
        #[arg(long)]
        near: Option<String>,
        /// `earliest,latest` in unix seconds.
        #[arg(long)]
        window: Option<String>,
        #[arg(long)]
        notice: Option<String>,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Send a signed quote card answering a request.
    Quote {
        #[arg(long)]
        request: String,
        #[arg(long)]
        scope: String,
        #[arg(long)]
        currency: String,
        #[arg(long)]
        amount: String,
        #[arg(long)]
        payee: String,
        #[arg(long)]
        tax: Option<String>,
        #[arg(long)]
        fees: Option<String>,
        #[arg(long = "method")]
        methods: Vec<String>,
        #[arg(long)]
        timing: String,
        /// `earliest,latest` in unix seconds.
        #[arg(long)]
        schedule: Option<String>,
        #[arg(long = "where")]
        where_: String,
        #[arg(long)]
        address: Option<String>,
        #[arg(long)]
        cancellation_file: PathBuf,
        #[arg(long)]
        refund_file: PathBuf,
        #[arg(long)]
        dispute: String,
        #[arg(long)]
        expires_hours: u64,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Accept an offered quote, signing an agreement receipt.
    Accept {
        #[arg(long)]
        quote: String,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Decline a quote. A declined quote is hidden from view; the other side is
    /// not told.
    Decline {
        #[arg(long)]
        quote: String,
        #[arg(long)]
        note: Option<String>,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Ingest cards from the conversation history into transaction state.
    Sync {
        #[arg(long)]
        conversation: String,
        #[arg(long)]
        full: bool,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// View cards filed for a conversation in timestamp order.
    Thread {
        #[arg(long)]
        conversation: String,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// View the agreement receipt for a quote.
    Agreement {
        #[arg(long)]
        quote: String,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
}

#[allow(clippy::too_many_arguments)]
async fn handle_request(
    conversation: &str,
    description: &str,
    categories: &[String],
    listing: Option<&str>,
    near: Option<&str>,
    window: Option<&str>,
    notice: Option<&str>,
    gateway_url: &str,
    host: Option<&str>,
    ctx: RpcCtx<'_>,
) -> Result<()> {
    let data_use_notice = notice
        .map(str::to_string)
        .unwrap_or_else(|| transaction::DEFAULT_DATA_USE_NOTICE.to_string());
    println!("{data_use_notice}");

    let mut params = json!({
        "conversation": conversation,
        "description": description,
        "categories": categories,
        "data_use_notice": data_use_notice,
    });
    if let Some(l) = listing {
        params["listing_id"] = json!(l);
    }
    if let Some(n) = near {
        params["area"] = parse_near(n)?;
    }
    if let Some(w) = window {
        params["window"] = parse_window(w)?;
    }

    call_and_print(ctx, gateway_url, host, "request.set", params).await
}

/// Validate a quote's terms and build the `quote.set` params object:
/// resolve the currency's minor-unit exponent, parse the amount/tax/fees
/// against it, check `--timing`/`--where` are one of their fixed values,
/// and read the cancellation/refund terms files.
#[allow(clippy::too_many_arguments)]
fn build_quote_params(
    request: &str,
    scope: &str,
    currency: &str,
    amount: &str,
    payee: &str,
    tax: Option<&str>,
    fees: Option<&str>,
    methods: &[String],
    timing: &str,
    schedule: Option<&str>,
    where_: &str,
    address: Option<&str>,
    cancellation_file: &Path,
    refund_file: &Path,
    dispute: &str,
    expires_hours: u64,
) -> Result<Value> {
    let curr = currency.trim().to_uppercase();
    let exp = money::currency_minor_exponent(&curr).ok_or_else(|| {
        anyhow::anyhow!("currency '{currency}' is not a currency code this build knows")
    })?;
    let amount_minor = parse_minor_units(amount, &curr, exp)?;
    let tax_minor = match tax {
        Some(t) => parse_minor_units(t, &curr, exp)?,
        None => 0,
    };
    let fees_minor = match fees {
        Some(f) => parse_minor_units(f, &curr, exp)?,
        None => 0,
    };

    let timing_str = timing.trim().to_lowercase();
    if timing_str != "before-work" && timing_str != "after-work" {
        anyhow::bail!("--timing must be 'before-work' or 'after-work'");
    }

    let where_str = where_.trim().to_lowercase();
    if where_str != "at-provider" && where_str != "at-customer" && where_str != "remote" {
        anyhow::bail!("--where must be 'at-provider', 'at-customer', or 'remote'");
    }

    if address.is_some() {
        println!("{}", transaction::ADDRESS_DISCLOSURE_NOTICE);
    }

    let cancellation_terms = fs::read_to_string(cancellation_file)
        .with_context(|| format!("reading cancellation file {}", cancellation_file.display()))?;
    let refund_terms = fs::read_to_string(refund_file)
        .with_context(|| format!("reading refund file {}", refund_file.display()))?;

    let mut location = json!({ "where": where_str });
    if let Some(addr) = address {
        location["address"] = json!(addr);
    }

    let mut terms = json!({
        "scope": scope,
        "currency": curr,
        "amount_minor": amount_minor,
        "tax_minor": tax_minor,
        "fees_minor": fees_minor,
        "payment_methods": methods,
        "payee": payee,
        "payment_timing": timing_str,
        "location": location,
        "cancellation_terms": cancellation_terms,
        "refund_terms": refund_terms,
        "dispute_path": dispute,
    });
    if let Some(s) = schedule {
        terms["schedule"] = parse_window(s)?;
    }

    Ok(json!({
        "request_record_id": request,
        "expires_in_secs": expires_hours * 3600,
        "terms": terms,
    }))
}

async fn handle_decline(
    quote: &str,
    note: Option<&str>,
    gateway_url: &str,
    host: Option<&str>,
    ctx: RpcCtx<'_>,
) -> Result<()> {
    println!("A declined quote is hidden from view; the other side is not told.");
    let mut params = json!({ "quote_record_id": quote });
    if let Some(n) = note {
        params["note"] = json!(n);
    }
    call_and_print(ctx, gateway_url, host, "quote.decline", params).await
}

/// Print a conversation's cards in three trust-tier blocks: verified,
/// refused (a known type that failed verification), and unknown type -- a
/// CLI that prints only the good news hides exactly what the Hub is
/// required to show.
fn print_thread_cards(cards: &[Value]) {
    let verified: Vec<_> = cards
        .iter()
        .filter(|c| c.get("verified").and_then(Value::as_bool).unwrap_or(false))
        .collect();
    let refused: Vec<_> = cards
        .iter()
        .filter(|c| {
            c.get("known").and_then(Value::as_bool).unwrap_or(false)
                && !c.get("verified").and_then(Value::as_bool).unwrap_or(false)
        })
        .collect();
    let unknown: Vec<_> =
        cards.iter().filter(|c| !c.get("known").and_then(Value::as_bool).unwrap_or(true)).collect();

    println!("{} verified card(s):", verified.len());
    for c in &verified {
        let card_type = c.get("card_type").and_then(Value::as_str).unwrap_or("?");
        let version = c.get("version").and_then(Value::as_u64).unwrap_or(0);
        let msg_id = c.get("message_id").and_then(Value::as_str).unwrap_or("?");
        let issuer = c.get("issuer").and_then(Value::as_str).unwrap_or("?");
        let record_id = c.get("record_id").and_then(Value::as_str).unwrap_or("?");
        let declined_tag = if c.get("declined").and_then(Value::as_bool).unwrap_or(false) {
            " [declined]"
        } else {
            ""
        };
        let data_str =
            c.get("data").map(|d| serde_json::to_string(d).unwrap_or_default()).unwrap_or_default();
        println!(
            "- [{msg_id}] {card_type} v{version} by {issuer}, record: {record_id}{declined_tag}, \
             data: {data_str}"
        );
    }

    if !refused.is_empty() {
        println!(
            "\n{} refused card(s) (never trusted, shown so you know they were delivered):",
            refused.len()
        );
        for c in &refused {
            let msg_id = c.get("message_id").and_then(Value::as_str).unwrap_or("?");
            let card_type = c.get("card_type").and_then(Value::as_str).unwrap_or("?");
            let reason = c.get("reason").and_then(Value::as_str).unwrap_or("unknown reason");
            println!("- [{msg_id}] {card_type}: refused: {reason}");
        }
    }

    if !unknown.is_empty() {
        println!("\n{} unknown-type card(s):", unknown.len());
        for c in &unknown {
            let msg_id = c.get("message_id").and_then(Value::as_str).unwrap_or("?");
            let card_type = c.get("card_type").and_then(Value::as_str).unwrap_or("?");
            let version = c.get("version").and_then(Value::as_u64).unwrap_or(0);
            println!("- [{msg_id}] {card_type} v{version}: unknown card type");
        }
    }
}

async fn handle_thread(
    conversation: &str,
    gateway_url: &str,
    host: Option<&str>,
    ctx: RpcCtx<'_>,
) -> Result<()> {
    let v = session::rpc_call(
        gateway_url,
        host,
        ctx.run_as,
        ctx.ucan_path,
        ctx.dir,
        "transaction.thread",
        json!({ "conversation": conversation }),
    )
    .await?;

    let empty = vec![];
    let cards = v.get("cards").and_then(Value::as_array).unwrap_or(&empty);
    print_thread_cards(cards);
    Ok(())
}

pub(super) async fn handle_transaction(
    command: &TransactionCommands,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> Result<()> {
    let ctx = RpcCtx { run_as, ucan_path, dir };
    match command {
        TransactionCommands::Request {
            conversation,
            description,
            categories,
            listing,
            near,
            window,
            notice,
            gateway_url,
            host,
        } => {
            handle_request(
                conversation,
                description,
                categories,
                listing.as_deref(),
                near.as_deref(),
                window.as_deref(),
                notice.as_deref(),
                gateway_url,
                host.as_deref(),
                ctx,
            )
            .await?;
        }
        TransactionCommands::Quote {
            request,
            scope,
            currency,
            amount,
            payee,
            tax,
            fees,
            methods,
            timing,
            schedule,
            where_,
            address,
            cancellation_file,
            refund_file,
            dispute,
            expires_hours,
            gateway_url,
            host,
        } => {
            let params = build_quote_params(
                request,
                scope,
                currency,
                amount,
                payee,
                tax.as_deref(),
                fees.as_deref(),
                methods,
                timing,
                schedule.as_deref(),
                where_,
                address.as_deref(),
                cancellation_file,
                refund_file,
                dispute,
                *expires_hours,
            )?;
            call_and_print(ctx, gateway_url, host.as_deref(), "quote.set", params).await?;
        }
        TransactionCommands::Accept { quote, gateway_url, host } => {
            let params = json!({ "quote_record_id": quote });
            call_and_print(ctx, gateway_url, host.as_deref(), "agreement.accept", params).await?;
        }
        TransactionCommands::Decline { quote, note, gateway_url, host } => {
            handle_decline(quote, note.as_deref(), gateway_url, host.as_deref(), ctx).await?;
        }
        TransactionCommands::Sync { conversation, full, gateway_url, host } => {
            let params = json!({ "conversation": conversation, "full": full });
            call_and_print(ctx, gateway_url, host.as_deref(), "transaction.sync", params).await?;
        }
        TransactionCommands::Thread { conversation, gateway_url, host } => {
            handle_thread(conversation, gateway_url, host.as_deref(), ctx).await?;
        }
        TransactionCommands::Agreement { quote, gateway_url, host } => {
            let params = json!({ "quote_record_id": quote });
            call_and_print(ctx, gateway_url, host.as_deref(), "agreement.get", params).await?;
        }
    }
    Ok(())
}

/// Parses `earliest,latest` unix timestamps at this boundary.
pub(crate) fn parse_window(input: &str) -> Result<serde_json::Value> {
    let parts: Vec<&str> = input.split(',').collect();
    let [earliest, latest] = parts.as_slice() else {
        anyhow::bail!("expected earliest,latest unix seconds");
    };
    let earliest: u64 = earliest.trim().parse().context("invalid earliest unix seconds")?;
    let latest: u64 = latest.trim().parse().context("invalid latest unix seconds")?;
    if earliest > latest {
        anyhow::bail!("earliest cannot be after latest");
    }
    Ok(json!({
        "earliest_secs": earliest,
        "latest_secs": latest,
    }))
}

/// Parses a decimal amount string to minor units integer using currency
/// exponent.
pub(crate) fn parse_minor_units(input: &str, currency: &str, exp: u32) -> Result<i64> {
    let t = input.trim();
    if t.is_empty() {
        anyhow::bail!("amount cannot be empty");
    }
    let parts: Vec<&str> = t.split('.').collect();
    let (whole, frac) = match parts.as_slice() {
        [w] => (*w, ""),
        [w, f] => {
            if exp == 0 {
                anyhow::bail!("currency '{currency}' has no minor units, expected integer like 12");
            }
            if f.len() > exp as usize {
                anyhow::bail!(
                    "amount '{input}' has more decimal places than currency '{currency}' ({exp})"
                );
            }
            (*w, *f)
        }
        _ => anyhow::bail!("invalid amount '{input}'"),
    };
    if whole.is_empty()
        || !whole.chars().all(|c| c.is_ascii_digit())
        || !frac.chars().all(|c| c.is_ascii_digit())
    {
        anyhow::bail!("invalid amount '{input}'");
    }
    let mut minor_str = whole.to_string();
    minor_str.push_str(frac);
    for _ in 0..(exp as usize - frac.len()) {
        minor_str.push('0');
    }
    let minor: i64 = minor_str.parse().context("amount out of range")?;
    Ok(minor)
}
