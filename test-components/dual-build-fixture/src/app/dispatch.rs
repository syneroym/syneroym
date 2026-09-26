use super::*;

#[expect(clippy::too_many_lines, reason = "dispatch table matching all test-component RPC routes")]
pub(super) async fn dispatch<H: AppHost>(
    host: &H,
    req: Request,
) -> Result<serde_json::Value, String> {
    match req {
        Request::StoreMessages { count } => {
            ensure_collection(host, MESSAGES).await?;
            for i in 0..count {
                host.put(
                    MESSAGES.into(),
                    RecordWriteValue {
                        id: format!("m{i}"),
                        payload: format!(r#"{{"seq":{i}}}"#).into_bytes(),
                    },
                )
                .await
                .map_err(fmt_err)?;
            }
            let page = host
                .query(
                    MESSAGES.into(),
                    QueryOptions { filter: None, limit: Some(count), cursor: None },
                )
                .await
                .map_err(fmt_err)?;
            Ok(json!({ "written": count, "read": page.records.len() }))
        }
        Request::ReadMessages { limit } => {
            ensure_collection(host, MESSAGES).await?;
            let page = host
                .query(
                    MESSAGES.into(),
                    QueryOptions { filter: None, limit: Some(limit), cursor: None },
                )
                .await
                .map_err(fmt_err)?;
            Ok(json!({ "read": page.records.len() }))
        }
        Request::AdminDdl { sql } => match host.execute_ddl(sql).await {
            Ok(()) => Ok(json!({ "admin-ddl": "allowed" })),
            Err(e) => Err(fmt_err(e)),
        },
        Request::GetMissing { id } => {
            ensure_collection(host, MESSAGES).await?;
            let found = AppDataLayer::get(host, MESSAGES.into(), id).await.map_err(fmt_err)?;
            Ok(json!({ "found": found.is_some() }))
        }
        Request::PutBlob { body } => {
            let hash = host.put_blob(body.clone().into_bytes()).await.map_err(fmt_err)?;
            Ok(json!({ "hash": hash, "bytes": body.len() }))
        }
        Request::GetBlob { hash } => {
            let bytes = host.get_blob(hash).await.map_err(fmt_err)?;
            Ok(json!({ "bytes": bytes.len(), "body": String::from_utf8_lossy(&bytes) }))
        }
        Request::StreamBlob { chunks, read_chunk } => {
            let mut w = host.open_upload().await.map_err(fmt_err)?;
            for c in &chunks {
                w.write(c.clone().into_bytes()).await.map_err(fmt_err)?;
            }
            let hash = w.finish().await.map_err(fmt_err)?; // consumes `w`
            let mut r = host.open_download(hash.clone(), 0).await.map_err(fmt_err)?;
            let mut out = Vec::new();
            loop {
                let part = r.read(read_chunk).await.map_err(fmt_err)?;
                if part.is_empty() {
                    break;
                }
                out.extend_from_slice(&part);
            }
            Ok(json!({ "hash": hash, "bytes": out.len(), "body": String::from_utf8_lossy(&out) }))
        }
        Request::SubscribeTopic { topic } => {
            host.subscribe(topic).await.map_err(fmt_err)?;
            Ok(json!({ "subscribed": true }))
        }
        Request::PublishTopic { topic, payload } => {
            host.publish(topic, payload.into_bytes()).await.map_err(fmt_err)?;
            Ok(json!({ "published": true }))
        }
        Request::ReadInbox => {
            ensure_collection(host, INBOX).await?;
            let page = host
                .query(INBOX.into(), QueryOptions { filter: None, limit: Some(100), cursor: None })
                .await
                .map_err(fmt_err)?;
            let entries: Vec<serde_json::Value> = page
                .records
                .into_iter()
                .filter_map(|r| serde_json::from_slice(&r.payload).ok())
                .collect();
            Ok(json!({ "entries": entries }))
        }
        Request::Unsubscribe { topic } => {
            host.subscribe(topic.clone()).await.map_err(fmt_err)?;
            host.unsubscribe(topic).await.map_err(fmt_err)?;
            Ok(json!({ "unsubscribed": true }))
        }
        Request::Patch { id } => {
            ensure_collection(host, SCRATCH).await?;
            host.put(
                SCRATCH.into(),
                RecordWriteValue { id: id.clone(), payload: b"{\"seq\":0}".to_vec() },
            )
            .await
            .map_err(fmt_err)?;
            host.patch(SCRATCH.into(), id.clone(), b"{\"patched\":true}".to_vec())
                .await
                .map_err(fmt_err)?;
            let after = AppDataLayer::get(host, SCRATCH.into(), id).await.map_err(fmt_err)?;
            Ok(json!({ "after": after.map(|r| String::from_utf8_lossy(&r.payload).into_owned()) }))
        }
        Request::BatchMutate { id_a, id_b } => {
            ensure_collection(host, SCRATCH).await?;
            host.batch_mutate(
                SCRATCH.into(),
                vec![
                    Mutation::Put(RecordWriteValue {
                        id: id_a.clone(),
                        payload: b"{\"n\":1}".to_vec(),
                    }),
                    Mutation::Put(RecordWriteValue {
                        id: id_b.clone(),
                        payload: b"{\"n\":2}".to_vec(),
                    }),
                ],
            )
            .await
            .map_err(fmt_err)?;
            let a_found =
                AppDataLayer::get(host, SCRATCH.into(), id_a).await.map_err(fmt_err)?.is_some();
            let b_found =
                AppDataLayer::get(host, SCRATCH.into(), id_b).await.map_err(fmt_err)?.is_some();
            Ok(json!({ "a_found": a_found, "b_found": b_found }))
        }
        Request::CreateFence { id } => {
            ensure_collection(host, SCRATCH).await?;
            let r1 = host
                .create(
                    SCRATCH.into(),
                    vec![RecordWriteValue { id: id.clone(), payload: b"{\"v\":1}".to_vec() }],
                )
                .await
                .map_err(fmt_err)?;
            let r2 = host
                .create(
                    SCRATCH.into(),
                    vec![RecordWriteValue { id: id.clone(), payload: b"{\"v\":2}".to_vec() }],
                )
                .await
                .map_err(fmt_err)?;
            Ok(json!([r1, r2]))
        }
        Request::DeleteMany { id } => {
            ensure_collection(host, SCRATCH).await?;
            host.put(SCRATCH.into(), RecordWriteValue { id: id.clone(), payload: b"{}".to_vec() })
                .await
                .map_err(fmt_err)?;
            let deleted = host.delete_many(SCRATCH.into(), String::new()).await.map_err(fmt_err)?;
            let still_present =
                AppDataLayer::get(host, SCRATCH.into(), id).await.map_err(fmt_err)?.is_some();
            Ok(json!({ "deleted": deleted, "still_present": still_present }))
        }
        Request::DropCollection => {
            ensure_collection(host, SCRATCH).await?;
            host.drop_collection(SCRATCH.into()).await.map_err(fmt_err)?;
            ensure_collection(host, SCRATCH).await?;
            Ok(json!({ "dropped": true }))
        }
        Request::DeleteBlob { body } => {
            let hash = host.put_blob(body.into_bytes()).await.map_err(fmt_err)?;
            host.delete_blob(hash.clone()).await.map_err(fmt_err)?;
            let still_gettable = host.get_blob(hash).await.is_ok();
            Ok(json!({ "deleted": true, "still_gettable": still_gettable }))
        }
        Request::AbortUpload { chunks } => {
            let mut w = host.open_upload().await.map_err(fmt_err)?;
            for c in &chunks {
                w.write(c.clone().into_bytes()).await.map_err(fmt_err)?;
            }
            w.abort().await; // consumes `w`
            Ok(json!({ "aborted": true }))
        }
        Request::OpenConversation { peer_address } => {
            let id = host.open_direct(peer_address).await.map_err(fmt_err)?;
            Ok(json!({ "conversation": id }))
        }
        Request::SendMessage { conversation, body } => {
            let id = AppConversation::send(
                host,
                conversation,
                "text/plain".to_string(),
                body.into_bytes(),
            )
            .await
            .map_err(fmt_err)?;
            Ok(json!({ "message": id }))
        }
        Request::ReadHistory { conversation, limit } => {
            let page = host.history(conversation, limit, None).await.map_err(fmt_err)?;
            Ok(json!({
                "messages": page.messages.iter().map(message_json).collect::<Vec<_>>(),
                "next-cursor": page.next_cursor,
            }))
        }
        Request::DeliveryStatus { message } => {
            let state = host.delivery_status(message).await.map_err(fmt_err)?;
            Ok(json!({ "state": delivery_state_str(state) }))
        }
        Request::ReadOutbox => {
            let messages = host.outbox().await.map_err(fmt_err)?;
            let list = messages.iter().map(message_json).collect::<Vec<_>>();
            Ok(json!({
                "outbox": list,
            }))
        }
        Request::RetryMessage { message } => {
            host.retry(message).await.map_err(fmt_err)?;
            Ok(json!({ "retried": true }))
        }
        Request::CreateGroup => {
            let id = host.create_group().await.map_err(fmt_err)?;
            Ok(json!({ "conversation": id }))
        }
        Request::AddMember { conversation, member_address } => {
            host.add_member(conversation, member_address).await.map_err(fmt_err)?;
            Ok(json!({ "added": true }))
        }
        Request::RemoveMember { conversation, member_address } => {
            host.remove_member(conversation, member_address).await.map_err(fmt_err)?;
            Ok(json!({ "removed": true }))
        }
        Request::Members { conversation } => {
            let members = host.members(conversation).await.map_err(fmt_err)?;
            Ok(json!({ "members": members }))
        }
        Request::MembershipHistory { conversation } => {
            let history = host.membership_history(conversation).await.map_err(fmt_err)?;
            let events: Vec<_> = history
                .iter()
                .map(|e| {
                    json!({
                        "entry": e.entry,
                        "action": e.action,
                        "subject": e.subject,
                        "epoch": e.epoch,
                        "sender-timestamp": e.sender_timestamp,
                    })
                })
                .collect();
            Ok(json!({
                "history": events,
            }))
        }
        Request::SyncNow { conversation } => {
            host.sync_now(conversation).await.map_err(fmt_err)?;
            Ok(json!({ "synced": true }))
        }
        Request::ListConversations => {
            let list = host.conversations().await.map_err(fmt_err)?;
            let summaries: Vec<_> = list
                .iter()
                .map(|c| {
                    json!({
                        "id": c.id,
                        "participants": c.participants,
                        "kind": match c.kind {
                            ConversationKind::Direct => "direct",
                            ConversationKind::Group => "group",
                        },
                        "created-at": c.created_at,
                        "last-activity-at": c.last_activity_at,
                    })
                })
                .collect();
            Ok(json!({ "conversations": summaries }))
        }
        Request::ReadConversationInbox => {
            ensure_collection(host, CONV_INBOX).await?;
            let page = host
                .query(
                    CONV_INBOX.into(),
                    QueryOptions { filter: None, limit: Some(1000), cursor: None },
                )
                .await
                .map_err(fmt_err)?;
            let entries: Vec<serde_json::Value> = page
                .records
                .into_iter()
                .filter_map(|r| serde_json::from_slice(&r.payload).ok())
                .collect();
            Ok(json!({ "entries": entries }))
        }
        Request::ReadStateLog => {
            ensure_collection(host, CONV_STATE_LOG).await?;
            let page = host
                .query(
                    CONV_STATE_LOG.into(),
                    QueryOptions { filter: None, limit: Some(1000), cursor: None },
                )
                .await
                .map_err(fmt_err)?;
            let entries: Vec<serde_json::Value> = page
                .records
                .into_iter()
                .filter_map(|r| serde_json::from_slice(&r.payload).ok())
                .collect();
            Ok(json!({ "entries": entries }))
        }
        Request::ProxyCallSelf { service_id, interface, method, params } => {
            let target = CallTarget::Service(service_id);
            let res = host.call(target, interface, method, params, None).await.map_err(fmt_err)?;
            Ok(json!({ "result": res }))
        }
        Request::ProxyCallDependency { name, interface, method, params } => {
            let res = host
                .call(CallTarget::Dependency(name), interface, method, params, None)
                .await
                .map_err(fmt_err)?;
            Ok(json!({ "result": res }))
        }
        Request::ProxyCallUnboundDependency { name } => {
            let res = host
                .call(
                    CallTarget::Dependency(name),
                    "any".to_string(),
                    "any".to_string(),
                    "{}".to_string(),
                    None,
                )
                .await;
            match res {
                Ok(v) => Ok(json!({ "result": v })),
                Err(e) => Ok(json!({ "error": fmt_err(e) })),
            }
        }
        Request::ProxyCallCrossServiceNative { target, interface, method, params } => {
            let res = host.call(CallTarget::Service(target), interface, method, params, None).await;
            match res {
                Ok(v) => Ok(json!({ "result": v })),
                Err(e) => Ok(json!({ "error": fmt_err(e) })),
            }
        }
        Request::ProxyEnqueue { name, idempotency_key } => {
            host.enqueue(
                CallTarget::Dependency(name),
                "syneroym-test:dual-build-fixture/test-driver@0.1.0".to_string(),
                "run".to_string(),
                "{}".to_string(),
                Some(CallOptions {
                    protocol: None,
                    idempotent: false,
                    timeout_ms: None,
                    routing_key: None,
                    idempotency_key,
                }),
            )
            .await
            .map_err(fmt_err)?;
            Ok(json!({ "enqueued": true }))
        }
        Request::ProxyEnqueueNoKey { name } => {
            let res = host
                .enqueue(
                    CallTarget::Dependency(name),
                    "syneroym-test:dual-build-fixture/test-driver@0.1.0".to_string(),
                    "run".to_string(),
                    "{}".to_string(),
                    None,
                )
                .await;
            match res {
                Ok(()) => Ok(json!({ "enqueued": true })),
                Err(e) => Ok(json!({ "error": fmt_err(e) })),
            }
        }
        Request::ProxyEnqueueEmptyKey { name } => {
            let res = host
                .enqueue(
                    CallTarget::Dependency(name),
                    "syneroym-test:dual-build-fixture/test-driver@0.1.0".to_string(),
                    "run".to_string(),
                    "{}".to_string(),
                    Some(CallOptions {
                        protocol: None,
                        idempotent: false,
                        timeout_ms: None,
                        routing_key: None,
                        idempotency_key: Some(String::new()),
                    }),
                )
                .await;
            match res {
                Ok(()) => Ok(json!({ "enqueued": true })),
                Err(e) => Ok(json!({ "error": fmt_err(e) })),
            }
        }
        Request::ReadConfig { key } => {
            let value = AppAppConfig::get(host, key).await.map_err(fmt_err)?;
            Ok(json!({ "value": value }))
        }
        Request::ReadConfigSection { prefix } => {
            let entries = host.get_section(prefix).await.map_err(fmt_err)?;
            Ok(json!({ "entries": entries }))
        }
        Request::RevealSecret { key } => {
            let res = host.reveal(key).await;
            match res {
                Ok(bytes) => Ok(json!({ "secret": String::from_utf8_lossy(&bytes) })),
                Err(e) => Ok(json!({ "error": fmt_err(e) })),
            }
        }
        Request::WsSend { conn, body } => {
            let res = AppWebSocket::send(host, conn, body.into_bytes(), FrameKind::Text).await;
            match res {
                Ok(()) => Ok(json!({ "sent": true })),
                Err(e) => Ok(json!({ "error": e })),
            }
        }
        Request::ReadWsLog => {
            ensure_collection(host, WS_LOG).await?;
            let page = host
                .query(WS_LOG.into(), QueryOptions { filter: None, limit: Some(100), cursor: None })
                .await
                .map_err(fmt_err)?;
            let events: Vec<serde_json::Value> = page
                .records
                .into_iter()
                .filter_map(|r| serde_json::from_slice(&r.payload).ok())
                .collect();
            Ok(json!({ "events": events }))
        }
        Request::ReadHttpStore => {
            ensure_collection(host, HTTP_STORE).await?;
            let page = host
                .query(
                    HTTP_STORE.into(),
                    QueryOptions { filter: None, limit: Some(100), cursor: None },
                )
                .await
                .map_err(fmt_err)?;
            let entries: Vec<serde_json::Value> = page
                .records
                .into_iter()
                .map(|r| {
                    json!({
                        "id": r.id,
                        "payload": String::from_utf8_lossy(&r.payload),
                    })
                })
                .collect();
            Ok(json!({ "entries": entries }))
        }
        Request::SignAsService { draft } => {
            let signed_json = host.sign_record(draft, Principal::Service).await.map_err(fmt_err)?;
            Ok(json!(signed_json))
        }
        Request::SignAsDelegated { draft, delegation_json } => {
            let signed_json = host
                .sign_record(draft, Principal::Delegated(delegation_json))
                .await
                .map_err(fmt_err)?;
            Ok(json!(signed_json))
        }
        Request::SigningIdentity => {
            let id = host.signing_identity().await.map_err(fmt_err)?;
            Ok(json!({
                "signing_did": id.signing_did,
                "pubkey_hex": id.pubkey_hex,
                "owner_did": id.owner_did,
            }))
        }
        Request::VerifyRecord { signed_json, now_secs } => {
            let env: Envelope = serde_json::from_str(&signed_json).map_err(fmt_err)?;
            let check_now = now_secs.unwrap_or(env.issued_at_secs);
            let rec =
                signed_record::verify(&env, &VerifyOptions::new(check_now)).map_err(fmt_err)?;
            Ok(json!({
                "record_id": rec.record_id,
                "signer_did": rec.signer_did,
                "asserter_did": rec.issuer,
                "subject": rec.subject,
                "payload": rec.payload,
                "supersedes": rec.supersedes,
                "valid": true,
            }))
        }
        Request::CallerOrigin => {
            let arm = match AppInvocation::caller(host).await {
                CallerOrigin::Internal => json!({ "arm": "internal" }),
                CallerOrigin::Verified(did) => json!({ "arm": "verified", "did": did }),
                CallerOrigin::Anonymous => json!({ "arm": "anonymous" }),
            };
            Ok(arm)
        }
    }
}
