//! Session management subcommands
//!
//! Commands to open, inspect, and close local person sessions at the client
//! gateway.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use syneroym_core::protocol_utils::gateway_session_assertion;
use syneroym_identity::{DelegationCertificate, Identity, delegation::SCOPE_ROUTING, substrate};

use super::member_identity;
use crate::DEFAULT_GATEWAY_URL;

#[derive(Subcommand, Debug, Clone)]
pub enum SessionCommands {
    /// Open a local person session at a client gateway, so the gateway
    /// proxies as this person's DID instead of the node's own.
    Login {
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        /// Community registry to publish this person's master anchor to.
        /// Without a published anchor every proxied request is rejected at
        /// the destination -- the same requirement `identity
        /// certify-instance` has.
        #[arg(long)]
        registry_url: Option<String>,
        /// Lifetime of the owner->node delegation certificate this mints.
        #[arg(long, default_value_t = 24)]
        expires_hours: u64,
    },
    /// Who the gateway currently thinks this client is.
    Status {
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
    },
    /// Print the stored token, for `curl -H "Authorization: Bearer $(...)"`.
    Token {
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
    },
    /// End the session at the gateway and delete the local file.
    Logout {
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChallengeResponse {
    nonce: String,
    node_did: String,
    expires_at_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LoginRequest {
    person_did: String,
    nonce: String,
    signature: String,
    delegation: DelegationCertificate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LoginResponse {
    token: String,
    person_did: String,
    expires_at_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WhoamiResponse {
    person_did: String,
    auth: String,
    expires_at_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredSession {
    gateway_url: String,
    node_did: String,
    person_did: String,
    token: String,
    expires_at_secs: u64,
}

fn sanitize_gateway_url(url: &str) -> String {
    url.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect()
}

fn session_file_path(dir: &Path, gateway_url: &str) -> PathBuf {
    dir.join("sessions").join(format!("{}.json", sanitize_gateway_url(gateway_url)))
}

fn save_session_file(path: &Path, session: &StoredSession) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_string_pretty(session)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .with_context(|| format!("failed to create session file at {}", path.display()))?;
        file.write_all(data.as_bytes())?;
    }
    #[cfg(not(unix))]
    {
        fs::write(path, data)?;
    }
    Ok(())
}

fn load_session_file(path: &Path) -> Result<StoredSession> {
    let data = fs::read_to_string(path)
        .with_context(|| format!("failed to read session file at {}", path.display()))?;
    serde_json::from_str(&data)
        .with_context(|| format!("invalid session JSON at {}", path.display()))
}

pub async fn handle(command: &SessionCommands, dir: &Path, run_as: Option<&str>) -> Result<()> {
    match command {
        SessionCommands::Login { gateway_url, registry_url, expires_hours } => {
            let identity_name =
                run_as.context("session login requires --as <name> for the person identity")?;
            let key_path = dir.join("identities").join(format!("{identity_name}.key"));
            if !key_path.exists() {
                bail!("Identity '{}' not found at {}", identity_name, key_path.display());
            }
            let person_identity = Identity::load_from_path(&key_path)?;
            let person_did = substrate::derive_did_key(&person_identity.public_key());

            let client = Client::new();
            let base_url = gateway_url.trim_end_matches('/');

            // 1. Fetch challenge
            let challenge_url = format!("{base_url}/_syneroym/session/challenge");
            let resp = client
                .post(&challenge_url)
                .send()
                .await
                .with_context(|| format!("failed to connect to gateway at {challenge_url}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let err_text = resp.text().await.unwrap_or_default();
                bail!("challenge request failed ({status}): {err_text}");
            }
            let ch: ChallengeResponse = resp.json().await?;

            // 2. Sign delegation and challenge
            let node_pubkey = substrate::resolve_did_key(&ch.node_did)
                .with_context(|| format!("failed to resolve node DID '{}'", ch.node_did))?;
            let cert = DelegationCertificate::issue(
                &person_identity,
                node_pubkey,
                expires_hours.saturating_mul(3600),
                SCOPE_ROUTING.to_string(),
            )?;
            let assertion = gateway_session_assertion(&ch.node_did, &ch.nonce, &person_did);
            let signature = person_identity.sign_json(&assertion)?;

            // 3. Publish/refresh anchor before login
            member_identity::refresh_anchor_or_warn(registry_url.as_deref(), &person_identity)
                .await?;

            // 4. POST login
            let login_url = format!("{base_url}/_syneroym/session/login");
            let login_req = LoginRequest {
                person_did: person_did.clone(),
                nonce: ch.nonce,
                signature,
                delegation: cert,
            };
            let resp = client
                .post(&login_url)
                .json(&login_req)
                .send()
                .await
                .with_context(|| format!("failed to connect to gateway at {login_url}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let err_text = resp.text().await.unwrap_or_default();
                if let Ok(val) = serde_json::from_str::<Value>(&err_text)
                    && let Some(err_msg) = val.get("error").and_then(|v| v.as_str())
                {
                    bail!("gateway login failed ({status}): {err_msg}");
                }
                bail!("gateway login failed ({status}): {err_text}");
            }
            let grant: LoginResponse = resp.json().await?;

            // 5. Persist session
            let session_path = session_file_path(dir, gateway_url);
            let stored = StoredSession {
                gateway_url: gateway_url.clone(),
                node_did: ch.node_did.clone(),
                person_did: grant.person_did.clone(),
                token: grant.token,
                expires_at_secs: grant.expires_at_secs,
            };
            save_session_file(&session_path, &stored)?;

            println!(
                "Logged in as {person_did} to node {} (session expires at {})",
                ch.node_did, grant.expires_at_secs
            );
        }
        SessionCommands::Status { gateway_url } => {
            let session_path = session_file_path(dir, gateway_url);
            if !session_path.exists() {
                bail!("no active session for {gateway_url}");
            }
            let session = load_session_file(&session_path)?;

            let client = Client::new();
            let base_url = gateway_url.trim_end_matches('/');
            let whoami_url = format!("{base_url}/_syneroym/session/whoami");
            let resp = client
                .get(&whoami_url)
                .header("Authorization", format!("Bearer {}", session.token))
                .send()
                .await
                .with_context(|| format!("failed to connect to gateway at {whoami_url}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let err_text = resp.text().await.unwrap_or_default();
                bail!("session status check failed ({status}): {err_text}");
            }
            let whoami: WhoamiResponse = resp.json().await?;
            println!("Person DID: {}", whoami.person_did);
            println!("Auth: {}", whoami.auth);
            println!("Expires at: {}", whoami.expires_at_secs);
        }
        SessionCommands::Token { gateway_url } => {
            let session_path = session_file_path(dir, gateway_url);
            if !session_path.exists() {
                bail!("no active session for {gateway_url}");
            }
            let session = load_session_file(&session_path)?;
            println!("{}", session.token);
        }
        SessionCommands::Logout { gateway_url } => {
            let session_path = session_file_path(dir, gateway_url);
            if session_path.exists() {
                if let Ok(session) = load_session_file(&session_path) {
                    let client = Client::new();
                    let base_url = gateway_url.trim_end_matches('/');
                    let logout_url = format!("{base_url}/_syneroym/session/logout");
                    let _ = client
                        .post(&logout_url)
                        .header("Authorization", format!("Bearer {}", session.token))
                        .send()
                        .await;
                }
                let _ = fs::remove_file(&session_path);
            }
            println!("Logged out of {gateway_url}");
        }
    }
    Ok(())
}
