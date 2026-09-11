//! Inspecting and replaying a service's durable proxy outbox.

use chrono::DateTime;
use syneroym_sdk::SyneroymClient;

pub(super) async fn handle_outbox(client: &mut SyneroymClient, svc_id: &str) -> anyhow::Result<()> {
    let items = client.proxy_outbox(svc_id.to_string()).await?;
    if items.is_empty() {
        println!("No queued proxy calls for {svc_id}");
    } else {
        println!("{:<8} {:<10} {:<50}", "ID", "ATTEMPTS", "IDEMPOTENCY KEY");
        println!("{:-<70}", "");
        for item in items {
            println!("{:<8} {:<10} {:<50}", item.id, item.attempts, item.idempotency_key);
        }
    }
    Ok(())
}

pub(super) async fn handle_dead_letters(
    client: &mut SyneroymClient,
    svc_id: &str,
) -> anyhow::Result<()> {
    let items = client.proxy_dead_letters(svc_id.to_string()).await?;
    if items.is_empty() {
        println!("No proxy dead letters for {svc_id}");
    } else {
        println!(
            "{:<8} {:<10} {:<30} {:<40} {:<40}",
            "ID", "ATTEMPTS", "CREATED", "IDEMPOTENCY KEY", "LAST ERROR"
        );
        println!("{:-<130}", "");
        for item in items {
            println!(
                "{:<8} {:<10} {:<30} {:<40} {:<40}",
                item.id,
                item.attempts,
                DateTime::from_timestamp_millis(item.created_at)
                    .map_or_else(|| "-".to_string(), |dt| dt.to_rfc3339()),
                item.idempotency_key,
                item.last_error
            );
        }
    }
    Ok(())
}

pub(super) async fn handle_replay(
    client: &mut SyneroymClient,
    svc_id: &str,
    dead_letter_id: u64,
) -> anyhow::Result<()> {
    client.proxy_replay(svc_id.to_string(), dead_letter_id).await?;
    println!("Re-enqueued dead letter {dead_letter_id} for {svc_id}");
    Ok(())
}
