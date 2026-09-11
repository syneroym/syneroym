//! Inspecting and re-arming a service's own saga log.

use syneroym_sdk::SyneroymClient;

pub(super) async fn handle_sagas(client: &mut SyneroymClient, svc_id: &str) -> anyhow::Result<()> {
    let items = client.sagas(svc_id.to_string()).await?;
    if items.is_empty() {
        println!("No sagas for {svc_id}");
    } else {
        println!(
            "{:<38} {:<20} {:<13} {:<10} {:<40}",
            "SAGA ID", "NAME", "STATE", "STEPS", "LAST ERROR"
        );
        println!("{:-<125}", "");
        for item in items {
            println!(
                "{:<38} {:<20} {:<13?} {:<10} {:<40}",
                item.saga_id,
                item.name,
                item.state,
                format!("{}/{}", item.compensated_steps, item.steps),
                item.last_error.as_deref().unwrap_or("-")
            );
        }
    }
    Ok(())
}

pub(super) async fn handle_saga_compensate(
    client: &mut SyneroymClient,
    svc_id: &str,
    saga_id: &str,
) -> anyhow::Result<()> {
    client.saga_compensate(svc_id.to_string(), saga_id.to_string()).await?;
    println!("Re-armed saga {saga_id} for {svc_id}");
    Ok(())
}
