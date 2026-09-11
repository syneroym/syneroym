//! Prints this installation's Roym Conversation service id and Hub gateway
//! host, for pasting into `profile.set`.

use std::{path::Path, time::Duration};

use anyhow::Result;
use syneroym_core::util::generate_service_host;
use syneroym_sdk::DeployedService;

pub(super) async fn handle_address(
    domain: &str,
    api_url: &str,
    substrate_opt: Option<String>,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> Result<()> {
    let substrate_did = crate::commands::get_substrate_did(substrate_opt, dir)?;
    let mut client = crate::commands::client_for(substrate_did, api_url, dir, run_as, ucan_path)?;
    client.wait_for_ready(Duration::from_secs(5)).await?;

    let svcs = client.list_svcs().await?;
    let conversation_id = find_roym_service(&svcs, "conversation")?;
    let web_id = find_roym_service(&svcs, "web")?;
    let hub_host = generate_service_host(None, &web_id, None, domain)?;

    println!("conversation service id: {conversation_id}");
    println!("  paste this into profile.set as `conversation_address`");
    println!("Hub gateway host:        {hub_host}");
    Ok(())
}

/// The physical service id of a Roym logical service, found by the app
/// interface it registers (`syneroym-roym:<name>/...`). Reads only what
/// `svc list` already returns, so it invents no resolution path: no host
/// surface reports a service its own routing address, so a person would
/// otherwise have to read it out of a deploy log.
pub(crate) fn find_roym_service(svcs: &[DeployedService], name: &str) -> Result<String> {
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
