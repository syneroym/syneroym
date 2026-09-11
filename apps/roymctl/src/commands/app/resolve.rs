//! Resolve subcommand for SynApps: registry/topology address resolution.

use std::{fs, path::Path};

use anyhow::Context;
use syneroym_app_orchestration::{
    TopologyFetcher,
    models::{AppDid, LogicalServiceName},
};
use syneroym_identity::Identity;
use syneroym_sdk::RegistryTopologyFetcher;
use syneroym_ucan::CapabilityToken;

pub(super) async fn handle_resolve(
    app_did: String,
    service_name: String,
    api_url: &str,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> anyhow::Result<()> {
    let app_did = AppDid::try_new(app_did.clone())?;
    let service_name = LogicalServiceName::try_new(service_name.clone())?;

    let mut fetcher = RegistryTopologyFetcher::new(api_url.to_string());
    if let Some(name) = run_as {
        let path = dir.join("identities").join(format!("{name}.key"));
        let id = Identity::load_from_path(&path)
            .with_context(|| format!("no local identity '{name}' at {}", path.display()))?;
        fetcher = fetcher.with_identity(&id);
    }
    if let Some(path) = ucan_path {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read UCAN token at {}", path.display()))?;
        let token: CapabilityToken = serde_json::from_str(&raw)
            .with_context(|| format!("invalid UCAN token JSON at {}", path.display()))?;
        fetcher = fetcher.with_ucan(token);
    }

    let signed =
        fetcher.fetch(&app_did, &service_name).await.map_err(|e| anyhow::anyhow!("{e}"))?;
    signed
        .verify(&app_did)
        .context("the fetched document did not verify against the resolved app DID")?;

    println!("app: {app_did}  service: {service_name}");
    println!("mode: {:?}  epoch: {}", signed.document.mode, signed.document.epoch.0);
    println!("members:");
    for member in &signed.document.members {
        println!("  {member}");
    }

    Ok(())
}
