//! Transaction subcommands for Roym.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use serde_json::{Value, json};
use syneroym_roym_core::{
    extension::{SCHEDULING_EXTENSION, SCHEDULING_SLOT_SCHEMA},
    lang::LangTag,
    money, transaction,
};

use super::{
    directory::{RpcCtx, call_and_print},
    parse_near,
};
use crate::{DEFAULT_GATEWAY_URL, commands::session};

#[derive(Args, Debug, Clone)]
pub struct QuoteArgs {
    #[arg(long)]
    pub enquiry: String,
    #[arg(long)]
    pub scope: String,
    #[arg(long)]
    pub lang: String,
    #[arg(long)]
    pub currency: String,
    #[arg(long)]
    pub amount: String,
    #[arg(long)]
    pub payee: String,
    #[arg(long)]
    pub tax: Option<String>,
    #[arg(long)]
    pub fees: Option<String>,
    #[arg(long = "method")]
    pub methods: Vec<String>,
    #[arg(long)]
    pub timing: String,
    /// `earliest,latest` in unix seconds.
    #[arg(long)]
    pub schedule: Option<String>,
    #[arg(long = "where")]
    pub where_: String,
    #[arg(long)]
    pub address: Option<String>,
    #[arg(long)]
    pub cancellation_file: PathBuf,
    #[arg(long)]
    pub refund_file: PathBuf,
    #[arg(long)]
    pub dispute: String,
    #[arg(long)]
    pub expires_hours: u64,
    #[arg(long)]
    pub listing_id: Option<String>,
    #[arg(long)]
    pub slot: Option<String>,
    #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
    pub gateway_url: String,
    #[arg(long)]
    pub host: Option<String>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum TransactionCommands {
    /// Send a signed enquiry card in a conversation.
    Enquiry {
        #[arg(long)]
        conversation: String,
        #[arg(long)]
        description: String,
        #[arg(long)]
        lang: String,
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
    Quote(Box<QuoteArgs>),
    /// Accept an offered quote, signing an agreement receipt.
    Accept {
        #[arg(long)]
        quote: String,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Decline a deal. A signed decline card is sent to the other side.
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
    /// Agreement operations: get, cancel, progress, list.
    Agreement {
        #[command(subcommand)]
        command: super::agreement::AgreementCommands,
    },
    /// Payment operations: request, ack, get, verify.
    Payment {
        #[command(subcommand)]
        command: super::agreement::PaymentCommands,
    },
    /// Fulfilment operations: sign, get.
    Fulfilment {
        #[command(subcommand)]
        command: super::agreement::FulfilmentCommands,
    },
}

struct EnquiryParamsArgs<'a> {
    conversation: &'a str,
    description: &'a str,
    lang: &'a str,
    categories: &'a [String],
    listing: Option<&'a str>,
    near: Option<&'a str>,
    window: Option<&'a str>,
    notice: Option<&'a str>,
}

fn build_enquiry_params(args: EnquiryParamsArgs<'_>) -> Result<Value> {
    let parsed_lang = LangTag::parse(args.lang)?;
    let data_use_notice = args
        .notice
        .map(str::to_string)
        .unwrap_or_else(|| transaction::DEFAULT_DATA_USE_NOTICE.to_string());

    let mut params = json!({
        "conversation": args.conversation,
        "description": {
            "lang": parsed_lang.as_str(),
            "text": args.description,
        },
        "categories": args.categories,
        "data_use_notice": data_use_notice,
    });
    if let Some(l) = args.listing {
        params["listing_id"] = json!(l);
    }
    if let Some(n) = args.near {
        params["area"] = parse_near(n)?;
    }
    if let Some(w) = args.window {
        params["window"] = parse_window(w)?;
    }

    Ok(params)
}

#[allow(clippy::too_many_arguments)]
async fn handle_enquiry(
    conversation: &str,
    description: &str,
    lang: &str,
    categories: &[String],
    listing: Option<&str>,
    near: Option<&str>,
    window: Option<&str>,
    notice: Option<&str>,
    gateway_url: &str,
    host: Option<&str>,
    ctx: RpcCtx<'_>,
) -> Result<()> {
    let data_use_notice = notice.unwrap_or(transaction::DEFAULT_DATA_USE_NOTICE);
    println!("{data_use_notice}");

    let params = build_enquiry_params(EnquiryParamsArgs {
        conversation,
        description,
        lang,
        categories,
        listing,
        near,
        window,
        notice,
    })?;

    call_and_print(ctx, gateway_url, host, "enquiry.set", params).await
}

/// Validate a quote's terms and build the `quote.set` params object:
/// resolve the currency's minor-unit exponent, parse the amount/tax/fees
/// against it, check `--timing`/`--where` are one of their fixed values,
/// and read the cancellation/refund terms files.
#[allow(clippy::too_many_arguments)]
fn build_quote_params(
    enquiry: &str,
    scope: &str,
    lang: &str,
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
    listing_id: Option<&str>,
    slot: Option<&str>,
) -> Result<Value> {
    let parsed_lang = LangTag::parse(lang)?;
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
    if where_str != "at-giver" && where_str != "at-receiver" && where_str != "remote" {
        anyhow::bail!("--where must be 'at-giver', 'at-receiver', or 'remote'");
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

    let schedule_val = match schedule {
        Some(s) => Some(parse_window(s)?),
        None => None,
    };
    let commitments = build_quote_commitments(
        &parsed_lang,
        scope,
        &curr,
        amount,
        amount_minor,
        payee,
        tax_minor,
        fees_minor,
        methods,
        &timing_str,
        location,
        schedule_val,
        listing_id,
        slot,
    );

    let terms = json!({
        "currency": curr,
        "commitments": commitments,
        "scope": { "lang": parsed_lang.as_str(), "text": scope },
        "cancellation_terms": { "lang": parsed_lang.as_str(), "text": cancellation_terms },
        "refund_terms": { "lang": parsed_lang.as_str(), "text": refund_terms },
        "dispute_path": { "lang": parsed_lang.as_str(), "text": dispute },
    });
    let quote_params = json!({
        "enquiry_record_id": enquiry,
        "expires_in_secs": expires_hours * 3600,
        "terms": terms,
    });

    Ok(quote_params)
}

#[allow(clippy::too_many_arguments)]
fn build_quote_commitments(
    parsed_lang: &LangTag,
    scope: &str,
    curr: &str,
    amount: &str,
    amount_minor: i64,
    payee: &str,
    tax_minor: i64,
    fees_minor: i64,
    methods: &[String],
    timing_str: &str,
    location: Value,
    schedule: Option<Value>,
    listing_id: Option<&str>,
    slot: Option<&str>,
) -> Vec<Value> {
    let mut c1 = json!({
        "giver": "quote-author",
        "description": { "lang": parsed_lang.as_str(), "text": scope },
        "declared_value_minor": amount_minor,
        "after": if timing_str == "before-work" && amount_minor > 0 { vec!["c2"] } else { vec![] },
        "location": location,
    });
    if let Some(w) = schedule {
        c1["window"] = w;
    }
    if let Some(sl) = slot {
        c1["resource_kind"] = json!("service-in-slot");
        c1["kind"] = json!("fulfilment");
        c1["extension"] = json!({ "id": SCHEDULING_EXTENSION, "version": 1 });
        c1["details"] = json!({
            "extension": SCHEDULING_EXTENSION,
            "schema": SCHEDULING_SLOT_SCHEMA,
            "data": { "listing_id": listing_id.unwrap_or_default(), "slot_id": sl },
        });
    } else {
        c1["resource_kind"] = json!("goods-or-service");
        c1["kind"] = json!("fulfilment");
    }

    if amount_minor > 0 {
        let curr_desc = format!("{curr} {amount}");
        let c2 = json!({
            "giver": "counterparty",
            "resource_kind": "currency",
            "kind": "currency",
            "description": { "lang": parsed_lang.as_str(), "text": curr_desc },
            "declared_value_minor": amount_minor,
            "payee": payee,
            "payment_methods": methods,
            "tax_minor": tax_minor,
            "fees_minor": fees_minor,
            "after": if timing_str == "after-work" { vec!["c1"] } else { vec![] },
        });
        vec![c1, c2]
    } else {
        vec![c1]
    }
}

async fn handle_decline(
    quote: &str,
    note: Option<&str>,
    gateway_url: &str,
    host: Option<&str>,
    ctx: RpcCtx<'_>,
) -> Result<()> {
    let mut params = json!({ "declines": quote });
    if let Some(n) = note {
        params["note"] = json!(n);
    }
    call_and_print(ctx, gateway_url, host, "deal.decline", params).await
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
        TransactionCommands::Enquiry {
            conversation,
            description,
            lang,
            categories,
            listing,
            near,
            window,
            notice,
            gateway_url,
            host,
        } => {
            handle_enquiry(
                conversation,
                description,
                lang,
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
        TransactionCommands::Quote(args) => {
            let params = build_quote_params(
                &args.enquiry,
                &args.scope,
                &args.lang,
                &args.currency,
                &args.amount,
                &args.payee,
                args.tax.as_deref(),
                args.fees.as_deref(),
                &args.methods,
                &args.timing,
                args.schedule.as_deref(),
                &args.where_,
                args.address.as_deref(),
                &args.cancellation_file,
                &args.refund_file,
                &args.dispute,
                args.expires_hours,
                args.listing_id.as_deref(),
                args.slot.as_deref(),
            )?;
            call_and_print(ctx, &args.gateway_url, args.host.as_deref(), "quote.set", params)
                .await?;
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
        TransactionCommands::Agreement { command } => {
            super::agreement::handle_agreement(command, dir, run_as, ucan_path).await?;
        }
        TransactionCommands::Payment { command } => {
            super::agreement::handle_payment(command, dir, run_as, ucan_path).await?;
        }
        TransactionCommands::Fulfilment { command } => {
            super::agreement::handle_fulfilment(command, dir, run_as, ucan_path).await?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_enquiry_params_validates_lang() {
        let params = build_enquiry_params(EnquiryParamsArgs {
            conversation: "conv1",
            description: "desc",
            lang: "HI-latn",
            categories: &[],
            listing: None,
            near: None,
            window: None,
            notice: None,
        });
        assert!(params.is_ok());
        if let Ok(p) = params {
            assert_eq!(p["description"]["lang"], "hi-Latn");
        }

        let err = build_enquiry_params(EnquiryParamsArgs {
            conversation: "conv1",
            description: "desc",
            lang: "hi_IN",
            categories: &[],
            listing: None,
            near: None,
            window: None,
            notice: None,
        });
        assert!(err.is_err());
        if let Err(e) = err {
            assert!(e.to_string().contains("is not a well-formed BCP 47 tag"));
        }
    }

    #[test]
    fn build_quote_params_validates_lang() {
        let dir = tempfile::tempdir();
        assert!(dir.is_ok());
        let Ok(dir) = dir else { return };
        let cancel_path = dir.path().join("cancel.txt");
        let refund_path = dir.path().join("refund.txt");
        assert!(fs::write(&cancel_path, "cancel terms").is_ok());
        assert!(fs::write(&refund_path, "refund terms").is_ok());

        let params = build_quote_params(
            "req1",
            "scope",
            "HI-latn",
            "EUR",
            "10.00",
            "did:key:payee",
            None,
            None,
            &[],
            "before-work",
            None,
            "remote",
            None,
            &cancel_path,
            &refund_path,
            "arbitration",
            24,
            None,
            None,
        );
        assert!(params.is_ok());
        if let Ok(p) = params {
            assert_eq!(p["terms"]["scope"]["lang"], "hi-Latn");
        }

        let err = build_quote_params(
            "req1",
            "scope",
            "hi_IN",
            "EUR",
            "10.00",
            "did:key:payee",
            None,
            None,
            &[],
            "before-work",
            None,
            "remote",
            None,
            &cancel_path,
            &refund_path,
            "arbitration",
            24,
            None,
            None,
        );
        assert!(err.is_err());
        if let Err(e) = err {
            assert!(e.to_string().contains("is not a well-formed BCP 47 tag"));
        }
    }
}
