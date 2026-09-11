//! `svc remove`, `svc list`, and `svc restart` -- installed-service
//! lifecycle management, without touching a service's deploy-time identity
//! or endpoint record.

use chrono::DateTime;
use syneroym_sdk::{SyneroymClient, Visibility};

pub(super) async fn handle_remove(client: &mut SyneroymClient, svc_id: &str) -> anyhow::Result<()> {
    // Unmanaged: an operator-driven `svc remove` always presents
    // generation 0, the same convention `svc deploy` uses.
    client.undeploy(svc_id.to_string(), 0).await?;
    println!("Successfully removed svc {svc_id}");
    Ok(())
}

pub(super) async fn handle_list(client: &mut SyneroymClient) -> anyhow::Result<()> {
    // Lists all installed SynSvcs registered in the local substrate registry.
    let services = client.list_svcs().await?;
    println!(
        "{:<50} {:<10} {:<12} {:<30} {:<50}",
        "SERVICE ID", "TYPE", "VISIBILITY", "INSTANCE CERT EXPIRES", "INTERFACES"
    );
    println!("{:-<158}", "");
    for svc in services {
        let vis_str = svc.visibility.map_or("-", Visibility::as_str);
        println!(
            "{:<50} {:<10} {:<12} {:<30} {:<50}",
            svc.service_id,
            svc.endpoint_type,
            vis_str,
            format_expiry(svc.instance_certificate_expires_at),
            svc.interfaces.join(", ")
        );
    }
    Ok(())
}

pub(super) async fn handle_restart(
    client: &mut SyneroymClient,
    svc_id: &str,
) -> anyhow::Result<()> {
    // Unmanaged: an operator-driven `svc restart` always presents
    // generation 0, the same convention `svc deploy`/`svc remove` use.
    client.restart(svc_id.to_string(), 0).await?;
    println!("Successfully restarted svc {svc_id}");
    Ok(())
}

/// `-` for a service with no installed instance certificate; otherwise an
/// RFC 3339 timestamp, so "when does this fall over" is answerable without
/// reading logs (ADR-0020 §3).
pub(crate) fn format_expiry(expires_at_secs: Option<u64>) -> String {
    match expires_at_secs {
        Some(secs) => DateTime::from_timestamp(secs as i64, 0)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_else(|| "-".to_string()),
        None => "-".to_string(),
    }
}
