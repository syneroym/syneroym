//! Commands specific to the Roym product app.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result};
use clap::Subcommand;
use serde_json::{Value, json};
use syneroym_identity::{DelegationCertificate, Identity, substrate};
use syneroym_roym_core::{money, transaction};
use syneroym_sdk::DeployedService;
use syneroym_signed_record::SCOPE_RECORD_SIGNING;
use tokio::{sync::Semaphore, task::JoinSet};

use crate::DEFAULT_GATEWAY_URL;

#[derive(Subcommand, Debug, Clone)]
pub enum RoymCommands {
    /// Enrol the person's record-signing certificate for Roym services.
    EnrolSigning {
        #[arg(long)]
        master: Option<String>,
        #[arg(long, default_value_t = 720)]
        expires_hours: u64,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
        #[arg(long)]
        registry_url: Option<String>,
    },
    /// Query record-signing status across Roym services.
    SigningStatus {
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Print this installation's own Roym Conversation service id and the
    /// gateway host for the Hub. Paste the service id into `profile.set` as
    /// `conversation_address` so others can message you, without reading a
    /// deploy log. Reads only what `svc list` already reports.
    Address {
        /// The domain the Hub gateway host is served under.
        #[arg(long, default_value = "localhost")]
        domain: String,
    },
    /// The SynOrg / Directory service: publishing a listing, adding
    /// sources, and searching -- the same JSON-RPC API the Hub drives,
    /// through the gateway, with no browser involved.
    Directory {
        #[command(subcommand)]
        command: DirectoryCommands,
    },
    /// The Transaction vertical: requests, quotes, agreements, sync and
    /// threads.
    Transaction {
        #[command(subcommand)]
        command: Box<TransactionCommands>,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum DirectoryCommands {
    /// List the directories this installation has been given.
    Sources {
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Add a directory by its Roym Directory service DID.
    Add {
        did: String,
        #[arg(long)]
        label: Option<String>,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Remove a directory.
    Remove {
        did: String,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Search every added directory, in parallel, and merge the answers.
    /// Prints the verified hits, the refused evidence, and any source
    /// errors as their own blocks -- a CLI that prints only the good news
    /// hides exactly what the Hub is required to show.
    Find {
        #[arg(long)]
        text: Option<String>,
        #[arg(long = "category")]
        categories: Vec<String>,
        /// `lat,lon,radius_m` in decimal degrees and metres; converted to
        /// integer micro-degrees at this boundary, never signed as a
        /// decimal.
        #[arg(long)]
        near: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: u32,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Publish one of this installation's own listings to a chosen
    /// directory.
    Publish {
        listing_id: String,
        #[arg(long)]
        to: String,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Read a directory's own public statement about itself.
    Info {
        did: String,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Create or update this installation's own SynOrg settings -- journey
    /// step S2.
    Serve {
        #[arg(long)]
        name: String,
        #[arg(long)]
        rules_file: PathBuf,
        #[arg(long = "category")]
        categories: Vec<String>,
        #[arg(long)]
        support: String,
        #[arg(long)]
        dispute: String,
        #[arg(long, default_value_t = 30)]
        retention_days: u64,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// The SynOrg's own roster (S4-S6's approval half).
    Member {
        #[command(subcommand)]
        command: MemberCommands,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum MemberCommands {
    Add {
        did: String,
        #[arg(long, default_value = "")]
        note: String,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    Remove {
        did: String,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    List {
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
}

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

/// Every Roym service that signs a record and so needs a record-signing
/// certificate of its own.
const SIGNING_SERVICES: &[&str] = &["profile", "catalog", "conversation", "transaction"];

pub async fn handle(
    command: &RoymCommands,
    api_url: &str,
    substrate_opt: Option<String>,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> Result<()> {
    match command {
        RoymCommands::EnrolSigning { master, expires_hours, gateway_url, host, registry_url } => {
            let master_name = master.as_deref().or(run_as).unwrap_or("owner");
            let key_path = dir.join("identities").join(format!("{master_name}.key"));
            if !key_path.exists() {
                anyhow::bail!(
                    "Master identity '{master_name}' not found at {}",
                    key_path.display()
                );
            }
            let master_identity = Identity::load_from_path(&key_path)?;
            let master_did = substrate::derive_did_key(&master_identity.public_key());

            super::member_identity::refresh_anchor_or_warn(
                registry_url.as_deref(),
                &master_identity,
            )
            .await?;

            let mut failures = 0u32;
            for prefix in SIGNING_SERVICES {
                match enrol_one(
                    prefix,
                    &master_identity,
                    &master_did,
                    master_name,
                    *expires_hours,
                    gateway_url,
                    host.as_deref(),
                    run_as,
                    ucan_path,
                    dir,
                )
                .await
                {
                    Ok(expires_at) => println!("{prefix}: enrolled until timestamp {expires_at}"),
                    Err(e) => {
                        failures += 1;
                        eprintln!("{prefix}: FAILED: {e:#}");
                    }
                }
            }
            if failures > 0 {
                anyhow::bail!("{failures} service(s) failed to enrol");
            }
        }
        RoymCommands::SigningStatus { gateway_url, host } => {
            let mut failures = 0u32;
            for prefix in SIGNING_SERVICES {
                match super::session::rpc_call(
                    gateway_url,
                    host.as_deref(),
                    run_as,
                    ucan_path,
                    dir,
                    &format!("{prefix}.signing-status"),
                    json!({}),
                )
                .await
                {
                    Ok(status_val) => println!(
                        "{prefix}: {}",
                        serde_json::to_string(&status_val).unwrap_or_default()
                    ),
                    Err(e) => {
                        failures += 1;
                        eprintln!("{prefix}: FAILED: {e:#}");
                    }
                }
            }
            if failures > 0 {
                anyhow::bail!("{failures} service(s) failed to report status");
            }
        }
        RoymCommands::Directory { command } => {
            handle_directory(command, dir, run_as, ucan_path).await?;
        }
        RoymCommands::Transaction { command } => {
            handle_transaction(command, dir, run_as, ucan_path).await?;
        }
        RoymCommands::Address { domain } => {
            let substrate_did = super::get_substrate_did(substrate_opt, dir)?;
            let mut client = super::client_for(substrate_did, api_url, dir, run_as, ucan_path)?;
            client.wait_for_ready(Duration::from_secs(5)).await?;

            let svcs = client.list_svcs().await?;
            let conversation_id = find_roym_service(&svcs, "conversation")?;
            let web_id = find_roym_service(&svcs, "web")?;
            let hub_host = syneroym_core::util::generate_service_host(None, &web_id, None, domain)?;

            println!("conversation service id: {conversation_id}");
            println!("  paste this into profile.set as `conversation_address`");
            println!("Hub gateway host:        {hub_host}");
        }
    }
    Ok(())
}

async fn handle_directory(
    command: &DirectoryCommands,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> Result<()> {
    match command {
        DirectoryCommands::Sources { gateway_url, host } => {
            let v = super::session::rpc_call(
                gateway_url,
                host.as_deref(),
                run_as,
                ucan_path,
                dir,
                "directory.sources",
                json!({}),
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
        DirectoryCommands::Add { did, label, gateway_url, host } => {
            let mut params = json!({ "did": did });
            if let Some(l) = label {
                params["label"] = json!(l);
            }
            let v = super::session::rpc_call(
                gateway_url,
                host.as_deref(),
                run_as,
                ucan_path,
                dir,
                "directory.add-source",
                params,
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
        DirectoryCommands::Remove { did, gateway_url, host } => {
            let v = super::session::rpc_call(
                gateway_url,
                host.as_deref(),
                run_as,
                ucan_path,
                dir,
                "directory.remove-source",
                json!({ "did": did }),
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
        DirectoryCommands::Publish { listing_id, to, gateway_url, host } => {
            let v = super::session::rpc_call(
                gateway_url,
                host.as_deref(),
                run_as,
                ucan_path,
                dir,
                "directory.publish-to-source",
                json!({ "listing_id": listing_id, "source": to }),
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
        DirectoryCommands::Info { did, gateway_url, host } => {
            let v = super::session::rpc_call(
                gateway_url,
                host.as_deref(),
                run_as,
                ucan_path,
                dir,
                "directory.probe-info",
                json!({ "did": did }),
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
        DirectoryCommands::Serve {
            name,
            rules_file,
            categories,
            support,
            dispute,
            retention_days,
            gateway_url,
            host,
        } => {
            let rules = fs::read_to_string(rules_file)
                .with_context(|| format!("reading {}", rules_file.display()))?;
            let params = json!({
                "name": name,
                "rules": rules,
                "area": [],
                "categories": categories,
                "support_contact": support,
                "dispute_path": dispute,
                "retention_secs": retention_days * 24 * 3600,
                "publication_limits": { "window_secs": 24 * 3600, "max_per_window": 20 },
            });
            let v = super::session::rpc_call(
                gateway_url,
                host.as_deref(),
                run_as,
                ucan_path,
                dir,
                "directory.set-settings",
                params,
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
        DirectoryCommands::Member { command } => {
            handle_member(command, dir, run_as, ucan_path).await?
        }
        DirectoryCommands::Find { text, categories, near, limit, gateway_url, host } => {
            find(
                text.as_deref(),
                categories,
                near.as_deref(),
                *limit,
                gateway_url,
                host.as_deref(),
                dir,
                run_as,
                ucan_path,
            )
            .await?;
        }
    }
    Ok(())
}

async fn handle_member(
    command: &MemberCommands,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> Result<()> {
    match command {
        MemberCommands::Add { did, note, gateway_url, host } => {
            let v = super::session::rpc_call(
                gateway_url,
                host.as_deref(),
                run_as,
                ucan_path,
                dir,
                "member.add",
                json!({ "did": did, "note": note }),
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
        MemberCommands::Remove { did, gateway_url, host } => {
            let v = super::session::rpc_call(
                gateway_url,
                host.as_deref(),
                run_as,
                ucan_path,
                dir,
                "member.remove",
                json!({ "did": did }),
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
        MemberCommands::List { gateway_url, host } => {
            let v = super::session::rpc_call(
                gateway_url,
                host.as_deref(),
                run_as,
                ucan_path,
                dir,
                "member.list",
                json!({}),
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
    }
    Ok(())
}

async fn handle_transaction(
    command: &TransactionCommands,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> Result<()> {
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
            let data_use_notice =
                notice.clone().unwrap_or_else(|| transaction::DEFAULT_DATA_USE_NOTICE.to_string());
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

            let v = super::session::rpc_call(
                gateway_url,
                host.as_deref(),
                run_as,
                ucan_path,
                dir,
                "request.set",
                params,
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&v)?);
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
            let curr = currency.trim().to_uppercase();
            let exp = money::currency_minor_exponent(&curr).ok_or_else(|| {
                anyhow::anyhow!("currency '{currency}' is not a currency code this build knows")
            })?;
            let amount_minor = parse_minor_units(amount, &curr, exp)?;
            let tax_minor = match tax.as_deref() {
                Some(t) => parse_minor_units(t, &curr, exp)?,
                None => 0,
            };
            let fees_minor = match fees.as_deref() {
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

            let cancellation_terms = fs::read_to_string(cancellation_file).with_context(|| {
                format!("reading cancellation file {}", cancellation_file.display())
            })?;
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

            let params = json!({
                "request_record_id": request,
                "expires_in_secs": expires_hours * 3600,
                "terms": terms,
            });

            let v = super::session::rpc_call(
                gateway_url,
                host.as_deref(),
                run_as,
                ucan_path,
                dir,
                "quote.set",
                params,
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
        TransactionCommands::Accept { quote, gateway_url, host } => {
            let v = super::session::rpc_call(
                gateway_url,
                host.as_deref(),
                run_as,
                ucan_path,
                dir,
                "agreement.accept",
                json!({ "quote_record_id": quote }),
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
        TransactionCommands::Decline { quote, note, gateway_url, host } => {
            println!("A declined quote is hidden from view; the other side is not told.");
            let mut params = json!({ "quote_record_id": quote });
            if let Some(n) = note {
                params["note"] = json!(n);
            }
            let v = super::session::rpc_call(
                gateway_url,
                host.as_deref(),
                run_as,
                ucan_path,
                dir,
                "quote.decline",
                params,
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
        TransactionCommands::Sync { conversation, full, gateway_url, host } => {
            let v = super::session::rpc_call(
                gateway_url,
                host.as_deref(),
                run_as,
                ucan_path,
                dir,
                "transaction.sync",
                json!({ "conversation": conversation, "full": full }),
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
        TransactionCommands::Thread { conversation, gateway_url, host } => {
            let v = super::session::rpc_call(
                gateway_url,
                host.as_deref(),
                run_as,
                ucan_path,
                dir,
                "transaction.thread",
                json!({ "conversation": conversation }),
            )
            .await?;

            let empty = vec![];
            let cards = v.get("cards").and_then(Value::as_array).unwrap_or(&empty);
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
            let unknown: Vec<_> = cards
                .iter()
                .filter(|c| !c.get("known").and_then(Value::as_bool).unwrap_or(true))
                .collect();

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
                let data_str = c
                    .get("data")
                    .map(|d| serde_json::to_string(d).unwrap_or_default())
                    .unwrap_or_default();
                println!(
                    "- [{msg_id}] {card_type} v{version} by {issuer}, record: \
                     {record_id}{declined_tag}, data: {data_str}"
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
                    let reason =
                        c.get("reason").and_then(Value::as_str).unwrap_or("unknown reason");
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
        TransactionCommands::Agreement { quote, gateway_url, host } => {
            let v = super::session::rpc_call(
                gateway_url,
                host.as_deref(),
                run_as,
                ucan_path,
                dir,
                "agreement.get",
                json!({ "quote_record_id": quote }),
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
    }
    Ok(())
}

/// Parses `lat,lon,radius_m` at this boundary and converts to integer
/// micro-degrees: nothing decimal reaches a signed payload, and this is
/// the one place a person's decimal input becomes that integer.
fn parse_near(input: &str) -> Result<serde_json::Value> {
    let parts: Vec<&str> = input.split(',').collect();
    let [lat, lon, radius] = parts.as_slice() else {
        anyhow::bail!("--near expects lat,lon,radius_m");
    };
    let lat: f64 = lat.trim().parse().context("invalid latitude")?;
    let lon: f64 = lon.trim().parse().context("invalid longitude")?;
    let radius: f64 = radius.trim().parse().context("invalid radius_m")?;
    Ok(json!({
        "kind": "circle",
        "lat_e6": (lat * 1e6).round() as i64,
        "lon_e6": (lon * 1e6).round() as i64,
        "radius_m": radius.round() as u64,
    }))
}

/// Parses `earliest,latest` unix timestamps at this boundary.
fn parse_window(input: &str) -> Result<serde_json::Value> {
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
fn parse_minor_units(input: &str, currency: &str, exp: u32) -> Result<i64> {
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

/// What one directory contributed, as this client saw it -- mirrors the
/// Hub's `SourceOutcome`. A `directory.query-source` reply is always a
/// JSON-RPC success whose `result.error` carries any per-source failure;
/// a 503 from this node's own guest-HTTP admission arrives as an `Err`
/// from `rpc_call` instead and maps to `NotStarted`.
enum SourceOutcome {
    Ok { truncated: bool },
    NotStarted,
    Failed { words: String },
}

impl SourceOutcome {
    fn from_reply(reply: anyhow::Result<Value>) -> Self {
        let result = match reply {
            Ok(v) => v,
            Err(e) => {
                // A 503 is this node's own guest-HTTP admission refusing
                // to start the call -- matched on the typed status, not
                // the error's Display text.
                if let Some(http) = e.downcast_ref::<super::session::RpcHttpError>()
                    && http.status == 503
                {
                    return SourceOutcome::NotStarted;
                }
                return SourceOutcome::Failed { words: format!("could not be reached: {e}") };
            }
        };
        let truncated = result.get("truncated").and_then(Value::as_bool).unwrap_or(false);
        match result.get("error") {
            Some(err) if !err.is_null() => {
                let kind = err.get("kind").and_then(Value::as_str).unwrap_or("unreadable");
                SourceOutcome::Failed { words: source_error_words(kind).to_string() }
            }
            _ => SourceOutcome::Ok { truncated },
        }
    }

    /// The line to print for this source, or `None` when it answered
    /// cleanly with nothing worth saying.
    fn note(&self) -> Option<String> {
        match self {
            SourceOutcome::Ok { truncated: false } => None,
            SourceOutcome::Ok { truncated: true } => {
                Some("this directory had more matches than it would return".to_string())
            }
            SourceOutcome::NotStarted => {
                Some("this installation was busy and did not start the call".to_string())
            }
            SourceOutcome::Failed { words } => Some(words.clone()),
        }
    }
}

/// Words for a merged hit's `credential` (membership) verdict. `unknown`
/// -- its only value until a membership-credential source lands -- reads
/// as "not checked"; other values pass through so a real verdict is not
/// hidden behind a constant.
fn membership_words(credential: &str) -> String {
    match credential {
        "unknown" => "not checked".to_string(),
        other => other.to_string(),
    }
}

/// The same wording the Hub shows for each `SourceError` kind.
fn source_error_words(kind: &str) -> &'static str {
    match kind {
        "not-started" => "this installation was busy and did not start the call",
        "timed-out" => "the directory did not answer in time",
        "not-found" => "no directory answers at that address",
        "refused" => "the directory refused the request",
        _ => "the directory's answer could not be read",
    }
}

#[allow(clippy::too_many_arguments)]
async fn find(
    text: Option<&str>,
    categories: &[String],
    near: Option<&str>,
    limit: u32,
    gateway_url: &str,
    host: Option<&str>,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> Result<()> {
    let mut query = json!({ "categories": categories, "limit": limit });
    if let Some(t) = text {
        query["text"] = json!(t);
    }
    if let Some(n) = near {
        query["area"] = parse_near(n)?;
    }

    let start = super::session::rpc_call(
        gateway_url,
        host,
        run_as,
        ucan_path,
        dir,
        "directory.start-run",
        json!({}),
    )
    .await?;
    let run_id =
        start.get("run_id").and_then(|v| v.as_str()).context("start-run: no run_id")?.to_string();
    let sources: Vec<String> = start
        .get("sources")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let max_concurrency =
        start.get("max_concurrency").and_then(|v| v.as_u64()).unwrap_or(1).max(1) as usize;

    if sources.is_empty() {
        println!("No directories added. Add one with `roymctl roym directory add <did>`,");
        println!("or reach a provider directly by link -- a directory is optional.");
    }

    // A continuous worker pool capped at `max_concurrency`, the same
    // shape the Hub's fan-out uses -- not a per-chunk barrier that idles
    // the pool while the slowest source in a chunk finishes.
    let query_one = |source: String| {
        let gateway_url = gateway_url.to_string();
        let host = host.map(str::to_string);
        let dir = dir.to_path_buf();
        let run_as = run_as.map(str::to_string);
        let ucan_path = ucan_path.map(|p| p.to_path_buf());
        let query = query.clone();
        let run_id = run_id.clone();
        async move {
            let result = super::session::rpc_call(
                &gateway_url,
                host.as_deref(),
                run_as.as_deref(),
                ucan_path.as_deref(),
                &dir,
                "directory.query-source",
                json!({ "run_id": run_id, "source": source, "query": query }),
            )
            .await;
            (source, SourceOutcome::from_reply(result))
        }
    };

    let permits = Arc::new(Semaphore::new(max_concurrency.max(1)));
    let mut set = JoinSet::new();
    for source in &sources {
        // The semaphore is never closed, so acquire only ever succeeds;
        // if it somehow did not, running the source unbounded is a safe
        // fallback.
        let permit = permits.clone().acquire_owned().await.ok();
        let fut = query_one(source.clone());
        set.spawn(async move {
            let out = fut.await;
            drop(permit);
            out
        });
    }
    let mut outcomes: Vec<(String, SourceOutcome)> = Vec::new();
    while let Some(joined) = set.join_next().await {
        if let Ok(pair) = joined {
            outcomes.push(pair);
        }
    }

    // A source this node refused to start (a 503 from guest-HTTP
    // admission) is retried once, serially, now that the fan-out's
    // permits are free again -- the same single retry the Hub does.
    let retry: Vec<String> = outcomes
        .iter()
        .filter(|(_, o)| matches!(o, SourceOutcome::NotStarted))
        .map(|(s, _)| s.clone())
        .collect();
    for source in retry {
        let (_, again) = query_one(source.clone()).await;
        if let Some(slot) = outcomes.iter_mut().find(|(s, _)| *s == source) {
            slot.1 = again;
        }
    }

    for (source, outcome) in &outcomes {
        if let Some(line) = outcome.note() {
            println!("source {source}: {line}");
        }
    }

    let merged = super::session::rpc_call(
        gateway_url,
        host,
        run_as,
        ucan_path,
        dir,
        "directory.merge",
        json!({ "run_id": run_id }),
    )
    .await?;

    let empty = vec![];
    let hits = merged.get("hits").and_then(|v| v.as_array()).unwrap_or(&empty);
    println!("{} result(s):", hits.len());
    for hit in hits {
        let listing_id = hit.get("listing_id").and_then(|v| v.as_str()).unwrap_or("?");
        let title = hit.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let issuer = hit.get("issuer").and_then(|v| v.as_str()).unwrap_or("?");
        let age = hit.get("age_secs").and_then(|v| v.as_u64()).unwrap_or(0);
        let revocation = hit.get("revocation_status").and_then(|v| v.as_str()).unwrap_or("unknown");
        // `credential` is the membership verdict `merge` carries. "unknown"
        // (its only value in R1) renders as "not checked"; a later slice
        // adds real values without changing the field, and each gets its
        // own word here rather than a hardcoded string swallowing it.
        let membership =
            membership_words(hit.get("credential").and_then(|v| v.as_str()).unwrap_or("unknown"));
        let sources_val = hit.get("sources").cloned().unwrap_or_default();
        println!(
            "- {listing_id} \"{title}\" by {issuer}, age {age}s, revocation: {revocation}, \
             membership: {membership}, sources: {sources_val}"
        );
    }

    let empty_refused = vec![];
    let refused = merged.get("refused").and_then(|v| v.as_array()).unwrap_or(&empty_refused);
    if !refused.is_empty() {
        println!(
            "\n{} refused (never trusted, shown so you know a directory served them):",
            refused.len()
        );
        for r in refused {
            println!("- {}", serde_json::to_string(r)?);
        }
    }
    Ok(())
}

/// The physical service id of a Roym logical service, found by the app
/// interface it registers (`syneroym-roym:<name>/...`). Reads only what
/// `svc list` already returns, so it invents no resolution path: no host
/// surface reports a service its own routing address, so a person would
/// otherwise have to read it out of a deploy log.
fn find_roym_service(svcs: &[DeployedService], name: &str) -> Result<String> {
    let prefix = format!("syneroym-roym:{name}/");
    let matches: Vec<&str> = svcs
        .iter()
        .filter(|s| s.interfaces.iter().any(|i| i.starts_with(&prefix)))
        .map(|s| s.service_id.as_str())
        .collect();
    match matches.as_slice() {
        [] => anyhow::bail!(
            "no Roym '{name}' service is deployed on this installation -- deploy the Roym app \
             first"
        ),
        [only] => Ok((*only).to_string()),
        many => anyhow::bail!(
            "{} Roym '{name}' services are deployed; cannot choose one address",
            many.len()
        ),
    }
}

#[allow(clippy::too_many_arguments)]
async fn enrol_one(
    prefix: &str,
    master_identity: &Identity,
    master_did: &str,
    master_name: &str,
    expires_hours: u64,
    gateway_url: &str,
    host: Option<&str>,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
    dir: &Path,
) -> Result<u64> {
    let status_val = super::session::rpc_call(
        gateway_url,
        host,
        run_as,
        ucan_path,
        dir,
        &format!("{prefix}.signing-status"),
        json!({}),
    )
    .await
    .with_context(|| format!("failed to fetch {prefix}.signing-status"))?;

    if let Some(recorded_owner) = status_val.get("owner_did").and_then(|v| v.as_str())
        && recorded_owner != master_did
    {
        anyhow::bail!(
            "this installation's recorded owner is '{recorded_owner}', but master identity \
             '{master_name}' has DID '{master_did}'"
        );
    }

    let signing_did_str = status_val
        .get("signing_did")
        .and_then(|v| v.as_str())
        .context("signing-status output missing 'signing_did'")?;
    let signing_pubkey =
        substrate::resolve_did_key(signing_did_str).context("failed to resolve signing_did")?;

    let cert = DelegationCertificate::issue(
        master_identity,
        signing_pubkey,
        expires_hours * 3600,
        SCOPE_RECORD_SIGNING.to_string(),
    )?;

    super::session::rpc_call(
        gateway_url,
        host,
        run_as,
        ucan_path,
        dir,
        &format!("{prefix}.install-signing-certificate"),
        json!({ "certificate": cert.to_json()? }),
    )
    .await
    .with_context(|| format!("failed to install {prefix} signing certificate"))?;

    Ok(cert.expires_at_secs)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn svc(service_id: &str, interfaces: &[&str]) -> DeployedService {
        serde_json::from_value(json!({
            "service_id": service_id,
            "interfaces": interfaces,
            "endpoint_type": "wasm",
        }))
        .unwrap()
    }

    fn roym_svcs() -> Vec<DeployedService> {
        vec![
            svc("did:key:zWeb", &["syneroym-roym:web/api@0.1.0"]),
            svc("did:key:zProfile", &["syneroym-roym:profile/api@0.1.0"]),
            svc("did:key:zConv", &["syneroym-roym:conversation/api@0.1.0"]),
            svc("did:key:zOther", &["syneroym:http/incoming-handler@0.2.0"]),
        ]
    }

    #[test]
    fn find_roym_service_matches_by_app_interface() {
        let svcs = roym_svcs();
        assert_eq!(find_roym_service(&svcs, "conversation").unwrap(), "did:key:zConv");
        assert_eq!(find_roym_service(&svcs, "web").unwrap(), "did:key:zWeb");
    }

    #[test]
    fn find_roym_service_errors_when_absent() {
        let err = find_roym_service(&roym_svcs(), "directory").unwrap_err().to_string();
        assert!(err.contains("no Roym 'directory' service is deployed"), "{err}");
    }

    #[test]
    fn find_roym_service_errors_when_ambiguous() {
        let svcs = vec![
            svc("did:key:zConvA", &["syneroym-roym:conversation/api@0.1.0"]),
            svc("did:key:zConvB", &["syneroym-roym:conversation/api@0.1.0"]),
        ];
        let err = find_roym_service(&svcs, "conversation").unwrap_err().to_string();
        assert!(err.contains("2 Roym 'conversation' services"), "{err}");
    }

    #[test]
    fn parse_window_valid_and_invalid() {
        let v = parse_window("100,200").unwrap();
        assert_eq!(v["earliest_secs"], 100);
        assert_eq!(v["latest_secs"], 200);

        assert!(parse_window("200,100").is_err());
        assert!(parse_window("100").is_err());
        assert!(parse_window("100,200,300").is_err());
        assert!(parse_window("abc,200").is_err());
    }

    #[test]
    fn parse_minor_units_handles_various_exponents() {
        assert_eq!(parse_minor_units("12.34", "USD", 2).unwrap(), 1234);
        assert_eq!(parse_minor_units("12.3", "USD", 2).unwrap(), 1230);
        assert_eq!(parse_minor_units("12", "USD", 2).unwrap(), 1200);
        assert_eq!(parse_minor_units("0.05", "USD", 2).unwrap(), 5);
        assert_eq!(parse_minor_units("0", "USD", 2).unwrap(), 0);
        assert!(parse_minor_units("12.345", "USD", 2).is_err());

        assert_eq!(parse_minor_units("1200", "JPY", 0).unwrap(), 1200);
        assert!(parse_minor_units("12.5", "JPY", 0).is_err());

        assert_eq!(parse_minor_units("12.5", "KWD", 3).unwrap(), 12500);
        assert_eq!(parse_minor_units("12.500", "KWD", 3).unwrap(), 12500);
        assert_eq!(parse_minor_units("12.123", "KWD", 3).unwrap(), 12123);
        assert!(parse_minor_units("12.1234", "KWD", 3).is_err());
    }
}
