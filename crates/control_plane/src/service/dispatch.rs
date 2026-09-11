use super::*;

#[async_trait::async_trait]
impl NativeService for ControlPlaneService {
    async fn dispatch(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        info!("Orchestrator received dispatch: {}.{}", invocation.interface, invocation.method);

        if invocation.interface.as_str() == SECURITY_INTERFACE {
            // KEK injection/rotation and vault writes are node-owner
            // operations: a KEK unlocks every service database on this
            // node, so there is no meaningful resource narrower than the
            // node itself to scope this to. The gate is `substrate/admin`
            // on the bare `substrate:<node_did>` resource -- holdable only
            // by a verified `ControllerAgreement` controller (or
            // `[iam].admin_ucan_root`). No exemption for substrate-injected
            // callers: nothing inside the substrate dispatches to this
            // interface.
            if !self.has_node_wide_ability(&invocation.caller, Ability::SUBSTRATE_ADMIN) {
                return Err(RpcError::Custom(
                    PERMISSION_DENIED_CODE,
                    format!(
                        "caller {} holds no substrate/admin on this substrate; the security \
                         interface is node-owner only",
                        invocation.caller.caller_did
                    ),
                    None,
                ));
            }
            match invocation.method.as_str() {
                "inject-kek" => {
                    let (kek_hex,): (String,) =
                        serde_json::from_value(invocation.params).map_err(|e| {
                            RpcError::InvalidParams(format!(
                                "Failed to parse inject-kek params: {e}"
                            ))
                        })?;
                    let kek_bytes = hex::decode(kek_hex)
                        .map_err(|e| RpcError::InvalidParams(format!("Invalid hex KEK: {e}")))?;
                    if kek_bytes.len() != 32 {
                        return Err(RpcError::InvalidParams(
                            "KEK must be exactly 32 bytes".to_string(),
                        ));
                    }
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(&kek_bytes);
                    self.key_store
                        .inject_kek(arr)
                        .map_err(|e| RpcError::InternalError(e.to_string()))?;
                    return Ok(NativeResponse {
                        payload: serde_json::json!({"status": "injected"}),
                    });
                }
                "rotate-kek" => {
                    let (new_kek_hex,): (String,) = serde_json::from_value(invocation.params)
                        .map_err(|e| {
                            RpcError::InvalidParams(format!(
                                "Failed to parse rotate-kek params: {e}"
                            ))
                        })?;
                    let new_kek_bytes = hex::decode(new_kek_hex)
                        .map_err(|e| RpcError::InvalidParams(format!("Invalid hex KEK: {e}")))?;
                    if new_kek_bytes.len() != 32 {
                        return Err(RpcError::InvalidParams(
                            "New KEK must be exactly 32 bytes".to_string(),
                        ));
                    }
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(&new_kek_bytes);
                    self.storage_provider
                        .rotate_kek(&self.key_store, arr)
                        .await
                        .map_err(|e| RpcError::InternalError(e.to_string()))?;
                    return Ok(NativeResponse {
                        payload: serde_json::json!({"status": "rotated"}),
                    });
                }
                "set-secret" => {
                    let (service_id, key, value): (String, String, Vec<u8>) =
                        serde_json::from_value(invocation.params).map_err(|e| {
                            RpcError::InvalidParams(format!(
                                "Failed to parse set-secret params: {e}"
                            ))
                        })?;
                    let store = self
                        .storage_provider
                        .open_service_db(&service_id, &self.key_store)
                        .await
                        .map_err(|e| RpcError::InternalError(e.to_string()))?;
                    store
                        .write_secret(&key, &value)
                        .await
                        .map_err(|e| RpcError::InternalError(e.to_string()))?;
                    return Ok(NativeResponse {
                        payload: serde_json::json!({"status": "secret_set"}),
                    });
                }
                _ => {
                    return Err(RpcError::MethodNotFound(invocation.method));
                }
            }
        }

        if invocation.interface.as_str() == "signing" {
            let Some(signer) = self.record_signer.get().cloned() else {
                return Err(RpcError::InternalError(
                    "this node has no record signer configured".to_string(),
                ));
            };
            match invocation.method.as_str() {
                "identity" => {
                    let target_service_id = if invocation.params.is_null()
                        || invocation.params.as_array().is_some_and(|a| a.is_empty())
                    {
                        self.service_id.clone()
                    } else if let Ok((s,)) =
                        serde_json::from_value::<(String,)>(invocation.params.clone())
                    {
                        s
                    } else if let Ok(s) =
                        serde_json::from_value::<String>(invocation.params.clone())
                    {
                        s
                    } else {
                        return Err(RpcError::InvalidParams(
                            "signing identity expects optional service_id string parameter"
                                .to_string(),
                        ));
                    };
                    let id = signer
                        .identity(&target_service_id)
                        .map_err(crate::synsvc_native::signing_error)?;
                    return Ok(NativeResponse {
                        payload: serde_json::json!({
                            "signing_did": id.signing_did,
                            "pubkey_hex": id.pubkey_hex,
                            "owner_did": id.owner_did,
                        }),
                    });
                }
                other => {
                    return Err(RpcError::MethodNotFound(format!("unknown method: {other}")));
                }
            }
        }

        if invocation.interface.as_str() != ORCHESTRATOR_INTERFACE {
            return Err(RpcError::InternalError(format!(
                "Interface {} not handled by orchestrator",
                invocation.interface
            )));
        }

        match invocation.method.as_str() {
            "readyz" => {
                let service_id = serde_json::from_value::<(String,)>(invocation.params.clone())
                    .map(|(s,)| s)
                    .or_else(|_| serde_json::from_value::<String>(invocation.params.clone()))
                    .or_else(|_| {
                        #[derive(serde::Deserialize)]
                        struct ReadyzPayload {
                            #[serde(alias = "service-id")]
                            service_id: String,
                        }
                        serde_json::from_value::<ReadyzPayload>(invocation.params)
                            .map(|p| p.service_id)
                    })
                    .unwrap_or_default();
                self.readyz(service_id, &invocation.caller)
                    .await
                    .map_err(RpcError::InternalError)?;
                Ok(ready_response())
            }
            "resolve-instance-identity" => {
                let (service_id,): (String,) = serde_json::from_value(invocation.params.clone())
                    .or_else(|_| serde_json::from_value::<String>(invocation.params).map(|s| (s,)))
                    .map_err(|e| {
                        RpcError::InvalidParams(format!(
                            "Failed to parse resolve-instance-identity params: {e}"
                        ))
                    })?;
                let identity = self
                    .instance_identity(service_id, &invocation.caller)
                    .await
                    .map_err(RpcError::InternalError)?;
                Ok(NativeResponse {
                    payload: serde_json::to_value(identity).unwrap_or(Value::Null),
                })
            }
            "deploy" => {
                let (service_id, manifest): (String, DeployManifest) =
                    serde_json::from_value(invocation.params).map_err(|e| {
                        RpcError::InvalidParams(format!("Failed to parse deploy params: {e}"))
                    })?;
                self.deploy(service_id, manifest, &invocation.caller)
                    .await
                    .map_err(RpcError::InternalError)?;
                Ok(NativeResponse { payload: serde_json::json!({"status": "deployed"}) })
            }
            "write-bindings" => {
                let (write,): (BindingWrite,) =
                    serde_json::from_value(invocation.params).map_err(|e| {
                        RpcError::InvalidParams(format!(
                            "Failed to parse write-bindings params: {e}"
                        ))
                    })?;
                let outcomes = self
                    .write_bindings(write, &invocation.caller)
                    .await
                    .map_err(RpcError::InternalError)?;
                Ok(NativeResponse {
                    payload: serde_json::to_value(outcomes).unwrap_or(Value::Null),
                })
            }
            "deploy-plan" => {
                let (plan,): (DeploymentPlan,) = serde_json::from_value(invocation.params.clone())
                    .or_else(|_| {
                        serde_json::from_value::<DeploymentPlan>(invocation.params).map(|p| (p,))
                    })
                    .map_err(|e| {
                        RpcError::InvalidParams(format!("Failed to parse deploy-plan params: {e}"))
                    })?;
                self.deploy_plan(plan, &invocation.caller)
                    .await
                    .map_err(RpcError::InternalError)?;
                Ok(NativeResponse { payload: serde_json::json!({"status": "deployed_plan"}) })
            }
            "undeploy" => {
                let (service_id, generation): (String, u64) =
                    serde_json::from_value(invocation.params).map_err(|e| {
                        RpcError::InvalidParams(format!("Failed to parse undeploy params: {e}"))
                    })?;
                self.undeploy(service_id, generation, &invocation.caller)
                    .await
                    .map_err(RpcError::InternalError)?;
                Ok(NativeResponse { payload: serde_json::json!({"status": "undeployed"}) })
            }
            "restart" => {
                let (service_id, generation): (String, u64) =
                    serde_json::from_value(invocation.params).map_err(|e| {
                        RpcError::InvalidParams(format!("Failed to parse restart params: {e}"))
                    })?;
                self.restart(service_id, generation, &invocation.caller)
                    .await
                    .map_err(RpcError::InternalError)?;
                Ok(NativeResponse { payload: serde_json::json!({"status": "restarted"}) })
            }
            "run-scheduled" => {
                #[derive(serde::Deserialize)]
                struct RunScheduledParams {
                    #[serde(alias = "service-id")]
                    service_id: String,
                    generation: u64,
                    interface: String,
                    method: String,
                    #[serde(alias = "params-json")]
                    params_json: Option<String>,
                }
                let params =
                    serde_json::from_value::<(String, u64, String, String, Option<String>)>(
                        invocation.params.clone(),
                    )
                    .map(|(service_id, generation, interface, method, params_json)| {
                        RunScheduledParams {
                            service_id,
                            generation,
                            interface,
                            method,
                            params_json,
                        }
                    })
                    .or_else(|_| serde_json::from_value::<RunScheduledParams>(invocation.params))
                    .map_err(|e| RpcError::InvalidParams(e.to_string()))?;
                self.run_scheduled(
                    params.service_id,
                    params.generation,
                    params.interface,
                    params.method,
                    params.params_json,
                    &invocation.caller,
                )
                .await
                .map_err(RpcError::InternalError)?;
                Ok(NativeResponse { payload: serde_json::json!({"status": "ran"}) })
            }
            "renew-cert" => {
                let (service_id, generation, instance_certificate): (String, u64, String) =
                    serde_json::from_value(invocation.params).map_err(|e| {
                        RpcError::InvalidParams(format!("Failed to parse renew-cert params: {e}"))
                    })?;
                self.renew_cert(service_id, generation, instance_certificate, &invocation.caller)
                    .await
                    .map_err(RpcError::InternalError)?;
                Ok(NativeResponse { payload: serde_json::json!({"status": "cert_renewed"}) })
            }
            "app-instance-management-of" => {
                let (app_instance_id,): (String,) = serde_json::from_value(invocation.params)
                    .map_err(|e| {
                        RpcError::InvalidParams(format!(
                            "Failed to parse app-instance-management-of params: {e}"
                        ))
                    })?;
                let management = self
                    .app_instance_management_of(app_instance_id, &invocation.caller)
                    .await
                    .map_err(RpcError::InternalError)?;
                Ok(NativeResponse {
                    payload: serde_json::to_value(management).unwrap_or(Value::Null),
                })
            }
            "claim-app-instance" => {
                let (app_instance_id, generation): (String, u64) =
                    serde_json::from_value(invocation.params).map_err(|e| {
                        RpcError::InvalidParams(format!(
                            "Failed to parse claim-app-instance params: {e}"
                        ))
                    })?;
                self.claim_app_instance(app_instance_id, generation, &invocation.caller)
                    .await
                    .map_err(RpcError::InternalError)?;
                Ok(NativeResponse { payload: serde_json::json!({"status": "claimed"}) })
            }
            "release-app-instance" => {
                let (app_instance_id, generation): (String, u64) =
                    serde_json::from_value(invocation.params).map_err(|e| {
                        RpcError::InvalidParams(format!(
                            "Failed to parse release-app-instance params: {e}"
                        ))
                    })?;
                self.release_app_instance(app_instance_id, generation, &invocation.caller)
                    .await
                    .map_err(RpcError::InternalError)?;
                Ok(NativeResponse { payload: serde_json::json!({"status": "released"}) })
            }
            "proxy-outbox" => {
                let service_id = parse_service_id_param(invocation.params);
                let items = self
                    .proxy_outbox(service_id, &invocation.caller)
                    .await
                    .map_err(RpcError::InternalError)?;
                Ok(NativeResponse { payload: serde_json::to_value(items).unwrap_or(Value::Null) })
            }
            "proxy-dead-letters" => {
                let service_id = parse_service_id_param(invocation.params);
                let items = self
                    .proxy_dead_letters(service_id, &invocation.caller)
                    .await
                    .map_err(RpcError::InternalError)?;
                Ok(NativeResponse { payload: serde_json::to_value(items).unwrap_or(Value::Null) })
            }
            "proxy-replay" => {
                #[derive(serde::Deserialize)]
                struct ReplayParams {
                    #[serde(alias = "service-id")]
                    service_id: String,
                    #[serde(alias = "dead-letter-id")]
                    dead_letter_id: u64,
                }
                let params = serde_json::from_value::<(String, u64)>(invocation.params.clone())
                    .map(|(service_id, dead_letter_id)| ReplayParams { service_id, dead_letter_id })
                    .or_else(|_| serde_json::from_value::<ReplayParams>(invocation.params))
                    .map_err(|e| RpcError::InvalidParams(e.to_string()))?;
                self.proxy_replay(params.service_id, params.dead_letter_id, &invocation.caller)
                    .await
                    .map_err(RpcError::InternalError)?;
                Ok(NativeResponse { payload: serde_json::json!({"status": "replayed"}) })
            }
            "sagas" => {
                let service_id = parse_service_id_param(invocation.params);
                let items = self
                    .sagas(service_id, &invocation.caller)
                    .await
                    .map_err(RpcError::InternalError)?;
                Ok(NativeResponse { payload: serde_json::to_value(items).unwrap_or(Value::Null) })
            }
            "saga-compensate" => {
                #[derive(serde::Deserialize)]
                struct SagaCompensateParams {
                    #[serde(alias = "service-id")]
                    service_id: String,
                    #[serde(alias = "saga-id")]
                    saga_id: String,
                }
                let params = serde_json::from_value::<(String, String)>(invocation.params.clone())
                    .map(|(service_id, saga_id)| SagaCompensateParams { service_id, saga_id })
                    .or_else(|_| serde_json::from_value::<SagaCompensateParams>(invocation.params))
                    .map_err(|e| RpcError::InvalidParams(e.to_string()))?;
                self.saga_compensate(params.service_id, params.saga_id, &invocation.caller)
                    .await
                    .map_err(RpcError::InternalError)?;
                Ok(NativeResponse { payload: serde_json::json!({"status": "compensating"}) })
            }
            "list" => {
                let services =
                    self.list(&invocation.caller).await.map_err(RpcError::InternalError)?;
                Ok(NativeResponse {
                    payload: serde_json::to_value(services).unwrap_or(Value::Null),
                })
            }
            "status" => {
                let service_ids = parse_status_params(invocation.params);
                let status = self
                    .status(service_ids, &invocation.caller)
                    .await
                    .map_err(RpcError::InternalError)?;
                Ok(NativeResponse { payload: serde_json::to_value(status).unwrap_or(Value::Null) })
            }
            "node-facts-only" => {
                // A4-06: `status`'s `node` field alone, with none of
                // `status`'s per-service phase-check-and-probe cost -- for a
                // caller (e.g. `app deploy`'s preflight) that wants only
                // these four fields.
                let facts = self.node_facts(&invocation.caller).await;
                Ok(NativeResponse { payload: serde_json::to_value(facts).unwrap_or(Value::Null) })
            }
            "republish" => {
                // Node-wide, like `status`: this republishes this node's own
                // endpoint record, not a single service's, so there is no
                // narrower resource to scope the gate to.
                if !self.has_node_wide_ability(&invocation.caller, Ability::ORCHESTRATOR_STATUS) {
                    return Err(RpcError::Custom(
                        PERMISSION_DENIED_CODE,
                        "caller holds no orchestrator/status on this node".to_string(),
                        None,
                    ));
                }
                self.republish_now().await.map_err(|e| RpcError::InternalError(e.to_string()))?;
                Ok(NativeResponse { payload: serde_json::json!({"status": "republished"}) })
            }
            method => Err(RpcError::MethodNotFound(method.to_string())),
        }
    }
}

fn ready_response() -> NativeResponse {
    NativeResponse { payload: serde_json::json!({"status": "ok"}) }
}

/// Accepts `[[ids]]`, `[ids]`, `{"service_ids": [...]}`, and no params at all
/// -- the same tolerance `readyz`'s params parsing already gives JSON-RPC
/// callers in this tree, which are not consistent about positional-versus-
/// named params. Anything unparseable is treated as an empty list ("every
/// service this caller may see") rather than a hard error, matching
/// `readyz`'s own `unwrap_or_default()`.
/// The single `service-id` argument the `proxy-*` verbs take, accepted in
/// the same three shapes the neighbouring per-service verbs already do
/// (positional tuple, bare string, or a named object).
fn parse_service_id_param(params: Value) -> String {
    serde_json::from_value::<(String,)>(params.clone())
        .map(|(s,)| s)
        .or_else(|_| serde_json::from_value::<String>(params.clone()))
        .or_else(|_| {
            #[derive(serde::Deserialize)]
            struct ServiceIdPayload {
                #[serde(alias = "service-id")]
                service_id: String,
            }
            serde_json::from_value::<ServiceIdPayload>(params).map(|p| p.service_id)
        })
        .unwrap_or_default()
}

fn parse_status_params(params: Value) -> Vec<String> {
    serde_json::from_value::<(Vec<String>,)>(params.clone())
        .map(|(ids,)| ids)
        .or_else(|_| serde_json::from_value::<Vec<String>>(params.clone()))
        .or_else(|_| {
            #[derive(serde::Deserialize)]
            struct StatusPayload {
                #[serde(default, alias = "service-ids")]
                service_ids: Vec<String>,
            }
            serde_json::from_value::<StatusPayload>(params).map(|p| p.service_ids)
        })
        .unwrap_or_default()
}
