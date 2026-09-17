use super::*;

/// Called by both builds when a subscribed message arrives -- from the
/// exported `guest-api::handle-message` on WASM, from the shim's broker pump
/// natively. Persists through `data-layer`, never in process memory.
///
/// `topic` arrives fully namespaced (`svc/<service_id>/<topic>`) on both
/// builds, and is stored verbatim.
pub async fn on_message<H: AppHost>(
    host: &H,
    topic: String,
    payload: Vec<u8>,
) -> Result<(), String> {
    ensure_collection(host, INBOX).await?;
    let id = format!("{topic}:{}", inbox_entry_id(&payload));
    host.put(
        INBOX.into(),
        RecordWriteValue {
            id,
            payload: serde_json::to_vec(&json!({
                "topic": topic,
                "payload": String::from_utf8_lossy(&payload),
            }))
            .map_err(|e| e.to_string())?,
        },
    )
    .await
    .map_err(fmt_err)
}

/// Called by both builds when a durable conversation message arrives --
/// from the exported `guest-api::on-message` on WASM, from
/// `ConversationSink::on_message` natively. Persists through `data-layer`,
/// never in-process state, keyed by the message's own id so a
/// redelivery overwrites rather than duplicates.
pub async fn on_conversation_message<H: AppHost>(host: &H, msg: Message) -> Result<(), String> {
    ensure_collection(host, CONV_INBOX).await?;
    host.put(
        CONV_INBOX.into(),
        RecordWriteValue {
            id: msg.id.clone(),
            payload: serde_json::to_vec(&message_json(&msg)).map_err(|e| e.to_string())?,
        },
    )
    .await
    .map_err(fmt_err)
}

/// Called by both builds on a delivery-state transition. Appends rather
/// than overwrites (keyed by message id + a monotonic-enough sequence
/// derived from the state name) so a test can observe the transition
/// sequence, not just the latest state.
pub async fn on_conversation_state<H: AppHost>(
    host: &H,
    message: String,
    state: DeliveryState,
) -> Result<(), String> {
    ensure_collection(host, CONV_STATE_LOG).await?;
    let state_str = delivery_state_str(state);
    host.put(
        CONV_STATE_LOG.into(),
        RecordWriteValue {
            id: format!("{message}:{state_str}"),
            payload: serde_json::to_vec(&json!({ "message": message, "state": state_str }))
                .map_err(|e| e.to_string())?,
        },
    )
    .await
    .map_err(fmt_err)
}
