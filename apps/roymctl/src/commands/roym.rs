//! Commands specific to the Roym product app.

use std::{path::Path, time::Duration};

use anyhow::{Context, Result};
use clap::Subcommand;
use serde_json::json;
use syneroym_identity::{DelegationCertificate, Identity, substrate};
use syneroym_sdk::DeployedService;
use syneroym_signed_record::SCOPE_RECORD_SIGNING;

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
        rules_file: std::path::PathBuf,
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

/// Every Roym service that signs a record and so needs a record-signing
/// certificate of its own. `directory` and `transaction` sign nothing yet,
/// so a certificate there would be a verb no flow exercises.
const SIGNING_SERVICES: &[&str] = &["profile", "catalog", "conversation"];

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
            let rules = std::fs::read_to_string(rules_file)
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

    for chunk in sources.chunks(max_concurrency) {
        let mut set = tokio::task::JoinSet::new();
        for source in chunk {
            let source = source.clone();
            let gateway_url = gateway_url.to_string();
            let host = host.map(str::to_string);
            let dir = dir.to_path_buf();
            let run_as = run_as.map(str::to_string);
            let ucan_path = ucan_path.map(|p| p.to_path_buf());
            let query = query.clone();
            let run_id = run_id.clone();
            set.spawn(async move {
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
                (source, result)
            });
        }
        while let Some(joined) = set.join_next().await {
            if let Ok((source, Err(e))) = joined {
                println!("source {source}: could not run the query ({e})");
            }
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
        let credential = hit.get("credential").and_then(|v| v.as_str()).unwrap_or("unknown");
        let sources_val = hit.get("sources").cloned().unwrap_or_default();
        println!(
            "- {listing_id} \"{title}\" by {issuer}, age {age}s, revocation: {revocation}, \
             membership: {credential}, sources: {sources_val}"
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
}
