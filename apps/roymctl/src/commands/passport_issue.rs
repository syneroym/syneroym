//! Signing a passport: the flags and steps that every `passport issue`
//! command shares.

use std::path::{Path, PathBuf};

use anyhow::{Context, ensure};
use clap::{ArgGroup, Args};
use serde_json::{Map, Value};
use syneroym_core::config::DEFAULT_SUBSTRATE_KEY_FILE;
use syneroym_identity::{Identity, substrate};
use syneroym_passport::{PassportId, PassportUse};

use super::identity;

pub(crate) const SECS_PER_DAY: u64 = 24 * 60 * 60;

/// The signer, the audience and the life of a passport, and where to write
/// it. The use limits are the flags of each command.
#[derive(Args, Debug, Clone)]
#[command(group(ArgGroup::new("signer").required(true).args(["from", "node_key"])))]
pub struct IssueSigner {
    /// Local identity that signs: the controller of the host substrate.
    #[arg(long)]
    pub from: Option<String>,
    /// Sign with the node key `<dir>/substrate.key`. Only for a host
    /// substrate that has no controller.
    #[arg(long)]
    pub node_key: bool,
    /// The owner DID the passport is for.
    #[arg(long)]
    pub to: String,
    /// Days until the passport expires.
    #[arg(long)]
    pub expires_days: u64,
    /// Where to write the passport. Omitted: standard output.
    #[arg(long)]
    pub out: Option<PathBuf>,
}

/// Signs a passport for `use_` with `limits`, writes it, and prints its id.
/// The operator needs the id to deny the passport later. The host node DID
/// comes from the global `--substrate` (or `<dir>/substrate.key`).
pub(crate) fn issue(
    signer: &IssueSigner,
    use_: &PassportUse,
    limits: Map<String, Value>,
    substrate_opt: Option<String>,
    dir: &Path,
) -> anyhow::Result<PassportId> {
    let host_node_did = super::get_substrate_did(substrate_opt, dir)?;
    let key = load_signer(dir, signer.from.as_deref(), signer.node_key, &host_node_did)?;
    ensure!(signer.expires_days > 0, "--expires-days must be at least 1");
    let expires_in_secs =
        signer.expires_days.checked_mul(SECS_PER_DAY).context("--expires-days is too large")?;
    let passport = syneroym_passport::issue_passport(
        &key,
        &host_node_did,
        use_,
        &signer.to,
        limits,
        expires_in_secs,
        syneroym_passport::now_secs(),
    )?;
    let json = serde_json::to_string_pretty(&passport)?;
    match &signer.out {
        Some(path) => super::write_secret_file(path, json.as_bytes(), "passport")?,
        None => println!("{json}"),
    }
    let id = PassportId::of(&passport)?;
    eprintln!("passport id: {id}");
    Ok(id)
}

/// The signing key: a named local identity, or the node key of the host
/// substrate. The command never picks one by itself.
fn load_signer(
    dir: &Path,
    from: Option<&str>,
    node_key: bool,
    host_node_did: &str,
) -> anyhow::Result<Identity> {
    match (from, node_key) {
        (Some(name), false) => {
            Identity::load_from_path(identity::require_key_present(dir, name, "Identity")?)
        }
        (None, true) => {
            let key = Identity::load_from_path(dir.join(DEFAULT_SUBSTRATE_KEY_FILE))
                .context("--node-key given, but there is no substrate.key in --dir")?;
            let key_did = substrate::derive_did_key(&key.public_key());
            ensure!(
                key_did == host_node_did,
                "substrate.key is the key of {key_did}, not of --substrate {host_node_did}"
            );
            Ok(key)
        }
        _ => anyhow::bail!("pass exactly one of --from <name> or --node-key"),
    }
}
