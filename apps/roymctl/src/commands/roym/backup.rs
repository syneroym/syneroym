//! Backup and restore subcommands for Roym.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use syneroym_identity::{
    Identity,
    backup::{self, IdentityBackup, SealedBlob},
    substrate,
};
use syneroym_roym_core::clock;

use crate::{
    DEFAULT_GATEWAY_URL,
    commands::{session, write_secret_file},
};

pub const ARCHIVE_VERSION: u32 = 1;
pub const ARCHIVE_INFO: &[u8] = b"syneroym-roym-archive-v1";
pub const RESTORE_DATA_SUCCESS_NOTICE: &str =
    "Your history and records are restored and can be read. Conversations from before the restore \
     cannot continue: this installation has new addresses. Share your new address with the people \
     you talk to, and start new conversations with them. Repeated imports are safe to run if a \
     previous restore was interrupted.";

const EXPORT_SERVICES: &[&str] =
    &["profile", "catalog", "conversation", "transaction", "directory"];
const IMPORT_ORDER: &[&str] = &["profile", "catalog", "conversation", "transaction", "directory"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoymArchive {
    pub archive_version: u32,
    pub subject_did: String,
    pub produced_at_secs: u64,
    pub identity: IdentityBackup,
    pub data: SealedBlob,
}

#[derive(Subcommand, Debug, Clone)]
pub enum BackupCommands {
    /// Create an encrypted backup archive of Roym identity and service data.
    Create {
        #[arg(long, value_name = "PATH")]
        master: PathBuf,
        #[arg(long, value_name = "PATH")]
        out: PathBuf,
        #[arg(long, value_name = "PATH")]
        recovery_key_out: Option<PathBuf>,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Restore master identity from a backup archive.
    RestoreIdentity {
        #[arg(long = "in", value_name = "PATH")]
        r#in: PathBuf,
        #[arg(long)]
        recovery_key: Option<String>,
        #[arg(long, value_name = "PATH")]
        recovery_key_file: Option<PathBuf>,
        #[arg(long, value_name = "PATH")]
        out: PathBuf,
    },
    /// Restore app service data from a backup archive.
    RestoreData {
        #[arg(long = "in", value_name = "PATH")]
        r#in: PathBuf,
        #[arg(long)]
        recovery_key: Option<String>,
        #[arg(long, value_name = "PATH")]
        recovery_key_file: Option<PathBuf>,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
}

pub fn data_aad_bytes(archive_version: u32, subject_did: &str, produced_at_secs: u64) -> Vec<u8> {
    let val = json!({
        "archive_version": archive_version,
        "produced_at_secs": produced_at_secs,
        "subject_did": subject_did,
    });
    let canonical = substrate::canonicalize_json_value(&val);
    serde_json::to_vec(&canonical).unwrap_or_default()
}

fn resolve_recovery_key(
    recovery_key: Option<&str>,
    recovery_key_file: Option<&Path>,
) -> Result<String> {
    if let Some(rk) = recovery_key {
        return Ok(rk.trim().to_string());
    }
    if let Some(path) = recovery_key_file {
        let content = fs::read_to_string(path)
            .with_context(|| format!("reading recovery key file from {}", path.display()))?;
        return Ok(content.trim().to_string());
    }
    anyhow::bail!("either --recovery-key or --recovery-key-file must be provided")
}

pub(super) async fn handle_backup(
    cmd: &BackupCommands,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> Result<()> {
    match cmd {
        BackupCommands::Create { master, out, recovery_key_out, gateway_url, host } => {
            handle_create(
                master,
                out,
                recovery_key_out.as_deref(),
                gateway_url,
                host.as_deref(),
                dir,
                run_as,
                ucan_path,
            )
            .await
        }
        BackupCommands::RestoreIdentity { r#in: in_path, recovery_key, recovery_key_file, out } => {
            let key = resolve_recovery_key(recovery_key.as_deref(), recovery_key_file.as_deref())?;
            handle_restore_identity(in_path, &key, out)
        }
        BackupCommands::RestoreData {
            r#in: in_path,
            recovery_key,
            recovery_key_file,
            gateway_url,
            host,
        } => {
            let key = resolve_recovery_key(recovery_key.as_deref(), recovery_key_file.as_deref())?;
            handle_restore_data(in_path, &key, gateway_url, host.as_deref(), dir, run_as, ucan_path)
                .await
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "CLI command handler takes individual option arguments directly"
)]
async fn handle_create(
    master: &Path,
    out: &Path,
    recovery_key_out: Option<&Path>,
    gateway_url: &str,
    host: Option<&str>,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> Result<()> {
    let identity = Identity::load_from_path(master)
        .with_context(|| format!("loading master key from {}", master.display()))?;
    let subject_did = substrate::derive_did_key(&identity.public_key());
    let recovery_key = backup::generate_recovery_key()?;
    let identity_backup = backup::export(&identity, &recovery_key)?;

    let bundles = fetch_service_bundles(gateway_url, host, dir, run_as, ucan_path).await?;
    let payload = serde_json::to_vec(&json!({ "bundles": bundles }))?;
    let now_secs = clock::now_secs();
    let aad = data_aad_bytes(ARCHIVE_VERSION, &subject_did, now_secs);
    let sealed_data = backup::seal(&payload, &recovery_key, ARCHIVE_INFO, &aad)?;

    let archive = RoymArchive {
        archive_version: ARCHIVE_VERSION,
        subject_did,
        produced_at_secs: now_secs,
        identity: identity_backup,
        data: sealed_data,
    };

    let encoded = backup::encode_recovery_key(&recovery_key);
    if let Some(rk_out) = recovery_key_out {
        write_secret_file(rk_out, encoded.as_bytes(), "recovery key file")?;
    }

    let json_str = serde_json::to_string_pretty(&archive)?;
    write_secret_file(out, json_str.as_bytes(), "roym archive")?;

    println!("Archive exported to {}", out.display());
    println!("Recovery key (save this now; it is shown once and cannot be recovered):");
    println!("{encoded}");
    Ok(())
}

async fn fetch_service_bundles(
    gateway_url: &str,
    host: Option<&str>,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> Result<BTreeMap<String, Value>> {
    let mut bundles = BTreeMap::new();
    for svc in EXPORT_SERVICES {
        let resp = session::rpc_call(
            gateway_url,
            host,
            run_as,
            ucan_path,
            dir,
            &format!("{svc}.export"),
            json!({}),
        )
        .await
        .with_context(|| format!("failed to export {svc}"))?;
        let bundle = resp.get("bundle").cloned().unwrap_or(resp);
        bundles.insert((*svc).to_string(), bundle);
    }
    Ok(bundles)
}

fn handle_restore_identity(in_path: &Path, recovery_key_str: &str, out: &Path) -> Result<()> {
    let contents = fs::read_to_string(in_path)
        .with_context(|| format!("reading archive from {}", in_path.display()))?;
    let archive: RoymArchive = serde_json::from_str(&contents)
        .with_context(|| format!("parsing archive {}", in_path.display()))?;
    if archive.archive_version != ARCHIVE_VERSION {
        anyhow::bail!("unknown archive version {}", archive.archive_version);
    }

    let recovery_key = backup::decode_recovery_key(recovery_key_str)
        .map_err(|e| anyhow::anyhow!("invalid recovery key: {e:?}"))?;
    let identity = backup::import(&archive.identity, &recovery_key)
        .map_err(|e| anyhow::anyhow!("could not decrypt identity: {e:?}"))?;

    write_secret_file(out, &identity.to_bytes(), "master key")?;
    println!("Identity restored to {}", out.display());
    Ok(())
}

async fn handle_restore_data(
    in_path: &Path,
    recovery_key_str: &str,
    gateway_url: &str,
    host: Option<&str>,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> Result<()> {
    verify_signing_enrolled(gateway_url, host, dir, run_as, ucan_path).await?;

    let contents = fs::read_to_string(in_path)
        .with_context(|| format!("reading archive from {}", in_path.display()))?;
    let archive: RoymArchive = serde_json::from_str(&contents)
        .with_context(|| format!("parsing archive {}", in_path.display()))?;
    if archive.archive_version != ARCHIVE_VERSION {
        anyhow::bail!("unknown archive version {}", archive.archive_version);
    }

    let recovery_key = backup::decode_recovery_key(recovery_key_str)
        .map_err(|e| anyhow::anyhow!("invalid recovery key: {e:?}"))?;
    let aad =
        data_aad_bytes(archive.archive_version, &archive.subject_did, archive.produced_at_secs);
    let decrypted = backup::open(&archive.data, &recovery_key, ARCHIVE_INFO, &aad)
        .map_err(|e| anyhow::anyhow!("could not decrypt data: {e:?}"))?;

    let data_val: Value =
        serde_json::from_slice(&decrypted).context("parsing decrypted data payload")?;
    let bundles = data_val
        .get("bundles")
        .and_then(Value::as_object)
        .context("archive missing 'bundles' mapping")?;

    for svc in IMPORT_ORDER {
        if let Some(bundle) = bundles.get(*svc) {
            let res = session::rpc_call(
                gateway_url,
                host,
                run_as,
                ucan_path,
                dir,
                &format!("{svc}.import"),
                json!({ "bundle": bundle }),
            )
            .await
            .with_context(|| format!("failed to import {svc}"))?;
            println!("{svc} import: {}", serde_json::to_string(&res).unwrap_or_default());
        }
    }

    println!("{RESTORE_DATA_SUCCESS_NOTICE}");
    Ok(())
}

async fn verify_signing_enrolled(
    gateway_url: &str,
    host: Option<&str>,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> Result<()> {
    let status_val = session::rpc_call(
        gateway_url,
        host,
        run_as,
        ucan_path,
        dir,
        "transaction.signing-status",
        json!({}),
    )
    .await
    .context("failed to check transaction.signing-status")?;

    let is_enrolled =
        status_val.get("certificate").and_then(|c| c.get("state")).and_then(Value::as_str)
            == Some("installed");
    if !is_enrolled {
        anyhow::bail!(
            "installation is not enrolled for signing; run `roymctl roym enrol-signing` first"
        );
    }
    Ok(())
}
