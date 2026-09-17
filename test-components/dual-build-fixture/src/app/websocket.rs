use super::*;

pub async fn on_ws_open<H: AppHost>(host: &H, conn: String) {
    if let Err(e) = ensure_collection(host, WS_LOG).await {
        eprintln!("failed to ensure WS_LOG: {e}");
        return;
    }
    let id = format!("{conn}:open");
    if let Err(e) = host
        .put(
            WS_LOG.into(),
            RecordWriteValue {
                id,
                payload: serde_json::to_vec(&json!({
                    "event": "open",
                    "conn": conn,
                }))
                .unwrap_or_default(),
            },
        )
        .await
    {
        eprintln!("failed to write ws open log: {e:?}");
    }
}

pub async fn on_ws_message<H: AppHost>(host: &H, conn: String, frame: Vec<u8>, kind: FrameKind) {
    if let Err(e) = ensure_collection(host, WS_LOG).await {
        eprintln!("failed to ensure WS_LOG: {e}");
        return;
    }
    // Count existing message rows for this connection in WS_LOG to assign a
    // deterministic monotonic sequence number that matches between WASM and
    // native builds. The 1000-row limit is deliberate for fixture scale.
    let msg_count = match host
        .query(WS_LOG.into(), QueryOptions { filter: None, limit: Some(1000), cursor: None })
        .await
    {
        Ok(page) => {
            page.records.iter().filter(|r| r.id.starts_with(&format!("{conn}:message:"))).count()
        }
        Err(e) => {
            eprintln!("failed to query WS_LOG for message sequence: {e:?}");
            0
        }
    };
    let seq = (msg_count + 1) as u64;
    let id = format!("{conn}:message:{seq}:{}", inbox_entry_id(&frame));
    if let Err(e) = host
        .put(
            WS_LOG.into(),
            RecordWriteValue {
                id,
                payload: serde_json::to_vec(&json!({
                    "event": "message",
                    "conn": conn,
                    "seq": seq,
                    "frame": String::from_utf8_lossy(&frame),
                    "kind": match kind {
                        FrameKind::Text => "text",
                        FrameKind::Binary => "binary",
                    },
                }))
                .unwrap_or_default(),
            },
        )
        .await
    {
        eprintln!("failed to write ws message log: {e:?}");
    }
}

pub async fn on_ws_close<H: AppHost>(host: &H, conn: String) {
    if let Err(e) = ensure_collection(host, WS_LOG).await {
        eprintln!("failed to ensure WS_LOG: {e}");
        return;
    }
    let id = format!("{conn}:close");
    if let Err(e) = host
        .put(
            WS_LOG.into(),
            RecordWriteValue {
                id,
                payload: serde_json::to_vec(&json!({
                    "event": "close",
                    "conn": conn,
                }))
                .unwrap_or_default(),
            },
        )
        .await
    {
        eprintln!("failed to write ws close log: {e:?}");
    }
}
